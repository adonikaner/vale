//! **Dying, and getting up again.** The packet family between health reaching
//! zero and standing back up, and the four different ways that can happen.
//!
//! ```text
//! (health -> 0)                                    the only announcement there is
//! CMSG_REPOP_REQUEST      (no body)                release the spirit
//! SMSG_CORPSE_RECLAIM_DELAY  u32 delayMs           …and how long before you may take it back
//! MSG_CORPSE_QUERY        (no body)                where did I leave the body?
//! MSG_CORPSE_QUERY        u8 found [, i32 mapId, f32 x, f32 y, f32 z, u32 corpseMapId]
//! CMSG_RECLAIM_CORPSE     u64 guid                 stand up on the corpse
//! SMSG_RESURRECT_REQUEST  u64 caster, u32 len, cstring name, u8 sickness, u8 timer
//! CMSG_RESURRECT_RESPONSE u64 caster, u8 accept    …yes or no
//! SMSG_SPIRIT_HEALER_CONFIRM u64 healer            the spirit healer's offer
//! CMSG_SPIRIT_HEALER_ACTIVATE u64 healer           …taken, at 25% durability
//! ```
//!
//! ## Nothing here says "you died"
//!
//! There is no death packet. `Unit::SetDeathState(JUST_DIED)` writes the health
//! to zero and roots a player (`Unit.cpp:7618`), and that is the whole of the
//! statement — which is why [`crate::state::objects::Entity::is_dead`] is health and
//! not a flag, and why a create block for a corpse expresses the death by
//! *omitting* the field (`Object::_SetCreateBits` skips every zero).
//!
//! The client's own state is therefore three booleans over two fields:
//!
//! * **dead** — `UNIT_FIELD_HEALTH == 0`. The body is on the floor and the
//!   spirit is still in it; the game's own `DEATH` box is up.
//! * **ghost** — `PLAYER_FLAGS & PLAYER_FLAGS_GHOST`, which
//!   `Player::BuildPlayerRepop` sets by casting spell 8326 on the player. The
//!   spirit has been released, the health is 1 (not 0 — a ghost is *alive* to
//!   every other rule in the game), and the body is somewhere else.
//! * **may the release box be shown at all** — `PLAYER_FIELD_BYTES`' flag byte,
//!   which `Player::KillPlayer` writes with `PLAYER_FIELD_BYTE_RELEASE_TIMER`
//!   set **iff the map is not instanceable**. In a dungeon there is no six
//!   minute auto-release, and `GetReleaseTimeRemaining()` answering `-1` is
//!   what makes the box say `DEATH_RELEASE_NOTIMER` instead of counting.
//!
//! ## Two clocks, and the client owns one of them
//!
//! Unlike the logout countdown — which is entirely the server's, see
//! [`crate::play::logout`] — these two are the client's own and neither is ever
//! restated:
//!
//! * **The auto-release**, [`AUTO_RELEASE_SECS`]: `Player::KillPlayer` sets
//!   `m_deathTimer = CORPSE_REPOP_TIME` (6 minutes) and the server releases the
//!   spirit itself when it expires. The client counts the same six minutes down
//!   in the `DEATH` box; nothing on the wire syncs them.
//! * **The reclaim delay**, which *is* stated — once, by
//!   `SMSG_CORPSE_RECLAIM_DELAY` at the moment of release — and then counted
//!   down locally. Thirty seconds for a first death, then 60, then 120, on a
//!   five-minute decay (`copseReclaimDelay`, `DEATH_EXPIRE_STEP`). The server
//!   re-checks it in `HandleReclaimCorpseOpcode` and **drops the packet
//!   silently** if it has not elapsed, so a client that ignored it would offer
//!   a button that does nothing.

use crate::bytes::{Reader, Writer};

/// `CORPSE_RECLAIM_RADIUS` (`Corpse.h:40`) — how close the ghost has to be to
/// its own body for `CMSG_RECLAIM_CORPSE` to be accepted, in yards.
///
/// It is the client's job to notice: `CORPSE_IN_RANGE` and `CORPSE_OUT_OF_RANGE`
/// are events the interface registers for (`UIParent.lua`) and nothing on the
/// wire raises them. The server's own check is the same distance, so a client
/// that guessed low would simply never offer the button and one that guessed
/// high would offer a button the server drops.
pub const CORPSE_RECLAIM_RADIUS: f32 = 39.0;

/// `CORPSE_REPOP_TIME` (`Player.h:71`) — six minutes, after which the *server*
/// releases the spirit whether or not the box was pressed.
pub const AUTO_RELEASE_SECS: u32 = 6 * 60;

/// `PLAYER_FLAGS_GHOST` (`Player.h:319`) — the spirit has been released.
///
/// Set by `Player::BuildPlayerRepop` through aura spell 8326 and cleared by
/// `ResurrectPlayer`. **It is what separates a corpse from a ghost**, and
/// neither is derivable from the health: a corpse is health 0, a ghost is
/// health 1, and 1 is also what a nearly-dead living player has.
pub const PLAYER_FLAGS_GHOST: u32 = 0x0000_0010;

/// `PLAYER_FIELD_BYTE_RELEASE_TIMER` (`Player.h:378`), in byte 0 of
/// `PLAYER_FIELD_BYTES` — "display time till auto release spirit".
///
/// `Player::KillPlayer` sets it for a non-instanceable map and clears it
/// otherwise, which is the whole of why dying in a dungeon shows a release box
/// with no countdown on it.
pub const RELEASE_TIMER: u32 = 0x08;

/// `PLAYER_FIELD_BYTE_NO_RELEASE_WINDOW` (`Player.h:379`) — do not offer the
/// release box at all.
pub const NO_RELEASE_WINDOW: u32 = 0x10;

/// Where the body is, as `MSG_CORPSE_QUERY` answers it.
///
/// **`map_id` and `corpse_map_id` are different questions and both are sent.**
/// When the corpse is inside an instance the server answers with the *entrance*
/// map and the entrance's coordinates (`temp->ghostEntranceMap`), so the ghost
/// is pointed at the door rather than at a place it cannot walk to; the corpse's
/// real map comes second, and is what decides whether the arrow on the world map
/// is drawn at all.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CorpseLocation {
    /// Where to send the ghost — the corpse's map, or an instance's entrance.
    pub map_id: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// The map the body is actually lying on.
    pub corpse_map_id: u32,
}

/// Somebody has offered to resurrect us — `SMSG_RESURRECT_REQUEST`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResurrectOffer {
    /// Who cast it. **Echoed verbatim in the answer**, and a `CMSG_RESURRECT_RESPONSE`
    /// carrying a zero guid is logged as an "Instant resurrect hack" by
    /// `HandleResurrectResponseOpcode` before anything else is read.
    pub caster: u64,
    /// The caster's name — **empty when a player cast it**, which is not a bug:
    /// `Spell::SendResurrectRequest` sends `""` for a player caster and the
    /// creature's name otherwise, because the interface has the player's name
    /// already and a creature's it would have to query.
    pub name: String,
    /// Whether accepting brings resurrection sickness with it, which is the
    /// choice between three different popups in `UIParent.lua`.
    pub sickness: bool,
    /// Whether the reclaim delay applies. Cleared for a spell carrying
    /// `SPELL_ATTR_EX3_NO_RES_TIMER` — a soulstone, a battleground rebirth.
    pub timer: bool,
}

/// `SMSG_CORPSE_RECLAIM_DELAY` — `{u32 delayMs}`.
///
/// Sent once, at release. The value is already in milliseconds
/// (`delay * IN_MILLISECONDS`).
pub fn parse_corpse_reclaim_delay(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(4).then(|| r.u32())
}

/// `MSG_CORPSE_QUERY`, the reply — `{u8 found}` and, if found, five more values.
///
/// `Some(None)` is "the server looked and there is no corpse", which is a real
/// and common answer (alive, or the bones have already been spawned);
/// `None` is a body that would not parse.
pub fn parse_corpse_query(body: &[u8]) -> Option<Option<CorpseLocation>> {
    let mut r = Reader::new(body);
    if !r.has(1) {
        return None;
    }
    if r.u8() == 0 {
        return Some(None);
    }
    if !r.has(4 * 5) {
        return None;
    }
    let map_id = r.u32() as i32;
    Some(Some(CorpseLocation {
        map_id,
        x: r.f32(),
        y: r.f32(),
        z: r.f32(),
        corpse_map_id: r.u32(),
    }))
}

/// `SMSG_RESURRECT_REQUEST` — `{u64 caster, u32 nameLen, cstring name, u8 sickness, u8 timer}`.
///
/// **The length is sent *and* the string is terminated**, which is unusual
/// enough to be worth stating: `Spell::SendResurrectRequest` writes
/// `strlen(name) + 1` and then the string through `WorldPacket::operator<<`,
/// which appends its own NUL. The length is therefore redundant and is skipped
/// rather than trusted — reading the string is what advances the cursor.
pub fn parse_resurrect_request(body: &[u8]) -> Option<ResurrectOffer> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    let caster = r.u64();
    let _name_len = r.u32();
    let name = r.cstring();
    if !r.has(2) {
        return None;
    }
    Some(ResurrectOffer {
        caster,
        name,
        sickness: r.u8() != 0,
        timer: r.u8() != 0,
    })
}

/// `SMSG_SPIRIT_HEALER_CONFIRM` — `{u64 healer}`.
pub fn parse_spirit_healer_confirm(body: &[u8]) -> Option<u64> {
    let mut r = Reader::new(body);
    r.has(8).then(|| r.u64())
}

/// `CMSG_RECLAIM_CORPSE` — `{u64 guid}`, the player's own.
///
/// `HandleReclaimCorpseOpcode` reads the guid and then never uses it: every
/// check it makes is against `GetPlayer()`'s own corpse. It is still sent,
/// because a body the server reads and this client does not write is a
/// desynchronised stream rather than an ignored field.
pub fn reclaim_corpse_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// `CMSG_RESURRECT_RESPONSE` — `{u64 caster, u8 accept}`.
///
/// The guid is the one the offer arrived with, not ours:
/// `IsRessurectRequestedBy` compares it against the stored caster and a
/// mismatch is silently dropped.
pub fn resurrect_response_body(caster: u64, accept: bool) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(caster).u8(u8::from(accept));
    w.buf
}

/// `CMSG_SPIRIT_HEALER_ACTIVATE` — `{u64 healer}`.
pub fn spirit_healer_activate_body(healer: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(healer);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reply's two shapes, in `HandleCorpseQueryOpcode`'s own order.
    #[test]
    fn a_corpse_query_answers_with_a_flag_and_then_a_place() {
        assert_eq!(parse_corpse_query(&[0]), Some(None), "no corpse");

        let mut w = Writer::new();
        w.u8(1)
            .u32(0)
            .f32(-8949.95)
            .f32(-132.49)
            .f32(83.53)
            .u32(0);
        let found = parse_corpse_query(&w.buf).expect("five values after the flag");
        let place = found.expect("found");
        assert_eq!(place.map_id, 0);
        assert_eq!(place.corpse_map_id, 0);
        assert!((place.x - -8949.95).abs() < 0.01);

        // **A truncated tail is not "no corpse".** Answering `Some(None)` for a
        // short body would send the ghost's arrow to the origin of the map.
        assert_eq!(parse_corpse_query(&w.buf[..10]), None);
        assert_eq!(parse_corpse_query(&[]), None);
    }

    /// An instance corpse answers with two different maps, which is the whole
    /// reason the second value exists.
    #[test]
    fn an_instance_corpse_points_at_the_entrance_and_names_its_own_map() {
        let mut w = Writer::new();
        // Deadmines (map 36) body, pointed at Westfall (map 0)'s entrance.
        w.u8(1).u32(0).f32(-11208.0).f32(1666.0).f32(24.6).u32(36);
        let place = parse_corpse_query(&w.buf).unwrap().unwrap();
        assert_eq!(place.map_id, 0, "walk to the door");
        assert_eq!(place.corpse_map_id, 36, "the body is inside");
    }

    /// `Spell::SendResurrectRequest`'s own field order, with the length that is
    /// deliberately not trusted.
    #[test]
    fn a_resurrect_offer_carries_its_caster_and_its_two_flags() {
        let mut w = Writer::new();
        w.u64(0x1234_5678).u32(6).cstring("Priest").u8(1).u8(0);
        assert_eq!(
            parse_resurrect_request(&w.buf),
            Some(ResurrectOffer {
                caster: 0x1234_5678,
                name: "Priest".to_string(),
                sickness: true,
                timer: false,
            })
        );

        // A player caster sends an empty name and the interface uses its own.
        let mut w = Writer::new();
        w.u64(9).u32(1).cstring("").u8(0).u8(1);
        let offer = parse_resurrect_request(&w.buf).expect("an empty name is a name");
        assert_eq!(offer.name, "");
        assert!(!offer.sickness);
        assert!(offer.timer);

        // The two flags are the last two bytes and both are required: a body
        // that stops after the name would otherwise read as "no sickness", which
        // is the one answer that costs the player something.
        assert_eq!(parse_resurrect_request(&w.buf[..w.buf.len() - 1]), None);
        assert_eq!(parse_resurrect_request(&[0; 8]), None);
    }

    /// The answer echoes the caster the offer named, because the server
    /// compares them.
    #[test]
    fn the_resurrect_answer_echoes_the_caster_it_was_offered_by() {
        assert_eq!(
            resurrect_response_body(0x0102_0304_0506_0708, true),
            vec![8, 7, 6, 5, 4, 3, 2, 1, 1]
        );
        assert_eq!(resurrect_response_body(1, false).last(), Some(&0));
    }

    #[test]
    fn the_reclaim_delay_is_milliseconds_and_needs_all_four_bytes() {
        assert_eq!(parse_corpse_reclaim_delay(&[0x30, 0x75, 0, 0]), Some(30_000));
        assert_eq!(parse_corpse_reclaim_delay(&[0x30, 0x75, 0]), None);
    }
}
