//! Chat: `SMSG_MESSAGECHAT` in, `CMSG_MESSAGECHAT` out.
//!
//! Sources: `game/Handlers/ChatHandler.cpp` (the inbound half and every check it
//! applies), `ChatHandler::BuildChatPacket` in `game/Chat/Chat.cpp` (the wire
//! layout), and the `ChatMsg` / `Language` enums in `game/SharedDefines.h`.
//!
//! Chat matters here for a reason beyond talking to people: **a GM command is a
//! chat message**. `ProcessChatMessageAfterSecurityCheck` hands the text to
//! `ChatHandler::ParseCommands` before it reaches anyone else, so `.tele
//! tanaris` typed into a say is how this client asks to be somewhere else — and
//! being somewhere else is what testing terrain, dressing and animation needs.
//! That is the whole reason it is worth doing before a chat *frame* exists.
//!
//! ## The two things that make a message vanish
//!
//! Both are checked before the text is looked at, and both fail **silently from
//! the client's point of view** — the server logs a line and returns.
//!
//! * **`LANG_UNIVERSAL` is not allowed for a say.**
//!   `IsLanguageAllowedForChatType` permits language 0 for `CHAT_MSG_AFK` and
//!   `CHAT_MSG_DND` and *nothing else*, so the obvious "no particular language"
//!   value is exactly the one that cannot be used. A message must carry a real
//!   racial language.
//! * **…and one the character actually knows.** `KnowsLanguage` is a skill
//!   check, so sending Common as a tauren earns a `SMSG_NOTIFICATION` and
//!   nothing else. [`Language::for_race`] is the mapping, which is a client-side
//!   rule: the server states the race and never states the language.
//!
//! Between them these two mean a client that sends `(CHAT_MSG_SAY,
//! LANG_UNIVERSAL)` — the reading anyone would try first — never gets a word out
//! and sees no error at all.
//!
//! **And a third, which is not a bug and cost an hour anyway: a dead player
//! cannot speak.** The `CHAT_MSG_SAY` arm of the handler's second switch is
//! `if (!GetPlayer()->IsAlive()) return;` — no notification, no log line, no
//! reply. A GM command from the same corpse still works, because
//! `ParseCommands` runs in the *first* switch and returns before the alive check
//! is ever reached. So the observable behaviour of a client testing this on a
//! character who happens to be a ghost is that `.tele` is answered and `say` is
//! ignored, which reads exactly like an outbound bug in the say path. It was
//! settled by the server's own `Chat.log` staying empty and a `corpse` row for
//! that character, and then by the same message going out fine from a living one.

use crate::bytes::{Reader, Writer};

/// `ChatMsg` (`game/SharedDefines.h`), the ones this client names.
///
/// The wire carries a `u8` and the table has ninety-odd entries, most of them
/// combat-log rows a 1.12 client renders in its own colours. So this is the
/// named subset and [`ChatMessage::kind`] keeps the raw byte — the same rule
/// [`crate::socket::world::Packet`] follows for an opcode, and for the same reason: an
/// unrecognised value is data, not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatType {
    Say,
    Party,
    Raid,
    Guild,
    Officer,
    Yell,
    Whisper,
    /// The echo of a whisper *we* sent, so it can be shown as "To X:".
    WhisperInform,
    Emote,
    TextEmote,
    /// Server output, and where a GM command's reply arrives.
    System,
    MonsterSay,
    MonsterYell,
    MonsterEmote,
    MonsterWhisper,
    Channel,
    ChannelJoin,
    ChannelLeave,
    ChannelList,
    ChannelNotice,
    ChannelNoticeUser,
    Afk,
    Dnd,
    Ignored,
    Skill,
    Loot,
}

impl ChatType {
    pub fn code(self) -> u8 {
        use ChatType::*;
        match self {
            Say => 0x00,
            Party => 0x01,
            Raid => 0x02,
            Guild => 0x03,
            Officer => 0x04,
            Yell => 0x05,
            Whisper => 0x06,
            WhisperInform => 0x07,
            Emote => 0x08,
            TextEmote => 0x09,
            System => 0x0A,
            MonsterSay => 0x0B,
            MonsterYell => 0x0C,
            MonsterEmote => 0x0D,
            Channel => 0x0E,
            ChannelJoin => 0x0F,
            ChannelLeave => 0x10,
            ChannelList => 0x11,
            ChannelNotice => 0x12,
            ChannelNoticeUser => 0x13,
            Afk => 0x14,
            Dnd => 0x15,
            Ignored => 0x16,
            Skill => 0x17,
            Loot => 0x18,
            MonsterWhisper => 0x1A,
        }
    }

    pub fn from_code(code: u8) -> Option<ChatType> {
        use ChatType::*;
        Some(match code {
            0x00 => Say,
            0x01 => Party,
            0x02 => Raid,
            0x03 => Guild,
            0x04 => Officer,
            0x05 => Yell,
            0x06 => Whisper,
            0x07 => WhisperInform,
            0x08 => Emote,
            0x09 => TextEmote,
            0x0A => System,
            0x0B => MonsterSay,
            0x0C => MonsterYell,
            0x0D => MonsterEmote,
            0x0E => Channel,
            0x0F => ChannelJoin,
            0x10 => ChannelLeave,
            0x11 => ChannelList,
            0x12 => ChannelNotice,
            0x13 => ChannelNoticeUser,
            0x14 => Afk,
            0x15 => Dnd,
            0x16 => Ignored,
            0x17 => Skill,
            0x18 => Loot,
            0x1A => MonsterWhisper,
            _ => return None,
        })
    }

    /// Whether sending this needs a target name in front of the text.
    ///
    /// `HandleMessagechatOpcode` reads a name for a whisper and a channel name
    /// for a channel, and reads the message *immediately* for everything else —
    /// so an extra string here consumes the message and an absent one makes the
    /// message the target.
    pub fn needs_target(self) -> bool {
        matches!(self, ChatType::Whisper | ChatType::Channel)
    }

    /// A label for a line of this kind. The 1.12 client's own wording, which is
    /// what makes a log readable without colours.
    pub fn prefix(self) -> &'static str {
        use ChatType::*;
        match self {
            Say | MonsterSay => "says",
            Yell | MonsterYell => "yells",
            Emote | MonsterEmote | TextEmote => "",
            Whisper | MonsterWhisper => "whispers",
            WhisperInform => "to",
            Party => "[Party]",
            Raid => "[Raid]",
            Guild => "[Guild]",
            Officer => "[Officer]",
            Afk => "[AFK]",
            Dnd => "[DND]",
            _ => "",
        }
    }
}

/// `Language` (`game/SharedDefines.h`). Only the racial ones plus the two
/// specials, because those are the only ones a client picks between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// **Not usable for ordinary chat** — see the module comment.
    Universal = 0,
    Orcish = 1,
    Darnassian = 2,
    Taurahe = 3,
    Dwarvish = 6,
    Common = 7,
    Gutterspeak = 33,
    Troll = 14,
    Gnomish = 13,
}

impl Language {
    /// The language a character of this race speaks by default.
    ///
    /// **A client-side rule with nothing on the wire to check it against**: the
    /// server sends the race and never the language, and it *rejects* a language
    /// the character has no skill in. Every Alliance race knows Common and every
    /// Horde race knows Orcish — which is why the racial tongues above are not
    /// used here — so this is the pair of values that always works, and it is
    /// what the real client's default chat language resolves to.
    ///
    /// `ChrRaces` ids: 1 human, 2 orc, 3 dwarf, 4 night elf, 5 undead, 6 tauren,
    /// 7 gnome, 8 troll. An unknown race is treated as Alliance rather than
    /// refused, because a message in the wrong language is recoverable and no
    /// message at all is not.
    pub fn for_race(race: u8) -> Language {
        match race {
            2 | 5 | 6 | 8 => Language::Orcish,
            _ => Language::Common,
        }
    }

    pub fn code(self) -> u32 {
        self as u32
    }
}

/// One line of chat, as it arrived.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    /// The raw `ChatMsg` byte, kept whether or not [`ChatType`] names it.
    pub kind: u8,
    pub language: u32,
    /// Who said it. Zero for server output — `CHAT_MSG_SYSTEM` is built with a
    /// default `ObjectGuid`.
    pub sender: u64,
    /// The sender's name, when the packet carries one.
    ///
    /// **Only the monster types do.** A player's line carries a GUID and nothing
    /// else, on the assumption that the client has the name already — which is
    /// true of anyone in view and false of a guild member across the continent,
    /// so a name here is an answer and its absence is a lookup.
    pub sender_name: Option<String>,
    /// Who it was aimed at, for the monster types. Rarely useful; kept because
    /// it has to be consumed to reach the text.
    pub target: u64,
    pub channel: Option<String>,
    pub text: String,
    /// `CHAT_TAG_*`: 0 none, 1 AFK, 2 DND, 4 GM.
    pub tag: u8,
}

impl ChatMessage {
    pub fn chat_type(&self) -> Option<ChatType> {
        ChatType::from_code(self.kind)
    }
}

/// Parse `SMSG_MESSAGECHAT`.
///
/// `ChatHandler::BuildChatPacket` is the layout, and it is **type-dependent with
/// no length prefix** — so the type byte decides how many bytes to skip before
/// the text, and getting it wrong reads the message length out of the middle of
/// a GUID. Three shapes beyond the default one:
///
/// * **`SAY`, `PARTY` and `YELL` write the sender's GUID *twice*.** Not a typo in
///   the server and not a target: `data << ObjectGuid(senderGuid)` appears on two
///   consecutive lines. Reading it once leaves eight bytes of GUID in front of
///   the length, which comes out as a message hundreds of megabytes long — so
///   this one at least fails loudly rather than plausibly.
/// * **the monster types carry a name inline**, as a `u32` length and then the
///   string, followed by the target's GUID. That is how a creature can talk
///   without the client having ever queried it.
/// * **a channel line leads with the channel name** and a rank, and *then* the
///   GUID.
///
/// The text is `u32 length` then a NUL-terminated string, where the length
/// **includes the NUL** (`strlen(message) + 1`).
pub fn parse_message(body: &[u8]) -> Option<ChatMessage> {
    let mut r = Reader::new(body);
    if !r.has(5) {
        return None;
    }
    let kind = r.u8();
    let language = r.u32();

    let mut sender = 0;
    let mut sender_name = None;
    let mut target = 0;
    let mut channel = None;

    // Matched on the raw byte, because the layout has to be decided even for a
    // type this client does not name — and the default branch is one GUID, which
    // is what every combat-log row is.
    match ChatType::from_code(kind) {
        Some(ChatType::MonsterWhisper | ChatType::MonsterEmote) => {
            sender_name = Some(read_counted_string(&mut r)?);
            target = read_u64(&mut r)?;
        }
        Some(ChatType::Say | ChatType::Party | ChatType::Yell) => {
            sender = read_u64(&mut r)?;
            // The same GUID again. See the doc comment.
            let _ = read_u64(&mut r)?;
        }
        Some(ChatType::MonsterSay | ChatType::MonsterYell) => {
            sender = read_u64(&mut r)?;
            sender_name = Some(read_counted_string(&mut r)?);
            target = read_u64(&mut r)?;
        }
        Some(ChatType::Channel) => {
            channel = Some(r.cstring());
            let _rank = read_u32(&mut r)?;
            sender = read_u64(&mut r)?;
        }
        _ => sender = read_u64(&mut r)?,
    }

    let text = read_counted_string(&mut r)?;
    // The tag is the last byte and some server paths are terse; a missing one is
    // not worth discarding a message for.
    let tag = if r.has(1) { r.u8() } else { 0 };

    Some(ChatMessage {
        kind,
        language,
        sender,
        sender_name,
        target,
        channel,
        text,
        tag,
    })
}

/// `SMSG_NOTIFICATION` — a bare NUL-terminated string and nothing else.
///
/// Worth handling rather than counting as unhandled, because it is how the
/// server explains a rejection: "you have not learned that language", the
/// anticheat's own warnings, and `SendNotification`'s output generally. Dropped,
/// those failures are indistinguishable from the packet never having been sent.
pub fn parse_notification(body: &[u8]) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    Some(Reader::new(body).cstring())
}

/// `CMSG_MESSAGECHAT`: `u32 type`, `u32 language`, an optional target name, then
/// the text.
///
/// The target is the whisper's recipient or the channel's name, and it is
/// present for exactly the types [`ChatType::needs_target`] names — the server
/// reads the message straight after the language for everything else.
pub fn message_body(kind: ChatType, language: Language, target: Option<&str>, text: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(u32::from(kind.code()));
    w.u32(language.code());
    if kind.needs_target() {
        // Empty rather than absent when the caller forgot: the server reads a
        // string here unconditionally, so leaving it out would make the *text*
        // the target and send an empty message.
        w.cstring(target.unwrap_or(""));
    }
    w.cstring(text);
    w.buf
}

/// A `u32` count and then that many bytes, the last of which is a NUL.
///
/// The count is `strlen + 1`, so it is trusted only as far as the buffer goes —
/// these are 20-year-old formats read from a live socket, and a length that
/// overran used to be the one thing that could panic a parser.
fn read_counted_string(r: &mut Reader) -> Option<String> {
    let len = read_u32(r)? as usize;
    if len == 0 || !r.has(len) {
        return None;
    }
    let bytes = r.bytes(len);
    let text = bytes.split(|b| *b == 0).next().unwrap_or(&[]);
    Some(String::from_utf8_lossy(text).into_owned())
}

fn read_u32(r: &mut Reader) -> Option<u32> {
    r.has(4).then(|| r.u32())
}

fn read_u64(r: &mut Reader) -> Option<u64> {
    r.has(8).then(|| r.u64())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn say(guid: u64, text: &str) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(ChatType::Say.code());
        w.u32(Language::Common.code());
        w.u64(guid);
        w.u64(guid); // BuildChatPacket writes it twice
        w.u32(text.len() as u32 + 1);
        w.cstring(text);
        w.u8(0);
        w.buf
    }

    /// **A say carries the sender's GUID twice, and this is the assertion that
    /// says so.** `BuildChatPacket` writes `data << ObjectGuid(senderGuid)` on
    /// two consecutive lines for `SAY`, `PARTY` and `YELL`; reading one leaves
    /// eight bytes in front of the length.
    #[test]
    fn a_say_repeats_the_senders_guid() {
        let msg = parse_message(&say(0x1234, "hello")).expect("a say");
        assert_eq!(msg.chat_type(), Some(ChatType::Say));
        assert_eq!(msg.sender, 0x1234);
        assert_eq!(msg.text, "hello");
        assert_eq!(msg.language, Language::Common.code());

        // One GUID short: the length is then read out of the second GUID and is
        // nonsense, which the bounds check turns into a refusal rather than a
        // panic or a 4 GB string.
        let mut short = say(0x1234, "hello");
        short.drain(5..13);
        assert!(parse_message(&short).is_none());
    }

    /// A creature's line carries its **name** inline, which is what lets it talk
    /// without the client having queried it.
    #[test]
    fn a_creature_says_who_it_is() {
        let mut w = Writer::new();
        w.u8(ChatType::MonsterSay.code());
        w.u32(Language::Universal.code());
        w.u64(0xAA);
        w.u32(4);
        w.cstring("Rex");
        w.u64(0xBB); // the target it is talking to
        w.u32(6);
        w.cstring("grrrr");
        w.u8(0);

        let msg = parse_message(&w.buf).expect("a monster say");
        assert_eq!(msg.sender_name.as_deref(), Some("Rex"));
        assert_eq!(msg.sender, 0xAA);
        assert_eq!(msg.target, 0xBB);
        assert_eq!(msg.text, "grrrr");
    }

    /// Server output — and therefore every GM command's reply — is one GUID and
    /// no name.
    #[test]
    fn system_output_is_the_default_shape() {
        let mut w = Writer::new();
        w.u8(ChatType::System.code());
        w.u32(Language::Universal.code());
        w.u64(0);
        w.u32(15);
        w.cstring("Teleported to.");
        w.u8(0);

        let msg = parse_message(&w.buf).expect("a system line");
        assert_eq!(msg.chat_type(), Some(ChatType::System));
        assert_eq!(msg.sender, 0);
        assert_eq!(msg.text, "Teleported to.");
    }

    /// An unnamed type still parses, on the default layout. The combat log is
    /// almost entirely these, and a client that refused them would fill its
    /// warning channel with ordinary traffic.
    #[test]
    fn a_type_this_client_does_not_name_is_still_read() {
        let mut w = Writer::new();
        w.u8(0x2D); // CHAT_MSG_COMBAT_XP_GAIN
        w.u32(0);
        w.u64(0);
        w.u32(4);
        w.cstring("+42");
        w.u8(0);

        let msg = parse_message(&w.buf).expect("an unnamed type");
        assert!(msg.chat_type().is_none());
        assert_eq!(msg.text, "+42");
    }

    /// **The outbound body has a target string for exactly two types.** An extra
    /// one consumes the message; a missing one makes the message the target.
    #[test]
    fn only_a_whisper_and_a_channel_carry_a_target() {
        let say = message_body(ChatType::Say, Language::Common, None, ".tele tanaris");
        // type, language, then straight into the text.
        assert_eq!(&say[..8], &[0, 0, 0, 0, 7, 0, 0, 0]);
        assert_eq!(&say[8..], b".tele tanaris\0");

        let whisper = message_body(ChatType::Whisper, Language::Common, Some("Merrick"), "hi");
        assert_eq!(&whisper[8..], b"Merrick\0hi\0");

        // A whisper with nowhere to go still sends an empty name rather than
        // shifting the text into the name's place.
        let nowhere = message_body(ChatType::Whisper, Language::Common, None, "hi");
        assert_eq!(&nowhere[8..], b"\0hi\0");
    }

    /// **Every Alliance race speaks Common and every Horde race Orcish**, and
    /// the wrong one earns a notification and silence.
    #[test]
    fn a_races_language_is_the_one_it_knows() {
        for alliance in [1, 3, 4, 7] {
            assert_eq!(Language::for_race(alliance), Language::Common);
        }
        for horde in [2, 5, 6, 8] {
            assert_eq!(Language::for_race(horde), Language::Orcish);
        }
        // Neither is `Universal`, which `IsLanguageAllowedForChatType` refuses
        // for a say — the value anyone would reach for first.
        assert_ne!(Language::for_race(1), Language::Universal);
    }

    #[test]
    fn a_notification_is_one_string() {
        assert_eq!(
            parse_notification(b"You have not learned that language.\0").as_deref(),
            Some("You have not learned that language.")
        );
        assert!(parse_notification(b"").is_none());
    }
}
