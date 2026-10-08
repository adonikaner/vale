//! `/roll`: one opcode in both directions.
//!
//! ```text
//! MSG_RANDOM_ROLL  client -> server   u32 min, u32 max
//! MSG_RANDOM_ROLL  server -> client   u32 min, u32 max, u32 result, u64 roller
//! ```
//!
//! `RandomRoll(min, max)` sends the request; `ChatFrame.lua`'s `/random`,
//! `/rand`, `/rnd` and `/roll` call it with `"1", "100"` when no number is
//! typed. vmangos' `WorldSession::HandleRandomRollOpcode` drops a request whose
//! `min` is above its `max` or whose `max` is above 1,000,000, picks the result
//! with `urand(min, max)`, and sends the answer to every member of the roller's
//! group, the roller included, or to the roller alone when there is no group.
//! The roller's guid is a plain `u64`, not a packed guid.
//!
//! The client words the answer with `RANDOM_ROLL_RESULT`,
//! `"%s rolls %d (%d-%d)"`: the roller's name, the result, then the range.

use crate::bytes::{Reader, Writer};

/// The largest `max` vmangos accepts; a request above it is dropped without a
/// reply.
pub const MAX_ROLL: u32 = 1_000_000;

/// `MSG_RANDOM_ROLL` from the server, read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomRoll {
    pub min: u32,
    pub max: u32,
    pub result: u32,
    pub roller: u64,
}

/// `MSG_RANDOM_ROLL` from the server: twenty bytes.
pub fn parse_random_roll(body: &[u8]) -> Option<RandomRoll> {
    if body.len() < 20 {
        return None;
    }
    let mut r = Reader::new(body);
    Some(RandomRoll {
        min: r.u32(),
        max: r.u32(),
        result: r.u32(),
        roller: r.u64(),
    })
}

/// `MSG_RANDOM_ROLL` to the server: the range.
pub fn random_roll_body(min: u32, max: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(min).u32(max);
    w.buf
}

/// Whether vmangos answers a request for this range. A request it would drop
/// is not sent, since nothing would come back.
pub fn is_answered(min: u32, max: u32) -> bool {
    min <= max && max <= MAX_ROLL
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The result sits between the range and the guid. Read as `(min, result,
    /// max)` a roll of 37 in 1-100 would print as 100 rolled in 1-37.
    #[test]
    fn the_answer_is_range_then_result_then_roller() {
        let mut w = Writer::new();
        w.u32(1).u32(100).u32(37).u64(0x0000_0000_0000_002a);
        assert_eq!(
            parse_random_roll(&w.buf),
            Some(RandomRoll { min: 1, max: 100, result: 37, roller: 42 })
        );
        assert_eq!(parse_random_roll(&w.buf[..19]), None);
    }

    #[test]
    fn the_request_is_the_range() {
        assert_eq!(random_roll_body(1, 100), [1, 0, 0, 0, 100, 0, 0, 0]);
        assert!(is_answered(1, 100));
        assert!(is_answered(5, 5));
        assert!(!is_answered(10, 1));
        assert!(!is_answered(1, MAX_ROLL + 1));
    }
}
