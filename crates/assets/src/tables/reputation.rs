//! **A player's own standing** — `Faction.dbc`, and the list the reputation
//! panel is drawn from.
//!
//! [`super::faction`] answers "may I attack this?" out of `FactionTemplate.dbc`
//! and says in its own note that a *player's* standing is the half it does not
//! model. This is that half, and it is a different table with a different shape:
//! 190 rows keyed by faction id, of which **64 have a `reputationListID`** and
//! are the only ones a character can have a bar for.
//!
//! ## Nothing here is reconstructed, and that matters more than usual
//!
//! The wire carries **two numbers per slot and no structure at all**:
//! `SMSG_INITIALIZE_FACTIONS` is 64 pairs of `(flags: u8, standing: i32)` in
//! reputation-list order. Every other thing the panel draws — which rows exist,
//! which are headings, what order they are in, where a bar starts and ends, and
//! what the standing *is* — is a rule the client keeps to itself. So this is
//! the kind of rule that renders plausibly wrong, and all of it follows the
//! client rather than being reasoned about:
//!
//! ```text
//! the builder: the DBC walk, the race/class index, the two lists
//!   …and one row of it, including the header a parent implies
//! the recount: child counts, the two sorts, what is displayed
//!   …the header comparator, and the row comparator
//! GetFactionInfo — the eleven returns, in order
//! the rank, and the eight bands it comes off
//! the standing: the packet's number is a *delta from base*
//! ```
//!
//! ## Three rules that are not guessable and are load-bearing
//!
//! **The standing on the wire is a delta.** The client's standing is
//! `state[rep].standing + state[rep].base`, where `base` is the DBC's
//! `reputationBase` for this character's race and class. vmangos agrees from the
//! other side — `ReputationMgr::SetOneFactionReputation` stores
//! `standing - BaseRep` — so a client that draws the packet's number raw shows a
//! dwarf as Neutral with Ironforge, and the error is exactly the base.
//!
//! **A heading is a flag, not a shape.** The client reads bit 3 of the
//! *server-sent* flags to decide `isHeader`; vmangos calls the same bit
//! `FACTION_FLAG_INVISIBLE_FORCED`, which is a different name for the same
//! column. **Five** rows carry it in the shipped file — Alliance (469), Horde
//! (67), Steamwheedle Cartel (169), Alliance Forces (891) and Horde Forces
//! (892) — which is precisely the set of headings a 1.12 reputation panel has,
//! so the reading checks out against the data as well as against the code.
//! `vale reputation` prints them, and if that count moves it is the reading
//! of the bit that moved.
//!
//! **Two headings exist that are not factions at all.** Faction id `0` is
//! `FACTION_OTHER` and faction id `-1` is `FACTION_INACTIVE` — both
//! `GlobalStrings.lua` keys rather than DBC names, the second
//! appended at the end of the build and **collapsed on the spot**
//! Everything a character has marked inactive is
//! re-parented under it by the recount, which is why "inactive" moves a row
//! rather than hiding it.
//!
//! ## What is displayed is a prefix, which is why the sorts are here
//!
//! `GetFactionInfo(i)` indexes the row array *directly*, and
//! `GetNumFactions()` is a separate counter. The two only agree because the row
//! comparator sorts every row under a collapsed heading — and every heading with
//! no children — to the end, so the visible rows are the array's own prefix.
//! Get that wrong and the panel does not fail: it draws the right number of bars
//! with the wrong factions on them.

use crate::tables::dbc::Dbc;
use crate::AssetError;
use std::collections::HashMap;

/// How many reputation-list slots the wire has. `SMSG_INITIALIZE_FACTIONS` is
/// exactly this many pairs, and the client refuses a `reputationListID` outside
/// it.
pub const SLOTS: usize = 64;

/// The flag byte each slot carries, as the server sends it.
///
/// Six of the seven are vmangos' `FactionFlags` verbatim; [`flags::HEADER`] is
/// the one this client names differently, and the reason is in the module note.
pub mod flags {
    /// The row exists at all. A slot without it is a faction the character has
    /// never met, and the panel has no line for it.
    pub const VISIBLE: u8 = 0x01;
    /// The crossed-swords box is ticked.
    pub const AT_WAR: u8 = 0x02;
    /// The server tracks it and never shows it.
    pub const HIDDEN: u8 = 0x04;
    /// **The row is a heading.** vmangos calls this `INVISIBLE_FORCED`; the
    /// client reads it to set a row's `isHeader`.
    pub const HEADER: u8 = 0x08;
    /// The at-war box is greyed — one's own side, and the two battleground
    /// parents.
    pub const PEACE_FORCED: u8 = 0x10;
    /// The player filed it under the *Inactive* heading.
    pub const INACTIVE: u8 = 0x20;
    /// Set on the two competing outland factions; unused in 1.12 and kept so
    /// the byte is documented whole.
    pub const RIVAL: u8 = 0x40;
}

/// **The eight standings' floors**, from the client's own table.
///
/// Nine entries because a bar wants both ends: `barMin` is `[rank]` and `barMax`
/// is `[rank + 1]`. vmangos' cumulative `PointsInRank`
/// produces the identical sequence, which is two authorities on a table that
/// never crosses the wire.
pub const RANK_FLOORS: [i32; 9] = [-42000, -6000, -3000, 0, 3000, 9000, 21000, 42000, 43000];

/// The number of standings — Hated through Exalted.
pub const RANKS: usize = 8;

/// **The standing a raw reputation total sits in**, zero-based.
///
/// The interface wants it one-based, because `FACTION_STANDING_LABEL1` is Hated
/// and `FACTION_BAR_COLORS[1]` is its colour — see [`Info::standing_id`].
pub fn rank_of(standing: i32) -> usize {
    // Descending, exactly as the client's cascade runs, so the boundary case
    // (`standing == floor`) lands in the higher band the way the client's
    // does.
    for rank in (0..RANKS).rev() {
        if standing >= RANK_FLOORS[rank] {
            return rank;
        }
    }
    0
}

/// One `Faction.dbc` row, in the four columns the panel and the builder need.
///
/// The four-wide arrays are the table's own shape: a faction states up to four
/// `(raceMask, classMask) -> (base, flags)` alternatives and the character picks
/// the one that fits them. See [`Factions::fitting`].
#[derive(Debug, Clone, Default)]
pub struct Faction {
    pub id: u32,
    /// `-1` for the 126 rows that are creature factions and nothing else.
    pub reputation_list_id: i32,
    pub race_mask: [u32; 4],
    pub class_mask: [u32; 4],
    pub base: [i32; 4],
    pub flags: [u32; 4],
    /// Field 18 — the heading this faction goes under, or `0` for none, which is
    /// the *Other* heading rather than an absence.
    pub parent: u32,
    pub name: String,
    pub description: String,
}

mod fields {
    pub const ID: usize = 0;
    pub const REPUTATION_LIST_ID: usize = 1;
    pub const RACE_MASK: usize = 2;
    pub const CLASS_MASK: usize = 6;
    pub const BASE: usize = 10;
    pub const FLAGS: usize = 14;
    pub const PARENT: usize = 18;
    /// The localised name block: eight locales then a flag word, which is why
    /// the description is nine fields further on and not one.
    pub const NAME: usize = 19;
    pub const DESCRIPTION: usize = 28;
    pub const ALTERNATIVES: usize = 4;
}

/// `Faction.dbc`, by faction id.
#[derive(Debug, Clone, Default)]
pub struct Factions(HashMap<u32, Faction>);

impl Factions {
    pub fn parse(raw: &[u8]) -> Result<Factions, AssetError> {
        let dbc = Dbc::parse(raw)?;
        let mut rows = HashMap::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            let Some(id) = dbc.u32_at(record, fields::ID) else {
                continue;
            };
            let quad = |first: usize| {
                let mut out = [0u32; fields::ALTERNATIVES];
                for (i, slot) in out.iter_mut().enumerate() {
                    *slot = dbc.u32_at(record, first + i).unwrap_or(0);
                }
                out
            };
            let base_raw = quad(fields::BASE);
            let mut base = [0i32; fields::ALTERNATIVES];
            for (i, slot) in base.iter_mut().enumerate() {
                *slot = base_raw[i] as i32;
            }
            rows.insert(
                id,
                Faction {
                    id,
                    reputation_list_id: dbc.u32_at(record, fields::REPUTATION_LIST_ID).unwrap_or(0)
                        as i32,
                    race_mask: quad(fields::RACE_MASK),
                    class_mask: quad(fields::CLASS_MASK),
                    base,
                    flags: quad(fields::FLAGS),
                    parent: dbc.u32_at(record, fields::PARENT).unwrap_or(0),
                    name: dbc.string_at(record, fields::NAME).unwrap_or_default(),
                    description: dbc.string_at(record, fields::DESCRIPTION).unwrap_or_default(),
                },
            );
        }
        Ok(Factions(rows))
    }

    pub fn get(&self, id: u32) -> Option<&Faction> {
        self.0.get(&id)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every row, in **descending id order** — the walk the builder makes.
    ///
    /// The client walks `Faction.dbc` from its last record to its first
    /// and records happen to be in ascending id order in the shipped file. The
    /// direction is preserved because it decides which of two equally-named rows
    /// is inserted first, and the row sort below is stable.
    pub fn descending(&self) -> Vec<&Faction> {
        let mut out: Vec<&Faction> = self.0.values().collect();
        out.sort_by(|a, b| b.id.cmp(&a.id));
        out
    }

    /// **What the server sends a freshly-made character** — the flags the DBC
    /// states for this race and class, and a standing delta of zero.
    ///
    /// vmangos' `ReputationMgr::Initialize` plus `GetDefaultStateFlags`, which
    /// between them are exactly this: every row with a `reputationListID` gets a
    /// slot, and the slot's flags are `ReputationFlags[GetIndexFitTo(race,
    /// class)]`. Nothing here invents a standing — a character who has done
    /// nothing has a delta of zero everywhere and the *base* is what a bar shows.
    ///
    /// Used by `vale reputation` and by the headless probes, which have no
    /// server: without it every check of this panel would be a check of an empty
    /// one, which is the shape of "excused number" this project keeps retiring.
    pub fn default_states(&self, race: u8, class: u8) -> [(u8, i32); SLOTS] {
        let mut out = [(0u8, 0i32); SLOTS];
        for row in self.0.values() {
            let rep = row.reputation_list_id;
            if rep < 0 || rep as usize >= SLOTS {
                continue;
            }
            if let Some(i) = Factions::fitting(row, race, class) {
                out[rep as usize] = (row.flags[i] as u8, 0);
            }
        }
        out
    }

    /// **Which of a row's four alternatives this character is**, or `None`.
    ///
    /// A zero race mask means "any race", a zero class mask means
    /// "any class", and a pair that is zero in *both* is an unused slot rather
    /// than a match for everybody. The masks are one-based — `1 << (race - 1)` —
    /// which is the same convention `SkillRaceClassInfo` uses.
    ///
    /// The client examines all four and lets the **last** match win. That is
    /// the client's own behaviour (there is no early exit) and it matters
    /// nowhere in the shipped file, where no row has two matching alternatives
    /// for one character; it is mirrored anyway, because "mirrored" is checkable
    /// and "equivalent on this data" is a claim about data that could change.
    pub fn fitting(row: &Faction, race: u8, class: u8) -> Option<usize> {
        let race_bit = 1u32 << race.saturating_sub(1);
        let class_bit = 1u32 << class.saturating_sub(1);
        let mut found = None;
        for i in 0..fields::ALTERNATIVES {
            let (races, classes) = (row.race_mask[i], row.class_mask[i]);
            if races == 0 && classes == 0 {
                continue;
            }
            if races != 0 && races & race_bit == 0 {
                continue;
            }
            if classes != 0 && classes & class_bit == 0 {
                continue;
            }
            found = Some(i);
        }
        found
    }
}

/// One reputation-list slot: what the server said, plus the base the DBC says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Slot {
    /// The faction this slot is, or `0` for a slot no row claims.
    pub faction_id: u32,
    pub flags: u8,
    /// `reputationBase` for this character — see [`Factions::fitting`].
    pub base: i32,
    /// **The wire's number, which is a delta.** [`Slot::total`] is what a bar
    /// is drawn from.
    pub standing: i32,
}

impl Slot {
    /// The number every other read here is derived from.
    pub fn total(&self) -> i32 {
        self.base.saturating_add(self.standing)
    }
}

/// One line of the panel: a heading or a bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The slot this row draws, or `-1` for a heading that is not itself a
    /// faction one can have standing with.
    pub reputation_list_id: i32,
    /// `-1` is the *Inactive* heading and `0` the *Other* one; see the module
    /// note.
    pub faction_id: i32,
    pub is_header: bool,
    /// The heading this row belongs under — its own faction id if it is one.
    pub group: i32,
    /// Filled by the recount: where `group` sits in [`Reputation::headers`], or
    /// `headers.len()` for a row whose heading is not registered.
    pub header_slot: usize,
}

/// One heading, and how many bars are under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub faction_id: i32,
    pub children: usize,
}

/// What `GetFactionInfo` answers, in the order the interface unpacks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub name: String,
    pub description: String,
    /// **One-based**, because `FACTION_STANDING_LABEL1` is Hated.
    pub standing_id: usize,
    pub bar_min: i32,
    pub bar_max: i32,
    pub bar_value: i32,
    pub at_war: bool,
    pub can_toggle_at_war: bool,
    pub is_header: bool,
    pub is_collapsed: bool,
    pub is_watched: bool,
}

/// The `GlobalStrings.lua` keys the two synthetic headings are named by.
pub const OTHER_KEY: &str = "FACTION_OTHER";
pub const INACTIVE_KEY: &str = "FACTION_INACTIVE";

/// **The whole of what the reputation panel reads**, rebuilt the way the client
/// rebuilds it.
///
/// Held by the renderer as one resource and by `vale reputation` as a local;
/// it needs no window, so the panel is checkable without one.
#[derive(Debug, Clone)]
pub struct Reputation {
    slots: [Slot; SLOTS],
    rows: Vec<Row>,
    headers: Vec<Header>,
    /// One bit per heading, **set when expanded** — the mask starts at
    /// `0xffffffff` and every heading registration resets it.
    /// Only the low 32 slots have a bit; a 33rd heading is always
    /// drawn open, which is the client's own behaviour and unreachable in 1.12.
    expanded: u32,
    /// Faction id of the row the detail pane is about, `0` for none.
    selected: i32,
    /// How many rows `GetNumFactions` reports — the visible prefix.
    displayed: usize,
    /// `PLAYER_FIELD_WATCHED_FACTION_INDEX`, a reputation-list id or `-1`.
    watched: i32,
}

impl Default for Reputation {
    fn default() -> Self {
        Reputation {
            slots: [Slot::default(); SLOTS],
            rows: Vec::new(),
            headers: Vec::new(),
            expanded: u32::MAX,
            selected: 0,
            displayed: 0,
            watched: -1,
        }
    }
}

impl Reputation {
    /// **Rebuild from the 64 slots** — which is what
    /// `SMSG_INITIALIZE_FACTIONS` runs and what a character entering the world
    /// runs.
    ///
    /// `flags` and `standing` are the wire's; everything else is derived here.
    /// Takes the whole array rather than being fed slot by slot because the
    /// packet is the server's complete statement, exactly as the spellbook is.
    pub fn rebuild(
        &mut self,
        factions: &Factions,
        race: u8,
        class: u8,
        wire: &[(u8, i32); SLOTS],
    ) {
        for (slot, (flags, standing)) in self.slots.iter_mut().zip(wire.iter()) {
            *slot = Slot { faction_id: 0, flags: *flags, base: 0, standing: *standing };
        }
        self.rows.clear();
        self.headers.clear();
        self.expanded = u32::MAX;

        for row in factions.descending() {
            let rep = row.reputation_list_id;
            if rep < 0 || rep as usize >= SLOTS {
                continue;
            }
            let rep = rep as usize;
            self.slots[rep].faction_id = row.id;
            // The base is the *last* fitting alternative, and the row is only
            // listed if the server called it visible — see `fitting`.
            if let Some(i) = Factions::fitting(row, race, class) {
                self.slots[rep].base = row.base[i];
                if self.slots[rep].flags & flags::VISIBLE != 0 {
                    self.add_row(factions, rep);
                }
            }
        }

        // **The Inactive heading is appended last and collapsed on the spot**.
        // It is not a faction, so it carries `-1` in both columns.
        self.headers.push(Header { faction_id: INACTIVE_FACTION, children: 0 });
        self.expanded = u32::MAX;
        self.rows.push(Row {
            reputation_list_id: INACTIVE_FACTION,
            faction_id: INACTIVE_FACTION,
            is_header: true,
            group: INACTIVE_FACTION,
            header_slot: 0,
        });
        // One-based, like every other index the interface passes.
        let last = self.rows.len();
        self.set_collapsed(factions, last, true);
    }

    /// One slot's row, and the heading its parent implies.
    fn add_row(&mut self, factions: &Factions, rep: usize) {
        if self
            .rows
            .iter()
            .any(|row| row.reputation_list_id == rep as i32)
        {
            return;
        }
        let faction_id = self.slots[rep].faction_id;
        let is_header = self.slots[rep].flags & flags::HEADER != 0;
        let entry = factions.get(faction_id);
        let group = if is_header {
            self.headers.push(Header { faction_id: faction_id as i32, children: 0 });
            self.expanded = u32::MAX;
            faction_id as i32
        } else {
            entry.map(|e| e.parent).unwrap_or(0) as i32
        };
        self.rows.push(Row {
            reputation_list_id: rep as i32,
            faction_id: faction_id as i32,
            is_header,
            group,
            header_slot: 0,
        });
        if is_header {
            return;
        }
        // **A bar with no heading row yet makes one, immediately after itself.**
        // The heading may be a real faction (Steamwheedle Cartel) or the parent
        // `0`, which is the synthetic *Other*.
        if self
            .rows
            .iter()
            .any(|row| row.is_header && row.faction_id == group)
        {
            return;
        }
        let parent_rep = factions
            .get(group.max(0) as u32)
            .map(|e| e.reputation_list_id)
            .unwrap_or(-1);
        self.rows.push(Row {
            reputation_list_id: parent_rep,
            faction_id: group,
            is_header: true,
            group,
            header_slot: 0,
        });
        self.headers.push(Header { faction_id: group, children: 0 });
        self.expanded = u32::MAX;
    }

    /// **The recount** — run after every change that can move a row.
    ///
    /// Three things in one pass, and the order is the client's: count each
    /// heading's bars, sort the headings, then assign every row its heading slot,
    /// count what is displayed and sort the rows.
    fn recount(&mut self, factions: &Factions) {
        for slot in 0..self.headers.len() {
            let id = self.headers[slot].faction_id;
            self.headers[slot].children = (0..self.rows.len())
                .filter(|&i| !self.rows[i].is_header && self.group_of(i) == id)
                .count();
        }

        // An empty heading last; then by localised name; a heading
        // with no DBC row (Other, Inactive) after one that has; and those two
        // between themselves by faction id, which puts Other above Inactive.
        self.headers.sort_by(|a, b| {
            let empty = (a.children == 0, b.children == 0);
            match empty {
                (true, true) => return std::cmp::Ordering::Equal,
                (true, false) => return std::cmp::Ordering::Greater,
                (false, true) => return std::cmp::Ordering::Less,
                _ => {}
            }
            let names = (
                factions.get(a.faction_id.max(0) as u32),
                factions.get(b.faction_id.max(0) as u32),
            );
            match names {
                (Some(x), Some(y)) => x.name.cmp(&y.name),
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, None) => a.faction_id.cmp(&b.faction_id),
            }
        });

        self.displayed = self.rows.len();
        for i in 0..self.rows.len() {
            let group = self.group_of(i);
            let slot = self
                .headers
                .iter()
                .position(|h| h.faction_id == group)
                .unwrap_or(self.headers.len());
            self.rows[i].header_slot = slot;
            if !self.rows[i].is_header && self.is_collapsed_slot(slot) {
                self.displayed -= 1;
            }
            if self.rows[i].is_header
                && self.headers.get(slot).is_some_and(|h| h.children == 0)
            {
                self.displayed -= 1;
            }
        }

        // Hidden rows to the end, then by heading slot, then the
        // heading itself before its bars, then the bars by localised name.
        let expanded = self.expanded;
        let headers = self.headers.len();
        let hidden = |row: &Row| {
            !row.is_header
                && row.header_slot < 32
                && expanded & (1u32 << row.header_slot) == 0
                && row.header_slot < headers
        };
        self.rows.sort_by(|a, b| {
            if a.header_slot != b.header_slot {
                return match (hidden(a), hidden(b)) {
                    (false, true) => std::cmp::Ordering::Less,
                    (true, false) => std::cmp::Ordering::Greater,
                    (true, true) => std::cmp::Ordering::Equal,
                    (false, false) => a.header_slot.cmp(&b.header_slot),
                };
            }
            match (a.is_header, b.is_header) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                (true, true) => std::cmp::Ordering::Equal,
                (false, false) => {
                    let names = (
                        factions.get(a.faction_id.max(0) as u32),
                        factions.get(b.faction_id.max(0) as u32),
                    );
                    match names {
                        (Some(x), Some(y)) => x.name.cmp(&y.name),
                        _ => std::cmp::Ordering::Equal,
                    }
                }
            }
        });
    }

    /// **Which heading a row is filed under, right now** — and the one place
    /// *inactive* is applied.
    ///
    /// A row the player marked inactive answers `-1` whatever its
    /// parent says, which is how the Inactive heading fills.
    fn group_of(&self, row: usize) -> i32 {
        let row = &self.rows[row];
        if self.slot_of(row).is_some_and(|s| s.flags & flags::INACTIVE != 0) {
            return INACTIVE_FACTION;
        }
        row.group
    }

    fn slot_of(&self, row: &Row) -> Option<&Slot> {
        usize::try_from(row.reputation_list_id)
            .ok()
            .and_then(|rep| self.slots.get(rep))
    }

    fn is_collapsed_slot(&self, slot: usize) -> bool {
        slot < 32 && slot < self.headers.len() && self.expanded & (1u32 << slot) == 0
    }

    // --- what the interface reads ---------------------------------------

    /// `GetNumFactions()` — **the visible prefix**, not the row count.
    pub fn num_factions(&self) -> usize {
        self.displayed
    }

    /// `GetFactionInfo(index)`, one-based. `None` for an index past the end,
    /// which is what the panel's own loop relies on.
    pub fn info(&self, factions: &Factions, index: usize) -> Option<Info> {
        let row = self.rows.get(index.checked_sub(1)?)?;
        let collapsed = self.is_collapsed_slot(row.header_slot);
        let entry = factions.get(row.faction_id.max(0) as u32);
        // A heading with no DBC row is one of the two synthetic
        // ones, and its name is a key rather than a word.
        let Some(entry) = entry.filter(|_| row.faction_id > 0) else {
            return Some(Info {
                name: if row.faction_id == INACTIVE_FACTION {
                    INACTIVE_KEY.to_string()
                } else {
                    OTHER_KEY.to_string()
                },
                description: String::new(),
                standing_id: 1,
                bar_min: 0,
                bar_max: 0,
                bar_value: 0,
                at_war: false,
                can_toggle_at_war: false,
                is_header: true,
                is_collapsed: collapsed,
                is_watched: false,
            });
        };
        let slot = self.slot_of(row).copied().unwrap_or_default();
        let standing = slot.total();
        let rank = rank_of(standing);
        Some(Info {
            name: entry.name.clone(),
            description: entry.description.clone(),
            standing_id: rank + 1,
            bar_min: RANK_FLOORS[rank],
            bar_max: RANK_FLOORS[rank + 1],
            bar_value: standing,
            at_war: slot.flags & flags::AT_WAR != 0,
            // Below Hostile's floor it can never be unticked, and
            // one's own side is peace-forced.
            can_toggle_at_war: standing >= RANK_FLOORS[2]
                && slot.flags & flags::PEACE_FORCED == 0,
            is_header: row.is_header,
            is_collapsed: collapsed,
            is_watched: row.reputation_list_id == self.watched,
        })
    }

    /// `IsFactionInactive(index)`, one-based.
    pub fn is_inactive(&self, index: usize) -> bool {
        index
            .checked_sub(1)
            .and_then(|i| self.rows.get(i))
            .and_then(|row| self.slot_of(row))
            .is_some_and(|slot| slot.flags & flags::INACTIVE != 0)
    }

    /// `GetSelectedFaction()` — **a display index, not an id**, found by
    /// searching the rows for the stored faction. `0` when the
    /// selected faction is not on screen, which is what hides the detail pane.
    pub fn selected(&self) -> usize {
        if self.selected == 0 {
            return 0;
        }
        self.rows
            .iter()
            .take(self.displayed)
            .position(|row| row.faction_id == self.selected)
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    /// `SetSelectedFaction(index)`.
    pub fn select(&mut self, index: usize) {
        self.selected = self.faction_at(index);
    }

    /// `GetWatchedFactionInfo()` -> `(name, standingID, barMin, barMax, barValue)`.
    ///
    /// Reads the *slot* rather than a row, so a watched faction
    /// under a collapsed heading still draws its bar over the action bar — which
    /// is the reference's behaviour and the reason this is not `info()`.
    pub fn watched(&self, factions: &Factions) -> Option<(String, usize, i32, i32, i32)> {
        let rep = usize::try_from(self.watched).ok()?;
        let slot = self.slots.get(rep)?;
        let entry = factions.get(slot.faction_id)?;
        let standing = slot.total();
        let rank = rank_of(standing);
        Some((
            entry.name.clone(),
            rank + 1,
            RANK_FLOORS[rank],
            RANK_FLOORS[rank + 1],
            standing,
        ))
    }

    /// The reputation-list id `SetWatchedFactionIndex(index)` would send, and
    /// which `PLAYER_FIELD_WATCHED_FACTION_INDEX` comes back as.
    pub fn watched_index(&self) -> i32 {
        self.watched
    }

    /// …set from the field, which is the server's copy and the authority.
    pub fn set_watched(&mut self, reputation_list_id: i32) {
        self.watched = reputation_list_id;
    }

    /// The reputation-list id a display index names, for the three verbs that
    /// send one. `-1` for a heading that is not a faction.
    pub fn reputation_id_at(&self, index: usize) -> i32 {
        index
            .checked_sub(1)
            .and_then(|i| self.rows.get(i))
            .map(|row| row.reputation_list_id)
            .unwrap_or(-1)
    }

    /// …and the faction id, which is what the selection is stored as.
    fn faction_at(&self, index: usize) -> i32 {
        index
            .checked_sub(1)
            .and_then(|i| self.rows.get(i))
            .map(|row| row.faction_id)
            .unwrap_or(0)
    }

    // --- what the interface changes ---------------------------------------

    /// `CollapseFactionHeader` / `ExpandFactionHeader`, one-based. A row that is not a heading does nothing, which is the
    /// client's own guard.
    pub fn set_collapsed(&mut self, factions: &Factions, index: usize, collapsed: bool) {
        let Some(row) = index.checked_sub(1).and_then(|i| self.rows.get(i)) else {
            return;
        };
        if !row.is_header {
            return;
        }
        let group = row.group;
        let Some(slot) = self.headers.iter().position(|h| h.faction_id == group) else {
            return;
        };
        if slot >= 32 {
            return;
        }
        if collapsed {
            self.expanded &= !(1u32 << slot);
        } else {
            self.expanded |= 1u32 << slot;
        }
        self.recount(factions);
    }

    /// `SetFactionActive` / `SetFactionInactive`, one-based. Local only — the
    /// server is told separately and says nothing back.
    pub fn set_inactive(&mut self, factions: &Factions, index: usize, inactive: bool) {
        self.set_flag(factions, index, flags::INACTIVE, inactive);
    }

    /// `FactionToggleAtWar`, one-based. Returns the new state, which is what the
    /// caller owes the server.
    pub fn toggle_at_war(&mut self, factions: &Factions, index: usize) -> Option<(i32, bool)> {
        let rep = self.reputation_id_at(index);
        let now = usize::try_from(rep)
            .ok()
            .and_then(|r| self.slots.get(r))
            .map(|s| s.flags & flags::AT_WAR != 0)?;
        self.set_flag(factions, index, flags::AT_WAR, !now);
        Some((rep, !now))
    }

    fn set_flag(&mut self, factions: &Factions, index: usize, bit: u8, on: bool) {
        let rep = self.reputation_id_at(index);
        let Ok(rep) = usize::try_from(rep) else {
            return;
        };
        let Some(slot) = self.slots.get_mut(rep) else {
            return;
        };
        if on {
            slot.flags |= bit;
        } else {
            slot.flags &= !bit;
        }
        self.recount(factions);
    }

    // --- what the server changes -----------------------------------------

    /// `SMSG_SET_FACTION_STANDING`: one slot's delta moved.
    pub fn apply_standing(&mut self, factions: &Factions, rep: usize, standing: i32) {
        if let Some(slot) = self.slots.get_mut(rep) {
            slot.standing = standing;
            self.recount(factions);
        }
    }

    /// `SMSG_SET_FACTION_VISIBLE`: a faction met for the first time. Adds the row and its heading if it is new.
    pub fn apply_visible(&mut self, factions: &Factions, rep: usize) {
        if self.slots.get(rep).is_none_or(|s| s.flags & flags::VISIBLE != 0) {
            return;
        }
        self.add_row(factions, rep);
        if let Some(slot) = self.slots.get_mut(rep) {
            slot.flags |= flags::VISIBLE;
        }
        self.recount(factions);
    }

    /// `SMSG_SET_FACTION_ATWAR`: the server's own statement about the box.
    pub fn apply_at_war(&mut self, factions: &Factions, rep: usize, at_war: bool) {
        if let Some(slot) = self.slots.get_mut(rep) {
            if at_war {
                slot.flags |= flags::AT_WAR;
            } else {
                slot.flags &= !flags::AT_WAR;
            }
            self.recount(factions);
        }
    }

    /// Every row, for `vale reputation` and the tests. Not what the panel
    /// reads — see [`Self::info`].
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn headers(&self) -> &[Header] {
        &self.headers
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// **The sixty-four slots as friend-or-foe reads them** — faction id, the
    /// rank the total sits in, and the at-war bit.
    ///
    /// One allocation per rebuild rather than a lookup per unit per frame: the
    /// reaction rule wants a slice it can scan
    /// ([`crate::tables::faction::Standing`]), the list only changes when a
    /// reputation packet arrives, and slots no row claims are dropped here so
    /// the scan is over the twenty-odd factions the character actually has.
    ///
    /// **A faction absent from this list is one with no reputation bar**, which
    /// is the same test the client makes against `Faction.dbc`'s
    /// `reputationListID` — the two agree because a faction with a list id is
    /// exactly a faction with a slot.
    pub fn standings(&self) -> Vec<(u32, crate::tables::faction::FactionState)> {
        self.slots
            .iter()
            .filter(|slot| slot.faction_id != 0)
            .map(|slot| {
                (
                    slot.faction_id,
                    crate::tables::faction::FactionState {
                        rank: crate::tables::faction::Rank::from_index(
                            rank_of(slot.total()) as u32
                        ),
                        at_war: slot.flags & flags::AT_WAR != 0,
                    },
                )
            })
            .collect()
    }
}

/// The faction id the *Inactive* heading carries — not a faction, and negative
/// so that it can never collide with one.
pub const INACTIVE_FACTION: i32 = -1;

/// …and the *Other* heading's, which is `parentFactionID`'s own "none".
pub const OTHER_FACTION: i32 = 0;

#[cfg(test)]
mod tests {
    use super::*;

    /// The bands are the client's, and both ends of every bar come off one
    /// array — so an off-by-one here is a bar that fills at the wrong rate
    /// rather than a failure.
    #[test]
    fn the_eight_standings_are_the_client_s_own_bands() {
        assert_eq!(rank_of(-42000), 0, "Hated's floor");
        assert_eq!(rank_of(-6001), 0);
        assert_eq!(rank_of(-6000), 1, "Hostile");
        assert_eq!(rank_of(-3000), 2, "Unfriendly");
        assert_eq!(rank_of(-1), 2);
        assert_eq!(rank_of(0), 3, "Neutral");
        assert_eq!(rank_of(2999), 3);
        assert_eq!(rank_of(3000), 4, "Friendly");
        assert_eq!(rank_of(9000), 5, "Honored");
        assert_eq!(rank_of(21000), 6, "Revered");
        assert_eq!(rank_of(42000), 7, "Exalted");
        assert_eq!(rank_of(42999), 7, "…and the cap is inside it");
        // vmangos' cumulative widths, from the other side of the wire.
        let widths = [36000, 3000, 3000, 3000, 6000, 12000, 21000, 1000];
        let mut floor = -42000;
        for (rank, width) in widths.iter().enumerate() {
            assert_eq!(RANK_FLOORS[rank], floor, "rank {rank}");
            floor += width;
        }
        assert_eq!(RANK_FLOORS[8], floor);
    }

    /// A zero mask is "any", a pair of zeroes is an unused alternative — the
    /// distinction that decides whether a neutral faction is listed for
    /// everybody or for nobody.
    #[test]
    fn a_zero_mask_is_any_and_two_zeroes_is_no_alternative_at_all() {
        let mut row = Faction { id: 1, ..Faction::default() };
        assert_eq!(Factions::fitting(&row, 1, 1), None, "all four unused");
        row.race_mask[0] = 0;
        row.class_mask[0] = 0;
        row.base[0] = 5;
        row.race_mask[1] = 0b0000_0001; // human
        row.class_mask[1] = 0;
        assert_eq!(Factions::fitting(&row, 1, 1), Some(1), "human takes its own");
        assert_eq!(Factions::fitting(&row, 2, 1), None, "an orc takes neither");
        row.class_mask[2] = 0b0000_0100; // hunter
        row.race_mask[2] = 0;
        assert_eq!(Factions::fitting(&row, 2, 3), Some(2), "any race, hunter only");
    }

    /// A minimal world: one heading with two bars, one parentless bar that
    /// implies *Other*, and the *Inactive* heading the build always appends.
    fn world() -> (Factions, Reputation) {
        let mut map = std::collections::HashMap::new();
        let mut add = |id: u32, rep: i32, parent: u32, name: &str, flags: u32| {
            map.insert(
                id,
                Faction {
                    id,
                    reputation_list_id: rep,
                    race_mask: [0xffff_ffff, 0, 0, 0],
                    class_mask: [0, 0, 0, 0],
                    base: [0, 0, 0, 0],
                    flags: [flags, 0, 0, 0],
                    parent,
                    name: name.to_string(),
                    description: format!("about {name}"),
                },
            );
        };
        add(169, 10, 0, "Steamwheedle Cartel", flags::HEADER as u32);
        add(21, 1, 169, "Booty Bay", 0);
        add(369, 7, 169, "Gadgetzan", 0);
        add(529, 13, 0, "Argent Dawn", 0);
        let factions = Factions(map);
        let mut wire = [(0u8, 0i32); SLOTS];
        for rep in [1usize, 7, 10, 13] {
            wire[rep] = (flags::VISIBLE, 0);
        }
        wire[10].0 |= flags::HEADER;
        let mut rep = Reputation::default();
        rep.rebuild(&factions, 1, 1, &wire);
        (factions, rep)
    }

    /// The whole shape in one assertion: two headings plus *Other* and
    /// *Inactive*, the empty one hidden, each heading before its own bars, and
    /// the bars alphabetical.
    #[test]
    fn the_list_is_headings_first_then_bars_by_name_with_the_empty_heading_hidden() {
        let (factions, rep) = world();
        let seen: Vec<(String, bool)> = (1..=rep.num_factions())
            .filter_map(|i| rep.info(&factions, i))
            .map(|info| (info.name, info.is_header))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("Steamwheedle Cartel".to_string(), true),
                ("Booty Bay".to_string(), false),
                ("Gadgetzan".to_string(), false),
                (OTHER_KEY.to_string(), true),
                ("Argent Dawn".to_string(), false),
            ],
            "…and FACTION_INACTIVE is not in it, because it has no children"
        );
        // The Inactive heading is a row all the same — it is only undisplayed.
        assert_eq!(rep.rows().len(), rep.num_factions() + 1);
    }

    /// Collapsing a heading takes its bars off the list without taking them out
    /// of it, which is the prefix invariant the panel depends on.
    #[test]
    fn a_collapsed_heading_shortens_the_prefix_and_keeps_the_rows() {
        let (factions, mut rep) = world();
        let before = rep.num_factions();
        rep.set_collapsed(&factions, 1, true);
        assert_eq!(rep.num_factions(), before - 2, "Booty Bay and Gadgetzan");
        let info = rep.info(&factions, 1).expect("the heading is still first");
        assert!(info.is_collapsed && info.is_header);
        for i in 1..=rep.num_factions() {
            let info = rep.info(&factions, i).unwrap();
            assert_ne!(info.name, "Booty Bay", "a hidden row is past the prefix");
        }
        rep.set_collapsed(&factions, 1, false);
        assert_eq!(rep.num_factions(), before, "…and it comes back");
    }

    /// Marking a faction inactive **moves** it under the Inactive heading, which
    /// is a different thing from hiding it — and it makes that heading appear.
    #[test]
    fn inactive_moves_a_row_rather_than_hiding_it() {
        let (factions, mut rep) = world();
        let argent = (1..=rep.num_factions())
            .find(|&i| rep.info(&factions, i).unwrap().name == "Argent Dawn")
            .expect("listed under Other");
        rep.set_inactive(&factions, argent, true);
        assert!(rep.is_inactive(rep.rows().iter().position(|r| r.faction_id == 529).unwrap() + 1));
        let names: Vec<String> = (1..=rep.num_factions())
            .map(|i| rep.info(&factions, i).unwrap().name)
            .collect();
        assert!(names.contains(&INACTIVE_KEY.to_string()), "the heading appeared");
        assert!(!names.contains(&OTHER_KEY.to_string()), "…and Other emptied");
        // It is still listed — under the new heading rather than the old one.
        let under = names.iter().position(|n| n == "Argent Dawn").unwrap();
        let heading = names.iter().position(|n| n == INACTIVE_KEY).unwrap();
        assert!(heading < under, "a heading precedes its own bars");
    }

    /// **The collapse mask is indexed by heading *slot*, and the recount
    /// re-sorts the headings** — so a heading that changes position inherits
    /// whichever bit its new neighbour had.
    ///
    /// That is the client's own aliasing rather than a simplification here:
    /// the client finds the slot, sets the bit and only then goes into the
    /// recount, whose `qsort` reorders the pointer array the bit
    /// indexes. Recorded as a test because the obvious "fix" — keying collapse
    /// on the faction id — would make this client disagree with the reference
    /// in a way nothing else would catch.
    #[test]
    fn collapse_follows_the_slot_and_the_slots_are_re_sorted() {
        let (factions, mut rep) = world();
        // Steamwheedle is slot 0 and Other slot 1; collapse the first.
        rep.set_collapsed(&factions, 1, true);
        assert_eq!(rep.info(&factions, 1).unwrap().name, "Steamwheedle Cartel");
        assert!(rep.info(&factions, 1).unwrap().is_collapsed);
        // Empty Steamwheedle out. It sorts last, and slot 0 becomes Other —
        // which now reads as the collapsed one.
        let booty = rep.rows().iter().position(|r| r.faction_id == 21).unwrap() + 1;
        rep.set_inactive(&factions, booty, true);
        let gadget = rep.rows().iter().position(|r| r.faction_id == 369).unwrap() + 1;
        rep.set_inactive(&factions, gadget, true);
        let first = rep.info(&factions, 1).unwrap();
        assert_ne!(first.name, "Steamwheedle Cartel", "it emptied and sorted away");
        assert!(first.is_collapsed, "…and the bit stayed with the slot");
    }

    /// The wire's number is a delta and the DBC's base is the rest of it —
    /// the one arithmetic error that draws a plausible bar for the wrong rank.
    #[test]
    fn the_standing_is_the_packet_s_delta_plus_the_race_s_base() {
        let mut map = std::collections::HashMap::new();
        map.insert(
            72,
            Faction {
                id: 72,
                reputation_list_id: 19,
                race_mask: [0b0000_0001, 0b0000_0010, 0, 0],
                class_mask: [0, 0, 0, 0],
                base: [3000, -42000, 0, 0],
                flags: [flags::VISIBLE as u32, 0, 0, 0],
                parent: 0,
                name: "Stormwind".into(),
                description: String::new(),
            },
        );
        let factions = Factions(map);
        let mut wire = [(0u8, 0i32); SLOTS];
        wire[19] = (flags::VISIBLE, 1000);

        // Index 1 is the *Other* heading Stormwind's parentless row implies;
        // the bar is under it.
        let mut human = Reputation::default();
        human.rebuild(&factions, 1, 1, &wire);
        let info = human.info(&factions, 2).unwrap();
        assert_eq!(info.bar_value, 4000, "3000 base + 1000 earned");
        assert_eq!(info.standing_id, 5, "Friendly, one-based");

        let mut orc = Reputation::default();
        orc.rebuild(&factions, 2, 1, &wire);
        let info = orc.info(&factions, 2).unwrap();
        assert_eq!(info.bar_value, -41000, "the other alternative's base");
        assert_eq!(info.standing_id, 1, "Hated");
        assert!(!info.can_toggle_at_war, "below Hostile's floor it is stuck");
    }

    /// `GetSelectedFaction` answers a **display index**, so it moves when the
    /// list does — and answers 0 rather than a stale row when the selection
    /// leaves the prefix.
    #[test]
    fn the_selection_is_stored_as_a_faction_and_read_as_an_index() {
        let (factions, mut rep) = world();
        let gadgetzan = (1..=rep.num_factions())
            .find(|&i| rep.info(&factions, i).unwrap().name == "Gadgetzan")
            .unwrap();
        rep.select(gadgetzan);
        assert_eq!(rep.selected(), gadgetzan);
        rep.set_collapsed(&factions, 1, true);
        assert_eq!(rep.selected(), 0, "off the list, so the detail pane hides");
        rep.set_collapsed(&factions, 1, false);
        assert_eq!(rep.selected(), gadgetzan, "…and back");
    }

    /// A faction met for the first time arrives as one packet naming one slot,
    /// and it has to bring its heading with it.
    #[test]
    fn a_newly_visible_faction_brings_its_heading() {
        let (factions, rep) = world();
        // Take Booty Bay out and put it back the way the server would.
        let mut wire = [(0u8, 0i32); SLOTS];
        wire[10] = (flags::VISIBLE | flags::HEADER, 0);
        wire[13] = (flags::VISIBLE, 0);
        let mut fresh = Reputation::default();
        fresh.rebuild(&factions, 1, 1, &wire);
        let before = fresh.num_factions();
        assert!(
            !(1..=before).any(|i| fresh.info(&factions, i).unwrap().name
                == "Steamwheedle Cartel"),
            "an empty heading is not displayed"
        );
        fresh.apply_visible(&factions, 1);
        // **Two lines, not one**: the bar, and the heading that was a row all
        // along and was only undisplayed for having no children.
        assert_eq!(fresh.num_factions(), before + 2);
        assert!(fresh.rows().iter().any(|r| r.faction_id == 21));
        let _ = rep;
    }
}
