//! **Being summoned: one packet in, one out.**
//!
//! A warlock's Ritual of Summoning, a meeting stone and a GM's summon all end
//! in `Player::SendSummonRequest`, which stores the destination on the server
//! and asks the player:
//!
//! ```text
//! SMSG_SUMMON_REQUEST    u64 summoner, u32 zone, u32 autoDeclineDelay (ms)
//! CMSG_SUMMON_RESPONSE   u64 summoner
//! ```
//!
//! **The answer is only ever yes.** The reference's `ConfirmSummon` sends the stored guid and nothing else, and the popup has no
//! decline that sends anything: a summon nobody accepts expires on the server
//! at `autoDeclineDelay` (`m_summon_expire`). vmangos' `SummonResponse` reads
//! the guid alone, which agrees.
//!
//! The zone is an `AreaTable` id — the summoner's zone, not the destination
//! point — and it is what `GetSummonConfirmAreaName` names.

use crate::bytes::{Reader, Writer};

/// `SMSG_SUMMON_REQUEST`, read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummonRequest {
    pub summoner: u64,
    /// `AreaTable.dbc` — the summoner's zone.
    pub zone: u32,
    /// How long the offer stands, in milliseconds. vmangos sends
    /// `MAX_PLAYER_SUMMON_DELAY` seconds (two minutes).
    pub delay_ms: u32,
}

/// `SMSG_SUMMON_REQUEST` — sixteen bytes.
pub fn parse_summon_request(body: &[u8]) -> Option<SummonRequest> {
    if body.len() < 16 {
        return None;
    }
    let mut r = Reader::new(body);
    Some(SummonRequest {
        summoner: r.u64(),
        zone: r.u32(),
        delay_ms: r.u32(),
    })
}

/// `CMSG_SUMMON_RESPONSE` — the summoner's guid, which is the whole of "yes".
pub fn summon_response_body(summoner: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(summoner);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The field order is the thing to pin: the zone and the delay are two
    /// adjacent `u32`s, and swapped they read as a zone id of 120,000 and an
    /// offer that expires in a few milliseconds.
    #[test]
    fn the_summon_request_reads_back_what_was_written() {
        let mut w = Writer::new();
        w.u64(0x0000_0000_0000_0009).u32(1519).u32(120_000);
        assert_eq!(
            parse_summon_request(&w.buf),
            Some(SummonRequest { summoner: 9, zone: 1519, delay_ms: 120_000 })
        );
        assert_eq!(parse_summon_request(&w.buf[..15]), None);
        assert_eq!(summon_response_body(9), 9u64.to_le_bytes().to_vec());
    }
}
