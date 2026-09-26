//! `MCCV`: the shading painted onto a chunk's own vertices.
//!
//! ## What the region is
//!
//! 145 four-byte entries in the same interleaved order `MCVT` and `MCNR` use, so
//! vertex *n* of the heights is vertex *n* of the colours and nothing here has to
//! restate the 145-vertex layout. Each entry is a `CImVector` — **blue, green,
//! red, alpha**, in that byte order, which is the one thing about this chunk that
//! is easy to get backwards and is invisible on grey.
//!
//! The rule for what the bytes mean is `vale_assets`', not this crate's:
//! [`vale_assets::world::adt::decode_colours`] is the one reading of it, and
//! the only number repeated here is [`NEUTRAL`], because a *writer* needs the
//! value that means "leave this alone" and a reader does not.
//!
//! ## No 1.12 tile has one, and that decides how this is written
//!
//! Measured over `Azeroth_32_48`: zero `MCCV` sub-chunks in 256 map chunks, and
//! the header dword its offset lives in is zero on every one of them. So every
//! chunk this crate paints is a chunk the region is **added** to, and two
//! properties follow that are worth having in mind before touching anything
//! here:
//!
//! * [`ensure`] creates it filled with [`NEUTRAL`], which draws identically to
//!   having no region at all. A brush can therefore create-then-paint without a
//!   visible step between the two.
//! * [`clear`] takes it away again rather than filling it with neutral bytes. A
//!   chunk that has been painted and then unpainted should write back as the
//!   chunk it was, and 580 bytes of 0x7F is not that.
//!
//! ## The length of the region never changes
//!
//! It is [`BYTES`] or it is absent. That is what makes a colour stroke a
//! *vertex* edit like a height stroke rather than a re-mesh like a hole or a
//! texture: the mesh keeps its shape, its indices and its length, and only an
//! attribute moves. See `Edit::Colours`.

use super::{AdtFile, MapChunk, Region, SubChunk};
use vale_assets::world::adt::{COLOUR_NEUTRAL, HEIGHTS_PER_CHUNK};

/// The byte that means "do not shade this vertex" — `vale_assets`' own, so
/// the writer and the reader cannot drift.
pub const NEUTRAL: u8 = COLOUR_NEUTRAL;

/// Bytes per vertex: blue, green, red, alpha.
pub const STRIDE: usize = 4;

/// …and bytes in the whole region.
pub const BYTES: usize = HEIGHTS_PER_CHUNK * STRIDE;

/// One vertex's entry, in the order a person thinks in rather than the order the
/// file stores.
///
/// **Red, green, blue, alpha** — the swap into the file's `CImVector` order
/// happens in [`set`] and [`get`] and nowhere else.
pub type Colour = [u8; 4];

/// The entry every vertex starts at.
pub const NEUTRAL_COLOUR: Colour = [NEUTRAL; 4];

/// A chunk's 145 entries, or an empty vector when it has no `MCCV`.
///
/// Empty and not 145 neutrals, because the two are different states of the file
/// and only one of them costs bytes. A caller that wants to *read* a vertex
/// without caring should use [`at`].
pub fn colours(chunk: &MapChunk) -> Vec<Colour> {
    let Some(mccv) = chunk.region(Region::Colours) else {
        return Vec::new();
    };
    (0..HEIGHTS_PER_CHUNK)
        .map(|i| at_in(&mccv.data, i))
        .collect()
}

/// One vertex's entry, neutral where the chunk has no region or the region is
/// short.
pub fn at(chunk: &MapChunk, vertex: usize) -> Colour {
    match chunk.region(Region::Colours) {
        Some(mccv) => at_in(&mccv.data, vertex),
        None => NEUTRAL_COLOUR,
    }
}

fn at_in(data: &[u8], vertex: usize) -> Colour {
    let byte = |k: usize| data.get(vertex * STRIDE + k).copied().unwrap_or(NEUTRAL);
    // Blue, green, red, alpha in the file.
    [byte(2), byte(1), byte(0), byte(3)]
}

/// **Give a chunk an `MCCV` if it has none**, filled with [`NEUTRAL`].
///
/// Answers whether it had to make one, which is what a caller recording an
/// [`crate::ops::Edit`] wants to know: the region appearing is part of the
/// change and has to come back out on an undo.
pub fn ensure(chunk: &mut MapChunk) -> bool {
    if chunk.region(Region::Colours).is_some() {
        return false;
    }
    chunk.regions[Region::Colours as usize] = Some(SubChunk::new(vec![NEUTRAL; BYTES]));
    true
}

/// …and take it away again.
///
/// The counterpart of [`ensure`], and the reason unpainting is not "paint
/// neutral": a chunk with no region is the chunk the file shipped.
pub fn clear(chunk: &mut MapChunk) {
    chunk.regions[Region::Colours as usize] = None;
}

/// Write one vertex's entry, creating the region if the chunk has none.
pub fn set(chunk: &mut MapChunk, vertex: usize, colour: Colour) {
    if vertex >= HEIGHTS_PER_CHUNK {
        return;
    }
    ensure(chunk);
    let Some(mccv) = chunk.region_mut(Region::Colours) else {
        return;
    };
    if mccv.data.len() < BYTES {
        mccv.data.resize(BYTES, NEUTRAL);
    }
    // Red, green, blue, alpha in; blue, green, red, alpha out.
    let at = vertex * STRIDE;
    mccv.data[at] = colour[2];
    mccv.data[at + 1] = colour[1];
    mccv.data[at + 2] = colour[0];
    mccv.data[at + 3] = colour[3];
    // `declared` has to move with the payload or the writer states a length the
    // region contradicts — `SubChunk::set` is the usual door for that and this
    // writes in place, so it says so itself.
    mccv.declared = mccv.data.len() as u32;
}

/// …and all 145 at once, which is what a stroke writes.
pub fn set_all(chunk: &mut MapChunk, colours: &[Colour]) {
    ensure(chunk);
    let Some(mccv) = chunk.region_mut(Region::Colours) else {
        return;
    };
    let mut data = vec![NEUTRAL; BYTES];
    for (i, colour) in colours.iter().take(HEIGHTS_PER_CHUNK).enumerate() {
        data[i * STRIDE] = colour[2];
        data[i * STRIDE + 1] = colour[1];
        data[i * STRIDE + 2] = colour[0];
        data[i * STRIDE + 3] = colour[3];
    }
    mccv.set(data);
}

/// **Is every vertex of this chunk still neutral?**
///
/// What [`crate::ops::Shading`] asks before it leaves a region behind: a stroke
/// that has been undone back to nothing should take the chunk back to having no
/// `MCCV`, and the only way to know is to look.
pub fn is_neutral(chunk: &MapChunk) -> bool {
    match chunk.region(Region::Colours) {
        None => true,
        Some(mccv) => mccv.data.iter().all(|&b| b == NEUTRAL),
    }
}

/// The whole region's bytes, for an [`crate::ops::Edit`] to carry — `None` for a
/// chunk that has none, which is the state an undo has to be able to get back
/// to.
pub fn capture(tile: &AdtFile, index: usize) -> Option<Vec<u8>> {
    tile.chunk(index)
        .and_then(|chunk| chunk.region(Region::Colours))
        .map(|mccv| mccv.data.clone())
}

/// …and putting one back, including putting *nothing* back.
pub fn restore(tile: &mut AdtFile, index: usize, data: Option<&Vec<u8>>) {
    let Some(chunk) = tile.chunk_mut(index) else {
        return;
    };
    match data {
        Some(bytes) => match chunk.region_mut(Region::Colours) {
            Some(mccv) => mccv.set(bytes.clone()),
            None => {
                chunk.regions[Region::Colours as usize] = Some(SubChunk::new(bytes.clone()))
            }
        },
        None => clear(chunk),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adt::file::FLAG_HAS_COLOURS;

    fn chunk() -> MapChunk {
        MapChunk {
            header: [0u8; crate::adt::file::MCNK_HEADER],
            regions: Default::default(),
            index_extra: [0, 0],
        }
    }

    /// **A chunk starts with no region and neutral answers**, which is the state
    /// every shipped 1.12 chunk is in.
    #[test]
    fn a_chunk_with_no_region_reads_neutral() {
        let chunk = chunk();
        assert!(colours(&chunk).is_empty());
        assert_eq!(at(&chunk, 0), NEUTRAL_COLOUR);
        assert_eq!(at(&chunk, HEIGHTS_PER_CHUNK - 1), NEUTRAL_COLOUR);
        assert!(is_neutral(&chunk));
    }

    /// **The file's order is blue, green, red, alpha and this crate's is not.**
    /// The swap is invisible on grey, which is exactly the value a first draft
    /// tests with, so it is asserted on a colour with three different channels.
    #[test]
    fn the_file_holds_it_the_other_way_round() {
        let mut chunk = chunk();
        set(&mut chunk, 3, [0x10, 0x20, 0x30, 0x40]);
        let raw = &chunk.region(Region::Colours).expect("made").data;
        assert_eq!(
            &raw[3 * STRIDE..3 * STRIDE + 4],
            &[0x30, 0x20, 0x10, 0x40],
            "the region should hold b, g, r, a"
        );
        assert_eq!(at(&chunk, 3), [0x10, 0x20, 0x30, 0x40], "and read back as r, g, b, a");
    }

    /// Writing one vertex leaves the other 144 neutral, which is what makes a
    /// brush a brush rather than a fill.
    #[test]
    fn writing_one_vertex_leaves_the_rest_alone() {
        let mut chunk = chunk();
        set(&mut chunk, 7, [0, 0, 0, 0]);
        let all = colours(&chunk);
        assert_eq!(all.len(), HEIGHTS_PER_CHUNK);
        for (i, colour) in all.iter().enumerate() {
            match i {
                7 => assert_eq!(*colour, [0, 0, 0, 0]),
                _ => assert_eq!(*colour, NEUTRAL_COLOUR, "vertex {i}"),
            }
        }
        assert!(!is_neutral(&chunk));
    }

    /// **The region's length never changes**, which is what makes a colour
    /// stroke a vertex edit rather than a re-mesh.
    #[test]
    fn the_region_is_one_length_or_absent() {
        let mut chunk = chunk();
        assert!(ensure(&mut chunk));
        assert!(!ensure(&mut chunk), "the second call makes nothing");
        assert_eq!(chunk.region(Region::Colours).expect("made").data.len(), BYTES);
        set(&mut chunk, 100, [1, 2, 3, 4]);
        assert_eq!(chunk.region(Region::Colours).expect("kept").data.len(), BYTES);
        set_all(&mut chunk, &[[9, 9, 9, 9]; HEIGHTS_PER_CHUNK]);
        assert_eq!(chunk.region(Region::Colours).expect("kept").data.len(), BYTES);
    }

    /// **Unpainting takes the region away rather than filling it with neutral
    /// bytes.** A chunk painted and then undone should write back as the chunk
    /// the file shipped, and 580 bytes of 0x7F is not that.
    #[test]
    fn clearing_removes_the_region_rather_than_neutralising_it() {
        let mut chunk = chunk();
        set(&mut chunk, 0, [0, 0, 0, 255]);
        assert!(chunk.region(Region::Colours).is_some());
        clear(&mut chunk);
        assert!(chunk.region(Region::Colours).is_none());
        assert!(is_neutral(&chunk));
    }

    /// The flag the writer keeps in step with the region — asserted here rather
    /// than only in the writer, because the constant is what a later client
    /// reads and the value is not this project's to choose.
    #[test]
    fn the_flag_is_the_documented_bit() {
        assert_eq!(FLAG_HAS_COLOURS, 0x40);
    }
}
