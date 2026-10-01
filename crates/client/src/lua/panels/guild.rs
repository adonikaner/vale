//! The guild tab's C functions, and the roster, ranks and guild names behind
//! them.
//!
//! ```text
//! IsInGuild()  IsGuildLeader()            the character's own membership
//! GetGuildInfo(unit)                      guild name, rank name, rank index
//!
//! GetNumGuildMembers([offline])           the rows the roster shows
//! GetGuildRosterInfo(i)                   one row, ten returns
//! GetGuildRosterLastOnline(i)             years, months, days, hours
//! GetGuildRosterSelection()  SetGuildRosterSelection(i)
//! GetGuildRosterShowOffline()  SetGuildRosterShowOffline(flag)
//! SortGuildRoster(key)
//! GetGuildRosterMOTD()  GetGuildInfoText()
//!
//! CanGuildInvite()  CanGuildRemove()  CanGuildPromote()  CanGuildDemote()
//! CanEditMOTD()  CanEditPublicNote()  CanEditOfficerNote()
//! CanViewOfficerNote()  CanEditGuildInfo()
//!
//! GuildRoster()  GuildInfo()              ask the server
//! GuildInviteByName(n)  GuildUninviteByName(n)  GuildPromoteByName(n)
//! GuildDemoteByName(n)  GuildSetLeaderByName(n)
//! GuildSetMOTD(text)  SetGuildInfoText(text)
//! GuildRosterSetPublicNote(i, text)  GuildRosterSetOfficerNote(i, text)
//! GuildLeave()  GuildDisband()  AcceptGuild()  DeclineGuild()
//!
//! GuildControlGetNumRanks()  GuildControlGetRankName(i)
//! GuildControlSetRank(i)  GuildControlGetRankFlags()
//! GuildControlSetRankFlag(id, checked)  GuildControlSaveRank(name)
//! GuildControlAddRank(name)  GuildControlDelRank(name)
//! ```
//!
//! ## The state is held, and only the requests are queued
//!
//! `GuildStatus_Update` calls `SetGuildRosterSelection` and reads
//! `GetGuildRosterSelection` and `GetGuildRosterInfo` in the same handler, and
//! the guild-control window ticks a checkbox and reads the thirteen flags
//! back before the frame ends. A write applied by a Bevy system on the next
//! frame would be read before it landed. So every read and write here goes
//! through one [`Guild`] board, and what queues is the list of
//! [`GuildVerb`]s for the server, which [`crate::interface::guild`] drains.
//!
//! ## What the roster's rows are
//!
//! `GetNumGuildMembers` counts the rows the roster shows, which leaves out
//! offline members while the *Show Offline Members* box is clear. Every index
//! the interface passes is a row of that view, in the order
//! `SortGuildRoster` last set. The selection is kept as the member's guid, so
//! a roster that is re-read or re-sorted keeps the same member selected; the
//! index answered is that member's current row, or 0 when the row is hidden.
//!
//! ## `GetGuildInfo` needs a name the roster does not carry
//!
//! A unit's guild is the id in its `PLAYER_GUILDID` field. The name and the
//! rank names come from `SMSG_GUILD_QUERY_RESPONSE`. `GetGuildInfo` for a
//! guild this client has no answer for records the id, and
//! [`crate::interface::guild`] sends the query. Until the answer arrives the
//! two names are nil and the rank index, which is the unit's own field, is
//! still answered: `GuildStatus_Update` compares it with a number, and
//! `PaperDollFrame_SetGuild` hides its line while the name is nil. That
//! function runs again on `PLAYER_GUILD_UPDATE`, which is raised when the
//! answer arrives.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use vale_assets::tables::area::Areas;
use vale_assets::tables::charcreate::CharCreate;
use vale_protocol::play::guild::{presence, rights, GuildQuery, Member, Roster};
use vale_protocol::socket::session::GuildVerb;

use super::super::api::Answers;

/// The unscoped functions this file registers, sorted. Checked against
/// [`register`] by `every_verb_is_registered`.
pub const VERBS: [&str; 44] = [
    "AcceptGuild",
    "CanEditGuildInfo",
    "CanEditMOTD",
    "CanEditOfficerNote",
    "CanEditPublicNote",
    "CanGuildDemote",
    "CanGuildInvite",
    "CanGuildPromote",
    "CanGuildRemove",
    "CanViewOfficerNote",
    "DeclineGuild",
    "GetGuildInfoText",
    "GetGuildRosterInfo",
    "GetGuildRosterLastOnline",
    "GetGuildRosterMOTD",
    "GetGuildRosterSelection",
    "GetGuildRosterShowOffline",
    "GetNumGuildMembers",
    "GuildControlAddRank",
    "GuildControlDelRank",
    "GuildControlGetNumRanks",
    "GuildControlGetRankFlags",
    "GuildControlGetRankName",
    "GuildControlSaveRank",
    "GuildControlSetRank",
    "GuildControlSetRankFlag",
    "GuildDemoteByName",
    "GuildDisband",
    "GuildInfo",
    "GuildInviteByName",
    "GuildLeave",
    "GuildPromoteByName",
    "GuildRoster",
    "GuildRosterSetOfficerNote",
    "GuildRosterSetPublicNote",
    "GuildSetLeaderByName",
    "GuildSetMOTD",
    "GuildUninviteByName",
    "IsGuildLeader",
    "IsInGuild",
    "SetGuildInfoText",
    "SetGuildRosterSelection",
    "SetGuildRosterShowOffline",
    "SortGuildRoster",
];

/// The scoped read, for the interface census.
pub const READS: [&str; 1] = ["GetGuildInfo"];

/// What `GetGuildInfo` needs from the world.
pub trait GuildAnswers {
    /// The guild id and rank index of the unit `token` names: its
    /// `PLAYER_GUILDID` and `PLAYER_GUILDRANK`. `None` for a unit that is
    /// not a player, is in no guild, or does not exist.
    fn unit_guild(&self, _token: &str) -> Option<(u32, u32)> {
        None
    }
}

impl GuildAnswers for super::super::api::Live<'_, '_, '_> {
    fn unit_guild(&self, token: &str) -> Option<(u32, u32)> {
        use vale_protocol::state::fields::player;
        let id = crate::interface::api::UnitId::parse(token)?;
        let unit = self.units.get(id)?;
        if unit.kind != vale_protocol::state::update::ObjectType::Player {
            return None;
        }
        let world = self.world.as_ref()?;
        let world = world.lock().unwrap_or_else(|e| e.into_inner());
        let object = world.get(unit.guid)?;
        let guild = object.field(player::GUILDID).filter(|id| *id != 0)?;
        Some((guild, object.field(player::GUILDRANK).unwrap_or(0)))
    }
}

/// The column `SortGuildRoster` orders the roster by. The names are the
/// `sortType` values the column headers in `FriendsFrame.xml` pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Zone,
    Level,
    Class,
    Rank,
    Note,
    Online,
}

impl SortKey {
    fn named(key: &str) -> Option<SortKey> {
        Some(match key {
            "name" => SortKey::Name,
            "zone" => SortKey::Zone,
            "level" => SortKey::Level,
            "class" => SortKey::Class,
            "rank" => SortKey::Rank,
            "note" => SortKey::Note,
            "online" => SortKey::Online,
            _ => return None,
        })
    }
}

/// The guild state the interface reads and writes, shared between the
/// interpreter and the ECS.
pub struct Guild {
    /// The character's `PLAYER_GUILDID`, 0 for no guild, and
    /// `PLAYER_GUILDRANK`. Written by [`crate::interface::guild`].
    guild_id: u32,
    rank: u32,
    /// Every `SMSG_GUILD_QUERY_RESPONSE` received, by guild id.
    guilds: HashMap<u32, GuildQuery>,
    /// Guild ids asked for, so each is queried once.
    asked: HashSet<u32>,
    /// Guild ids to query, drained by [`Self::take_wanted`].
    wanted: Vec<u32>,
    /// `SMSG_GUILD_ROSTER`, in the order the server sent it.
    motd: String,
    info: String,
    rank_rights: Vec<u32>,
    members: Vec<Member>,
    /// Whether offline members are rows. The interface sets it from the saved
    /// variable `SHOW_OFFLINE_GUILD_MEMBERS`, whose shipped value is 1.
    show_offline: bool,
    /// The sort column, and whether the order is reversed. `None` keeps the
    /// server's order.
    sort: Option<(SortKey, bool)>,
    /// Indices into `members`: the rows shown, in order.
    view: Vec<usize>,
    /// The selected member's guid.
    selected: Option<u64>,
    /// The rank the guild-control window is editing, one-based, and the
    /// rights its checkboxes hold. `GuildControlSaveRank` sends them.
    control_rank: usize,
    control_rights: u32,
    /// `ChrClasses.dbc` and `AreaTable.dbc`, for a member's class and zone
    /// names, and `GlobalStrings.lua`.
    pub classes: Option<Arc<CharCreate>>,
    pub areas: Option<Arc<Areas>>,
    pub strings: Option<Arc<vale_assets::interface::strings::Strings>>,
    /// Incremented when the roster or its view changes.
    pub roster_version: u32,
    /// Incremented when the character's membership changes or a guild's name
    /// arrives, which are the two things `GetGuildInfo` answers from.
    pub membership_version: u32,
}

impl Default for Guild {
    fn default() -> Guild {
        Guild {
            guild_id: 0,
            rank: 0,
            guilds: HashMap::new(),
            asked: HashSet::new(),
            wanted: Vec::new(),
            motd: String::new(),
            info: String::new(),
            rank_rights: Vec::new(),
            members: Vec::new(),
            show_offline: true,
            sort: None,
            view: Vec::new(),
            selected: None,
            control_rank: 1,
            control_rights: 0,
            classes: None,
            areas: None,
            strings: None,
            roster_version: 0,
            membership_version: 0,
        }
    }
}

impl Guild {
    /// The character's guild id and rank, from its own update fields.
    /// Leaving a guild drops the roster, which belonged to that guild.
    pub fn set_membership(&mut self, guild: u32, rank: u32) {
        if (self.guild_id, self.rank) == (guild, rank) {
            return;
        }
        if self.guild_id != guild {
            self.set_roster(Roster::default());
            self.selected = None;
        }
        self.guild_id = guild;
        self.rank = rank;
        self.want(guild);
        self.membership_version = self.membership_version.wrapping_add(1);
    }

    pub fn guild_id(&self) -> u32 {
        self.guild_id
    }

    /// The character's rank index, 0 for the guild master.
    pub fn rank(&self) -> u32 {
        self.rank
    }

    /// The saved emblem of the character's guild, when its query has
    /// answered. The tabard designer starts its design from it.
    pub fn own_emblem(&self) -> Option<vale_protocol::play::guild::Emblem> {
        self.guilds.get(&self.guild_id).map(|guild| guild.emblem)
    }

    /// `SMSG_GUILD_QUERY_RESPONSE`.
    pub fn note_query(&mut self, query: GuildQuery) {
        self.guilds.insert(query.id, query);
        self.membership_version = self.membership_version.wrapping_add(1);
    }

    /// Record that a guild's name is needed. Each id is asked for once.
    fn want(&mut self, guild: u32) {
        if guild != 0 && !self.guilds.contains_key(&guild) && self.asked.insert(guild) {
            self.wanted.push(guild);
        }
    }

    /// The guild ids to query, emptied.
    pub fn take_wanted(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.wanted)
    }

    /// The name of the character's guild, when its query has answered.
    pub fn own_name(&self) -> Option<&str> {
        self.guilds.get(&self.guild_id).map(|g| g.name.as_str())
    }

    /// `SMSG_GUILD_ROSTER`: the whole roster, replacing the one held.
    pub fn set_roster(&mut self, roster: Roster) {
        self.motd = roster.motd;
        self.info = roster.info;
        self.rank_rights = roster.rank_rights;
        self.members = roster.members;
        self.rebuild_view();
    }

    /// The message of the day, which also arrives alone in an
    /// `SMSG_GUILD_EVENT`.
    pub fn set_motd(&mut self, motd: String) {
        self.motd = motd;
    }

    /// `GlobalStrings.lua`'s value for `key`, or `fallback`.
    pub fn string(&self, key: &str, fallback: &str) -> String {
        self.strings
            .as_ref()
            .and_then(|s| s.get(key))
            .unwrap_or(fallback)
            .to_string()
    }

    /// Apply the offline filter and the sort to `members`.
    fn rebuild_view(&mut self) {
        let mut view: Vec<usize> = (0..self.members.len())
            .filter(|i| self.show_offline || self.members[*i].online())
            .collect();
        if let Some((key, reversed)) = self.sort {
            // The names are resolved once per member, not once per compare.
            let words: Vec<String> = match key {
                SortKey::Zone => self.members.iter().map(|m| self.zone_name(m.zone)).collect(),
                SortKey::Class => self.members.iter().map(|m| self.class_name(m.class)).collect(),
                _ => Vec::new(),
            };
            let members = &self.members;
            // A stable sort, so members equal in the sorted column keep the
            // order they had.
            view.sort_by(|a, b| {
                let (x, y) = (&members[*a], &members[*b]);
                let order = match key {
                    SortKey::Name => x.name.to_lowercase().cmp(&y.name.to_lowercase()),
                    SortKey::Zone | SortKey::Class => words[*a].cmp(&words[*b]),
                    SortKey::Level => y.level.cmp(&x.level),
                    SortKey::Rank => x.rank.cmp(&y.rank),
                    SortKey::Note => x.note.to_lowercase().cmp(&y.note.to_lowercase()),
                    // Online members first, then the most recently seen.
                    SortKey::Online => y
                        .online()
                        .cmp(&x.online())
                        .then(x.days_offline.total_cmp(&y.days_offline)),
                };
                if reversed {
                    order.reverse()
                } else {
                    order
                }
            });
        }
        self.view = view;
        self.roster_version = self.roster_version.wrapping_add(1);
    }

    /// The member at a one-based row of the view.
    fn row(&self, index: usize) -> Option<&Member> {
        self.members.get(*self.view.get(index.checked_sub(1)?)?)
    }

    /// The selected member's one-based row, or 0.
    fn selection(&self) -> usize {
        let Some(guid) = self.selected else { return 0 };
        self.view
            .iter()
            .position(|i| self.members[*i].guid == guid)
            .map_or(0, |row| row + 1)
    }

    /// How many ranks the guild has: the roster's count, or the query's
    /// before a roster has arrived.
    fn rank_count(&self) -> usize {
        if !self.rank_rights.is_empty() {
            return self.rank_rights.len();
        }
        self.guilds.get(&self.guild_id).map_or(0, |g| g.ranks.len())
    }

    /// A rank's name in a guild, by rank index.
    fn rank_name(&self, guild: u32, rank: u32) -> Option<&str> {
        self.guilds
            .get(&guild)?
            .ranks
            .get(rank as usize)
            .map(String::as_str)
    }

    /// Whether the character's rank holds a right. False before the roster,
    /// which carries the rights, has arrived.
    fn can(&self, right: u32) -> bool {
        self.guild_id != 0
            && self
                .rank_rights
                .get(self.rank as usize)
                .is_some_and(|held| held & right != 0)
    }

    fn class_name(&self, class: u8) -> String {
        self.classes
            .as_ref()
            .and_then(|create| create.every_class().find(|c| c.id == class).map(|c| c.name.clone()))
            .unwrap_or_else(|| self.string("UNKNOWN", "Unknown"))
    }

    /// A zone id as the enclosing zone's name.
    fn zone_name(&self, zone: u32) -> String {
        self.areas
            .as_ref()
            .map(|areas| areas.zone_name(zone))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.string("UNKNOWN", "Unknown"))
    }

    /// `""`, `CHAT_FLAG_AFK` or `CHAT_FLAG_DND` for an online member.
    fn status_word(&self, flags: u8) -> String {
        if flags & presence::DND != 0 {
            self.string("CHAT_FLAG_DND", "<DND>")
        } else if flags & presence::AFK != 0 {
            self.string("CHAT_FLAG_AFK", "<AFK>")
        } else {
            String::new()
        }
    }
}

/// A member's time offline as whole years, months, days and hours, taking a
/// year as 365 days and a month as 30.
fn last_online(days: f32) -> (u32, u32, u32, u32) {
    let days = days.max(0.0);
    let whole = days.floor() as u32;
    let hours = ((days - days.floor()) * 24.0).floor() as u32;
    (whole / 365, (whole % 365) / 30, (whole % 365) % 30, hours)
}

pub type Held = Rc<RefCell<Guild>>;
pub type Queue = Rc<RefCell<Vec<GuildVerb>>>;

/// The interface's boolean: nil and `false` are false, and so is the number
/// 0, which is what `SHOW_OFFLINE_GUILD_MEMBERS` holds when the box is clear.
fn truthy(value: &mlua::Value) -> bool {
    match value {
        mlua::Value::Nil | mlua::Value::Boolean(false) => false,
        mlua::Value::Integer(i) => *i != 0,
        mlua::Value::Number(n) => *n != 0.0,
        _ => true,
    }
}

fn one_or_nil(flag: bool) -> Option<i64> {
    flag.then_some(1)
}

fn trimmed(name: Option<String>) -> Option<String> {
    let name = name?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Register the forty-four unscoped functions. The board is also kept in the
/// interpreter's app data, where the scoped `GetGuildInfo` finds it.
pub(in crate::lua) fn register(lua: &mlua::Lua, held: &Held, queue: &Queue) -> mlua::Result<()> {
    let globals = lua.globals();
    lua.set_app_data(Rc::clone(held));

    // A read of the board that answers 1 or nil.
    let flag = |name: &str, test: fn(&Guild) -> bool| -> mlua::Result<()> {
        let get = Rc::clone(held);
        globals.set(
            name,
            lua.create_function(move |_, ()| Ok(one_or_nil(test(&get.borrow()))))?,
        )
    };
    flag("IsInGuild", |g| g.guild_id != 0)?;
    // Rank 0 is the guild master.
    flag("IsGuildLeader", |g| g.guild_id != 0 && g.rank == 0)?;
    flag("CanGuildInvite", |g| g.can(rights::INVITE))?;
    flag("CanGuildRemove", |g| g.can(rights::REMOVE))?;
    flag("CanGuildPromote", |g| g.can(rights::PROMOTE))?;
    flag("CanGuildDemote", |g| g.can(rights::DEMOTE))?;
    flag("CanEditMOTD", |g| g.can(rights::SET_MOTD))?;
    flag("CanEditPublicNote", |g| g.can(rights::EDIT_PUBLIC_NOTE))?;
    flag("CanEditOfficerNote", |g| g.can(rights::EDIT_OFFICER_NOTE))?;
    flag("CanViewOfficerNote", |g| g.can(rights::VIEW_OFFICER_NOTE))?;
    flag("CanEditGuildInfo", |g| g.can(rights::MODIFY_INFO))?;
    flag("GetGuildRosterShowOffline", |g| g.show_offline)?;

    // The rows shown. A true argument counts every member, offline included.
    let get = Rc::clone(held);
    globals.set(
        "GetNumGuildMembers",
        lua.create_function(move |_, all: mlua::Value| {
            let board = get.borrow();
            Ok(if truthy(&all) {
                board.members.len()
            } else {
                board.view.len()
            })
        })?,
    )?;

    // `(name, rank, rankIndex, level, class, zone, note, officernote, online,
    // status)`. Nothing for a row that does not exist: `GuildStatus_Update`
    // reads thirteen rows whatever the count and hides the empty ones.
    let get = Rc::clone(held);
    globals.set(
        "GetGuildRosterInfo",
        lua.create_function(move |lua, index: Option<usize>| {
            let board = get.borrow();
            let Some(member) = index.and_then(|i| board.row(i)) else {
                return Ok(mlua::Variadic::new());
            };
            let text = |s: &str| lua.create_string(s).map(mlua::Value::String);
            let rank = board.rank_name(board.guild_id, member.rank).unwrap_or_default();
            Ok(mlua::Variadic::from(vec![
                text(&member.name)?,
                text(rank)?,
                mlua::Value::Number(f64::from(member.rank)),
                mlua::Value::Number(f64::from(member.level)),
                text(&board.class_name(member.class))?,
                text(&board.zone_name(member.zone))?,
                text(&member.note)?,
                text(&member.officer_note)?,
                if member.online() {
                    mlua::Value::Number(1.0)
                } else {
                    mlua::Value::Nil
                },
                text(&board.status_word(member.presence))?,
            ]))
        })?,
    )?;

    // Nothing for an online member or a row that does not exist;
    // `GuildFrame_GetLastOnline` is called for offline rows only.
    let get = Rc::clone(held);
    globals.set(
        "GetGuildRosterLastOnline",
        lua.create_function(move |_, index: Option<usize>| {
            let board = get.borrow();
            Ok(match index.and_then(|i| board.row(i)) {
                Some(member) if !member.online() => {
                    let (years, months, days, hours) = last_online(member.days_offline);
                    mlua::Variadic::from(vec![years, months, days, hours])
                }
                _ => mlua::Variadic::new(),
            })
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetGuildRosterSelection",
        lua.create_function(move |_, ()| Ok(get.borrow().selection()))?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "SetGuildRosterSelection",
        lua.create_function(move |_, index: Option<usize>| {
            let mut board = get.borrow_mut();
            board.selected = index.and_then(|i| board.row(i)).map(|member| member.guid);
            Ok(())
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "SetGuildRosterShowOffline",
        lua.create_function(move |_, show: mlua::Value| {
            let mut board = get.borrow_mut();
            let show = truthy(&show);
            if board.show_offline != show {
                board.show_offline = show;
                board.rebuild_view();
            }
            Ok(())
        })?,
    )?;

    // Sorting by the column already sorted by reverses the order.
    let get = Rc::clone(held);
    globals.set(
        "SortGuildRoster",
        lua.create_function(move |_, key: Option<String>| {
            let Some(key) = key.as_deref().and_then(SortKey::named) else {
                return Ok(());
            };
            let mut board = get.borrow_mut();
            board.sort = Some(match board.sort {
                Some((held, reversed)) if held == key => (key, !reversed),
                _ => (key, false),
            });
            board.rebuild_view();
            Ok(())
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetGuildRosterMOTD",
        lua.create_function(move |lua, ()| lua.create_string(&get.borrow().motd))?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetGuildInfoText",
        lua.create_function(move |lua, ()| lua.create_string(&get.borrow().info))?,
    )?;

    // --- the guild-control window -----------------------------------------

    let get = Rc::clone(held);
    globals.set(
        "GuildControlGetNumRanks",
        lua.create_function(move |_, ()| Ok(get.borrow().rank_count()))?,
    )?;

    // One-based. The empty string for a rank the guild does not have.
    let get = Rc::clone(held);
    globals.set(
        "GuildControlGetRankName",
        lua.create_function(move |lua, index: Option<usize>| {
            let board = get.borrow();
            let name = index
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| board.rank_name(board.guild_id, i as u32))
                .unwrap_or_default();
            lua.create_string(name)
        })?,
    )?;

    // Choose the rank to edit, and load its rights into the checkboxes.
    let get = Rc::clone(held);
    globals.set(
        "GuildControlSetRank",
        lua.create_function(move |_, index: Option<usize>| {
            let mut board = get.borrow_mut();
            let index = index.unwrap_or(1).max(1);
            board.control_rank = index;
            board.control_rights = board.rank_rights.get(index - 1).copied().unwrap_or(0);
            Ok(())
        })?,
    )?;

    // Thirteen values, 1 or nil, in the checkboxes' order.
    let get = Rc::clone(held);
    globals.set(
        "GuildControlGetRankFlags",
        lua.create_function(move |_, ()| {
            let held = get.borrow().control_rights;
            Ok(mlua::Variadic::from_iter(
                rights::CONTROL_ORDER.iter().map(|bit| one_or_nil(held & bit != 0)),
            ))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GuildControlSetRankFlag",
        lua.create_function(move |_, (id, checked): (Option<usize>, mlua::Value)| {
            let Some(bit) = id
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| rights::CONTROL_ORDER.get(i))
            else {
                return Ok(());
            };
            let mut board = get.borrow_mut();
            if truthy(&checked) {
                board.control_rights |= bit;
            } else {
                board.control_rights &= !bit;
            }
            Ok(())
        })?,
    )?;

    // Send the edited rank. The server answers with a new query response and
    // a new roster, which replace what is held.
    let get = Rc::clone(held);
    let send = Rc::clone(queue);
    globals.set(
        "GuildControlSaveRank",
        lua.create_function(move |_, name: Option<String>| {
            let board = get.borrow();
            send.borrow_mut().push(GuildVerb::Rank {
                rank: (board.control_rank - 1) as u32,
                rights: board.control_rights,
                name: name.unwrap_or_default(),
            });
            Ok(())
        })?,
    )?;

    let send = Rc::clone(queue);
    globals.set(
        "GuildControlAddRank",
        lua.create_function(move |_, name: Option<String>| {
            if let Some(name) = trimmed(name) {
                send.borrow_mut().push(GuildVerb::AddRank(name));
            }
            Ok(())
        })?,
    )?;

    // The interface passes the lowest rank's name. The request names no rank:
    // the server deletes the lowest.
    let send = Rc::clone(queue);
    globals.set(
        "GuildControlDelRank",
        lua.create_function(move |_, _name: mlua::Value| {
            send.borrow_mut().push(GuildVerb::DelRank);
            Ok(())
        })?,
    )?;

    // --- the requests -------------------------------------------------------

    let plain = |name: &str, verb: fn() -> GuildVerb| -> mlua::Result<()> {
        let send = Rc::clone(queue);
        globals.set(
            name,
            lua.create_function(move |_, ()| {
                send.borrow_mut().push(verb());
                Ok(())
            })?,
        )
    };
    plain("GuildRoster", || GuildVerb::Roster)?;
    plain("GuildInfo", || GuildVerb::Info)?;
    plain("GuildLeave", || GuildVerb::Leave)?;
    plain("GuildDisband", || GuildVerb::Disband)?;
    plain("AcceptGuild", || GuildVerb::Accept)?;
    plain("DeclineGuild", || GuildVerb::Decline)?;

    // A request about one member, by name. An empty name sends nothing.
    let named = |name: &str, verb: fn(String) -> GuildVerb| -> mlua::Result<()> {
        let send = Rc::clone(queue);
        globals.set(
            name,
            lua.create_function(move |_, player: Option<String>| {
                if let Some(player) = trimmed(player) {
                    send.borrow_mut().push(verb(player));
                }
                Ok(())
            })?,
        )
    };
    named("GuildInviteByName", GuildVerb::Invite)?;
    named("GuildUninviteByName", GuildVerb::Remove)?;
    named("GuildPromoteByName", GuildVerb::Promote)?;
    named("GuildDemoteByName", GuildVerb::Demote)?;
    named("GuildSetLeaderByName", GuildVerb::Leader)?;

    // An empty message is sent: it clears the message of the day.
    let send = Rc::clone(queue);
    globals.set(
        "GuildSetMOTD",
        lua.create_function(move |_, text: Option<String>| {
            send.borrow_mut().push(GuildVerb::Motd(text.unwrap_or_default()));
            Ok(())
        })?,
    )?;

    // The server sends nothing back for this request, so the held text is
    // replaced here. `GuildInfoFrame` calls `GuildRoster()` after it, and the
    // roster that answers carries the text the server stored.
    let get = Rc::clone(held);
    let send = Rc::clone(queue);
    globals.set(
        "SetGuildInfoText",
        lua.create_function(move |_, text: Option<String>| {
            let text = text.unwrap_or_default();
            get.borrow_mut().info = text.clone();
            send.borrow_mut().push(GuildVerb::InfoText(text));
            Ok(())
        })?,
    )?;

    // The two notes take a row; the request takes the member's name.
    let note = |name: &str, verb: fn(String, String) -> GuildVerb| -> mlua::Result<()> {
        let get = Rc::clone(held);
        let send = Rc::clone(queue);
        globals.set(
            name,
            lua.create_function(move |_, (index, text): (Option<usize>, Option<String>)| {
                let board = get.borrow();
                if let Some(member) = index.and_then(|i| board.row(i)) {
                    send.borrow_mut()
                        .push(verb(member.name.clone(), text.unwrap_or_default()));
                }
                Ok(())
            })?,
        )
    };
    note("GuildRosterSetPublicNote", |player, note| {
        GuildVerb::PublicNote { player, note }
    })?;
    note("GuildRosterSetOfficerNote", |player, note| {
        GuildVerb::OfficerNote { player, note }
    })?;

    Ok(())
}

/// `GetGuildInfo(unit)`: the guild's name, the unit's rank name and its rank
/// index. Nothing for a unit in no guild. For a guild whose name has not
/// arrived the two names are nil. See the module note.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    globals.set(
        "GetGuildInfo",
        scope.create_function(move |lua, token: Option<String>| {
            let token = token.unwrap_or_default().to_ascii_lowercase();
            let (Some((guild, rank)), Some(held)) =
                (answers.unit_guild(&token), lua.app_data_ref::<Held>())
            else {
                return Ok(mlua::Variadic::new());
            };
            let mut board = held.borrow_mut();
            let Some(query) = board.guilds.get(&guild) else {
                board.want(guild);
                return Ok(mlua::Variadic::from(vec![
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Number(f64::from(rank)),
                ]));
            };
            let rank_name = query.ranks.get(rank as usize).map_or("", String::as_str);
            Ok(mlua::Variadic::from(vec![
                mlua::Value::String(lua.create_string(&query.name)?),
                mlua::Value::String(lua.create_string(rank_name)?),
                mlua::Value::Number(f64::from(rank)),
            ]))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(guid: u64, name: &str, rank: u32, level: u8, online: bool) -> Member {
        Member {
            guid,
            presence: if online { presence::ONLINE } else { 0 },
            name: name.into(),
            rank,
            level,
            class: 1,
            zone: 12,
            days_offline: if online { 0.0 } else { 400.5 },
            note: format!("{name}'s note"),
            officer_note: String::new(),
        }
    }

    fn registered() -> (Held, Queue, mlua::Lua) {
        let lua = mlua::Lua::new();
        let held: Held = Rc::default();
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &held, &queue).expect("registers");
        {
            let mut board = held.borrow_mut();
            board.set_membership(7, 1);
            board.note_query(GuildQuery {
                id: 7,
                name: "The Watch".into(),
                ranks: vec!["Guild Master".into(), "Officer".into(), "Member".into()],
                ..GuildQuery::default()
            });
            board.set_roster(Roster {
                motd: "Raid at eight".into(),
                info: "We raid.".into(),
                rank_rights: vec![
                    0x000f_f1ff,
                    rights::EMPTY | rights::INVITE | rights::SET_MOTD,
                    rights::EMPTY,
                ],
                members: vec![
                    member(1, "Cade", 0, 60, true),
                    member(2, "Bram", 1, 45, true),
                    member(3, "Adele", 2, 30, false),
                ],
            });
        }
        (held, queue, lua)
    }

    /// The list above and the registration are the same set, in both
    /// directions.
    #[test]
    fn every_verb_is_registered() {
        let names = |lua: &mlua::Lua| -> std::collections::BTreeSet<String> {
            lua.globals()
                .pairs::<String, mlua::Value>()
                .filter_map(Result::ok)
                .map(|(name, _)| name)
                .collect()
        };
        let lua = mlua::Lua::new();
        let before = names(&lua);
        register(&lua, &Rc::default(), &Rc::new(RefCell::new(Vec::new()))).expect("registers");
        let added: std::collections::BTreeSet<String> =
            names(&lua).difference(&before).cloned().collect();
        let listed: std::collections::BTreeSet<String> =
            VERBS.iter().map(|s| (*s).to_string()).collect();
        assert_eq!(added, listed);
    }

    /// A row answers the ten values in the order `GuildStatus_Update` assigns
    /// them, and a row past the end answers nothing.
    #[test]
    fn a_roster_row_answers_ten_values_and_a_missing_row_none() {
        let (_held, _queue, lua) = registered();
        let (name, rank, rank_index, level, note, online): (String, String, i64, i64, String, i64) = lua
            .load("local n, r, ri, l, c, z, no, on, o, s = GetGuildRosterInfo(2); return n, r, ri, l, no, o")
            .eval()
            .expect("values");
        assert_eq!((name.as_str(), rank.as_str(), rank_index, level), ("Bram", "Officer", 1, 45));
        assert_eq!((note.as_str(), online), ("Bram's note", 1));
        let count: i64 = lua
            .load("return select('#', GetGuildRosterInfo(9))")
            .eval()
            .expect("count");
        assert_eq!(count, 0);
        let offline: mlua::Value = lua
            .load("local n, r, ri, l, c, z, no, on, o = GetGuildRosterInfo(3); return o")
            .eval()
            .expect("value");
        assert!(matches!(offline, mlua::Value::Nil));
    }

    /// Hiding offline members removes their rows from the count and from the
    /// indices, and the number 0 hides them as nil does.
    #[test]
    fn the_offline_box_decides_which_members_are_rows() {
        let (_held, _queue, lua) = registered();
        let rows = |lua: &mlua::Lua| -> (i64, i64) {
            lua.load("return GetNumGuildMembers(), GetNumGuildMembers(1)")
                .eval()
                .expect("counts")
        };
        assert_eq!(rows(&lua), (3, 3));
        lua.load("SetGuildRosterShowOffline(0)").exec().expect("runs");
        assert_eq!(rows(&lua), (2, 3));
        let shown: mlua::Value = lua.load("return GetGuildRosterShowOffline()").eval().expect("value");
        assert!(matches!(shown, mlua::Value::Nil));
        lua.load("SetGuildRosterShowOffline(1)").exec().expect("runs");
        assert_eq!(rows(&lua), (3, 3));
    }

    /// The selection follows the member through a sort, and sorting by the
    /// same column twice reverses the order.
    #[test]
    fn the_selection_follows_the_member_through_a_sort() {
        let (_held, _queue, lua) = registered();
        let first = |lua: &mlua::Lua| -> String {
            lua.load("return (GetGuildRosterInfo(1))").eval().expect("name")
        };
        lua.load("SetGuildRosterSelection(1)").exec().expect("runs");
        assert_eq!(first(&lua), "Cade");
        lua.load("SortGuildRoster('name')").exec().expect("runs");
        assert_eq!(first(&lua), "Adele");
        let selected: i64 = lua.load("return GetGuildRosterSelection()").eval().expect("index");
        assert_eq!(selected, 3, "Cade is now the third row");
        lua.load("SortGuildRoster('name')").exec().expect("runs");
        assert_eq!(first(&lua), "Cade");
        lua.load("SetGuildRosterSelection(0)").exec().expect("runs");
        let selected: i64 = lua.load("return GetGuildRosterSelection()").eval().expect("index");
        assert_eq!(selected, 0);
    }

    /// The rights are those of the character's own rank, and the guild master
    /// is rank 0.
    #[test]
    fn the_rights_are_the_characters_own_ranks() {
        let (held, _queue, lua) = registered();
        let (invite, remove, leader): (mlua::Value, mlua::Value, mlua::Value) = lua
            .load("return CanGuildInvite(), CanGuildRemove(), IsGuildLeader()")
            .eval()
            .expect("values");
        assert!(!matches!(invite, mlua::Value::Nil), "an officer may invite");
        assert!(matches!(remove, mlua::Value::Nil));
        assert!(matches!(leader, mlua::Value::Nil));
        held.borrow_mut().set_membership(7, 0);
        let (remove, leader): (i64, i64) = lua
            .load("return CanGuildRemove(), IsGuildLeader()")
            .eval()
            .expect("values");
        assert_eq!((remove, leader), (1, 1));
    }

    /// The control window loads a rank's rights, edits one, reads the
    /// thirteen flags back in the same handler, and sends the rank by its
    /// zero-based index.
    #[test]
    fn the_control_window_edits_and_sends_a_rank() {
        let (_held, queue, lua) = registered();
        let (seventh, eighth): (mlua::Value, mlua::Value) = lua
            .load(
                r#"
                GuildControlSetRank(2)
                GuildControlSetRankFlag(8, 1)
                local f = { GuildControlGetRankFlags() }
                return f[7], f[8]
                "#,
            )
            .eval()
            .expect("flags");
        assert!(!matches!(seventh, mlua::Value::Nil), "invite was already held");
        assert!(!matches!(eighth, mlua::Value::Nil), "remove was just ticked");
        let count: i64 = lua
            .load("return select('#', GuildControlGetRankFlags())")
            .eval()
            .expect("count");
        assert_eq!(count, 13);
        lua.load("GuildControlSetRankFlag(7, nil); GuildControlSaveRank('Captain')")
            .exec()
            .expect("runs");
        assert_eq!(
            queue.borrow().as_slice(),
            [GuildVerb::Rank {
                rank: 1,
                rights: rights::EMPTY | rights::REMOVE | rights::SET_MOTD,
                name: "Captain".into(),
            }]
        );
        let (ranks, name): (i64, String) = lua
            .load("return GuildControlGetNumRanks(), GuildControlGetRankName(3)")
            .eval()
            .expect("values");
        assert_eq!((ranks, name.as_str()), (3, "Member"));
    }

    /// A note is set by row and sent by the member's name, and a request with
    /// an empty name sends nothing.
    #[test]
    fn a_request_names_the_member() {
        let (_held, queue, lua) = registered();
        lua.load(
            r#"
            GuildRosterSetPublicNote(2, "alt")
            GuildInviteByName("  ")
            GuildPromoteByName("Adele")
            GuildSetMOTD("")
            "#,
        )
        .exec()
        .expect("runs");
        assert_eq!(
            queue.borrow().as_slice(),
            [
                GuildVerb::PublicNote {
                    player: "Bram".into(),
                    note: "alt".into()
                },
                GuildVerb::Promote("Adele".into()),
                GuildVerb::Motd(String::new()),
            ]
        );
    }

    /// Leaving the guild drops the roster, and each unknown guild is asked
    /// for once.
    #[test]
    fn leaving_drops_the_roster_and_a_guild_is_asked_for_once() {
        let (held, _queue, _lua) = registered();
        let mut board = held.borrow_mut();
        assert!(board.take_wanted().contains(&7), "asked before the answer arrived");
        board.want(9);
        board.want(9);
        assert_eq!(board.take_wanted(), [9]);
        board.set_membership(0, 0);
        assert!(board.members.is_empty() && board.view.is_empty());
        assert_eq!(board.own_name(), None);
    }

    /// 400.5 days is one year, one month, five days and twelve hours.
    #[test]
    fn the_time_offline_is_split_into_four_units() {
        assert_eq!(last_online(400.5), (1, 1, 5, 12));
        assert_eq!(last_online(0.25), (0, 0, 0, 6));
        assert_eq!(last_online(-1.0), (0, 0, 0, 0));
    }
}
