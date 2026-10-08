//! Four server statements that the client shows as a line of text and that
//! change no state: a whisper to a name nobody is playing, a shutdown or restart
//! countdown, a zone under attack, and a defense message.
//!
//! ```text
//! SMSG_CHAT_PLAYER_NOT_FOUND  cstring name
//! SMSG_SERVER_MESSAGE         u32 type, cstring text
//! SMSG_ZONE_UNDER_ATTACK      u32 area id
//! SMSG_DEFENSE_MESSAGE        u32 zone id, u32 text length + 1, cstring text
//! ```
//!
//! Sources in vmangos: `Server/Packets/Chat.cpp` (`ChatPlayerNotFound`),
//! `Server/Packets/Misc.cpp` (`ServerMessage`, `ZoneUnderAttack`) and
//! `Maps/Map.cpp` (`Map::SendDefenseMessage`).
//!
//! ## When each is sent
//!
//! * `SMSG_CHAT_PLAYER_NOT_FOUND` answers a `CHAT_MSG_WHISPER` whose target
//!   name does not normalise, is offline, or is a GM who refuses whispers
//!   (`WorldSession::SendPlayerNotFoundNotice`). Without it a whisper to a
//!   misspelled name produces no reply at all.
//! * `SMSG_SERVER_MESSAGE` is sent by `World::ShutdownMsg` while a timed
//!   shutdown or restart counts down, and by `World::ShutdownCancel` with an
//!   empty text when one is cancelled. Type 3 is a free text from
//!   `World::SendServerMessage`.
//! * `SMSG_ZONE_UNDER_ATTACK` is sent by `Creature::SendZoneUnderAttackMessage`
//!   when a guard or a PvP-enabling creature is killed by a player. It goes to
//!   every player of the other team on the map, at most once per area every
//!   ten seconds.
//! * `SMSG_DEFENSE_MESSAGE` is sent only by the Eastern Plaguelands towers in
//!   `OutdoorPvP/OutdoorPvPEP.cpp`, to everyone on the map.

use crate::bytes::Reader;

/// `ServerMessageType` (vmangos `World.h`): the `u32` that leads
/// `SMSG_SERVER_MESSAGE`. Each value is a row of `ServerMessages.dbc`, whose
/// text has one `%s` for the packet's own text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMessageType {
    ShutdownTime,
    RestartTime,
    Custom,
    ShutdownCancelled,
    RestartCancelled,
}

impl ServerMessageType {
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            1 => Self::ShutdownTime,
            2 => Self::RestartTime,
            3 => Self::Custom,
            4 => Self::ShutdownCancelled,
            5 => Self::RestartCancelled,
            _ => return None,
        })
    }

    pub fn code(self) -> u32 {
        match self {
            Self::ShutdownTime => 1,
            Self::RestartTime => 2,
            Self::Custom => 3,
            Self::ShutdownCancelled => 4,
            Self::RestartCancelled => 5,
        }
    }
}

/// `SMSG_SERVER_MESSAGE`, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerMessage {
    /// The `ServerMessages.dbc` row, kept raw: a type this enum does not name
    /// is passed on, and the table decides whether it has a row for it.
    pub kind: u32,
    /// The text for the row's `%s`. For a countdown it is vmangos'
    /// `secsToTimeString` of the time left; for a cancellation it is empty.
    pub text: String,
}

impl ServerMessage {
    pub fn message_type(&self) -> Option<ServerMessageType> {
        ServerMessageType::from_code(self.kind)
    }
}

/// `SMSG_DEFENSE_MESSAGE`, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefenseMessage {
    /// An `AreaTable.dbc` id: the zone the message is about.
    pub zone: u32,
    pub text: String,
}

/// `SMSG_CHAT_PLAYER_NOT_FOUND`: the name the whisper was addressed to.
///
/// The name is a string with its terminator; a body with no terminator is still
/// read to its end, since a truncated name is still the best text available.
/// An empty body is refused.
pub fn parse_player_not_found(body: &[u8]) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    Some(Reader::new(body).cstring())
}

/// `SMSG_SERVER_MESSAGE`: a `u32` type, then the text.
pub fn parse_server_message(body: &[u8]) -> Option<ServerMessage> {
    if body.len() < 4 {
        return None;
    }
    let mut r = Reader::new(body);
    let kind = r.u32();
    let text = r.cstring();
    Some(ServerMessage { kind, text })
}

/// `SMSG_ZONE_UNDER_ATTACK`: one `AreaTable.dbc` id.
pub fn parse_zone_under_attack(body: &[u8]) -> Option<u32> {
    if body.len() < 4 {
        return None;
    }
    Some(Reader::new(body).u32())
}

/// `SMSG_DEFENSE_MESSAGE`: the zone, a length that counts the terminator, and
/// the text.
///
/// The text is read as a terminated string rather than by the length, which
/// gives the same result for every body vmangos writes. A length that runs past
/// the body is refused, because it says the body was cut short.
pub fn parse_defense_message(body: &[u8]) -> Option<DefenseMessage> {
    if body.len() < 8 {
        return None;
    }
    let mut r = Reader::new(body);
    let zone = r.u32();
    let length = r.u32() as usize;
    if length > r.remaining() {
        return None;
    }
    let text = r.cstring();
    Some(DefenseMessage { zone, text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Writer;

    #[test]
    fn the_whisper_refusal_is_the_name_alone() {
        assert_eq!(parse_player_not_found(b"Dolgrin\0").as_deref(), Some("Dolgrin"));
        assert_eq!(parse_player_not_found(b"Dolg").as_deref(), Some("Dolg"));
        assert_eq!(parse_player_not_found(&[]), None);
    }

    /// The type comes first: read the other way round, a countdown's text would
    /// be taken as a type.
    #[test]
    fn a_server_message_is_its_type_then_its_text() {
        let mut w = Writer::new();
        w.u32(1).cstring("15 Minute(s)");
        let message = parse_server_message(&w.buf).unwrap();
        assert_eq!(message.message_type(), Some(ServerMessageType::ShutdownTime));
        assert_eq!(message.text, "15 Minute(s)");

        let mut w = Writer::new();
        w.u32(5).cstring("");
        let cancelled = parse_server_message(&w.buf).unwrap();
        assert_eq!(cancelled.message_type(), Some(ServerMessageType::RestartCancelled));
        assert_eq!(cancelled.text, "");

        assert_eq!(parse_server_message(&[1, 0, 0]), None);
        assert_eq!(ServerMessageType::from_code(6), None);
        for code in 1..=5 {
            assert_eq!(ServerMessageType::from_code(code).map(ServerMessageType::code), Some(code));
        }
    }

    #[test]
    fn a_zone_under_attack_is_one_area_id() {
        assert_eq!(parse_zone_under_attack(&87u32.to_le_bytes()), Some(87));
        assert_eq!(parse_zone_under_attack(&[87, 0]), None);
    }

    #[test]
    fn a_defense_message_counts_its_terminator() {
        let text = "Crown Guard Tower has been taken by the Horde!";
        let mut w = Writer::new();
        w.u32(139).u32(text.len() as u32 + 1).cstring(text);
        assert_eq!(
            parse_defense_message(&w.buf),
            Some(DefenseMessage { zone: 139, text: text.to_string() })
        );
        // A length longer than what follows is a cut body.
        let mut w = Writer::new();
        w.u32(139).u32(40).cstring("short");
        assert_eq!(parse_defense_message(&w.buf), None);
    }
}
