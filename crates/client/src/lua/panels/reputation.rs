//! **The twelve C functions `ReputationFrame.lua` is written against**, and the
//! panel state behind them.
//!
//! ```text
//! GetNumFactions()             how many lines are on screen — not how many rows exist
//! GetFactionInfo(i)            …and one of them, eleven returns deep
//! GetSelectedFaction()         which line the detail pane is about, 0 for none
//! SetSelectedFaction(i)        …and clicking a bar
//! IsFactionInactive(i)         the third tick box
//! GetWatchedFactionInfo()      what the bar over the action bar shows
//! SetWatchedFactionIndex(i)    …and the tick that puts it there; 0 clears
//! CollapseFactionHeader(i)     the +/- on a heading
//! ExpandFactionHeader(i)
//! FactionToggleAtWar(i)        the crossed swords
//! SetFactionActive(i) SetFactionInactive(i)
//! ```
//!
//! ## Everything here is *held*, not queued, and that is forced
//!
//! Six of the twelve are writes, and the panel re-reads itself **inside the same
//! handler**: `ReputationBar_OnClick` is `SetSelectedFaction(this.id)` followed
//! three lines later by `ReputationFrame_Update()`, whose first act is
//! `GetNumFactions()`. A queued write applied by a Bevy system next frame would
//! draw the *previous* selection, and the click would look like it did nothing.
//! So this file keeps the same shape [`super::charcreate`] does — a
//! [`Held`] board every read and every write goes through — rather than the
//! queue-and-apply shape the rest of the directory uses.
//!
//! What still queues is the half the *server* needs: three of the six writes owe
//! a packet, and those go on a [`Queue`] that
//! [`crate::interface::reputation`] drains. None of the three is
//! acknowledged, which is why the local copy is the one that draws.
//!
//! ## The rules are not here
//!
//! Which rows exist, what order they are in, what a bar's ends are and what
//! "inactive" does to a row are all
//! [`vale_assets::tables::reputation::Reputation`]'s, unit-tested with no
//! window. This file is the twelve
//! signatures and the two conversions between them: **one-based indices**, and
//! the `1`/nil the interface reads as a boolean.
//!
//! ## The two synthetic headings are keys, not names
//!
//! `GetFactionInfo` on the *Other* and *Inactive* headings answers a
//! `GlobalStrings.lua` key (the client builds the name through the same lookup
//! `GetText` does), so this file resolves it — a panel showing the literal text
//! `FACTION_OTHER` is what a client that forgets to would draw.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use vale_assets::tables::reputation::{Factions, Reputation};
use vale_protocol::socket::session::ReputationVerb;

/// The **unscoped reads and writes** this file registers, sorted — the same list
/// [`super::super::api::READS`] is for the scoped ones.
///
/// All twelve are unscoped because none of them touches the world: the panel is
/// 64 slots of server state joined to `Faction.dbc`, and both live on [`Held`].
pub const VERBS: [&str; 12] = [
    "CollapseFactionHeader",
    "ExpandFactionHeader",
    "FactionToggleAtWar",
    "GetFactionInfo",
    "GetNumFactions",
    "GetSelectedFaction",
    "GetWatchedFactionInfo",
    "IsFactionInactive",
    "SetFactionActive",
    "SetFactionInactive",
    "SetSelectedFaction",
    "SetWatchedFactionIndex",
];

/// **The panel's whole state**, shared between the interpreter and the ECS.
#[derive(Default)]
pub struct Standing {
    /// `Faction.dbc`. `None` before the archives are open, which draws an empty
    /// panel — the same "nothing" every other table-less read here answers.
    pub factions: Option<Arc<Factions>>,
    /// The list itself. Empty until `SMSG_INITIALIZE_FACTIONS` lands.
    pub list: Reputation,
    /// `GlobalStrings.lua`, for the two synthetic headings' names.
    pub strings: Option<Arc<vale_assets::interface::strings::Strings>>,
    /// **Bumped by every change**, whoever made it — the client's own recount
    /// raises `UPDATE_FACTION` at its end, and this is
    /// what lets [`crate::interface::reputation`] do the same without
    /// diffing the list.
    pub version: u32,
    /// **The server's own 64 slots, held whether or not a list has been built
    /// from them yet.**
    ///
    /// It has to be held, and the reason is an ordering nothing on this side
    /// controls: vmangos sends `SMSG_INITIALIZE_FACTIONS` from
    /// `SendInitialPacketsBefore**AddToMap**` (`Player.cpp:19237`), so it
    /// arrives *before* the update block that carries the character — and the
    /// list cannot be built without the character, because every slot's base
    /// reputation is a `Faction.dbc` column chosen by race and class. A version
    /// of this that applied the packet on the frame it arrived dropped the
    /// server's only statement about reputation for the whole session, and did
    /// it silently.
    ///
    /// The three smaller packets patch this as well as the built list, so the
    /// wire stays the server's copy however early any of them lands.
    wire: Option<Box<[(u8, i32); 64]>>,
    /// Which `(race, class)` the list was built for, or `None` for one that has
    /// not been built. **Not a bare `bool`**: it is what stops
    /// [`Self::rebuild_if_ready`] running twice and throwing away the tick boxes
    /// the player has since set, which the wire does not carry.
    built_for: Option<(u8, u8)>,
    /// Whether the list has ever been built. A character who has met nobody
    /// still gets the packet, so this tells "no packet yet" from "no factions",
    /// which are drawn the same way and are not the same state.
    pub initialized: bool,
}

impl Standing {
    /// **Take the server's whole table** — `SMSG_INITIALIZE_FACTIONS`.
    ///
    /// Recorded rather than applied: see [`Self::wire`] for the ordering that
    /// forces it. [`Self::rebuild_if_ready`] is what turns it into a list, on
    /// whichever later frame the character and the archives are both there.
    pub fn initialize(&mut self, wire: &[(u8, i32); 64]) {
        self.wire = Some(Box::new(*wire));
        self.built_for = None;
        self.version = self.version.wrapping_add(1);
    }

    /// **Build the list if everything it needs has arrived**, and do nothing on
    /// every other frame.
    ///
    /// Cheap enough to call per frame: three `Option` tests and an equality
    /// against the pair it was last built for.
    pub fn rebuild_if_ready(&mut self, race: u8, class: u8) {
        if self.built_for == Some((race, class)) || race == 0 {
            return;
        }
        let (Some(factions), Some(wire)) = (self.factions.clone(), self.wire.as_ref()) else {
            return;
        };
        self.list.rebuild(&factions, race, class, wire);
        self.built_for = Some((race, class));
        self.initialized = true;
        self.version = self.version.wrapping_add(1);
    }

    /// …one slot's delta, `SMSG_SET_FACTION_STANDING`.
    pub fn standing(&mut self, rep: u32, standing: i32) {
        if let Some(slot) = self.wire.as_mut().and_then(|w| w.get_mut(rep as usize)) {
            slot.1 = standing;
        }
        self.with(|list, factions| list.apply_standing(factions, rep as usize, standing));
    }

    /// …a faction met, `SMSG_SET_FACTION_VISIBLE`.
    pub fn visible(&mut self, rep: u32) {
        if let Some(slot) = self.wire.as_mut().and_then(|w| w.get_mut(rep as usize)) {
            slot.0 |= vale_assets::tables::reputation::flags::VISIBLE;
        }
        self.with(|list, factions| list.apply_visible(factions, rep as usize));
    }

    /// …and the crossed swords, `SMSG_SET_FACTION_ATWAR`. The packet carries the
    /// whole flag byte and bit 1 is the one that matters.
    pub fn at_war(&mut self, rep: u32, flags: u8) {
        use vale_assets::tables::reputation::flags;
        let on = flags & flags::AT_WAR != 0;
        if let Some(slot) = self.wire.as_mut().and_then(|w| w.get_mut(rep as usize)) {
            slot.0 = (slot.0 & !flags::AT_WAR) | (flags & flags::AT_WAR);
        }
        self.with(|list, factions| list.apply_at_war(factions, rep as usize, on));
    }

    /// `PLAYER_FIELD_WATCHED_FACTION_INDEX` moved — the server's copy, and the
    /// authority over what `SetWatchedFactionIndex` asked for.
    pub fn watched(&mut self, reputation_list_id: i32) {
        if self.list.watched_index() == reputation_list_id {
            return;
        }
        self.list.set_watched(reputation_list_id);
        self.version = self.version.wrapping_add(1);
    }

    fn with(&mut self, edit: impl FnOnce(&mut Reputation, &Factions)) {
        let Some(factions) = self.factions.clone() else {
            return;
        };
        edit(&mut self.list, &factions);
        self.version = self.version.wrapping_add(1);
    }

    /// The name a row draws, with the two synthetic headings' keys resolved.
    fn word(&self, raw: String) -> String {
        if raw != vale_assets::tables::reputation::OTHER_KEY
            && raw != vale_assets::tables::reputation::INACTIVE_KEY
        {
            return raw;
        }
        self.strings
            .as_ref()
            .and_then(|s| s.get(&raw))
            .map(str::to_string)
            // **The key itself rather than an empty line**, which is what the
            // reference does for a `GetText` miss and is the one form that says
            // out loud which key is missing.
            .unwrap_or(raw)
    }
}

pub type Held = Rc<RefCell<Standing>>;
pub type Queue = Rc<RefCell<Vec<ReputationVerb>>>;

/// Register all twelve. Unscoped — see the module comment.
pub(in crate::lua) fn register(lua: &mlua::Lua, held: &Held, queue: &Queue) -> mlua::Result<()> {
    let globals = lua.globals();

    /// A verb taking a one-based index and changing the board.
    macro_rules! act {
        ($name:expr, |$board:ident, $factions:ident, $index:ident| $body:expr) => {{
            let held = Rc::clone(held);
            let f = lua.create_function(move |_, index: Option<usize>| {
                let $index = index.unwrap_or(0);
                let mut $board = held.borrow_mut();
                let Some($factions) = $board.factions.clone() else {
                    return Ok(());
                };
                $body;
                $board.version = $board.version.wrapping_add(1);
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }

    let get = Rc::clone(held);
    globals.set(
        "GetNumFactions",
        lua.create_function(move |_, ()| Ok(get.borrow().list.num_factions()))?,
    )?;

    // **Eleven returns and the interface unpacks all eleven**, so a short answer
    // is not a partial one: `local name, description, standingID, … = GetFactionInfo(i)`
    // shifts every name left of the missing value.
    let get = Rc::clone(held);
    globals.set(
        "GetFactionInfo",
        lua.create_function(move |lua, index: Option<usize>| {
            let board = get.borrow();
            let Some(factions) = board.factions.as_ref() else {
                return Ok(mlua::Variadic::new());
            };
            let Some(info) = board.list.info(factions, index.unwrap_or(0)) else {
                return Ok(mlua::Variadic::new());
            };
            let one = |flag: bool| {
                if flag {
                    mlua::Value::Number(1.0)
                } else {
                    mlua::Value::Nil
                }
            };
            Ok(mlua::Variadic::from(vec![
                mlua::Value::String(lua.create_string(board.word(info.name))?),
                mlua::Value::String(lua.create_string(&info.description)?),
                mlua::Value::Number(info.standing_id as f64),
                mlua::Value::Number(info.bar_min as f64),
                mlua::Value::Number(info.bar_max as f64),
                mlua::Value::Number(info.bar_value as f64),
                one(info.at_war),
                one(info.can_toggle_at_war),
                one(info.is_header),
                one(info.is_collapsed),
                one(info.is_watched),
            ]))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetSelectedFaction",
        lua.create_function(move |_, ()| Ok(get.borrow().list.selected()))?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "IsFactionInactive",
        lua.create_function(move |_, index: Option<usize>| {
            Ok(get
                .borrow()
                .list
                .is_inactive(index.unwrap_or(0))
                .then_some(1u32))
        })?,
    )?;

    // **Nil when nothing is watched**, which is the branch that hides the bar —
    // `ReputationWatchBar_Update`'s `if ( name )`.
    let get = Rc::clone(held);
    globals.set(
        "GetWatchedFactionInfo",
        lua.create_function(move |lua, ()| {
            let board = get.borrow();
            let watched = board
                .factions
                .as_ref()
                .and_then(|factions| board.list.watched(factions));
            let Some((name, standing, min, max, value)) = watched else {
                return Ok(mlua::Variadic::new());
            };
            Ok(mlua::Variadic::from(vec![
                mlua::Value::String(lua.create_string(&name)?),
                mlua::Value::Number(standing as f64),
                mlua::Value::Number(min as f64),
                mlua::Value::Number(max as f64),
                mlua::Value::Number(value as f64),
            ]))
        })?,
    )?;

    act!("SetSelectedFaction", |board, _f, index| board
        .list
        .select(index));
    act!("CollapseFactionHeader", |board, factions, index| board
        .list
        .set_collapsed(&factions, index, true));
    act!("ExpandFactionHeader", |board, factions, index| board
        .list
        .set_collapsed(&factions, index, false));

    // The three that owe a packet. Each edits the board *and* records, because
    // none of them is acknowledged — see the module comment.
    macro_rules! send {
        ($name:expr, |$board:ident, $factions:ident, $index:ident| $body:expr) => {{
            let held = Rc::clone(held);
            let queue = Rc::clone(queue);
            let f = lua.create_function(move |_, index: Option<usize>| {
                let $index = index.unwrap_or(0);
                let mut $board = held.borrow_mut();
                let Some($factions) = $board.factions.clone() else {
                    return Ok(());
                };
                let verb: Option<ReputationVerb> = $body;
                $board.version = $board.version.wrapping_add(1);
                if let Some(verb) = verb {
                    queue.borrow_mut().push(verb);
                }
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }

    send!("FactionToggleAtWar", |board, factions, index| {
        board
            .list
            .toggle_at_war(&factions, index)
            .and_then(|(rep, at_war)| {
                u32::try_from(rep)
                    .ok()
                    .map(|reputation_list_id| ReputationVerb::AtWar { reputation_list_id, at_war })
            })
    });
    send!("SetFactionInactive", |board, factions, index| {
        let rep = board.list.reputation_id_at(index);
        board.list.set_inactive(&factions, index, true);
        u32::try_from(rep)
            .ok()
            .map(|reputation_list_id| ReputationVerb::Inactive { reputation_list_id, inactive: true })
    });
    send!("SetFactionActive", |board, factions, index| {
        let rep = board.list.reputation_id_at(index);
        board.list.set_inactive(&factions, index, false);
        u32::try_from(rep)
            .ok()
            .map(|reputation_list_id| ReputationVerb::Inactive { reputation_list_id, inactive: false })
    });
    // **`SetWatchedFactionIndex(0)` is the clear**, and it reaches the wire as
    // `-1`: index 0 names no row, and `reputation_id_at` answers `-1` for one
    // that does not exist. That is the client's own arithmetic (it resolves
    // an out-of-range index to faction 0, whose reputation id is -1),
    // not a special case here.
    send!("SetWatchedFactionIndex", |board, _factions, index| {
        let reputation_list_id = board.list.reputation_id_at(index);
        board.list.set_watched(reputation_list_id);
        Some(ReputationVerb::Watched { reputation_list_id })
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::tables::reputation::flags;

    /// A board with one heading and two bars under it, built the way the packet
    /// would build it.
    fn board() -> (Held, Queue, mlua::Lua) {
        let dbc = sample_dbc();
        let factions = Arc::new(Factions::parse(&dbc).expect("a DBC"));
        let mut wire = [(0u8, 0i32); 64];
        wire[10] = (flags::VISIBLE | flags::HEADER, 0);
        wire[1] = (flags::VISIBLE, 500);
        wire[7] = (flags::VISIBLE, 0);
        let mut standing = Standing { factions: Some(factions), ..Standing::default() };
        standing.initialize(&wire);
        standing.rebuild_if_ready(1, 1);
        let held: Held = Rc::new(RefCell::new(standing));
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        let lua = mlua::Lua::new();
        register(&lua, &held, &queue).expect("registers");
        (held, queue, lua)
    }

    /// Three rows of a `Faction.dbc` written by hand — the same 37 fields the
    /// shipped file has, so [`Factions::parse`]'s column indices are exercised
    /// rather than bypassed.
    fn sample_dbc() -> Vec<u8> {
        let rows: [(u32, i32, u32, u32, &str); 3] = [
            (169, 10, 0, flags::HEADER as u32, "Steamwheedle Cartel"),
            (21, 1, 169, 0, "Booty Bay"),
            (369, 7, 169, 0, "Gadgetzan"),
        ];
        let fields = 37usize;
        let mut strings: Vec<u8> = vec![0];
        let mut records = Vec::new();
        for (id, rep, parent, flags, name) in rows {
            let mut record = vec![0u32; fields];
            record[0] = id;
            record[1] = rep as u32;
            record[2] = 0xffff_ffff; // every race
            record[14] = flags;
            record[18] = parent;
            record[19] = strings.len() as u32;
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
            record[28] = 0;
            for word in record {
                records.extend_from_slice(&word.to_le_bytes());
            }
        }
        let mut out = b"WDBC".to_vec();
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&(fields as u32).to_le_bytes());
        out.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        out.extend_from_slice(&records);
        out.extend_from_slice(&strings);
        out
    }

    /// The panel's own loop, run for real: the count, the eleven returns, and
    /// the heading-then-bars order it draws.
    #[test]
    fn the_panel_s_own_read_loop_answers() {
        let (_held, _queue, lua) = board();
        let names: Vec<String> = lua
            .load(
                r#"
                local out = {}
                for i = 1, GetNumFactions() do
                    local name, description, standingID, barMin, barMax, barValue,
                          atWar, canToggle, isHeader, isCollapsed, isWatched = GetFactionInfo(i)
                    table.insert(out, name .. "/" .. standingID .. "/" .. tostring(isHeader))
                end
                return out
            "#,
            )
            .eval()
            .expect("the loop runs");
        assert_eq!(
            names,
            vec![
                "Steamwheedle Cartel/4/1".to_string(),
                "Booty Bay/4/nil".to_string(),
                "Gadgetzan/4/nil".to_string(),
            ],
            "Neutral is standing 4, one-based"
        );
    }

    /// **A click has to be visible to the redraw in the same handler** — this is
    /// the whole reason the state is held rather than queued.
    #[test]
    fn a_selection_is_readable_by_the_redraw_that_follows_it() {
        let (_held, _queue, lua) = board();
        let selected: usize = lua
            .load("SetSelectedFaction(2); return GetSelectedFaction()")
            .eval()
            .expect("both run");
        assert_eq!(selected, 2);
    }

    /// Collapsing shortens the list, and it does so before the next read.
    #[test]
    fn collapsing_a_heading_shortens_the_list_at_once() {
        let (_held, _queue, lua) = board();
        let (before, after): (usize, usize) = lua
            .load(
                "local a = GetNumFactions(); CollapseFactionHeader(1); \
                 return a, GetNumFactions()",
            )
            .eval()
            .expect("both run");
        assert_eq!((before, after), (3, 1));
    }

    /// The three verbs that owe a packet queue exactly one each, with the
    /// reputation-list id the wire wants rather than the display index.
    #[test]
    fn the_three_verbs_that_owe_a_packet_queue_one_each() {
        let (_held, queue, lua) = board();
        lua.load("FactionToggleAtWar(2); SetFactionInactive(3); SetWatchedFactionIndex(2)")
            .exec()
            .expect("all three run");
        let sent = queue.borrow().clone();
        assert_eq!(
            sent,
            vec![
                ReputationVerb::AtWar { reputation_list_id: 1, at_war: true },
                ReputationVerb::Inactive { reputation_list_id: 7, inactive: true },
                ReputationVerb::Watched { reputation_list_id: 1 },
            ],
            "Booty Bay is slot 1 and Gadgetzan slot 7"
        );
    }

    /// `SetWatchedFactionIndex(0)` is the interface's own "stop watching", and
    /// it has to reach the wire as `-1` rather than as slot 0 — which is a real
    /// faction.
    #[test]
    fn watching_nothing_sends_minus_one_and_not_slot_zero() {
        let (held, queue, lua) = board();
        lua.load("SetWatchedFactionIndex(0)").exec().expect("runs");
        assert_eq!(
            queue.borrow().as_slice(),
            &[ReputationVerb::Watched { reputation_list_id: -1 }]
        );
        assert_eq!(held.borrow().list.watched_index(), -1);
        let watched: Option<String> = lua
            .load("local name = GetWatchedFactionInfo(); return name")
            .eval()
            .expect("runs");
        assert!(watched.is_none(), "…and the bar hides");
    }

    /// Without `Faction.dbc` every read answers its own nothing rather than a
    /// plausible constant, and no write raises.
    #[test]
    fn an_empty_panel_is_what_a_missing_table_draws() {
        let held: Held = Rc::new(RefCell::new(Standing::default()));
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        let lua = mlua::Lua::new();
        register(&lua, &held, &queue).expect("registers");
        let count: usize = lua.load("return GetNumFactions()").eval().expect("runs");
        assert_eq!(count, 0);
        lua.load("SetSelectedFaction(1); CollapseFactionHeader(1); FactionToggleAtWar(1)")
            .exec()
            .expect("none of them raise");
        assert!(queue.borrow().is_empty(), "and nothing is sent");
    }
}
