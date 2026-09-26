//! **Who you know** — the friends list, the ignore list and the /who search:
//! four packets in, six verbs back.
//!
//! ```text
//! SMSG_FRIEND_LIST     103  u8 count, then count x (u64 guid, u8 status,
//!                           and if status != 0: u32 area, u32 level, u32 class)
//! SMSG_FRIEND_STATUS   104  u8 result, u64 guid,
//!                           and for the two "online" results: u8 status,
//!                           u32 area, u32 level, u32 class
//! SMSG_IGNORE_LIST     107  u8 count, then count x u64 guid
//! SMSG_WHO              99  u32 listed, u32 online, then listed x
//!                           (cstring name, cstring guild, u32 level,
//!                            u32 class, u32 race, u32 zone)
//!
//! CMSG_FRIEND_LIST     102  no body
//! CMSG_ADD_FRIEND      105  cstring name
//! CMSG_DEL_FRIEND      106  u64 guid
//! CMSG_ADD_IGNORE      108  cstring name
//! CMSG_DEL_IGNORE      109  u64 guid
//! CMSG_WHO              98  u32 levelMin, u32 levelMax, cstring name,
//!                           cstring guild, u32 raceMask, u32 classMask,
//!                           u32 zoneCount, zoneCount x u32,
//!                           u32 termCount, termCount x cstring
//! ```
//!
//! All ten layouts are vmangos' `WorldPackets::Social` (`Packets/Social.h` and
//! `.cpp`), `WorldPackets::Misc` and `MiscHandler.cpp`'s
//! `WhoListClientQueryTask`, which is the authority for anything that crosses
//! the wire.
//!
//! ## The two lists are guids and the panel wants names
//!
//! `SMSG_FRIEND_LIST` and `SMSG_IGNORE_LIST` carry **nothing but a guid** per
//! entry — no name, on either list. `FriendsList_Update` draws
//! `GetFriendInfo(i)`'s first return and `IgnoreList_Update` draws
//! `GetIgnoreName(i)` and nothing else, so both panels are empty until a
//! `CMSG_NAME_QUERY` has answered for every guid on them. That is the same road
//! [`crate::state::objects::ObjectManager::resolve_names`] already walks for the
//! units in view, and it is why a friends list can draw *Unknown* on the frame
//! it arrives: `FriendsList_Update`'s own `if ( not name )` branch is written
//! for exactly this.
//!
//! ## …and the who list is names and nothing else
//!
//! `SMSG_WHO` is the mirror image: every row is spelled out — the player's name,
//! the guild's name, and four ids — and **no guid at all**. So a who row cannot
//! become a unit, a target or an invite by anything but its name, which is why
//! `FriendsFrameWhoButton_OnClick` reads the *label* off the button rather than
//! an index, and why `InviteByName` exists at all.
//!
//! ## The count that is not the count
//!
//! `SMSG_WHO`'s two leading words are **listed** and **online**, and they
//! differ: vmangos stops filling rows at 49 and then writes the whole matching
//! population into the second word. `WhoList_Update` reads both — the first
//! sizes the list, the second prints *"N players total"* — and compares the
//! second against `MAX_WHOS_FROM_SERVER` to decide whether to say the answer was
//! truncated. A reader that takes one number and repeats it draws a truncation
//! notice that never appears.

use crate::bytes::{Reader, Writer};

/// One row of `SMSG_FRIEND_LIST`.
///
/// The last three are only on the wire for an online friend; for an offline one
/// they are zero here and the interface never reads them —
/// `FriendsList_Update`'s `if ( connected )` picks a different template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Friend {
    pub guid: u64,
    /// 0 offline, 1 online, 2 AFK, 3 DND — vmangos' `FRIEND_STATUS_*`.
    pub status: u8,
    /// The zone id, which the panel turns into a name through `AreaTable.dbc`.
    pub area: u32,
    pub level: u32,
    /// A `ChrClasses.dbc` id, not a class mask.
    pub class: u32,
}

impl Friend {
    /// Is this friend logged in? All three non-zero statuses are.
    pub fn online(self) -> bool {
        self.status != 0
    }
}

/// `SMSG_FRIEND_LIST` (103).
///
/// A short tail stops the walk rather than failing the packet: the friends
/// already read out of a damaged body are still the server's answer.
pub fn parse_friend_list(body: &[u8]) -> Option<Vec<Friend>> {
    let mut r = Reader::new(body);
    if !r.has(1) {
        return None;
    }
    let count = r.u8() as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let Some(friend) = read_friend(&mut r) else {
            break;
        };
        out.push(friend);
    }
    Some(out)
}

/// The `(guid, status, [area, level, class])` group, shared by the list and the
/// status packet — the two spell it identically and vmangos writes both from the
/// same `FriendInfo`.
fn read_friend(r: &mut Reader) -> Option<Friend> {
    if !r.has(9) {
        return None;
    }
    let guid = r.u64();
    let status = r.u8();
    let mut friend = Friend {
        guid,
        status,
        ..Friend::default()
    };
    if status != 0 {
        if !r.has(12) {
            return None;
        }
        friend.area = r.u32();
        friend.level = r.u32();
        friend.class = r.u32();
    }
    Some(friend)
}

/// `SMSG_IGNORE_LIST` (107): guids, and nothing else at all.
pub fn parse_ignore_list(body: &[u8]) -> Option<Vec<u64>> {
    let mut r = Reader::new(body);
    if !r.has(1) {
        return None;
    }
    let count = r.u8() as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if !r.has(8) {
            break;
        }
        out.push(r.u64());
    }
    Some(out)
}

/// `SMSG_FRIEND_STATUS`'s first byte — vmangos' `FriendsResult` (`SocialMgr.h`).
///
/// Which `GlobalStrings.lua` key each one prints is [`FriendsResult::message`],
/// and that half is a reading rather than a measurement: see its own note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendsResult {
    DbError,
    ListFull,
    Online,
    Offline,
    NotFound,
    Removed,
    AddedOnline,
    AddedOffline,
    Already,
    Yourself,
    Enemy,
    IgnoreFull,
    IgnoreSelf,
    IgnoreNotFound,
    IgnoreAlready,
    IgnoreAdded,
    IgnoreRemoved,
    IgnoreAmbiguous,
    /// Anything this client has no name for, including the whole mute family,
    /// which 1.12's interface has no window for. Carried rather than refused, on
    /// this crate's own rule about unknown values.
    Unknown(u8),
}

impl FriendsResult {
    pub fn from_code(code: u8) -> FriendsResult {
        match code {
            0x00 => FriendsResult::DbError,
            0x01 => FriendsResult::ListFull,
            0x02 => FriendsResult::Online,
            0x03 => FriendsResult::Offline,
            0x04 => FriendsResult::NotFound,
            0x05 => FriendsResult::Removed,
            0x06 => FriendsResult::AddedOnline,
            0x07 => FriendsResult::AddedOffline,
            0x08 => FriendsResult::Already,
            0x09 => FriendsResult::Yourself,
            0x0a => FriendsResult::Enemy,
            0x0b => FriendsResult::IgnoreFull,
            0x0c => FriendsResult::IgnoreSelf,
            0x0d => FriendsResult::IgnoreNotFound,
            0x0e => FriendsResult::IgnoreAlready,
            0x0f => FriendsResult::IgnoreAdded,
            0x10 => FriendsResult::IgnoreRemoved,
            0x11 => FriendsResult::IgnoreAmbiguous,
            other => FriendsResult::Unknown(other),
        }
    }

    /// **Does this result change the friends list?**
    ///
    /// It has to be asked, because nothing re-sends the list:
    /// `SMSG_FRIEND_LIST` arrives once at login and vmangos answers every later
    /// add, removal and status change with this one packet. So the client keeps
    /// its own copy and patches it, and this is what decides which copy.
    pub fn touches_friends(self) -> bool {
        matches!(
            self,
            FriendsResult::AddedOnline
                | FriendsResult::AddedOffline
                | FriendsResult::Removed
                | FriendsResult::Online
                | FriendsResult::Offline
        )
    }

    /// …and the same question for the ignore list.
    pub fn touches_ignores(self) -> bool {
        matches!(
            self,
            FriendsResult::IgnoreAdded | FriendsResult::IgnoreRemoved
        )
    }

    /// Does this result *remove* the named player from whichever list it is
    /// about? Three of the twenty do; the rest either add or restate.
    pub fn removes(self) -> bool {
        matches!(
            self,
            FriendsResult::Removed | FriendsResult::IgnoreRemoved
        )
    }

    /// The `GlobalStrings.lua` key this result prints in the chat frame, or
    /// `None` for one the reference says nothing about.
    ///
    /// **This is a reading, not a measurement.** The keys are the ones
    /// `SocialMgr.h` names in its own comments and the ones `GlobalStrings.lua`
    /// carries — every value from `0x00` to `0x11` has exactly one plausible key
    /// and the two lists line up — but the client's own table was not checked.
    /// If it is wrong the cost is bounded and visible: a
    /// sentence in the chat frame that says the wrong thing about an event that
    /// did happen.
    ///
    /// A key ending `_S` takes the player's name; `ERR_FRIEND_ONLINE_SS` takes
    /// it twice, which is how 1.12 builds a clickable player link out of one
    /// format string.
    pub fn message(self) -> Option<&'static str> {
        Some(match self {
            FriendsResult::DbError => "ERR_FRIEND_DB_ERROR",
            FriendsResult::ListFull => "ERR_FRIEND_LIST_FULL",
            FriendsResult::Online => "ERR_FRIEND_ONLINE_SS",
            FriendsResult::Offline => "ERR_FRIEND_OFFLINE_S",
            FriendsResult::NotFound => "ERR_FRIEND_NOT_FOUND",
            FriendsResult::Removed => "ERR_FRIEND_REMOVED_S",
            FriendsResult::AddedOnline | FriendsResult::AddedOffline => "ERR_FRIEND_ADDED_S",
            FriendsResult::Already => "ERR_FRIEND_ALREADY_S",
            FriendsResult::Yourself => "ERR_FRIEND_SELF",
            FriendsResult::Enemy => "ERR_FRIEND_WRONG_FACTION",
            FriendsResult::IgnoreFull => "ERR_IGNORE_FULL",
            FriendsResult::IgnoreSelf => "ERR_IGNORE_SELF",
            FriendsResult::IgnoreNotFound => "ERR_IGNORE_NOT_FOUND",
            FriendsResult::IgnoreAlready => "ERR_IGNORE_ALREADY_S",
            FriendsResult::IgnoreAdded => "ERR_IGNORE_ADDED_S",
            FriendsResult::IgnoreRemoved => "ERR_IGNORE_REMOVED_S",
            FriendsResult::IgnoreAmbiguous => "ERR_IGNORE_AMBIGUOUS",
            FriendsResult::Unknown(_) => return None,
        })
    }

    /// How many times a key wants the player's name.
    ///
    /// Two for `ERR_FRIEND_ONLINE_SS`, one for every other key ending `_S`, and
    /// none for the five that are whole sentences. Read off the key rather than
    /// stored beside it, because the suffix *is* the statement of its own arity
    /// — that is what it is for.
    pub fn name_slots(key: &str) -> usize {
        if key.ends_with("_SS") {
            2
        } else if key.ends_with("_S") {
            1
        } else {
            0
        }
    }
}

/// `SMSG_FRIEND_STATUS` (104): one answer about one player.
///
/// The `friend` half is only meaningful for [`FriendsResult::AddedOnline`] and
/// [`FriendsResult::Online`] — vmangos writes the online block for exactly those
/// two — so it is `None` for every other result rather than a zeroed row that
/// would read as an offline friend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendStatus {
    pub result: FriendsResult,
    pub guid: u64,
    pub friend: Option<Friend>,
}

/// `SMSG_FRIEND_STATUS` (104).
pub fn parse_friend_status(body: &[u8]) -> Option<FriendStatus> {
    let mut r = Reader::new(body);
    if !r.has(9) {
        return None;
    }
    let result = FriendsResult::from_code(r.u8());
    let guid = r.u64();
    // **The online block is decided by the result and not by what is left in the
    // body**, on vmangos' own branch: it writes the four extra fields for
    // exactly two results. Reading "is there more?" instead would take padding a
    // different server appends as a level.
    let friend = if matches!(result, FriendsResult::AddedOnline | FriendsResult::Online) {
        if r.has(13) {
            let status = r.u8();
            Some(Friend {
                guid,
                status,
                area: r.u32(),
                level: r.u32(),
                class: r.u32(),
            })
        } else {
            None
        }
    } else {
        None
    };
    Some(FriendStatus {
        result,
        guid,
        friend,
    })
}

/// One row of `SMSG_WHO` — every field spelled out, and no guid anywhere.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WhoRow {
    pub name: String,
    /// Empty for a player in no guild; vmangos writes the empty string rather
    /// than omitting the field.
    pub guild: String,
    pub level: u32,
    /// A `ChrClasses.dbc` id.
    pub class: u32,
    /// A `ChrRaces.dbc` id.
    pub race: u32,
    /// A zone id, for `AreaTable.dbc`.
    pub zone: u32,
}

/// `SMSG_WHO` (99): the rows, and the online total behind them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WhoResults {
    pub rows: Vec<WhoRow>,
    /// The server's whole matching population, which is **not** `rows.len()`
    /// once it passes 49. See the module note.
    pub online: u32,
}

/// How many rows the first allocation is sized for — vmangos stops at 49 and the
/// interface's own `MAX_WHOS_FROM_SERVER` is 50. A server that sends more is read
/// to the end anyway.
const WHO_ROW_CAP: usize = 50;

/// `SMSG_WHO` (99).
pub fn parse_who(body: &[u8]) -> Option<WhoResults> {
    let mut r = Reader::new(body);
    if !r.has(8) {
        return None;
    }
    let listed = r.u32() as usize;
    let online = r.u32();
    let mut rows = Vec::with_capacity(listed.min(WHO_ROW_CAP));
    for _ in 0..listed {
        // Two strings then four words. A row is variable-length, so the only
        // honest bound before reading the names is one byte apiece.
        if !r.has(2) {
            break;
        }
        let name = r.cstring();
        let guild = r.cstring();
        if !r.has(16) {
            break;
        }
        rows.push(WhoRow {
            name,
            guild,
            level: r.u32(),
            class: r.u32(),
            race: r.u32(),
            zone: r.u32(),
        });
    }
    Some(WhoResults { rows, online })
}

/// `CMSG_FRIEND_LIST` (102) — **no body at all**.
///
/// vmangos reads it as a `NullClientPacket`, so a body would be ignored rather
/// than refused; it is spelled as an empty vector so that the one door every
/// outbound packet goes through keeps its shape.
pub fn friend_list_body() -> Vec<u8> {
    Vec::new()
}

/// `CMSG_ADD_FRIEND` (105) — by **name**, because the client has no guid for
/// somebody it has not met. The server looks the name up in its own player cache
/// and answers `SMSG_FRIEND_STATUS` with a guid.
pub fn add_friend_body(name: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(name);
    w.buf
}

/// `CMSG_DEL_FRIEND` (106) — by **guid**, because by then the client has one.
/// The asymmetry is the server's: `HandleDelFriendOpcode` reads an `ObjectGuid`.
pub fn del_friend_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// `CMSG_ADD_IGNORE` (108) — by name, as [`add_friend_body`].
pub fn add_ignore_body(name: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(name);
    w.buf
}

/// `CMSG_DEL_IGNORE` (109) — by guid, as [`del_friend_body`].
pub fn del_ignore_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// Every race, every class. `0xFFFFFFFF` rather than the seven or nine bits that
/// exist, which is what the reference sends and what makes an unfiltered search
/// independent of how many races a build has.
pub const MASK_ALL: u32 = 0xFFFF_FFFF;

/// The server's own limits, restated so that [`who_body`] can enforce them
/// rather than have the request silently dropped: vmangos returns without
/// answering when either is exceeded, which at this end looks exactly like a
/// lost packet.
pub const MAX_WHO_ZONES: usize = 10;
pub const MAX_WHO_TERMS: usize = 4;

/// What `/who` asks for.
///
/// Every field is a filter and every one has a "match anything" value, which is
/// what an empty `/who` sends: the whole level range, no name, no guild, every
/// race, every class, no zone and no term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoRequest {
    pub level_min: u32,
    pub level_max: u32,
    pub name: String,
    pub guild: String,
    /// A bit per `ChrRaces.dbc` id. [`MASK_ALL`] for no filter.
    pub race_mask: u32,
    /// A bit per `ChrClasses.dbc` id. [`MASK_ALL`] for no filter.
    pub class_mask: u32,
    /// At most [`MAX_WHO_ZONES`].
    pub zones: Vec<u32>,
    /// At most [`MAX_WHO_TERMS`].
    pub terms: Vec<String>,
}

impl Default for WhoRequest {
    fn default() -> Self {
        WhoRequest {
            // **Zero, not one.** The client zeroes the bottom of the range and
            // sets the top to 100; the parser that produces one of these is
            // [`vale_assets::interface::whoquery`], and this default is its
            // default restated so the two cannot drift.
            level_min: 0,
            // vmangos raises anything at or above `MAX_LEVEL` to its own
            // ceiling, so 100 here is not a cap on what comes back.
            level_max: 100,
            name: String::new(),
            guild: String::new(),
            race_mask: MASK_ALL,
            class_mask: MASK_ALL,
            zones: Vec::new(),
            terms: Vec::new(),
        }
    }
}

/// `CMSG_WHO` (98).
///
/// The two lists are **truncated to the server's limits** rather than sent
/// whole. Exceeding either makes `HandleWhoOpcode` return without answering, and
/// an unanswered who is indistinguishable from a lost packet at this end — the
/// list simply never fills.
pub fn who_body(request: &WhoRequest) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(request.level_min);
    w.u32(request.level_max);
    w.cstring(&request.name);
    w.cstring(&request.guild);
    w.u32(request.race_mask);
    w.u32(request.class_mask);
    let zones = &request.zones[..request.zones.len().min(MAX_WHO_ZONES)];
    w.u32(zones.len() as u32);
    for zone in zones {
        w.u32(*zone);
    }
    let terms = &request.terms[..request.terms.len().min(MAX_WHO_TERMS)];
    w.u32(terms.len() as u32);
    for term in terms {
        w.cstring(term);
    }
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn friend_bytes(guid: u64, status: u8, online: Option<(u32, u32, u32)>) -> Vec<u8> {
        let mut w = Writer::new();
        w.u64(guid);
        w.u8(status);
        if let Some((area, level, class)) = online {
            w.u32(area);
            w.u32(level);
            w.u32(class);
        }
        w.buf
    }

    /// **An offline friend is nine bytes and an online one twenty-one**, which
    /// is the whole of why this list cannot be read at a fixed stride.
    #[test]
    fn a_friend_list_mixes_two_row_widths() {
        let mut body = vec![2u8];
        body.extend(friend_bytes(0x1122, 0, None));
        body.extend(friend_bytes(0x3344, 2, Some((12, 60, 1))));
        let list = parse_friend_list(&body).expect("a body with a count parses");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].guid, 0x1122);
        assert!(!list[0].online());
        assert_eq!(list[0].level, 0);
        assert_eq!(list[1].guid, 0x3344);
        assert!(list[1].online());
        assert_eq!((list[1].area, list[1].level, list[1].class), (12, 60, 1));
    }

    /// A truncated tail keeps what was read rather than discarding the packet —
    /// this crate's own rule about damaged bodies.
    #[test]
    fn a_short_friend_list_keeps_what_it_read() {
        let mut body = vec![3u8];
        body.extend(friend_bytes(0x11, 0, None));
        body.extend([0u8; 4]);
        let list = parse_friend_list(&body).expect("the count is there");
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn an_ignore_list_is_guids() {
        let mut w = Writer::new();
        w.u8(2).u64(7).u64(9);
        assert_eq!(parse_ignore_list(&w.buf), Some(vec![7, 9]));
    }

    /// **The online block is gated on the result, not on the body's length.** A
    /// `Removed` carrying trailing bytes must not be read as an online friend —
    /// see the note on [`parse_friend_status`].
    #[test]
    fn only_the_two_online_results_carry_a_row() {
        let mut w = Writer::new();
        w.u8(0x06).u64(42).u8(1).u32(12).u32(60).u32(4);
        let added = parse_friend_status(&w.buf).expect("added-online parses");
        assert_eq!(added.result, FriendsResult::AddedOnline);
        assert_eq!(added.friend.map(|f| f.level), Some(60));

        let mut w = Writer::new();
        w.u8(0x05).u64(42).u8(1).u32(12).u32(60).u32(4);
        let removed = parse_friend_status(&w.buf).expect("removed parses");
        assert_eq!(removed.result, FriendsResult::Removed);
        assert_eq!(removed.friend, None);
    }

    /// An unknown result byte is data rather than a failure, and it prints
    /// nothing rather than printing the wrong thing.
    #[test]
    fn an_unknown_result_is_carried_and_says_nothing() {
        let mut w = Writer::new();
        w.u8(0x16).u64(1);
        let status = parse_friend_status(&w.buf).expect("carried");
        assert_eq!(status.result, FriendsResult::Unknown(0x16));
        assert_eq!(status.result.message(), None);
    }

    /// The name arity is read off the key's own suffix.
    #[test]
    fn the_key_states_how_many_names_it_takes() {
        assert_eq!(FriendsResult::name_slots("ERR_FRIEND_ONLINE_SS"), 2);
        assert_eq!(FriendsResult::name_slots("ERR_FRIEND_ADDED_S"), 1);
        assert_eq!(FriendsResult::name_slots("ERR_FRIEND_SELF"), 0);
    }

    /// **The two leading words are different numbers**, and a reader that takes
    /// one of them draws the wrong total under the list.
    #[test]
    fn a_who_reply_keeps_the_online_total_apart_from_the_rows() {
        let mut w = Writer::new();
        w.u32(1).u32(273);
        w.cstring("Bram").cstring("The Watch");
        w.u32(60).u32(1).u32(1).u32(12);
        let results = parse_who(&w.buf).expect("a who body parses");
        assert_eq!(results.online, 273);
        assert_eq!(results.rows.len(), 1);
        assert_eq!(results.rows[0].name, "Bram");
        assert_eq!(results.rows[0].guild, "The Watch");
        assert_eq!(results.rows[0].zone, 12);
    }

    /// A guildless player is an empty string on the wire, not a missing field.
    #[test]
    fn a_guildless_who_row_reads_as_an_empty_guild() {
        let mut w = Writer::new();
        w.u32(1).u32(1);
        w.cstring("Solo").cstring("");
        w.u32(5).u32(2).u32(3).u32(4);
        let results = parse_who(&w.buf).expect("parses");
        assert_eq!(results.rows[0].guild, "");
        assert_eq!(results.rows[0].level, 5);
    }

    /// The request's two lists are cut to the server's limits, because going
    /// over either of them is answered with silence.
    #[test]
    fn a_who_request_is_cut_to_the_servers_limits() {
        let request = WhoRequest {
            zones: (0..20).collect(),
            terms: (0..9).map(|i| format!("t{i}")).collect(),
            ..WhoRequest::default()
        };
        let body = who_body(&request);
        let mut r = Reader::new(&body);
        r.u32();
        r.u32();
        r.cstring();
        r.cstring();
        r.u32();
        r.u32();
        assert_eq!(r.u32() as usize, MAX_WHO_ZONES);
        for _ in 0..MAX_WHO_ZONES {
            r.u32();
        }
        assert_eq!(r.u32() as usize, MAX_WHO_TERMS);
    }

    /// An empty search filters on nothing, which is what an unadorned `/who`
    /// sends.
    #[test]
    fn an_empty_search_filters_on_nothing() {
        let body = who_body(&WhoRequest::default());
        let mut r = Reader::new(&body);
        assert_eq!(r.u32(), 0);
        assert_eq!(r.u32(), 100);
        assert_eq!(r.cstring(), "");
        assert_eq!(r.cstring(), "");
        assert_eq!(r.u32(), MASK_ALL);
        assert_eq!(r.u32(), MASK_ALL);
        assert_eq!(r.u32(), 0);
        assert_eq!(r.u32(), 0);
        assert_eq!(r.remaining(), 0);
    }

    /// The two additions are names and the two removals are guids — the
    /// asymmetry is the server's, and getting it backwards is a packet vmangos
    /// silently drops.
    #[test]
    fn adding_is_by_name_and_removing_is_by_guid() {
        assert_eq!(add_friend_body("Bram"), b"Bram\0".to_vec());
        assert_eq!(add_ignore_body("Bram"), b"Bram\0".to_vec());
        assert_eq!(del_friend_body(0x0102), 0x0102u64.to_le_bytes().to_vec());
        assert_eq!(del_ignore_body(0x0102), 0x0102u64.to_le_bytes().to_vec());
        assert!(friend_list_body().is_empty());
    }
}
