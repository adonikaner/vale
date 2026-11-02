//! What is read from a map's saved tiles, and what is made from it.
//!
//! A [`Scan`] is the area id and the middle of every chunk of every tile the
//! map's WDT lists, read through the project and the archives off the main
//! thread. Edits not yet saved are not in it. From a scan:
//!
//! ```text
//! Fit to map         a rectangle round what a row covers: for a zone, the
//!                    chunks of the zone and its sub-areas; for a continent or
//!                    a single map, every chunk (vale_edit::worldmap::fit_rect)
//! Write zone grid    a continent's Interface\WorldMap\<dir>.zmp, which names
//!                    the zone under the pointer on its picture
//! Draw highlight     a zone's <dir>Highlight.blp from the zone grid made from
//!                    the scan, or a continent's from the tiles it has
//! ```
//!
//! A scan is kept per map until the project's files change.

use std::collections::HashMap;

use bevy::tasks::{block_on, futures_lite::future, Task};

use super::{map_directory, rows, AfterScan, Entry, Kind, WorldMaps};
use crate::jobs::Progress;
use crate::session::EditSession;
use crate::tables::zones::area;
use vale_assets::tables::worldmap::sheet_fraction;
use vale_assets::world::adt::{CHUNK_SIZE, TILE_SIZE};
use vale_assets::world::terrain::tile_corner;
use vale_assets::world::blp;
use vale_client::assets::GameAssets;
use vale_edit::worldmap as pieces;

/// Every chunk of a map: the world x and y of its middle, and its area id;
/// and the tiles that were read.
pub(crate) struct Scan {
    revision: u64,
    pub(crate) chunks: Vec<(f32, f32, u32)>,
    pub(crate) tiles: Vec<(u32, u32)>,
}

/// A scan being made.
pub(crate) struct Running {
    pub(crate) map: u32,
    revision: u64,
    pub(crate) progress: Progress,
    task: Task<Scan>,
}

/// The scans made, by map, and the one being made.
#[derive(Default)]
pub(crate) struct Scans {
    done: HashMap<u32, Scan>,
    pub(crate) running: Option<Running>,
}

impl Scans {
    /// `map`'s scan, while the project's files are as they were when it was
    /// made.
    pub(crate) fn of(&self, map: u32, session: &EditSession) -> Option<&Scan> {
        self.done.get(&map).filter(|scan| scan.revision == session.files_revision)
    }

    /// Start reading `map`'s tiles. Answers `false` when its scan is already
    /// made and `true` when one is now being made.
    pub(crate) fn start(&mut self, map: u32, session: &mut EditSession, assets: &GameAssets) -> Result<bool, String> {
        if self.of(map, session).is_some() {
            return Ok(false);
        }
        if self.running.as_ref().is_some_and(|running| running.map == map) {
            return Ok(true);
        }
        let directory = map_directory(session, assets, map).ok_or_else(|| format!("map {map} is not in Map.dbc"))?;
        let tiles = map_tiles(session, assets, map);
        if tiles.is_empty() {
            return Err(format!("{directory} has no tiles in its WDT"));
        }
        let read = assets.shared_reader();
        let progress = Progress::new(tiles.len());
        let worker = progress.clone();
        let revision = session.files_revision;
        let task = bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
            let mut chunks = Vec::new();
            let mut found = Vec::new();
            for (done, &(x, y)) in tiles.iter().enumerate() {
                let adt = read(&vale_assets::adt_path(&directory, x, y))
                    .and_then(|bytes| vale_assets::world::adt::Adt::parse(&bytes).ok());
                if let Some(adt) = adt {
                    found.push((x, y));
                    // A chunk's position is its corner of greatest x and y.
                    chunks.extend(adt.chunks.iter().map(|chunk| {
                        (chunk.position[0] - CHUNK_SIZE / 2.0, chunk.position[1] - CHUNK_SIZE / 2.0, chunk.area_id)
                    }));
                }
                worker.set(done + 1);
            }
            Scan { revision, chunks, tiles: found }
        });
        self.running = Some(Running { map, revision, progress, task });
        Ok(true)
    }

    /// Keep a finished scan. Answers whether one finished.
    pub(crate) fn collect(&mut self, session: &EditSession) -> bool {
        let Some(running) = self.running.as_mut() else { return false };
        let Some(scan) = block_on(future::poll_once(&mut running.task)) else { return false };
        let (map, revision) = (running.map, running.revision);
        self.running = None;
        if revision == session.files_revision {
            self.done.insert(map, scan);
        }
        true
    }
}

/// The tiles `map`'s WDT lists, as the project and the archives hold it, as
/// (column, row).
pub(crate) fn map_tiles(session: &mut EditSession, assets: &GameAssets, map: u32) -> Vec<(u32, u32)> {
    let Some(directory) = map_directory(session, assets, map) else { return Vec::new() };
    let path = format!("World\\Maps\\{directory}\\{directory}.wdt");
    assets
        .with_archive(|chain| Ok(chain.read(&path).ok()))
        .ok()
        .flatten()
        .and_then(|bytes| vale_edit::wdt::WdtFile::parse(&bytes).ok())
        .map_or_else(Vec::new, |wdt| wdt.tiles())
}

/// The world bounds of `tiles`: least x, least y, most x, most y.
pub(crate) fn tile_bounds(tiles: &[(u32, u32)]) -> [f32; 4] {
    let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    for &(col, row) in tiles {
        let ([north, west], size) = (tile_corner(col, row), TILE_SIZE);
        bounds = [bounds[0].min(north - size), bounds[1].min(west - size), bounds[2].max(north), bounds[3].max(west)];
    }
    bounds
}

/// The zone an area id belongs to: its parent, or itself when it has none.
fn zone_of(session: &EditSession) -> impl Fn(u32) -> u32 + 'static {
    let parents: HashMap<u32, u32> = session
        .table(area::TABLE)
        .map(|table| {
            (0..table.record_count())
                .filter_map(|record| Some((table.u32_at(record, 0)?, table.u32_at(record, area::PARENT)?)))
                .collect()
        })
        .unwrap_or_default();
    move |id| match parents.get(&id) {
        Some(&parent) if parent != 0 => parent,
        _ => id,
    }
}

/// Do what was waiting for the chosen map's scan. Answers what was done.
pub(crate) fn after(maps: &mut WorldMaps, entry: &Entry, session: &mut EditSession, _now: f64) -> String {
    let Some(what) = maps.after_scan.take() else { return String::new() };
    let Some(row) = entry.area.clone() else { return "the whole world has no tiles".to_string() };
    let Some(scan) = maps.scans.of(row.map, session) else { return "the map's tiles were not read".to_string() };
    let zone = zone_of(session);
    match what {
        AfterScan::FitRect => {
            let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
            let mut counted = 0;
            for &(x, y, id) in &scan.chunks {
                if entry.kind == Kind::Zone && zone(id) != row.area {
                    continue;
                }
                counted += 1;
                let half = CHUNK_SIZE / 2.0;
                bounds = [bounds[0].min(x - half), bounds[1].min(y - half), bounds[2].max(x + half), bounds[3].max(y + half)];
            }
            if counted == 0 {
                return format!("no chunk of the map is painted with {}; paint it with the Areas tool first", entry.name);
            }
            rows::set_rect(session, row.id, pieces::fit_rect(bounds), None);
            format!("fitted the rectangle to {counted} chunks")
        }
        AfterScan::WriteGrid => {
            let grid = pieces::zone_grid(scan.chunks.iter().copied());
            let cells = grid.iter().filter(|id| **id != 0).count();
            let path = format!("Interface\\WorldMap\\{}.zmp", entry.directory);
            match session.save_bytes(&path, pieces::zmp_bytes(&grid)) {
                true => format!("wrote {path}: {cells} of 16384 cells name an area"),
                false => session.status.clone(),
            }
        }
        AfterScan::WriteHighlight => {
            let path = format!("Interface\\WorldMap\\{0}\\{0}Highlight.blp", entry.directory);
            let (sheet, side) = match entry.kind {
                Kind::Zone => {
                    let grid = pieces::zone_grid(scan.chunks.iter().copied());
                    let fraction = sheet_fraction(row.top - row.bottom, row.left - row.right);
                    let inside = |x: f32, y: f32| {
                        pieces::grid_cell(x, y).is_some_and(|cell| grid[cell] != 0 && zone(grid[cell]) == row.area)
                    };
                    (pieces::zone_highlight(&row, fraction, inside), pieces::ZONE_SHEET)
                }
                Kind::Continent => {
                    let Some((_, range)) = rows::placement(session, row.map) else {
                        return "the continent has no WorldMapContinent row".to_string();
                    };
                    let has = |col: u32, tile_row: u32| scan.tiles.contains(&(col, tile_row));
                    (pieces::continent_highlight(range, has), pieces::CONTINENT_SHEET)
                }
                _ => return "only a zone and a continent have a highlight".to_string(),
            };
            let bytes = match blp::encode_dxt1(&sheet, side, side) {
                Ok(bytes) => bytes,
                Err(why) => return format!("{path} could not be encoded: {why}"),
            };
            match session.save_bytes(&path, bytes) {
                true => format!("wrote {path}"),
                false => session.status.clone(),
            }
        }
    }
}
