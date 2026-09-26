//! The hole mask: the sixteen bits that decide where the ground is not drawn.
//!
//! ## It is one field and the smallest edit in this crate
//!
//! `MCNK`'s header carries a `u16` at `0x3c` — a 4x4 grid over the chunk's 8x8
//! cells, each bit covering a 2x2 block. Setting one takes four cells out of the
//! ground and nothing else in the file changes: no region moves, no offset is
//! recomputed, no list is renumbered. [`crate::adt::header::McnkHeaderMut`]
//! already wrote it and nothing asked.
//!
//! ## …and it is the one edit that cannot reach the screen live
//!
//! Every other tool in this crate has a live path, because what it changes is
//! something the thing on screen *holds*: a height is a vertex in a buffer, a
//! blend weight is a texel in an atlas. A hole is neither. The mask decides how
//! many vertices a chunk contributes at all — `chunk_mesh_vertices` emits five
//! per cell that is *not* a hole — so cutting one changes the length of every
//! draw group after it in the tile's mesh and the index ranges of all of them.
//!
//! There is nothing to patch. The tile is read again, which since
//! `tools::terrain::swap` costs a task on the compute pool and nothing on
//! screen. See [`crate::ops::Edit::remeshes`], which is the fork that says so.
//!
//! ## Which bit is which is `vale_assets`' statement and not this one
//!
//! `vale_assets::world::adt::hole_bit` converts a world position to a bit and
//! `hole_square` gives the square back. Working it out here as well would put a
//! hole a quadrant away from the pointer on half the chunks, on a file that
//! still parses — the failure nothing reports.

use super::{AdtFile, MapChunk};

/// One chunk's mask, or `None` for a chunk index a tile does not have.
pub fn holes(tile: &AdtFile, chunk: usize) -> Option<u16> {
    tile.chunk(chunk).map(|chunk| chunk.head().holes())
}

/// …and the same, written.
pub fn set_holes(chunk: &mut MapChunk, mask: u16) {
    chunk.head_mut().set_holes(mask);
}

/// The mask a chunk would have after cutting or patching the bit under a world
/// position, or `None` when the position is not over that chunk.
///
/// `cut` is which way round: `true` takes the ground out, `false` puts it back.
/// A no-op — patching where there is no hole — answers the mask unchanged, which
/// is what lets the caller record nothing rather than an empty change.
pub fn with_bit_at(tile: &AdtFile, chunk: usize, x: f32, y: f32, cut: bool) -> Option<u16> {
    let head = tile.chunk(chunk)?.head();
    let bit = vale_assets::world::adt::hole_bit(head.position(), x, y)?;
    let mask = head.holes();
    Some(match cut {
        true => mask | (1 << bit),
        false => mask & !(1 << bit),
    })
}

/// How many of a tile's 4,096 hole bits are set.
///
/// For a panel that wants to say what a map already has rather than only what
/// this session did to it.
pub fn cut_in(tile: &AdtFile) -> u32 {
    tile.chunks
        .iter()
        .map(|chunk| chunk.head().holes().count_ones())
        .sum()
}
