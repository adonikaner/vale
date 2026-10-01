//! Inspecting another player: `CMSG_INSPECT`, `SMSG_INSPECT` and
//! `MSG_INSPECT_HONOR_STATS`.
//!
//! The inspect window draws the other player's gear from their visible-item
//! update fields, which every client in view already has, so `CMSG_INSPECT`
//! asks for nothing the window draws. vmangos answers it with `SMSG_INSPECT`
//! carrying the guid alone, after its range and hostility checks
//! (`Handlers/MiscHandler.cpp`, `HandleInspectOpcode`).
//!
//! The honor tab is the one part with its own answer.
//! `MSG_INSPECT_HONOR_STATS` goes out with the guid and comes back with that
//! player's kill counts and contributions; [`parse_honor_stats`] reads it.
//! Layout per vmangos' `WorldPackets::Misc::InspectHonorStatsResponse`
//! (`Server/Packets/Misc.cpp`), for a 1.6.1 or later client:
//!
//! ```text
//! u64 guid
//! u8  highest rank
//! u16 today's honorable kills     u16 today's dishonorable kills
//! u16 yesterday's honorable kills u16 unused
//! u16 last week's honorable kills u16 unused
//! u16 this week's honorable kills u16 unused
//! u32 lifetime honorable kills    u32 lifetime dishonorable kills
//! u32 yesterday's honor           u32 last week's honor
//! u32 this week's honor           u32 last week's standing
//! u8  rank progress (0..255)
//! ```
//!
//! vmangos writes today's two kill counts as one `u32`
//! (`PLAYER_FIELD_SESSION_KILLS`), honorable in the low half, which on the wire
//! is the two `u16`s above in that order.

use crate::bytes::{Reader, Writer};

/// The body is 50 bytes.
const HONOR_STATS_LEN: usize = 8 + 1 + 2 * 8 + 4 * 6 + 1;

/// One player's honor, as `MSG_INSPECT_HONOR_STATS` states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InspectHonor {
    pub guid: u64,
    pub highest_rank: u8,
    pub today_kills: u16,
    pub today_dishonorable: u16,
    pub yesterday_kills: u16,
    pub last_week_kills: u16,
    pub this_week_kills: u16,
    pub lifetime_kills: u32,
    pub lifetime_dishonorable: u32,
    pub yesterday_honor: u32,
    pub last_week_honor: u32,
    pub this_week_honor: u32,
    pub last_week_standing: u32,
    /// Progress toward the next rank, 0..255 for 0..1.
    pub rank_progress: u8,
}

/// `CMSG_INSPECT` and the `MSG_INSPECT_HONOR_STATS` request: the guid alone.
pub fn inspect_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// `SMSG_INSPECT`: the guid the server accepted the request for.
pub fn parse_inspect(body: &[u8]) -> Option<u64> {
    (body.len() >= 8).then(|| Reader::new(body).u64())
}

/// `MSG_INSPECT_HONOR_STATS` from the server. `None` for a short body.
pub fn parse_honor_stats(body: &[u8]) -> Option<InspectHonor> {
    if body.len() < HONOR_STATS_LEN {
        return None;
    }
    let mut r = Reader::new(body);
    let guid = r.u64();
    let highest_rank = r.u8();
    let today_kills = r.u16();
    let today_dishonorable = r.u16();
    let yesterday_kills = r.u16();
    let _ = r.u16();
    let last_week_kills = r.u16();
    let _ = r.u16();
    let this_week_kills = r.u16();
    let _ = r.u16();
    Some(InspectHonor {
        guid,
        highest_rank,
        today_kills,
        today_dishonorable,
        yesterday_kills,
        last_week_kills,
        this_week_kills,
        lifetime_kills: r.u32(),
        lifetime_dishonorable: r.u32(),
        yesterday_honor: r.u32(),
        last_week_honor: r.u32(),
        this_week_honor: r.u32(),
        last_week_standing: r.u32(),
        rank_progress: r.u8(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field reads back from its own place, in vmangos' order, with
    /// distinct values so that a swap of any two is caught.
    #[test]
    fn the_honor_stats_read_back_in_order() {
        let mut w = Writer::new();
        w.u64(0x42).u8(7);
        w.u16(11).u16(12).u16(13).u16(0).u16(14).u16(0).u16(15).u16(0);
        w.u32(16).u32(17).u32(18).u32(19).u32(20).u32(21).u8(128);
        assert_eq!(w.buf.len(), HONOR_STATS_LEN);
        assert_eq!(
            parse_honor_stats(&w.buf),
            Some(InspectHonor {
                guid: 0x42,
                highest_rank: 7,
                today_kills: 11,
                today_dishonorable: 12,
                yesterday_kills: 13,
                last_week_kills: 14,
                this_week_kills: 15,
                lifetime_kills: 16,
                lifetime_dishonorable: 17,
                yesterday_honor: 18,
                last_week_honor: 19,
                this_week_honor: 20,
                last_week_standing: 21,
                rank_progress: 128,
            })
        );
        assert_eq!(parse_honor_stats(&w.buf[..HONOR_STATS_LEN - 1]), None);
    }

    /// The request is the guid, and the server's inspect answer is too.
    #[test]
    fn the_inspect_bodies_are_the_guid() {
        assert_eq!(inspect_body(0x42), 0x42u64.to_le_bytes().to_vec());
        assert_eq!(parse_inspect(&0x42u64.to_le_bytes()), Some(0x42));
        assert_eq!(parse_inspect(&[0; 7]), None);
    }
}
