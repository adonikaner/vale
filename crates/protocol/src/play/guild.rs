//! The guild: nine packets in and twenty-one requests out.
//!
//! ```text
//! SMSG_GUILD_QUERY_RESPONSE   85  u32 id, cstring name, 10 x cstring rank,
//!                                 i32 emblemStyle, i32 emblemColor,
//!                                 i32 borderStyle, i32 borderColor,
//!                                 i32 background
//! SMSG_GUILD_INVITE          131  cstring inviter, cstring guild
//! SMSG_GUILD_DECLINE         134  cstring player
//! SMSG_GUILD_INFO            136  cstring name, u32 day, u32 month, u32 year,
//!                                 u32 members, u32 accounts
//! SMSG_GUILD_ROSTER          138  u32 members, cstring motd, cstring info,
//!                                 u32 ranks, ranks x u32 rights, then per
//!                                 member: u64 guid, u8 presence, cstring name,
//!                                 u32 rank, u8 level, u8 class, u32 zone,
//!                                 (f32 daysOffline, only when presence is 0),
//!                                 cstring note, cstring officerNote
//! SMSG_GUILD_EVENT           146  u8 event, u8 count, count x cstring,
//!                                 then u64 guid when the body has 8 more bytes
//! SMSG_GUILD_COMMAND_RESULT  147  u32 command, cstring name, u32 result
//! MSG_SAVE_GUILD_EMBLEM      497  u32 result
//! MSG_TABARDVENDOR_ACTIVATE  498  u64 npc
//!
//! CMSG_GUILD_QUERY            84  u32 id
//! CMSG_GUILD_INVITE          130  cstring name
//! CMSG_GUILD_ACCEPT          132  no body
//! CMSG_GUILD_DECLINE         133  no body
//! CMSG_GUILD_INFO            135  no body
//! CMSG_GUILD_ROSTER          137  no body
//! CMSG_GUILD_PROMOTE         139  cstring name
//! CMSG_GUILD_DEMOTE          140  cstring name
//! CMSG_GUILD_LEAVE           141  no body
//! CMSG_GUILD_REMOVE          142  cstring name
//! CMSG_GUILD_DISBAND         143  no body
//! CMSG_GUILD_LEADER          144  cstring name
//! CMSG_GUILD_MOTD            145  cstring text
//! CMSG_GUILD_RANK            561  u32 rank, u32 rights, cstring name
//! CMSG_GUILD_ADD_RANK        562  cstring name
//! CMSG_GUILD_DEL_RANK        563  no body
//! CMSG_GUILD_SET_PUBLIC_NOTE 564  cstring player, cstring note
//! CMSG_GUILD_SET_OFFICER_NOTE 565 cstring player, cstring note
//! CMSG_GUILD_INFO_TEXT       764  cstring text
//! MSG_SAVE_GUILD_EMBLEM      497  u64 npc, i32 emblemStyle, i32 emblemColor,
//!                                 i32 borderStyle, i32 borderColor,
//!                                 i32 background
//! MSG_TABARDVENDOR_ACTIVATE  498  u64 npc
//! ```
//!
//! The layouts follow vmangos' `Server/Packets/Guild.cpp`, and the values of
//! the enumerations follow its `Guild/Guild.h`.
//!
//! ## What the server does not send
//!
//! A character's guild is two of its own update fields, `PLAYER_GUILDID` and
//! `PLAYER_GUILDRANK`. No packet announces joining or leaving a guild to the
//! character it happens to; the fields change. The guild's name and its rank
//! names are the answer to `CMSG_GUILD_QUERY`, which the client sends for a
//! guild id it has no name for.
//!
//! The roster is sent only in answer to `CMSG_GUILD_ROSTER`, and after a note
//! or a rank is edited. A member joining, leaving or changing rank arrives as
//! an `SMSG_GUILD_EVENT`, which names the member and carries no row, so the
//! client asks for the roster again to redraw it.
//!
//! ## The officer note is empty, not absent
//!
//! Every member row carries both notes. For a viewer whose rank lacks
//! [`rights::VIEW_OFFICER_NOTE`] the server writes an empty officer note.

use crate::bytes::{Reader, Writer};

/// How many rank names `SMSG_GUILD_QUERY_RESPONSE` carries. The server always
/// writes ten and leaves the unused ones empty.
pub const RANK_SLOTS: usize = 10;

/// The bits of a rank's rights word. Values follow vmangos' `Guild.h`, with
/// the `0x40` bit the server sets on every rank removed: it states no right.
pub mod rights {
    pub const CHAT_LISTEN: u32 = 0x0000_0001;
    pub const CHAT_SPEAK: u32 = 0x0000_0002;
    pub const OFFICER_CHAT_LISTEN: u32 = 0x0000_0004;
    pub const OFFICER_CHAT_SPEAK: u32 = 0x0000_0008;
    pub const INVITE: u32 = 0x0000_0010;
    pub const REMOVE: u32 = 0x0000_0020;
    /// Set on every rank, including one with no rights.
    pub const EMPTY: u32 = 0x0000_0040;
    pub const PROMOTE: u32 = 0x0000_0080;
    pub const DEMOTE: u32 = 0x0000_0100;
    pub const SET_MOTD: u32 = 0x0000_1000;
    pub const EDIT_PUBLIC_NOTE: u32 = 0x0000_2000;
    pub const VIEW_OFFICER_NOTE: u32 = 0x0000_4000;
    pub const EDIT_OFFICER_NOTE: u32 = 0x0000_8000;
    pub const MODIFY_INFO: u32 = 0x0001_0000;

    /// The thirteen rights in the order of the guild-control window's
    /// checkboxes, which is the order of the `GUILDCONTROL_OPTION1..13`
    /// strings in `GlobalStrings.lua`.
    pub const CONTROL_ORDER: [u32; 13] = [
        CHAT_LISTEN,
        CHAT_SPEAK,
        OFFICER_CHAT_LISTEN,
        OFFICER_CHAT_SPEAK,
        PROMOTE,
        DEMOTE,
        INVITE,
        REMOVE,
        SET_MOTD,
        EDIT_PUBLIC_NOTE,
        VIEW_OFFICER_NOTE,
        EDIT_OFFICER_NOTE,
        MODIFY_INFO,
    ];
}

/// The bits of a roster member's presence byte. Zero is offline.
pub mod presence {
    pub const ONLINE: u8 = 1;
    pub const AFK: u8 = 2;
    pub const DND: u8 = 4;
}

/// The five numbers of a guild's tabard design, each zero-based. A guild
/// with no emblem has -1 in every field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Emblem {
    pub style: i32,
    pub color: i32,
    pub border_style: i32,
    pub border_color: i32,
    pub background: i32,
}

/// How many values each of the five numbers has, in the order of
/// [`Emblem::fields`]: 170 emblems, 17 emblem colours, 6 borders, 17 border
/// colours and 51 backgrounds. The 1.12.1 client refuses to save a design
/// with a number outside its count.
pub const EMBLEM_COUNTS: [i32; 5] = [170, 17, 6, 17, 51];

impl Emblem {
    /// A guild that has never saved an emblem.
    pub const NONE: Emblem = Emblem {
        style: -1,
        color: -1,
        border_style: -1,
        border_color: -1,
        background: -1,
    };

    /// The five numbers in wire order: emblem style, emblem colour, border
    /// style, border colour, background. This is also the order of the
    /// tabard designer's five rows.
    pub fn fields(self) -> [i32; 5] {
        [
            self.style,
            self.color,
            self.border_style,
            self.border_color,
            self.background,
        ]
    }

    pub fn from_fields(fields: [i32; 5]) -> Emblem {
        Emblem {
            style: fields[0],
            color: fields[1],
            border_style: fields[2],
            border_color: fields[3],
            background: fields[4],
        }
    }

    /// Whether every number is inside its count. A guild with no emblem has
    /// -1 in every field and is not drawable.
    pub fn in_range(self) -> bool {
        self.fields()
            .iter()
            .zip(EMBLEM_COUNTS)
            .all(|(value, count)| (0..count).contains(value))
    }
}

/// `SMSG_GUILD_QUERY_RESPONSE` (85): a guild's name, its rank names and its
/// emblem.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GuildQuery {
    pub id: u32,
    pub name: String,
    /// The rank names in rank order, guild master first, without the empty
    /// trailing slots.
    pub ranks: Vec<String>,
    pub emblem: Emblem,
}

/// `SMSG_GUILD_QUERY_RESPONSE` (85).
pub fn parse_query(body: &[u8]) -> Option<GuildQuery> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let id = r.u32();
    let name = r.cstring();
    let mut ranks: Vec<String> = (0..RANK_SLOTS).map(|_| r.cstring()).collect();
    // The server writes ten names and leaves the unused ones empty. A rank in
    // use always has a name, so the count is the last non-empty slot.
    while ranks.last().is_some_and(String::is_empty) {
        ranks.pop();
    }
    // A body that ends before the emblem still names the guild.
    let emblem = if r.has(20) {
        Emblem {
            style: r.u32() as i32,
            color: r.u32() as i32,
            border_style: r.u32() as i32,
            border_color: r.u32() as i32,
            background: r.u32() as i32,
        }
    } else {
        Emblem::default()
    };
    Some(GuildQuery {
        id,
        name,
        ranks,
        emblem,
    })
}

/// One row of `SMSG_GUILD_ROSTER`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Member {
    pub guid: u64,
    /// [`presence`] bits; zero for a member who is offline.
    pub presence: u8,
    pub name: String,
    /// The rank index: 0 is the guild master and a larger number is a lower
    /// rank.
    pub rank: u32,
    pub level: u8,
    /// A `ChrClasses.dbc` id.
    pub class: u8,
    /// An `AreaTable.dbc` zone id. For an offline member it is the zone they
    /// logged out in.
    pub zone: u32,
    /// Days since the member logged out, as a fraction. Zero for a member who
    /// is online; the field is not on the wire for them.
    pub days_offline: f32,
    pub note: String,
    /// Empty when the viewer's rank may not read officer notes.
    pub officer_note: String,
}

impl Member {
    pub fn online(&self) -> bool {
        self.presence != 0
    }
}

/// `SMSG_GUILD_ROSTER` (138): the whole member list, the two guild texts, and
/// each rank's rights.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Roster {
    pub motd: String,
    pub info: String,
    /// One rights word per rank, in rank order. Its length is the number of
    /// ranks the guild has.
    pub rank_rights: Vec<u32>,
    pub members: Vec<Member>,
}

/// `SMSG_GUILD_ROSTER` (138).
///
/// A body that ends inside a member keeps the members read before it. vmangos
/// stops writing members when the packet would exceed its size limit, and
/// still states the full count in the first word.
pub fn parse_roster(body: &[u8]) -> Option<Roster> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let count = r.u32() as usize;
    let motd = r.cstring();
    let info = r.cstring();
    if !r.has(4) {
        return None;
    }
    let ranks = r.u32() as usize;
    if !r.has(ranks.checked_mul(4)?) {
        return None;
    }
    let rank_rights = (0..ranks).map(|_| r.u32()).collect();
    // The count is the server's and is not trusted as an allocation size.
    let mut members = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        // The fixed part of a row before its name: guid and presence.
        if !r.has(9) {
            break;
        }
        let guid = r.u64();
        let presence = r.u8();
        let name = r.cstring();
        // Rank, level, class and zone.
        if !r.has(10) {
            break;
        }
        let rank = r.u32();
        let level = r.u8();
        let class = r.u8();
        let zone = r.u32();
        let days_offline = if presence == 0 {
            if !r.has(4) {
                break;
            }
            r.f32()
        } else {
            0.0
        };
        let note = r.cstring();
        let officer_note = r.cstring();
        members.push(Member {
            guid,
            presence,
            name,
            rank,
            level,
            class,
            zone,
            days_offline,
            note,
            officer_note,
        });
    }
    Some(Roster {
        motd,
        info,
        rank_rights,
        members,
    })
}

/// `SMSG_GUILD_EVENT`'s first byte. Values follow vmangos' `GuildEvents`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Strings: the promoter, the member, the new rank's name.
    Promotion,
    /// Strings: the demoter, the member, the new rank's name.
    Demotion,
    /// String: the message of the day.
    Motd,
    /// String: the member who joined.
    Joined,
    /// String: the member who left.
    Left,
    /// Strings: the member removed, the member who removed them.
    Removed,
    /// String: the guild master.
    LeaderIs,
    /// Strings: the old guild master, the new one.
    LeaderChanged,
    Disbanded,
    TabardChange,
    RankNameChanged,
    RosterChanged,
    /// String: the member who logged in.
    SignedOn,
    /// String: the member who logged out.
    SignedOff,
    /// A value this client has no name for, carried so that it can be logged.
    Unknown(u8),
}

impl EventKind {
    pub fn from_code(code: u8) -> EventKind {
        match code {
            0x00 => EventKind::Promotion,
            0x01 => EventKind::Demotion,
            0x02 => EventKind::Motd,
            0x03 => EventKind::Joined,
            0x04 => EventKind::Left,
            0x05 => EventKind::Removed,
            0x06 => EventKind::LeaderIs,
            0x07 => EventKind::LeaderChanged,
            0x08 => EventKind::Disbanded,
            0x09 => EventKind::TabardChange,
            0x0a => EventKind::RankNameChanged,
            0x0b => EventKind::RosterChanged,
            0x0c => EventKind::SignedOn,
            0x0d => EventKind::SignedOff,
            other => EventKind::Unknown(other),
        }
    }

    /// Whether the event means the held roster no longer matches the
    /// server's. The motd, the leader announcement and the tabard do not
    /// change a member row.
    pub fn changes_roster(self) -> bool {
        matches!(
            self,
            EventKind::Promotion
                | EventKind::Demotion
                | EventKind::Joined
                | EventKind::Left
                | EventKind::Removed
                | EventKind::LeaderChanged
                | EventKind::RankNameChanged
                | EventKind::RosterChanged
                | EventKind::SignedOn
                | EventKind::SignedOff
        )
    }
}

/// `SMSG_GUILD_EVENT` (146): something happened in the guild, stated as a
/// kind and up to three strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildEvent {
    pub kind: EventKind,
    /// What each string means depends on the kind; see [`EventKind`].
    pub params: Vec<String>,
    /// The member the event is about, for the kinds the server states one
    /// for: joining, leaving, logging in and logging out.
    pub guid: Option<u64>,
}

/// `SMSG_GUILD_EVENT` (146).
pub fn parse_event(body: &[u8]) -> Option<GuildEvent> {
    let mut r = Reader::new(body);
    if !r.has(2) {
        return None;
    }
    let kind = EventKind::from_code(r.u8());
    let count = r.u8() as usize;
    let mut params = Vec::with_capacity(count.min(3));
    for _ in 0..count {
        if !r.has(1) {
            break;
        }
        params.push(r.cstring());
    }
    let guid = r.has(8).then(|| r.u64());
    Some(GuildEvent { kind, params, guid })
}

/// `SMSG_GUILD_COMMAND_RESULT`'s command word. Values follow vmangos'
/// `Typecommand`.
pub mod command {
    pub const CREATE: u32 = 0x00;
    pub const INVITE: u32 = 0x01;
    pub const QUIT: u32 = 0x03;
    pub const FOUNDER: u32 = 0x0e;
}

/// `SMSG_GUILD_COMMAND_RESULT` (147): the answer to a guild request, which is
/// a refusal unless `result` is zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    /// One of [`command`].
    pub command: u32,
    /// A player's or a guild's name, or empty. Which one depends on the
    /// command and the result.
    pub name: String,
    /// Zero for success. Otherwise vmangos' `CommandErrors`.
    pub result: u32,
}

impl CommandResult {
    /// The `GlobalStrings.lua` key this result prints, or `None` for one that
    /// prints nothing.
    ///
    /// The keys are matched to the codes by name: each `CommandErrors` value
    /// in vmangos' `Guild.h` has one key of the same name in
    /// `GlobalStrings.lua`. Result 8 is two errors that share a value: a
    /// guild master leaving, for the quit command, and a missing right for
    /// every other command.
    pub fn message(&self) -> Option<&'static str> {
        Some(match (self.result, self.command) {
            (0x00, command::CREATE) => "ERR_GUILD_CREATE_S",
            (0x00, command::INVITE) => "ERR_GUILD_INVITE_S",
            (0x00, command::QUIT) => "ERR_GUILD_QUIT_S",
            (0x00, command::FOUNDER) => "ERR_GUILD_FOUNDER_S",
            (0x00, _) => return None,
            (0x01, _) => "ERR_GUILD_INTERNAL",
            (0x02, _) => "ERR_ALREADY_IN_GUILD",
            (0x03, _) => "ERR_ALREADY_IN_GUILD_S",
            (0x04, _) => "ERR_INVITED_TO_GUILD",
            (0x05, _) => "ERR_ALREADY_INVITED_TO_GUILD_S",
            (0x06, _) => "ERR_GUILD_NAME_INVALID",
            (0x07, _) => "ERR_GUILD_NAME_EXISTS_S",
            (0x08, command::QUIT) => "ERR_GUILD_LEADER_LEAVE",
            (0x08, _) => "ERR_GUILD_PERMISSIONS",
            (0x09, _) => "ERR_GUILD_PLAYER_NOT_IN_GUILD",
            (0x0a, _) => "ERR_GUILD_PLAYER_NOT_IN_GUILD_S",
            (0x0b, _) => "ERR_GUILD_PLAYER_NOT_FOUND_S",
            (0x0c, _) => "ERR_GUILD_NOT_ALLIED",
            (0x0d, _) => "ERR_GUILD_RANK_TOO_HIGH_S",
            (0x0e, _) => "ERR_GUILD_RANK_TOO_LOW_S",
            (0x11, _) => "ERR_GUILD_RANKS_LOCKED",
            (0x12, _) => "ERR_GUILD_RANK_IN_USE",
            (0x13, _) => "ERR_IGNORING_YOU_S",
            _ => return None,
        })
    }
}

/// `SMSG_GUILD_COMMAND_RESULT` (147).
pub fn parse_command_result(body: &[u8]) -> Option<CommandResult> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let command = r.u32();
    let name = r.cstring();
    if !r.has(4) {
        return None;
    }
    Some(CommandResult {
        command,
        name,
        result: r.u32(),
    })
}

/// `SMSG_GUILD_INVITE` (131): somebody invites the character to a guild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub inviter: String,
    pub guild: String,
}

/// `SMSG_GUILD_INVITE` (131).
pub fn parse_invite(body: &[u8]) -> Option<Invite> {
    let mut r = Reader::new(body);
    if !r.has(2) {
        return None;
    }
    Some(Invite {
        inviter: r.cstring(),
        guild: r.cstring(),
    })
}

/// `SMSG_GUILD_DECLINE` (134): the name of the player who declined the
/// character's invitation.
pub fn parse_decline(body: &[u8]) -> Option<String> {
    let mut r = Reader::new(body);
    if !r.has(1) {
        return None;
    }
    Some(r.cstring())
}

/// `SMSG_GUILD_INFO` (136): the answer to `/ginfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildInfo {
    pub name: String,
    pub day: u32,
    pub month: u32,
    pub year: u32,
    pub members: u32,
    pub accounts: u32,
}

/// `SMSG_GUILD_INFO` (136).
pub fn parse_info(body: &[u8]) -> Option<GuildInfo> {
    let mut r = Reader::new(body);
    if !r.has(1) {
        return None;
    }
    let name = r.cstring();
    if !r.has(20) {
        return None;
    }
    Some(GuildInfo {
        name,
        day: r.u32(),
        month: r.u32(),
        year: r.u32(),
        members: r.u32(),
        accounts: r.u32(),
    })
}

/// `MSG_SAVE_GUILD_EMBLEM`'s result word. Values follow vmangos'
/// `GuildEmblem`.
pub mod emblem_result {
    pub const SUCCESS: u32 = 0;
    pub const INVALID_TABARD_COLORS: u32 = 1;
    pub const NO_GUILD: u32 = 2;
    pub const NOT_GUILD_MASTER: u32 = 3;
    pub const NOT_ENOUGH_MONEY: u32 = 4;
    /// A refusal that prints nothing: the designer is out of reach.
    pub const FAIL_NO_MESSAGE: u32 = 5;

    /// The `GlobalStrings.lua` key a result prints, or `None` for
    /// [`FAIL_NO_MESSAGE`] and for a value outside the six.
    pub fn message(result: u32) -> Option<&'static str> {
        Some(match result {
            SUCCESS => "ERR_GUILDEMBLEM_SUCCESS",
            INVALID_TABARD_COLORS => "ERR_GUILDEMBLEM_INVALID_TABARD_COLORS",
            NO_GUILD => "ERR_GUILDEMBLEM_NOGUILD",
            NOT_GUILD_MASTER => "ERR_GUILDEMBLEM_NOTGUILDMASTER",
            NOT_ENOUGH_MONEY => "ERR_GUILDEMBLEM_NOTENOUGHMONEY",
            _ => return None,
        })
    }
}

/// What the server charges to save an emblem, in copper: ten gold. vmangos'
/// `HandleSaveGuildEmblemOpcode` states the amount; no packet carries it.
pub const EMBLEM_COST: u32 = 100_000;

/// `MSG_SAVE_GUILD_EMBLEM` (497), from the server: one of
/// [`emblem_result`].
pub fn parse_emblem_result(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(4).then(|| r.u32())
}

/// `MSG_TABARDVENDOR_ACTIVATE` (498), from the server: the designer's guid.
pub fn parse_tabard_vendor(body: &[u8]) -> Option<u64> {
    let mut r = Reader::new(body);
    r.has(8).then(|| r.u64())
}

/// `MSG_TABARDVENDOR_ACTIVATE` (498), to the server.
pub fn tabard_vendor_body(npc: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(npc);
    w.buf
}

/// `MSG_SAVE_GUILD_EMBLEM` (497), to the server: the designer and the five
/// numbers of the design.
pub fn emblem_body(npc: u64, emblem: Emblem) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(npc)
        .u32(emblem.style as u32)
        .u32(emblem.color as u32)
        .u32(emblem.border_style as u32)
        .u32(emblem.border_color as u32)
        .u32(emblem.background as u32);
    w.buf
}

/// A body that is one string: the name of a player, a rank or a text.
pub fn text_body(text: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(text);
    w.buf
}

/// `CMSG_GUILD_QUERY` (84).
pub fn query_body(guild: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(guild);
    w.buf
}

/// `CMSG_GUILD_RANK` (561): replace one rank's name and rights. `rights` is
/// the bits of [`rights`]; the [`rights::EMPTY`] bit is added here, as the
/// server stores it on every rank.
pub fn rank_body(rank: u32, rights: u32, name: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(rank).u32(rights | rights::EMPTY).cstring(name);
    w.buf
}

/// `CMSG_GUILD_SET_PUBLIC_NOTE` (564) and `CMSG_GUILD_SET_OFFICER_NOTE` (565),
/// which share a layout.
pub fn note_body(player: &str, note: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(player).cstring(note);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster_body() -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(2).cstring("Raid at eight").cstring("We raid.");
        w.u32(2).u32(0x000f_f1ff).u32(0x43);
        // Online: no offline time on the wire.
        w.u64(0x10).u8(presence::ONLINE | presence::AFK).cstring("Bram");
        w.u32(0).u8(60).u8(1).u32(1519).cstring("tank").cstring("reliable");
        // Offline: four more bytes before the notes.
        w.u64(0x11).u8(0).cstring("Odalys");
        w.u32(1).u8(42).u8(8).u32(12).f32(2.5).cstring("").cstring("");
        w.buf
    }

    /// The offline time is on the wire only for a member whose presence byte
    /// is zero. A reader that always takes it, or never does, misreads every
    /// row after the first of the other kind.
    #[test]
    fn a_roster_row_carries_the_offline_time_only_when_offline() {
        let roster = parse_roster(&roster_body()).expect("parses");
        assert_eq!(roster.motd, "Raid at eight");
        assert_eq!(roster.info, "We raid.");
        assert_eq!(roster.rank_rights, [0x000f_f1ff, 0x43]);
        assert_eq!(roster.members.len(), 2);
        let bram = &roster.members[0];
        assert!(bram.online());
        assert_eq!((bram.name.as_str(), bram.rank, bram.level, bram.class), ("Bram", 0, 60, 1));
        assert_eq!(bram.zone, 1519);
        assert_eq!((bram.note.as_str(), bram.officer_note.as_str()), ("tank", "reliable"));
        assert_eq!(bram.days_offline, 0.0);
        let odalys = &roster.members[1];
        assert!(!odalys.online());
        assert_eq!((odalys.name.as_str(), odalys.rank, odalys.zone), ("Odalys", 1, 12));
        assert_eq!(odalys.days_offline, 2.5);
    }

    /// A roster cut short keeps the members that were whole.
    #[test]
    fn a_truncated_roster_keeps_the_rows_before_the_cut() {
        let body = roster_body();
        let roster = parse_roster(&body[..body.len() - 8]).expect("parses");
        assert_eq!(roster.members.len(), 1);
        assert_eq!(roster.members[0].name, "Bram");
    }

    /// The query answer always has ten rank slots, and the unused ones are
    /// empty strings that are not ranks.
    #[test]
    fn the_query_answer_drops_the_empty_rank_slots() {
        let mut w = Writer::new();
        w.u32(7).cstring("The Watch");
        for rank in ["Guild Master", "Officer", "Veteran", "Member", "Initiate"] {
            w.cstring(rank);
        }
        for _ in 5..RANK_SLOTS {
            w.cstring("");
        }
        w.u32(3).u32(4).u32(5).u32(6).u32(7);
        let query = parse_query(&w.buf).expect("parses");
        assert_eq!((query.id, query.name.as_str()), (7, "The Watch"));
        assert_eq!(query.ranks.len(), 5);
        assert_eq!(query.ranks[4], "Initiate");
        assert_eq!(
            query.emblem,
            Emblem {
                style: 3,
                color: 4,
                border_style: 5,
                border_color: 6,
                background: 7
            }
        );
    }

    /// The guid at the end of an event is present for some kinds and absent
    /// for others, and the string count says where the strings end.
    #[test]
    fn an_event_reads_its_strings_and_the_optional_guid() {
        let mut w = Writer::new();
        w.u8(0x00).u8(3).cstring("Bram").cstring("Odalys").cstring("Officer");
        let event = parse_event(&w.buf).expect("parses");
        assert_eq!(event.kind, EventKind::Promotion);
        assert_eq!(event.params, ["Bram", "Odalys", "Officer"]);
        assert_eq!(event.guid, None);

        let mut w = Writer::new();
        w.u8(0x0c).u8(1).cstring("Odalys").u64(0x11);
        let event = parse_event(&w.buf).expect("parses");
        assert_eq!(event.kind, EventKind::SignedOn);
        assert_eq!(event.guid, Some(0x11));
        assert!(event.kind.changes_roster());
        assert!(!EventKind::Motd.changes_roster());
    }

    /// Result 8 is a different sentence for the quit command than for any
    /// other, and a success prints only for the commands that have a
    /// sentence.
    #[test]
    fn a_command_result_names_its_sentence_by_result_and_command() {
        let result = |command, result| CommandResult {
            command,
            name: String::new(),
            result,
        };
        assert_eq!(result(command::QUIT, 8).message(), Some("ERR_GUILD_LEADER_LEAVE"));
        assert_eq!(result(command::INVITE, 8).message(), Some("ERR_GUILD_PERMISSIONS"));
        assert_eq!(result(command::QUIT, 0).message(), Some("ERR_GUILD_QUIT_S"));
        assert_eq!(result(0x13, 0).message(), None);
        assert_eq!(result(command::INVITE, 0x0b).message(), Some("ERR_GUILD_PLAYER_NOT_FOUND_S"));

        let mut w = Writer::new();
        w.u32(command::INVITE).cstring("Nobody").u32(0x0b);
        let parsed = parse_command_result(&w.buf).expect("parses");
        assert_eq!((parsed.command, parsed.name.as_str(), parsed.result), (1, "Nobody", 0x0b));
    }

    /// The rank request always carries the bit the server keeps on every
    /// rank, so a rank saved with no boxes ticked is stored as the server's
    /// own empty rank.
    #[test]
    fn the_rank_body_carries_the_empty_bit() {
        let body = rank_body(2, rights::INVITE, "Veteran");
        let mut r = Reader::new(&body);
        assert_eq!(r.u32(), 2);
        assert_eq!(r.u32(), rights::INVITE | rights::EMPTY);
        assert_eq!(r.cstring(), "Veteran");
    }

    /// The emblem request is the designer's guid and the five numbers in the
    /// order the query answer states them.
    #[test]
    fn the_emblem_body_is_the_guid_and_five_numbers() {
        let emblem = Emblem {
            style: 3,
            color: 4,
            border_style: 5,
            border_color: 6,
            background: 7,
        };
        let body = emblem_body(0x99, emblem);
        assert_eq!(body.len(), 28);
        let mut r = Reader::new(&body);
        assert_eq!(r.u64(), 0x99);
        assert_eq!([r.u32(), r.u32(), r.u32(), r.u32(), r.u32()], [3, 4, 5, 6, 7]);
        assert_eq!(parse_emblem_result(&[4, 0, 0, 0]), Some(emblem_result::NOT_ENOUGH_MONEY));
        assert_eq!(parse_tabard_vendor(&tabard_vendor_body(0x99)), Some(0x99));
    }

    /// A design is drawable when each number is inside its count, and the
    /// all -1 emblem of a guild that never saved one is not.
    #[test]
    fn an_emblem_is_in_range_when_each_number_is_inside_its_count() {
        assert!(!Emblem::NONE.in_range());
        assert!(Emblem::default().in_range());
        assert!(Emblem::from_fields([169, 16, 5, 16, 50]).in_range());
        assert!(!Emblem::from_fields([170, 0, 0, 0, 0]).in_range());
        assert!(!Emblem::from_fields([0, 0, 6, 0, 0]).in_range());
        assert_eq!(Emblem::from_fields([1, 2, 3, 4, 5]).fields(), [1, 2, 3, 4, 5]);
        assert_eq!(emblem_result::message(5), None);
        assert_eq!(emblem_result::message(4), Some("ERR_GUILDEMBLEM_NOTENOUGHMONEY"));
    }

    /// The control window's thirteen rights are distinct bits, none of them
    /// the bit that states no right.
    #[test]
    fn the_control_order_is_thirteen_distinct_rights() {
        let mut seen = 0u32;
        for bit in rights::CONTROL_ORDER {
            assert_eq!(bit.count_ones(), 1);
            assert_eq!(seen & bit, 0);
            assert_ne!(bit, rights::EMPTY);
            seen |= bit;
        }
    }
}
