//! `MCLQ`: the water, ocean, lava and slime standing on a map chunk.
//!
//! ## The `MCNK` header declares what `MCLQ` holds
//!
//! `MCLQ` is the one region of an `MCNK` that does not describe itself. It
//! carries one 804-byte block per liquid, back to back, and does not record how
//! many blocks there are or which liquid each one is. The `MCNK` header's flag
//! bits record both, in bit order: river, ocean, magma, slime. A writer must
//! keep the flags and the blocks in step, and a reader that uses either one
//! alone reads the region wrongly.
//!
//! The region's own size field is zero on every one of the 1,280 chunks of the
//! five real tiles, wet or dry, so the header's `sizeLiquid` is the only correct
//! statement of the region's extent. [`crate::adt::SubChunk::set_keeping_size`]
//! writes a block back and leaves that size field as the file had it, rather
//! than writing a number the game never wrote.
//!
//! ## Layout of one block
//!
//! ```text
//! 0     f32 min, f32 max          the surface's own range
//! 8     81 x 8 bytes              a 9x9 grid of vertices
//! 656   64 bytes                  an 8x8 grid of tile flags
//! 720   u32 nFlowvs, 2 x 40       flow vectors, present whatever nFlowvs is
//! 804   …the next block
//! ```
//!
//! A vertex is a union whose meaning depends on the liquid type. Only bytes 0
//! and 4..8 are understood here. Bytes 4..8 are an `f32` height for all four
//! liquids, and the height is an absolute world z, not an offset from the
//! chunk's. Byte 0 is a *depth* for water and ocean: the opacity's parameter, 0
//! at the shore and 255 in the deep. For magma and slime byte 0 is the low half
//! of a texture coordinate; that is `vale_assets`' measurement, not an
//! assumption. Bytes 1..4 are not understood.
//!
//! A vertex with no surface over it holds `f32::MAX` (`0x7F7FFFFF`). The
//! shipped blocks carry it wherever there is no water, and the block's own
//! `min`/`max` cover every vertex that does not hold `f32::MAX`. That rule is
//! neither "all vertices" (the range would then be `f32::MAX`) nor "the corners
//! of the wet cells" (a real tile has real heights at vertices no wet cell
//! touches). It was measured on `Azeroth_34_51` chunk 14, where a round trip
//! that computed the range either of the other two ways disagreed with the
//! file.
//!
//! A tile flag's low nibble is 15 for a dry cell and the liquid's type for a
//! wet one ([`cell_type`]: water 4, ocean 1, magma 6). Dry cells give a pool a
//! shape other than the whole chunk, and they are why a chunk can declare a
//! liquid and still be mostly dry. The 1.12.1 client draws nothing on a cell of
//! type 0. The high nibble holds [`FISHABLE`] and [`FATIGUE`].
//!
//! ## Why a `Pool` keeps its block as bytes
//!
//! [`Pool`] carries the block exactly as the file wrote it and changes only the
//! fields an edit names. An earlier version decoded the block into heights,
//! depths and flags and re-encoded from those, and the round trip failed on the
//! first real tile: the three bytes after the depth and the flow tail hold data
//! this crate does not understand, and a re-encode from the understood fields
//! drops that data. `crate::adt` keeps a whole tile as bytes for the same
//! reason.
//!
//! ## Grid order
//!
//! Rows run along decreasing world x and columns along decreasing world y from
//! the chunk's origin, as in every other per-cell field.
//! `vale_assets::world::adt::cell_at` is the single statement of that order,
//! and this module calls it rather than repeating the arithmetic. The vertex
//! grid is 9x9 over the same cells, so cell `(row, col)`'s four corners are
//! vertices `(row, col)`, `(row, col+1)`, `(row+1, col)` and `(row+1, col+1)`.
//!
//! `vale_assets`' parser re-orders both grids into `WmoLiquid`'s own frame,
//! because the renderer draws from that frame. This crate works in the file's
//! order, which is the order it writes.
//!
//! ## Why an edit is the whole region
//!
//! Giving a dry chunk water adds 804 bytes in the middle of a two-megabyte file
//! and sets a header flag; taking it away removes them and clears the flag. The
//! smallest unit that inverts correctly is the chunk's whole liquid region
//! together with the flags that declare it. `alpha::Paint` uses a whole region
//! as its unit for the same reason.

use super::{AdtFile, MapChunk, Region};
use vale_assets::world::adt::{mcnk_flags, INNER_SIDE};
use vale_assets::world::wmo::Liquid;

/// Vertices on a side of the 9x9 grid.
pub const SIDE: usize = INNER_SIDE + 1;
/// Vertices in the 9x9 grid.
const VERTS: usize = SIDE * SIDE;
/// Cells in the 8x8 grid.
pub(crate) const CELLS: usize = INNER_SIDE * INNER_SIDE;

/// Bytes per vertex — see the module comment on what they mean.
const VERTEX: usize = 8;
/// Where the tile flags start within a block.
const TILES_AT: usize = 8 + VERTS * VERTEX;
/// Where the flow tail starts within a block.
const FLOW_AT: usize = TILES_AT + CELLS;
/// One whole block.
pub(crate) const BLOCK: usize = FLOW_AT + 4 + 2 * 40;

/// The low nibble of a tile flag that means no liquid on the cell.
pub(crate) const DRY: u8 = 0x0F;

/// The two bits of the high nibble. The format does not name them; their
/// meaning comes from a census of shipped tiles: `vale water <Map> <x> <y>`
/// prints the count of each value over a tile's wet cells.
///
/// `0x80` is the one bit vmangos' map extractor reads: `flags & (1 << 7)`
/// marks the cell as deep water, where a swimmer takes fatigue. In the shipped
/// tiles it is set on the open sea and nowhere else: 14,492 of 15,511 cells on
/// `Azeroth_30_39`, none of Menethil's 9,008, none of Elwynn's lake.
///
/// `0x40` is set on nearly every sea and lake cell (all 9,008 at Menethil, all
/// 328 off Westfall, 1,337 of 2,636 on Elwynn's lake tile) and clear on the
/// rest of that lake tile's cells and on 147 of 158 river cells at Kalimdor
/// 40,23. Nothing in vmangos reads it, and nothing in this project did. The
/// name, fishable, is the one the format's public reading gives it, and the
/// distribution is consistent with that and with no other reading measured.
pub const FISHABLE: u8 = 0x40;
pub const FATIGUE: u8 = 0x80;

/// The low nibble a wet cell of each kind carries: the cell's liquid type.
///
/// Measured over the wet cells of eight shipped tiles, where every wet cell of
/// a block carries its kind's value: water 4 (Azeroth 32 49, 37 49 and 31 57,
/// Kalimdor 32 45, 36 32 and 40 23), ocean 1 (Azeroth 28 48, 30 39 and
/// 31 57) and magma 6 (Azeroth 33 44). No shipped tile has slime; 3 is the
/// value the format's public reading gives it. The 1.12.1 client draws
/// nothing on a cell whose type is 0.
pub fn cell_type(kind: Liquid) -> u8 {
    match kind {
        Liquid::Water => 4,
        Liquid::Ocean => 1,
        Liquid::Magma => 6,
        Liquid::Slime => 3,
    }
}

/// The height a vertex with no surface over it carries: `f32::MAX`,
/// `0x7F7FFFFF`. See the module comment, and [`Pool::reseat_range`], which
/// leaves such vertices out of the range.
pub(crate) const NO_SURFACE: f32 = f32::MAX;

/// The four liquids and the header bit that declares each, in the order the
/// blocks appear.
///
/// The order is the flags' bit order; `MCLQ` itself does not record it. It
/// decides which block is which on a chunk that carries both a river and the
/// ocean it flows into.
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
    /// A pool of this kind with every cell dry.
    ///
    /// It starts dry because the tool that makes one then wets the cells it
    /// wants. A pool that started wet would fill the whole chunk with water,
    /// which no edit asked for.
    ///
    /// Every vertex is [`NO_SURFACE`], the game's own marker for a vertex with
    /// nothing over it, and the flow tail is zeros, which is a still pool. The
    /// level only seeds the range; [`Pool::set_vertex`] writes the real heights
    /// where the brush lands.
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

    /// The pool with its vertices and its cells moved about within the chunk:
    /// the record now at `(row, col)` of a grid `n` on a side is the one that
    /// was at `source(n, row, col)`. Called with 9 for the vertices and 8 for
    /// the tile flags. A quarter turn and a mirror are each one such map.
    ///
    /// Each vertex's eight bytes move whole, so the bytes this crate does not
    /// understand go with their vertex. The height range is the same set of
    /// heights and is left as it was. The flow vectors after the flags are
    /// not turned; they are zero for a pool that stands still.
    pub(crate) fn rearranged(&self, source: &dyn Fn(usize, usize, usize) -> (usize, usize)) -> Pool {
        let mut bytes = self.bytes.clone();
        for row in 0..SIDE {
            for col in 0..SIDE {
                let (from_row, from_col) = source(SIDE, row, col);
                let to = 8 + (row * SIDE + col) * VERTEX;
                let from = 8 + (from_row * SIDE + from_col) * VERTEX;
                bytes[to..to + VERTEX].copy_from_slice(&self.bytes[from..from + VERTEX]);
            }
        }
        let cells = SIDE - 1;
        for row in 0..cells {
            for col in 0..cells {
                let (from_row, from_col) = source(cells, row, col);
                bytes[TILES_AT + row * cells + col] = self.bytes[TILES_AT + from_row * cells + from_col];
            }
        }
        Pool {
            kind: self.kind,
            bytes,
        }
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

    /// The depth byte at one vertex, which only water and ocean have. See the
    /// module comment.
    pub fn depth(&self, row: usize, col: usize) -> u8 {
        self.bytes[8 + (row * SIDE + col) * VERTEX]
    }

    /// Whether cell `(row, col)` has liquid on it.
    pub fn is_wet(&self, row: usize, col: usize) -> bool {
        self.bytes
            .get(TILES_AT + row * INNER_SIDE + col)
            .is_some_and(|flag| flag & 0x0F != DRY)
    }

    /// Set whether cell `(row, col)` has liquid on it. Wetting writes the
    /// pool's [`cell_type`] into the low nibble and keeps the high nibble,
    /// which is the cell's own (see [`Self::cell_flags`]). Drying writes the
    /// whole byte back to [`DRY`], because a dry cell is neither fishable nor
    /// deep and a shipped dry cell reads [`DRY`].
    pub(crate) fn set_wet(&mut self, row: usize, col: usize, wet: bool) {
        let typed = cell_type(self.kind);
        let Some(flag) = self.bytes.get_mut(TILES_AT + row * INNER_SIDE + col) else {
            return;
        };
        *flag = match wet {
            true => (*flag & 0xF0) | typed,
            false => DRY,
        };
    }

    /// Whether cell `(row, col)` is wet and carries this pool's
    /// [`cell_type`].
    pub fn is_typed(&self, row: usize, col: usize) -> bool {
        self.bytes
            .get(TILES_AT + row * INNER_SIDE + col)
            .is_some_and(|flag| flag & 0x0F == cell_type(self.kind))
    }

    /// Give every wet cell whose type is 0 this pool's [`cell_type`], keeping
    /// its high nibble. Returns how many cells changed.
    ///
    /// Type 0 is the value `vale_edit` wrote on every cell it wetted before
    /// [`Self::set_wet`] wrote the type, and the 1.12.1 client draws nothing
    /// on such a cell. No shipped tile carries it, so a shipped tile is left
    /// unchanged. A cell with any other type is left as it is.
    pub(crate) fn repair_untyped(&mut self) -> usize {
        let typed = cell_type(self.kind);
        let mut changed = 0;
        for flag in &mut self.bytes[TILES_AT..TILES_AT + CELLS] {
            if *flag & 0x0F == 0 {
                *flag = (*flag & 0xF0) | typed;
                changed += 1;
            }
        }
        changed
    }

    /// The cell's high nibble: [`FISHABLE`] and [`FATIGUE`], and two bits no
    /// shipped tile sets.
    pub fn cell_flags(&self, row: usize, col: usize) -> u8 {
        self.bytes
            .get(TILES_AT + row * INNER_SIDE + col)
            .map_or(0, |flag| flag & 0xF0)
    }

    /// Set the cell's high nibble, keeping the low nibble. Only the high four
    /// bits of `flags` are taken.
    pub(crate) fn set_cell_flags(&mut self, row: usize, col: usize, flags: u8) {
        let Some(flag) = self.bytes.get_mut(TILES_AT + row * INNER_SIDE + col) else {
            return;
        };
        *flag = (*flag & 0x0F) | (flags & 0xF0);
    }

    /// Whether any cell is wet. A pool with no wet cell draws nothing, and
    /// [`set_pools`] treats it as removed.
    pub(crate) fn any_wet(&self) -> bool {
        (0..INNER_SIDE).any(|row| (0..INNER_SIDE).any(|col| self.is_wet(row, col)))
    }

    /// Set the surface height and depth at the four corners of one cell.
    ///
    /// The four corners rather than one, because a cell's surface is the quad
    /// its corners make: writing one vertex tilts the quads of the three
    /// neighbours that share it.
    #[cfg(test)]
    pub(crate) fn fill_cell(&mut self, row: usize, col: usize, height: f32, depth: u8) {
        for (dr, dc) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let at = 8 + ((row + dr) * SIDE + col + dc) * VERTEX;
            if at + VERTEX > TILES_AT {
                continue;
            }
            self.bytes[at] = depth;
            self.bytes[at + 4..at + 8].copy_from_slice(&height.to_le_bytes());
        }
    }

    /// Set the surface height at one vertex of the 9x9 grid, and its depth
    /// byte when the kind has one. For magma and slime that byte is part of a
    /// texture coordinate and is left alone. See the module comment.
    pub(crate) fn set_vertex(&mut self, row: usize, col: usize, height: f32, depth: u8) {
        let at = 8 + (row * SIDE + col) * VERTEX;
        if row >= SIDE || col >= SIDE {
            return;
        }
        if self.has_depth() {
            self.bytes[at] = depth;
        }
        self.bytes[at + 4..at + 8].copy_from_slice(&height.to_le_bytes());
    }

    /// Set the depth byte alone at one vertex, for a kind that has one.
    pub fn set_depth(&mut self, row: usize, col: usize, depth: u8) {
        if row < SIDE && col < SIDE && self.has_depth() {
            self.bytes[8 + (row * SIDE + col) * VERTEX] = depth;
        }
    }

    /// Whether byte 0 of a vertex is a depth: water and ocean.
    pub(crate) fn has_depth(&self) -> bool {
        matches!(self.kind, Liquid::Water | Liquid::Ocean)
    }

    /// The depth byte for a surface standing `above` yards over the ground, on
    /// the curve measured on the shipped tiles.
    ///
    /// `vale water` reports the relation over 3,522 wet vertices of
    /// `Azeroth_32_49`: bytes 0..31 stand +0.6 yards above the ground, 32..95
    /// stand +6.2, and 96 and over stand +12.6. That is roughly eight per yard
    /// over the first ten yards, which is the slope used here. The byte is the
    /// opacity's parameter, so a pool written with a flat 255 has no shallows
    /// and its shore does not fade out.
    pub(crate) fn depth_for(above: f32) -> u8 {
        (above.max(0.0) * 8.0).round().clamp(0.0, 255.0) as u8
    }

    /// Set the block's own `min`/`max` from its heights.
    ///
    /// The range covers every vertex that is not [`NO_SURFACE`], which is the
    /// rule the shipped blocks follow. Two other rules were tested against
    /// `Azeroth_34_51` chunk 14 and both disagree with the file: all vertices
    /// gives `f32::MAX`, and the corners of the wet cells gives too narrow a
    /// range, because a real tile carries heights at vertices no wet cell
    /// touches.
    ///
    /// [`set_pools`] calls it, so a caller that changed a height does not have
    /// to. On a pool nothing changed it reproduces the file exactly, which the
    /// round-trip check tests.
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
            // The region is shorter than the flags claim. The blocks already
            // read are kept, and the missing ones are not made up.
            break;
        };
        out.push(Pool {
            kind,
            bytes: block.to_vec(),
        });
    }
    out
}

/// Write a chunk's liquids back, with the header flags that declare them.
///
/// A pool with no wet cell is dropped rather than written: its flag would
/// declare a liquid that draws nothing, which the file can hold but nothing
/// shows. A tool removes water this way: it dries the cells and this drops the
/// pool.
pub(crate) fn set_pools(chunk: &mut MapChunk, pools: &[Pool]) {
    let mut data = Vec::with_capacity(pools.len() * BLOCK);
    let mut flags = chunk.head().flags() & !mcnk_flags::LQ_ANY;
    // In `KINDS` order, not the caller's, because that is the order the bits
    // are read back in. In the caller's order, a river's block could be written
    // where the ocean's flag says the ocean's block is.
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
        // The size field is left as the file wrote it. See
        // [`crate::adt::SubChunk::set_keeping_size`], and the module comment.
        Some(sub) => sub.set_keeping_size(data),
        None => {
            let mut sub = super::SubChunk::new(data);
            sub.declared = 0;
            chunk.regions[Region::Liquid as usize] = Some(sub);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wetted cell carries its kind's type in the low nibble and keeps its
    /// high nibble; a dried one is the whole byte [`DRY`].
    #[test]
    fn a_wet_cell_carries_its_liquids_type() {
        for (kind, typed) in [(Liquid::Water, 4), (Liquid::Ocean, 1), (Liquid::Magma, 6), (Liquid::Slime, 3)] {
            let mut pool = Pool::flat(kind, 0.0);
            pool.set_cell_flags(2, 3, FISHABLE);
            pool.set_wet(2, 3, true);
            assert_eq!(pool.bytes[TILES_AT + 2 * INNER_SIDE + 3], FISHABLE | typed, "{kind:?}");
            assert!(pool.is_wet(2, 3) && pool.is_typed(2, 3), "{kind:?}");
            pool.set_wet(2, 3, false);
            assert_eq!(pool.bytes[TILES_AT + 2 * INNER_SIDE + 3], DRY, "{kind:?}");
        }
    }

    /// A tile whose wet cells carry type 0 is repaired to its pools' types,
    /// one edit per chunk, and a second repair finds nothing.
    #[test]
    fn a_wet_cell_with_no_type_is_given_its_pools_type() {
        let mut tile = crate::adt::blank_tile(32, 48, r"Tileset\Generic\Black.blp", 0.0, 0);
        let mut pool = Pool::flat(Liquid::Ocean, 1.0);
        pool.set_wet(0, 0, true);
        pool.set_wet(0, 1, true);
        pool.bytes[TILES_AT] = FISHABLE;
        pool.bytes[TILES_AT + 1] = FATIGUE;
        set_pools(tile.chunk_mut(7).expect("a blank tile has every chunk"), &[pool]);

        let done = crate::ops::water::repair_liquid_types(&mut tile);
        assert_eq!((done.cells, done.edits.len()), (2, 1));
        let pools = pools(&tile, 7);
        assert!(pools[0].is_typed(0, 0) && pools[0].is_typed(0, 1));
        assert_eq!((pools[0].cell_flags(0, 0), pools[0].cell_flags(0, 1)), (FISHABLE, FATIGUE));

        let again = crate::ops::water::repair_liquid_types(&mut tile);
        assert_eq!((again.cells, again.edits.len()), (0, 0));
    }
}
