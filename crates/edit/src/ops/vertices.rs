//! A set of terrain vertices, chosen one tile at a time, and what is done to
//! the set as a whole.
//!
//! Every other way of moving the ground here is a brush: a footprint and a
//! falloff, applied where the pointer is. A selection is the other kind of
//! tool. The vertices are chosen first, by painting over them, and then moved
//! together by one amount, put at one height or on one tilted plane, or
//! smoothed, with no falloff. That is how a cliff gets a straight top and a
//! building a level pad with a hard edge.
//!
//! The same set can mask a brush instead: a stroke leaves every vertex in it
//! where it is, or moves nothing else ([`Mask`], and
//! [`super::Brush::stroke_masked`]).
//!
//! ## A vertex two chunks share is in the set twice
//!
//! A chunk has 145 vertices, and the outer ones on its edge are also the next
//! chunk's. The set is kept per chunk, by index, so a shared vertex is an
//! entry in each chunk that has it. [`Selected::mark`] chooses by world
//! position, which puts both entries in or out together, and that is what
//! keeps the chunks welded when the set is moved. The vertices on a tile's
//! sides are the next tile's as well; a caller with several tiles open marks
//! each of them with the same position.

use super::{reshade, Edit, Shape};
use crate::adt::{heights, AdtFile};
use std::collections::BTreeMap;
use vale_assets::world::adt::CHUNK_SIZE;

/// The selected vertices of one tile: per chunk, one flag per vertex.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selected {
    chunks: BTreeMap<usize, Vec<bool>>,
}

impl Selected {
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// How many entries the set holds. A vertex two chunks share counts once
    /// for each.
    pub fn count(&self) -> usize {
        self.chunks
            .values()
            .map(|flags| flags.iter().filter(|on| **on).count())
            .sum()
    }

    /// Whether one vertex of one chunk is in the set.
    pub fn holds(&self, chunk: usize, vertex: usize) -> bool {
        self.chunks
            .get(&chunk)
            .is_some_and(|flags| flags.get(vertex).copied().unwrap_or(false))
    }

    /// Put in, or take out, every vertex of the tile within `radius` of `at`
    /// in the brush's own shape. Returns how many entries changed.
    pub fn mark(&mut self, tile: &AdtFile, at: [f32; 2], radius: f32, shape: Shape, on: bool) -> usize {
        let mut changed = 0;
        for index in 0..tile.chunks.len() {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            let near = |o: f32, p: f32| p >= o - CHUNK_SIZE - radius && p <= o + radius;
            if !near(origin[0], at[0]) || !near(origin[1], at[1]) {
                continue;
            }
            for vertex in 0..heights::VERTICES {
                let (dx, dy) = heights::vertex_offset(vertex);
                let (x, y) = (origin[0] - dx, origin[1] - dy);
                if shape.distance(x - at[0], y - at[1]) > radius {
                    continue;
                }
                if !on && !self.chunks.contains_key(&index) {
                    continue;
                }
                let flags = self
                    .chunks
                    .entry(index)
                    .or_insert_with(|| vec![false; heights::VERTICES]);
                if flags[vertex] != on {
                    flags[vertex] = on;
                    changed += 1;
                }
            }
            // A chunk with nothing left in it is not kept.
            if self.chunks.get(&index).is_some_and(|flags| !flags.contains(&true)) {
                self.chunks.remove(&index);
            }
        }
        changed
    }

    /// The world position of every vertex in the set, for a caller that draws
    /// them.
    pub fn positions(&self, tile: &AdtFile) -> Vec<[f32; 3]> {
        let mut found = Vec::new();
        for (&index, flags) in &self.chunks {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            let all = heights::heights(chunk);
            for (vertex, _) in flags.iter().enumerate().filter(|(_, on)| **on) {
                if let Some(&height) = all.get(vertex) {
                    found.push(heights::vertex_position(origin, vertex, height));
                }
            }
        }
        found
    }

    /// The sums of the selected vertices' x, y and height, and how many
    /// there are, so a caller with several tiles can add them up before
    /// dividing. The quotient is the selection's centre at its mean height.
    pub fn weighed(&self, tile: &AdtFile) -> ([f64; 3], usize) {
        let (mut sum, mut count) = ([0.0f64; 3], 0usize);
        for position in self.positions(tile) {
            for axis in 0..3 {
                sum[axis] += f64::from(position[axis]);
            }
            count += 1;
        }
        (sum, count)
    }

    /// The heights the selected vertices would have, each given by `to` from
    /// its world position, without writing them. See [`Plan`].
    pub fn plan(&self, tile: &AdtFile, to: impl Fn([f32; 3]) -> f32) -> Plan {
        let mut chunks = Vec::new();
        for (&index, flags) in &self.chunks {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            let before = heights::heights(chunk);
            if before.is_empty() {
                continue;
            }
            let after: Vec<f32> = before
                .iter()
                .enumerate()
                .map(|(vertex, &height)| match flags.get(vertex) {
                    Some(true) => to(heights::vertex_position(origin, vertex, height)),
                    _ => height,
                })
                .collect();
            if after != before {
                chunks.push((index, before, after));
            }
        }
        Plan { chunks }
    }

    /// Move every selected vertex up or down by `by` yards. The shape the
    /// vertices make is kept.
    pub fn shift(&self, tile: &mut AdtFile, by: f32) -> Vec<Edit> {
        self.plan(tile, |at| at[2] + by).write(tile)
    }

    /// Put every selected vertex at one height.
    pub fn level(&self, tile: &mut AdtFile, to: f32) -> Vec<Edit> {
        self.plan(tile, |_| to).write(tile)
    }

    /// Put every selected vertex on the plane through `pivot` with the slope
    /// `slope`, in yards of rise per yard along world x and y. See
    /// [`super::Brush::tilted`] for a slope from an angle and a bearing.
    pub fn tilt(&self, tile: &mut AdtFile, pivot: [f32; 3], slope: [f32; 2]) -> Vec<Edit> {
        self.plan(tile, |at| plane(pivot, slope, at)).write(tile)
    }

    /// The heights one pass of smoothing gives the selected vertices: each
    /// moves `amount` of the way, 0 to 1, toward the mean of the four
    /// vertices beside it. `ground` answers the height at a world position.
    /// A caller with several tiles open answers from all of them, so the two
    /// copies of a vertex on a tile's side move alike.
    pub fn smoothed(
        &self,
        tile: &AdtFile,
        amount: f32,
        ground: impl Fn(f32, f32) -> Option<f32>,
    ) -> Plan {
        // The spacing of the vertex grid. An outer vertex's four neighbours
        // are outer vertices and an inner one's are inner.
        const STEP: f32 = CHUNK_SIZE / 8.0;
        self.plan(tile, |at| {
            let (mut sum, mut count) = (0.0, 0);
            for (dx, dy) in [(STEP, 0.0), (-STEP, 0.0), (0.0, STEP), (0.0, -STEP)] {
                if let Some(height) = ground(at[0] + dx, at[1] + dy) {
                    sum += height;
                    count += 1;
                }
            }
            match count {
                0 => at[2],
                n => at[2] + amount.clamp(0.0, 1.0) * (sum / n as f32 - at[2]),
            }
        })
    }
}

/// The height of a plane at a position: the plane through `pivot` that rises
/// `slope[0]` yards per yard along world x and `slope[1]` along world y.
pub fn plane(pivot: [f32; 3], slope: [f32; 2], at: [f32; 3]) -> f32 {
    pivot[2] + slope[0] * (at[0] - pivot[0]) + slope[1] * (at[1] - pivot[1])
}

/// New heights for some chunks of one tile, worked out and not yet written.
///
/// Working out and writing are separate so that an operation which reads the
/// ground round a vertex reads it as it was, on every tile, before any of it
/// moves.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Per chunk: its index, its heights now, and its heights after.
    chunks: Vec<(usize, Vec<f32>, Vec<f32>)>,
}

impl Plan {
    /// Write the heights, and return the edits, already applied, with the
    /// normals that follow.
    pub fn write(self, tile: &mut AdtFile) -> Vec<Edit> {
        let mut edits = Vec::new();
        for (chunk, before, after) in self.chunks {
            let Some(mesh) = tile.chunk_mut(chunk) else {
                continue;
            };
            heights::set_heights(mesh, &after);
            edits.push(Edit::Heights {
                chunk,
                before,
                after,
            });
        }
        if !edits.is_empty() {
            reshade(tile, &mut edits);
        }
        edits
    }
}

/// How a brush stroke treats a selection. See
/// [`super::Brush::stroke_masked`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mask {
    /// The stroke moves the ground round the selection and not the selection.
    Protect,
    /// The stroke moves the selection and nothing else.
    Confine,
}

impl Mask {
    /// Whether a stroke under this mask may move one vertex of one chunk.
    pub fn lets(self, selected: &Selected, chunk: usize, vertex: usize) -> bool {
        match self {
            Mask::Protect => !selected.holds(chunk, vertex),
            Mask::Confine => selected.holds(chunk, vertex),
        }
    }
}
