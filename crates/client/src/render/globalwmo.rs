//! The maps that are one building and no ground: every 1.12 map whose `WDT`
//! carries a `MWMO`/`MODF` pair instead of a tile grid.
//!
//! Twenty of the game's forty-three maps are like this, among them the
//! Stockade, Uldaman, Blackrock Depths, the Deeprun Tram and Dire Maul. They
//! have no ADT, so the terrain streamer asks for nine files that are not there
//! and draws nothing. This module draws the building.
//!
//! ## The host is a tile with no ground
//!
//! Everything a building needs after "here is a `PlacedModel`" is keyed off a
//! [`crate::render::terrain::TerrainTile`]: [`crate::render::wmos::spawn_wmos`] reads its
//! `PendingWmos`, hands the `MODD` furniture to the doodad pass through its
//! `PendingDoodads`, and `retire_colliders` keeps the hulls for as long as the
//! tile stands. So the host spawned here is a `TerrainTile`: one entity with
//! the four components a streamed tile carries (`PendingWmos`,
//! `PendingDoodads`, `TileDoodads`, `ResidentDoodads`), and none of the ground,
//! water, atlas or foliage, which a map with no `MCNK` cannot have.
//!
//! Its coordinate is [`HOST_TILE`], which is a key and not a place:
//! `CollisionWorld` buckets hulls by tile and iterates every bucket, so the
//! number has to be stable and unique to this host, not correct.
//!
//! ## The terrain pass leaves it alone
//!
//! `request_tiles` despawns any `TerrainTile` outside the 3x3 it wants, and
//! the Deeprun Tram is 2.5 km long. The host carries
//! [`crate::render::terrain::HeldTile`], which that query excludes, so it is
//! not culled, and this pass owns the host's lifetime.
//!
//! The exception is leaving the world: `residency::leave_world` despawns
//! everything `With<TerrainTile>` when the session ends, host included.
//!
//! ## A host that goes is put back
//!
//! A host can be despawned without the map changing: by `leave_world`, or by a
//! host application that streams [`HOST_TILE`] again after changing the
//! building's files. This pass checks every frame that its host is still
//! there, and spawns it again from the placements it kept, which costs a
//! `spawn` and no archive read.
//!
//! The placements themselves are read again when a
//! [`crate::assets::PathsChanged`] names the map's `WDT`, or says every path
//! may have changed: a host that points the global placement at another
//! building changes the `WDT`, and the kept placements would still name the
//! old one.

use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};

use vale_assets::{wdt_path, world::adt::PlacedModel, world::wdt::Wdt};

use crate::assets::GameAssets;

/// The tile coordinate the host is filed under: a bucket key, not a position
/// (see the module doc). `(64, 64)` is off the end of a map's 64x64 grid, so
/// it cannot collide with a streamed tile. vmangos' extractor marks the same
/// record with a coordinate past the end of the grid for the same reason
/// (`WMOInstance(..., 65, 65, ...)`).
const HOST_TILE: (u32, u32) = (64, 64);

/// The host entity, so this pass can find it.
#[derive(Component)]
pub(crate) struct GlobalWmo;

/// What has been built, and for which map.
#[derive(Resource, Default)]
pub(crate) struct GlobalBuilding {
    /// The `Map.dbc` directory the host was built for: the same string
    /// [`WorldStatus::map_name`] carries, and `None` outside the world.
    map: Option<String>,
    /// The `WDT` read, off the main thread: 33 KB from a newly opened archive
    /// chain is too slow for the frame a teleport lands.
    reading: Option<Task<Option<Vec<PlacedModel>>>>,
    /// What the read answered, kept after the host is spawned, so that a host
    /// despawned by something else can be spawned again without reading the
    /// archives. See [`stream_global_wmo`]. The twenty maps this applies to
    /// state one placement each.
    placements: Vec<PlacedModel>,
    /// When the next read is due after one failed.
    ///
    /// `read_global` opens an archive chain on the compute pool, and it is most
    /// likely to fail at a login, when the session thread's `MapTerrain::open`,
    /// the tile loader and the WMO loader open the same sixteen files. A failed
    /// read is retried after [`RETRY_SECS`] rather than on the next frame.
    retry_at: Option<f32>,
    /// How many reads have failed for this map. The retry is bounded by
    /// [`MAX_READ_ATTEMPTS`], because [`Self::settling`] counts a queued retry
    /// as outstanding work, and an unbounded retry against a `WDT` that will
    /// not read holds the loading screen up for ever.
    attempts: u8,
}

impl GlobalBuilding {
    /// How much of this map's own geometry has not arrived, in the units
    /// [`crate::render::terrain::LoadedTiles::settling`] reports.
    ///
    /// Read by [`crate::glue::loading`]. Without it, the loading screen on a
    /// WMO-only map comes down as soon as the nine absent ADTs have failed to
    /// read, several seconds before the building is drawn.
    pub fn settling(&self) -> usize {
        usize::from(self.reading.is_some() || self.retry_at.is_some())
    }
}

pub(crate) struct GlobalWmoPlugin;

impl Plugin for GlobalWmoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GlobalBuilding>()
            .add_message::<crate::assets::PathsChanged>()
            // Before the building pass, so a host spawned this frame has its
            // `PendingWmos` drained on the same frame. After the teardown,
            // which also despawns this host: when both queue a despawn of it on
            // the frame a session ends, this pass's commands are applied last,
            // so its `try_despawn` is the one that finds nothing.
            .add_systems(
                Update,
                stream_global_wmo
                    .before(crate::render::wmos::spawn_wmos)
                    .after(crate::render::residency::leave_world),
            );
    }
}

/// Keep the host in step with the map the character is on.
///
/// There are three states: no map (outside the world), a map being read, and
/// a map whose answer has landed. A map change, or a change to the map's
/// `WDT`, resets to the first.
fn stream_global_wmo(
    focus: Res<crate::render::focus::WorldFocus>,
    assets: Res<GameAssets>,
    time: Res<Time>,
    mut changed: MessageReader<crate::assets::PathsChanged>,
    mut state: ResMut<GlobalBuilding>,
    mut commands: Commands,
    hosts: Query<Entity, With<GlobalWmo>>,
) {
    let wanted = focus.present.then(|| focus.map_name.clone());
    let now = time.elapsed_secs();

    let wdt = state.map.as_deref().map(wdt_path);
    let wdt_changed = changed.read().fold(false, |any, change| {
        any || change.all || wdt.as_deref().is_some_and(|wdt| change.paths.iter().any(|path| path.eq_ignore_ascii_case(wdt)))
    });
    if wdt_changed {
        state.map = None;
    }

    if state.map != wanted {
        // `try_despawn`, because `residency::leave_world` also despawns the
        // host and runs in the same schedule: on the frame a session ends both
        // can have it queued, and a plain `despawn` of an entity already
        // queued is a Bevy warning on every logout.
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

    // The host was despawned by something else: `leave_world`, or a host
    // application streaming `HOST_TILE` again. Spawn it again from the kept
    // placements.
    if !state.placements.is_empty() && hosts.is_empty() {
        debug!("global WMO: the host for {map} has gone; spawning it again");
        spawn_host(&mut commands, &map, state.placements.clone());
        return;
    }

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
    // [`read_global`] answers `Some(vec![])` for a map it read that has no
    // global placement, which is every ordinary map, and `None` for a read
    // that did not happen, which is a transient archive failure and is
    // retried. Taking the second as the first would leave a dungeon empty for
    // the session.
    let Some(placements) = finished else {
        state.attempts = state.attempts.saturating_add(1);
        if state.attempts >= MAX_READ_ATTEMPTS {
            // Nothing else reports that a map has no geometry: the terrain
            // streamer is correct for a map with no ADTs, so the only symptom
            // is an empty room. The loading screen is let down rather than
            // hanging.
            error!(
                "global WMO: {map}'s WDT would not read in {MAX_READ_ATTEMPTS} attempts; \
                 this map will have no geometry until the next login"
            );
            return;
        }
        warn!(
            "global WMO: could not read {map}'s WDT (attempt {}); retrying in {RETRY_SECS}s",
            state.attempts
        );
        state.retry_at = Some(now + RETRY_SECS);
        return;
    };
    if placements.is_empty() {
        return;
    }
    info!(
        "global WMO: {} placement(s) for {map}: {}",
        placements.len(),
        placements[0].path,
    );
    state.placements = placements.clone();
    spawn_host(&mut commands, &map, placements);
}

/// How long a failed `WDT` read waits before the next. Each attempt opens an
/// archive chain, so the retry is not every frame; two seconds is short enough
/// that a character in a dungeon is not left in an empty room for long.
const RETRY_SECS: f32 = 2.0;

/// How many times the `WDT` is read before the map is given up on.
///
/// The failure retried is a contended archive open on the frame a session
/// starts, which is over in well under a second, so three attempts across four
/// seconds clear it or it will not clear. Bounded because
/// [`GlobalBuilding::settling`] counts a queued retry.
const MAX_READ_ATTEMPTS: u8 = 3;

/// The read, off the loader pool.
fn read_task(assets: &GameAssets, map: &str) -> Task<Option<Vec<PlacedModel>>> {
    let dir = assets.gamedata_dir.clone();
    let map = map.to_string();
    // The overlay is taken with the folder name, as `terrain::read_tile` takes
    // it: a chain opened without it answers the archives' bytes for a path a
    // host overrides. The chain is borrowed from the pool, as there.
    let overlay = assets.overlay();
    let chains = assets.chains();
    AsyncComputeTaskPool::get().spawn(async move { read_global(&chains, &dir, &map, overlay) })
}

/// Spawn the host. See the module doc for why it is a `TerrainTile`.
fn spawn_host(commands: &mut Commands, map: &str, placements: Vec<PlacedModel>) {
    debug!("global WMO: hosting {} placement(s) for {map}", placements.len());
    let host = crate::render::wmos::spawn_held(commands, HOST_TILE, placements);
    commands.entity(host).insert(GlobalWmo);
}

/// Read the `WDT`'s global placements, off the loader pool.
///
/// `None` means the read did not happen; `Some(vec![])` means it did and the
/// map states no global building. The caller retries the first and accepts
/// the second.
///
/// The chain is borrowed from [`vale_assets::archive::ChainPool`] rather than
/// shared with [`GameAssets`], as `terrain::read_tile` does: the shared handle
/// is behind a mutex the main thread takes several times a frame.
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
    // `placed_global_wmos` and not `is_wmo_only`: a map with both a tile grid
    // and a global `MODF` hosts its building too.
    Some(wdt.placed_global_wmos())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host's coordinate cannot be a real tile's. If it could, the terrain
    /// streamer's 3x3 could want it, and a streamed tile would share the host's
    /// collision bucket.
    #[test]
    fn the_host_tile_is_off_the_grid() {
        assert!(HOST_TILE.0 as usize >= vale_assets::world::wdt::MAP_TILES);
        assert!(HOST_TILE.1 as usize >= vale_assets::world::wdt::MAP_TILES);
    }

    /// A pass with nothing to read reports nothing outstanding. Every outdoor
    /// map and every frame before login is in this state, and any other answer
    /// would hold the loading screen up for ever.
    #[test]
    fn a_pass_with_nothing_to_read_holds_the_loading_screen_up_for_nothing() {
        assert_eq!(GlobalBuilding::default().settling(), 0);
    }

    /// A queued retry is outstanding work, so the screen does not come down in
    /// the two seconds between a failed read and the next; a map given up on
    /// is not, so a `WDT` that will never read lets the screen down.
    #[test]
    fn a_queued_retry_holds_the_loading_screen_and_a_dead_map_lets_it_down() {
        let mut state = GlobalBuilding {
            map: Some("uldaman".to_string()),
            retry_at: Some(12.0),
            attempts: 1,
            ..Default::default()
        };
        assert_eq!(state.settling(), 1, "a retry is still work outstanding");
        // Given up on: no read and no retry queued, whatever `attempts` says.
        state.retry_at = None;
        state.attempts = MAX_READ_ATTEMPTS;
        assert_eq!(state.settling(), 0);
    }

    /// The read is attempted at least twice, since the failure it survives is
    /// a single contended archive open on the frame a session starts.
    #[test]
    fn the_read_is_retried_rather_than_merely_attempted() {
        assert!(MAX_READ_ATTEMPTS >= 2);
        assert!(RETRY_SECS > 0.0);
    }
}
