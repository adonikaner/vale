//! Tutorial tips: a 256-bit mask the server keeps per account, and three
//! requests that change it.
//!
//! ```text
//! SMSG_TUTORIAL_FLAGS   8 x u32        bit n = tutorial n + 1 has been seen
//! CMSG_TUTORIAL_FLAG    u32 n          mark tutorial n + 1 as seen
//! CMSG_TUTORIAL_CLEAR   (empty)        mark every tutorial as seen: tips off
//! CMSG_TUTORIAL_RESET   (empty)        mark none as seen: tips on again
//! ```
//!
//! Sources in vmangos: `WorldSession::SendTutorialsData`
//! (`Server/WorldSession.cpp`, `ACCOUNT_TUTORIALS_COUNT` words of 32 bits) and
//! `HandleTutorialFlagOpcode`, `HandleTutorialClearOpcode` and
//! `HandleTutorialResetOpcode` (`Handlers/CharacterHandler.cpp`). The server
//! stores the mask in `character_tutorial` and sends nothing back for any of
//! the three requests.
//!
//! ## Numbering
//!
//! The interface numbers tutorials from 1 (`TUTORIAL_TITLE1` to
//! `TUTORIAL_TITLE50` in `GlobalStrings.lua`, `TUTORIAL_TRIGGER`'s argument,
//! `FlagTutorial(id)`). The wire numbers them from 0: tutorial `id` is bit
//! `id - 1`, held in word `(id - 1) / 32` at bit `(id - 1) % 32`, and
//! `CMSG_TUTORIAL_FLAG` carries `id - 1`.
//!
//! ## When the mask is sent
//!
//! `Player::SendInitialPacketsBeforeAddToMap` sends it at login and again after
//! every transfer to another map, so a session can receive it more than once.

use crate::bytes::Reader;

/// How many 32-bit words the mask has (vmangos `ACCOUNT_TUTORIALS_COUNT`).
pub const WORDS: usize = 8;

/// The highest tutorial id the interface names (`TUTORIAL_TITLE50`). The
/// 1.12.1 client ignores `FlagTutorial` for an id above it.
pub const LAST_TUTORIAL: u32 = 50;

/// The tutorial mask, one bit per tutorial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TutorialFlags(pub [u32; WORDS]);

impl TutorialFlags {
    /// Every bit set: the state `CMSG_TUTORIAL_CLEAR` leaves.
    pub const ALL: TutorialFlags = TutorialFlags([u32::MAX; WORDS]);

    /// Whether tutorial `id` (1-based) is set. An id of 0 or past the mask is
    /// reported as set, so nothing is ever raised for it.
    pub fn is_set(&self, id: u32) -> bool {
        let Some(bit) = id.checked_sub(1).map(|n| n as usize) else {
            return true;
        };
        self.0.get(bit / 32).is_none_or(|word| word & (1 << (bit % 32)) != 0)
    }

    /// Set tutorial `id` (1-based). An id outside the mask changes nothing.
    pub fn set(&mut self, id: u32) {
        let Some(bit) = id.checked_sub(1).map(|n| n as usize) else {
            return;
        };
        if let Some(word) = self.0.get_mut(bit / 32) {
            *word |= 1 << (bit % 32);
        }
    }

    /// Whether every bit is set, which is how the 1.12.1 client defines tips
    /// being turned off: `TutorialsEnabled()` answers nil exactly then.
    pub fn all_set(&self) -> bool {
        self.0.iter().all(|&word| word == u32::MAX)
    }
}

/// `SMSG_TUTORIAL_FLAGS`: eight words.
///
/// A shorter body is refused rather than padded, since vmangos always sends
/// all eight and a short one means the packet was cut.
pub fn parse_tutorial_flags(body: &[u8]) -> Option<TutorialFlags> {
    if body.len() < WORDS * 4 {
        return None;
    }
    let mut r = Reader::new(body);
    let mut words = [0u32; WORDS];
    for word in &mut words {
        *word = r.u32();
    }
    Some(TutorialFlags(words))
}

/// `CMSG_TUTORIAL_FLAG`: the zero-based index of tutorial `id`.
pub fn flag_tutorial_body(id: u32) -> Vec<u8> {
    id.saturating_sub(1).to_le_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tutorial_one_is_bit_zero_of_word_zero() {
        let mut body = vec![0u8; 32];
        body[0] = 0b0000_0010; // tutorial 2
        body[4] = 0b0000_0001; // tutorial 33
        let flags = parse_tutorial_flags(&body).unwrap();
        assert!(!flags.is_set(1));
        assert!(flags.is_set(2));
        assert!(flags.is_set(33));
        assert!(!flags.is_set(42));
        assert_eq!(parse_tutorial_flags(&body[..31]), None);
    }

    #[test]
    fn setting_and_the_cleared_state() {
        let mut flags = TutorialFlags::default();
        flags.set(42);
        assert!(flags.is_set(42));
        assert_eq!(flags.0[1], 1 << 9);
        assert!(!flags.all_set());
        assert!(TutorialFlags::ALL.all_set());
        // Ids outside the mask are never raised and never stored.
        assert!(flags.is_set(0));
        assert!(flags.is_set(257));
        flags.set(0);
        flags.set(300);
        assert_eq!(flags.0.iter().map(|w| w.count_ones()).sum::<u32>(), 1);
    }

    #[test]
    fn the_flag_request_carries_the_zero_based_index() {
        assert_eq!(flag_tutorial_body(1), [0, 0, 0, 0]);
        assert_eq!(flag_tutorial_body(42), [41, 0, 0, 0]);
    }
}
