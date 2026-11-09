//! The ground under a brush as one grid of heights, read across chunks and
//! tiles, for the brush modes that compare a vertex with its neighbours: Blur
//! and Erode.
//!
//! ## Why the modes read a grid rather than the chunks
//!
//! A chunk stores 145 heights: a 9x9 grid of outer vertices and an 8x8 grid of
//! inner vertices at the centres of the squares between them. A vertex on a
//! chunk's edge is stored in both chunks beside it, and a vertex on a tile's
//! edge in both files. A mode that reads a vertex's neighbours needs the
//! neighbours on the far side of those edges. So the caller reads every vertex
//! the brush reaches into one [`Lattice`], the mode works on the lattice, and
//! each chunk then takes its new heights from it. Both copies of a shared
//! vertex read the same cell of the lattice, so they stay equal.
//!
//! ## The grid's axes
//!
//! Outer and inner vertices together lie on a lattice of half the outer
//! spacing, on which every other point is a vertex, as on a checkerboard.
//! Counted along the two diagonals, the vertices form a full square grid:
//! `u = (i + j) / 2` and `v = (i - j) / 2`, where `i` and `j` count half
//! spacings along world x and world y. Neighbours along `u` and `v` are joined
//! by the mesh's own edges between an outer and an inner vertex, [`STEP`]
//! yards apart. Neighbours along a diagonal of the grid are two outer or two
//! inner vertices, one outer spacing apart. Every cell is a vertex, so a blur
//! can run along rows and columns and a drop of water can move between cells
//! without crossing the gaps of the checkerboard.

use std::f32::consts::FRAC_1_SQRT_2;

use crate::adt::{heights, AdtFile};
use vale_assets::world::adt::{vertex_offset, CHUNK_SIZE, UNIT_SIZE};

/// Half the spacing of the outer vertices, in yards: the spacing of the
/// checkerboard lattice the outer and inner vertices lie on together.
const HALF: f32 = UNIT_SIZE / 2.0;

/// The distance between neighbouring cells of a [`Lattice`] along `u` or `v`,
/// in yards: from an outer vertex to the inner vertex beside it.
pub const STEP: f32 = UNIT_SIZE * FRAC_1_SQRT_2;

/// Heights on the vertex grid over a square of the world.
///
/// A cell with no height holds NaN: the vertex is on a tile that was not
/// read, or beyond the edge of the map. Every operation here skips such
/// cells, and none gives one a height.
#[derive(Debug, Clone, PartialEq)]
pub struct Lattice {
    /// The grid coordinates of the first cell.
    u0: i32,
    v0: i32,
    /// Cells along `u`, and along `v`.
    width: usize,
    depth: usize,
    /// Row by row along `v`, `width` cells to a row.
    heights: Vec<f32>,
    /// The world square the lattice covers: its least and greatest x and y.
    low: [f32; 2],
    high: [f32; 2],
}

/// The grid coordinates of the vertex at a world position. For a position
/// between vertices, a nearby vertex's.
fn key(x: f32, y: f32) -> (i32, i32) {
    let i = (x / HALF).round() as i32;
    let j = (y / HALF).round() as i32;
    ((i + j).div_euclid(2), (i - j).div_euclid(2))
}

impl Lattice {
    /// An empty lattice over the square `reach` yards from `at` on each side,
    /// with no heights read yet.
    pub fn around(at: [f32; 2], reach: f32) -> Lattice {
        let low = [at[0] - reach, at[1] - reach];
        let high = [at[0] + reach, at[1] + reach];
        let (i0, i1) = ((low[0] / HALF).floor() as i32, (high[0] / HALF).ceil() as i32);
        let (j0, j1) = ((low[1] / HALF).floor() as i32, (high[1] / HALF).ceil() as i32);
        // The square in (i, j) is a diamond in (u, v); these are the bounds of
        // the diamond, so every vertex in the square has a cell.
        let (u0, u1) = ((i0 + j0).div_euclid(2), (i1 + j1).div_euclid(2) + 1);
        let (v0, v1) = ((i0 - j1).div_euclid(2), (i1 - j0).div_euclid(2) + 1);
        let width = (u1 - u0 + 1) as usize;
        let depth = (v1 - v0 + 1) as usize;
        Lattice { u0, v0, width, depth, heights: vec![f32::NAN; width * depth], low, high }
    }

    /// Read the heights of every vertex of `tile` inside the lattice's square.
    /// Called once for each tile the square reaches.
    pub fn gather(&mut self, tile: &AdtFile) {
        for index in 0..tile.chunks.len() {
            let Some(chunk) = tile.chunk(index) else { continue };
            let origin = chunk.head().position();
            // A chunk runs CHUNK_SIZE toward -x and -y from its origin.
            if origin[0] < self.low[0]
                || origin[0] - CHUNK_SIZE > self.high[0]
                || origin[1] < self.low[1]
                || origin[1] - CHUNK_SIZE > self.high[1]
            {
                continue;
            }
            for (vertex, height) in heights::heights(chunk).into_iter().enumerate() {
                let (dx, dy) = vertex_offset(vertex);
                if let Some(cell) = self.cell(origin[0] - dx, origin[1] - dy) {
                    self.heights[cell] = height;
                }
            }
        }
    }

    /// The height of the vertex at a world position, or `None` when the
    /// position is outside the lattice or its vertex was not read.
    pub fn at(&self, x: f32, y: f32) -> Option<f32> {
        self.cell(x, y).map(|cell| self.heights[cell]).filter(|h| h.is_finite())
    }

    /// The index of the cell at a world position.
    fn cell(&self, x: f32, y: f32) -> Option<usize> {
        let (u, v) = key(x, y);
        self.index(u as i64 - self.u0 as i64, v as i64 - self.v0 as i64)
    }

    /// The index of the cell `a` along `u` and `b` along `v` from the first.
    fn index(&self, a: i64, b: i64) -> Option<usize> {
        let inside = a >= 0 && b >= 0 && (a as usize) < self.width && (b as usize) < self.depth;
        inside.then(|| b as usize * self.width + a as usize)
    }

    /// A world position as continuous grid coordinates from the first cell.
    fn grid(&self, x: f32, y: f32) -> (f32, f32) {
        let (i, j) = (x / HALF, y / HALF);
        ((i + j) / 2.0 - self.u0 as f32, (i - j) / 2.0 - self.v0 as f32)
    }

    /// How many cells hold a height.
    pub fn count(&self) -> usize {
        self.heights.iter().filter(|h| h.is_finite()).count()
    }

    /// The lattice with each height replaced by a local mean over about
    /// `across` yards.
    ///
    /// Two rounds of a box mean along `u` and then along `v`. A box mean
    /// along a row costs the same at any width, because it is read off a
    /// running sum. Two rounds give a kernel that falls off toward its edge
    /// rather than a square plateau, which leaves no square marks on the
    /// ground. A cell with no height adds nothing to its neighbours' means.
    ///
    /// Bumps narrower than `across` are removed and wider ground keeps its
    /// shape, which is the difference from Smooth: Smooth moves every vertex
    /// under the brush toward one height.
    pub fn blurred(&self, across: f32) -> Lattice {
        // A box `2 * half + 1` cells wide, which is `across` yards.
        let half = (across / STEP / 2.0).round().max(1.0) as usize;
        let mut out = self.clone();
        for _ in 0..2 {
            out.box_mean(half, true);
            out.box_mean(half, false);
        }
        out
    }

    /// One box mean of `2 * half + 1` cells, along `u` or along `v`.
    fn box_mean(&mut self, half: usize, along_u: bool) {
        let (len, lines) = match along_u {
            true => (self.width, self.depth),
            false => (self.depth, self.width),
        };
        let width = self.width;
        let index = move |line: usize, k: usize| match along_u {
            true => line * width + k,
            false => k * width + line,
        };
        let mut sums = vec![0.0f64; len + 1];
        let mut counts = vec![0u32; len + 1];
        let mut line_out = vec![0.0f32; len];
        for line in 0..lines {
            for k in 0..len {
                let h = self.heights[index(line, k)];
                let finite = h.is_finite();
                sums[k + 1] = sums[k] + if finite { h as f64 } else { 0.0 };
                counts[k + 1] = counts[k] + finite as u32;
            }
            for (k, out) in line_out.iter_mut().enumerate() {
                let h = self.heights[index(line, k)];
                if !h.is_finite() {
                    *out = h;
                    continue;
                }
                let (lo, hi) = (k.saturating_sub(half), (k + half + 1).min(len));
                // At least one: the cell itself has a height.
                let n = counts[hi] - counts[lo];
                *out = ((sums[hi] - sums[lo]) / n as f64) as f32;
            }
            for (k, out) in line_out.iter().enumerate() {
                self.heights[index(line, k)] = *out;
            }
        }
    }

    /// The lattice after `passes` passes of slope collapse: wherever the ground
    /// between two neighbours is steeper than `talus` degrees, ground moves
    /// from the higher to the lower until the slope is the talus angle.
    ///
    /// This is thermal erosion, the way loose rock and soil settle. Ground is
    /// moved, not removed, so the total height of the cells is kept. A
    /// fractional count runs the whole passes at that fraction of their
    /// strength, so a stroke held for any number of frames moves the ground at
    /// one rate.
    pub fn slumped(&self, talus: f32, passes: f32) -> Lattice {
        let mut out = self.clone();
        if passes <= 0.0 {
            return out;
        }
        let count = passes.ceil().min(MAX_PASSES);
        let share = passes / count;
        let rise = talus.clamp(0.0, 89.0).to_radians().tan();
        for _ in 0..count as usize {
            out.slump(rise, share);
        }
        out
    }

    /// One pass of [`Self::slumped`], at `share` of a full pass's strength.
    fn slump(&mut self, rise: f32, share: f32) {
        // The eight neighbours and their distances in yards: the four along
        // the grid's axes are an outer and an inner vertex apart, the four on
        // its diagonals one outer spacing apart.
        const AROUND: [(i64, i64); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
        let distance = |(a, b): (i64, i64)| match a != 0 && b != 0 {
            true => UNIT_SIZE,
            false => STEP,
        };
        let mut delta = vec![0.0f32; self.heights.len()];
        let mut excess = [0.0f32; 8];
        for b in 0..self.depth as i64 {
            for a in 0..self.width as i64 {
                let here = self.index(a, b).unwrap();
                let h = self.heights[here];
                if !h.is_finite() {
                    continue;
                }
                let (mut total, mut most) = (0.0f32, 0.0f32);
                for (k, &(da, db)) in AROUND.iter().enumerate() {
                    excess[k] = 0.0;
                    let Some(there) = self.index(a + da, b + db) else { continue };
                    let other = self.heights[there];
                    if !other.is_finite() {
                        continue;
                    }
                    let over = h - other - rise * distance((da, db));
                    if over > 0.0 {
                        excess[k] = over;
                        total += over;
                        most = most.max(over);
                    }
                }
                if total == 0.0 {
                    continue;
                }
                // Half the largest excess, so a pair of cells meets at the
                // talus slope rather than passing it, shared among the lower
                // neighbours by how far each is over it.
                let moved = share * most * 0.5;
                delta[here] -= moved;
                for (k, &(da, db)) in AROUND.iter().enumerate() {
                    if excess[k] > 0.0 {
                        let there = self.index(a + da, b + db).unwrap();
                        delta[there] += moved * excess[k] / total;
                    }
                }
            }
        }
        for (h, d) in self.heights.iter_mut().zip(delta) {
            *h += d;
        }
    }

    /// The lattice after a drop of water has run downhill from each of
    /// `starts`, given as world positions.
    ///
    /// This is hydraulic erosion. Each drop follows the slope with a little
    /// inertia. While it speeds up and can carry more, it picks up ground
    /// from the cells around it; where it slows, climbs or carries more than
    /// it can, it puts ground down. Gullies form along the paths drops share,
    /// and the ground they carry settles in hollows and at the foot of slopes.
    /// A drop stops when it evaporates, reaches the edge of the lattice or
    /// reaches a cell with no height.
    ///
    /// The drops work in grid units, heights divided by [`STEP`], so a slope
    /// of 1 is 45 degrees and the constants below do not depend on the
    /// spacing of the vertices.
    pub fn washed(&self, starts: &[[f32; 2]]) -> Lattice {
        let mut g: Vec<f32> = self.heights.iter().map(|h| h / STEP).collect();
        for &[x, y] in starts {
            let (u, v) = self.grid(x, y);
            self.drop(&mut g, u, v);
        }
        let mut out = self.clone();
        out.heights = g.into_iter().map(|h| h * STEP).collect();
        out
    }

    /// One drop of [`Self::washed`], from grid position `(u, v)`.
    fn drop(&self, g: &mut [f32], mut u: f32, mut v: f32) {
        let (mut du, mut dv) = (0.0f32, 0.0f32);
        let (mut speed, mut water, mut carried) = (1.0f32, 1.0f32, 0.0f32);
        for _ in 0..LIFETIME {
            let Some((h, gu, gv)) = self.sample(g, u, v) else { return };
            // Mostly downhill, keeping a little of the last direction.
            du = du * INERTIA - gu * (1.0 - INERTIA);
            dv = dv * INERTIA - gv * (1.0 - INERTIA);
            let length = (du * du + dv * dv).sqrt();
            if length < 1e-6 {
                // Level ground: the drop has nowhere to run, and what it
                // carries stays where it is.
                self.spread(g, u, v, carried);
                return;
            }
            du /= length;
            dv /= length;
            let (from_u, from_v) = (u, v);
            u += du;
            v += dv;
            let Some((after, _, _)) = self.sample(g, u, v) else {
                self.spread(g, from_u, from_v, carried);
                return;
            };
            let fall = h - after;
            let capacity = (fall * speed * water * CAPACITY).max(MIN_CAPACITY);
            if fall < 0.0 || carried > capacity {
                // Uphill, it fills the hollow it left, up to the step it
                // climbed; otherwise it puts down part of what it cannot
                // carry.
                let put = match fall < 0.0 {
                    true => carried.min(-fall),
                    false => (carried - capacity) * DEPOSIT,
                };
                carried -= put;
                self.spread(g, from_u, from_v, put);
            } else {
                // Never more than the fall, so a drop cannot dig a pit deeper
                // than the slope it ran down.
                let take = ((capacity - carried) * ERODE).min(fall);
                carried += take;
                self.spread(g, from_u, from_v, -take);
            }
            speed = (speed * speed + fall * GRAVITY).max(0.0).sqrt();
            water *= 1.0 - EVAPORATE;
        }
        self.spread(g, u, v, carried);
    }

    /// The height and its gradient at a grid position, from the four cells
    /// around it, or `None` when one of them is outside the lattice or has no
    /// height.
    fn sample(&self, g: &[f32], u: f32, v: f32) -> Option<(f32, f32, f32)> {
        let (cu, cv) = (u.floor(), v.floor());
        let (fu, fv) = (u - cu, v - cv);
        let (a, b) = (cu as i64, cv as i64);
        let h00 = g[self.index(a, b)?];
        let h10 = g[self.index(a + 1, b)?];
        let h01 = g[self.index(a, b + 1)?];
        let h11 = g[self.index(a + 1, b + 1)?];
        if !(h00.is_finite() && h10.is_finite() && h01.is_finite() && h11.is_finite()) {
            return None;
        }
        let gu = (h10 - h00) * (1.0 - fv) + (h11 - h01) * fv;
        let gv = (h01 - h00) * (1.0 - fu) + (h11 - h10) * fu;
        let h = h00 * (1.0 - fu) * (1.0 - fv) + h10 * fu * (1.0 - fv) + h01 * (1.0 - fu) * fv + h11 * fu * fv;
        Some((h, gu, gv))
    }

    /// Add `amount` to the four cells around a grid position, each by its
    /// share of the position. A negative amount takes ground away.
    fn spread(&self, g: &mut [f32], u: f32, v: f32, amount: f32) {
        if amount == 0.0 {
            return;
        }
        let (cu, cv) = (u.floor(), v.floor());
        let (fu, fv) = (u - cu, v - cv);
        let (a, b) = (cu as i64, cv as i64);
        for (da, db, share) in [
            (0, 0, (1.0 - fu) * (1.0 - fv)),
            (1, 0, fu * (1.0 - fv)),
            (0, 1, (1.0 - fu) * fv),
            (1, 1, fu * fv),
        ] {
            if let Some(cell) = self.index(a + da, b + db) {
                if g[cell].is_finite() {
                    g[cell] += amount * share;
                }
            }
        }
    }
}

/// The most passes [`Lattice::slumped`] runs for one call. A frame that asks
/// for more runs this many at a greater share each, which stays stable
/// because a share is capped at one full pass.
const MAX_PASSES: f32 = 16.0;

/// How many steps a drop of [`Lattice::washed`] runs before it stops.
const LIFETIME: usize = 40;
/// How much of its last direction a drop keeps at each step.
const INERTIA: f32 = 0.05;
/// How much ground a drop can carry for each unit of fall, speed and water.
const CAPACITY: f32 = 4.0;
/// What a drop can carry on level ground, so it still wears ground there.
const MIN_CAPACITY: f32 = 0.01;
/// The share of its spare capacity a drop picks up at each step.
const ERODE: f32 = 0.3;
/// The share of its excess a drop puts down at each step.
const DEPOSIT: f32 = 0.3;
/// The share of its water a drop loses at each step.
const EVAPORATE: f32 = 0.02;
/// How much a drop speeds up for each unit it falls.
const GRAVITY: f32 = 4.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// A lattice whose height at every cell is `height(x, y)`.
    fn filled(at: [f32; 2], reach: f32, height: impl Fn(f32, f32) -> f32) -> Lattice {
        let mut lattice = Lattice::around(at, reach);
        for b in 0..lattice.depth {
            for a in 0..lattice.width {
                let (u, v) = (a as i32 + lattice.u0, b as i32 + lattice.v0);
                let (x, y) = ((u + v) as f32 * HALF, (u - v) as f32 * HALF);
                lattice.heights[b * lattice.width + a] = height(x, y);
            }
        }
        lattice
    }

    /// Every outer and inner vertex of a chunk has its own cell, and the
    /// vertices on the edge two chunks share land in one cell.
    #[test]
    fn each_vertex_has_one_cell() {
        let tile = crate::adt::blank::blank_tile(32, 32, "x.blp", 10.0, 0);
        let origin = tile.chunk(0).unwrap().head().position();
        let mut lattice = Lattice::around([origin[0] - CHUNK_SIZE, origin[1] - CHUNK_SIZE], CHUNK_SIZE * 1.5);
        lattice.gather(&tile);
        let mut cells = std::collections::BTreeSet::new();
        for vertex in 0..145 {
            let (dx, dy) = vertex_offset(vertex);
            cells.insert(lattice.cell(origin[0] - dx, origin[1] - dy).unwrap());
        }
        assert_eq!(cells.len(), 145, "two vertices of one chunk share a cell");
        // The chunk beside it along -y starts on this chunk's last column.
        let next = tile.chunk(1).unwrap().head().position();
        assert_eq!(lattice.cell(origin[0], origin[1] - CHUNK_SIZE), lattice.cell(next[0], next[1]));
        assert_eq!(lattice.at(origin[0], origin[1]), Some(10.0));
    }

    /// Blur removes bumps narrower than its width and keeps a slope wider than
    /// it.
    #[test]
    fn blur_removes_narrow_bumps_and_keeps_wide_slopes() {
        let at = [100.0, 100.0];
        // A slope of 0.1 with a ripple of 2 yards every 6 yards on it.
        let rippled = filled(at, 120.0, |x, _| 0.1 * x + 2.0 * (x * std::f32::consts::TAU / 6.0).sin());
        let blurred = rippled.blurred(25.0);
        let (x0, x1) = (at[0] - 20.0, at[0] + 20.0);
        let mut worst = 0.0f32;
        let mut x = x0;
        while x < x1 {
            let h = blurred.at(x, at[1]).unwrap();
            worst = worst.max((h - 0.1 * x).abs());
            x += STEP;
        }
        // Under a quarter of the ripple is left. The cells are 2.08 yards apart
        // along x, so a 6-yard ripple is sampled coarsely and a box mean does
        // not cancel it exactly.
        assert!(worst < 0.5, "a ripple of 2 yd left {worst} yd off the slope");
    }

    /// Slope collapse keeps the total height and leaves no slope steeper
    /// than the talus angle once it has run long enough.
    #[test]
    fn slumping_keeps_the_ground_and_ends_at_the_talus() {
        let at = [0.0, 0.0];
        // A spike 40 yards tall on level ground.
        let spike = filled(at, 30.0, |x, y| if x.abs() < 2.0 && y.abs() < 2.0 { 40.0 } else { 0.0 });
        let total = |l: &Lattice| l.heights.iter().map(|&h| h as f64).sum::<f64>();
        let mut slumped = spike.clone();
        for _ in 0..40 {
            slumped = slumped.slumped(30.0, 16.0);
        }
        assert!((total(&slumped) - total(&spike)).abs() < 1e-2, "ground was lost or made");
        let rise = 30.0f32.to_radians().tan();
        let mut steepest = 0.0f32;
        for b in 0..slumped.depth as i64 {
            for a in 0..slumped.width as i64 - 1 {
                let here = slumped.heights[slumped.index(a, b).unwrap()];
                let there = slumped.heights[slumped.index(a + 1, b).unwrap()];
                steepest = steepest.max((here - there).abs() / STEP);
            }
        }
        assert!(steepest < rise * 1.1, "slope {steepest} left against a talus of {rise}");
        assert!(slumped.at(0.0, 0.0).unwrap() < 20.0, "the spike did not come down");
    }

    /// Drops wear a slope down at the top and put ground down lower, and
    /// never touch a cell with no height.
    #[test]
    fn drops_move_ground_downhill() {
        let at = [0.0, 0.0];
        // A slope falling toward +x, with no heights past x = 25.
        let slope = filled(at, 40.0, |x, _| if x > 25.0 { f32::NAN } else { 40.0 - x });
        let starts: Vec<[f32; 2]> =
            (0..200).map(|k| [-20.0 + (k % 10) as f32 * 0.5, -10.0 + (k / 10) as f32]).collect();
        let washed = slope.washed(&starts);
        let changed = |x: f32| -> f32 {
            let mut sum = 0.0;
            for k in -8..=8 {
                let y = k as f32 * 1.5;
                sum += washed.at(x, y).unwrap() - slope.at(x, y).unwrap();
            }
            sum
        };
        assert!(changed(-15.0) < 0.0, "the upper slope was not worn down");
        assert_eq!(washed.count(), slope.count(), "a cell with no height was given one");
    }
}
