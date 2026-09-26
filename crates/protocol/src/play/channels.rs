//! Chat channels — `General - Elwynn Forest`, `Trade - City`, and whatever a
//! player makes with `/join`.
//!
//! ## The packets
//!
//! Eighteen opcodes, and every outbound one is strings:
//!
//! ```text
//! CMSG_JOIN_CHANNEL         cstring name, cstring password
//! CMSG_LEAVE_CHANNEL        cstring name
//! CMSG_CHANNEL_LIST         cstring name                 -> SMSG_CHANNEL_LIST
//! CMSG_CHANNEL_PASSWORD     cstring name, cstring password
//! CMSG_CHANNEL_SET_OWNER    cstring name, cstring player
//! CMSG_CHANNEL_OWNER        cstring name                 -> CHANNEL_OWNER notice
//! CMSG_CHANNEL_MODERATOR    cstring name, cstring player
//! CMSG_CHANNEL_UNMODERATOR  cstring name, cstring player
//! CMSG_CHANNEL_MUTE         cstring name, cstring player
//! CMSG_CHANNEL_UNMUTE       cstring name, cstring player
//! CMSG_CHANNEL_INVITE       cstring name, cstring player
//! CMSG_CHANNEL_KICK         cstring name, cstring player
//! CMSG_CHANNEL_BAN          cstring name, cstring player
//! CMSG_CHANNEL_UNBAN        cstring name, cstring player
//! CMSG_CHANNEL_ANNOUNCEMENTS cstring name
//! CMSG_CHANNEL_MODERATE     cstring name
//!
//! SMSG_CHANNEL_NOTIFY       u8 notice, cstring channel, then per notice
//! SMSG_CHANNEL_LIST         cstring channel, u8 flags, u32 count,
//!                           count x (u64 guid, u8 member flags)
//! ```
//!
//! The field names are vmangos' `ChannelHandler.cpp` (`packet.channelName`,
//! `packet.channelPassword`, `packet.playerName`, `packet.password`), and the
//! string order is the 1.12 wire's — name first — which is the order every
//! two-string packet of this era keeps. The inbound layouts are
//! `Channel::MakeNotifyPacket` and the thirty `Make*` after it in
//! `Channel.cpp`, and `Channel::List`; the per-notice tails are
//! [`NoticeTail`]. **Not seen from a live server yet.**
//!
//! ## The notices
//!
//! [`Notice`] is `ChatNotify` from `Channel.h`, thirty-two codes. Two are
//! events of their own in the interface (`CHAT_MSG_CHANNEL_JOIN` and
//! `_LEAVE`, somebody else coming and going); the rest are one event,
//! `CHAT_MSG_CHANNEL_NOTICE` or `_NOTICE_USER`, whose `arg1` is the word
//! between `CHAT_` and `_NOTICE` in `GlobalStrings.lua` — `YOU_JOINED`,
//! `WRONG_PASSWORD`, `PLAYER_KICKED`. [`Notice::word`] is that word, and
//! [`Notice::names_a_user`] which of the two events it is.
//!
//! **`0x02` is both `YOU_JOINED` and `YOU_CHANGED`**, and the server does
//! not say which: the file has `CHAT_YOU_CHANGED_NOTICE = "Changed Channel:
//! [%s]"` beside `CHAT_YOU_JOINED_NOTICE`, the enum has one code, and vmangos
//! comments the second name out. The client decides — a zone channel
//! replacing the one in the same slot is a change — and that decision lives
//! with the slots, in `crate::game::session::channels`. The same is true of
//! `0x01` (`LEFT` and `SUSPENDED`), which this client reads as `LEFT`.
//!
//! `MODE_CHANGE` (`0x0C`) carries a guid and the member's flags before and
//! after; the interface's words for it are four (`SET_MODERATOR`,
//! `UNSET_MODERATOR`, `SET_VOICE`, `UNSET_VOICE`), chosen by which
//! [`member_flags`] bit moved — [`mode_change_word`].
//!
//! ## What is on the wire is a guid, and the interface wants a name
//!
//! Every notice about a person carries their guid. The event carries their
//! name, so the client resolves it the way it resolves everybody's — the
//! name cache, and `CMSG_NAME_QUERY` for a stranger — which is why
//! [`NoticeTail`] keeps guids and the name is the session's job.

use crate::bytes::{Reader, Writer};

/// `ChatNotify` — `Channel.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    Joined = 0x00,
    Left = 0x01,
    YouJoined = 0x02,
    YouLeft = 0x03,
    WrongPassword = 0x04,
    NotMember = 0x05,
    NotModerator = 0x06,
    PasswordChanged = 0x07,
    OwnerChanged = 0x08,
    PlayerNotFound = 0x09,
    NotOwner = 0x0A,
    ChannelOwner = 0x0B,
    ModeChange = 0x0C,
    AnnouncementsOn = 0x0D,
    AnnouncementsOff = 0x0E,
    ModerationOn = 0x0F,
    ModerationOff = 0x10,
    Muted = 0x11,
    PlayerKicked = 0x12,
    Banned = 0x13,
    PlayerBanned = 0x14,
    PlayerUnbanned = 0x15,
    PlayerNotBanned = 0x16,
    PlayerAlreadyMember = 0x17,
    Invite = 0x18,
    InviteWrongFaction = 0x19,
    WrongFaction = 0x1A,
    InvalidName = 0x1B,
    NotModerated = 0x1C,
    PlayerInvited = 0x1D,
    PlayerInviteBanned = 0x1E,
    Throttled = 0x1F,
}

impl Notice {
    pub fn from_code(code: u8) -> Option<Notice> {
        use Notice::*;
        Some(match code {
            0x00 => Joined,
            0x01 => Left,
            0x02 => YouJoined,
            0x03 => YouLeft,
            0x04 => WrongPassword,
            0x05 => NotMember,
            0x06 => NotModerator,
            0x07 => PasswordChanged,
            0x08 => OwnerChanged,
            0x09 => PlayerNotFound,
            0x0A => NotOwner,
            0x0B => ChannelOwner,
            0x0C => ModeChange,
            0x0D => AnnouncementsOn,
            0x0E => AnnouncementsOff,
            0x0F => ModerationOn,
            0x10 => ModerationOff,
            0x11 => Muted,
            0x12 => PlayerKicked,
            0x13 => Banned,
            0x14 => PlayerBanned,
            0x15 => PlayerUnbanned,
            0x16 => PlayerNotBanned,
            0x17 => PlayerAlreadyMember,
            0x18 => Invite,
            0x19 => InviteWrongFaction,
            0x1A => WrongFaction,
            0x1B => InvalidName,
            0x1C => NotModerated,
            0x1D => PlayerInvited,
            0x1E => PlayerInviteBanned,
            0x1F => Throttled,
            _ => return None,
        })
    }

    /// The interface's word for this notice — the `X` of `CHAT_X_NOTICE` in
    /// `GlobalStrings.lua`, which `ChatFrame_OnEvent` looks up from `arg1`.
    /// `Joined` and `Left` are events of their own and have no word;
    /// `ModeChange`'s is [`mode_change_word`].
    pub fn word(self) -> Option<&'static str> {
        use Notice::*;
        Some(match self {
            Joined | Left | ModeChange => return None,
            YouJoined => "YOU_JOINED",
            YouLeft => "YOU_LEFT",
            WrongPassword => "WRONG_PASSWORD",
            NotMember => "NOT_MEMBER",
            NotModerator => "NOT_MODERATOR",
            PasswordChanged => "PASSWORD_CHANGED",
            OwnerChanged => "OWNER_CHANGED",
            PlayerNotFound => "PLAYER_NOT_FOUND",
            NotOwner => "NOT_OWNER",
            ChannelOwner => "CHANNEL_OWNER",
            AnnouncementsOn => "ANNOUNCEMENTS_ON",
            AnnouncementsOff => "ANNOUNCEMENTS_OFF",
            ModerationOn => "MODERATION_ON",
            ModerationOff => "MODERATION_OFF",
            Muted => "MUTED",
            PlayerKicked => "PLAYER_KICKED",
            Banned => "BANNED",
            PlayerBanned => "PLAYER_BANNED",
            PlayerUnbanned => "PLAYER_UNBANNED",
            PlayerNotBanned => "PLAYER_NOT_BANNED",
            PlayerAlreadyMember => "PLAYER_ALREADY_MEMBER",
            Invite => "INVITE",
            InviteWrongFaction => "INVITE_WRONG_FACTION",
            WrongFaction => "WRONG_FACTION",
            InvalidName => "INVALID_NAME",
            NotModerated => "NOT_MODERATED",
            PlayerInvited => "PLAYER_INVITED",
            PlayerInviteBanned => "PLAYER_INVITE_BANNED",
            Throttled => "THROTTLED",
        })
    }

    /// Whether the notice is about a person and lands as
    /// `CHAT_MSG_CHANNEL_NOTICE_USER` (with the name in `arg2`, and the second
    /// name in `arg5` where there is one) rather than `CHAT_MSG_CHANNEL_NOTICE`.
    /// Transcribed from the `%s` count of each `CHAT_*_NOTICE` string: one
    /// `%s` is the channel alone.
    pub fn names_a_user(self) -> bool {
        use Notice::*;
        matches!(
            self,
            PasswordChanged
                | OwnerChanged
                | PlayerNotFound
                | ChannelOwner
                | ModeChange
                | AnnouncementsOn
                | AnnouncementsOff
                | ModerationOn
                | ModerationOff
                | PlayerKicked
                | PlayerBanned
                | PlayerUnbanned
                | PlayerNotBanned
                | PlayerAlreadyMember
                | Invite
                | PlayerInvited
                | PlayerInviteBanned
        )
    }
}

/// A channel member's flags — `Channel.h`'s `ChannelMemberFlags`, the byte
/// after each guid in `SMSG_CHANNEL_LIST` and the two in a `MODE_CHANGE`.
pub mod member_flags {
    pub const OWNER: u8 = 0x01;
    pub const MODERATOR: u8 = 0x02;
    pub const VOICED: u8 = 0x04;
    pub const MUTED: u8 = 0x08;
    pub const CUSTOM: u8 = 0x10;
    pub const MIC_MUTED: u8 = 0x20;
}

/// The interface's word for a `MODE_CHANGE`, from which bit moved: a
/// moderator made or unmade, a voice given or a mute. `None` when nothing
/// the interface has a sentence for moved.
pub fn mode_change_word(old: u8, new: u8) -> Option<&'static str> {
    let gained = new & !old;
    let lost = old & !new;
    if gained & member_flags::MODERATOR != 0 {
        Some("SET_MODERATOR")
    } else if lost & member_flags::MODERATOR != 0 {
        Some("UNSET_MODERATOR")
    } else if gained & member_flags::MUTED != 0 {
        // `CHAT_UNSET_VOICE_NOTICE = "[%s] %s muted."`
        Some("UNSET_VOICE")
    } else if lost & member_flags::MUTED != 0 || gained & member_flags::VOICED != 0 {
        Some("SET_VOICE")
    } else {
        None
    }
}

/// What follows the channel name in a `SMSG_CHANNEL_NOTIFY`, by notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeTail {
    /// The notices that are the channel alone.
    None,
    /// One person: who joined, who left, who changed the password, the new
    /// owner, whoever toggled a setting, who was invited, who is already on.
    Guid(u64),
    /// `YOU_JOINED`: the channel's flags, and the instance number vmangos
    /// always sends as zero ("the non-zero number will be appended to the
    /// channel name" — its own comment; the interface's `arg10`).
    Joined { flags: u32, instance: u32 },
    /// A name the server could not resolve, or whose invitation was refused.
    Name(String),
    /// The owner's name, or `Nobody`.
    Owner(String),
    /// `MODE_CHANGE`: whose flags, and the byte before and after.
    Mode { guid: u64, old: u8, new: u8 },
    /// Kicked, banned, unbanned: the target and who did it.
    Pair { target: u64, source: u64 },
}

/// One `SMSG_CHANNEL_NOTIFY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelNotify {
    pub notice: Notice,
    pub channel: String,
    pub tail: NoticeTail,
}

/// `SMSG_CHANNEL_NOTIFY`: `u8 notice, cstring channel, tail`.
///
/// A code the enum does not carry is `None` rather than a guess — *unknown
/// values are data*; the caller reports it.
pub fn parse_notify(body: &[u8]) -> Option<ChannelNotify> {
    let mut r = Reader::new(body);
    if !r.has(1) {
        return None;
    }
    let notice = Notice::from_code(r.u8())?;
    let channel = r.cstring();
    use Notice::*;
    let tail = match notice {
        Joined | Left | PasswordChanged | OwnerChanged | AnnouncementsOn | AnnouncementsOff
        | ModerationOn | ModerationOff | PlayerAlreadyMember | Invite => {
            NoticeTail::Guid(guid(&mut r)?)
        }
        YouJoined => {
            if !r.has(8) {
                return None;
            }
            NoticeTail::Joined { flags: r.u32(), instance: r.u32() }
        }
        PlayerNotFound | PlayerNotBanned | PlayerInvited | PlayerInviteBanned => {
            NoticeTail::Name(r.cstring())
        }
        ChannelOwner => NoticeTail::Owner(r.cstring()),
        ModeChange => {
            let who = guid(&mut r)?;
            if !r.has(2) {
                return None;
            }
            NoticeTail::Mode { guid: who, old: r.u8(), new: r.u8() }
        }
        PlayerKicked | PlayerBanned | PlayerUnbanned => {
            let target = guid(&mut r)?;
            let source = guid(&mut r)?;
            NoticeTail::Pair { target, source }
        }
        YouLeft | WrongPassword | NotMember | NotModerator | NotOwner | Muted | Banned
        | InviteWrongFaction | WrongFaction | InvalidName | NotModerated | Throttled => {
            NoticeTail::None
        }
    };
    Some(ChannelNotify { notice, channel, tail })
}

fn guid(r: &mut Reader) -> Option<u64> {
    r.has(8).then(|| r.u64())
}

/// One member of a listed channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Member {
    pub guid: u64,
    /// [`member_flags`].
    pub flags: u8,
}

/// `SMSG_CHANNEL_LIST`: who is on a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelList {
    pub channel: String,
    /// The channel's own flags — vmangos' `ChannelFlags`, not the DBC's.
    pub flags: u8,
    pub members: Vec<Member>,
}

/// `SMSG_CHANNEL_LIST`: `cstring channel, u8 flags, u32 count, count x (u64
/// guid, u8 flags)`. The count is `int32` on the server and written after
/// the fact; a count past the body is a damaged packet and `None`.
pub fn parse_list(body: &[u8]) -> Option<ChannelList> {
    let mut r = Reader::new(body);
    let channel = r.cstring();
    if !r.has(5) {
        return None;
    }
    let flags = r.u8();
    let count = r.u32() as usize;
    if !r.has(count.checked_mul(9)?) {
        return None;
    }
    let members = (0..count)
        .map(|_| {
            let guid = r.u64();
            let flags = r.u8();
            Member { guid, flags }
        })
        .collect();
    Some(ChannelList { channel, flags, members })
}

// ---------------------------------------------------------------------------
// Outbound bodies — every one a free function, beside its parser.

fn one(name: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(name);
    w.buf
}

fn two(name: &str, second: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(name);
    w.cstring(second);
    w.buf
}

/// `CMSG_JOIN_CHANNEL`: the name and the password, empty for none. The
/// server refuses a name that does not begin with a letter
/// (`HandleJoinChannelOpcode`) with an `INVALID_NAME` notice.
pub fn join_body(name: &str, password: &str) -> Vec<u8> {
    two(name, password)
}

/// `CMSG_LEAVE_CHANNEL`.
pub fn leave_body(name: &str) -> Vec<u8> {
    one(name)
}

/// `CMSG_CHANNEL_LIST`, answered by `SMSG_CHANNEL_LIST` or a `NOT_MEMBER`.
pub fn list_body(name: &str) -> Vec<u8> {
    one(name)
}

/// `CMSG_CHANNEL_PASSWORD`.
pub fn password_body(name: &str, password: &str) -> Vec<u8> {
    two(name, password)
}

/// `CMSG_CHANNEL_SET_OWNER`.
pub fn set_owner_body(name: &str, player: &str) -> Vec<u8> {
    two(name, player)
}

/// `CMSG_CHANNEL_OWNER`: who owns it, answered by a `CHANNEL_OWNER` notice.
pub fn owner_body(name: &str) -> Vec<u8> {
    one(name)
}

/// `CMSG_CHANNEL_MODERATOR` and the seven other name-and-player packets:
/// `UNMODERATOR`, `MUTE`, `UNMUTE`, `INVITE`, `KICK`, `BAN`, `UNBAN`. One
/// layout, so one body.
pub fn player_body(name: &str, player: &str) -> Vec<u8> {
    two(name, player)
}

/// `CMSG_CHANNEL_ANNOUNCEMENTS` and `CMSG_CHANNEL_MODERATE`: toggles, the
/// name alone.
pub fn toggle_body(name: &str) -> Vec<u8> {
    one(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notify(notice: u8, channel: &str, tail: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(notice);
        w.cstring(channel);
        w.bytes(tail);
        w.buf
    }

    #[test]
    fn every_code_round_trips_and_the_words_are_the_files_keys() {
        for code in 0u8..0x20 {
            let notice = Notice::from_code(code).expect("a code in the enum");
            assert_eq!(notice as u8, code);
        }
        assert_eq!(Notice::from_code(0x20), None);
        assert_eq!(Notice::YouJoined.word(), Some("YOU_JOINED"));
        assert_eq!(Notice::Joined.word(), None, "an event of its own");
        assert_eq!(Notice::ModeChange.word(), None, "four words, by the flags");
        assert!(Notice::PlayerKicked.names_a_user());
        assert!(!Notice::WrongPassword.names_a_user());
    }

    #[test]
    fn you_joined_carries_the_flags_and_the_instance() {
        let mut tail = Writer::new();
        tail.u32(0x18).u32(0);
        let body = notify(0x02, "General - Elwynn Forest", &tail.buf);
        let parsed = parse_notify(&body).expect("parses");
        assert_eq!(parsed.notice, Notice::YouJoined);
        assert_eq!(parsed.channel, "General - Elwynn Forest");
        assert_eq!(parsed.tail, NoticeTail::Joined { flags: 0x18, instance: 0 });
        // …and short by a byte is refused rather than read as zero.
        assert_eq!(parse_notify(&body[..body.len() - 1]), None);
    }

    #[test]
    fn the_guid_and_pair_and_name_tails_parse_by_notice() {
        let mut g = Writer::new();
        g.u64(0x1234);
        let joined = parse_notify(&notify(0x00, "World", &g.buf)).expect("parses");
        assert_eq!(joined.tail, NoticeTail::Guid(0x1234));

        let mut pair = Writer::new();
        pair.u64(7).u64(9);
        let kicked = parse_notify(&notify(0x12, "World", &pair.buf)).expect("parses");
        assert_eq!(kicked.tail, NoticeTail::Pair { target: 7, source: 9 });

        let mut name = Writer::new();
        name.cstring("Bram");
        let missing = parse_notify(&notify(0x09, "World", &name.buf)).expect("parses");
        assert_eq!(missing.tail, NoticeTail::Name("Bram".into()));

        let owner = parse_notify(&notify(0x0B, "World", &name.buf)).expect("parses");
        assert_eq!(owner.tail, NoticeTail::Owner("Bram".into()));

        let mut mode = Writer::new();
        mode.u64(5).u8(0x00).u8(0x02);
        let changed = parse_notify(&notify(0x0C, "World", &mode.buf)).expect("parses");
        assert_eq!(changed.tail, NoticeTail::Mode { guid: 5, old: 0, new: 2 });

        let bare = parse_notify(&notify(0x04, "World", &[])).expect("parses");
        assert_eq!(bare.tail, NoticeTail::None);
        assert_eq!(parse_notify(&[]), None);
        assert_eq!(parse_notify(&notify(0x7F, "World", &[])), None, "an unknown code is refused");
    }

    #[test]
    fn a_mode_change_names_the_bit_that_moved() {
        assert_eq!(mode_change_word(0, member_flags::MODERATOR), Some("SET_MODERATOR"));
        assert_eq!(mode_change_word(member_flags::MODERATOR, 0), Some("UNSET_MODERATOR"));
        assert_eq!(mode_change_word(0, member_flags::MUTED), Some("UNSET_VOICE"));
        assert_eq!(mode_change_word(member_flags::MUTED, 0), Some("SET_VOICE"));
        assert_eq!(mode_change_word(member_flags::OWNER, member_flags::OWNER), None);
    }

    #[test]
    fn a_list_is_the_channel_its_flags_and_every_member() {
        let mut w = Writer::new();
        w.cstring("World").u8(0x01).u32(2).u64(10).u8(0x01).u64(11).u8(0x00);
        let list = parse_list(&w.buf).expect("parses");
        assert_eq!(list.channel, "World");
        assert_eq!(list.flags, 1);
        assert_eq!(
            list.members,
            vec![Member { guid: 10, flags: 1 }, Member { guid: 11, flags: 0 }]
        );
        // A count past the body is a damaged packet.
        let mut short = Writer::new();
        short.cstring("World").u8(0).u32(3).u64(10).u8(0);
        assert_eq!(parse_list(&short.buf), None);
    }

    #[test]
    fn the_bodies_are_the_strings_in_wire_order() {
        assert_eq!(join_body("World", ""), b"World\0\0");
        assert_eq!(join_body("Secret", "hunter2"), b"Secret\0hunter2\0");
        assert_eq!(leave_body("World"), b"World\0");
        assert_eq!(player_body("World", "Bram"), b"World\0Bram\0");
        assert_eq!(toggle_body("World"), b"World\0");
    }
}
