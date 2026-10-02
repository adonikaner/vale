//! Carrying what stands on the ground when the ground moves.
//!
//! An `MDDF` or `MODF` record holds an absolute height. Raising the ground
//! under a tree therefore buries the tree, in the file and in the game, until
//! the tree is moved as well. [`standing`] reads the ground height under every
//! placement of a tile before the ground is edited, and [`follow`] moves each
//! placement afterwards by as much as the ground under it moved.
//!
//! ## What is moved, and by how much
//!
//! Every doodad and building whose origin is over the tile, by the change in
//! the ground's height at its origin. The rule is the change and not the new
//! height: a lamp hung two yards up a wall, a bird circling and a tree sunk
//! half a yard into a slope each keep the offset from the ground they had.
//!
//! A building's `MODF` record also carries its bounding box, in the same axes
//! as its position. The box moves with it, since the client culls the
//! building by the box.
//!
//! ## What is not moved
//!
//! A placement whose origin is over another tile is left to that tile: the
//! ground under it is not in this file. A placement over a hole has no ground
//! height and is left alone. Spawns of the server's database are not
//! placements of a tile and are not this module's.

use super::Edit;
use crate::adt::{heights, AdtFile};
use vale_assets::world::adt::placement_to_world;

/// The smallest change in the ground's height that moves a placement, in
/// yards. A smooth or a flatten leaves most of the ground under the brush's
/// rim moved by less than this, and a record rewritten for a thousandth of a
/// yard is an undo entry full of moves nobody can see.
pub const LEAST: f32 = 0.01;

/// The ground height under each placement of a tile, by the placement's
/// `unique_id`. See [`standing`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Standing {
    pub doodads: Vec<(u32, f32)>,
    pub buildings: Vec<(u32, f32)>,
}

/// What [`follow`] moved.
#[derive(Debug, Default)]
pub struct Followed {
    /// One [`Edit::Doodad`] or [`Edit::Building`] per placement moved, already
    /// applied to the tile.
    pub edits: Vec<Edit>,
    /// The `MDDF` indices moved, for a caller that has them drawn.
    pub doodads: Vec<usize>,
    /// The `MODF` indices moved.
    pub buildings: Vec<usize>,
}

/// The ground height at a placement's origin, or `None` where the origin is
/// not over this tile's ground.
fn ground_under(tile: &AdtFile, position: [f32; 3]) -> Option<f32> {
    let [x, y, _] = placement_to_world(position);
    heights::height_at(tile, x, y)
}

/// Read the ground height under every placement of the tile. Called before
/// the ground is edited; the answer is what [`follow`] measures against.
pub fn standing(tile: &AdtFile) -> Standing {
    Standing {
        doodads: tile
            .doodad_list()
            .iter()
            .filter_map(|doodad| Some((doodad.unique_id, ground_under(tile, doodad.position)?)))
            .collect(),
        buildings: tile
            .building_list()
            .iter()
            .filter_map(|building| {
                Some((building.unique_id, ground_under(tile, building.position)?))
            })
            .collect(),
    }
}

/// Move every placement by as much as the ground under it has moved since
/// `was` was read, and return the edits, already applied.
///
/// A placement is matched by its `unique_id`. One that was added since is not
/// in `was` and is not moved; one that was removed is not in the tile.
pub fn follow(tile: &mut AdtFile, was: &Standing) -> Followed {
    let mut done = Followed::default();
    for (index, doodad) in tile.doodad_list().into_iter().enumerate() {
        let Some(&(_, before)) = was.doodads.iter().find(|(id, _)| *id == doodad.unique_id) else {
            continue;
        };
        let Some(after) = ground_under(tile, doodad.position) else {
            continue;
        };
        let by = after - before;
        if by.abs() < LEAST {
            continue;
        }
        let mut moved = doodad;
        // The file's second component is height. See `placement_to_world`.
        moved.position[1] += by;
        if let Some(edit) = Edit::move_doodad(tile, index, moved) {
            edit.apply(tile);
            done.edits.push(edit);
            done.doodads.push(index);
        }
    }
    for (index, building) in tile.building_list().into_iter().enumerate() {
        let Some(&(_, before)) = was.buildings.iter().find(|(id, _)| *id == building.unique_id)
        else {
            continue;
        };
        let Some(after) = ground_under(tile, building.position) else {
            continue;
        };
        let by = after - before;
        if by.abs() < LEAST {
            continue;
        }
        let mut moved = building;
        moved.position[1] += by;
        moved.bounds_lower[1] += by;
        moved.bounds_upper[1] += by;
        if let Some(edit) = Edit::move_building(tile, index, moved) {
            edit.apply(tile);
            done.edits.push(edit);
            done.buildings.push(index);
        }
    }
    done
}
