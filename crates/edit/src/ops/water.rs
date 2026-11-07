//! The water brush: water flooded onto the chunks it reaches, or drained from
//! them, as `MCLQ` and the flags that declare it.

use crate::adt::{liquid, AdtFile};
use vale_assets::world::adt::CHUNK_SIZE;
use super::{ChunkLiquid, Edit};

/// A brush that wets or dries liquid cells under a circle.
///
/// ## The unit is the liquid cell, and the height is the brush's level
///
/// `MCLQ`'s wet flags are the chunk's own 8x8 grid of 4.17-yard cells, the same
/// cells the ground is meshed in. That is four times finer than the hole stamp
/// and the same resolution as the terrain.
///
/// The surface is flat at a level the caller holds, not at the ground height
/// under the pointer. Water is level; a brush that took its height from the
/// ground under the pointer would paint a staircase down a hillside, which the
/// format cannot represent well and which is not what a user wants. Choosing
/// the level is therefore a separate action from painting with it;
/// `crate::tools::water` has the three ways to choose one.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterBrush {
    /// Yards.
    pub radius: f32,
    /// The surface height every cell this paints is set to.
    pub level: f32,
    pub kind: vale_assets::world::wmo::Liquid,
    /// The high nibble written on every cell this wets: [`liquid::FISHABLE`],
    /// [`liquid::FATIGUE`], or neither. Before this field existed the brush
    /// wrote zero there, so no water painted with it could be fished; the
    /// shipped sea carries `FISHABLE` on nearly every cell.
    /// [`WaterAction::Mark`] writes it without moving the water.
    pub cell_flags: u8,
    /// A sloped surface in place of [`Self::level`]: two points, each a world
    /// position and a surface height. The surface runs straight from one to
    /// the other and is level past either end, so a river is laid as a chain
    /// of these. See [`Self::level_at`].
    pub slope: Option<[[f32; 3]; 2]>,
}

impl Default for WaterBrush {
    fn default() -> WaterBrush {
        WaterBrush {
            radius: 12.0,
            level: 0.0,
            kind: vale_assets::world::wmo::Liquid::Water,
            cell_flags: liquid::FISHABLE,
            slope: None,
        }
    }
}

/// What one step of a water stroke does to the cells under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaterAction {
    /// Wet them at the level, with the brush's flags.
    Flood,
    /// Dry them, whatever liquid is on them.
    Drain,
    /// Write the brush's flags on the cells that are already wet, and leave
    /// the water where it is — the way to make an existing lake fishable, or
    /// an existing sea deep, without repainting it.
    Mark,
    /// Work the depth bytes of the wet cells out again from the ground under
    /// them, and leave the water where it is. The depth is taken when a cell
    /// is flooded, so it is out of date once the bed under it is reshaped.
    Depth,
}

/// The four corners of a liquid cell: each one's place in the 9x9 vertex
/// grid and its world position, from the cell's square.
fn corners(high: [f32; 2], low: [f32; 2], row: usize, col: usize) -> [(usize, usize, f32, f32); 4] {
    [
        (row, col, high[0], high[1]),
        (row, col + 1, high[0], low[1]),
        (row + 1, col, low[0], high[1]),
        (row + 1, col + 1, low[0], low[1]),
    ]
}

/// What one step of a water stroke did.
pub struct Flooded {
    pub edits: Vec<Edit>,
    /// How many cells it wet or dried, for the status line. A stroke over ground
    /// that is already as asked reports zero and records nothing.
    pub cells: usize,
}

impl WaterBrush {
    /// The surface height this brush floods to at a world position: the
    /// level, or along the slope. The height at a point between the two
    /// ends is the two heights mixed by how far along the line between them
    /// the point is; across the line it does not change.
    pub fn level_at(&self, x: f32, y: f32) -> f32 {
        let Some([start, end]) = self.slope else {
            return self.level;
        };
        let along = [end[0] - start[0], end[1] - start[1]];
        let length = along[0] * along[0] + along[1] * along[1];
        if length < 1e-6 {
            return start[2];
        }
        let t = (((x - start[0]) * along[0] + (y - start[1]) * along[1]) / length).clamp(0.0, 1.0);
        start[2] + t * (end[2] - start[2])
    }

    /// Wet every cell the circle covers, or dry them.
    ///
    /// `ground` answers the terrain height at a world position, and is the
    /// caller's because this crate has the heights and not the tile a position
    /// falls in — it is what the depth byte is computed from, so that a pool has
    /// shallows at its shore instead of one flat opacity. See
    /// [`liquid::Pool::depth_for`].
    pub fn stroke(
        &self,
        tile: &mut AdtFile,
        at: [f32; 2],
        action: WaterAction,
        ground: impl Fn(f32, f32) -> Option<f32>,
    ) -> Flooded {
        let mut edits = Vec::new();
        let mut cells = 0;
        for index in self.chunks_under(tile, at) {
            let Some(chunk) = tile.chunk(index) else { continue };
            let origin = chunk.head().position();
            // The cell under the pointer is always taken, and the others by
            // their centres. `AreaBrush::covers` states the same rule for the
            // same reason: a radius smaller than a cell must still change
            // something, or a click near a corner does nothing.
            let under = vale_assets::world::adt::cell_at(origin, at[0], at[1]);
            let before = ChunkLiquid::capture(tile, index);
            let mut pools = liquid::pools(tile, index);
            let mut touched = 0;

            for row in 0..vale_assets::world::adt::INNER_SIDE {
                for col in 0..vale_assets::world::adt::INNER_SIDE {
                    let (high, low) =
                        vale_assets::world::adt::cell_square(origin, row, col, 1);
                    let centre = [(high[0] + low[0]) * 0.5, (high[1] + low[1]) * 0.5];
                    let (dx, dy) = (centre[0] - at[0], centre[1] - at[1]);
                    let reached = dx * dx + dy * dy <= self.radius * self.radius;
                    if !reached && under != Some((row, col)) {
                        continue;
                    }
                    // Drying and marking act on every liquid on the cell;
                    // wetting acts on the brush's one. A cell under both a
                    // river and the ocean is then cleared or marked in one
                    // pass rather than two.
                    match action {
                        WaterAction::Drain => {
                            for pool in pools.iter_mut() {
                                if pool.is_wet(row, col) {
                                    pool.set_wet(row, col, false);
                                    touched += 1;
                                }
                            }
                            continue;
                        }
                        WaterAction::Mark => {
                            for pool in pools.iter_mut() {
                                if pool.is_wet(row, col) && pool.cell_flags(row, col) != self.cell_flags & 0xF0
                                {
                                    pool.set_cell_flags(row, col, self.cell_flags);
                                    touched += 1;
                                }
                            }
                            continue;
                        }
                        WaterAction::Depth => {
                            for pool in pools.iter_mut().filter(|pool| pool.has_depth()) {
                                if !pool.is_wet(row, col) {
                                    continue;
                                }
                                let mut changed = false;
                                for (r, c, x, y) in corners(high, low, row, col) {
                                    let Some(floor) = ground(x, y) else { continue };
                                    let depth = liquid::Pool::depth_for(pool.height(r, c) - floor);
                                    if pool.depth(r, c) != depth {
                                        pool.set_depth(r, c, depth);
                                        changed = true;
                                    }
                                }
                                touched += usize::from(changed);
                            }
                            continue;
                        }
                        WaterAction::Flood => {}
                    }
                    let slot = match pools.iter().position(|pool| pool.kind == self.kind) {
                        Some(slot) => slot,
                        None => {
                            pools.push(liquid::Pool::flat(self.kind, self.level_at(at[0], at[1])));
                            pools.len() - 1
                        }
                    };
                    let pool = &mut pools[slot];
                    // Each corner at the surface over it, so a sloped surface
                    // is a slope and two cells that share a corner agree on it.
                    let surface = corners(high, low, row, col)
                        .map(|(r, c, x, y)| (r, c, x, y, self.level_at(x, y)));
                    // A cell is left alone only when it is already wet at
                    // this height, with these flags and its kind's type, so
                    // painting over a cell with no type gives it one.
                    if pool.is_typed(row, col)
                        && surface
                            .iter()
                            .all(|&(r, c, _, _, h)| (pool.height(r, c) - h).abs() < 1e-3)
                        && pool.cell_flags(row, col) == self.cell_flags & 0xF0
                    {
                        continue;
                    }
                    pool.set_wet(row, col, true);
                    pool.set_cell_flags(row, col, self.cell_flags);
                    // The depth at each corner, from the ground under that
                    // corner, so the shallows follow the shore.
                    for (r, c, x, y, height) in surface {
                        let above = ground(x, y).map(|floor| height - floor);
                        pool.set_vertex(r, c, height, liquid::Pool::depth_for(above.unwrap_or(0.0)));
                    }
                    touched += 1;
                }
            }
            if touched == 0 {
                continue;
            }
            let Some(chunk) = tile.chunk_mut(index) else { continue };
            liquid::set_pools(chunk, &pools);
            let after = ChunkLiquid::capture(tile, index);
            if after == before {
                continue;
            }
            cells += touched;
            edits.push(Edit::Liquid {
                chunk: index,
                before: Box::new(before),
                after: Box::new(after),
            });
        }
        Flooded { edits, cells }
    }

}

/// Give every wet liquid cell on a tile whose type is 0 its pool's type — see
/// [`liquid::Pool::repair_untyped`]. One [`Edit::Liquid`] per chunk that
/// changed, and the number of cells, on [`WaterBrush::stroke`]'s terms.
pub fn repair_liquid_types(tile: &mut AdtFile) -> Flooded {
    let mut edits = Vec::new();
    let mut cells = 0;
    for index in 0..tile.chunks.len() {
        let mut pools = liquid::pools(tile, index);
        let changed: usize = pools.iter_mut().map(|pool| pool.repair_untyped()).sum();
        if changed == 0 {
            continue;
        }
        let before = ChunkLiquid::capture(tile, index);
        let Some(chunk) = tile.chunk_mut(index) else { continue };
        liquid::set_pools(chunk, &pools);
        let after = ChunkLiquid::capture(tile, index);
        if after == before {
            continue;
        }
        cells += changed;
        edits.push(Edit::Liquid {
            chunk: index,
            before: Box::new(before),
            after: Box::new(after),
        });
    }
    Flooded { edits, cells }
}

/// Whether wet cell `(row, col)` of `pool` lies wholly under the ground of a
/// chunk whose 145 world heights are `ground` and whose hole mask is `holes`.
///
/// The ground over a cell is four triangles that meet at the cell's inner
/// vertex, and the liquid's surface is its four corner heights. Both surfaces
/// are flat between the cell's four corners and its centre, so the liquid is
/// under the ground everywhere when it is under it at those five points. The
/// liquid's height at the centre is taken as the higher of its two diagonals'
/// midpoints, which holds whichever diagonal its quad is split along.
///
/// A cell under a hole is not buried, since the hole shows the liquid. A cell
/// with a corner at `f32::MAX`, the height of a vertex with no surface, or a
/// corner level with the ground,
/// is not buried either.
pub fn is_buried(pool: &liquid::Pool, ground: &[f32], holes: u16, row: usize, col: usize) -> bool {
    use vale_assets::world::adt::{inner_index, outer_index, HEIGHTS_PER_CHUNK};
    if ground.len() < HEIGHTS_PER_CHUNK || !pool.is_wet(row, col) {
        return false;
    }
    if holes >> ((row / 2) * 4 + col / 2) & 1 != 0 {
        return false;
    }
    let corners = [(row, col), (row, col + 1), (row + 1, col), (row + 1, col + 1)];
    let surface = corners.map(|(r, c)| pool.height(r, c));
    if surface.contains(&liquid::NO_SURFACE) {
        return false;
    }
    if corners
        .iter()
        .zip(surface)
        .any(|(&(r, c), h)| h >= ground[outer_index(r, c)])
    {
        return false;
    }
    let centre = (surface[0] + surface[3]).max(surface[1] + surface[2]) * 0.5;
    centre < ground[inner_index(row, col)]
}

/// How many wet liquid cells of a chunk are buried, by [`is_buried`]. A cell
/// wet in two pools counts once for each.
pub fn buried_cells(tile: &AdtFile, chunk: usize) -> usize {
    let Some(map_chunk) = tile.chunk(chunk) else {
        return 0;
    };
    let (ground, holes) = (crate::adt::heights::heights(map_chunk), map_chunk.head().holes());
    let side = vale_assets::world::adt::INNER_SIDE;
    liquid::pools(tile, chunk)
        .iter()
        .map(|pool| {
            (0..side * side)
                .filter(|n| is_buried(pool, &ground, holes, n / side, n % side))
                .count()
        })
        .sum()
}

/// Dry every buried liquid cell of one chunk ([`is_buried`]). A pool left with
/// no wet cell is dropped with its header flag, as [`liquid::set_pools`] does.
/// Answers the edit that records it and how many cells were dried, or `None`
/// when no cell is buried.
pub fn drain_buried(tile: &mut AdtFile, chunk: usize) -> Option<(Edit, usize)> {
    let map_chunk = tile.chunk(chunk)?;
    let (ground, holes) = (crate::adt::heights::heights(map_chunk), map_chunk.head().holes());
    let side = vale_assets::world::adt::INNER_SIDE;
    let mut pools = liquid::pools(tile, chunk);
    let mut dried = 0;
    for pool in pools.iter_mut() {
        for n in 0..side * side {
            let (row, col) = (n / side, n % side);
            if is_buried(pool, &ground, holes, row, col) {
                pool.set_wet(row, col, false);
                dried += 1;
            }
        }
    }
    if dried == 0 {
        return None;
    }
    let before = ChunkLiquid::capture(tile, chunk);
    liquid::set_pools(tile.chunk_mut(chunk)?, &pools);
    let after = ChunkLiquid::capture(tile, chunk);
    let edit = Edit::Liquid {
        chunk,
        before: Box::new(before),
        after: Box::new(after),
    };
    Some((edit, dried))
}

impl WaterBrush {
    /// Which of a tile's chunks the circle can reach at all.
    fn chunks_under(&self, tile: &AdtFile, at: [f32; 2]) -> Vec<usize> {
        (0..tile.chunks.len())
            .filter(|&i| {
                let Some(chunk) = tile.chunk(i) else {
                    return false;
                };
                let origin = chunk.head().position();
                let near =
                    |o: f32, p: f32| p >= o - CHUNK_SIZE - self.radius && p <= o + self.radius;
                near(origin[0], at[0]) && near(origin[1], at[1])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adt::heights;
    use vale_assets::world::adt::{inner_index, mcnk_flags, outer_index, INNER_SIDE};
    use vale_assets::world::wmo::Liquid;

    /// A blank tile at height 0 whose chunk 7 carries water at `level` on
    /// every cell.
    fn flooded(level: f32) -> AdtFile {
        let mut tile = crate::adt::blank_tile(32, 48, r"Tileset\Generic\Black.blp", 0.0, 0);
        let mut pool = liquid::Pool::flat(Liquid::Water, level);
        for row in 0..INNER_SIDE {
            for col in 0..INNER_SIDE {
                pool.set_wet(row, col, true);
                pool.fill_cell(row, col, level, 0);
            }
        }
        liquid::set_pools(tile.chunk_mut(7).unwrap(), &[pool]);
        tile
    }

    /// Change one of chunk 7's 145 heights.
    fn lift(tile: &mut AdtFile, index: usize, height: f32) {
        let chunk = tile.chunk_mut(7).unwrap();
        let mut ground = heights::heights(chunk);
        ground[index] = height;
        heights::set_heights(chunk, &ground);
    }

    /// Water under the ground is buried except where a corner of its cell
    /// reaches the surface, where the ground's centre dips below it, or where
    /// a hole shows it. Draining dries only the buried cells, and a second
    /// drain finds nothing.
    #[test]
    fn only_the_cells_the_ground_covers_entirely_are_drained() {
        let mut tile = flooded(-2.0);
        assert_eq!(buried_cells(&tile, 7), 64);
        // One corner shared by cells (2,2), (2,3), (3,2) and (3,3), raised
        // above the water.
        lift(&mut tile, outer_index(3, 3), -3.0);
        // Cell (0,5)'s centre, lowered below the water.
        lift(&mut tile, inner_index(0, 5), -2.5);
        // Hole bit 15 covers cells (6..8, 6..8).
        let flags = tile.chunk(7).unwrap().head().holes() | 1 << 15;
        crate::ops::Edit::Holes { chunk: 7, before: 0, after: flags }.apply(&mut tile);
        assert_eq!(buried_cells(&tile, 7), 64 - 4 - 1 - 4);

        let (_, dried) = drain_buried(&mut tile, 7).expect("buried cells");
        assert_eq!(dried, 55);
        let pools = liquid::pools(&tile, 7);
        assert_eq!(pools.len(), 1);
        for (row, col) in [(2, 2), (3, 3), (0, 5), (6, 6), (7, 7)] {
            assert!(pools[0].is_wet(row, col), "({row},{col}) is kept");
        }
        assert!(!pools[0].is_wet(0, 0));
        assert!(drain_buried(&mut tile, 7).is_none());
    }

    /// Water level with the ground or above it is not buried, and a chunk
    /// whose every wet cell is buried loses its pool and the header flag.
    #[test]
    fn a_wholly_buried_pool_is_dropped() {
        assert_eq!(buried_cells(&flooded(0.0), 7), 0);
        assert_eq!(buried_cells(&flooded(1.0), 7), 0);
        let mut tile = flooded(-1.0);
        let (edit, dried) = drain_buried(&mut tile, 7).unwrap();
        assert_eq!(dried, 64);
        assert!(liquid::pools(&tile, 7).is_empty());
        assert_eq!(tile.chunk(7).unwrap().head().flags() & mcnk_flags::LQ_ANY, 0);
        edit.revert(&mut tile);
        assert_eq!(buried_cells(&tile, 7), 64);
    }
}
