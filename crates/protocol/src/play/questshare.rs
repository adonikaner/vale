//! Sharing a quest with the group: two requests out, two answers in, and the
//! ordinary quest page in between.
//!
//! ```text
//! CMSG_PUSHQUESTTOPARTY      u32 quest            "Share Quest" in the quest log
//!   -> MSG_QUEST_PUSH_RESULT u64 member, u8 result     to the sharer, per member
//!   -> SMSG_QUESTGIVER_QUEST_DETAILS (giver = the sharer)   to each member who may take it
//! MSG_QUEST_PUSH_RESULT      u64 sharer, u8 result      a member's decline, relayed
//!
//! SMSG_QUEST_CONFIRM_ACCEPT  u32 quest, cstring title, u64 accepter
//! CMSG_QUEST_CONFIRM_ACCEPT  u32 quest                 "Yes" on the QUEST_ACCEPT popup
//! ```
//!
//! Sources in vmangos: `Server/Packets/Quest.cpp` for every layout,
//! `Handlers/QuestHandler.cpp` (`HandlePushQuestToParty`,
//! `HandleQuestPushResult`, `HandleQuestConfirmAccept`,
//! `HandleQuestgiverAcceptQuestOpcode`) and `Player::SendQuestConfirmAccept`.
//!
//! ## Pushing a quest
//!
//! `HandlePushQuestToParty` sends the sharer result 0 (sharing) for every other
//! member, then a second result for each member who may not take the quest:
//! too far away (more than 14 yards, `QUEST_SHARE_DISTANCE`), already done, on
//! the quest already, not eligible, quest log full, or busy with another share.
//! A member who may take it is sent the quest's detail page with the sharer's
//! player guid as the giver, which opens `QuestFrame` the same way an NPC does.
//! When that member accepts, the sharer is sent result 2; when the member closes
//! the page, the member's client sends `MSG_QUEST_PUSH_RESULT` with result 3 and
//! the sharer's guid, and the server relays it to the sharer. Until the server
//! sees one of those two, it keeps the member's share pending and answers later
//! shares to that member with result 5 (busy).
//!
//! ## Accepting a party quest
//!
//! A quest with `QUEST_FLAGS_PARTY_ACCEPT` (0x2) is offered to the rest of the
//! group when one member accepts it. Every other member on the same map who
//! may take it is sent `SMSG_QUEST_CONFIRM_ACCEPT` naming the accepting player,
//! and answers with `CMSG_QUEST_CONFIRM_ACCEPT`. There is no reply beyond the
//! quest log's own update fields. Declining sends nothing.

use crate::bytes::{Reader, Writer};

/// `QuestShareMessages` (vmangos `QuestDef.h`): the result byte of
/// `MSG_QUEST_PUSH_RESULT`. Each value has one `GlobalStrings.lua` key, which
/// takes the other player's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushResult {
    Sharing,
    CantTake,
    Accepted,
    Declined,
    TooFar,
    Busy,
    LogFull,
    OnQuest,
    AlreadyDone,
}

impl PushResult {
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Sharing,
            1 => Self::CantTake,
            2 => Self::Accepted,
            3 => Self::Declined,
            4 => Self::TooFar,
            5 => Self::Busy,
            6 => Self::LogFull,
            7 => Self::OnQuest,
            8 => Self::AlreadyDone,
            _ => return None,
        })
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Sharing => 0,
            Self::CantTake => 1,
            Self::Accepted => 2,
            Self::Declined => 3,
            Self::TooFar => 4,
            Self::Busy => 5,
            Self::LogFull => 6,
            Self::OnQuest => 7,
            Self::AlreadyDone => 8,
        }
    }

    /// The `GlobalStrings.lua` key the client shows for this result, with the
    /// other player's name as its `%s`. vmangos' `QuestDef.h` names the same
    /// key beside each value.
    pub fn key(self) -> &'static str {
        match self {
            Self::Sharing => "ERR_QUEST_PUSH_SUCCESS_S",
            Self::CantTake => "ERR_QUEST_PUSH_INVALID_S",
            Self::Accepted => "ERR_QUEST_PUSH_ACCEPTED_S",
            Self::Declined => "ERR_QUEST_PUSH_DECLINED_S",
            Self::TooFar => "ERR_QUEST_PUSH_TOO_FAR_S",
            Self::Busy => "ERR_QUEST_PUSH_BUSY_S",
            Self::LogFull => "ERR_QUEST_PUSH_LOG_FULL_S",
            Self::OnQuest => "ERR_QUEST_PUSH_ONQUEST_S",
            Self::AlreadyDone => "ERR_QUEST_PUSH_ALREADY_DONE_S",
        }
    }
}

/// `MSG_QUEST_PUSH_RESULT` from the server, read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushOutcome {
    /// The other player: the member the result is about.
    pub member: u64,
    /// The raw result byte. [`PushResult::from_code`] names it; a value past
    /// the enum has no key and shows nothing.
    pub result: u8,
}

/// `SMSG_QUEST_CONFIRM_ACCEPT`, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmAccept {
    pub quest_id: u32,
    pub title: String,
    /// The member who accepted the quest.
    pub accepter: u64,
}

/// `MSG_QUEST_PUSH_RESULT` from the server: a plain guid and one byte.
pub fn parse_push_result(body: &[u8]) -> Option<PushOutcome> {
    if body.len() < 9 {
        return None;
    }
    let mut r = Reader::new(body);
    Some(PushOutcome { member: r.u64(), result: r.u8() })
}

/// `SMSG_QUEST_CONFIRM_ACCEPT`: the quest, its title, and who accepted it.
pub fn parse_confirm_accept(body: &[u8]) -> Option<ConfirmAccept> {
    // The id, at least the title's terminator, and the guid.
    if body.len() < 4 + 1 + 8 {
        return None;
    }
    let mut r = Reader::new(body);
    let quest_id = r.u32();
    let title = r.cstring();
    if !r.has(8) {
        return None;
    }
    Some(ConfirmAccept { quest_id, title, accepter: r.u64() })
}

/// `CMSG_PUSHQUESTTOPARTY`: the quest to share.
pub fn push_quest_body(quest_id: u32) -> Vec<u8> {
    quest_id.to_le_bytes().to_vec()
}

/// `CMSG_QUEST_CONFIRM_ACCEPT`: the quest the popup offered.
pub fn confirm_accept_body(quest_id: u32) -> Vec<u8> {
    quest_id.to_le_bytes().to_vec()
}

/// `MSG_QUEST_PUSH_RESULT` from the client: the sharer's guid and a result.
/// The 1.12.1 client sends it with [`PushResult::Declined`] when a quest page
/// whose giver is a player is closed without accepting. vmangos reads the guid
/// and ignores it, using the share it has stored for this player.
pub fn push_result_body(sharer: u64, result: PushResult) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(sharer).u8(result.code());
    w.buf
}

/// Whether a guid is a player's: its top sixteen bits are vmangos'
/// `HIGHGUID_PLAYER`, which is 0 (`ObjectGuid.h`). A quest page whose giver is
/// a player is a shared quest, and closing it is answered with
/// [`PushResult::Declined`].
pub fn is_player_guid(guid: u64) -> bool {
    guid != 0 && guid >> 48 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_guid_has_no_high_part() {
        assert!(is_player_guid(17));
        assert!(!is_player_guid(0));
        assert!(!is_player_guid(0xF130_0000_0001_2345));
        assert!(!is_player_guid(0x4000_0000_0000_0011));
    }

    #[test]
    fn every_result_names_its_own_key() {
        for code in 0..=8u8 {
            let result = PushResult::from_code(code).unwrap();
            assert_eq!(result.code(), code);
            assert!(result.key().starts_with("ERR_QUEST_PUSH_"));
        }
        assert_eq!(PushResult::from_code(9), None);
        assert_eq!(PushResult::Declined.key(), "ERR_QUEST_PUSH_DECLINED_S");
    }

    #[test]
    fn a_push_result_is_a_plain_guid_and_a_byte() {
        let mut w = Writer::new();
        w.u64(0x0000_0000_0000_0011).u8(4);
        assert_eq!(parse_push_result(&w.buf), Some(PushOutcome { member: 17, result: 4 }));
        assert_eq!(parse_push_result(&w.buf[..8]), None);
        assert_eq!(push_result_body(17, PushResult::TooFar), w.buf);
    }

    /// The title is variable-length and the guid follows it, so a reader that
    /// took the guid from a fixed offset would read part of the title.
    #[test]
    fn the_confirm_offer_puts_the_guid_after_the_title() {
        let mut w = Writer::new();
        w.u32(5301).cstring("Lord Valthalak").u64(0x0000_0000_0000_0021);
        assert_eq!(
            parse_confirm_accept(&w.buf),
            Some(ConfirmAccept { quest_id: 5301, title: "Lord Valthalak".to_string(), accepter: 33 })
        );
        assert_eq!(parse_confirm_accept(&w.buf[..w.buf.len() - 1]), None);
        assert_eq!(confirm_accept_body(5301), 5301u32.to_le_bytes().to_vec());
        assert_eq!(push_quest_body(5301), 5301u32.to_le_bytes().to_vec());
    }
}
