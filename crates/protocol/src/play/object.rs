//! **Using something in the world that is not a person** — the door, the chest,
//! the ore vein, the mailbox, the lever.
//!
//! One packet out and nothing back that is about *this* packet:
//!
//! ```text
//! CMSG_GAMEOBJ_USE  u64 guid   right-click it
//! ```
//!
//! ## The reply is the world changing, not an answer
//!
//! There is no `SMSG_GAMEOBJ_USE_RESPONSE` and nothing to wait for. What a use
//! does arrives through whichever subsystem the *server* decided it belongs to,
//! and that is the whole reason this file is four lines long:
//!
//! * **a door or a lever** changes `GAMEOBJECT_STATE` in an update block, and
//!   the model swaps between its `Opened` and `Closed` sequences —
//!   `crates/client/src/world/entities/pose.rs` already does that, and it is why
//!   nothing here has to know a door from a chest;
//! * **a chest** answers with `SMSG_LOOT_RESPONSE`, exactly as a corpse does;
//! * **an ore vein or a herb** is a chest whose lock names a *skill*, so the
//!   server casts the gathering spell at us and the answer is
//!   `SMSG_SPELL_START` and then a loot window;
//! * **a quest goober** answers with `SMSG_QUESTGIVER_QUEST_DETAILS`;
//! * **a refusal** is a message off the client's own table — `ERR_DOOR_LOCKED`,
//!   `ERR_USE_LOCKED_WITH_SPELL_S` — which reaches the error frame like every
//!   other one.
//!
//! So this client sends the ask and lets the existing paths carry whatever comes
//! of it. **Nothing is predicted**, on the same terms as every other verb here:
//! a door that will not open must not swing shut on the client and then snap
//! back a round trip later.
//!
//! ## What may be used at all is *not* decided here
//!
//! Which of the thirty `GAMEOBJECT_TYPE_*` values react to a click, and which
//! pointer belongs over one, is a rule about the template rather than about the
//! wire — see `vale_assets::look::object`, which owns it and can be
//! unit-tested with no socket. This module only knows how to say it.

use crate::bytes::Writer;

/// `CMSG_GAMEOBJ_USE` — the guid and nothing else.
///
/// **A full eight-byte guid, not a packed one.** `HandleGameObjectUseOpcode`
/// reads `recv_data >> guid` into an `ObjectGuid`, which is the unpacked form;
/// the packed encoding only ever appears in movement and update blocks. Sending
/// the short form here is a guid the server cannot find and a click that does
/// nothing at all — with no error, because a lookup miss is a silent return.
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
}
