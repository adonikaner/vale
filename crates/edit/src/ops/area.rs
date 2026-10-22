//! The area brush: which `AreaTable` id each chunk it reaches carries.

use crate::adt::{area, heights, AdtFile};
use vale_assets::world::adt::CHUNK_SIZE;
use super::Edit;

/// **Assigning an area to the ground**, by the chunk, under a circle.
///
/// ## The unit is a chunk and there is nothing smaller
///
/// `MCNK` carries one `areaId` for all 145 of its vertices, so a zone boundary
/// is a 33-yard staircase in the shipped game as much as in anything edited
/// here. A brush is still the right shape — what a person is doing is *making
/// this hillside part of that place* — but its radius chooses **how many
/// chunks**, not where within one.
///
/// ## Which chunks a circle takes, and why it is stated
///
/// The chunk under the pointer **always**, plus every chunk whose centre is
/// inside the circle. Two properties come out of that and both are wanted:
///
/// * a radius under half a chunk is exactly one chunk, which is the common
///   case and the one that must never surprise anybody;
/// * a stroke never does *nothing*, which is what "the circle must overlap the
///   square" gives you when the pointer is near a corner — a brush that
///   sometimes ignores a click is a brush nobody trusts.
///
/// It is deliberately not a falloff. There is nothing to blend: an area id is
/// a row id, and half of one is not a place.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaBrush {
    /// Yards. See the rule above for what it means in chunks.
    pub radius: f32,
    /// The `AreaTable.dbc` row to write. **`0` is a real value** — a chunk
    /// belonging to no area — so there is no "unset" and no `Option` here; see
    /// `crate::adt::area`.
    pub area: u32,
}

impl Default for AreaBrush {
    fn default() -> AreaBrush {
        AreaBrush {
            // One chunk: half of `CHUNK_SIZE` less a little, so the default
            // stroke is the square under the pointer and its immediate
            // neighbours are only reached deliberately.
            radius: 16.0,
            area: 0,
        }
    }
}

impl AreaBrush {
    /// Which of a tile's chunks this stroke covers, at a world position.
    ///
    /// Separate from [`Self::stroke`] so that a tool can **draw what it is about
    /// to do** from the same rule that does it. A preview computed a second way
    /// is a preview that lies, and for a gesture whose result is invisible —
    /// nothing on screen changes when an area id does — the preview is the only
    /// feedback there is.
    pub fn covers(&self, tile: &AdtFile, at: [f32; 2]) -> Vec<usize> {
        let under = heights::chunk_at(tile, at[0], at[1]);
        let mut found: Vec<usize> = Vec::new();
        for (index, chunk) in tile.chunks.iter().enumerate() {
            let origin = chunk.head().position();
            // The chunk's centre: the origin is its *maximum* corner and the
            // cells run in decreasing x and y from it.
            let centre = [origin[0] - CHUNK_SIZE * 0.5, origin[1] - CHUNK_SIZE * 0.5];
            let (dx, dy) = (centre[0] - at[0], centre[1] - at[1]);
            if dx * dx + dy * dy <= self.radius * self.radius || under == Some(index) {
                found.push(index);
            }
        }
        found
    }

    /// …and apply it: every covered chunk that does not already carry the area
    /// takes it.
    ///
    /// **A chunk that already reads the wanted id yields no edit at all**, which
    /// is what keeps a held stroke from putting one entry on the history per
    /// frame for ground it has already painted.
    pub fn stroke(&self, tile: &mut AdtFile, at: [f32; 2]) -> Vec<Edit> {
        let mut edits: Vec<Edit> = Vec::new();
        for index in self.covers(tile, at) {
            let Some(before) = area::area(tile, index) else {
                continue;
            };
            if before == self.area {
                continue;
            }
            let edit = Edit::Area {
                chunk: index,
                before,
                after: self.area,
            };
            edit.apply(tile);
            edits.push(edit);
        }
        edits
    }
}
