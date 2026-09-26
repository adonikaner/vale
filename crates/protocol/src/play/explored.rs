//! **Where the character has been** — `PLAYER_EXPLORED_ZONES`, 64 update fields
//! read as one bit string.
//!
//! It is the only piece of *server* state the world map draws, and the only
//! thing in this crate whose consumer is a picture rather than a number: an
//! explored sub-region is a `WorldMapOverlay` row painted over a zone's
//! parchment, and an unexplored one is bare paper. See
//! [`vale_assets::tables::worldmap::WorldMap::overlays`], which is the join.
//!
//! ## One bit string, indexed two ways, and they agree
//!
//! The field is 64 dwords (`fields::player::EXPLORED_ZONES_1`, size 64), and the
//! two ends of the wire index it differently:
//!
//! ```text
//! server   PLAYER_EXPLORED_ZONES_1 + (bit / 32),  1 << (bit % 32)   Player.cpp
//! client   byte[bit / 8],                         1 << (bit % 8)
//! ```
//!
//! Those are the same bit of the same little-endian bit string, which is what
//! makes either reading safe to copy. This file takes the **client's**, because
//! the client's is the one with the range check the interface depends on:
//! the client compares `bit / 8` against 256 and takes the *explored* branch
//! when it is out of range, so an overlay naming an area whose bit is past the field
//! is shown rather than hidden. [`Explored::is_explored`] keeps that.
//!
//! ## `PRIVATE`, so it is only ever ours
//!
//! vmangos marks the field `UF_FLAG_PRIVATE` — nobody else's update block
//! carries it, so [`Explored::read`] answers `None` for every unit but the local
//! player, exactly as [`crate::play::stats::UnitStats::read`] does and for the same
//! reason.
//!
//! **`None` and "nothing explored" are different answers and both are drawn the
//! same way**, which is why nothing here tries to distinguish them: a fresh
//! character really has an all-zero mask, and a map with no overlays on it is
//! what the reference draws for one.

use crate::state::fields::player::EXPLORED_ZONES_1;
use crate::state::objects::Entity;

/// How many dwords the field is — vmangos's `PLAYER_EXPLORED_ZONES_SIZE`, and
/// the `size 64` beside the constant in [`crate::state::fields`].
pub const DWORDS: usize = 64;

/// …and how many bits that comes to. The client's 256-byte bound is this over 8.
pub const BITS: u32 = (DWORDS * 32) as u32;

/// The character's exploration mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explored {
    words: [u32; DWORDS],
}

impl Default for Explored {
    fn default() -> Self {
        Explored {
            words: [0; DWORDS],
        }
    }
}

impl Explored {
    /// Read the mask off a unit's fields, or `None` for a unit that carries none
    /// — which is everyone but us. See the module comment.
    ///
    /// **A single set bit is enough to count as carrying one**, and the reason
    /// is the update protocol rather than taste: a *values* block sends only the
    /// dwords that changed, so a character who has just discovered one area
    /// arrives with 1 of the 64 present. Testing for all 64 would answer `None`
    /// for every session after the create block.
    pub fn read(entity: &Entity) -> Option<Explored> {
        Self::decode(|index| entity.field(index))
    }

    /// The same read against a bare field lookup, for the tests and for anything
    /// holding fields rather than an [`Entity`].
    pub fn decode(field: impl Fn(u16) -> Option<u32>) -> Option<Explored> {
        let mut words = [0u32; DWORDS];
        let mut any = false;
        for (i, word) in words.iter_mut().enumerate() {
            if let Some(value) = field(EXPLORED_ZONES_1 + i as u16) {
                *word = value;
                any = true;
            }
        }
        any.then_some(Explored { words })
    }

    /// **Whether `bit` has been explored** — the client's test, range check
    /// included.
    ///
    /// A bit past the end of the field answers **true**, which is the client's
    /// own branch and not a convenience: it takes the accepting path for a byte
    /// index past the field, so an area whose
    /// `areaBit` is out of range is treated as seen. Nothing in the shipped
    /// tables reaches it — the largest `AreaTable` bit is well under
    /// [`BITS`] — and it is copied rather than dropped because a rule this
    /// shaped is not one to re-derive from the cases that occur.
    pub fn is_explored(&self, bit: u32) -> bool {
        if bit >= BITS {
            return true;
        }
        self.words[(bit / 32) as usize] & (1 << (bit % 32)) != 0
    }

    /// How many bits are set — the one number a check can compare against a
    /// server's own `.explorecheat`, and what `vale live` prints.
    pub fn count(&self) -> u32 {
        self.words.iter().map(|word| word.count_ones()).sum()
    }

    /// Set a bit, for the tests and for the CLI's "what would a fully explored
    /// map look like" probe. Out of range is ignored rather than panicking, on
    /// [`Self::is_explored`]'s own terms.
    pub fn set(&mut self, bit: u32) {
        if bit < BITS {
            self.words[(bit / 32) as usize] |= 1 << (bit % 32);
        }
    }

    /// Everything explored — what the server's `.explorecheat 1` writes, and the
    /// only way to see every overlay a zone has without walking it.
    pub fn everything() -> Explored {
        Explored {
            words: [u32::MAX; DWORDS],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two indexings agree, which is the whole claim the module comment
    /// makes: a bit the *server* would set in dword `n` at `1 << (b % 32)` is
    /// the bit this file answers for.
    #[test]
    fn the_servers_word_and_the_clients_byte_are_the_same_bit() {
        for bit in [0u32, 1, 7, 8, 31, 32, 33, 255, 256, 2047] {
            let mut fields = vec![(EXPLORED_ZONES_1 + (bit / 32) as u16, 1u32 << (bit % 32))];
            // …and one unrelated word set, so "any field present" is not what
            // is being measured.
            fields.push((EXPLORED_ZONES_1 + 63, 0));
            let explored = Explored::decode(|index| {
                fields
                    .iter()
                    .find(|(at, _)| *at == index)
                    .map(|(_, value)| *value)
            })
            .expect("the field is present");
            assert!(explored.is_explored(bit), "bit {bit}");
            assert_eq!(explored.count(), 1, "bit {bit}");
            // Its neighbours are not.
            assert!(!explored.is_explored(bit + 1) || bit + 1 >= BITS);
        }
    }

    /// **A unit with none of the field has no mask**, which is every unit but
    /// the local player — and a unit with *one* dword of it has one, which is
    /// what a values block after a discovery looks like.
    #[test]
    fn only_a_unit_that_carries_the_field_has_a_mask() {
        assert_eq!(Explored::decode(|_| None), None);
        let one = Explored::decode(|index| (index == EXPLORED_ZONES_1 + 3).then_some(0))
            .expect("one dword is a mask");
        assert_eq!(one.count(), 0, "present and empty is not absent");
    }

    /// **A bit past the field is explored**, which is the client's own branch —
    /// see [`Explored::is_explored`].
    #[test]
    fn a_bit_past_the_end_of_the_field_reads_as_seen() {
        let empty = Explored::default();
        assert!(!empty.is_explored(BITS - 1));
        assert!(empty.is_explored(BITS));
        assert!(empty.is_explored(u32::MAX));
        assert_eq!(Explored::everything().count(), BITS);
    }
}
