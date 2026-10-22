//! The water brush: water flooded onto the chunks it reaches, or drained from
//! them, as `MCLQ` and the flags that declare it.

use crate::adt::{liquid, AdtFile};
use vale_assets::world::adt::CHUNK_SIZE;
use super::{ChunkLiquid, Edit};

/// **Putting water on the ground**, by the liquid cell, under a circle.
///
/// ## The unit is a cell and the height is not the pointer's
///
/// `MCLQ`'s wet flags are the chunk's own 8x8 grid — 4.17-yard cells, the same
/// ones the ground is meshed in — so this is four times finer than the hole
/// stamp and the same resolution as the terrain itself.
///
/// **The surface is flat at a level the caller holds, not at the ground under
/// the pointer.** Water is level; a brush that took its height from wherever
/// the pointer happened to be would paint a staircase down a hillside, which is
/// not a thing the format can even represent well and is never what anybody
/// means. Choosing the level is therefore a separate gesture from painting with
/// it — see `crate::tools::water`, where the three ways to choose one are.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterBrush {
    /// Yards.
    pub radius: f32,
    /// The surface height every cell this paints is set to.
    pub level: f32,
    pub kind: vale_assets::world::wmo::Liquid,
    /// **The high nibble written on every cell this wets** —
    /// [`liquid::FISHABLE`], [`liquid::FATIGUE`], or neither. A pool the
    /// brush makes carried zero there before this existed, so nothing painted
    /// here could be fished; the shipped sea carries `FISHABLE` on nearly
    /// every cell. [`WaterAction::Mark`] writes it without moving the water.
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
            // **The cell under the pointer is always taken**, and the rest by
            // their centres — the rule `AreaBrush::covers` states and for the
            // same reason: a radius smaller than a cell must still do
            // something, or a click near a corner is a click that is ignored.
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
                    // **Drying takes every liquid, wetting takes one.** A cell
                    // under a river and the ocean both is not a thing to make a
                    // person clear twice — and marking takes every liquid for
                    // the same reason.
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
                    if pool.is_wet(row, col)
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
