//! The flag that says a chunk cannot be walked on.
//!
//! ## It is the server's flag, not the client's
//!
//! `MCNK`'s flags word has a bit at `0x02` that wowdev.wiki names `impass`. It
//! marks ground that players may not climb. What reads it is **vmangos**, when
//! it builds its movement maps: `mmaps` are generated from these same ADTs, and
//! a chunk marked impassable is one the navigation mesh leaves out.
//!
//! So this is the first edit in this crate whose effect is not on screen at all.
//! Raising ground changes what the viewport draws; marking it impassable
//! changes where a server will let a character go, once its `mmaps` are rebuilt
//! from the edited files. `crate::project`'s own note about publishing not
//! reaching the server applies here more sharply than anywhere else: **nothing
//! at all happens until the server's maps are rebuilt.**
//!
//! ## No shipped 1.12 tile sets it
//!
//! Measured over four tiles from two maps — `Azeroth_32_48`, `Azeroth_31_31`,
//! `Azeroth_32_49` and `Kalimdor_43_30`, 1,024 chunks:
//!
//! ```text
//! 0x01  has MCSH      1024 chunks
//! 0x04  river           91
//! 0x08  ocean          256
//! 0x02  impassable       0     <- never, on any of them
//! ```
//!
//! So writing it is adding something 1.12's own data does not use. That is not a
//! reason not to — the bit is documented, the server reads it, and a map built
//! for this project rather than for 1.12 may well want it — but it is a reason
//! to say so, which is what this comment is for. It is *not* a format deviation
//! on `MCCV`'s scale: the bit is in the shipped header, in a word the client
//! already reads for other reasons, and setting it adds no bytes.

use super::file::AdtFile;
use super::header::at;

/// `MCNK`'s *impassable* bit.
///
/// Named here rather than in `vale_assets::world::adt::mcnk_flags` because
/// nothing in the client reads it: that module is the flags the *renderer*
/// keys on, and adding one it ignores would invite the next reader to look for
/// where it is used.
pub const IMPASSABLE: u32 = 0x02;

/// Is this chunk marked impassable?
pub fn impassable(tile: &AdtFile, chunk_index: usize) -> bool {
    tile.chunks
        .get(chunk_index)
        .is_some_and(|chunk| u32_at(&chunk.header, at::FLAGS) & IMPASSABLE != 0)
}

/// Mark it, or unmark it. Returns whether the bit moved.
pub fn set_impassable(tile: &mut AdtFile, chunk_index: usize, impassable: bool) -> bool {
    let Some(chunk) = tile.chunks.get_mut(chunk_index) else {
        return false;
    };
    let was = u32_at(&chunk.header, at::FLAGS);
    let now = match impassable {
        true => was | IMPASSABLE,
        false => was & !IMPASSABLE,
    };
    if now == was {
        return false;
    }
    put_u32(&mut chunk.header, at::FLAGS, now);
    true
}

/// How many of a tile's chunks are marked, for a panel to report.
pub fn count(tile: &AdtFile) -> usize {
    (0..tile.chunks.len())
        .filter(|&index| impassable(tile, index))
        .count()
}

fn u32_at(buf: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&buf[at..at + 4]);
    u32::from_le_bytes(word)
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adt::blank_tile;
    use vale_assets::world::adt::mcnk_flags;

    const GRASS: &str = r"Tileset\Elwynn\ElwynnGrassBase.blp";

    #[test]
    fn the_bit_goes_on_and_off() {
        let mut tile = blank_tile(32, 48, GRASS, 0.0, 0);
        assert!(!impassable(&tile, 0));
        assert_eq!(count(&tile), 0);

        assert!(set_impassable(&mut tile, 0, true));
        assert!(impassable(&tile, 0));
        assert_eq!(count(&tile), 1);

        // Setting what is already set says nothing moved, so a caller can skip
        // marking the tile unsaved.
        assert!(!set_impassable(&mut tile, 0, true));

        assert!(set_impassable(&mut tile, 0, false));
        assert!(!impassable(&tile, 0));
        assert_eq!(count(&tile), 0);
    }

    /// **It leaves every other flag alone**, which matters because the flags
    /// word carries the shadow bit and the four liquid bits — and a chunk that
    /// lost its liquid declaration would keep its `MCLQ` and stop drawing it.
    #[test]
    fn the_other_flags_are_untouched() {
        let mut tile = blank_tile(32, 48, GRASS, 0.0, 0);
        let mixed = mcnk_flags::HAS_MCSH | mcnk_flags::LQ_RIVER | mcnk_flags::LQ_OCEAN;
        tile.chunks[0].head_mut().set_flags(mixed);

        assert!(set_impassable(&mut tile, 0, true));
        let now = tile.chunks[0].head().flags();
        assert_eq!(now & mixed, mixed, "another flag was lost");
        assert_ne!(now & IMPASSABLE, 0);

        assert!(set_impassable(&mut tile, 0, false));
        assert_eq!(tile.chunks[0].head().flags(), mixed, "the word did not come back");
    }

    /// A chunk that does not exist is refused rather than panicking, which is
    /// the same tolerance every other edit here has.
    #[test]
    fn a_chunk_that_is_not_there_is_refused() {
        let mut tile = blank_tile(32, 48, GRASS, 0.0, 0);
        assert!(!set_impassable(&mut tile, 9999, true));
        assert!(!impassable(&tile, 9999));
    }

    /// A marked tile is still a tile both readers read — the bit lives in a
    /// word that is already there, so nothing about the file's shape changes.
    #[test]
    fn a_marked_tile_still_parses() {
        let mut tile = blank_tile(32, 48, GRASS, 0.0, 0);
        for index in (0..256).step_by(3) {
            set_impassable(&mut tile, index, true);
        }
        let raw = tile.write();
        assert!(vale_assets::world::adt::Adt::parse(&raw).is_ok());
        let read = AdtFile::parse(&raw).expect("re-parses");
        assert_eq!(count(&read), 86);
        assert_eq!(read.write(), raw);
    }
}
