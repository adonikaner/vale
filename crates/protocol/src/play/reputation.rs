//! **What the character has earned** — the four packets behind the reputation
//! panel, and the three verbs it sends back.
//!
//! ```text
//! SMSG_INITIALIZE_FACTIONS  290   u32 count, then count x (u8 flags, i32 standing)
//! SMSG_SET_FACTION_VISIBLE  291   u32 reputationListId
//! SMSG_SET_FACTION_STANDING 292   u32 count, then count x (u32 listId, i32 standing)
//! SMSG_SET_FACTION_ATWAR    787   u32 reputationListId, u8 flags
//! SMSG_SET_FORCED_REACTIONS 677   u32 count, then count x (u32 factionId, u32 rank)
//!
//! CMSG_SET_FACTION_ATWAR    293   u32 reputationListId, u8 flag
//! CMSG_SET_FACTION_INACTIVE 791   u32 reputationListId, u8 inactive
//! CMSG_SET_WATCHED_FACTION  792   i32 reputationListId    (-1 clears)
//! ```
//!
//! All seven layouts are vmangos' `WorldPackets::Misc` (`Misc.h`/`Misc.cpp`),
//! which is the authority for anything that crosses the wire.
//!
//! ## The number in the packet is not the number on the bar
//!
//! `standing` here is a **delta from the character's base reputation**, and the
//! base is a `Faction.dbc` column selected by race and class — so nothing in this
//! file can turn one of these into a standing on its own. vmangos stores
//! `standing - BaseRep` (`ReputationMgr::SetOneFactionReputation`) and the client
//! adds it back. The join lives in
//! [`vale_assets::tables::reputation::Reputation`], where the rest of the
//! panel's rules are; this crate carries the two raw numbers and says so.
//!
//! ## The initial packet is fixed-width and the count is a formality
//!
//! vmangos sends a `std::array<FactionInitEntry, 64>` and writes `factions.size()`
//! in front of it, so the count is always 64 and every slot is present whether or
//! not the character has met that faction — an unmet one is flags `0`, which the
//! panel reads as "no row". [`parse_initialize_factions`] still reads the count
//! rather than assuming it, and **clamps** rather than failing: a server that
//! sends fewer leaves the tail at zero, which is the same thing as not having
//! met them.

use crate::state::objects::Entity;

/// How many reputation-list slots there are. The same 64 as
/// [`vale_assets::tables::reputation::SLOTS`]; restated here because this
/// crate does not depend on that one.
pub const SLOTS: usize = 64;

/// `SMSG_INITIALIZE_FACTIONS`, as the panel wants it: one `(flags, standing)`
/// per slot, in reputation-list order.
pub type FactionStates = [(u8, i32); SLOTS];

/// `SMSG_INITIALIZE_FACTIONS` (290).
///
/// `None` only for a body with no count in it; a short tail is *not* a failure —
/// see the module note.
pub fn parse_initialize_factions(body: &[u8]) -> Option<FactionStates> {
    let count = u32::from_le_bytes(body.get(0..4)?.try_into().ok()?) as usize;
    let mut out = [(0u8, 0i32); SLOTS];
    for (slot, entry) in out.iter_mut().enumerate().take(count.min(SLOTS)) {
        let at = 4 + slot * 5;
        let Some(bytes) = body.get(at..at + 5) else {
            break;
        };
        *entry = (bytes[0], i32::from_le_bytes(bytes[1..5].try_into().ok()?));
    }
    Some(out)
}

/// `SMSG_SET_FACTION_STANDING` (292): one or more slots' deltas moved.
///
/// More than one because vmangos flushes every `needSend` slot with the one that
/// changed — a quest that spills reputation onto a parent faction arrives as a
/// single packet naming both, and a reader that takes only the first draws one of
/// the two bars.
pub fn parse_set_faction_standing(body: &[u8]) -> Option<Vec<(u32, i32)>> {
    let count = u32::from_le_bytes(body.get(0..4)?.try_into().ok()?) as usize;
    let mut out = Vec::with_capacity(count.min(SLOTS));
    for i in 0..count {
        let at = 4 + i * 8;
        let Some(bytes) = body.get(at..at + 8) else {
            break;
        };
        out.push((
            u32::from_le_bytes(bytes[0..4].try_into().ok()?),
            i32::from_le_bytes(bytes[4..8].try_into().ok()?),
        ));
    }
    Some(out)
}

/// `SMSG_SET_FACTION_VISIBLE` (291): a faction met for the first time.
///
/// vmangos does **not** send this while the character is still loading
/// (`ReputationMgr::SendVisible` returns early on `PlayerLoading`), so everything
/// known at login arrives in `SMSG_INITIALIZE_FACTIONS` instead and this is only
/// ever the mid-session case.
pub fn parse_set_faction_visible(body: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(body.get(0..4)?.try_into().ok()?))
}

/// `SMSG_SET_FACTION_ATWAR` (787): `(reputationListId, flags)`.
///
/// The second value is the slot's **whole flag byte**, not a boolean — the
/// client masks bit 1 out of it and leaves the rest
/// of its own copy alone.
pub fn parse_set_faction_at_war(body: &[u8]) -> Option<(u32, u8)> {
    let bytes = body.get(0..5)?;
    Some((
        u32::from_le_bytes(bytes[0..4].try_into().ok()?),
        bytes[4],
    ))
}

/// `SMSG_SET_FORCED_REACTIONS` (677): the ranks the server insists on,
/// whatever `FactionTemplate.dbc` and the character's own standing say.
///
/// vmangos builds it in `ReputationMgr::SendForceReactions` out of
/// `m_forcedReactions`, which `ApplyForceReaction` writes and which
/// `SPELL_AURA_FORCE_REACTION` is the only thing in the game that touches. The
/// packet is the **whole** map every time — an aura falling off sends the map
/// without it rather than a removal — so a reader replaces its list rather than
/// merging into it.
///
/// This is the one channel by which a server can make a friendly faction
/// hostile to a particular character without changing a field on any unit, and
/// a client that ignores it draws a guard green while the server has it
/// attacking. See [`vale_assets::tables::faction::Standing`], which is where
/// the answer is used.
///
/// A body with no count is `None`; a truncated tail stops where it runs out,
/// which is [`parse_initialize_factions`]' rule and for the same reason.
pub fn parse_forced_reactions(body: &[u8]) -> Option<Vec<(u32, u32)>> {
    let count = u32::from_le_bytes(body.get(0..4)?.try_into().ok()?) as usize;
    let mut out = Vec::with_capacity(count.min(64));
    for entry in 0..count {
        let at = 4 + entry * 8;
        let Some(bytes) = body.get(at..at + 8) else {
            break;
        };
        out.push((
            u32::from_le_bytes(bytes[0..4].try_into().ok()?),
            u32::from_le_bytes(bytes[4..8].try_into().ok()?),
        ));
    }
    Some(out)
}

/// `CMSG_SET_FACTION_ATWAR` (293).
pub fn set_faction_at_war_body(reputation_list_id: u32, at_war: bool) -> Vec<u8> {
    let mut out = reputation_list_id.to_le_bytes().to_vec();
    // vmangos reads a `uint8` and tests it against zero; the client sends the
    // whole flag byte with bit 1 set or clear, and the low bit is what matters.
    out.push(if at_war { 0x02 } else { 0x00 });
    out
}

/// `CMSG_SET_FACTION_INACTIVE` (791).
pub fn set_faction_inactive_body(reputation_list_id: u32, inactive: bool) -> Vec<u8> {
    let mut out = reputation_list_id.to_le_bytes().to_vec();
    out.push(u8::from(inactive));
    out
}

/// `CMSG_SET_WATCHED_FACTION` (792).
///
/// **`-1` is the clear**, which is what `SetWatchedFactionIndex(0)` becomes: the
/// interface's index 0 resolves to no row, and the client sends the missing
/// row's reputation id. vmangos reads a signed `int32` and writes it straight
/// into `PLAYER_FIELD_WATCHED_FACTION_INDEX`.
pub fn set_watched_faction_body(reputation_list_id: i32) -> Vec<u8> {
    reputation_list_id.to_le_bytes().to_vec()
}

/// `PLAYER_FIELD_WATCHED_FACTION_INDEX` off a unit's fields, or `None` for
/// anybody but the local player — the field is `PRIVATE`, like the stat block.
///
/// The client reads it at `[playerFields + 0x10c4]`, which is index
/// 188 + 1073 = 1261 with `PLAYER_FIELD_DUEL_ARBITER` as the block's base — the
/// same conversion [`crate::play::stats`] documents.
pub fn watched_faction(entity: &Entity) -> Option<i32> {
    entity
        .field(crate::state::fields::player::WATCHED_FACTION_INDEX)
        .map(|raw| raw as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initialize(count: u32, entries: &[(u8, i32)]) -> Vec<u8> {
        let mut out = count.to_le_bytes().to_vec();
        for (flags, standing) in entries {
            out.push(*flags);
            out.extend_from_slice(&standing.to_le_bytes());
        }
        out
    }

    /// **Eight bytes an entry here and five in the packet above**, which is
    /// the trap: the two reputation packets that carry a list carry it in
    /// different widths, and reading this one at five loses every entry after
    /// the first — silently, as a forced reaction on a faction nobody has.
    #[test]
    fn a_forced_reaction_is_two_whole_words() {
        let mut body = 2u32.to_le_bytes().to_vec();
        body.extend_from_slice(&72u32.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&76u32.to_le_bytes());
        body.extend_from_slice(&7u32.to_le_bytes());
        assert_eq!(
            parse_forced_reactions(&body).expect("a count"),
            vec![(72, 1), (76, 7)]
        );
    }

    /// An empty map is what an aura falling off sends, and it has to read as
    /// "nothing forced" rather than as a body that failed — see the note on the
    /// parser about why a reader replaces its list.
    #[test]
    fn an_empty_forced_map_is_a_body_rather_than_a_failure() {
        assert_eq!(parse_forced_reactions(&0u32.to_le_bytes()), Some(Vec::new()));
        assert_eq!(parse_forced_reactions(&[]), None);
        // A count with nothing behind it stops where it runs out.
        assert_eq!(parse_forced_reactions(&4u32.to_le_bytes()), Some(Vec::new()));
    }

    /// Five bytes an entry, not eight — the flags are a `u8` and there is no
    /// padding, which is the one thing about this packet that reads plausibly
    /// wrong: a four-byte flags field shifts every standing after the first.
    #[test]
    fn an_entry_is_one_flag_byte_and_four_standing_bytes() {
        let body = initialize(64, &[(0x01, 0), (0x09, 21000), (0x21, -6000)]);
        let states = parse_initialize_factions(&body).expect("a count");
        assert_eq!(states[0], (0x01, 0));
        assert_eq!(states[1], (0x09, 21000));
        assert_eq!(states[2], (0x21, -6000));
        assert_eq!(states[3], (0, 0), "…and the tail is unmet, not garbage");
    }

    /// A truncated tail is the ordinary case for a server that sends fewer than
    /// 64, and it must not lose the entries that did arrive.
    #[test]
    fn a_short_body_keeps_what_it_carried() {
        let body = initialize(64, &[(0x01, 100)]);
        let states = parse_initialize_factions(&body).expect("a count");
        assert_eq!(states[0], (0x01, 100));
        assert_eq!(states[63], (0, 0));
        assert!(parse_initialize_factions(&[1, 2]).is_none(), "no count at all");
    }

    /// The standing packet is a *list*, because reputation spills onto parents.
    #[test]
    fn a_standing_update_may_name_more_than_one_slot() {
        let mut body = 2u32.to_le_bytes().to_vec();
        body.extend_from_slice(&19u32.to_le_bytes());
        body.extend_from_slice(&1500i32.to_le_bytes());
        body.extend_from_slice(&11u32.to_le_bytes());
        body.extend_from_slice(&(-25i32).to_le_bytes());
        assert_eq!(
            parse_set_faction_standing(&body).unwrap(),
            vec![(19, 1500), (11, -25)]
        );
    }

    /// The at-war packet carries the whole flag byte and the client reads one
    /// bit of it.
    #[test]
    fn the_at_war_packet_carries_flags_rather_than_a_boolean() {
        let mut body = 19u32.to_le_bytes().to_vec();
        body.push(0x13);
        let (rep, flags) = parse_set_faction_at_war(&body).unwrap();
        assert_eq!(rep, 19);
        assert_eq!(flags & 0x02, 0x02, "at war");
        assert_eq!(flags & 0x10, 0x10, "…and peace-forced, which is not our bit");
    }

    /// The three outbound bodies, byte for byte against vmangos' readers.
    #[test]
    fn the_three_verbs_are_the_widths_the_server_reads() {
        assert_eq!(set_faction_at_war_body(19, true), vec![19, 0, 0, 0, 0x02]);
        assert_eq!(set_faction_at_war_body(19, false), vec![19, 0, 0, 0, 0x00]);
        assert_eq!(set_faction_inactive_body(7, true), vec![7, 0, 0, 0, 1]);
        assert_eq!(set_watched_faction_body(-1), vec![0xff, 0xff, 0xff, 0xff]);
        assert_eq!(set_watched_faction_body(19), vec![19, 0, 0, 0]);
    }
}
