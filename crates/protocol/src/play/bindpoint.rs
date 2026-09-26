//! **Where the hearthstone sends you, and how it is changed.**
//!
//! Five opcodes, of which this client reads three and sends one. The subject is
//! small and the failure it produced was not: with none of it read, the
//! hearthstone's tooltip said *"Returns you to ."* and the innkeeper's
//! *"Make this inn your home"* did nothing at all.
//!
//! ## Why the gossip option looked dead
//!
//! It was not. `Player::OnGossipSelect`'s `GOSSIP_OPTION_INNKEEPER` arm is
//!
//! ```text
//! PlayerTalkClass->CloseGossip();
//! SetBindPoint(guid);
//! ```
//!
//! and `SetBindPoint` sends `SMSG_BINDER_CONFIRM` and nothing else. **The bind
//! has not happened at that point.** It happens when the client answers
//! `CMSG_BINDER_ACTIVATE`, which makes the innkeeper cast spell 3286 at the
//! player (`WorldSession::SendBindPoint`); `Spell::EffectBind` then writes the
//! home and sends `SMSG_BINDPOINTUPDATE` and `SMSG_PLAYERBOUND`.
//!
//! So a client that drops the confirmation gets the *whole* of the visible
//! effect anyway — the gossip window closes, because `CloseGossip` runs first —
//! and nothing else ever happens. That is exactly "clicking it does nothing",
//! and there is no unanswered packet to notice it by unless the unhandled list
//! is read.
//!
//! ## The three inbound bodies
//!
//! From `WorldPackets::Misc` and `WorldPackets::Npc` in vmangos:
//!
//! ```text
//! SMSG_BINDPOINTUPDATE   f32 x, f32 y, f32 z, u32 mapId, u32 areaId
//! SMSG_PLAYERBOUND       u64 binderGuid, u32 areaId
//! SMSG_BINDER_CONFIRM    u64 binderGuid
//! ```
//!
//! `SMSG_BINDPOINTUPDATE` arrives **once in the login burst** as well as after
//! a bind — `SendInitialPacketsBeforeAddToMap` sends it from `m_homebind` — so
//! a character who has never spoken to an innkeeper still has a home, and the
//! tooltip has something to say from the first frame.
//!
//! `SMSG_BINDZONEREPLY` (343) is the fifth and is not read: nothing on this
//! client asks the question it answers.

use crate::bytes::{Reader, Writer};

/// **Where the character's hearthstone returns them to.**
///
/// The area id is the useful half — it is what `GetBindLocation()` answers,
/// through `AreaTable.dbc`, and what the hearthstone's own `$z` is substituted
/// with. The position is kept because the packet states it and because it is
/// the only thing that says *which map* the home is on, which an area id alone
/// does not settle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BindPoint {
    pub position: [f32; 3],
    pub map_id: u32,
    /// `AreaTable.dbc` row — a **sub-zone** where the inn has one, which is
    /// what makes the tooltip read "Goldshire" rather than "Elwynn Forest".
    pub area_id: u32,
}

/// `SMSG_BINDPOINTUPDATE` — 20 bytes, and every one of them fixed width.
pub fn parse_bind_point(body: &[u8]) -> Option<BindPoint> {
    if body.len() < 20 {
        return None;
    }
    let mut r = Reader::new(body);
    let position = [r.f32(), r.f32(), r.f32()];
    Some(BindPoint {
        position,
        map_id: r.u32(),
        area_id: r.u32(),
    })
}

/// `SMSG_PLAYERBOUND` — who bound us and where, `(binder guid, area id)`.
///
/// The area id repeats `SMSG_BINDPOINTUPDATE`'s, which arrives in the same
/// `Spell::EffectBind`. It is read anyway because the two are sent
/// independently and a client that trusted only one of them would be reading a
/// packet it does not need to.
pub fn parse_player_bound(body: &[u8]) -> Option<(u64, u32)> {
    if body.len() < 12 {
        return None;
    }
    let mut r = Reader::new(body);
    Some((r.u64(), r.u32()))
}

/// `SMSG_BINDER_CONFIRM` — the guid of the innkeeper asking.
pub fn parse_binder_confirm(body: &[u8]) -> Option<u64> {
    if body.len() < 8 {
        return None;
    }
    Some(Reader::new(body).u64())
}

/// `CMSG_BINDER_ACTIVATE` — the answer, and the packet that actually binds.
///
/// `HandleBinderActivateOpcode` looks the guid up with
/// `GetNPCIfCanInteractWith(guid, UNIT_NPC_FLAG_INNKEEPER)`, so it must be the
/// innkeeper's own guid and not the player's: a wrong one is dropped silently,
/// which would read as the confirmation doing nothing.
pub fn binder_activate_body(npc_guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(npc_guid);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The three bodies, byte for byte** — built the way vmangos' own
    /// `AppendBodyTo` writes them.
    ///
    /// The field order of `SMSG_BINDPOINTUPDATE` is the thing worth pinning:
    /// the map and the area are two adjacent `u32`s at the end, and a reader
    /// that swapped them would resolve a plausible area name for the wrong
    /// place rather than failing.
    #[test]
    fn the_three_inbound_bodies_read_back_what_was_written() {
        let mut w = Writer::new();
        w.f32(-8867.68).f32(673.373).f32(97.9034).u32(0).u32(1519);
        let point = parse_bind_point(&w.buf).expect("20 bytes is a whole one");
        assert_eq!(point.map_id, 0);
        assert_eq!(point.area_id, 1519, "Stormwind City, the human home");
        assert!((point.position[0] + 8867.68).abs() < 0.01);

        let mut w = Writer::new();
        w.u64(0xF130_0000_0000_0007).u32(1519);
        assert_eq!(
            parse_player_bound(&w.buf),
            Some((0xF130_0000_0000_0007, 1519))
        );

        let mut w = Writer::new();
        w.u64(0xF130_0000_0000_0007);
        assert_eq!(parse_binder_confirm(&w.buf), Some(0xF130_0000_0000_0007));

        // …and the answer is the guid alone, which is what
        // `GetNPCIfCanInteractWith` looks up.
        assert_eq!(
            binder_activate_body(0xF130_0000_0000_0007),
            0xF130_0000_0000_0007u64.to_le_bytes().to_vec()
        );
    }

    /// **A short body is `None`, not a panic.** Every parser in this crate
    /// answers that way, and the confirmation is the one here that would
    /// otherwise be read during the login burst.
    #[test]
    fn a_truncated_body_is_declined() {
        assert_eq!(parse_bind_point(&[0u8; 19]), None);
        assert_eq!(parse_player_bound(&[0u8; 11]), None);
        assert_eq!(parse_binder_confirm(&[0u8; 7]), None);
    }
}
