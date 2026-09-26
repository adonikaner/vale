//! **`/played`: one bodiless request and an eight-byte answer.**
//!
//! ```text
//! CMSG_PLAYED_TIME   (empty)
//! SMSG_PLAYED_TIME   u32 total seconds, u32 seconds at this level
//! ```
//!
//! `RequestTimePlayed` sends the request and nothing else. The
//! answer raises `TIME_PLAYED_MSG` with the two numbers as `arg1` and `arg2`,
//! and `ChatFrame_DisplayTimePlayed` words them. vmangos' `HandlePlayedTime`
//! answers from `GetTotalPlayedTime` and `GetLevelPlayedTime`.

/// `SMSG_PLAYED_TIME` — `(total, this level)`, both in seconds.
pub fn parse_played_time(body: &[u8]) -> Option<(u32, u32)> {
    if body.len() < 8 {
        return None;
    }
    let mut r = crate::bytes::Reader::new(body);
    Some((r.u32(), r.u32()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn played_time_is_total_then_level() {
        let mut body = 90_061u32.to_le_bytes().to_vec();
        body.extend_from_slice(&3_600u32.to_le_bytes());
        assert_eq!(parse_played_time(&body), Some((90_061, 3_600)));
        assert_eq!(parse_played_time(&body[..7]), None);
    }
}
