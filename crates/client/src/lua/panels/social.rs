//! **The eighteen C functions `FriendsFrame.lua` is written against**, and the
//! three lists behind them.
//!
//! ```text
//! GetNumFriends()          how many rows the friends tab has
//! GetFriendInfo(i)         …and one of them, six returns deep
//! GetSelectedFriend()      which row is highlighted, 0 for none
//! SetSelectedFriend(i)     …and clicking one
//! AddFriend(name)          /friend <name>, and the Who panel's Add Friend
//! RemoveFriend(indexOrName)  …and the Remove button, which passes an index
//! ShowFriends()            /friends with no name: ask the server again
//!
//! GetNumIgnores()          the ignore tab, which is a list of names and
//! GetIgnoreName(i)         nothing else at all
//! GetSelectedIgnore()
//! SetSelectedIgnore(i)
//! AddIgnore(name)  DelIgnore(name)  AddOrDelIgnore(name)
//!
//! GetNumWhoResults()       two numbers — see below
//! GetWhoInfo(i)            …and a row, six returns deep
//! SendWho(line)            the typed line, parsed here rather than by the
//!                          interface
//! SetWhoToUI(flag)         …and whether the answer goes to the panel or to
//!                          the chat frame
//! ```
//!
//! ## Everything here is *held*, not queued, for [`super::reputation`]'s reason
//!
//! `FriendsFrameFriendButton_OnClick` is `SetSelectedFriend(this:GetID())`
//! followed one line later by `FriendsList_Update()`, whose first act is
//! `GetNumFriends()`. A queued write applied by a Bevy system next frame would
//! draw the previous selection and the click would look like it did nothing. So
//! this keeps a [`Held`] board that every read and every write goes through, and
//! what queues is only the half the *server* needs — a [`Queue`] of
//! [`SocialVerb`]s that [`crate::interface::social`] drains.
//!
//! ## The two lists are guids and the names arrive separately
//!
//! Neither `SMSG_FRIEND_LIST` nor `SMSG_IGNORE_LIST` carries a name; see
//! [`vale_protocol::play::social`]. So a row here is a guid plus whatever the
//! name cache has answered so far, and a friend whose name has not arrived draws
//! as `UNKNOWN` — which is the reference's own answer and not a gap. The client
//! is unambiguous about it: `GetFriendInfo` pushes the localised `UNKNOWN` for
//! the name, the class and the area whenever the lookup misses, and pushes it
//! for **all three** when the index has no row at all. It never returns nil, which is why
//! `FriendsList_Update`'s `if ( not name )` guard never fires in the reference.
//!
//! ## `GetFriendInfo` answers words, not ids
//!
//! `(name, level, class, area, connected, status)` — and `class` and `area` are
//! **names**, resolved here against `ChrClasses.dbc` and `AreaTable.dbc`. The
//! client reads both through the same locale-indexed field a `GetText` would
//! and, for the area, **walks to the enclosing zone first** (the parent id) —
//! so a
//! friend standing in Northshire Abbey is listed in Elwynn Forest.
//!
//! `connected` is `1`/nil and `status` is `""`, `CHAT_FLAG_AFK` or
//! `CHAT_FLAG_DND` — a **bit test** on the wire's status byte rather than a
//! comparison, `& 4` for DND and `& 2` for AFK.
//! vmangos' `FRIEND_STATUS_DND` is 4 and `FRIEND_STATUS_AFK` is 2, so the two
//! agree.
//!
//! ## `GetNumWhoResults` is two numbers and the second is the one that matters
//!
//! The client pushes the rows it holds, and then the server's whole matching
//! population. `WhoList_Update` compares
//! the second against `MAX_WHOS_FROM_SERVER` three lines in, so a single `0`
//! leaves the second nil and the tab dies on *compare number with nil*.
//!
//! ## The rules are not here
//!
//! What `/who z-"Elwynn Forest" 5-10 c-warrior` means is
//! [`vale_assets::interface::whoquery`]'s, unit-tested with no window. This file is the sixteen signatures and the
//! conversions between them: one-based indices, and the `1`/nil the interface
//! reads as a boolean.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use vale_assets::interface::whoquery;
use vale_assets::tables::area::Areas;
use vale_assets::tables::charcreate::CharCreate;
use vale_protocol::play::social::{Friend, WhoRow};
use vale_protocol::socket::session::SocialVerb;

/// The **unscoped reads and writes** this file registers, sorted — the same list
/// [`super::super::api::READS`] is for the scoped ones.
///
/// All eighteen are unscoped because none of them touches the world: the two
/// lists are the server's, the who list is the server's, and every name in all
/// three comes off the query cache rather than off an entity.
///
/// Checked against [`register`] by `every_verb_is_registered`, on this repo's
/// own lesson about lists kept beside the thing they describe.
pub const VERBS: [&str; 18] = [
    "AddFriend",
    "AddIgnore",
    "AddOrDelIgnore",
    "DelIgnore",
    "GetFriendInfo",
    "GetIgnoreName",
    "GetNumFriends",
    "GetNumIgnores",
    "GetNumWhoResults",
    "GetSelectedFriend",
    "GetSelectedIgnore",
    "GetWhoInfo",
    "RemoveFriend",
    "SendWho",
    "SetSelectedFriend",
    "SetSelectedIgnore",
    "SetWhoToUI",
    "ShowFriends",
];

/// One row of the friends list, as the panel wants it.
#[derive(Debug, Clone, Default)]
pub struct FriendRow {
    pub wire: Friend,
    /// Whatever `CMSG_NAME_QUERY` has answered, or `None` while it has not.
    pub name: Option<String>,
}

/// **The panel's whole state**, shared between the interpreter and the ECS.
#[derive(Default)]
pub struct Social {
    /// `SMSG_FRIEND_LIST`, in the order the server sent it.
    pub friends: Vec<FriendRow>,
    /// `SMSG_IGNORE_LIST` — a guid and whatever name has arrived for it.
    pub ignores: Vec<(u64, Option<String>)>,
    /// `SMSG_WHO`'s rows, and the online total behind them.
    pub who: Vec<WhoRow>,
    pub who_online: u32,
    /// **Where the next `SMSG_WHO` goes**, which the interface decides —
    /// `SetWhoToUI(1)` before opening the panel, `SetWhoToUI(nil)` for a `/who`
    /// that should print into the chat frame instead. Nothing here reads it;
    /// [`crate::interface::social`] does, and it is held rather than queued
    /// so that the flag set *before* the search is the one in force when the
    /// answer lands.
    pub who_to_ui: bool,
    /// One-based, `0` for nothing selected — the interface's own convention and
    /// what `GetSelectedFriend` answers.
    selected_friend: usize,
    selected_ignore: usize,
    /// `ChrClasses.dbc`, for a friend's class name, and `AreaTable.dbc` for the
    /// zone. `None` before the archives are open, which draws `UNKNOWN` in both
    /// columns — the reference's own answer when its lookup misses.
    pub classes: Option<Arc<CharCreate>>,
    pub areas: Option<Arc<Areas>>,
    /// `GlobalStrings.lua`, for `UNKNOWN`, the two chat flags and the five
    /// `WHO_TAG_*` prefixes.
    pub strings: Option<Arc<vale_assets::interface::strings::Strings>>,
    /// **Bumped by every change, whoever made it** — the same counter
    /// [`super::reputation::Standing::version`] is, and read the same way: it is
    /// what lets a Bevy system raise `FRIENDLIST_UPDATE` without diffing two
    /// lists. Separate counters, because the two tabs redraw independently.
    pub friends_version: u32,
    pub ignores_version: u32,
    pub who_version: u32,
}

impl Social {
    /// `SMSG_FRIEND_LIST` — the whole list, replacing whatever was held.
    ///
    /// **Names are carried across by guid** rather than dropped, so a list
    /// re-read mid-session does not blank every row back to `UNKNOWN` until the
    /// queries come round again.
    pub fn set_friends(&mut self, list: &[Friend]) {
        let held: std::collections::HashMap<u64, String> = self
            .friends
            .iter()
            .filter_map(|row| row.name.clone().map(|name| (row.wire.guid, name)))
            .collect();
        self.friends = list
            .iter()
            .map(|wire| FriendRow {
                wire: *wire,
                name: held.get(&wire.guid).cloned(),
            })
            .collect();
        self.clamp_selection();
        self.friends_version = self.friends_version.wrapping_add(1);
    }

    /// `SMSG_IGNORE_LIST` — the same, one field narrower.
    pub fn set_ignores(&mut self, list: &[u64]) {
        let held: std::collections::HashMap<u64, String> = self
            .ignores
            .iter()
            .filter_map(|(guid, name)| name.clone().map(|name| (*guid, name)))
            .collect();
        self.ignores = list
            .iter()
            .map(|guid| (*guid, held.get(guid).cloned()))
            .collect();
        self.clamp_selection();
        self.ignores_version = self.ignores_version.wrapping_add(1);
    }

    /// `SMSG_FRIEND_STATUS` with one of the two "online" results: a row this
    /// client may not have had, with its level, zone and class attached.
    pub fn note_friend(&mut self, friend: Friend) {
        match self
            .friends
            .iter_mut()
            .find(|row| row.wire.guid == friend.guid)
        {
            Some(row) => row.wire = friend,
            None => self.friends.push(FriendRow {
                wire: friend,
                name: None,
            }),
        }
        self.friends_version = self.friends_version.wrapping_add(1);
    }

    /// …and one that only says a guid is now on the list, which is what an
    /// *offline* add carries: no level, no zone, no class, and nothing but the
    /// name query to fill them in.
    pub fn add_friend_guid(&mut self, guid: u64) {
        if self.friends.iter().any(|row| row.wire.guid == guid) {
            return;
        }
        self.friends.push(FriendRow {
            wire: Friend {
                guid,
                ..Friend::default()
            },
            name: None,
        });
        self.friends_version = self.friends_version.wrapping_add(1);
    }

    pub fn remove_friend_guid(&mut self, guid: u64) {
        let before = self.friends.len();
        self.friends.retain(|row| row.wire.guid != guid);
        if self.friends.len() != before {
            self.clamp_selection();
            self.friends_version = self.friends_version.wrapping_add(1);
        }
    }

    pub fn add_ignore_guid(&mut self, guid: u64) {
        if self.ignores.iter().any(|(g, _)| *g == guid) {
            return;
        }
        self.ignores.push((guid, None));
        self.ignores_version = self.ignores_version.wrapping_add(1);
    }

    pub fn remove_ignore_guid(&mut self, guid: u64) {
        let before = self.ignores.len();
        self.ignores.retain(|(g, _)| *g != guid);
        if self.ignores.len() != before {
            self.clamp_selection();
            self.ignores_version = self.ignores_version.wrapping_add(1);
        }
    }

    /// `SMSG_WHO`.
    pub fn set_who(&mut self, rows: Vec<WhoRow>, online: u32) {
        self.who = rows;
        self.who_online = online;
        self.who_version = self.who_version.wrapping_add(1);
    }

    /// **Copy in whatever the name cache has learned**, answering whether
    /// anything changed.
    ///
    /// The caller is a Bevy system with the world lock, so this takes a lookup
    /// rather than the map: it is called every frame while any row is unnamed,
    /// and the whole of the cost when they are all named is one `all`.
    pub fn learn_names(&mut self, mut name_of: impl FnMut(u64) -> Option<String>) {
        let mut moved_friends = false;
        for row in &mut self.friends {
            if row.name.is_none() {
                row.name = name_of(row.wire.guid);
                moved_friends |= row.name.is_some();
            }
        }
        let mut moved_ignores = false;
        for (guid, name) in &mut self.ignores {
            if name.is_none() {
                *name = name_of(*guid);
                moved_ignores |= name.is_some();
            }
        }
        if moved_friends {
            self.friends_version = self.friends_version.wrapping_add(1);
        }
        if moved_ignores {
            self.ignores_version = self.ignores_version.wrapping_add(1);
        }
    }

    /// Is anything on either list still waiting for a name? The guard on the
    /// world lock in [`Self::learn_names`]' caller.
    pub fn wants_names(&self) -> bool {
        self.friends.iter().any(|row| row.name.is_none())
            || self.ignores.iter().any(|(_, name)| name.is_none())
    }

    /// Every guid on either list, which is what the session hands the object
    /// manager so the queries go out.
    pub fn friend_guids(&self) -> Vec<u64> {
        self.friends.iter().map(|row| row.wire.guid).collect()
    }

    pub fn ignore_guids(&self) -> Vec<u64> {
        self.ignores.iter().map(|(guid, _)| *guid).collect()
    }

    /// The guid at a one-based row, for the two removals that take an index.
    pub fn friend_guid_at(&self, index: usize) -> Option<u64> {
        Some(self.friends.get(index.checked_sub(1)?)?.wire.guid)
    }

    /// …and by name, for `RemoveFriend("Bram")` off the slash command.
    /// Case-insensitive, because a player types what they remember rather than
    /// what the server capitalised.
    pub fn friend_guid_named(&self, name: &str) -> Option<u64> {
        self.friends
            .iter()
            .find(|row| {
                row.name
                    .as_deref()
                    .is_some_and(|held| held.eq_ignore_ascii_case(name))
            })
            .map(|row| row.wire.guid)
    }

    pub fn ignore_guid_named(&self, name: &str) -> Option<u64> {
        self.ignores
            .iter()
            .find(|(_, held)| {
                held.as_deref()
                    .is_some_and(|held| held.eq_ignore_ascii_case(name))
            })
            .map(|(guid, _)| *guid)
    }

    /// **What this client calls a guid**, from whichever of the two lists holds
    /// it, or `None` for one no name query has answered.
    ///
    /// `None` rather than `UNKNOWN` here, deliberately: the caller is the chat
    /// line an `SMSG_FRIEND_STATUS` prints, and *"Unknown added to friends"* is
    /// worse than saying nothing — the sentence is about a name the player just
    /// typed, so a missing one means the packet arrived before the query did.
    pub fn name_of(&self, guid: u64) -> Option<String> {
        self.friends
            .iter()
            .find(|row| row.wire.guid == guid)
            .and_then(|row| row.name.clone())
            .or_else(|| {
                self.ignores
                    .iter()
                    .find(|(g, _)| *g == guid)
                    .and_then(|(_, name)| name.clone())
            })
    }

    /// A row's name, or the localised `UNKNOWN` — see the module note on why
    /// this is never nil.
    fn word(&self, name: Option<&str>) -> String {
        match name {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => self.string("UNKNOWN", "Unknown"),
        }
    }

    fn string(&self, key: &str, fallback: &str) -> String {
        self.strings
            .as_ref()
            .and_then(|s| s.get(key))
            .unwrap_or(fallback)
            .to_string()
    }

    /// **A selection past the end of its list is cleared**, which is what stops
    /// a removal leaving the highlight on a row that is now somebody else.
    fn clamp_selection(&mut self) {
        if self.selected_friend > self.friends.len() {
            self.selected_friend = 0;
        }
        if self.selected_ignore > self.ignores.len() {
            self.selected_ignore = 0;
        }
    }

    /// The five `WHO_TAG_*` prefixes and the three name joins, as
    /// [`whoquery::parse`] wants them.
    ///
    /// Built per search rather than held, because a `/who` is a line somebody
    /// typed and the lists are short — see [`whoquery::Lookups`], which says the
    /// same thing from the other side.
    pub fn parse_who(&self, line: &str) -> whoquery::WhoQuery {
        let tags = self
            .strings
            .as_ref()
            .map(|s| whoquery::Tags::from_strings(s))
            .unwrap_or_default();
        let zones: Vec<(u32, &str)> = self
            .areas
            .as_ref()
            .map(|areas| areas.zones().map(|a| (a.id, a.name.as_str())).collect())
            .unwrap_or_default();
        let (races, classes): (Vec<(u8, &str)>, Vec<(u8, &str)>) = match self.classes.as_ref() {
            Some(create) => (
                create
                    .races()
                    .iter()
                    .map(|r| (r.id, r.name.as_str()))
                    .collect(),
                create
                    .every_class()
                    .map(|c| (c.id, c.name.as_str()))
                    .collect(),
            ),
            None => (Vec::new(), Vec::new()),
        };
        whoquery::parse(
            line,
            &tags,
            &whoquery::Lookups {
                zones: &zones,
                races: &races,
                classes: &classes,
            },
        )
    }
}

pub type Held = Rc<RefCell<Social>>;
pub type Queue = Rc<RefCell<Vec<SocialVerb>>>;

/// Register all eighteen. Unscoped — see the module comment.
pub(in crate::lua) fn register(lua: &mlua::Lua, held: &Held, queue: &Queue) -> mlua::Result<()> {
    let globals = lua.globals();

    let get = Rc::clone(held);
    globals.set(
        "GetNumFriends",
        lua.create_function(move |_, ()| Ok(get.borrow().friends.len()))?,
    )?;

    // **Six returns and never nil.** See the module note: the no-such-row
    // branch pushes six values like every other path.
    let get = Rc::clone(held);
    globals.set(
        "GetFriendInfo",
        lua.create_function(move |lua, index: Option<usize>| {
            let board = get.borrow();
            let row = index
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| board.friends.get(i));
            let unknown = board.word(None);
            let (name, level, class, area, connected, status) = match row {
                Some(row) => (
                    board.word(row.name.as_deref()),
                    f64::from(row.wire.level.max(1)),
                    board.class_name(row.wire.class),
                    board.zone_name(row.wire.area),
                    row.wire.online(),
                    board.status_word(row.wire.status),
                ),
                // The reference's own answer for an index with no row: three
                // `UNKNOWN`s, level 1, not connected, no flag.
                None => (
                    unknown.clone(),
                    1.0,
                    unknown.clone(),
                    unknown,
                    false,
                    String::new(),
                ),
            };
            Ok(mlua::Variadic::from(vec![
                mlua::Value::String(lua.create_string(&name)?),
                mlua::Value::Number(level),
                mlua::Value::String(lua.create_string(&class)?),
                mlua::Value::String(lua.create_string(&area)?),
                if connected {
                    mlua::Value::Number(1.0)
                } else {
                    mlua::Value::Nil
                },
                mlua::Value::String(lua.create_string(&status)?),
            ]))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetSelectedFriend",
        lua.create_function(move |_, ()| Ok(get.borrow().selected_friend))?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "SetSelectedFriend",
        lua.create_function(move |_, index: Option<usize>| {
            let mut board = get.borrow_mut();
            let index = index.unwrap_or(0);
            board.selected_friend = if index <= board.friends.len() { index } else { 0 };
            Ok(())
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetNumIgnores",
        lua.create_function(move |_, ()| Ok(get.borrow().ignores.len()))?,
    )?;

    // **A name or nothing**, which is the one place this file and the friends
    // list differ: `IgnoreList_Update` calls `nameText:SetText(GetIgnoreName(i))`
    // for every one of the fifteen buttons whether or not the row exists, and
    // `SetText(nil)` is how a button is blanked.
    let get = Rc::clone(held);
    globals.set(
        "GetIgnoreName",
        lua.create_function(move |lua, index: Option<usize>| {
            let board = get.borrow();
            let Some(row) = index
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| board.ignores.get(i))
            else {
                return Ok(mlua::Value::Nil);
            };
            Ok(mlua::Value::String(
                lua.create_string(&board.word(row.1.as_deref()))?,
            ))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetSelectedIgnore",
        lua.create_function(move |_, ()| Ok(get.borrow().selected_ignore))?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "SetSelectedIgnore",
        lua.create_function(move |_, index: Option<usize>| {
            let mut board = get.borrow_mut();
            let index = index.unwrap_or(0);
            board.selected_ignore = if index <= board.ignores.len() { index } else { 0 };
            Ok(())
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetNumWhoResults",
        lua.create_function(move |_, ()| {
            let board = get.borrow();
            Ok((board.who.len(), board.who_online))
        })?,
    )?;

    // `(name, guild, level, race, class, zone)` — and note that the
    // race comes **before** the class, which is the opposite way round from
    // `GetFriendInfo`'s columns.
    let get = Rc::clone(held);
    globals.set(
        "GetWhoInfo",
        lua.create_function(move |lua, index: Option<usize>| {
            let board = get.borrow();
            let Some(row) = index
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| board.who.get(i))
            else {
                return Ok(mlua::Variadic::new());
            };
            Ok(mlua::Variadic::from(vec![
                mlua::Value::String(lua.create_string(&row.name)?),
                mlua::Value::String(lua.create_string(&row.guild)?),
                mlua::Value::Number(f64::from(row.level)),
                mlua::Value::String(lua.create_string(&board.race_name(row.race))?),
                mlua::Value::String(lua.create_string(&board.class_name(row.class))?),
                mlua::Value::String(lua.create_string(&board.zone_name(row.zone))?),
            ]))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "SetWhoToUI",
        lua.create_function(move |_, flag: mlua::Value| {
            get.borrow_mut().who_to_ui = !matches!(flag, mlua::Value::Nil | mlua::Value::Boolean(false));
            Ok(())
        })?,
    )?;

    // --- the six writes -----------------------------------------------------

    let get = Rc::clone(held);
    let send = Rc::clone(queue);
    globals.set(
        "SendWho",
        lua.create_function(move |_, line: Option<String>| {
            let query = get.borrow().parse_who(line.as_deref().unwrap_or_default());
            send.borrow_mut()
                .push(SocialVerb::Who(Box::new(request_of(&query))));
            Ok(())
        })?,
    )?;

    let send = Rc::clone(queue);
    globals.set(
        "ShowFriends",
        lua.create_function(move |_, ()| {
            send.borrow_mut().push(SocialVerb::List);
            Ok(())
        })?,
    )?;

    let send = Rc::clone(queue);
    globals.set(
        "AddFriend",
        lua.create_function(move |_, name: Option<String>| {
            if let Some(name) = trimmed(name) {
                send.borrow_mut().push(SocialVerb::AddFriend(name));
            }
            Ok(())
        })?,
    )?;

    let send = Rc::clone(queue);
    globals.set(
        "AddIgnore",
        lua.create_function(move |_, name: Option<String>| {
            if let Some(name) = trimmed(name) {
                send.borrow_mut().push(SocialVerb::AddIgnore(name));
            }
            Ok(())
        })?,
    )?;

    // **`RemoveFriend` takes an index *or* a name**, and both callers are in the
    // shipped directory: `FriendsFrame_RemoveFriend` passes
    // `FriendsFrame.selectedFriend` and `SlashCmdList["REMOVEFRIEND"]` passes
    // what was typed. The wire takes neither — `CMSG_DEL_FRIEND` is a guid — so
    // this is where both are turned into one.
    let get = Rc::clone(held);
    let send = Rc::clone(queue);
    globals.set(
        "RemoveFriend",
        lua.create_function(move |_, which: mlua::Value| {
            let board = get.borrow();
            if let Some(guid) = guid_of(&board, &which, Social::friend_guid_at, Social::friend_guid_named)
            {
                send.borrow_mut().push(SocialVerb::DelFriend(guid));
            }
            Ok(())
        })?,
    )?;

    // `DelIgnore` is only ever called with a name — `FriendsFrame_UnIgnore`
    // reads `GetIgnoreName(...)` first rather than passing the index — but it is
    // taken both ways for the same reason `RemoveFriend` is: the two are one
    // gesture in the panel and telling them apart at the call site is what
    // produces a verb that silently does nothing.
    let get = Rc::clone(held);
    let send = Rc::clone(queue);
    globals.set(
        "DelIgnore",
        lua.create_function(move |_, which: mlua::Value| {
            let board = get.borrow();
            if let Some(guid) = guid_of(&board, &which, ignore_guid_at, Social::ignore_guid_named) {
                send.borrow_mut().push(SocialVerb::DelIgnore(guid));
            }
            Ok(())
        })?,
    )?;

    // **The one verb that is a *toggle*** — `/ignore <name>` on somebody already
    // ignored un-ignores them, which is what `SlashCmdList["IGNORE"]` calls and
    // the only reason this name exists at all.
    let get = Rc::clone(held);
    let send = Rc::clone(queue);
    globals.set(
        "AddOrDelIgnore",
        lua.create_function(move |_, name: Option<String>| {
            let Some(name) = trimmed(name) else {
                return Ok(());
            };
            let board = get.borrow();
            let verb = match board.ignore_guid_named(&name) {
                Some(guid) => SocialVerb::DelIgnore(guid),
                None => SocialVerb::AddIgnore(name),
            };
            send.borrow_mut().push(verb);
            Ok(())
        })?,
    )?;

    Ok(())
}

/// A one-based row of the ignore list, as a free function so that
/// [`guid_of`] can take the two lists' lookups the same way.
fn ignore_guid_at(board: &Social, index: usize) -> Option<u64> {
    Some(board.ignores.get(index.checked_sub(1)?)?.0)
}

/// **An index or a name, whichever the caller had** — see `RemoveFriend` above.
///
/// A number that is not a row and a name nothing on the list answers to both
/// come back `None`, and the verb is not queued: sending `CMSG_DEL_FRIEND` for
/// guid 0 would be a packet the server acts on.
fn guid_of(
    board: &Social,
    which: &mlua::Value,
    by_index: impl Fn(&Social, usize) -> Option<u64>,
    by_name: impl Fn(&Social, &str) -> Option<u64>,
) -> Option<u64> {
    match which {
        mlua::Value::Integer(i) => by_index(board, (*i).try_into().ok()?),
        mlua::Value::Number(n) if *n >= 1.0 => by_index(board, *n as usize),
        mlua::Value::String(s) => {
            let text = s.to_str().ok()?;
            let text = text.trim();
            // 1.12 has no type discipline here: `RemoveFriend("3")` off a macro
            // is the same call as `RemoveFriend(3)` and the reference's own
            // argument reader coerces. A name that is all digits is not a name.
            match text.parse::<usize>() {
                Ok(index) => by_index(board, index),
                Err(_) => by_name(board, text),
            }
        }
        _ => None,
    }
}

fn trimmed(name: Option<String>) -> Option<String> {
    let name = name?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Turn a parsed query into the wire's own request.
///
/// Two structs rather than one because the crates do not depend on each other in
/// that direction: `assets` owns the *rule* and `protocol` owns the *body*, and
/// this file is the only place both are in scope.
fn request_of(query: &whoquery::WhoQuery) -> vale_protocol::play::social::WhoRequest {
    vale_protocol::play::social::WhoRequest {
        level_min: query.level_min,
        level_max: query.level_max,
        name: query.name.clone(),
        guild: query.guild.clone(),
        race_mask: query.race_mask,
        class_mask: query.class_mask,
        zones: query.zones.clone(),
        terms: query.terms.clone(),
    }
}

impl Social {
    /// A `ChrClasses.dbc` id as the localised class name, or `UNKNOWN`.
    fn class_name(&self, class: u32) -> String {
        let name = self.classes.as_ref().and_then(|create| {
            create
                .every_class()
                .find(|c| u32::from(c.id) == class)
                .map(|c| c.name.clone())
        });
        self.word(name.as_deref())
    }

    /// …and a `ChrRaces.dbc` id.
    fn race_name(&self, race: u32) -> String {
        let name = self.classes.as_ref().and_then(|create| {
            create
                .races()
                .iter()
                .find(|r| u32::from(r.id) == race)
                .map(|r| r.name.clone())
        });
        self.word(name.as_deref())
    }

    /// An area id as the **enclosing zone's** name — see the module note.
    fn zone_name(&self, area: u32) -> String {
        let name = self
            .areas
            .as_ref()
            .map(|areas| areas.zone_name(area))
            .filter(|name| !name.is_empty());
        self.word(name.as_deref())
    }

    /// `""`, `CHAT_FLAG_AFK` or `CHAT_FLAG_DND`, by a bit test on the status.
    fn status_word(&self, status: u8) -> String {
        if status & DND != 0 {
            self.string("CHAT_FLAG_DND", "<DND>")
        } else if status & AFK != 0 {
            self.string("CHAT_FLAG_AFK", "<AFK>")
        } else {
            String::new()
        }
    }
}

/// vmangos' `FRIEND_STATUS_AFK` and `FRIEND_STATUS_DND`, which the reference
/// reads as **bits** rather than as an enum — `test al, 4` then `test al, 2`.
const AFK: u8 = 2;
const DND: u8 = 4;

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Social {
        let mut social = Social::default();
        social.set_friends(&[
            Friend {
                guid: 11,
                status: 1,
                area: 12,
                level: 60,
                class: 1,
            },
            Friend {
                guid: 22,
                ..Friend::default()
            },
        ]);
        social
    }

    /// A name arriving late fills the row it belongs to and bumps the counter
    /// that raises `FRIENDLIST_UPDATE`.
    #[test]
    fn a_name_arriving_late_fills_its_row() {
        let mut social = board();
        let before = social.friends_version;
        assert!(social.wants_names());
        social.learn_names(|guid| (guid == 11).then(|| "Bram".to_string()));
        assert_eq!(social.friends[0].name.as_deref(), Some("Bram"));
        assert!(social.wants_names(), "the other row is still unnamed");
        assert_ne!(social.friends_version, before);
    }

    /// **A re-read of the list keeps the names it already had.** Without this a
    /// `ShowFriends()` blanks every row to `UNKNOWN` until the query pass comes
    /// round again, which is a visible flicker for a packet that changed
    /// nothing.
    #[test]
    fn re_reading_the_list_keeps_the_names() {
        let mut social = board();
        social.learn_names(|guid| Some(format!("player{guid}")));
        social.set_friends(&[Friend {
            guid: 11,
            status: 2,
            ..Friend::default()
        }]);
        assert_eq!(social.friends[0].name.as_deref(), Some("player11"));
        assert_eq!(social.friends[0].wire.status, 2, "the wire half is replaced");
    }

    /// A removal past the selection clears it, so the highlight cannot end up on
    /// a row that is now somebody else.
    #[test]
    fn removing_the_selected_row_clears_the_selection() {
        let mut social = board();
        social.selected_friend = 2;
        social.remove_friend_guid(22);
        assert_eq!(social.selected_friend, 0);
        assert_eq!(social.friends.len(), 1);
    }

    /// The status byte is read as bits, which is what the reference does.
    #[test]
    fn the_status_is_a_bit_test_and_not_an_enum() {
        let social = Social::default();
        assert_eq!(social.status_word(0), "");
        assert_eq!(social.status_word(1), "");
        assert_eq!(social.status_word(AFK), "<AFK>");
        assert_eq!(social.status_word(DND), "<DND>");
        // **DND wins**, because the reference tests it first — a status of 6
        // would otherwise read as AFK.
        assert_eq!(social.status_word(AFK | DND), "<DND>");
    }

    /// Both ways of naming a friend reach the same guid, and neither a bad index
    /// nor an unknown name reaches one at all.
    #[test]
    fn a_friend_is_found_by_index_or_by_name() {
        let mut social = board();
        social.learn_names(|guid| (guid == 11).then(|| "Bram".to_string()));
        assert_eq!(social.friend_guid_at(1), Some(11));
        assert_eq!(social.friend_guid_at(0), None);
        assert_eq!(social.friend_guid_at(9), None);
        assert_eq!(social.friend_guid_named("bram"), Some(11));
        assert_eq!(social.friend_guid_named("nobody"), None);
    }

    /// A board wired to a real Lua state, which is the only way the sixteen
    /// registrations are checked at all.
    fn registered() -> (Held, Queue, mlua::Lua) {
        let held: Held = Rc::new(RefCell::new(board()));
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        let lua = mlua::Lua::new();
        register(&lua, &held, &queue).expect("registers");
        (held, queue, lua)
    }

    /// **The published list is the registered one**, which is what stops the
    /// documentation above from drifting the way this repo's five API arrays
    /// each did at least once. Both directions: a name in [`VERBS`] that nothing
    /// registers, and a global registered that the list does not name.
    #[test]
    fn every_verb_is_registered() {
        let lua = mlua::Lua::new();
        let before: std::collections::BTreeSet<String> = lua
            .globals()
            .pairs::<String, mlua::Value>()
            .filter_map(Result::ok)
            .map(|(name, _)| name)
            .collect();
        register(&lua, &Rc::default(), &Rc::new(RefCell::new(Vec::new()))).expect("registers");
        let after: std::collections::BTreeSet<String> = lua
            .globals()
            .pairs::<String, mlua::Value>()
            .filter_map(Result::ok)
            .map(|(name, _)| name)
            .collect();
        let added: std::collections::BTreeSet<String> =
            after.difference(&before).cloned().collect();
        let listed: std::collections::BTreeSet<String> =
            VERBS.iter().map(|s| (*s).to_string()).collect();
        assert_eq!(added, listed);
    }

    /// **Six returns and never nil**, including for a row that does not exist —
    /// see the module note.
    #[test]
    fn a_friend_row_answers_six_values_even_when_there_is_no_row() {
        let (held, _queue, lua) = registered();
        held.borrow_mut()
            .learn_names(|guid| (guid == 11).then(|| "Bram".to_string()));
        let (name, level, connected, status): (String, i64, mlua::Value, String) = lua
            .load("local n, l, c, a, on, st = GetFriendInfo(1); return n, l, on, st")
            .eval()
            .expect("six values");
        assert_eq!(name, "Bram");
        assert_eq!(level, 60);
        assert!(!matches!(connected, mlua::Value::Nil));
        assert_eq!(status, "", "online and unflagged is the empty string");

        let count: i64 = lua
            .load("local a,b,c,d,e,f = GetFriendInfo(99); return 6")
            .eval()
            .expect("still six");
        assert_eq!(count, 6);
        let missing: String = lua.load("return (GetFriendInfo(99))").eval().expect("a word");
        assert_eq!(missing, "Unknown", "a row that is not there is not nil");
    }

    /// `GetNumWhoResults` answers **two** numbers, which is the shape
    /// `WhoList_Update` compares against `MAX_WHOS_FROM_SERVER`.
    #[test]
    fn the_who_count_is_two_numbers() {
        let (held, _queue, lua) = registered();
        held.borrow_mut().set_who(
            vec![WhoRow {
                name: "Bram".into(),
                guild: "The Watch".into(),
                level: 60,
                ..WhoRow::default()
            }],
            273,
        );
        let (rows, total): (i64, i64) = lua
            .load("local n, t = GetNumWhoResults(); return n, t")
            .eval()
            .expect("two numbers");
        assert_eq!((rows, total), (1, 273));
        // …and the row is `(name, guild, level, race, class, zone)` — the race
        // before the class, which is the opposite order from `GetFriendInfo`.
        let (name, guild, level): (String, String, i64) = lua
            .load("local n, g, l = GetWhoInfo(1); return n, g, l")
            .eval()
            .expect("a row");
        assert_eq!((name.as_str(), guild.as_str(), level), ("Bram", "The Watch", 60));
    }

    /// **`RemoveFriend` takes an index or a name**, and reaches the same guid
    /// either way — both callers are in the shipped directory.
    #[test]
    fn removing_a_friend_takes_an_index_or_a_name() {
        let (held, queue, lua) = registered();
        held.borrow_mut()
            .learn_names(|guid| (guid == 11).then(|| "Bram".to_string()));
        lua.load("RemoveFriend(1)").exec().expect("by index");
        lua.load("RemoveFriend(\"bram\")").exec().expect("by name");
        assert_eq!(
            *queue.borrow(),
            vec![SocialVerb::DelFriend(11), SocialVerb::DelFriend(11)]
        );
    }

    /// …and a row that is not there sends nothing at all, rather than a
    /// `CMSG_DEL_FRIEND` naming guid 0 — which the server would act on.
    #[test]
    fn removing_nobody_sends_nothing() {
        let (_held, queue, lua) = registered();
        lua.load("RemoveFriend(99); RemoveFriend(\"nobody\"); RemoveFriend(nil)")
            .exec()
            .expect("runs");
        assert!(queue.borrow().is_empty());
    }

    /// **`/ignore` is a toggle**, which is the only reason `AddOrDelIgnore`
    /// exists: the same name twice adds and then removes.
    #[test]
    fn the_ignore_verb_toggles() {
        let (held, queue, lua) = registered();
        lua.load("AddOrDelIgnore(\"Bram\")").exec().expect("adds");
        assert_eq!(*queue.borrow(), vec![SocialVerb::AddIgnore("Bram".into())]);
        queue.borrow_mut().clear();
        {
            let mut board = held.borrow_mut();
            board.set_ignores(&[42]);
            board.learn_names(|guid| (guid == 42).then(|| "Bram".to_string()));
        }
        lua.load("AddOrDelIgnore(\"bram\")").exec().expect("removes");
        assert_eq!(*queue.borrow(), vec![SocialVerb::DelIgnore(42)]);
    }

    /// A typed `/who` line reaches the wire's own request, through
    /// [`vale_assets::interface::whoquery`]. No tables here, so the two masks
    /// are the "matches nothing" a name nothing answers to produces — which is
    /// the rule, not an artefact of the harness.
    #[test]
    fn a_typed_who_line_becomes_a_request() {
        let (_held, queue, lua) = registered();
        lua.load("SendWho(\"10-20 hello\")").exec().expect("searches");
        let queued = queue.borrow();
        let SocialVerb::Who(request) = queued.first().expect("one verb") else {
            panic!("a who");
        };
        assert_eq!((request.level_min, request.level_max), (10, 20));
        assert_eq!(request.terms, vec!["hello".to_string()]);
    }

    /// `SetWhoToUI` is Lua truthiness, like every other flag in this directory.
    #[test]
    fn the_who_destination_is_a_flag() {
        let (held, _queue, lua) = registered();
        lua.load("SetWhoToUI(1)").exec().expect("set");
        assert!(held.borrow().who_to_ui);
        lua.load("SetWhoToUI(nil)").exec().expect("cleared");
        assert!(!held.borrow().who_to_ui);
    }

    /// **`GetIgnoreName` is the one read here that answers nil**, because
    /// `IgnoreList_Update` calls it for all fifteen buttons and `SetText(nil)`
    /// is how a row is blanked.
    #[test]
    fn an_ignore_row_that_is_not_there_is_nil() {
        let (held, _queue, lua) = registered();
        held.borrow_mut().set_ignores(&[42]);
        let present: mlua::Value = lua.load("return GetIgnoreName(1)").eval().expect("a value");
        assert!(matches!(present, mlua::Value::String(_)));
        let absent: mlua::Value = lua.load("return GetIgnoreName(9)").eval().expect("a value");
        assert!(matches!(absent, mlua::Value::Nil));
    }

    /// An add that names a guid already on the list changes nothing, which is
    /// what an "already your friend" refusal followed by a real add would
    /// otherwise duplicate.
    #[test]
    fn adding_a_guid_twice_is_one_row() {
        let mut social = Social::default();
        social.add_friend_guid(7);
        social.add_friend_guid(7);
        assert_eq!(social.friends.len(), 1);
        social.add_ignore_guid(9);
        social.add_ignore_guid(9);
        assert_eq!(social.ignores.len(), 1);
    }
}
