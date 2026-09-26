//! Somewhere to go: the zones of a map, and where the middle of each one is.
//!
//! ## The join
//!
//! Two tables, neither of which answers on its own. `AreaTable.dbc` names a
//! zone and says which map it is on but holds no position; `WorldMapArea.dbc`
//! holds the rectangle the world map draws a zone in, in world yards, but names
//! it only by the directory its parchment lives in. The row that joins them is
//! `WorldMapArea`'s own `areaId`, and `WorldMap::continents` already has the
//! rows grouped by map in the order the reference sorted them.
//!
//! Both tables are read through `vale_assets` and neither rule is restated
//! here: the rectangle's axes are `MapArea`'s (left and right are **y**, top and
//! bottom are **x**, each pair running from the larger value to the smaller) and
//! the name is `Areas::zone_name`'s.
//!
//! ## Why the middle of the rectangle
//!
//! It is the only position either table gives. For a zone shaped like Elwynn it
//! is a fair guess at the middle of the zone; for one shaped like the Barrens it
//! lands somewhere in it and not necessarily anywhere interesting. That is
//! enough for what this is — getting the camera near a place so it can be flown
//! the rest of the way — and it is stated so that nobody reads the number as a
//! zone's centre in any stronger sense.

use vale_client::assets::GameAssets;
use bevy::prelude::*;

/// One place the camera can be sent.
#[derive(Debug, Clone)]
pub struct Place {
    pub name: String,
    /// The middle of the zone's world-map rectangle, in the world's own axes.
    pub at: Vec2,
}

/// The zones of one map, sorted by name.
#[derive(Resource, Debug, Default)]
pub struct Places {
    /// Which map these are for, so a map change is noticed.
    pub map_id: u32,
    pub zones: Vec<Place>,
}

impl Places {
    /// Read the zones of a map. Empty for a map with no world-map rows, which is
    /// every instance.
    pub fn of(assets: &GameAssets, map_id: u32) -> Places {
        let mut zones = Vec::new();
        if let Ok(tables) = assets.display_tables() {
            if let Some(world_map) = tables.world_map() {
                let continent = world_map
                    .continents()
                    .iter()
                    .find(|continent| continent.map == map_id);
                for row in continent.map(|c| c.zones.as_slice()).unwrap_or(&[]) {
                    let Some(area) = world_map.area(*row) else {
                        continue;
                    };
                    let name = tables
                        .areas()
                        .map(|areas| areas.zone_name(area.area))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| area.directory.clone());
                    zones.push(Place {
                        name,
                        at: Vec2::new(
                            (area.top + area.bottom) * 0.5,
                            (area.left + area.right) * 0.5,
                        ),
                    });
                }
            }
        }
        zones.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        });
        Places { map_id, zones }
    }
}

/// The middle of a tile, in the world's own axes.
///
/// `vale_assets::world::terrain::tile_centre` is the rule and this only drops
/// its height, which a tile does not have one of. Inverting the tile grid here
/// instead would be a second copy of the arrangement that decides which index
/// counts along which axis, and the two would agree until one of them was
/// corrected.
pub fn middle_of_tile(x: u32, y: u32) -> Vec2 {
    let at = vale_assets::world::terrain::tile_centre(x, y);
    Vec2::new(at[0], at[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The middle of a tile is inside that tile, which is the round trip the
    /// panel's "go to a tile" rests on.
    #[test]
    fn the_middle_of_a_tile_is_inside_it() {
        for (x, y) in [(32u32, 32u32), (0, 0), (63, 63), (32, 48), (48, 32)] {
            let at = middle_of_tile(x, y);
            assert_eq!(
                vale_assets::tile_for_position(at.x, at.y),
                (x, y),
                "the middle of tile ({x}, {y}) landed on another tile"
            );
        }
    }
}
