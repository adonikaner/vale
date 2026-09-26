//! `vale navmesh`: the server's navmesh tiles, read.
//!
//! Two forms:
//!
//! ```text
//! vale navmesh                  every .mmtile in DataDir\mmaps: how many
//!                                  read, which did not and why, and the
//!                                  share of each surface class
//! vale navmesh Azeroth 32 48    one tile: its counts by surface, its
//!                                  outline, and whether its box lands on the
//!                                  terrain tile it is named after
//! ```
//!
//! `DataDir` is read from the `mangosd.conf` that `VALE_MANGOSD` names,
//! which is the file the server reads it from. Read-only.
//!
//! The one-tile form checks the axis conversion in
//! `vale_mangos::navmesh`: the middle of the tile's box, converted to world
//! axes, must fall on the terrain tile whose number is in the file name. A
//! reader that swapped two axes places the box on a different tile, or off the
//! map.

use std::collections::BTreeMap;
use std::path::PathBuf;

use vale_assets::world::terrain::tile_for_position;
use vale_config::Config;
use vale_mangos::datadir::{self, Tile};
use vale_mangos::navmesh::{self, NavTile, Surface};

use crate::common::open_assets;

/// Every form of the command.
pub fn cmd_navmesh(cfg: &Config, args: &[String]) -> Result<(), String> {
    let data_dir = data_dir()?;
    println!("DataDir: {}", data_dir.display());
    match (args.get(1), args.get(2), args.get(3)) {
        (None, _, _) => census(&data_dir),
        (Some(map), Some(x), Some(y)) => {
            let (x, y) = match (x.parse::<u32>(), y.parse::<u32>()) {
                (Ok(x), Ok(y)) => (x, y),
                _ => return Err("tile coordinates must be numbers".to_string()),
            };
            let map_id = match map.parse::<u32>() {
                Ok(id) => id,
                Err(_) => {
                    let mut assets = open_assets(cfg)?;
                    crate::water::map_id_for(&mut assets, map)
                        .ok_or_else(|| format!("Map.dbc has no map called {map}"))?
                }
            };
            one(&data_dir, Tile::new(map_id, x, y))
        }
        _ => Err("usage: vale navmesh [<MapName or id> <x> <y>]".to_string()),
    }
}

/// `DataDir` from the conf `VALE_MANGOSD` names.
fn data_dir() -> Result<PathBuf, String> {
    let at = std::env::var("VALE_MANGOSD")
        .map_err(|_| "no server: set VALE_MANGOSD to the server's mangosd.conf".to_string())?;
    let at = PathBuf::from(at);
    let conf = match at.is_dir() {
        true => at.join("mangosd.conf"),
        false => at,
    };
    datadir::from_conf(&conf).ok_or_else(|| format!("{}: no DataDir", conf.display()))
}

/// Every tile in `mmaps\`.
fn census(data_dir: &std::path::Path) -> Result<(), String> {
    let folder = data_dir.join("mmaps");
    let entries = std::fs::read_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.to_ascii_lowercase().ends_with(".mmtile"))
        .collect();
    names.sort();

    let mut per_map: BTreeMap<u32, usize> = BTreeMap::new();
    let mut surfaces: BTreeMap<Surface, (usize, usize)> = BTreeMap::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    let (mut polygons, mut triangles, mut edges, mut liquids) = (0usize, 0usize, 0usize, 0usize);
    for name in &names {
        let path = folder.join(name);
        let read = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| navmesh::parse_tile(&bytes));
        let tile = match read {
            Ok(tile) => tile,
            Err(e) => {
                failed.push((name.clone(), e));
                continue;
            }
        };
        if let Ok(map) = name[..3].parse::<u32>() {
            *per_map.entry(map).or_default() += 1;
        }
        polygons += tile.polygons;
        triangles += tile.triangles();
        edges += tile.edges.len();
        liquids += tile.uses_liquids as usize;
        for (surface, part) in &tile.parts {
            let entry = surfaces.entry(*surface).or_default();
            entry.0 += part.polygons;
            entry.1 += part.indices.len() / 3;
        }
    }

    println!(
        "{} tiles, {} read, {} failed; {} built with liquid",
        names.len(),
        names.len() - failed.len(),
        failed.len(),
        liquids
    );
    println!("{polygons} polygons, {triangles} detail triangles, {edges} outline edges");
    println!();
    println!("{:<14} {:>10} {:>7}  flags", "surface", "polygons", "share");
    for (surface, (polys, _)) in &surfaces {
        let share = 100.0 * *polys as f64 / polygons.max(1) as f64;
        println!("{:<14} {:>10} {:>6.1}%  {}", surface.name(), polys, share, surface.flags());
    }
    println!();
    println!("tiles per map:");
    for (map, count) in &per_map {
        println!("  {map:>3}  {count}");
    }
    for (name, why) in failed.iter().take(20) {
        println!("FAILED {name}: {why}");
    }
    match failed.is_empty() {
        true => Ok(()),
        false => Err(format!("{} tile(s) did not read", failed.len())),
    }
}

/// One tile.
fn one(data_dir: &std::path::Path, tile: Tile) -> Result<(), String> {
    let path = navmesh::tile_path(data_dir, tile);
    let nav: NavTile = navmesh::read_tile(data_dir, tile)?
        .ok_or_else(|| format!("{} does not exist", path.display()))?;
    println!("{}", path.display());
    println!(
        "Detour grid cell {},{}; built {} liquid",
        nav.grid.0,
        nav.grid.1,
        match nav.uses_liquids {
            true => "with",
            false => "without",
        }
    );
    println!(
        "{} polygons, {} detail triangles, {} outline edges, {} off-mesh connections",
        nav.polygons,
        nav.triangles(),
        nav.edges.len(),
        nav.off_mesh
    );
    println!();
    println!("{:<14} {:>9} {:>10} {:>9}  flags", "surface", "polygons", "triangles", "vertices");
    for (surface, part) in &nav.parts {
        println!(
            "{:<14} {:>9} {:>10} {:>9}  {}",
            surface.name(),
            part.polygons,
            part.indices.len() / 3,
            part.positions.len(),
            surface.flags()
        );
    }

    let [low, high] = nav.bounds;
    println!();
    println!(
        "box, world axes: x {:.1} .. {:.1}, y {:.1} .. {:.1}, height {:.1} .. {:.1}",
        low[0], high[0], low[1], high[1], low[2], high[2]
    );
    let middle = ((low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0);
    let lands = tile_for_position(middle.0, middle.1);
    let agrees = lands == (tile.x, tile.y);
    println!(
        "the box's middle is on terrain tile {},{}: {}",
        lands.0,
        lands.1,
        match agrees {
            true => "the tile the file is named after",
            false => "NOT the tile the file is named after",
        }
    );
    match agrees {
        true => Ok(()),
        false => Err("the axis conversion places this tile on the wrong terrain tile".to_string()),
    }
}
