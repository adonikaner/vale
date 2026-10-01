//! Eviction of cached render assets, and teardown of the world when a session
//! ends.
//!
//! The other passes in this directory acquire assets: the terrain streamer
//! loads a tile, the model cache reads an M2, the WMO cache stages a city, the
//! doodad stream spawns a resident. Each bounds what it spawns
//! (`VisibilityRange`, the 3x3, the draw-range stream). Before this module
//! none bounded what it keeps.
//!
//! Every number the HUD reported counted spawned things, so unbounded caches
//! changed none of them. The symptom was 140 fps at start and less after
//! teleporting around for a while, with no single frame slow and no count
//! changing. The growth is in GPU memory: a mesh and an image are owned by
//! their `Handle`, so a cache entry that is never dropped is a texture that is
//! never freed. The frame time does not rise until the driver starts paging,
//! and then it rises for every frame.
//!
//! There were two causes, each with its own fix:
//!
//! * The caches never evicted. [`crate::render::models::ModelCache`],
//!   [`crate::render::wmos::WmoCache`] and
//!   [`crate::render::models::MaterialPool`] held a strong handle to everything
//!   they had ever been asked for, so walking Elwynn → Stormwind → Tanaris held
//!   all three zones' geometry and every texture in them until the process
//!   ended. [`sweep`] evicts idle entries.
//! * Leaving the world despawned nothing. `Session::log_out` sets
//!   `active = None`, which makes `poll_world` return early, so the terrain,
//!   the entities, their skeletons, their poses and their emitters all stayed,
//!   and the client kept simulating and drawing them behind the
//!   character-select screen. [`leave_world`] despawns them.
//!
//! ## Eviction cannot remove an asset that is drawn
//!
//! A Bevy asset is owned by its handles, and these caches hold one handle,
//! never the only one: a spawned batch holds its own, and so does a parked
//! [`crate::render::doodads::Resident`]. Dropping the cache's handle to a model
//! that is still spawned frees nothing. The next tile that places the model
//! reads it again.
//!
//! The cost of evicting too early is therefore a re-read, not a missing asset,
//! and the two windows are sized against that cost. [`IDLE_SECS`] is long
//! enough that crossing a tile boundary and coming back does not cause a
//! re-read. [`WmoPlacement::path`] lets the sweep keep a placed building's
//! cache entry, so the most expensive re-read, a city, cannot happen while the
//! city is spawned.
//!
//! ## GPU memory is measured, not inferred from the counts
//!
//! The counts of held things estimate GPU memory; they do not measure it. The
//! two can differ, because an `Assets` entry is not an allocation and a
//! suballocator returns a block to its pool, not to the driver. [`gpu_bytes`]
//! reads the allocator's own total on the same sweep. The session's peak is
//! kept beside it, so that "it holds 500 MiB" can be told from "it climbed to
//! 900 and came back".
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
/// This is the 1.12.1 client's value. The client destroys a cached model
/// 30,000 ms after the last user of it releases it, and a new use before then
/// cancels that. This module's policy is the same: an entry is kept while
/// something holds it, and for an idle window after. The window here was twice
/// as long until the client's value was known.
///
/// The window has to outlast a player pacing across a tile boundary, where the
/// 3x3 shifts and a column is dropped and looked up again. Thirty seconds is
/// many times that round trip.
///
/// [`sweep`] walks every entry every [`SWEEP_SECS`], which costs O(held). A
/// list of idle entries in release order would let a sweep stop at the first
/// entry that is too young and cost O(evicted), but it is a second index that
/// has to be kept consistent with the cache. A walk of a few thousand hash
/// entries once every [`SWEEP_SECS`] takes microseconds.
pub const IDLE_SECS: f32 = 30.0;

/// How often the caches are swept.
///
/// It must be well under [`IDLE_SECS`]: the idle window is measured from the
/// last touch, so a sweep interval as long as the window would let an entry
/// live nearly twice the window. It must also be long enough that the sweep
/// adds nothing measurable to the frame. The sweep walks every cache entry and
/// every placed building, which is thousands of hash entries, so it costs
/// microseconds once every few seconds.
const SWEEP_SECS: f32 = 5.0;

/// [`HudReport`] slot. See `ui::report`.
#[cfg(feature = "diagnostics")]
const SLOT: Slot = Slot(90);

/// What the last sweep found and freed, for the HUD.
///
/// Models, dressings, buildings and materials are counts of what is held.
/// Every other count on the HUD is of what is spawned, so no other number on
/// it changes when the caches grow. That is why these have a line of their
/// own.
#[derive(Resource, Default)]
pub struct Residency {
    /// `Time::elapsed_secs` at the last sweep.
    swept_at: f32,
    /// When the world was left. While this is set, the next sweep is a purge.
    ///
    /// [`IDLE_SECS`] is sized for a player walking across a tile boundary, where
    /// evicting something that is about to be wanted again costs a re-read.
    /// At a logout everything the world held stops being wanted at once: the
    /// tiles, the doodads, the buildings, every creature's dressing and skin.
    /// Keeping it for the idle window would hold a city's meshes and textures
    /// behind the login screen. The next sweep therefore uses this instant as
    /// its deadline: it drops whatever nothing has asked for since the logout,
    /// which is everything except what the glue screen has asked for.
    ///
    /// The purge runs on the next sweep and not on the logout frame, because
    /// [`leave_world`]'s despawns are queued commands: the material pool's
    /// prune counts live handles, and the handles the world was holding are not
    /// dropped until those commands have been applied.
    left_at: Option<f32>,
    /// Cached geometry builds, dressings and skins. See
    /// [`ModelCache::resident`].
    pub models: usize,
    pub dressings: usize,
    pub skins: usize,
    /// Buildings held by [`WmoCache`].
    pub buildings: usize,
    /// Live entries in [`MaterialPool`], dead keys already pruned.
    pub materials: usize,
    /// What `Assets` holds. GPU memory follows these two counts; the cache
    /// counts above only estimate it.
    pub meshes: usize,
    pub images: usize,
    /// Everything dropped since the session started, so that "it is holding
    /// 2,000 models" can be told from "it has never let go of anything".
    pub freed: usize,
    /// What the GPU allocator has handed out, in bytes, and the most it has
    /// held this session. See [`gpu_bytes`].
    ///
    /// `None` where the backend does not report it. A zero here would read as
    /// "nothing allocated" for a client drawing a city.
    pub gpu: Option<u64>,
    pub gpu_peak: u64,
}

/// What the GPU allocator is holding, in bytes.
///
/// The unbounded caches were diagnosed from a symptom ("140 fps at login, less
/// an hour later, no count moving"), and the fix was measured in mesh and
/// image counts. Those counts estimate memory; they do not measure it. Both
/// can fall while the driver holds as much as before, because an `Assets`
/// entry is not a GPU allocation: a mesh is freed when its handle drops and
/// the render world's extraction of it is dropped, one frame later, and a
/// suballocator returns the block to its pool, not to the driver.
///
/// `total_allocated_bytes` is what wgpu's allocator has given out;
/// `total_reserved_bytes` is what it has taken from the driver and is holding
/// in its pools. This function reports the first, which is what the client is
/// responsible for. The second falls only when the allocator releases a block,
/// so it does not show whether an eviction freed anything.
///
/// The result is `None` on some backends, and that is not a failure. The
/// report comes from `gpu-allocator`, which wgpu-hal uses for Vulkan and DX12;
/// GL and the software backends have nothing to report.
///
/// Called once every [`SWEEP_SECS`] and never per frame: the call walks every
/// live allocation and collects a `Vec` of them, which is acceptable at 0.2 Hz
/// and too costly at 60 Hz.
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
            // `tick` runs before every system that stamps a cache entry, so a
            // lookup in the same frame records this frame's clock and not the
            // previous frame's. One frame either way does not matter to the
            // `IDLE_SECS` window. The order matters to `sweep`: reading a
            // clock that `tick` has not written yet would evict on the first
            // frame, when every stamp is still zero.
            .add_systems(Update, (tick, sweep).chain())
            // The teardown is not in that chain: it despawns the world that
            // `tick`, `sweep` and `report` count, and it runs at most once per
            // logout.
            .add_systems(Update, (leave_world, enter_world));
        // The HUD line is diagnostics, not policy. See `ui::debug` for what
        // the `diagnostics` feature removes. `report` is ordered after `sweep`
        // and is not in the chain above, because the sweep does not depend on
        // it.
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
    // device. `DrawCallPlugin` checks for one for the same reason.
    device: Option<Res<RenderDevice>>,
    placed: Query<&WmoPlacement>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Residency);
    let now = time.elapsed_secs();
    // A logout causes a sweep on the frame after it, whatever the interval.
    // Not on the logout frame: `leave_world`'s despawns are queued and the
    // handles they hold stay alive until the commands have been applied. See
    // [`Residency::left_at`].
    let purge = residency.left_at.filter(|left| now > *left);
    if purge.is_none() && now - residency.swept_at < SWEEP_SECS {
        return;
    }
    residency.swept_at = now;

    // A building that is placed in the world is wanted whatever its stamp
    // says. See `WmoPlacement::path`. About a dozen strings per tile.
    for placement in &placed {
        wmos.touch(&placement.path);
    }

    // A purge's deadline is the logout instant, not `IDLE_SECS` before now, so
    // what survives is what has been asked for since the logout. At the login
    // screen that is the glue scene and nothing else.
    let before = match residency.left_at.take().filter(|_| purge.is_some()) {
        Some(left) => left,
        None => now - IDLE_SECS,
    };
    let dropped = models.evict(before);
    let buildings = wmos.evict(before);
    // The prune runs after the two evictions. A material whose last holder was
    // a dressing evicted above is unreferenced only once that eviction has
    // happened. A prune that ran first would leave the key for another five
    // seconds, which is harmless but makes the material count on the HUD lag
    // by one sweep.
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
    // The GPU figure goes last on the line, after the counts that only
    // estimate it. A backend that does not report is shown as "gpu not
    // reported", not as zero.
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
/// `Session::log_out` drops the socket and sets `active = None`, and every
/// system that reconciles the world returns early when there is no active
/// session, so without this system a logout despawns nothing. The terrain kept
/// streaming around the last position, every entity kept its skeleton posed
/// sixty times a second, the emitters kept simulating, and the collision hulls
/// stayed keyed to a map nobody was on, all behind the character-select
/// screen.
///
/// That was not a permanent leak: logging back in reconciles the entity index
/// against the new session's own answers and despawns the strays on the first
/// poll. The cost was the whole per-frame cost of a city for as long as the
/// client sat at the character screen, and a `Solids` keyed to the old map
/// until the next `enter_world_blocking` cleared it.
///
/// ## The condition that fires the teardown
///
/// The teardown was first keyed on `WorldStatus::in_world` outliving
/// `Session::active`. That state does not always occur on a logout, and when
/// it does not, the teardown never runs.
///
/// `in_world` is a copy, written by `poll_world` from the session thread's own
/// status. The session thread sets its own copy false when it stops, which on
/// a logout is microseconds after it queues the `SMSG_LOGOUT_COMPLETE` event
/// that the client has not read yet. The ordinary frame order is therefore:
///
/// ```text
/// poll_world      active = Some, thread stopped -> status.in_world = false
/// logout::answer  Logout::Complete              -> active = None
/// leave_world     active is None, in_world is already false -> returns
/// ```
///
/// Nothing fires the teardown after that. The tiles stay, `LoadedTiles` keeps
/// the map and the centre it will never ask for again, the hulls stay keyed to
/// the map just left, and the caches are never told the world went.
/// `WmoCache::forget_failures` is never called, so a building that failed to
/// read once cannot be read again for the life of the process. On an ordinary
/// map none of that is visible, because the ground and the city are still
/// spawned from the session before and a relog lands back on them. On a map
/// whose whole geometry is one building it is visible: `globalwmo`'s own map
/// test despawns the host whatever this system does, so the world goes and the
/// bookkeeping that would rebuild it does not.
///
/// Neither of the two systems is ordered against the other, so Bevy's executor
/// chooses which runs first, and whether the teardown ran depended on that
/// choice.
///
/// [`ended`] therefore takes both signals as evidence that there was a world,
/// and either is enough. `in_world` is kept because the harness can set it and
/// because it is still true on the ordinary path. The session handle is added
/// because it survives `poll_world` writing the other.
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
    // The routes are built from `DisplayTables`' own taxi tables, and a new
    // session may open a different archive chain for those. There are nine
    // entries, so the reset is for correctness, not memory. See
    // [`crate::world::entities::transport::ShipRoutes`].
    mut ship_routes: ResMut<crate::world::entities::transport::ShipRoutes>,
    solids: Res<Solids>,
    mut focus: ResMut<crate::render::focus::WorldFocus>,
    tiles: Query<Entity, With<TerrainTile>>,
    entities: Query<Entity, With<WorldEntity>>,
) {
    if !ended(session.active.is_some(), status.in_world, &mut had_world) {
        return;
    }
    // Stop the streaming passes. They run on the focus alone, and
    // `follow_the_session` writes it only while there is a session, so
    // without this the focus keeps the last position and the tiles despawned
    // below are read again on the next frame, behind the character screen.
    // The next login then clears the collision hulls of those tiles (see
    // `enter_world_blocking`), and a tile that is already resident never
    // places its hulls a second time: the character walks through every
    // building and doodad on it, and holds its altitude inside every building's
    // box, because the mover waits for a hull that is not coming.
    focus.present = false;
    // The ground and, as its children, every doodad, building, room and piece
    // of furniture on it, plus the residents parked against each tile.
    let mut dropped = 0;
    for tile in &tiles {
        commands.entity(tile).despawn();
        dropped += 1;
    }
    tiles_held.forget();
    ship_routes.reset();

    // The units and objects and, as children of each root, their joints, their
    // attachments and their shadows.
    let mut units = 0;
    for entity in &entities {
        commands.entity(entity).despawn();
        units += 1;
    }
    index.0.clear();
    motion.reset();

    // A hull keyed to the map just left blocks movement where nothing is
    // drawn. `enter_world_blocking` also clears these, on the way in. Clearing
    // them here as well keeps a logged-out client from holding a city's
    // triangles for as long as the character screen is open.
    solids.0.clear();

    // The caches drop what they were holding for the world on the next sweep,
    // not on this frame. See [`Residency::left_at`] for the reason.
    residency.left_at = Some(time.elapsed_secs());

    // Forget which buildings failed to read. That set is the one cache entry
    // that is never evicted, so it would otherwise outlive the session. See
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

/// Drop every terrain tile that is resident on the frame a session begins.
///
/// `enter_world_blocking` clears the collision hulls before the session
/// thread starts. A tile places its hulls once, when its buildings and doodads
/// spawn, so a tile that was already resident at that moment would keep its
/// picture and have nothing solid in it for the rest of the session.
/// [`leave_world`] stops the streaming when a session ends, which leaves no
/// tile resident here in the ordinary case; this covers a host that was
/// drawing a map of its own when the character logged in. A despawned tile is
/// read again by the streamer and places its hulls then.
pub fn enter_world(
    mut commands: Commands,
    session: Res<Session>,
    mut had_session: Local<bool>,
    mut tiles_held: ResMut<LoadedTiles>,
    tiles: Query<Entity, With<TerrainTile>>,
) {
    if !began(session.active.is_some(), &mut had_session) {
        return;
    }
    let mut dropped = 0;
    for tile in &tiles {
        commands.entity(tile).despawn();
        dropped += 1;
    }
    if dropped > 0 {
        tiles_held.forget();
        info!("entered the world: {dropped} tiles from before the login dropped, to be read again");
    }
}

/// Whether this is the first frame of a session: there is one now and there
/// was none on the last frame.
fn began(alive: bool, had: &mut bool) -> bool {
    let first = alive && !*had;
    *had = alive;
    first
}

/// Whether this is the frame on which the world ended. [`leave_world`] runs
/// its teardown when this returns true. The result is a function of two
/// signals and the previous frame's state.
///
/// It is a free function for the same reason `interface::left_world` and
/// `glue::loading::reckon` are: an [`crate::world::session::ActiveSession`]
/// owns a socket and cannot be constructed in a test, so a rule written inside
/// the system would be the one part of the teardown that no test checks. The
/// rule can be wrong in both directions. Firing every frame despawns the glue
/// screen's own scene; never firing is the failure described in
/// [`leave_world`]'s doc.
///
/// `alive` is whether there is a session now. `in_world` is `poll_world`'s
/// copy of the session thread's status, which can go false a frame or more
/// before the session does. Either is evidence that there was a world on the
/// previous frame, and the teardown needs only one of them. `*had` carries
/// that evidence across the frame where `in_world` has already gone false and
/// `alive` has not yet.
fn ended(alive: bool, in_world: bool, had: &mut bool) -> bool {
    let there_was_a_world = *had || in_world;
    *had = alive;
    !alive && there_was_a_world
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session begins once: on the first frame it exists, and again only
    /// after a frame without one.
    #[test]
    fn a_session_begins_on_its_first_frame_only() {
        let mut had = false;
        assert!(!began(false, &mut had));
        assert!(began(true, &mut had));
        assert!(!began(true, &mut had));
        assert!(!began(false, &mut had));
        assert!(began(true, &mut had), "a second login");
    }

    /// A backend with no allocator report is shown as "gpu not reported", not
    /// as zero. This is the one number on the line that is not a count of
    /// something the client itself holds. `0 MiB gpu` beside eleven thousand
    /// meshes would be read as a valid measurement; `MaterialPool`'s counters
    /// were written to avoid the same failure.
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

    /// The teardown fires when the session goes, even though `in_world` fell
    /// first. This is the frame order described in [`leave_world`]'s doc, which
    /// made a logout leave the world's bookkeeping in place.
    ///
    /// The three frames are the ones the client runs: a session in the world;
    /// a session whose thread has already stopped, so `poll_world` has copied
    /// `in_world = false` while `Session::active` is still `Some`; and the
    /// frame after, where `logout::answer` has taken the session. The old rule
    /// looked only at the second signal and therefore never fired.
    #[test]
    fn a_world_whose_session_outlives_its_in_world_flag_is_still_torn_down() {
        let mut had = false;
        assert!(!ended(true, true, &mut had), "in the world");
        assert!(!ended(true, false, &mut had), "the thread stopped first");
        assert!(ended(false, false, &mut had), "…and the session went after it");
        // It fires once, not once a frame for as long as the character screen
        // is open. A second firing would despawn entities that are already
        // gone.
        assert!(!ended(false, false, &mut had));
    }

    /// The ordinary order still works, and a client that has never logged in
    /// is never told it has left.
    #[test]
    fn the_teardown_fires_once_on_the_way_out_and_never_before_the_way_in() {
        let mut had = false;
        // Nothing has happened yet: no session, no world.
        assert!(!ended(false, false, &mut had));
        assert!(!ended(true, false, &mut had), "entering, before the first poll");
        assert!(!ended(true, true, &mut had));
        // `Session::active` goes first here. The old rule was written for this
        // order, and it has to keep working.
        assert!(ended(false, true, &mut had));
        assert!(!ended(false, false, &mut had));
    }

    /// The sweep interval has to be well under the idle window, or an entry
    /// lives for nearly `IDLE_SECS + SWEEP_SECS` and `IDLE_SECS` no longer
    /// states the window. Tuning either constant can break this.
    #[test]
    fn the_sweep_runs_several_times_inside_the_idle_window() {
        assert!(
            SWEEP_SECS * 4.0 <= IDLE_SECS,
            "a {SWEEP_SECS}s sweep is too coarse for a {IDLE_SECS}s window"
        );
    }

    /// Leaving the world despawns the ground and the entities and forgets the
    /// tiles, so no city stays spawned behind the character screen. This is as
    /// much of the teardown as a test can reach without a login.
    ///
    /// `Session::active` is `None` in a headless app, so a fresh `WorldStatus`
    /// with `in_world` set is the state under test: `in_world` outliving the
    /// session.
    #[test]
    fn leaving_the_world_takes_the_world_with_it() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<Session>()
            .init_resource::<Residency>()
            .init_resource::<WorldStatus>()
            .init_resource::<LoadedTiles>()
            // The building cache, whose `failed` set the teardown forgets. See
            // [`WmoCache::forget_failures`].
            .init_resource::<WmoCache>()
            .init_resource::<EntityIndex>()
            .init_resource::<Motion>()
            // The transport routes, which the teardown resets for the same
            // reason it forgets the building cache's failures.
            .init_resource::<crate::world::entities::transport::ShipRoutes>()
            .init_resource::<Solids>()
            .init_resource::<crate::render::focus::WorldFocus>()
            .add_systems(Update, leave_world);
        app.world_mut()
            .resource_mut::<crate::render::focus::WorldFocus>()
            .present = true;

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
        // The streaming passes run on the focus alone. Left present, they
        // read the same tiles again behind the character screen, and the next
        // login clears the hulls from under them.
        assert!(
            !app.world().resource::<crate::render::focus::WorldFocus>().present,
            "the streamers were left running"
        );
        // The caches are told to drop what they were holding for the world, so
        // they do not keep a city's meshes for `IDLE_SECS` behind the login
        // screen. See [`Residency::left_at`].
        assert!(
            app.world().resource::<Residency>().left_at.is_some(),
            "the sweep was not told the world had gone"
        );
        // The set of buildings that failed to read is evicted by nothing else.
        // A failed building is remembered so that its placements stop asking
        // for it. Remembered across a session, it can never be read again,
        // which on a map whose whole geometry is one building is an empty room
        // with no floor for the rest of the process.
        assert_eq!(
            app.world_mut().resource_mut::<WmoCache>().forget_failures(),
            0,
            "the failures were already forgotten on the way out"
        );

        // The teardown does not fire a second time. That would despawn entities
        // that no longer exist: Bevy's B0003 warning, once a frame, for as long
        // as the character screen is open.
        app.update();
    }
}
