//! A tile's ground as two pictures, written out and read back: its heights as
//! a 16-bit height map, and its texture blends as one RGB image.
//!
//! These are the route to and from a tool outside this one: a height map is
//! what terrain generators and paint programs exchange, and a blend map is
//! the same for where a texture goes.
//!
//! ## The height map
//!
//! ```text
//! 257 x 257 pixels, 16 bits each, north at the top and west at the left
//!
//! even column, even row   an outer vertex: 129 x 129 of them over the tile
//! odd column, odd row     an inner vertex: 128 x 128, one per cell
//! any other pixel         not a vertex: the mean of the two outer vertices
//!                         beside it, written so the picture has no gaps,
//!                         and not read back
//! ```
//!
//! A chunk is 9 x 9 outer vertices and 8 x 8 inner ones, and neighbouring
//! chunks share a row or a column of outer vertices, so sixteen chunks a side
//! come to 129 outer vertices a side. A shared vertex is one pixel: the
//! export reads it from one of its chunks and the import writes it to every
//! chunk that has it, which keeps the chunks welded.
//!
//! A pixel is a fraction of a height range, 0 for [`HeightMap::low`] and
//! 65535 for [`HeightMap::high`]. The range is not in the pixels, so it is
//! the caller's to keep or to ask for. An export uses the lowest and highest
//! vertex of what it covers, which spends all sixteen bits on it: over a
//! 200-yard range one step is a ninth of an inch.
//!
//! ## A block of tiles is one picture
//!
//! Tiles side by side share their edge vertices, so a block of `w` x `h`
//! tiles is `w * 256 + 1` by `h * 256 + 1` pixels, tile `(i, j)` of the block
//! starting at pixel `(i * 256, j * 256)`; a blend picture is `w * 1024` by
//! `h * 1024`. [`window`] cuts one tile's picture out of a block and
//! [`place`] puts one in, and [`resample`] fits a picture of any other size
//! to the block first, so a picture drawn elsewhere can be imported.
//!
//! An import writes only the vertices whose pixel differs from the pixel
//! their present height exports to. A map exported and read straight back
//! therefore changes nothing, where writing every vertex would move each by
//! up to half a step and mark all 256 chunks changed.
//!
//! ## The blend map
//!
//! ```text
//! 1024 x 1024 pixels, 8 bits a channel, the same way up
//!
//! red, green, blue        the blend maps of a chunk's second, third and
//!                         fourth texture, over the base, which has none
//! one chunk               a 64 x 64 cell
//! ```
//!
//! It says how the textures a chunk already has are blended and not which
//! textures they are. An import writes the channels a chunk has layers for
//! and ignores the rest; it never adds or removes a layer. A chunk stored at
//! four bits a texel keeps sixteen levels on the way back.
//!
//! ## The tile's own edges
//!
//! The vertices on a tile's four sides are also the neighbouring tiles'. An
//! import moves this tile's copy alone, so a height map that changes the
//! edge leaves a step there until the neighbour is imported to match or the
//! border is stitched.

use super::{reshade, ChunkPaint, Edit};
use crate::adt::{alpha, heights, AdtFile};
use vale_assets::world::adt::{ALPHA_LEN, ALPHA_SIDE, CHUNKS_PER_SIDE};

/// Pixels along one side of a height map: two per cell of the vertex grid,
/// and one more.
pub const HEIGHT_SIDE: usize = CHUNKS_PER_SIDE * 16 + 1;

/// Pixels along one side of a blend map.
pub const BLEND_SIDE: usize = CHUNKS_PER_SIDE * ALPHA_SIDE;

/// A tile's heights as a picture. See the module comment.
#[derive(Debug, Clone, PartialEq)]
pub struct HeightMap {
    /// The height a pixel of 0 stands for, in yards.
    pub low: f32,
    /// The height a pixel of 65535 stands for.
    pub high: f32,
    /// [`HEIGHT_SIDE`] squared, row by row from the north.
    pub pixels: Vec<u16>,
}

impl HeightMap {
    fn pixel_of(&self, height: f32) -> u16 {
        let span = (self.high - self.low).max(f32::EPSILON);
        (((height - self.low) / span).clamp(0.0, 1.0) * 65535.0).round() as u16
    }

    fn height_of(&self, pixel: u16) -> f32 {
        self.low + f32::from(pixel) / 65535.0 * (self.high - self.low)
    }
}

/// Which vertex of which chunk a height-map pixel is, when it is one.
///
/// Returns every `(chunk, vertex)` the pixel stands for: one for an inner
/// vertex, and up to four for an outer vertex on a chunk corner.
fn vertices_at(column: usize, row: usize) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    match (column % 2, row % 2) {
        (1, 1) => {
            let (cx, cy) = (column / 16, row / 16);
            if let Some(vertex) = heights::inner((row % 16) / 2, (column % 16) / 2) {
                found.push((cy * CHUNKS_PER_SIDE + cx, vertex));
            }
        }
        (0, 0) => {
            // An outer vertex on a chunk's last row or column is also the
            // next chunk's first.
            let owners = |at: usize| -> Vec<(usize, usize)> {
                let (chunk, within) = (at / 16, (at % 16) / 2);
                let mut owners = Vec::new();
                if chunk < CHUNKS_PER_SIDE {
                    owners.push((chunk, within));
                }
                if within == 0 && chunk > 0 {
                    owners.push((chunk - 1, 8));
                }
                owners
            };
            for (cy, vertex_row) in owners(row) {
                for (cx, vertex_column) in owners(column) {
                    if let Some(vertex) = heights::outer(vertex_row, vertex_column) {
                        found.push((cy * CHUNKS_PER_SIDE + cx, vertex));
                    }
                }
            }
        }
        _ => {}
    }
    found
}

/// Every chunk's heights, or `None` for a tile that is missing a chunk.
fn all_heights(tile: &AdtFile) -> Option<Vec<Vec<f32>>> {
    (0..CHUNKS_PER_SIDE * CHUNKS_PER_SIDE)
        .map(|index| {
            let found = heights::heights(tile.chunk(index)?);
            (found.len() == heights::VERTICES).then_some(found)
        })
        .collect()
}

/// The tile's lowest and highest vertex. `None` for a tile without all 256
/// chunks.
pub fn height_range(tile: &AdtFile) -> Option<(f32, f32)> {
    let all = all_heights(tile)?;
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for height in all.iter().flatten() {
        low = low.min(*height);
        high = high.max(*height);
    }
    Some((low, high))
}

/// A range to export over: a flat stretch has none to divide by, so it is
/// given a yard, and its one height is pixel 0 and reads back as itself.
pub fn widened((low, high): (f32, f32)) -> (f32, f32) {
    (low, high.max(low + 1.0))
}

/// The tile's heights as a picture, over the range of its own lowest and
/// highest vertex. `None` for a tile without all 256 chunks.
pub fn export_heights(tile: &AdtFile) -> Option<HeightMap> {
    let (low, high) = widened(height_range(tile)?);
    export_heights_over(tile, low, high)
}

/// …over a range the caller chose, as a block of tiles shares one.
pub fn export_heights_over(tile: &AdtFile, low: f32, high: f32) -> Option<HeightMap> {
    let all = all_heights(tile)?;
    let mut map = HeightMap {
        low,
        high,
        pixels: vec![0; HEIGHT_SIDE * HEIGHT_SIDE],
    };
    let height_at = |column: usize, row: usize| -> Option<f32> {
        let &(chunk, vertex) = vertices_at(column, row).first()?;
        Some(all[chunk][vertex])
    };
    for row in 0..HEIGHT_SIDE {
        for column in 0..HEIGHT_SIDE {
            let height = match (column % 2, row % 2) {
                // Between two outer vertices along a row or a column.
                (1, 0) => height_at(column - 1, row).zip(height_at(column + 1, row)),
                (0, 1) => height_at(column, row - 1).zip(height_at(column, row + 1)),
                _ => height_at(column, row).map(|height| (height, height)),
            };
            if let Some((a, b)) = height {
                map.pixels[row * HEIGHT_SIDE + column] = map.pixel_of((a + b) / 2.0);
            }
        }
    }
    Some(map)
}

/// Write a height map onto the tile, and return the edits, already applied:
/// one [`Edit::Heights`] per chunk that changed and the normals that follow.
///
/// Empty for a picture of the wrong size, a tile without all its chunks, and
/// a picture that says what the tile already holds.
pub fn import_heights(tile: &mut AdtFile, map: &HeightMap) -> Vec<Edit> {
    if map.pixels.len() != HEIGHT_SIDE * HEIGHT_SIDE || map.high <= map.low {
        return Vec::new();
    }
    let Some(before) = all_heights(tile) else {
        return Vec::new();
    };
    let mut after = before.clone();
    for row in 0..HEIGHT_SIDE {
        for column in 0..HEIGHT_SIDE {
            let pixel = map.pixels[row * HEIGHT_SIDE + column];
            for (chunk, vertex) in vertices_at(column, row) {
                // Only where the picture says something else. See the module
                // comment.
                if map.pixel_of(before[chunk][vertex]) != pixel {
                    after[chunk][vertex] = map.height_of(pixel);
                }
            }
        }
    }
    let mut edits = Vec::new();
    for (chunk, (was, now)) in before.into_iter().zip(after).enumerate() {
        if was == now {
            continue;
        }
        if let Some(target) = tile.chunk_mut(chunk) {
            heights::set_heights(target, &now);
        }
        edits.push(Edit::Heights {
            chunk,
            before: was,
            after: now,
        });
    }
    reshade(tile, &mut edits);
    edits
}

/// One tile's square of `side` pixels, `channels` values each, cut out of a
/// block `width` pixels wide at pixel `(x, y)`.
pub fn window<T: Copy>(block: &[T], width: usize, channels: usize, side: usize, (x, y): (usize, usize)) -> Vec<T> {
    let mut out = Vec::with_capacity(side * side * channels);
    for row in 0..side {
        let from = ((y + row) * width + x) * channels;
        out.extend_from_slice(&block[from..from + side * channels]);
    }
    out
}

/// …and one tile's square put into a block.
pub fn place<T: Copy>(
    block: &mut [T],
    width: usize,
    channels: usize,
    side: usize,
    (x, y): (usize, usize),
    square: &[T],
) {
    for row in 0..side {
        let to = ((y + row) * width + x) * channels;
        let from = row * side * channels;
        block[to..to + side * channels].copy_from_slice(&square[from..from + side * channels]);
    }
}

/// A picture of `from` pixels scaled to `to`, `channels` values a pixel,
/// mixing the four nearest pixels. The corners stay on the corners, so a
/// picture's edge lands on the block's edge. The same size is a copy.
pub fn resample(
    pixels: &[f32],
    channels: usize,
    from: (usize, usize),
    to: (usize, usize),
) -> Vec<f32> {
    if from == to {
        return pixels.to_vec();
    }
    let along = |at: usize, of: usize, into: usize| -> (usize, usize, f32) {
        if of <= 1 || into <= 1 {
            return (0, 0, 0.0);
        }
        let x = at as f32 * (of - 1) as f32 / (into - 1) as f32;
        let low = (x.floor() as usize).min(of - 1);
        (low, (low + 1).min(of - 1), x - low as f32)
    };
    let mut out = vec![0.0; to.0 * to.1 * channels];
    for row in 0..to.1 {
        let (y0, y1, fy) = along(row, from.1, to.1);
        for column in 0..to.0 {
            let (x0, x1, fx) = along(column, from.0, to.0);
            for channel in 0..channels {
                let at = |x: usize, y: usize| pixels[(y * from.0 + x) * channels + channel];
                let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
                let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
                out[(row * to.0 + column) * channels + channel] = top + (bottom - top) * fy;
            }
        }
    }
    out
}

/// The tile's texture blends as one RGB picture, [`BLEND_SIDE`] squared,
/// three bytes a pixel. A chunk with fewer than four textures leaves the
/// channels it has no layer for at zero.
pub fn export_blend(tile: &AdtFile) -> Vec<u8> {
    let mut rgb = vec![0u8; BLEND_SIDE * BLEND_SIDE * 3];
    for index in 0..CHUNKS_PER_SIDE * CHUNKS_PER_SIDE {
        let Some(chunk) = tile.chunk(index) else {
            continue;
        };
        let paint = alpha::paint(chunk);
        let (cx, cy) = (index % CHUNKS_PER_SIDE, index / CHUNKS_PER_SIDE);
        for (channel, map) in paint.maps.iter().skip(1).take(3).enumerate() {
            for texel in 0..ALPHA_LEN.min(map.len()) {
                let (tx, ty) = (texel % ALPHA_SIDE, texel / ALPHA_SIDE);
                let at = (cy * ALPHA_SIDE + ty) * BLEND_SIDE + cx * ALPHA_SIDE + tx;
                rgb[at * 3 + channel] = map[texel];
            }
        }
    }
    rgb
}

/// Write a blend picture onto the tile, and return the edits, already
/// applied: one [`Edit::Paint`] per chunk whose blends changed.
///
/// A chunk takes the channels it has layers for. No layer is added or
/// removed, and no texture is named. Empty for a picture of the wrong size.
pub fn import_blend(tile: &mut AdtFile, rgb: &[u8]) -> Vec<Edit> {
    if rgb.len() != BLEND_SIDE * BLEND_SIDE * 3 {
        return Vec::new();
    }
    let mut edits = Vec::new();
    for index in 0..CHUNKS_PER_SIDE * CHUNKS_PER_SIDE {
        let Some(chunk) = tile.chunk(index) else {
            continue;
        };
        let mut paint = alpha::paint(chunk);
        if paint.len() < 2 {
            continue;
        }
        let (cx, cy) = (index % CHUNKS_PER_SIDE, index / CHUNKS_PER_SIDE);
        let mut moved = false;
        for (channel, map) in paint.maps.iter_mut().skip(1).take(3).enumerate() {
            for texel in 0..ALPHA_LEN.min(map.len()) {
                let (tx, ty) = (texel % ALPHA_SIDE, texel / ALPHA_SIDE);
                let at = (cy * ALPHA_SIDE + ty) * BLEND_SIDE + cx * ALPHA_SIDE + tx;
                let value = rgb[at * 3 + channel];
                if map[texel] != value {
                    map[texel] = value;
                    moved = true;
                }
            }
        }
        if !moved {
            continue;
        }
        let before = ChunkPaint::capture(tile, index);
        if let Some(chunk) = tile.chunk_mut(index) {
            alpha::set_paint(chunk, &paint);
        }
        // A chunk stored at four bits a texel rounds what it is given, so
        // the bytes may be what they were.
        let after = ChunkPaint::capture(tile, index);
        if after != before {
            edits.push(Edit::Paint {
                chunk: index,
                before: Box::new(before),
                after: Box::new(after),
            });
        }
    }
    edits
}
