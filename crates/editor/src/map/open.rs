//! Tile streaming for the session: the tiles around the focus read and parsed
//! on the task pool, and the tiles the focus has moved away from closed.

use crate::session::EditSession;
use vale_client::assets::GameAssets;
use bevy::prelude::*;

/// How many tiles are read and parsed on the task pool at once. Bevy's
/// async-compute pool has four threads and the renderer's own tile loads share
/// it, so two keeps the editor's reads from queueing the renderer's behind
/// them.
const OPEN_IN_FLIGHT: usize = 2;

/// The tiles being read and parsed off the main thread for the session. See
/// [`open_tiles`].
#[derive(Resource, Default)]
pub struct Opening {
    tasks: Vec<OpeningTile>,
    /// Claimed tiles that could not be read or parsed, by map, so they are
    /// not asked for again every frame. The reason is in the status line.
    failed: std::collections::HashSet<(String, (u32, u32))>,
}

/// One tile being read.
struct OpeningTile {
    coord: (u32, u32),
    /// The map and the project it was asked for. A tile that lands after
    /// either has changed is dropped.
    map: String,
    project: std::path::PathBuf,
    task: bevy::tasks::Task<Result<vale_edit::adt::AdtFile, String>>,
}

impl Opening {
    /// Whether a tile is being read.
    pub fn is_opening(&self, coord: (u32, u32)) -> bool {
        self.tasks.iter().any(|t| t.coord == coord)
    }
}

/// Close the open tiles the focus has moved away from.
///
/// [`open_tiles`] parses every tile the block reaches. Without this system
/// nothing closes one, so a session that jumps around a map holds every tile
/// it has been near: two megabytes of parsed file each, forty-nine per stop.
/// Memory climbed with every teleport and was only partly released. This
/// system drops the
/// tiles further than the reach plus one from the focus that nothing needs
/// kept: not unsaved, not named by the history (undo writes into an open tile
/// and skips a closed one), and not waiting on any of the session's live sets.
///
/// It keeps one ring past the reach, so a focus moving along a border does
/// not close and re-parse the same row of tiles. Chained after `open_tiles`,
/// and subject to the same two conditions.
pub(crate) fn close_tiles(
    mut session: Option<ResMut<EditSession>>,
    focus: Res<vale_client::render::focus::WorldFocus>,
    reach: Res<vale_client::render::terrain::TerrainReach>,
    state: Res<crate::playtest::Playtest>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if !focus.present || !state.editing() {
        return;
    }
    // `VALE_KEEP_TILES=1` switches this off, so the memory growth it removes
    // can be measured by comparing two `--tour` runs, one with the variable
    // set and one without. The variable is read once.
    static KEEP_EVERYTHING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *KEEP_EVERYTHING.get_or_init(|| std::env::var_os("VALE_KEEP_TILES").is_some()) {
        return;
    }
    let (cx, cy) = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    let keep = reach.tiles() + 1;
    let far: Vec<(u32, u32)> = session
        .tiles
        .keys()
        .copied()
        .filter(|&(x, y)| {
            (x as i32 - cx as i32).abs() > keep || (y as i32 - cy as i32).abs() > keep
        })
        .filter(|coord| !session.unsaved.contains(coord))
        .filter(|coord| {
            !session.stale.contains(coord)
                && !session.restream.contains(coord)
                && !session.dirty.contains_key(coord)
                && !session.regrow.contains_key(coord)
                && !session.moved.contains_key(coord)
                && !session.moved_buildings.contains_key(coord)
                && !session.repaint.contains_key(coord)
        })
        .collect();
    for coord in far {
        session.close(coord);
    }
}

/// Open the tiles around the focus: read and parse each on the task pool, and
/// put it in the session when it is ready.
///
/// It opens the block that is drawn, not only the tile under the pointer,
/// because the pointer cannot pick against a tile that is not open and a
/// stroke near a tile border reaches into the next one. The block is
/// `render::terrain::TerrainReach`'s own square, the same one the streamer
/// puts on screen, so the editable area and the visible area are the same:
/// otherwise a tool would refuse edits on drawn ground the person can see.
///
/// ## Off the main thread
///
/// A tile is a 10 to 35 ms archive read and a 5 to 10 ms parse. This used to
/// open one tile a frame on the main thread, through the main archive chain
/// and under its lock, so every frame of a block filling (49 tiles after a
/// jump) was a long frame. The read is now on a chain borrowed from the pool
/// with the editor's overlay set, as `render::terrain::read_tile`'s is, and
/// the parse runs on the same task; the main thread only moves the finished
/// `AdtFile` into the session.
///
/// Only the tiles the WDT claims are asked for, which is every tile the map
/// has. A claimed tile that fails is remembered in [`Opening::failed`] and not
/// asked for again on that map.
///
/// Tiles are asked for nearest first. A 3x3 reach does not need the ordering;
/// a 7x7 reach does: until the tile under the pointer is open, the pointer
/// reports no ground and the brush does nothing.
pub(crate) fn open_tiles(
    mut session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    focus: Res<vale_client::render::focus::WorldFocus>,
    reach: Res<vale_client::render::terrain::TerrainReach>,
    state: Res<crate::playtest::Playtest>,
    mut opening: ResMut<Opening>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    // The reads that have finished. Taken whether or not the focus is
    // present, so a task never outlives the need for it by more than a frame.
    let mut index = 0;
    while index < opening.tasks.len() {
        let done = bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(
            &mut opening.tasks[index].task,
        ));
        let Some(result) = done else {
            index += 1;
            continue;
        };
        let pending = opening.tasks.swap_remove(index);
        let current = pending.map == session.map && pending.project == session.project.root;
        match result {
            Ok(tile) if current => session.insert_opened(pending.coord, tile),
            Ok(_) => {}
            Err(why) => {
                if current {
                    session.status = why;
                }
                opening.failed.insert((pending.map, pending.coord));
            }
        }
    }

    // Not during a playtest. The focus follows the character then, so this
    // would parse two megabytes a frame along the character's route, for
    // tiles no tool may touch, and hold every one of them for the rest of the
    // session.
    if !focus.present || !state.editing() {
        return;
    }
    // The claims are read for the open map when it opens; until they are,
    // nothing is known to exist.
    if session.claims_for != session.map {
        return;
    }
    let (cx, cy) = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    let radius = reach.tiles();
    let mut wanted: Vec<(i32, (u32, u32))> = Vec::new();
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let (x, y) = (cx as i32 + dx, cy as i32 + dy);
            if !(0..64).contains(&x) || !(0..64).contains(&y) {
                continue;
            }
            let coord = (x as u32, y as u32);
            if session.tiles.contains_key(&coord)
                || !session.wdt_claims(coord)
                || opening.is_opening(coord)
                || opening.failed.contains(&(session.map.clone(), coord))
            {
                continue;
            }
            wanted.push((dx * dx + dy * dy, coord));
        }
    }
    wanted.sort_unstable_by_key(|(distance, _)| *distance);
    // `VALE_OPEN_INLINE=1`: the old path, one tile a frame read and parsed on
    // this thread, kept as a measurement probe so the two can be compared in
    // one binary with `--tour`.
    static INLINE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *INLINE.get_or_init(|| std::env::var_os("VALE_OPEN_INLINE").is_some()) {
        if let Some((_, coord)) = wanted.first() {
            if !session.open(&assets, *coord) {
                opening.failed.insert((session.map.clone(), *coord));
            }
        }
        return;
    }
    for (_, coord) in wanted {
        if opening.tasks.len() >= OPEN_IN_FLIGHT {
            return;
        }
        let chains = assets.chains();
        let dir = assets.gamedata_dir.clone();
        let overlay = assets.overlay();
        let path = session.key(coord).vpath();
        let task = bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
            let mut chain = chains.take(&dir).map_err(|e| format!("{path}: {e}"))?;
            chain.set_overlay(overlay);
            let bytes = chain.read(&path).map_err(|e| format!("{path}: {e}"))?;
            vale_edit::adt::AdtFile::parse(&bytes).map_err(|e| format!("{path}: {e}"))
        });
        opening.tasks.push(OpeningTile {
            coord,
            map: session.map.clone(),
            project: session.project.root.clone(),
            task,
        });
    }
}
