//! Text emotes — `/dance`, `/wave Bob`: the packet each way.
//!
//! ```text
//! CMSG_TEXT_EMOTE   u32 text_emote, u32 emote_num, u64 target guid
//! SMSG_TEXT_EMOTE   u64 sender guid, u32 text_emote, u32 emote_num,
//!                   u32 name length, name bytes (NUL included)
//! ```
//!
//! `text_emote` is the `EmotesText.dbc` id and `emote_num` the `Emotes.dbc`
//! id the client sends beside it; vmangos ignores the second and plays the
//! animation from its own reading of the row (`HandleTextEmoteOpcode`,
//! `em->textid`), echoing it in the broadcast. The broadcast is
//! `EmoteChatBuilder` in `ChatHandler.cpp`: the name is the *target's* — a
//! player's or a creature's — as a counted string, one NUL byte when there
//! is no target, and the sender is a guid the client names for itself.
//!
//! The animation itself arrives separately as `SMSG_EMOTE`, which
//! `play::action` reads; this packet is the sentence and the voice line,
//! composed on the client out of the three tables
//! `assets::tables::emotetext` reads. **Not yet seen from a live server.**

use crate::bytes::{Reader, Writer};

/// One `SMSG_TEXT_EMOTE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEmote {
    pub sender: u64,
    /// The `EmotesText.dbc` id.
    pub text_emote: u32,
    /// The `Emotes.dbc` id the sender put beside it.
    pub emote_num: u32,
    /// The target's name, empty for none.
    pub target: String,
}

/// `SMSG_TEXT_EMOTE` — see the module note. The counted name includes its
/// NUL; a count past the body is refused.
pub fn parse_text_emote(body: &[u8]) -> Option<TextEmote> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4 + 4 + 4) {
        return None;
    }
    let sender = r.u64();
    let text_emote = r.u32();
    let emote_num = r.u32();
    let len = r.u32() as usize;
    if !r.has(len) {
        return None;
    }
    let bytes = r.bytes(len);
    let name = bytes.split(|b| *b == 0).next().unwrap_or(&[]);
    Some(TextEmote {
        sender,
        text_emote,
        emote_num,
        target: String::from_utf8_lossy(name).into_owned(),
    })
}

/// `CMSG_TEXT_EMOTE`.
pub fn text_emote_body(text_emote: u32, emote_num: u32, target: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(text_emote).u32(emote_num).u64(target);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_emote_names_its_target_and_none_is_one_nul() {
        let mut w = Writer::new();
        w.u64(0x1234).u32(34).u32(10).u32(4).bytes(b"Bob\0");
        assert_eq!(
            parse_text_emote(&w.buf),
            Some(TextEmote { sender: 0x1234, text_emote: 34, emote_num: 10, target: "Bob".into() })
        );
        let mut w = Writer::new();
        w.u64(7).u32(34).u32(10).u32(1).u8(0);
        assert_eq!(parse_text_emote(&w.buf).map(|e| e.target), Some(String::new()));
        // A count past the body, and a body short of the head.
        let mut w = Writer::new();
        w.u64(7).u32(34).u32(10).u32(9).bytes(b"Bob\0");
        assert_eq!(parse_text_emote(&w.buf), None);
        assert_eq!(parse_text_emote(&[0; 12]), None);
    }

    #[test]
    fn the_body_is_the_two_ids_and_the_guid() {
        let body = text_emote_body(34, 10, 0x0102);
        assert_eq!(body.len(), 16);
        assert_eq!(&body[..4], &34u32.to_le_bytes());
        assert_eq!(&body[4..8], &10u32.to_le_bytes());
        assert_eq!(&body[8..], &0x0102u64.to_le_bytes());
    }
}
