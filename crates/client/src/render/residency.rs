//! What the client is still holding on to, and when it lets go.
//!
//! **Every other pass in this directory is about acquiring something.** The
//! terrain streamer loads a tile, the model cache reads an M2, the WMO cache
//! stages a city, the doodad stream spawns a resident. Each of them was written
//! with a bound on what it *spawns* — `VisibilityRange`, the 3x3, the draw-range
//! stream — and none of them had a bound on what it *keeps*.
//!
//! That is invisible to every number the HUD reports, because all of them count
//! spawned things. The report it produces instead is "140 fps when I start, and
//! less after teleporting around for a while", with no single frame slow and no
//! count moving. The growth is in GPU memory: a mesh and an image are owned by
//! their `Handle`, so a cache entry that is never dropped is a texture that is
//! never freed, and the frame does not get slower until the driver starts
//! paging — at which point it gets slower everywhere at once.
//!
//! Two mechanisms, and they are different in kind:
//!
//! * **the caches never evicted.** [`crate::render::models::ModelCache`],
//!   [`crate::render::wmos::WmoCache`] and
//!   [`crate::render::models::MaterialPool`] held a strong handle to everything
//!   they had ever been asked for, so walking Elwynn → Stormwind → Tanaris held
//!   all three zones' geometry and every texture in them until the process
//!   ended. [`sweep`] is the answer.
//! * **leaving the world tore nothing down.** `Session::log_out` sets
//!   `active = None`, which makes `poll_world` return early — so the terrain,
//!   the entities, their skeletons, their poses and their emitters all stayed,
//!   and the client went on simulating and drawing a city nobody was in behind
//!   the character-select screen. [`leave_world`] is the answer.
//!
//! ## What eviction can and cannot break
//!
//! Nothing here can pull an asset out from under something that is drawn. A
//! Bevy asset is owned by its handles, and these caches hold *a* handle, never
//! the only one: a spawned batch holds its own, and so does a parked
//! [`crate::render::doodads::Resident`]. Dropping the cache's share of a model
//! that is still standing in the world frees nothing at all — it only means the
//! next tile that places it re-reads it.
//!
//! So the cost of evicting too eagerly is a re-read, not a missing picture, and
//! the two windows are sized against that: [`IDLE_SECS`] is generous enough
//! that crossing a tile boundary and coming back does not pay for it, and
//! [`WmoPlacement::path`] exists so that the one genuinely expensive re-read —
//! a city — cannot happen at all while the city is standing.
//!
//! ## And the number it is all about is now on the line
//!
//! Everything above was diagnosed from the *shape* of a report and fixed
//! against counts of held things, which are a claim about GPU memory rather
//! than a measurement of it — the two can move apart, because an `Assets` entry
//! is not an allocation and a suballocator hands a block back to its pool
//! rather than to the driver. [`gpu_bytes`] closes that: the allocator's own
//! total, sampled on the same sweep, with the session's peak beside it so that
//! "it holds 500 MiB" can be told from "it climbed to 900 and came back".
//!
//! [`WmoPlacement::path`]: crate::render::wmos::WmoPlacement::path

use crate::render::models::{MaterialPool, ModelCache, M2Material};
use crate::render::terrain::{LoadedTiles, TerrainTile};
use crate::render::wmos::{WmoCache, WmoPlacement};
#[cfg(feature = "diagnostics")]
use crate::ui::report::{HudReport, Slot};
use crate::world::motion::Motion;
use crate::world::session::{EntityIndex, Session, Solids, WorldEntity, WorldStatus};
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;

/// How long a cached model, dressing, skin or building may go unwanted before
/// it is dropped.
///
/// **This is the reference client's own number, and it used to be a guess.**
/// The client's model cache sweeps its idle list against the millisecond
/// clock: an entry released **30,000 ms** ago or more is unlinked and
/// destroyed, and the walk ends at the first one younger than that.
///
/// An entry is stamped with its release time only when a release takes its
/// refcount **to zero**, and is pushed onto the list then; taking a new
/// reference takes it back off. So 1.12's model cache is
/// refcount-plus-idle-window, which is this module's policy — arrived at
/// independently, and wrong by a factor of two until the reference's window
/// was known.
///
/// The argument the old value carried is still the right argument and it simply
/// over-estimated: what has to be survived is a player pacing across a tile
/// boundary, where the 3x3 shifts and a column is dropped and looked up again.
/// Thirty seconds is many times that round trip, and it is what the game did.
///
/// **One thing about the reference's shape is deliberately not copied.** Its
/// idle list is a FIFO in release order, so the sweep stops at the first entry
/// too young and costs O(evicted); this one walks every entry every
/// [`SWEEP_SECS`] and costs O(held). At a few thousand hash entries a fifth of a
/// second apart that is microseconds, and a second index to keep in step is a
/// second place for the two to disagree. Stated because it is a difference, not
/// because it is felt.
pub const IDLE_SECS: f32 = 30.0;

/// How often the caches are swept.
///
/// It has to be well under [`IDLE_SECS`] — the idle window is measured from the
/// last *touch*, and a sweep interval as long as it would let an entry live
/// nearly twice as long as advertised. It also has to be long enough that the
/// sweep itself is not part of the frame: it walks every cache entry and every
/// placed building, which is thousands of hash entries, so it is microseconds
/// once every few seconds rather than anything a frame notices.
const SWEEP_SECS: f32 = 5.0;

/// [`HudReport`] slot. See `ui::report`.
#[cfg(feature = "diagnostics")]
const SLOT: Slot = Slot(90);

/// What the last sweep found and freed, for the HUD.
///
/// **These are the numbers the report this module exists for would be settled
/// by**, and the reason they are worth a line of their own is that no other
/// number on the window moves when they do: models, dressings, buildings and
/// materials are all counts of what is *held*, where every other count on the
/// HUD is of what is spawned.
#[derive(Resource, Default)]
pub struct Residency {
    /// `Time::elapsed_secs` at the last sweep.
    swept_at: f32,
    /// **When the world was left**, and what makes the next sweep a purge.
    ///
    /// [`IDLE_SECS`] is sized for a player walking across a tile boundary, where
    /// evicting something that is about to be wanted again costs a re-read.
    /// Logging out is the one moment when *everything* the world held stops
    /// being wanted at once — the tiles, the doodads, the buildings, every
    /// creature's dressing and skin — and a minute of it is a city's worth of
    /// meshes and textures kept alive behind the login screen for no reason at
    /// all. So the next sweep uses this instant as its deadline instead: it
    /// drops whatever nothing has asked for *since the logout*, which is
    /// everything except what the glue screen has just asked for.
    ///
    /// **The next sweep and not this frame**, because [`leave_world`]'s
    /// despawns are queued commands: the material pool's prune counts live
    /// handles, and the handles the world was holding are not dropped until
    /// those commands have been applied.
    left_at: Option<f32>,
    /// Cached geometry builds, dressings and skins — see
    /// [`ModelCache::resident`].
    pub models: usize,
    pub dressings: usize,
    pub skins: usize,
    /// Buildings held by [`WmoCache`].
    pub buildings: usize,
    /// Live entries in [`MaterialPool`], dead keys already pruned.
    pub materials: usize,
    /// What `Assets` actually holds, which is the number the GPU memory tracks
    /// and the one the caches above are only a claim about.
    pub meshes: usize,
    pub images: usize,
    /// Everything dropped since the session started, so that "it is holding
    /// 2,000 models" can be told from "it has never let go of anything".
    pub freed: usize,
    /// What the driver has actually handed out, in bytes, and the most it has
    /// ever held this session — see [`gpu_bytes`].
    ///
    /// `None` where the backend does not report it, which is honest rather than
    /// zero: a zero here would read as "nothing allocated" for a client drawing
    /// a city.
    pub gpu: Option<u64>,
    pub gpu_peak: u64,
}

/// What the GPU allocator is holding, in bytes.
///
/// **This is the number the whole module was written on a hunch about.** The
/// report that produced it — "140 fps at login, less an hour later, no count
/// moving" — was diagnosed as unbounded caches from the *shape* of the symptom,
/// and the fix was made and measured in mesh and image counts, which are a
/// claim about memory rather than a measurement of it. Two of those counts can
/// fall while the driver holds exactly as much as before, because an `Assets`
/// entry is not a GPU allocation: a mesh is freed when its handle drops *and*
/// the render world's extraction of it is dropped, one frame later, and a
/// suballocator hands the block back to its pool rather than to the driver.
///
/// `total_allocated_bytes` is what wgpu's allocator has given out;
/// `total_reserved_bytes` is what it has taken from the driver and is holding
/// in its pools. The first is what the client is responsible for and the one
/// reported here — the second falls only when the allocator decides to release
/// a block, so it is the wrong number for "did the eviction do anything".
///
/// **It is `None` on some backends and that is not a failure.** The report
/// comes from `gpu-allocator`, which wgpu-hal uses for Vulkan and DX12; GL and
/// the software backends have nothing to report and say so.
///
/// Called once every [`SWEEP_SECS`] and never per frame: the call walks every
/// live allocation and collects a `Vec` of them, which is fine at a fifth of a
/// hertz and would be silly at sixty.
fn gpu_bytes(device: &RenderDevice) -> Option<u64> {
    device
        .wgpu_device()
        .generate_allocator_report()
        .map(|report| report.total_allocated_bytes)
}

/// Bytes as mebibytes, for the HUD. Integer because a tenth of a mebibyte is
/// below the noise of a single texture upload.
fn mib(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

pub struct ResidencyPlugin;

impl Plugin for ResidencyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Residency>()
            // `tick` before everything that stamps a cache entry, so a lookup
            // in the same frame records this frame's clock rather than the
            // previous one's. One frame either way changes nothing about a
            // sixty-second window — it is stated because an ordering that
            // matters is stated, and because `sweep` reading a
            // clock `tick` has not written yet would evict on the *first*
            // frame, when every stamp is still zero.
            .add_systems(Update, (tick, sweep).chain())
            // Teardown is its own system and deliberately not in that chain:
            // it despawns the world the other three are counting, and it runs
            // at most once per logout.
            .add_systems(Update, leave_world);
        // …and the HUD line, which is the one part of this pass that is an
        // instrument rather than a policy — see `ui::debug` for what the
        // `diagnostics` feature takes out. `report` used to be the third link
        // of the chain above; the sweep does not depend on it.
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report.after(sweep).run_if(crate::ui::report::watched));
    }
}

/// Hand this frame's clock to the caches, so that a lookup can stamp what it
/// touched without any of them taking `Res<Time>`.
fn tick(time: Res<Time>, mut models: ResMut<ModelCache>, mut wmos: ResMut<WmoCache>) {
    let now = time.elapsed_secs();
    models.tick(now);
    wmos.tick(now);
}

/// Drop what nothing has wanted for [`IDLE_SECS`].
fn sweep(
    time: Res<Time>,
    mut residency: ResMut<Residency>,
    mut models: ResMut<ModelCache>,
    mut wmos: ResMut<WmoCache>,
    mut pool: ResMut<MaterialPool>,
    materials: Res<Assets<M2Material>>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    // Optional because a headless test app has no render world and therefore no
    // device — the same reason `DrawCallPlugin` checks for one.
    device: Option<Res<RenderDevice>>,
    placed: Query<&WmoPlacement>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Residency);
    let now = time.elapsed_secs();
    // **A logout is a sweep of its own, on the frame after it.** Not this frame:
    // `leave_world`'s despawns are queued and the handles they hold are still
    // alive until the commands have been applied. See [`Residency::left_at`].
    let purge = residency.left_at.filter(|left| now > *left);
    if purge.is_none() && now - residency.swept_at < SWEEP_SECS {
        return;
    }
    residency.swept_at = now;

    // A building standing in the world is wanted whatever the clock says — see
    // `WmoPlacement::path`. A dozen strings a tile.
    for placement in &placed {
        wmos.touch(&placement.path);
    }

    // …and a purge's deadline is the logout itself rather than a minute back,
    // so what survives is exactly what has been asked for *since* — which at
    // the login screen is the glue scene and nothing else.
    let before = match residency.left_at.take().filter(|_| purge.is_some()) {
        Some(left) => left,
        None => now - IDLE_SECS,
    };
    let dropped = models.evict(before);
    let buildings = wmos.evict(before);
    // **After the two evictions, not before.** A material whose last holder was
    // a dressing dropped a line ago is only unreferenced once that drop has
    // happened, and a prune that ran first would leave the key behind for
    // another five seconds — which is harmless and would make the count on the
    // HUD lag the thing it is reporting by exactly one sweep.
    let materials_dropped = pool.prune(&materials);

    residency.freed +=
        dropped.models + dropped.dressings + dropped.textures + buildings + materials_dropped;
    (residency.models, residency.dressings, residency.skins) = models.resident();
    let (held, _failed) = wmos.counts();
    residency.buildings = held;
    residency.materials = pool.distinct();
    residency.meshes = meshes.len();
    residency.images = images.len();
    residency.gpu = device.as_deref().and_then(gpu_bytes);
    residency.gpu_peak = residency.gpu_peak.max(residency.gpu.unwrap_or(0));

    if dropped.any() || buildings > 0 || materials_dropped > 0 {
        debug!(
            "residency: freed {} models, {} dressings, {} skins, {buildings} buildings, \
             {materials_dropped} materials — holding {} meshes, {} images, {} MiB",
            dropped.models,
            dropped.dressings,
            dropped.textures,
            residency.meshes,
            residency.images,
            residency.gpu.map(mib).unwrap_or(0),
        );
    }
}

/// The HUD line. See [`crate::ui::report`] for why it is written here rather
/// than in `hud.rs`.
#[cfg(feature = "diagnostics")]
fn report(residency: Res<Residency>, mut hud: ResMut<HudReport>) {
    if !residency.is_changed() {
        return;
    }
    // **The GPU figure is the point of the line and goes at the end of it**,
    // where the eye lands after the counts that are only a claim about it. A
    // backend that does not report is written out rather than shown as zero.
    let gpu = match residency.gpu {
        Some(bytes) => format!("{} MiB gpu, peak {}", mib(bytes), mib(residency.gpu_peak)),
        None => "gpu not reported".to_string(),
    };
    hud.set(
            crate::ui::report::Section::Scene,
        SLOT,
        "residency",
        format!(
            "held: {} models, {} dressings, {} skins, {} buildings, {} materials  \
             ({} meshes, {} images, {} freed, {gpu})",
            residency.models,
            residency.dressings,
            residency.skins,
            residency.buildings,
            residency.materials,
            residency.meshes,
            residency.images,
            residency.freed,
        ),
    );
}

/// Tear the world down when the session ends.
///
/// **Logging out used to leave the whole world standing.** `Session::log_out`
/// drops the socket and sets `active = None`, and every system that reconciles
/// the world begins by returning early when there is no active session — so
/// nothing despawned anything. The terrain kept streaming around the last
/// position, every entity kept its skeleton posed sixty times a second, the
/// emitters kept simulating, and the collision hulls stayed keyed to a map
/// nobody was on. The character-select screen was drawn over a live city.
///
/// It was not a permanent leak: logging back in reconciles the entity index
/// against the new session's own answers and despawns the strays on the first
/// poll. What it cost was the whole per-frame cost of a city for as long as the
/// client sat at the character screen, and a `Solids` keyed to the old map
/// until the next `enter_world_blocking` cleared it.
///
/// ## What it is keyed on, and the race that says why
///
/// It used to be keyed on `WorldStatus::in_world` outliving `Session::active`,
/// described here as "the one state that only happens on the way out". **That is
/// not true, and when it fails the whole teardown silently does not happen.**
///
/// `in_world` is a *copy*, written by `poll_world` from the session thread's own
/// status — and the session thread sets its own copy false the instant it stops,
/// which on a logout is microseconds after it queues the
/// `SMSG_LOGOUT_COMPLETE` event that the client has not read yet. So the frame
/// order that breaks it is the ordinary one:
///
/// ```text
/// poll_world      active = Some, thread stopped -> status.in_world = false
/// logout::answer  Logout::Complete              -> active = None
/// leave_world     active is None … and in_world is already false -> returns
/// ```
///
/// Nothing after that ever fires it: the tiles stay, `LoadedTiles` keeps the map
/// and the centre it will never ask for again, the hulls stay keyed to the map
/// just left, the caches are never told the world went, and — the one with the
/// longest reach — `WmoCache::forget_failures` is never called, so a building
/// that would not read once cannot be read again for the life of the **process**.
/// On an ordinary map none of that is visible, because the ground and the city
/// are still standing from the session before and a relog lands back on them. On
/// a map whose whole geometry is one building it is the report: the host is
/// despawned by `globalwmo`'s own map test whatever this does, so the world goes
/// and the bookkeeping that would rebuild it does not.
///
/// Which of the two systems runs first is Bevy's executor's choice — neither is
/// ordered against the other — so it is a race that reads as a deterministic bug.
///
/// **Both signals are now witnesses that there was a world and either is
/// enough**, which is [`ended`]: `in_world` is kept because the harness can set
/// it and because it is still true on the ordinary path, and the session handle
/// is added because it is the one that survives `poll_world` writing the other.
pub fn leave_world(
    mut commands: Commands,
    session: Res<Session>,
    time: Res<Time>,
    mut had_world: Local<bool>,
    mut residency: ResMut<Residency>,
    mut status: ResMut<WorldStatus>,
    mut tiles_held: ResMut<LoadedTiles>,
    mut wmos: ResMut<crate::render::wmos::WmoCache>,
    mut index: ResMut<EntityIndex>,
    mut motion: ResMut<Motion>,
    // …and the routes built off `DisplayTables`' own taxi tables, which a new
    // session may open a different archive chain for. Nine entries, so this is
    // correctness rather than memory. See
    // [`crate::world::entities::transport::ShipRoutes`].
    mut ship_routes: ResMut<crate::world::entities::transport::ShipRoutes>,
    solids: Res<Solids>,
    tiles: Query<Entity, With<TerrainTile>>,
    entities: Query<Entity, With<WorldEntity>>,
) {
    if !ended(session.active.is_some(), status.in_world, &mut had_world) {
        return;
    }
    // The ground, and with it — as children — every doodad, building, room and
    // piece of furniture on it, plus the residents parked against each tile.
    let mut dropped = 0;
    for tile in &tiles {
        commands.entity(tile).despawn();
        dropped += 1;
    }
    tiles_held.forget();
    ship_routes.reset();

    // The units and objects, and with them their joints, their attachments and
    // their shadows — all children of the root.
    let mut units = 0;
    for entity in &entities {
        commands.entity(entity).despawn();
        units += 1;
    }
    index.0.clear();
    motion.reset();

    // A hull keyed to the map just left is an invisible wall in an empty field.
    // `enter_world_blocking` clears these too, on the way *in*; doing it here
    // as well is what keeps a logged-out client from holding a city's worth of
    // triangles for as long as the character screen is open.
    solids.0.clear();

    // …and everything the caches were holding *for* it, on the next sweep — see
    // [`Residency::left_at`], which is why it is not this frame's.
    residency.left_at = Some(time.elapsed_secs());

    // …and which buildings would not read, which is the one cache entry that
    // is never evicted and therefore the one that can outlive its reason. See
    // [`crate::render::wmos::WmoCache::forget_failures`].
    let forgotten = wmos.forget_failures();

    status.in_world = false;
    status.entity_count = 0;
    status.warnings.clear();
    info!(
        "left the world: {dropped} tiles and {units} entities dropped\
         {}",
        match forgotten {
            0 => String::new(),
            n => format!(", {n} unreadable building(s) forgotten"),
        }
    );
}

/// **Is this the frame the world went away on?** — the edge [`leave_world`] is
/// keyed on, as a function of two signals and last frame's answer.
///
/// A free function for the reason `interface::left_world` is one and
/// `glue::loading::reckon` is one: an
/// [`crate::world::session::ActiveSession`] owns a socket and cannot be forged in
/// a test, so the *rule* would otherwise be the one part of the teardown that
/// nothing checks — and it is the part with a wrong answer on either side of it.
/// Firing every frame despawns the glue screen's own scene; never firing is the
/// bug in [`leave_world`]'s own doc.
///
/// `alive` is whether there is a session **now**; `in_world` is `poll_world`'s
/// copy of the session thread's opinion, which can fall a frame or more early.
/// Either is evidence that there was a world last frame, and the teardown needs
/// only one of them: `*had` is what carries that evidence across the frame where
/// the first has already gone false and the second has not yet.
fn ended(alive: bool, in_world: bool, had: &mut bool) -> bool {
    let there_was_a_world = *had || in_world;
    *had = alive;
    !alive && there_was_a_world
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A backend with no allocator report says so, rather than reporting
    /// zero.** This is the one number on the line that is not a count of
    /// something the client itself holds, so a plausible-looking `0 MiB gpu`
    /// beside eleven thousand meshes would be read as a working instrument
    /// saying something remarkable — which is exactly the failure mode
    /// `MaterialPool`'s counters were written to avoid.
    #[test]
    fn a_backend_that_cannot_report_memory_does_not_report_none_of_it() {
        let mut app = App::new();
        app.init_resource::<Residency>()
            .init_resource::<HudReport>()
            .add_systems(Update, report);
        app.update();
        let line = app
            .world()
            .resource::<HudReport>()
            .line(SLOT, "residency")
            .expect("the residency line")
            .to_string();
        assert!(line.contains("gpu not reported"), "{line}");

        app.world_mut().resource_mut::<Residency>().gpu = Some(700 * 1024 * 1024);
        app.world_mut().resource_mut::<Residency>().gpu_peak = 900 * 1024 * 1024;
        app.update();
        let line = app.world().resource::<HudReport>().line(SLOT, "residency").unwrap();
        assert!(line.contains("700 MiB gpu, peak 900"), "{line}");
    }

    /// **The teardown fires when the session goes, even though `in_world` fell
    /// first** — which is the race in [`leave_world`]'s own doc and the one that
    /// made a logout leave the world's bookkeeping standing.
    ///
    /// The three frames are the ones the client actually runs: a session in the
    /// world; a session whose thread has already stopped, so `poll_world` has
    /// copied `in_world = false` while `Session::active` is still `Some`; and the
    /// frame after, where `logout::answer` has taken the session. The old rule
    /// looked only at the second signal and therefore never fired at all.
    #[test]
    fn a_world_whose_session_outlives_its_in_world_flag_is_still_torn_down() {
        let mut had = false;
        assert!(!ended(true, true, &mut had), "in the world");
        assert!(!ended(true, false, &mut had), "the thread stopped first");
        assert!(ended(false, false, &mut had), "…and the session went after it");
        // Once, and not once a frame for as long as the character screen is
        // open — a second firing is a despawn of entities that are already gone.
        assert!(!ended(false, false, &mut had));
    }

    /// …and the ordinary order still works, and a client that has never logged
    /// in is never told it has left.
    #[test]
    fn the_teardown_fires_once_on_the_way_out_and_never_before_the_way_in() {
        let mut had = false;
        // Nothing has happened yet: no session, no world.
        assert!(!ended(false, false, &mut had));
        assert!(!ended(true, false, &mut had), "entering, before the first poll");
        assert!(!ended(true, true, &mut had));
        // `Session::active` goes first here, which is the order the old rule was
        // written for and which has to keep working.
        assert!(ended(false, true, &mut had));
        assert!(!ended(false, false, &mut had));
    }

    /// The sweep interval has to be well under the idle window, or an entry
    /// lives for nearly `IDLE_SECS + SWEEP_SECS` and the constant does not mean
    /// what it says. Cheap to state and easy to break by tuning one of them.
    #[test]
    fn the_sweep_runs_several_times_inside_the_idle_window() {
        assert!(
            SWEEP_SECS * 4.0 <= IDLE_SECS,
            "a {SWEEP_SECS}s sweep is too coarse for a {IDLE_SECS}s window"
        );
    }

    /// **The regression this module exists for**, at the level a test can reach
    /// without a login: leaving the world despawns the ground and the entities
    /// and forgets the tiles, rather than leaving a city standing behind the
    /// character screen.
    ///
    /// `Session::active` is `None` in a headless app, so the state under test —
    /// `in_world` outliving the session — is exactly what a fresh `WorldStatus`
    /// with `in_world` set describes.
    #[test]
    fn leaving_the_world_takes_the_world_with_it() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<Session>()
            .init_resource::<Residency>()
            .init_resource::<WorldStatus>()
            .init_resource::<LoadedTiles>()
            // …and the building cache, whose `failed` set is forgotten here —
            // see [`WmoCache::forget_failures`].
            .init_resource::<WmoCache>()
            .init_resource::<EntityIndex>()
            .init_resource::<Motion>()
            // …and the transport routes, forgotten here for the same reason the
            // building cache's failures are.
            .init_resource::<crate::world::entities::transport::ShipRoutes>()
            .init_resource::<Solids>()
            .add_systems(Update, leave_world);

        let tile = app
            .world_mut()
            .spawn(TerrainTile { coord: (32, 48) })
            .id();
        // A doodad on that tile: a child, so the despawn has to be recursive.
        let doodad = app.world_mut().spawn(ChildOf(tile)).id();
        app.world_mut().resource_mut::<WorldStatus>().in_world = true;

        // Not yet: `in_world` is false at first, and a client that has never
        // logged in must not be told it has left.
        app.world_mut().resource_mut::<WorldStatus>().in_world = false;
        app.update();
        assert!(app.world().get_entity(tile).is_ok(), "nothing to leave");

        app.world_mut().resource_mut::<WorldStatus>().in_world = true;
        app.update();
        assert!(app.world().get_entity(tile).is_err(), "the ground stayed");
        assert!(app.world().get_entity(doodad).is_err(), "its doodads stayed");
        assert!(!app.world().resource::<WorldStatus>().in_world);
        // …and the caches are told to let go of what they were holding for it,
        // rather than sitting on a city's worth of meshes for `IDLE_SECS`
        // behind the login screen. See [`Residency::left_at`].
        assert!(
            app.world().resource::<Residency>().left_at.is_some(),
            "the sweep was not told the world had gone"
        );
        // **…and the one cache entry nothing else ever evicts.** A building
        // that would not read is remembered so that its placements stop asking;
        // remembered across a session it is a building that can never come
        // back, which on a map whose whole geometry is one building is an empty
        // room with no floor for the rest of the process.
        assert_eq!(
            app.world_mut().resource_mut::<WmoCache>().forget_failures(),
            0,
            "the failures were already forgotten on the way out"
        );

        // …and it does not fire a second time, which would be a despawn of
        // entities that no longer exist — Bevy's B0003 warning, once a frame,
        // for as long as the character screen is open.
        app.update();
    }
}
