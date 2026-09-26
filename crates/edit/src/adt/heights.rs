//! `MCVT` and `MCNR`: the shape of the ground, and the light on it.
//!
//! ## The 145 vertices
//!
//! A map chunk holds nine rows of nine outer vertices interleaved with eight
//! rows of eight inner ones, in the order
//! `9, 8, 9, 8, … 9` — 145 in all. The layout rule is
//! `vale_assets::world::adt`'s and is not restated here: [`vertex_offset`]
//! is the one place this crate converts an index to a position, and it agrees
//! with `wedge_in` by construction. Rows run along **decreasing** world x from
//! the chunk origin and columns along decreasing world y, because the origin is
//! the chunk's maximum corner.
//!
//! `MCVT` stores each height relative to the chunk's own `position.z`.
//! [`heights`] adds it and [`set_heights`] takes it off again, so a caller only
//! ever sees world heights.
//!
//! ## The normals are a shading term and are recomputed, not read
//!
//! `MCNR` is three signed bytes per vertex, in the world's own axes and in the
//! order x, y, z, each `n * 127`. Measured on the flattest chunk of
//! `Azeroth_32_48` — height standard deviation 0.067 yards — every triple reads
//! `(0, 0, 126)`, and 127 is the largest value anywhere in the tile. So the
//! third byte is up and the scale is 127.
//!
//! These are the shading normals: smoothed across cell boundaries and authored
//! to make the lighting look right. They are not what a character stands on —
//! that is the geometric normal of the wedge under the foot, which
//! `vale_assets` computes from the heights and never from here. Raising the
//! ground without calling [`recompute_normals`] leaves the old shading on the
//! new shape, which reads as a hill lit from the wrong side.

use super::{AdtFile, Region};
use vale_assets::world::adt::{INNER_SIDE, OUTER_SIDE, UNIT_SIZE};

/// Heights, and normals, per map chunk.
pub const VERTICES: usize = OUTER_SIDE * OUTER_SIDE + INNER_SIDE * INNER_SIDE;

/// Vertices in one interleaved pair of rows: nine outer then eight inner.
const STRIDE: usize = OUTER_SIDE + INNER_SIDE;

/// Where a vertex sits, as a distance from the chunk origin along decreasing
/// world x and y. Both are positive and at most [`CHUNK_SIZE`].
///
/// [`CHUNK_SIZE`]: vale_assets::world::adt::CHUNK_SIZE
pub fn vertex_offset(index: usize) -> (f32, f32) {
    let row = index / STRIDE;
    let within = index % STRIDE;
    if within < OUTER_SIDE {
        (row as f32 * UNIT_SIZE, within as f32 * UNIT_SIZE)
    } else {
        (
            (row as f32 + 0.5) * UNIT_SIZE,
            (within - OUTER_SIDE) as f32 * UNIT_SIZE + 0.5 * UNIT_SIZE,
        )
    }
}

/// The index of an outer vertex, or `None` off the 9x9 grid.
pub fn outer(row: usize, col: usize) -> Option<usize> {
    (row < OUTER_SIDE && col < OUTER_SIDE).then(|| row * STRIDE + col)
}

/// The index of an inner vertex, or `None` off the 8x8 grid.
pub fn inner(row: usize, col: usize) -> Option<usize> {
    (row < INNER_SIDE && col < INNER_SIDE).then(|| row * STRIDE + OUTER_SIDE + col)
}

/// The world position of one vertex of one chunk.
pub fn vertex_position(origin: [f32; 3], index: usize, height: f32) -> [f32; 3] {
    let (dx, dy) = vertex_offset(index);
    [origin[0] - dx, origin[1] - dy, height]
}

/// A chunk's 145 heights in world terms, with its `position.z` added.
///
/// Empty when the chunk has no `MCVT`, which no 1.12 tile does.
pub fn heights(chunk: &super::MapChunk) -> Vec<f32> {
    let Some(mcvt) = chunk.region(Region::Heights) else {
        return Vec::new();
    };
    let base = chunk.head().position()[2];
    (0..VERTICES)
        .map(|i| {
            let at = i * 4;
            if at + 4 > mcvt.data.len() {
                return base;
            }
            let mut word = [0u8; 4];
            word.copy_from_slice(&mcvt.data[at..at + 4]);
            f32::from_le_bytes(word) + base
        })
        .collect()
}

/// Write 145 world heights back, relative to the chunk's own `position.z`.
///
/// The z is left where it is: it is the tile's own reference and moving it moves
/// every height in the chunk at once. `MCVT` is a fixed 580 bytes, so this never
/// changes the length of anything and nothing after it moves.
pub fn set_heights(chunk: &mut super::MapChunk, heights: &[f32]) {
    let base = chunk.head().position()[2];
    let mut data = Vec::with_capacity(VERTICES * 4);
    for i in 0..VERTICES {
        let h = heights.get(i).copied().unwrap_or(base) - base;
        data.extend_from_slice(&h.to_le_bytes());
    }
    match chunk.region_mut(Region::Heights) {
        Some(mcvt) => mcvt.set(data),
        None => chunk.regions[Region::Heights as usize] = Some(super::SubChunk::new(data)),
    }
}

/// Recompute one chunk's `MCNR` from the heights of the whole tile.
///
/// The gradient at a vertex needs its neighbours, and at a chunk edge those live
/// in the next chunk. Reaching across is what stops a raised hill from being lit
/// with a visible seam on the chunk boundary: within the tile every edge vertex
/// has a real neighbour, and only the four sides of the tile itself fall back to
/// a one-sided difference.
///
/// The neighbour is found by index rather than by position: a chunk's `MCIN`
/// slot is `index_y * 16 + index_x`, `index_x` grows along decreasing world y
/// and `index_y` along decreasing world x. Both were measured against
/// `Azeroth_32_48`, whose chunk 1 sits one column west of chunk 0 and whose
/// chunk 16 sits one row south.
pub fn recompute_normals(tile: &mut AdtFile, chunk_index: usize) {
    let Some(chunk) = tile.chunk(chunk_index) else {
        return;
    };
    if chunk.region(Region::Normals).is_none() {
        return;
    }
    let (ix, iy) = (chunk_index % 16, chunk_index / 16);

    // The tile-wide outer grid this chunk needs: its own 9x9 plus one ring, so
    // a central difference at row 8 or column 8 has something to reach for.
    let own = heights(chunk);
    let neighbour = |dx: i32, dy: i32| -> Option<Vec<f32>> {
        let (x, y) = (ix as i32 + dx, iy as i32 + dy);
        (0..16).contains(&x)
            .then(|| ())
            .and_then(|()| (0..16).contains(&y).then(|| ()))
            .and_then(|()| tile.chunk(y as usize * 16 + x as usize))
            .map(heights)
    };
    // `index_x` grows along decreasing world y, and +Y is west, so the chunk one
    // column back is the one further west. `index_y` grows along decreasing
    // world x, and +X is north, so the row after is the one further south.
    let west = neighbour(-1, 0);
    let east = neighbour(1, 0);
    let north = neighbour(0, -1);
    let south = neighbour(0, 1);

    // An outer vertex by row and column, where either may run one past the grid
    // in each direction and be answered by a neighbouring chunk. Row 9 of this
    // chunk is row 1 of the chunk one row south, because row 8 and row 0 are the
    // same vertex.
    let at = |row: i32, col: i32| -> Option<f32> {
        let (grid, row, col) = if row < 0 {
            (north.as_ref()?, row + OUTER_SIDE as i32 - 1, col)
        } else if row >= OUTER_SIDE as i32 {
            (south.as_ref()?, row - OUTER_SIDE as i32 + 1, col)
        } else {
            (&own, row, col)
        };
        let (grid, col) = if col < 0 {
            (west.as_ref()?, col + OUTER_SIDE as i32 - 1)
        } else if col >= OUTER_SIDE as i32 {
            (east.as_ref()?, col - OUTER_SIDE as i32 + 1)
        } else {
            (grid, col)
        };
        // Only one of the two can leave this chunk in one step: a diagonal
        // neighbour is never asked for, because a central difference moves along
        // one axis at a time.
        outer(row as usize, col as usize).and_then(|i| grid.get(i).copied())
    };

    let mut normals = vec![0i8; VERTICES * 3];
    let mut put = |index: usize, n: [f32; 3]| {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
        for k in 0..3 {
            let scaled = (n[k] / len * 127.0).round().clamp(-127.0, 127.0);
            normals[index * 3 + k] = scaled as i8;
        }
    };

    for row in 0..OUTER_SIDE {
        for col in 0..OUTER_SIDE {
            let (r, c) = (row as i32, col as i32);
            let here = at(r, c).unwrap_or(0.0);
            // A one-sided difference at the tile edge, a central one everywhere
            // else. `slope` is dz per yard along increasing world x or y, and
            // the grid runs the other way, so the two heights go in backwards.
            let dzdx = slope(at(r - 1, c), here, at(r + 1, c));
            let dzdy = slope(at(r, c - 1), here, at(r, c + 1));
            put(outer(row, col).unwrap(), [-dzdx, -dzdy, 1.0]);
        }
    }
    for row in 0..INNER_SIDE {
        for col in 0..INNER_SIDE {
            let (r, c) = (row as i32, col as i32);
            // The inner vertex sits at the centre of the cell whose corners are
            // the four outer vertices around it, so its gradient is the cell's.
            let corner = |dr: i32, dc: i32| at(r + dr, c + dc).unwrap_or(0.0);
            let dzdx = ((corner(0, 0) + corner(0, 1)) - (corner(1, 0) + corner(1, 1)))
                / (2.0 * UNIT_SIZE);
            let dzdy = ((corner(0, 0) + corner(1, 0)) - (corner(0, 1) + corner(1, 1)))
                / (2.0 * UNIT_SIZE);
            put(inner(row, col).unwrap(), [-dzdx, -dzdy, 1.0]);
        }
    }

    let bytes: Vec<u8> = normals.into_iter().map(|v| v as u8).collect();
    if let Some(mcnr) = tile
        .chunk_mut(chunk_index)
        .and_then(|c| c.region_mut(Region::Normals))
    {
        // `set` replaces the payload and its size field together. The thirteen
        // bytes of padding are not touched: they sit after the payload, no size
        // field covers them, and the next region's offset is what places them.
        mcnr.set(bytes);
    }
}

/// The slope through a vertex, in dz per yard along increasing world x or y.
///
/// The grid runs along *decreasing* world coordinates, so the neighbour at a
/// lower index is at a higher world position and the subtraction is that way
/// round. A vertex with only one neighbour — the four sides of a tile — gets the
/// one-sided difference rather than nothing.
fn slope(lower_index: Option<f32>, here: f32, higher_index: Option<f32>) -> f32 {
    match (lower_index, higher_index) {
        (Some(a), Some(b)) => (a - b) / (2.0 * UNIT_SIZE),
        (Some(a), None) => (a - here) / UNIT_SIZE,
        (None, Some(b)) => (here - b) / UNIT_SIZE,
        (None, None) => 0.0,
    }
}

/// One chunk as `vale_assets`' own ground query wants it.
///
/// [`vale_assets::world::adt::ChunkGround::height_at`] is the rule for what
/// the ground is between the vertices, and it is not a bilinear one: a cell is
/// four triangles meeting at the inner-grid vertex in its centre, and answering
/// any other way puts an editor's cursor at a different height from the one the
/// character walks on.
pub fn ground(chunk: &super::MapChunk) -> vale_assets::world::adt::ChunkGround {
    let head = chunk.head();
    vale_assets::world::adt::ChunkGround {
        position: head.position(),
        heights: heights(chunk),
        holes: head.holes(),
    }
}

/// How far outside a chunk's square a position may be and still be answered by
/// it, in yards.
///
/// **The chunk origins in the file do not tile exactly.** They are `f32` and
/// they step by 33.332032 yards where `CHUNK_SIZE` is 33.333332, so each square
/// begins 0.00065 yards below where the one above it ended and sixteen of them
/// span 533.314 against the tile's 533.333. Two consequences, both measured on
/// `Azeroth_32_48`: there is a 0.00065-yard band between every pair of adjacent
/// chunks that is inside neither, and the last two centimetres of the tile are
/// inside none at all. A strict containment test answers "no ground" in both.
///
/// That is invisible to the client, which only ever asks about a position a
/// character is standing at, and very visible to an editor, which asks about
/// arbitrary points and draws whatever comes back.
///
/// Two centimetres covers the gap with room to spare and is far below the
/// 4.17-yard cell the answer is interpolated across, so a position pulled this
/// far into a chunk is the same answer to within the width of the line drawing
/// it.
const REACH: f32 = 0.05;

/// The height of the edited ground at a world position, or `None` off this tile
/// and over a hole.
///
/// A position just outside every chunk's square — see [`REACH`] — is answered by
/// the nearest one rather than refused.
pub fn height_at(tile: &AdtFile, x: f32, y: f32) -> Option<f32> {
    use vale_assets::world::adt::CHUNK_SIZE;
    let chunk = tile.chunk(chunk_at(tile, x, y)?)?;
    if contains(chunk.head().position(), x, y) {
        return ground(chunk).height_at(x, y);
    }
    // Just outside, within [`REACH`]: pulled inside the square so the wedge
    // lookup has a cell to answer from. `ChunkGround::contains` is exclusive at
    // the far edge, which is what the nudge is for.
    let origin = chunk.head().position();
    let inside = |o: f32, p: f32| p.clamp(o - CHUNK_SIZE + REACH, o - REACH);
    ground(chunk).height_at(inside(origin[0], x), inside(origin[1], y))
}

/// **The slope of the edited ground at a world position**, as a unit normal in
/// the world's own axes, or `None` off this tile and over a hole.
///
/// The geometric normal of the wedge [`height_at`] answers from —
/// `vale_assets::world::adt::ChunkGround::normal_at`, which says why it is
/// not `MCNR`. What a placement is leaned onto: see
/// `vale_assets::world::adt::lean_to_normal`, which turns it into the two
/// `MDDF` angles. The same allowance at a chunk's edge as `height_at`.
pub fn normal_at(tile: &AdtFile, x: f32, y: f32) -> Option<[f32; 3]> {
    use vale_assets::world::adt::CHUNK_SIZE;
    let chunk = tile.chunk(chunk_at(tile, x, y)?)?;
    if contains(chunk.head().position(), x, y) {
        return ground(chunk).normal_at(x, y);
    }
    let origin = chunk.head().position();
    let inside = |o: f32, p: f32| p.clamp(o - CHUNK_SIZE + REACH, o - REACH);
    ground(chunk).normal_at(inside(origin[0], x), inside(origin[1], y))
}

/// …and the same height **as though the chunk had no holes**.
///
/// [`height_at`] refuses over a hole, which is right for everything that asks
/// where a character stands or where a brush lands: there is no ground there.
/// It is wrong for the one thing that has to aim *at* a hole. A tool that cuts
/// and patches them needs a surface to point at whether or not the ground is
/// still drawn, and the surface it needs is the one the missing cells came out
/// of — the wedge the heights still describe, because a hole takes cells out of
/// the *mesh* and leaves `MCVT` exactly as it was.
///
/// Without it the pointer falls through the moment a square is cut and lands on
/// whatever is behind — the far bank of a lake, the next hillside, or nothing —
/// so a held drag walks somewhere else and cuts a trail of squares nobody asked
/// for. That is what it was reported as.
pub fn solid_height_at(tile: &AdtFile, x: f32, y: f32) -> Option<f32> {
    use vale_assets::world::adt::CHUNK_SIZE;
    let chunk = tile.chunk(chunk_at(tile, x, y)?)?;
    let mut solid = ground(chunk);
    solid.holes = 0;
    if contains(chunk.head().position(), x, y) {
        return solid.height_at(x, y);
    }
    let origin = chunk.head().position();
    let inside = |o: f32, p: f32| p.clamp(o - CHUNK_SIZE + REACH, o - REACH);
    solid.height_at(inside(origin[0], x), inside(origin[1], y))
}

/// Which of a tile's 256 map chunks a world position is over, or `None` when it
/// is over none of them.
///
/// The chunk search [`height_at`] was doing inline, lifted out because a tool
/// wants the chunk itself: what it is painted with, which zone it is in, where
/// its holes are. **It is a search and not arithmetic** for the reason the
/// architecture doc records — `MCNK`'s stored origins step by 33.332032 where
/// `CHUNK_SIZE` is 33.333332, so the squares do not tile exactly and a position
/// computed from the tile corner lands in the wrong one near a boundary. Asking
/// each chunk's own origin is 256 comparisons and is the only answer that agrees
/// with the file.
///
/// A position in the 0.65-millimetre band between two squares, or in the last
/// two centimetres of the tile, is answered by the nearest chunk within
/// [`REACH`] rather than refused — which is the same allowance `height_at` has
/// always made and the reason it is made here instead.
pub fn chunk_at(tile: &AdtFile, x: f32, y: f32) -> Option<usize> {
    use vale_assets::world::adt::CHUNK_SIZE;
    // How far outside this chunk's square the position is, in the worse of the
    // two axes. Zero when it is inside.
    let outside = |origin: [f32; 3]| {
        let axis = |o: f32, p: f32| {
            let d = o - p;
            if d < 0.0 {
                -d
            } else if d >= CHUNK_SIZE {
                d - CHUNK_SIZE
            } else {
                0.0
            }
        };
        axis(origin[0], x).max(axis(origin[1], y))
    };

    let mut nearest: Option<(f32, usize)> = None;
    for (i, chunk) in tile.chunks.iter().enumerate() {
        let origin = chunk.head().position();
        // **The same predicate `ChunkGround::height_at` uses**, and not a gap of
        // zero. Both bounds are exclusive at the far edge there, so a position
        // exactly `CHUNK_SIZE` from the origin has a gap of zero and is still
        // refused; taking that as "inside" returned `None` from a chunk that had
        // already been chosen, instead of falling through to the nudge above.
        if contains(origin, x, y) {
            return Some(i);
        }
        let gap = outside(origin);
        if nearest.is_none_or(|(had, _)| gap < had) {
            nearest = Some((gap, i));
        }
    }
    nearest
        .filter(|(gap, _)| *gap <= REACH)
        .map(|(_, i)| i)
}

/// Whether a chunk's square holds a position, on
/// [`vale_assets::world::adt::ChunkGround::contains`]' own terms: the origin
/// is the maximum corner and the far edge is exclusive.
fn contains(origin: [f32; 3], x: f32, y: f32) -> bool {
    use vale_assets::world::adt::CHUNK_SIZE;
    (0.0..CHUNK_SIZE).contains(&(origin[0] - x)) && (0.0..CHUNK_SIZE).contains(&(origin[1] - y))
}

/// A chunk's 145 normals, decoded.
///
/// `vale_assets::world::adt::decode_normals` is the encoding and this only
/// hands it the region. Straight up for a chunk with no `MCNR`, which is what
/// the parser answers for one too.
pub fn normals(chunk: &super::MapChunk) -> Vec<[f32; 3]> {
    let bytes = chunk
        .region(Region::Normals)
        .map(|sub| sub.data.as_slice())
        .unwrap_or(&[]);
    vale_assets::world::adt::decode_normals(bytes)
}
