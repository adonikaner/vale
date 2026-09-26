//! `MCLQ`: the water, ocean, lava and slime standing on a map chunk.
//!
//! ## What it holds is stated outside it
//!
//! This is the one region of an `MCNK` that does not describe itself. `MCLQ`
//! carries one 804-byte block per liquid, back to back, and **says nothing
//! about how many or of what kind** — the `MCNK` header's flag bits do, in bit
//! order: river, ocean, magma, slime. So a writer has two things to keep in
//! step, and a reader that trusts either one alone is wrong.
//!
//! Its own size field is worse: it is **zero on every one of the 1,280 chunks
//! of the five real tiles**, wet or dry, and the header's `sizeLiquid` is the
//! only honest statement of the extent. [`crate::adt::SubChunk::set_keeping_size`]
//! is what writes a block back without inventing a number the game never wrote.
//!
//! ## One block, measured
//!
//! ```text
//! 0     f32 min, f32 max          the surface's own range
//! 8     81 x 8 bytes              a 9x9 grid of vertices
//! 656   64 bytes                  an 8x8 grid of tile flags
//! 720   u32 nFlowvs, 2 x 40       flow vectors, always present whatever it says
//! 804   …the next block
//! ```
//!
//! **A vertex is a union the liquid type picks between**, and only two of its
//! eight bytes are understood here: 4..8 are an `f32` height for all four kinds,
//! and it is an **absolute world z** rather than an offset from the chunk's;
//! byte 0 is a *depth* for water and ocean — the opacity's parameter, 0 at the
//! shore and 255 in the deep — and the low half of a texture coordinate for
//! magma and slime, which is `vale_assets`' measurement and not an
//! assumption. **Bytes 1..4 are not understood at all.**
//!
//! **A vertex with no surface over it reads `f32::MAX`.** `0x7F7FFFFF` is what
//! the shipped blocks carry wherever there is no water, and it is what the
//! block's own `min`/`max` are computed *around*: the range covers the vertices
//! that are not `f32::MAX`, which is neither "all of them" (that would be
//! `f32::MAX`) nor "the corners of the wet cells" (a real tile has real heights
//! at vertices no wet cell touches). Measured, on `Azeroth_34_51` chunk 14,
//! after a round trip computed the other two ways disagreed with the file.
//!
//! **A tile flag's low nibble is 15 for dry.** That is the flag that turns a
//! rectangle of water into a pool, and it is why a chunk can declare a liquid
//! and still be mostly dry. The high nibble is not this crate's either.
//!
//! ## A pool is a block of bytes, for the reason an `AdtFile` is
//!
//! [`Pool`] carries the block exactly as the file wrote it and changes the
//! fields an edit names. The first draft decoded it into heights, depths and
//! flags and re-encoded from those, and the round trip failed on the first real
//! tile: the three bytes after the depth are something, the flow tail is
//! something, and a re-encode from what is understood silently drops everything
//! that is not. That is the same argument `crate::adt` makes at the top, one
//! region down.
//!
//! ## The grid runs the way every other per-cell field does
//!
//! Rows along decreasing world x, columns along decreasing world y from the
//! chunk's origin — `vale_assets::world::adt::cell_at` is the one statement
//! of it and this calls it rather than repeating the arithmetic. **The vertex
//! grid is 9x9 over the same cells**, so cell `(row, col)`'s four corners are
//! vertices `(row, col)`, `(row, col+1)`, `(row+1, col)` and `(row+1, col+1)`.
//!
//! Note that `vale_assets`' *parser* re-orders both grids into `WmoLiquid`'s
//! own frame, because that is what the renderer draws from. This crate works in
//! the file's order, which is the order it writes.
//!
//! ## Why an edit is the whole region
//!
//! Giving a dry chunk water adds 804 bytes in the middle of a two-megabyte file
//! and sets a header flag; taking it away removes them and clears one. There is
//! no smaller unit than "this chunk's liquid, and the flags that declare it"
//! that inverts correctly — the same argument `alpha::Paint` makes one region
//! along.

use super::{AdtFile, MapChunk, Region};
use vale_assets::world::adt::{mcnk_flags, INNER_SIDE};
use vale_assets::world::wmo::Liquid;

/// Vertices on a side of the 9x9 grid.
pub const SIDE: usize = INNER_SIDE + 1;
/// …and how many there are.
const VERTS: usize = SIDE * SIDE;
/// …and cells in the 8x8 one.
pub const CELLS: usize = INNER_SIDE * INNER_SIDE;

/// Bytes per vertex — see the module comment on what they mean.
const VERTEX: usize = 8;
/// Where the tile flags start within a block.
const TILES_AT: usize = 8 + VERTS * VERTEX;
/// …and where the flow tail does.
const FLOW_AT: usize = TILES_AT + CELLS;
/// One whole block.
pub const BLOCK: usize = FLOW_AT + 4 + 2 * 40;

/// The low nibble of a tile flag that means **no liquid here**.
pub const DRY: u8 = 0x0F;

/// **The high nibble's two bits**, which the format does not name and which
/// were censused rather than read up — `vale water <Map> <x> <y>` prints
/// the count of each value over a tile's wet cells.
///
/// `0x80` is the one bit vmangos's map extractor reads: `flags & (1 << 7)`
/// marks the cell **deep water**, which is where a swimmer takes fatigue.
/// Measured, it is set on the open sea and nowhere else — 14,492 of 15,511
/// cells on `Azeroth_30_39`, none of Menethil's 9,008, none of Elwynn's lake.
///
/// `0x40` is on nearly every sea and lake cell — all 9,008 at Menethil, all
/// 328 off Westfall, 1,337 of 2,636 on Elwynn's lake tile — and off on the
/// rest of that lake tile's cells and on 147 of 158 river cells at Kalimdor
/// 40,23. Nothing in vmangos reads it and nothing in this project did; the
/// name is the one the format's public reading gives it, **fishable**, and
/// the distribution is consistent with that and with nothing else measured.
pub const FISHABLE: u8 = 0x40;
pub const FATIGUE: u8 = 0x80;

/// …and the height a vertex with no surface over it carries: `f32::MAX`,
/// `0x7F7FFFFF`. See the module comment, and [`Pool::reseat_range`], which is
/// the thing that has to know.
pub const NO_SURFACE: f32 = f32::MAX;

/// The four kinds and the header bit that declares each, **in the order the
/// blocks appear**.
///
/// The order is the whole of how a chunk carrying a river *and* the ocean it
/// flows into is read, and it is the flags' bit order rather than anything
/// `MCLQ` says.
pub const KINDS: [(u32, Liquid); 4] = [
    (mcnk_flags::LQ_RIVER, Liquid::Water),
    (mcnk_flags::LQ_OCEAN, Liquid::Ocean),
    (mcnk_flags::LQ_MAGMA, Liquid::Magma),
    (mcnk_flags::LQ_SLIME, Liquid::Slime),
];

/// One liquid standing on one chunk: its kind, and its block as the file wrote
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pool {
    pub kind: Liquid,
    /// [`BLOCK`] bytes. Read and written through the accessors below; carried
    /// whole so that what this crate does not understand survives an edit.
    bytes: Vec<u8>,
}

impl Pool {
    /// A pool of this kind, **entirely dry**.
    ///
    /// Dry rather than wet, because the tool that makes one then paints the
    /// cells it wants: a chunk that arrived full of water the moment it was
    /// given a kind would be an edit nobody asked for.
    ///
    /// Every vertex is [`NO_SURFACE`], which is the game's own marker for one
    /// with nothing over it, and the flow tail is zeros, which is what a still
    /// pool is. The level is only the range's seed — [`Pool::fill_cell`] writes
    /// the real heights where the brush lands.
    pub fn flat(kind: Liquid, height: f32) -> Pool {
        let mut bytes = vec![0u8; BLOCK];
        bytes[0..4].copy_from_slice(&height.to_le_bytes());
        bytes[4..8].copy_from_slice(&height.to_le_bytes());
        for index in 0..VERTS {
            let at = 8 + index * VERTEX;
            bytes[at + 4..at + 8].copy_from_slice(&NO_SURFACE.to_le_bytes());
        }
        bytes[TILES_AT..TILES_AT + CELLS].fill(DRY);
        Pool { kind, bytes }
    }

    /// The block, as it will be written.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The surface height at one vertex of the 9x9 grid.
    pub fn height(&self, row: usize, col: usize) -> f32 {
        let at = 8 + (row * SIDE + col) * VERTEX + 4;
        f32::from_le_bytes([
            self.bytes[at],
            self.bytes[at + 1],
            self.bytes[at + 2],
            self.bytes[at + 3],
        ])
    }

    /// …and the depth byte, which only water and ocean have — see the module
    /// comment.
    pub fn depth(&self, row: usize, col: usize) -> u8 {
        self.bytes[8 + (row * SIDE + col) * VERTEX]
    }

    /// Whether cell `(row, col)` has liquid on it.
    pub fn is_wet(&self, row: usize, col: usize) -> bool {
        self.bytes
            .get(TILES_AT + row * INNER_SIDE + col)
            .is_some_and(|flag| flag & 0x0F != DRY)
    }

    /// …and set it. Wetting keeps the flag's high nibble, which is the cell's
    /// own — see [`Self::cell_flags`]; drying writes the whole byte back to
    /// [`DRY`], since a dry cell is neither fishable nor deep and that is what
    /// a shipped dry cell reads.
    pub fn set_wet(&mut self, row: usize, col: usize, wet: bool) {
        let Some(flag) = self.bytes.get_mut(TILES_AT + row * INNER_SIDE + col) else {
            return;
        };
        *flag = match wet {
            true => *flag & 0xF0,
            false => DRY,
        };
    }

    /// The cell's high nibble: [`FISHABLE`] and [`FATIGUE`], and two bits no
    /// shipped tile sets.
    pub fn cell_flags(&self, row: usize, col: usize) -> u8 {
        self.bytes
            .get(TILES_AT + row * INNER_SIDE + col)
            .map_or(0, |flag| flag & 0xF0)
    }

    /// …and set it, keeping the low nibble. Only the high four bits of
    /// `flags` are taken.
    pub fn set_cell_flags(&mut self, row: usize, col: usize, flags: u8) {
        let Some(flag) = self.bytes.get_mut(TILES_AT + row * INNER_SIDE + col) else {
            return;
        };
        *flag = (*flag & 0x0F) | (flags & 0xF0);
    }

    /// Whether any cell is wet. A pool with none is one nothing draws, and is
    /// what [`set_pools`] takes as "this liquid is gone".
    pub fn any_wet(&self) -> bool {
        (0..INNER_SIDE).any(|row| (0..INNER_SIDE).any(|col| self.is_wet(row, col)))
    }

    /// Set the surface height and depth at the four corners of one cell.
    ///
    /// The four corners rather than one, because a cell's surface is the quad
    /// its corners make: writing one vertex tilts the quads of the three
    /// neighbours that share it.
    pub fn fill_cell(&mut self, row: usize, col: usize, height: f32, depth: u8) {
        for (dr, dc) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let at = 8 + ((row + dr) * SIDE + col + dc) * VERTEX;
            if at + VERTEX > TILES_AT {
                continue;
            }
            self.bytes[at] = depth;
            self.bytes[at + 4..at + 8].copy_from_slice(&height.to_le_bytes());
        }
    }

    /// **The depth byte for a surface standing this far above the ground**, on
    /// the curve the shipped tiles measure.
    ///
    /// `vale water` reports the relation over 3,522 wet vertices of
    /// `Azeroth_32_49`: byte 0..31 stands +0.6 yards above the ground, 32..95
    /// stands +6.2, and 96 and over stands +12.6. So roughly eight bytes a yard
    /// over the first ten, which is what this is — and it matters because the
    /// byte is the *opacity's* parameter: a pool written with a flat 255 is a
    /// pool with no shallows, and the shore stops fading out.
    pub fn depth_for(above: f32) -> u8 {
        (above.max(0.0) * 8.0).round().clamp(0.0, 255.0) as u8
    }

    /// Put the block's own `min`/`max` back in step with its heights.
    ///
    /// **Over every vertex that is not [`NO_SURFACE`]**, which is the rule the
    /// shipped blocks follow and is not the obvious one. Two other readings
    /// were tried against `Azeroth_34_51` chunk 14 and both disagree with the
    /// file: *all* of them answers `f32::MAX`, and *the corners of the wet
    /// cells* answers short, because a real tile carries heights at vertices no
    /// wet cell touches.
    ///
    /// Called by [`set_pools`], so a caller that moved a height cannot forget
    /// it — and it reproduces the file exactly on a pool nothing changed, which
    /// is what the round-trip check is for.
    fn reseat_range(&mut self) {
        let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
        for row in 0..SIDE {
            for col in 0..SIDE {
                let h = self.height(row, col);
                if h == NO_SURFACE {
                    continue;
                }
                low = low.min(h);
                high = high.max(h);
            }
        }
        if !low.is_finite() {
            return;
        }
        self.bytes[0..4].copy_from_slice(&low.to_le_bytes());
        self.bytes[4..8].copy_from_slice(&high.to_le_bytes());
    }
}

/// Every liquid standing on a chunk, in the order the blocks appear.
pub fn pools(tile: &AdtFile, chunk: usize) -> Vec<Pool> {
    let Some(chunk) = tile.chunk(chunk) else {
        return Vec::new();
    };
    let flags = chunk.head().flags();
    let data = chunk
        .region(Region::Liquid)
        .map(|sub| sub.data.as_slice())
        .unwrap_or(&[]);
    let mut out: Vec<Pool> = Vec::new();
    for (bit, kind) in KINDS {
        if flags & bit == 0 {
            continue;
        }
        let at = out.len() * BLOCK;
        let Some(block) = data.get(at..at + BLOCK) else {
            // A tail shorter than the flags claim. The blocks already read are
            // real; the rest are not invented.
            break;
        };
        out.push(Pool {
            kind,
            bytes: block.to_vec(),
        });
    }
    out
}

/// …and write them back, **with the header flags that declare them**.
///
/// A pool with no wet cell at all is dropped rather than written: its flag would
/// say the chunk has a liquid that draws nothing, which is a lie the file can
/// hold and nothing can see. That is also how a tool removes water — it dries
/// the cells and this notices.
pub fn set_pools(chunk: &mut MapChunk, pools: &[Pool]) {
    let mut data = Vec::with_capacity(pools.len() * BLOCK);
    let mut flags = chunk.head().flags() & !mcnk_flags::LQ_ANY;
    // **In `KINDS` order and not the caller's**, because that is the order the
    // bits are read back in. A caller handing them over the other way round
    // would write a river's block where the ocean's flag says the ocean's is.
    for (bit, kind) in KINDS {
        let Some(pool) = pools.iter().find(|pool| pool.kind == kind && pool.any_wet()) else {
            continue;
        };
        let mut pool = pool.clone();
        pool.reseat_range();
        flags |= bit;
        data.extend_from_slice(pool.bytes());
    }
    chunk.head_mut().set_flags(flags);
    match chunk.region_mut(Region::Liquid) {
        // **The size field is left as the file wrote it** — see
        // [`crate::adt::SubChunk::set_keeping_size`], and the module comment.
        Some(sub) => sub.set_keeping_size(data),
        None => {
            let mut sub = super::SubChunk::new(data);
            sub.declared = 0;
            chunk.regions[Region::Liquid as usize] = Some(sub);
        }
    }
}
