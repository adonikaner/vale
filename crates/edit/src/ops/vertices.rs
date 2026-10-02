//! A set of terrain vertices, chosen one tile at a time, and what is done to
//! the set as a whole.
//!
//! Every other way of moving the ground here is a brush: a footprint and a
//! falloff, applied where the pointer is. A selection is the other kind of
//! tool. The vertices are chosen first, by painting over them, and then moved
//! together by one amount or put at one height, with no falloff. That is how
//! a cliff gets a straight top and a building a level pad with a hard edge.
//!
//! The same set can protect ground instead: a brush given it leaves every
//! vertex in it where it is ([`super::Brush::stroke_keeping`]).
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

    /// The sum of the selected vertices' heights and how many there are, so a
    /// caller with several tiles can add them up before dividing.
    pub fn weighed(&self, tile: &AdtFile) -> (f64, usize) {
        let (mut sum, mut count) = (0.0f64, 0usize);
        for (&index, flags) in &self.chunks {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let all = heights::heights(chunk);
            for (vertex, _) in flags.iter().enumerate().filter(|(_, on)| **on) {
                if let Some(&height) = all.get(vertex) {
                    sum += f64::from(height);
                    count += 1;
                }
            }
        }
        (sum, count)
    }

    /// Write a new height to every selected vertex, and return the edits,
    /// already applied, with the normals that follow.
    fn rewrite(&self, tile: &mut AdtFile, to: impl Fn(f32) -> f32) -> Vec<Edit> {
        let mut edits = Vec::new();
        for (&index, flags) in &self.chunks {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let before = heights::heights(chunk);
            if before.is_empty() {
                continue;
            }
            let after: Vec<f32> = before
                .iter()
                .enumerate()
                .map(|(vertex, &height)| match flags.get(vertex) {
                    Some(true) => to(height),
                    _ => height,
                })
                .collect();
            if after == before {
                continue;
            }
            if let Some(chunk) = tile.chunk_mut(index) {
                heights::set_heights(chunk, &after);
            }
            edits.push(Edit::Heights {
                chunk: index,
                before,
                after,
            });
        }
        if !edits.is_empty() {
            reshade(tile, &mut edits);
        }
        edits
    }

    /// Move every selected vertex up or down by `by` yards. The shape the
    /// vertices make is kept.
    pub fn shift(&self, tile: &mut AdtFile, by: f32) -> Vec<Edit> {
        self.rewrite(tile, |height| height + by)
    }

    /// Put every selected vertex at one height.
    pub fn level(&self, tile: &mut AdtFile, to: f32) -> Vec<Edit> {
        self.rewrite(tile, |_| to)
    }
}
