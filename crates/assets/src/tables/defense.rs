//! Which of the character's channels hear that a zone is under attack.
//!
//! `SMSG_ZONE_UNDER_ATTACK` and `SMSG_DEFENSE_MESSAGE` name a zone and carry
//! no channel. The 1.12.1 client puts each line into the joined channels whose
//! `ChatChannels.dbc` row carries [`flags::DEFENSE`]:
//!
//! * WorldDefense, which has no [`flags::ZONE_DEP`], always hears it;
//! * LocalDefense, which has both flags, hears it only when the zone the line
//!   names is the zone the character's zone channels were joined for.
//!
//! The zone a line names is the packet's `AreaTable.dbc` id, or that row's
//! parent when the row has one; the client looks up one level, not the whole
//! chain. A character in no defense channel sees nothing: there is no other
//! surface for either packet, and no event is raised for them.
//!
//! `SMSG_ZONE_UNDER_ATTACK`'s text is `ZONE_UNDER_ATTACK` from
//! `GlobalStrings.lua` with the zone's name; `SMSG_DEFENSE_MESSAGE`'s is the
//! packet's own. Both arrive as `CHAT_MSG_CHANNEL` lines with no sender.

use crate::tables::area::Areas;
use crate::tables::channels::{flags, ChatChannels};

/// The zone a defense line is about: the area's parent when it has one, and
/// the area itself otherwise. `None` for an id the table does not carry, which
/// the 1.12.1 client ignores for `SMSG_ZONE_UNDER_ATTACK`.
pub fn zone_of_line(areas: &Areas, area: u32) -> Option<u32> {
    let row = areas.get(area)?;
    Some(if row.parent != 0 { row.parent } else { row.id })
}

/// Whether a joined channel, identified by its `ChatChannels.dbc` row, hears
/// a line about `line_zone`. `joined_zone` is the zone the character's zone
/// channels were joined for.
pub fn hears(channels: &ChatChannels, row: u32, line_zone: u32, joined_zone: Option<u32>) -> bool {
    let Some(channel) = channels.get(row) else {
        return false;
    };
    if !channel.has(flags::DEFENSE) {
        return false;
    }
    !channel.has(flags::ZONE_DEP) || joined_zone == Some(line_zone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::channels::ChatChannel;

    fn channels() -> ChatChannels {
        ChatChannels::from_rows(vec![
            ChatChannel { id: 1, flags: 0x3, pattern: "General - %s".into(), shortcut: "General".into() },
            ChatChannel { id: 22, flags: 0x10003, pattern: "LocalDefense - %s".into(), shortcut: "LocalDefense".into() },
            ChatChannel { id: 23, flags: 0x10004, pattern: "WorldDefense".into(), shortcut: "WorldDefense".into() },
        ])
    }

    #[test]
    fn local_defense_needs_the_zone_and_world_defense_does_not() {
        let channels = channels();
        // Elwynn Forest is 12; the line is about Elwynn.
        assert!(hears(&channels, 22, 12, Some(12)));
        assert!(!hears(&channels, 22, 12, Some(40)));
        assert!(!hears(&channels, 22, 12, None));
        assert!(hears(&channels, 23, 12, Some(40)));
        assert!(hears(&channels, 23, 12, None));
        // General and a custom channel never hear it.
        assert!(!hears(&channels, 1, 12, Some(12)));
        assert!(!hears(&channels, 0, 12, Some(12)));
    }
}
