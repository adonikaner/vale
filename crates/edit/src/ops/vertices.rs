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
//! A set is also the editor's record of **locked** vertices: the ones no
//! operation may move. [`Selected::hold`] takes the edits any operation has
//! made and puts every locked vertex back, so the lock does not depend on
//! each operation knowing about it. [`Selected::to_text`] and
//! [`Selected::from_text`] are the form a project stores a tile's locks in.
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

    /// Put every entry of `other` into the set. Returns how many were not
    /// already in it.
    pub fn add(&mut self, other: &Selected) -> usize {
        let mut added = 0;
        for (&index, flags) in &other.chunks {
            let mine = self
                .chunks
                .entry(index)
                .or_insert_with(|| vec![false; heights::VERTICES]);
            for (vertex, &on) in flags.iter().enumerate() {
                if on && !mine[vertex] {
                    mine[vertex] = true;
                    added += 1;
                }
            }
        }
        added
    }

    /// Take every entry of `other` out of the set. Returns how many were in it.
    pub fn take(&mut self, other: &Selected) -> usize {
        let mut taken = 0;
        for (&index, flags) in &other.chunks {
            let Some(mine) = self.chunks.get_mut(&index) else {
                continue;
            };
            for (vertex, &on) in flags.iter().enumerate() {
                if on && mine[vertex] {
                    mine[vertex] = false;
                    taken += 1;
                }
            }
            if !mine.contains(&true) {
                self.chunks.remove(&index);
            }
        }
        taken
    }

    /// The vertices on the tile's four sides: the outer vertices of the edge
    /// chunks that lie on the tile's boundary. Each is also a vertex of the
    /// tile beside it, so a set holding these keeps the ground at the seam
    /// where it is.
    ///
    /// Found by world position rather than by chunk and row number, so it does
    /// not depend on which way the chunk grid runs.
    pub fn tile_sides(tile: &AdtFile) -> Selected {
        const EPSILON: f32 = 0.01;
        let origins: Vec<(usize, [f32; 3])> = (0..tile.chunks.len())
            .filter_map(|index| tile.chunk(index).map(|chunk| (index, chunk.head().position())))
            .collect();
        let mut set = Selected::default();
        let (Some(top_x), Some(top_y)) = (
            origins.iter().map(|(_, at)| at[0]).reduce(f32::max),
            origins.iter().map(|(_, at)| at[1]).reduce(f32::max),
        ) else {
            return set;
        };
        let side = 16.0 * CHUNK_SIZE;
        let (low_x, low_y) = (top_x - side, top_y - side);
        let on_side = |p: f32, top: f32, low: f32| (p - top).abs() < EPSILON || (p - low).abs() < EPSILON;
        for (index, origin) in origins {
            for row in 0..9 {
                for col in 0..9 {
                    let Some(vertex) = heights::outer(row, col) else {
                        continue;
                    };
                    let (dx, dy) = heights::vertex_offset(vertex);
                    let (x, y) = (origin[0] - dx, origin[1] - dy);
                    if on_side(x, top_x, low_x) || on_side(y, top_y, low_y) {
                        set.chunks
                            .entry(index)
                            .or_insert_with(|| vec![false; heights::VERTICES])[vertex] = true;
                    }
                }
            }
        }
        set
    }

    /// Put every vertex in the set back at the height it had before `edits`,
    /// which have already been written into `tile`. This is how a set of locked
    /// vertices is kept: every operation that moves the ground passes its edits
    /// through here before they are recorded, so the lock holds whatever the
    /// operation was.
    ///
    /// A height edit left with nothing changed is removed. The normals of the
    /// chunks round a held vertex are worked out again from the held heights,
    /// and each normals edit keeps the `before` the operation recorded, so an
    /// undo still restores the shading it found. Returns how many entries were
    /// held.
    pub fn hold(&self, tile: &mut AdtFile, edits: &mut Vec<Edit>) -> usize {
        if self.is_empty() {
            return 0;
        }
        let mut held = 0;
        let mut written: Vec<(usize, Vec<f32>)> = Vec::new();
        for edit in edits.iter_mut() {
            let Edit::Heights { chunk, before, after } = edit else {
                continue;
            };
            let Some(flags) = self.chunks.get(chunk) else {
                continue;
            };
            let mut changed = false;
            for (vertex, &locked) in flags.iter().enumerate() {
                if !locked {
                    continue;
                }
                if let (Some(was), Some(now)) = (before.get(vertex), after.get_mut(vertex)) {
                    if now != was {
                        *now = *was;
                        held += 1;
                        changed = true;
                    }
                }
            }
            if changed {
                written.push((*chunk, after.clone()));
            }
        }
        if held == 0 {
            return 0;
        }
        for (chunk, after) in &written {
            if let Some(mesh) = tile.chunk_mut(*chunk) {
                heights::set_heights(mesh, after);
            }
        }
        edits.retain(|edit| !matches!(edit, Edit::Heights { before, after, .. } if before == after));

        // The normals, again. The first `before` recorded for a chunk is the
        // shading the operation found; later ones are its own intermediate
        // states.
        let mut found: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
        edits.retain(|edit| match edit {
            Edit::Normals { chunk, before, .. } => {
                found.entry(*chunk).or_insert_with(|| before.clone());
                false
            }
            _ => true,
        });
        let mut shade: std::collections::BTreeSet<usize> = found.keys().copied().collect();
        for (chunk, _) in &written {
            shade.extend(super::with_neighbours(*chunk));
        }
        for index in shade {
            let Some(now) = super::normals_of(tile, index) else {
                continue;
            };
            let before = found.remove(&index).unwrap_or(now);
            heights::recompute_normals(tile, index);
            let after = super::normals_of(tile, index).unwrap_or_default();
            if after != before {
                edits.push(Edit::Normals {
                    chunk: index,
                    before,
                    after,
                });
            }
        }
        held
    }

    /// The set as text: one line per chunk, the chunk's index and then its
    /// vertex indices, with a run of consecutive indices written `first-last`.
    /// See [`Self::from_text`].
    pub fn to_text(&self) -> String {
        let mut text = String::from(
            "# Locked terrain vertices of one tile, written by the world editor.\n\
             # Each line: a chunk index (0-255), then vertex indices (0-144) or ranges.\n",
        );
        for (&index, flags) in &self.chunks {
            let mut line = index.to_string();
            let mut vertex = 0;
            while vertex < flags.len() {
                if !flags[vertex] {
                    vertex += 1;
                    continue;
                }
                let first = vertex;
                while vertex + 1 < flags.len() && flags[vertex + 1] {
                    vertex += 1;
                }
                match first == vertex {
                    true => line.push_str(&format!(" {first}")),
                    false => line.push_str(&format!(" {first}-{vertex}")),
                }
                vertex += 1;
            }
            text.push_str(&line);
            text.push('\n');
        }
        text
    }

    /// Read [`Self::to_text`]'s form back. Blank lines and lines starting with
    /// `#` are skipped. A chunk index past 255 or a vertex index past 144 is an
    /// error naming the line.
    pub fn from_text(text: &str) -> Result<Selected, String> {
        let mut set = Selected::default();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = || format!("line {}: {line:?} is not a chunk and its vertices", number + 1);
            let mut words = line.split_whitespace();
            let index: usize = words.next().and_then(|w| w.parse().ok()).ok_or_else(bad)?;
            if index >= 256 {
                return Err(bad());
            }
            let flags = set
                .chunks
                .entry(index)
                .or_insert_with(|| vec![false; heights::VERTICES]);
            for word in words {
                let (first, last) = match word.split_once('-') {
                    Some((a, b)) => (a.parse::<usize>(), b.parse::<usize>()),
                    None => (word.parse::<usize>(), word.parse::<usize>()),
                };
                let (Ok(first), Ok(last)) = (first, last) else {
                    return Err(bad());
                };
                if first > last || last >= heights::VERTICES {
                    return Err(bad());
                }
                for vertex in first..=last {
                    flags[vertex] = true;
                }
            }
        }
        set.chunks.retain(|_, flags| flags.contains(&true));
        Ok(set)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The stored form reads back as the same set, with runs written as
    /// ranges, and a line that does not parse is refused with its number.
    #[test]
    fn a_set_survives_its_text_form() {
        let mut set = Selected::default();
        set.chunks.insert(17, {
            let mut flags = vec![false; heights::VERTICES];
            for vertex in [0, 1, 2, 3, 40, 144] {
                flags[vertex] = true;
            }
            flags
        });
        set.chunks.insert(255, {
            let mut flags = vec![false; heights::VERTICES];
            flags[9] = true;
            flags
        });
        let text = set.to_text();
        assert!(text.contains("\n17 0-3 40 144\n"), "{text}");
        assert!(text.contains("\n255 9\n"), "{text}");
        assert_eq!(Selected::from_text(&text).unwrap(), set);
        assert_eq!(Selected::from_text("").unwrap(), Selected::default());
        assert!(Selected::from_text("3 0-145").unwrap_err().starts_with("line 1:"));
        assert!(Selected::from_text("# x\n256 0").unwrap_err().starts_with("line 2:"));
    }

    /// Adding and taking are set union and difference, and taking the last
    /// entry of a chunk drops the chunk.
    #[test]
    fn sets_add_and_take() {
        let one = Selected::from_text("5 0-4").unwrap();
        let two = Selected::from_text("5 3-8\n6 1").unwrap();
        let mut both = one.clone();
        assert_eq!(both.add(&two), 5);
        assert_eq!(both, Selected::from_text("5 0-8\n6 1").unwrap());
        assert_eq!(both.take(&two), 7);
        assert_eq!(both, Selected::from_text("5 0-2").unwrap());
        assert_eq!(both.take(&one), 3);
        assert!(both.is_empty());
    }
}
