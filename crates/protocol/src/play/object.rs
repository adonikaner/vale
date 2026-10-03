//! Game objects (doors, chests, ore veins, mailboxes, levers): the packet that
//! uses one, and the two packets that play a one-shot animation on one.
//!
//! ```text
//! CMSG_GAMEOBJ_USE              u64 guid             right-click it
//! SMSG_GAMEOBJECT_CUSTOM_ANIM   u64 guid, u32 anim   play Custom0..Custom3
//! SMSG_GAMEOBJECT_DESPAWN_ANIM  u64 guid             play Despawn
//! ```
//!
//! ## What a use produces
//!
//! There is no `SMSG_GAMEOBJ_USE_RESPONSE`. The result of a use arrives
//! through the subsystem the server sends it to, so this module holds only the
//! request:
//!
//! * A door or a lever changes `GAMEOBJECT_STATE` in an update block, and the
//!   model swaps between its `Opened` and `Closed` sequences in
//!   `crates/client/src/world/entities/pose.rs`. Nothing here distinguishes a
//!   door from a chest.
//! * A chest answers with `SMSG_LOOT_RESPONSE`, as a corpse does.
//! * An ore vein or a herb is a chest whose lock names a skill, so the server
//!   casts the gathering spell at the character and the answer is
//!   `SMSG_SPELL_START` and then a loot window.
//! * A quest goober answers with `SMSG_QUESTGIVER_QUEST_DETAILS`.
//! * A refusal is a message from the client's message table
//!   (`ERR_DOOR_LOCKED`, `ERR_USE_LOCKED_WITH_SPELL_S`), which reaches the error
//!   frame like every other one.
//!
//! The client sends the request and predicts nothing. A door that will not
//! open must not swing shut on the client and snap back a round trip later.
//!
//! ## What may be used is decided elsewhere
//!
//! Which of the thirty `GAMEOBJECT_TYPE_*` values react to a click, and which
//! pointer belongs over one, is a rule about the template, not about the wire.
//! It is in `vale_assets::look::object`, where it can be unit-tested with no
//! socket.
//!
//! ## The two animation packets
//!
//! vmangos sends `SMSG_GAMEOBJECT_CUSTOM_ANIM` from
//! `GameObject::SendGameObjectCustomAnim` (`GameObject.cpp:2452`): a fishing
//! bobber when a fish bites, a trap when it fires, the Eastern Plaguelands
//! tower banners, and `ScriptCommands`' `playCustomAnim`.
//! `SMSG_GAMEOBJECT_DESPAWN_ANIM` comes from `SendObjectDeSpawnAnim`, before a
//! despawn. The 1.12.1 client plays a custom animation only for `anim` 0..3
//! and ignores a larger value. It has no handler for
//! `SMSG_GAMEOBJECT_SPAWN_ANIM` or `SMSG_GAMEOBJECT_RESET_STATE`, so this
//! client reads neither. Which sequence each value plays is a rule about
//! `AnimationData.dbc`; see `vale_assets::look::object::custom_anim`.

use crate::bytes::{Reader, Writer};

/// A one-shot animation the server asked a game object to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectAnim {
    /// `SMSG_GAMEOBJECT_CUSTOM_ANIM` with `anim` 0..3.
    Custom(u8),
    /// `SMSG_GAMEOBJECT_DESPAWN_ANIM`.
    Despawn,
}

/// `SMSG_GAMEOBJECT_CUSTOM_ANIM` (179): `u64 guid, u32 anim`. `None` for a
/// short body; the animation is `None` for an `anim` of 4 or more, which the
/// 1.12.1 client ignores.
pub fn parse_custom_anim(body: &[u8]) -> Option<(u64, Option<ObjectAnim>)> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    let guid = r.u64();
    let anim = r.u32();
    Some((guid, u8::try_from(anim).ok().filter(|anim| *anim < 4).map(ObjectAnim::Custom)))
}

/// `SMSG_GAMEOBJECT_DESPAWN_ANIM` (533): the guid and nothing else.
pub fn parse_despawn_anim(body: &[u8]) -> Option<u64> {
    let mut r = Reader::new(body);
    if !r.has(8) {
        return None;
    }
    Some(r.u64())
}

/// `CMSG_GAMEOBJ_USE`: the guid and nothing else.
///
/// The guid is the full eight bytes, not the packed form.
/// `HandleGameObjectUseOpcode` reads `recv_data >> guid` into an `ObjectGuid`,
/// which is the unpacked form; the packed encoding appears only in movement
/// and update blocks. The short form is a guid the server cannot find, and the
/// click does nothing, with no error, because a lookup miss returns silently.
pub fn use_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eight bytes, little-endian, and no packing.
    #[test]
    fn the_use_body_is_one_plain_guid() {
        assert_eq!(
            use_body(0xF110_0000_0000_1234),
            vec![0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x10, 0xF1]
        );
        assert_eq!(use_body(0).len(), 8, "a zero guid is still eight bytes");
    }

    /// A custom animation is a plain guid and a word; 0..3 play and a larger
    /// value is read and ignored. The despawn packet is the guid alone.
    #[test]
    fn the_two_animation_packets_read_a_guid_and_an_animation() {
        let mut body = 0xF110_0000_0000_1234u64.to_le_bytes().to_vec();
        body.extend_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            parse_custom_anim(&body),
            Some((0xF110_0000_0000_1234, Some(ObjectAnim::Custom(2))))
        );
        body[8] = 4;
        assert_eq!(parse_custom_anim(&body), Some((0xF110_0000_0000_1234, None)));
        assert_eq!(parse_custom_anim(&body[..11]), None);

        assert_eq!(parse_despawn_anim(&body[..8]), Some(0xF110_0000_0000_1234));
        assert_eq!(parse_despawn_anim(&body[..7]), None);
    }
}
