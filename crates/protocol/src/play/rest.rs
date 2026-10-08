//! `SMSG_SET_REST_START`: one `u32`, read and not used.
//!
//! ```text
//! SMSG_SET_REST_START   u32 seconds since resting started
//! ```
//!
//! vmangos sends it from `Player::SendInitialPacketsBeforeAddToMap`, at login
//! and after every transfer, and always with 0 (the source says "always 0 in
//! original code"). The 1.12.1 client records the time resting started from
//! it, and no interface function reads that time: `GetTimeToWellRested()`
//! answers nil whatever it holds. Whether the character is resting and how
//! rested it is come from update fields instead: `PLAYER_FLAGS` bit `0x20`
//! (`IsResting`), the rest state in `PLAYER_BYTES_2` (`GetRestState`) and
//! `PLAYER_REST_STATE_EXPERIENCE` (`GetXPExhaustion`).

use crate::bytes::Reader;

/// `SMSG_SET_REST_START`: four bytes.
pub fn parse_set_rest_start(body: &[u8]) -> Option<u32> {
    if body.len() < 4 {
        return None;
    }
    Some(Reader::new(body).u32())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_one_word() {
        assert_eq!(parse_set_rest_start(&[0, 0, 0, 0]), Some(0));
        assert_eq!(parse_set_rest_start(&[0, 0, 0]), None);
    }
}
