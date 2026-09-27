//! **The maps that are one building and no ground** — every 1.12 instance whose
//! `WDT` carries a `MWMO`/`MODF` pair instead of a tile grid.
//!
//! Twenty of the game's forty-three maps are like this: the Stockade, Uldaman,
//! Blackrock Depths, the Deeprun Tram, Dire Maul. They have **no ADT at all**,
//! so the terrain streamer asks for nine files that are not there, gets nothing
//! back nine times, and the character arrives in an empty world with the
//! server's creatures hanging in it. That was the report this module answers.
//!
//! ## It is a tile with no ground on it
//!
//! Everything a building needs downstream of "here is a `PlacedModel`" already
//! exists and is keyed off a [`TerrainTile`]: [`crate::render::wmos::spawn_wmos`]
//! reads its `PendingWmos`, hands the `MODD` furniture to the doodad pass
//! through its `PendingDoodads`, and `retire_colliders` keeps the hulls alive
//! for as long as the tile does. So the host spawned here **is** a
//! `TerrainTile` — one entity, carrying the four components a streamed tile
//! carries beside it (`PendingWmos`, `PendingDoodads`, `TileDoodads`,
//! `ResidentDoodads`), minus the ground, the water, the atlas and the foliage
//! a map with no `MCNK` cannot have.
//!
//! The coordinate on it is [`HOST_TILE`] and it is a **key rather than a
//! place**: `CollisionWorld` buckets hulls by tile and iterates every bucket, so
//! what the number has to be is stable and unique to this host, not correct.
//!
//! ## …which is why the terrain pass has to be told to leave it alone
//!
//! `request_tiles` despawns any `TerrainTile` outside the 3x3 it wants, and the
//! Deeprun Tram is 2.5 km long — the host would be culled the moment the
//! character walked away from whatever coordinate it was given. Its tile query
//! carries `Without<GlobalWmo>` for that, and this pass owns the host's whole
//! lifetime instead.
//!
//! The one thing it does **not** own is the way out: `residency::leave_world`
//! despawns everything `With<TerrainTile>` when the session ends, host
//! included, which is right — a building left standing after a logout is the
//! same bug as a tile left standing.
//!
//! **So this pass checks that its host is still there, and that is a fix
//! rather than a description.** This paragraph used to claim it "notices the
//! entity has gone rather than assuming it is still there" and the code did no
//! such thing: the whole system was a comparison of the map name against the
//! one it built for, so a host despawned on any frame where that name did not
//! also change was gone for good — an empty room, no collision under it, and
//! nothing that ever looked again. The placements are kept for that reason, so
//! putting it back costs a `spawn` rather than sixteen archive opens.

use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};

use vale_assets::{wdt_path, world::adt::PlacedModel, world::wdt::Wdt};

use crate::assets::GameAssets;
use crate::render::terrain::TerrainTile;

/// **The tile coordinate the host is filed under.** See the module doc: a
/// bucket key, not a position. `(64, 64)` is off the end of a map's 64x64 grid,
/// so it can never collide with a streamed tile — and it is vmangos' own choice
/// for the same record (`WMOInstance(..., 65, 65, ...)` marks a world spawn in
/// its extractor, one past the end for the same reason).
const HOST_TILE: (u32, u32) = (64, 64);

/// The host entity, so the terrain streamer can exclude it and so this pass can
/// find its own work again.
#[derive(Component)]
pub struct GlobalWmo;

/// What has been built, and for which map.
#[derive(Resource, Default)]
pub struct GlobalBuilding {
    /// The `Map.dbc` directory the host was built for — the same string
    /// [`WorldStatus::map_name`] carries, and `None` outside the world.
    map: Option<String>,
    /// The `WDT` read, which is 33 KB off a freshly opened archive chain and
    /// therefore not something to do on the frame a teleport lands.
    reading: Option<Task<Option<Vec<PlacedModel>>>>,
    /// **What the read said, kept after the host is spawned.**
    ///
    /// So that a host which goes away for a reason this pass did not cause can
    /// be put back without re-opening sixteen archives — see [`stream_global_wmo`],
    /// where the case that needs it is. A dozen strings and a matrix each; the
    /// twenty maps this applies to state one placement apiece.
    placements: Vec<PlacedModel>,
    /// **When the last read failed, so the next one is not this frame.**
    ///
    /// `read_global` opens its own archive chain on the compute pool, and the
    /// moment it is most likely to fail is exactly the moment it runs: a login,
    /// with the session thread's `MapTerrain::open`, the tile loader and the
    /// WMO loader all opening the same sixteen files. A failure used to be
    /// **permanent for the session** — `reading` was cleared with `map` left
    /// set, so the pass went inert and nothing but a logout could reset it,
    /// which is the shape of the report this answers. Retried, but not spun on.
    retry_at: Option<f32>,
    /// …and how many times it has failed for this map, because the retry has to
    /// be **bounded**.
    ///
    /// [`Self::settling`] counts a queued retry as outstanding work, which is
    /// right — the loading screen must not come down on the empty room the
    /// retry is about to fill — and would hold that screen up for ever against
    /// a map whose `WDT` genuinely will not read. See [`MAX_READ_ATTEMPTS`].
    attempts: u8,
}

impl GlobalBuilding {
    /// **How much of this map's own geometry has not arrived yet**, in the same
    /// currency [`crate::render::terrain::LoadedTiles::settling`] reports.
    ///
    /// Read by [`crate::glue::loading`]. Without it the loading screen
    /// comes down on a WMO-only map the instant the nine absent ADTs have
    /// failed to read — which is immediately, and several seconds before there
    /// is anything to look at.
    pub fn settling(&self) -> usize {
        usize::from(self.reading.is_some() || self.retry_at.is_some())
    }
}

pub struct GlobalWmoPlugin;

impl Plugin for GlobalWmoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GlobalBuilding>()
            // **Before the building pass**, so a host spawned this frame has
            // its `PendingWmos` drained on the same frame rather than the next
            // one. Stated rather than left to the two systems' disjoint access,
            // which is not a sequence at all.
            .add_systems(
                Update,
                stream_global_wmo
                    .before(crate::render::wmos::spawn_wmos)
                    // **…and after the teardown**, which is the one other
                    // thing that despawns this host. Both can queue a despawn
                    // of it on the frame a session ends; running second means
                    // the queue this pass adds to is applied last, so the
                    // `try_despawn` below is the one that finds nothing rather
                    // than `leave_world`'s plain one.
                    .after(crate::render::residency::leave_world),
            );
    }
}

/// Keep the host in step with the map the character is on.
///
/// Three states and nothing else: no map (outside the world), a map being read,
/// and a map whose answer has landed. A map change resets to the first.
fn stream_global_wmo(
    focus: Res<crate::render::focus::WorldFocus>,
    assets: Res<GameAssets>,
    time: Res<Time>,
    mut state: ResMut<GlobalBuilding>,
    mut commands: Commands,
    hosts: Query<Entity, With<GlobalWmo>>,
) {
    let wanted = focus.present.then(|| focus.map_name.clone());
    let now = time.elapsed_secs();

    if state.map != wanted {
        // **`try_despawn`, because this is not the only thing that despawns
        // it.** `residency::leave_world` takes every `TerrainTile` on the way
        // out and runs in the same schedule, so on the frame a session ends
        // both can have the host queued — and a plain `despawn` of an entity
        // already queued for despawn is a Bevy warning on every logout.
        for host in &hosts {
            commands.entity(host).try_despawn();
        }
        state.reading = None;
        state.placements.clear();
        state.retry_at = None;
        state.attempts = 0;
        state.map = wanted.clone();
        if let Some(map) = wanted {
            state.reading = Some(read_task(&assets, &map));
        }
        return;
    }
    let Some(map) = state.map.clone() else {
        return;
    };

    // **The host can go without this pass having asked**, and until this
    // existed the module doc claimed it was noticed when it was not:
    // `residency::leave_world` despawns every `TerrainTile`, host included, and
    // any frame on which that happens without `wanted` also changing leaves the
    // map permanently empty — no building, no collision under it, and nothing
    // that ever looks again. What it costs to check is one empty query.
    if !state.placements.is_empty() && hosts.is_empty() {
        warn!("global WMO: the host for {map} has gone — rebuilding it");
        spawn_host(&mut commands, &map, state.placements.clone());
        return;
    }

    // …and a read that failed is retried rather than being the end of the map.
    if let Some(at) = state.retry_at {
        if now >= at && state.reading.is_none() {
            state.retry_at = None;
            state.reading = Some(read_task(&assets, &map));
        }
        return;
    }

    let Some(task) = state.reading.as_mut() else {
        return;
    };
    let Some(finished) = block_on(future::poll_once(task)) else {
        return;
    };
    state.reading = None;
    // **A map with terrain and a `WDT` that will not read are the same answer
    // here and must not be treated the same way.** The first is every ordinary
    // map and is correct; the second is a transient failure of an archive open
    // on a busy compute pool, and taking it as "this map has no building"
    // blanks a dungeon for the whole session.
    //
    // They are told apart by [`read_global`], which answers `Some(vec![])` for a
    // map it read and that has no global placement, and `None` for a read that
    // did not happen.
    let Some(placements) = finished else {
        state.attempts = state.attempts.saturating_add(1);
        if state.attempts >= MAX_READ_ATTEMPTS {
            // **Given up on, and loudly.** Nothing else in the client will say
            // that a map has no geometry: the terrain streamer is already
            // correct for a map with no ADTs, so the only symptom is an empty
            // room, and the loading screen has to be allowed down or the client
            // hangs at it.
            error!(
                "global WMO: {map}'s WDT would not read in {MAX_READ_ATTEMPTS} attempts — \
                 this map will have no geometry until the next login"
            );
            return;
        }
        warn!(
            "global WMO: could not read {map}'s WDT (attempt {}) — retrying in {RETRY_SECS}s",
            state.attempts
        );
        state.retry_at = Some(now + RETRY_SECS);
        return;
    };
    if placements.is_empty() {
        return;
    }
    info!(
        "global WMO: {} placement(s) for {map} — {}",
        placements.len(),
        placements[0].path,
    );
    state.placements = placements.clone();
    spawn_host(&mut commands, &map, placements);
}

/// **How long a failed `WDT` read waits before being asked again.**
///
/// Long enough not to be a spin on a map that genuinely will not read — which
/// costs an archive-chain open each time — and short enough that a character
/// standing in a dungeon is not left in an empty room for a noticeable time.
const RETRY_SECS: f32 = 2.0;

/// …and how many times it is asked at all.
///
/// The failure this retry exists for is a **contended** archive open — several
/// threads opening the same sixteen files on the frame a session starts — which
/// is over in well under a second, so three attempts across four seconds either
/// clears it or it was never going to. Bounded because [`GlobalBuilding::settling`]
/// counts a queued retry, and an unbounded one is a loading screen that never
/// comes down.
const MAX_READ_ATTEMPTS: u8 = 3;

/// The read, off the loader pool.
fn read_task(assets: &GameAssets, map: &str) -> Task<Option<Vec<PlacedModel>>> {
    let dir = assets.gamedata_dir.clone();
    let map = map.to_string();
    // The overlay travels with the folder name, on `terrain::read_tile`'s own
    // terms: a chain opened without it answers the archives' bytes for a path a
    // host is overriding.
    let overlay = assets.overlay();
    // Borrowed rather than opened, on `terrain::read_tile`'s own terms.
    let chains = assets.chains();
    AsyncComputeTaskPool::get().spawn(async move { read_global(&chains, &dir, &map, overlay) })
}

/// The host itself — see the module doc on why it is a `TerrainTile`.
fn spawn_host(commands: &mut Commands, map: &str, placements: Vec<PlacedModel>) {
    debug!("global WMO: hosting {} placement(s) for {map}", placements.len());
    commands.spawn((
        TerrainTile { coord: HOST_TILE },
        GlobalWmo,
        crate::render::wmos::PendingWmos(placements),
        // Empty on both counts and both needed: the outdoor list is what an
        // `MDDF` fills and there is no ADT to fill it, and the indoor one is
        // what `spawn_wmos` appends the building's own `MODD` furniture to.
        crate::render::doodads::PendingDoodads::default(),
        crate::render::doodads::TileDoodads::default(),
        crate::render::doodads::ResidentDoodads::default(),
        Transform::default(),
        Visibility::default(),
    ));
}

/// The `WDT`, off the loader pool.
///
/// **`None` means the read did not happen and `Some(vec![])` means it did and
/// the map states no global building** — and the caller acts on the difference,
/// which is why they are not folded together. An ordinary outdoor map is the
/// second; an archive chain that would not open on a busy frame is the first,
/// and treating it as "this map has no building" is a dungeon blanked for the
/// rest of the session.
///
/// Deliberately reads on a chain of its own rather than sharing
/// [`GameAssets`]' one: this runs on the compute pool, exactly as
/// `terrain::read_tile` does and for the same reason — the shared handle is
/// behind a mutex the main thread holds several times a frame. The chain is
/// borrowed from [`vale_assets::archive::ChainPool`] rather than opened, so
/// a map change costs a `Vec::pop` where it used to cost seventeen archives.
fn read_global(
    chains: &vale_assets::archive::ChainPool,
    gamedata_dir: &str,
    map: &str,
    overlay: Option<vale_assets::archive::Overlay>,
) -> Option<Vec<PlacedModel>> {
    let mut archive = chains.take(gamedata_dir).ok()?;
    archive.set_overlay(overlay);
    let raw = archive.read(&wdt_path(map)).ok()?;
    let wdt = Wdt::parse(&raw).ok()?;
    // **Not `is_wmo_only`**, deliberately. That asks "does this map have *only*
    // a building", which is what `vale maps` reports; what decides whether
    // to host one is whether there is a placement to host, and a map with both
    // a tile grid and a global `MODF` would otherwise silently lose it.
    Some(wdt.placed_global_wmos())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The host's coordinate cannot be a real tile's**, which is the whole
    /// of what stops the terrain streamer's 3x3 from ever wanting it — and what
    /// would silently give a streamed tile the host's colliders if it were not
    /// true.
    #[test]
    fn the_host_tile_is_off_the_grid() {
        assert!(HOST_TILE.0 as usize >= vale_assets::world::wdt::MAP_TILES);
        assert!(HOST_TILE.1 as usize >= vale_assets::world::wdt::MAP_TILES);
    }

    /// **A pass that has not been asked for anything is not outstanding**,
    /// which is the state every outdoor map and every frame before login is in
    /// — and the one that would hold the loading screen up for ever if it
    /// answered anything else. The other side of it needs a task pool and is
    /// covered by the pass running at all.
    #[test]
    fn a_pass_with_nothing_to_read_holds_the_loading_screen_up_for_nothing() {
        assert_eq!(GlobalBuilding::default().settling(), 0);
    }

    /// **A queued retry is outstanding work and a map given up on is not**,
    /// which is the pair the loading screen is written against.
    ///
    /// The first half is the point of counting it at all: without it the screen
    /// comes down in the two seconds between a failed read and the one that
    /// succeeds, and what the player sees is the empty room the retry is about
    /// to fill. The second half is why the retry is bounded — a `WDT` that will
    /// never read must let the screen down rather than hanging the client at
    /// it.
    #[test]
    fn a_queued_retry_holds_the_loading_screen_and_a_dead_map_lets_it_down() {
        let mut state = GlobalBuilding {
            map: Some("uldaman".to_string()),
            retry_at: Some(12.0),
            attempts: 1,
            ..Default::default()
        };
        assert_eq!(state.settling(), 1, "a retry is still work outstanding");
        // Given up on: no read, no retry queued, and therefore nothing to wait
        // for — whatever `attempts` says.
        state.retry_at = None;
        state.attempts = MAX_READ_ATTEMPTS;
        assert_eq!(state.settling(), 0);
    }

    /// **The retry is bounded by a number that is at least two**, because one
    /// attempt is not a retry at all and the whole point is to survive a single
    /// contended archive open on the frame a session starts.
    #[test]
    fn the_read_is_retried_rather_than_merely_attempted() {
        assert!(MAX_READ_ATTEMPTS >= 2);
        assert!(RETRY_SECS > 0.0);
    }
}
