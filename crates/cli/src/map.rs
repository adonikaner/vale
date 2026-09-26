//! `vale map` — the tile grid of one map, **and what a map with no tiles is
//! made of instead.**

use crate::common::*;
use vale_assets::{world::wdt::Wdt, wdt_path};
use vale_config::Config;

pub fn cmd_map(cfg: &Config, map: &str) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let raw = assets.read(&wdt_path(map)).map_err(|e| e.to_string())?;
    let wdt = Wdt::parse(&raw).map_err(|e| e.to_string())?;

    println!("map {map}: version {}, flags {:#x}", wdt.version, wdt.flags);
    println!("  {} tile(s) present", wdt.tile_count());
    if !wdt.global_wmos.is_empty() {
        println!("  global WMO: {}", wdt.global_wmos.join(", "));
    }
    global_wmo(&mut assets, &wdt);

    let tiles = wdt.existing_tiles();
    if let (Some(first), Some(last)) = (tiles.first(), tiles.last()) {
        println!("  first tile {first:?}, last tile {last:?}");
        println!("  try: vale tile {map} {} {}", first.0, first.1);
    }
    Ok(())
}

/// **Where the global WMO actually lands**, which is the only check
/// [`Wdt::placed_global_wmos`]' origin rule has outside its unit tests.
///
/// A placement that is wrong by a whole map reads perfectly well from inside
/// the file — every number in it is finite and plausible — so what is printed
/// is the model's own `MOHD` box run through the placement matrix. Compared
/// against `select min(position_x) … from creature where map = <id>`, that box
/// has to contain where the server spawns things; see the rule's own doc, which
/// carries the six maps this was measured on.
fn global_wmo(assets: &mut vale_assets::Assets, wdt: &Wdt) {
    use vale_assets::world::collision::transform_point;
    use vale_assets::world::wmo::WmoRoot;

    for placed in wdt.placed_global_wmos() {
        println!(
            "  placement {:#010x} at ({:.1}, {:.1}, {:.1}), doodad set {}, name set {}",
            placed.unique_id,
            placed.position[0],
            placed.position[1],
            placed.position[2],
            placed.doodad_set,
            placed.name_set,
        );
        let Some(root) = assets
            .read(&placed.path)
            .ok()
            .and_then(|raw| WmoRoot::parse(&raw).ok())
        else {
            println!("    (its .wmo will not read: {})", placed.path);
            continue;
        };
        // The eight corners, because a rotation turns a box into a box only
        // after re-sorting the axes.
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for corner in 0..8u8 {
            let point = [
                root.bounds[usize::from(corner & 1)][0],
                root.bounds[usize::from((corner >> 1) & 1)][1],
                root.bounds[usize::from((corner >> 2) & 1)][2],
            ];
            let world = transform_point(&placed.matrix, point);
            for axis in 0..3 {
                lo[axis] = lo[axis].min(world[axis]);
                hi[axis] = hi[axis].max(world[axis]);
            }
        }
        println!(
            "    world box x {:.1}..{:.1}  y {:.1}..{:.1}  z {:.1}..{:.1}  \
             ({} groups, wmoID {})",
            lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], root.group_count, root.wmo_id,
        );
    }
}
