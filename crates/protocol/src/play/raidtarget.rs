//! Raid target icons: eight marks the group leader or an assistant places on
//! units, one opcode in both directions with two server forms.
//!
//! ```text
//! MSG_RAID_TARGET_UPDATE  client -> server   u8 0xFF                 ask for the list
//! MSG_RAID_TARGET_UPDATE  client -> server   u8 icon, u64 guid       place, or clear with guid 0
//! MSG_RAID_TARGET_UPDATE  server -> client   u8 0, u8 icon, u64 guid one change
//! MSG_RAID_TARGET_UPDATE  server -> client   u8 1, (u8 icon, u64 guid)*   the whole list
//! ```
//!
//! Sources in vmangos: `Server/Packets/Group.cpp` (`RaidTargetUpdate`,
//! `RaidTargetUpdateDelta`, `RaidTargetUpdateAll`),
//! `WorldSession::HandleRaidTargetUpdateOpcode` and `Group::SetTargetIcon`.
//!
//! ## What the server keeps
//!
//! A group holds eight guids, one per icon (`TARGET_ICON_COUNT`). An icon id of
//! 8 or more is ignored. Placing an icon on a guid first clears every other icon
//! that guid holds, each with its own change packet, so a unit carries at most
//! one icon. Only the leader or an assistant may place one; anyone in the group
//! may ask for the list.
//!
//! The list has no count: entries run to the end of the body, and only the
//! icons that are set are listed. `Group::SendUpdate` sends the list after
//! every `SMSG_GROUP_LIST` to a party, not to a raid; a raid member receives it
//! only by asking.
//!
//! ## Numbering
//!
//! The wire numbers the icons 0 to 7. The interface numbers them 1 to 8
//! (`GetRaidTargetIndex`, `SetRaidTarget`, and the `RAID_TARGET_1`..`8` strings),
//! and 0 there means no icon.

use crate::bytes::{Reader, Writer};

/// How many icons a group has (vmangos `TARGET_ICON_COUNT`).
pub const ICON_COUNT: u8 = 8;

/// The icon byte that turns a client packet into a request for the list.
pub const LIST_REQUEST: u8 = 0xFF;

/// `MSG_RAID_TARGET_UPDATE` from the server, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RaidTargetUpdate {
    /// One icon changed. A guid of 0 clears the icon.
    Change { icon: u8, guid: u64 },
    /// Every icon that is set, replacing what the client held.
    List(Vec<(u8, u64)>),
}

/// `MSG_RAID_TARGET_UPDATE` from the server: a form byte, then one change or
/// the list.
///
/// A list whose length is not a whole number of nine-byte entries is refused,
/// since the remainder says the body was cut. A form byte other than 0 or 1 is
/// refused for the same reason.
pub fn parse_raid_target_update(body: &[u8]) -> Option<RaidTargetUpdate> {
    let (&form, rest) = body.split_first()?;
    let mut r = Reader::new(rest);
    match form {
        0 => {
            if rest.len() < 9 {
                return None;
            }
            Some(RaidTargetUpdate::Change { icon: r.u8(), guid: r.u64() })
        }
        1 => {
            if rest.len() % 9 != 0 {
                return None;
            }
            let mut list = Vec::with_capacity(rest.len() / 9);
            while r.has(9) {
                list.push((r.u8(), r.u64()));
            }
            Some(RaidTargetUpdate::List(list))
        }
        _ => None,
    }
}

/// `MSG_RAID_TARGET_UPDATE` to the server: place `icon` (0..8) on `guid`, or
/// clear it with a guid of 0.
pub fn set_icon_body(icon: u8, guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(icon).u64(guid);
    w.buf
}

/// `MSG_RAID_TARGET_UPDATE` to the server: ask for the whole list.
pub fn list_request_body() -> Vec<u8> {
    vec![LIST_REQUEST]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_form_zero_then_icon_then_guid() {
        let mut w = Writer::new();
        w.u8(0).u8(7).u64(0xF130_0000_0000_1234);
        assert_eq!(
            parse_raid_target_update(&w.buf),
            Some(RaidTargetUpdate::Change { icon: 7, guid: 0xF130_0000_0000_1234 })
        );
        assert_eq!(parse_raid_target_update(&w.buf[..9]), None);
    }

    /// The list has no count, so its length is the only check on it.
    #[test]
    fn a_list_runs_to_the_end_of_the_body() {
        let mut w = Writer::new();
        w.u8(1).u8(0).u64(5).u8(3).u64(9);
        assert_eq!(
            parse_raid_target_update(&w.buf),
            Some(RaidTargetUpdate::List(vec![(0, 5), (3, 9)]))
        );
        assert_eq!(parse_raid_target_update(&[1]), Some(RaidTargetUpdate::List(Vec::new())));
        assert_eq!(parse_raid_target_update(&w.buf[..w.buf.len() - 1]), None);
        assert_eq!(parse_raid_target_update(&[2, 0]), None);
        assert_eq!(parse_raid_target_update(&[]), None);
    }

    #[test]
    fn the_client_sends_a_guid_only_with_an_icon() {
        assert_eq!(set_icon_body(2, 9), [2, 9, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(list_request_body(), [0xFF]);
    }
}
