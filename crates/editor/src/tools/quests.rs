//! The quest tool's state: the server's quests, which one is open, which
//! creatures and game objects give and take each, and the edits to them.
//!
//! ## The quest tool is a workspace, not a pointer tool
//!
//! A quest has no place in the world, so, as with [`super::items`], its
//! workspace takes the middle of the window. [`crate::ui::quests`] draws it.
//! The open quest and the edit operations are kept here rather than in `ui/`
//! so that `--quest 783` can put the tool in a known state before anything is
//! drawn.
//!
//! ## What is read from the database
//!
//! ```text
//! quest_template   4,433 rows on the reference database, as twelve columns a
//!                  list row and a chain need    vale_mangos::quest::all_quests_query
//! the relations    four tables, about 8,400 rows of (id, quest), and the name of
//!                  each of the ~1,850 creatures and game objects they mention
//! ```
//!
//! Both templates and relations are read in full. The relations are read
//! whole rather than per quest because they are looked up in both
//! directions: the workspace asks which creatures give a quest, and the
//! creature tool's window asks which quests a creature gives. One list in
//! memory answers both with a filter. They are read with the templates, on
//! one task, whenever an apply lands.
//!
//! ## Relations are rows in the project's store
//!
//! [`Relation::key`] is `(id, quest)` and its table is one of the four, so
//! adding one is a [`Life::Insert`] row carrying the patch band, removing one
//! that is in the database is a [`Life::Delete`] row, and removing one this
//! project added takes the claim back. [`Quests::relations_of_quest`] and
//! [`Quests::relations_of_holder`] return the database's rows with the
//! project's rows over them, each marked with what the project says is to
//! become of it.
//!
//! ## The names a quest's ids resolve to are fetched in batches
//!
//! A quest row names up to eighteen items and four creatures or game objects.
//! The form shows each by name (item `2589` as Linen Cloth) so that the row
//! can be checked by reading it. [`Quests::item`] and [`Quests::holder`]
//! answer from a cache and note a miss; [`fetch_the_names`] turns the misses
//! into one query per kind, on each frame that has any. An id the database
//! does not hold is cached as absent, so it is asked for once.
//!
//! ## The creature and game-object tools reach this through two fields
//!
//! [`Quests::window_for`] is the creature or game object whose quests that
//! tool's window is showing, and [`Quests::of_holder`] narrows the workspace's
//! list to one holder's quests. Both are set by the Quests button on a
//! selection — see [`crate::ui::creatures`] and [`crate::ui::gameobjects`] —
//! and neither is a dependency of this tool on those: they name a kind of
//! holder and a template entry, which is all a relation holds.

use super::Tool;
use crate::server::fresh::{claimed, Fresh};
use crate::session::EditSession;
use vale_mangos::quest::{self, RowValue};
use vale_mangos::row::{Edits, Key, Life, RowEdit};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::{HashMap, HashSet};

/// One row of the list, as the columns
/// [`vale_mangos::quest::all_quests_query`] reads.
#[derive(Debug, Clone)]
pub struct Known {
    /// The entry the project says the quest has: what the list shows, what
    /// the panel opens by and what the store keys its claim under.
    pub entry: u32,
    /// The entry the last read found it at, which differs while the
    /// project changes the quest's entry and has not applied it. See
    /// [`fold_the_store_in`].
    pub read_entry: u32,
    /// Which content patch of the row the server would load — the other half of
    /// the key an edit is written under.
    pub patch: u32,
    pub title: String,
    pub method: u32,
    pub zone_or_sort: i32,
    pub min_level: u32,
    pub level: u32,
    /// `Type`: a `QuestInfo.dbc` id.
    pub kind: u32,
    pub special_flags: u32,
    pub prev: i32,
    pub next: i32,
    pub next_in_chain: u32,
    /// What `zone_or_sort` is called, filled in once from `AreaTable.dbc`
    /// and `QuestSort.dbc` by the panel, which is what has the archives — see
    /// [`Quests::name_the_zones`]. Empty until then, and for a quest filed
    /// under nothing.
    pub zone: String,
    /// What this project says is to become of the row.
    pub claim: Life,
}

impl Known {
    /// Its key: the entry and the patch together.
    pub fn key(&self) -> Key {
        quest::template_key(self.entry, self.patch)
    }

    /// The second line of a list row: its level, where it is filed, what kind
    /// of quest it is, and whether it auto-completes, is disabled or is
    /// repeatable.
    pub fn sub(&self) -> String {
        let mut parts = vec![format!("level {}", self.level)];
        if !self.zone.is_empty() {
            parts.push(self.zone.clone());
        }
        if self.kind != 0 {
            parts.push(quest::value_word(&quest::TYPES, self.kind));
        }
        match self.method {
            0 => parts.push("auto-complete".to_string()),
            1 => parts.push("disabled".to_string()),
            _ => {}
        }
        if self.special_flags & 1 != 0 {
            parts.push("repeatable".to_string());
        }
        parts.join(" \u{b7} ")
    }

    /// This row with the project's edits over it, or `None` when the
    /// project says nothing about it.
    pub fn with_edits(&self, edits: &Edits) -> Option<Known> {
        let key = self.key();
        let life = edits.life(quest::TEMPLATE, &key);
        if !edits.touches(quest::TEMPLATE, &key) && life == self.claim {
            return None;
        }
        let get = |column: &str| edits.get(quest::TEMPLATE, &key, column);
        let number = |column: &str, had: i64| {
            get(column)
                .and_then(|value| value.trim().parse::<i64>().ok())
                .unwrap_or(had)
        };
        let zone_or_sort = number("ZoneOrSort", self.zone_or_sort as i64) as i32;
        Some(Known {
            title: get("Title")
                .map(crate::ui::rowform::unquote)
                .unwrap_or_else(|| self.title.clone()),
            method: number("Method", self.method as i64) as u32,
            zone_or_sort,
            min_level: number("MinLevel", self.min_level as i64) as u32,
            level: number("QuestLevel", self.level as i64) as u32,
            kind: number("Type", self.kind as i64) as u32,
            special_flags: number("SpecialFlags", self.special_flags as i64) as u32,
            prev: number("PrevQuestId", self.prev as i64) as i32,
            next: number("NextQuestId", self.next as i64) as i32,
            next_in_chain: number("NextQuestInChain", self.next_in_chain as i64) as u32,
            // A changed heading is named again by the panel; an unchanged one
            // keeps the name it had.
            zone: match zone_or_sort == self.zone_or_sort {
                true => self.zone.clone(),
                false => String::new(),
            },
            claim: life,
            ..self.clone()
        })
    }
}

/// One row of [`vale_mangos::quest::all_quests_query`], as a [`Known`].
fn read_known(row: &quest::Row) -> Option<Known> {
    let integer = |column: &str| row.integer(column).unwrap_or(0);
    let entry = row.integer("entry")? as u32;
    Some(Known {
        entry,
        read_entry: entry,
        patch: integer("patch") as u32,
        title: row.text("Title").unwrap_or_default().to_string(),
        method: integer("Method") as u32,
        zone_or_sort: integer("ZoneOrSort") as i32,
        min_level: integer("MinLevel") as u32,
        level: integer("QuestLevel") as u32,
        kind: integer("Type") as u32,
        special_flags: integer("SpecialFlags") as u32,
        prev: integer("PrevQuestId") as i32,
        next: integer("NextQuestId") as i32,
        next_in_chain: integer("NextQuestInChain") as u32,
        zone: String::new(),
        claim: Life::Update,
    })
}

/// One row of `quest_template`, whole.
#[derive(Debug, Clone)]
pub struct QuestRow {
    pub entry: u32,
    pub patch: u32,
    pub row: quest::Row,
}

/// Whether a relation's `id` is a creature's entry or a game object's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Holder {
    Creature,
    Object,
}

impl Holder {
    pub fn word(self) -> &'static str {
        match self {
            Holder::Creature => "creature",
            Holder::Object => "game object",
        }
    }

    /// The template table its name is in.
    pub fn template(self) -> &'static str {
        match self {
            Holder::Creature => vale_mangos::creature::TEMPLATE,
            Holder::Object => vale_mangos::gameobject::TEMPLATE,
        }
    }
}

/// Which end of a quest a relation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Hands the quest out: `*_questrelation`.
    Gives,
    /// Takes it back: `*_involvedrelation`.
    Takes,
}

impl Role {
    pub fn word(self) -> &'static str {
        match self {
            Role::Gives => "gives",
            Role::Takes => "takes",
        }
    }
}

/// One row of one of the four relation tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Relation {
    pub holder: Holder,
    pub role: Role,
    /// The creature's or game object's template entry.
    pub id: u32,
    pub quest: u32,
}

impl Relation {
    /// Which of the four tables the row is in.
    pub fn table(&self) -> &'static str {
        table_of(self.holder, self.role)
    }

    pub fn key(&self) -> Key {
        quest::relation_key(self.id, self.quest)
    }
}

/// The table a holder's role is kept in.
pub fn table_of(holder: Holder, role: Role) -> &'static str {
    match (holder, role) {
        (Holder::Creature, Role::Gives) => quest::CREATURE_GIVES,
        (Holder::Creature, Role::Takes) => quest::CREATURE_TAKES,
        (Holder::Object, Role::Gives) => quest::OBJECT_GIVES,
        (Holder::Object, Role::Takes) => quest::OBJECT_TAKES,
    }
}

/// The holder and role a relation table is for: the inverse of [`table_of`],
/// for a table name read out of the store.
fn relation_of_table(table: &str) -> Option<(Holder, Role)> {
    match table {
        quest::CREATURE_GIVES => Some((Holder::Creature, Role::Gives)),
        quest::CREATURE_TAKES => Some((Holder::Creature, Role::Takes)),
        quest::OBJECT_GIVES => Some((Holder::Object, Role::Gives)),
        quest::OBJECT_TAKES => Some((Holder::Object, Role::Takes)),
        _ => None,
    }
}

/// What the form needs of an item a quest names.
#[derive(Debug, Clone)]
pub struct ItemName {
    pub name: String,
    pub quality: u32,
    pub display_id: u32,
}

/// What a reference picker is choosing from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Item,
    Creature,
    Object,
    Quest,
    /// A client table, by its bare name. Searched by the panel, which has the
    /// session the table is opened through.
    Dbc(&'static str),
    /// A server table a column names by id and that has no name column of its
    /// own (a gossip menu, an equipment set, a loot list), by its name. Listed
    /// by what is in it and who uses it; see `vale_mangos::lists`.
    List(&'static str),
}

/// One row a picker offers.
#[derive(Debug, Clone)]
pub struct Hit {
    pub id: u32,
    pub title: String,
    pub sub: String,
}

/// One column of one server row, as the place a chosen value is written.
///
/// The item form, the creature form and the game object form each open the
/// reference picker and the display picker on a column of their own table.
/// This names the column, the value the database holds there, and the undo
/// entry the write goes on, so the dialogs write through one function.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnTarget {
    pub table: &'static str,
    pub key: vale_mangos::row::Key,
    pub column: &'static str,
    /// The column's value as read from the database. Choosing that value takes
    /// the edit off rather than writing it, because a statement for it would
    /// change nothing. `None` for a row this project creates.
    pub in_database: Option<String>,
    /// The undo entry's label: "Edit item", "Edit creature", "Edit game object".
    pub label: &'static str,
    /// The undo entry's subject, which names the row and the column; see
    /// [`crate::session::Gesture`].
    pub subject: String,
}

impl ColumnTarget {
    /// Write `written` to the column, or clear the edit when `written` is what
    /// the database holds.
    pub fn write(&self, session: &mut EditSession, written: String, now: f64) {
        let value = match Some(&written) == self.in_database.as_ref() {
            true => None,
            false => Some(written),
        };
        session.set_server_edit(
            self.table,
            &self.key,
            self.column,
            value,
            Some(crate::session::Gesture {
                label: self.label,
                subject: &self.subject,
                now,
            }),
        );
    }
}

/// Where a chosen row is written.
#[derive(Debug, Clone, PartialEq)]
pub enum PickFor {
    /// A column of the open quest. `negated` writes the id with a minus sign,
    /// which is how `ZoneOrSort` names a `QuestSort` row and
    /// `ReqCreatureOrGOId` a game object.
    Column { column: &'static str, negated: bool },
    /// A new relation between the open quest and the chosen creature or object.
    Relation { holder: Holder, role: Role },
    /// A new relation between a stated creature and the chosen quest, which is
    /// the creature window's direction.
    QuestOf { holder: Holder, id: u32, role: Role },
    /// A column of another server table's row: the item, creature and game
    /// object forms' use of the same dialog. The target says which row and
    /// column, and [`ColumnTarget::write`] is what a choice becomes.
    ServerColumn(ColumnTarget),
    /// An item to add to a loot set, which is the loot window's use of the
    /// same dialog. The choice is answered into [`Quests::loot_pick`] rather
    /// than written here, because adding a row is `crate::tools::loot`'s
    /// decision: a row already in the set and marked for removal is kept
    /// rather than created again.
    LootItem { table: &'static str, entry: u32 },
    /// An item to add to a vendor list, or a spell to add to a trainer list:
    /// the Vendor and Trainer windows' use of the same dialog. Answered into
    /// [`Quests::service_pick`], because an add is `crate::tools::services`'
    /// decision: it refuses a row the server would skip, and replaces a spell
    /// that is not a teaching spell with the one that teaches it.
    VendorItem { table: &'static str, entry: u32 },
    TrainerSpell { table: &'static str, entry: u32 },
    /// One cell of one row of a script, which is the script window's use of
    /// the same dialog. Answered into [`Quests::script_pick`], because a
    /// script is written whole by `crate::tools::behaviour` and not through
    /// the row store.
    ScriptCell { table: &'static str, id: u32, row: usize, column: &'static str },
}

/// The reference picker's state — see [`crate::ui::quests`], which draws it.
#[derive(Debug)]
pub struct Picker {
    pub target: Target,
    pub purpose: PickFor,
    /// For a column whose sign picks the table: the two targets, positive
    /// first, so the dialog can offer the switch. `None` for an ordinary one.
    pub either: Option<(Target, Target)>,
    pub query: String,
    pub focus: bool,
    pub hits: Vec<Hit>,
    /// What [`Self::hits`] was built for, so an unchanged query is not searched
    /// again on every frame.
    pub built: Option<(Target, String)>,
    /// The `EditSession::database_writes` [`Self::hits`] was built at.
    built_at: Option<u64>,
    task: Option<Task<Result<Vec<Hit>, String>>>,
}

impl Picker {
    pub fn new(target: Target, purpose: PickFor) -> Picker {
        Picker {
            target,
            purpose,
            either: None,
            query: String::new(),
            focus: true,
            hits: Vec::new(),
            built: None,
            built_at: None,
            task: None,
        }
    }

    /// Whether a database search is running.
    pub fn searching(&self) -> bool {
        self.task.is_some()
    }
}

/// How many rows a picker lists. A search that matches more says so.
pub const PICK_LIMIT: usize = 200;

/// What the quest workspace is holding.
#[derive(Resource, Default)]
pub struct Quests {
    /// Every quest the server would load, as the database has it.
    pub all: Vec<Known>,
    /// The quests this project creates, rebuilt from the store when it moves.
    pub created: Vec<Known>,
    created_for: Option<u64>,
    /// The highest entry the table holds at any patch.
    pub max_entry: Option<u32>,
    /// Every relation row the server would load, with each quest under the
    /// entry the project gives it — see [`fold_the_store_in`].
    pub relations: Vec<Relation>,
    /// The relation rows as the read returned them, under the database's
    /// entries. [`Self::relations`] is rebuilt from these when the store moves.
    relations_read: Vec<Relation>,
    /// The name of every creature and game object a relation mentions, and of
    /// any other that has been asked for — `None` for an id the database does
    /// not hold.
    holders: Fresh<HashMap<(Holder, u32), Option<String>>>,
    /// The name, quality and display id of every item named by a quest opened
    /// so far — `None` for an entry the database does not hold.
    items: Fresh<HashMap<u32, Option<ItemName>>>,
    /// What each id of a [`Target::List`] table a form names holds, as the
    /// picker lists it, by table and id — `None` for an id the database does
    /// not hold.
    lists: Fresh<HashMap<(&'static str, u32), Option<Hit>>>,
    /// What has been asked for and is in none of the caches yet.
    wanted_items: HashSet<u32>,
    wanted_holders: HashSet<(Holder, u32)>,
    wanted_lists: HashSet<(&'static str, u32)>,
    /// The name read in progress, and the `EditSession::database_writes` it
    /// was started at: an answer that lands after the counter has moved
    /// describes a database that is gone, and is dropped.
    names_task: Option<(u64, Task<Result<NamesRead, String>>)>,
    task: Option<Task<Result<TableRead, String>>>,
    /// What [`Self::all`] was read for: how many times the session had written
    /// the tables — see `EditSession::database_writes`.
    loaded: Option<u64>,
    /// Why the list is empty, when a read failed or no database is set.
    pub trouble: Option<String>,
    /// What is in the search box.
    pub query: String,
    matches: Vec<usize>,
    built: Option<(String, u64, usize, usize, Option<(Holder, u32)>)>,
    /// Only the quests one creature or game object gives or takes, when
    /// set. The list's own header shows it and clears it.
    pub of_holder: Option<(Holder, u32)>,
    /// The open quest, by entry.
    pub open: Option<u32>,
    /// The open quest's whole row, read when the open quest changes.
    pub row: Option<QuestRow>,
    row_task: Option<Task<Result<Option<QuestRow>, String>>>,
    /// What [`Self::row`] was made at, on `crate::tools::items::Items`'
    /// terms: the counter, and whether the project created the quest.
    row_at: Option<(u64, bool)>,
    /// The reference picker, when one is open.
    pub picker: Option<Picker>,
    /// The pass the picker was last drawn on. Several panels draw the one
    /// dialog (the quest and item workspaces, the loot, behaviour, vendor and
    /// trainer windows, and the creature and game object forms), and two of
    /// them may be up in one frame; the second draw of a pass is skipped, or
    /// egui would see two widgets with one id.
    pub picker_pass: Option<u64>,
    /// The creature or game object whose quests its tool's window is showing,
    /// as the kind of holder, its template entry and its name. `None` when the
    /// window is closed.
    pub window_for: Option<(Holder, u32, String)>,
    /// An item a form names that was clicked, to be opened in the item
    /// workspace. Answered by the shell after everything is drawn, because the
    /// shell holds the tool while the form and the inspector are drawn; see
    /// `crate::ui::draw`.
    pub show_item: Option<u32>,
    /// A quest a form names that was clicked, to be opened in the quest
    /// workspace. The creature and game object forms set it; the quest form
    /// sets [`Self::open`] directly, because it is the workspace.
    pub show_quest: Option<u32>,
    /// A client table's row a form names that was clicked, to be opened in the
    /// table browser. The same mailbox as `crate::tools::items::Items::show_row`,
    /// for the forms that have the quest tool in hand and not the item tool.
    pub show_row: Option<(&'static str, u32)>,
    /// An item chosen for a loot set through the picker, as the set's
    /// table and entry and the item: taken by the loot window on the frame
    /// after — see [`PickFor::LootItem`].
    pub loot_pick: Option<(&'static str, u32, u32)>,
    /// An item or a spell chosen for a vendor or trainer list, as the list's
    /// table and entry and the id chosen: taken by that window on the frame
    /// after. See [`PickFor::VendorItem`].
    pub service_pick: Option<(&'static str, u32, u32)>,
    /// A value chosen for one cell of a script row through the picker, as
    /// the script's table and id, the row's index, the column and the value:
    /// taken by the script window on the frame after. See
    /// [`PickFor::ScriptCell`].
    pub script_pick: Option<(&'static str, u32, usize, &'static str, u32)>,
    /// Whether the headings have been named — see [`Self::name_the_zones`].
    zones_named: bool,
    /// Whether the scripted flags have been acted on — see
    /// `crate::server::quests::on_the_command_line`, which waits on this.
    pub scripted_done: bool,
    seeded: bool,
}

/// What one read of the tables answered.
pub struct TableRead {
    quests: Vec<Known>,
    highest: Option<u32>,
    relations: Vec<Relation>,
    holders: Vec<((Holder, u32), String)>,
}

/// What one batch of name lookups answered.
pub struct NamesRead {
    items: Vec<(u32, Option<ItemName>)>,
    holders: Vec<((Holder, u32), Option<String>)>,
    lists: Vec<((&'static str, u32), Option<Hit>)>,
}

impl Quests {
    /// How many quests there are: the database's and this project's own.
    pub fn count(&self) -> usize {
        self.all.len() + self.created.len()
    }

    /// One of them, by an index over both lists — the database's first.
    pub fn at(&self, index: usize) -> Option<&Known> {
        match self.all.get(index) {
            Some(known) => Some(known),
            None => self.created.get(index - self.all.len()),
        }
    }

    /// One quest by entry, from either list.
    pub fn by_entry(&self, entry: u32) -> Option<&Known> {
        // The database's list is sorted by entry, which is the query's own
        // `ORDER BY`, so this is a binary search rather than a walk of 4,433.
        match self.all.binary_search_by_key(&entry, |known| known.entry) {
            Ok(at) => self.all.get(at),
            Err(_) => self.created.iter().find(|known| known.entry == entry),
        }
    }

    /// One quest as it should be drawn: the database's reading with the
    /// project's edits over it.
    pub fn shown(&self, index: usize, edits: &Edits) -> Option<Known> {
        let base = self.at(index)?;
        Some(base.with_edits(edits).unwrap_or_else(|| base.clone()))
    }

    /// One quest by entry, with the project's edits over it, as
    /// [`Self::shown`] gives it.
    pub fn shown_entry(&self, entry: u32, edits: &Edits) -> Option<Known> {
        let base = self.by_entry(entry)?;
        Some(base.with_edits(edits).unwrap_or_else(|| base.clone()))
    }

    /// The open quest, with this project's edits over it.
    pub fn open_quest(&self, edits: &Edits) -> Option<Known> {
        self.shown_entry(self.open?, edits)
    }

    /// What a quest is called, for a line that has only its entry.
    pub fn title_of(&self, entry: u32, edits: &Edits) -> Option<String> {
        self.shown_entry(entry, edits).map(|known| known.title)
    }

    /// The rows the query matches, as indices, the newest search cached.
    ///
    /// A number matches the quest with that entry; text matches a title, a
    /// heading or a word of the second line. [`Self::of_holder`] narrows the
    /// whole list first.
    pub fn matches(&mut self, edits: &Edits, revision: u64) -> &[usize] {
        let asked = (
            self.query.trim().to_ascii_lowercase(),
            revision,
            self.all.len(),
            self.created.len(),
            self.of_holder,
        );
        if self.built.as_ref() == Some(&asked) {
            return &self.matches;
        }
        let query = asked.0.clone();
        self.built = Some(asked);
        let narrowed: Option<HashSet<u32>> = self.of_holder.map(|(holder, id)| {
            self.relations_of_holder(holder, id, edits)
                .into_iter()
                .map(|(relation, _)| relation.quest)
                .collect()
        });
        let by_entry: Option<u32> = query.parse().ok();
        let mut matches = Vec::new();
        for index in 0..self.count() {
            let Some(known) = self.shown(index, edits) else {
                continue;
            };
            if narrowed
                .as_ref()
                .is_some_and(|set| !set.contains(&known.entry))
            {
                continue;
            }
            let hit = query.is_empty()
                || by_entry == Some(known.entry)
                || known.title.to_ascii_lowercase().contains(&query)
                || known.sub().to_ascii_lowercase().contains(&query);
            if hit {
                matches.push(index);
            }
        }
        self.matches = matches;
        &self.matches
    }

    /// Forget the cached search, for a caller that has changed what is in the
    /// lists rather than what is in the box.
    pub fn forget_matches(&mut self) {
        self.built = None;
    }

    /// Name every quest's heading, once, from the two client tables.
    ///
    /// `zone_of` is the panel's, because the panel is what has the session the
    /// tables are opened through: it answers `None` until both are open, and
    /// this does nothing until it answers.
    pub fn name_the_zones(&mut self, mut zone_of: impl FnMut(i32) -> Option<String>) {
        if self.zones_named || self.all.is_empty() {
            return;
        }
        // Asked once with a value every install has, to find out whether the
        // tables are open yet.
        if zone_of(12).is_none() {
            return;
        }
        let mut memo: HashMap<i32, String> = HashMap::new();
        for known in self.all.iter_mut().chain(self.created.iter_mut()) {
            if known.zone_or_sort == 0 {
                continue;
            }
            let name = memo
                .entry(known.zone_or_sort)
                .or_insert_with(|| zone_of(known.zone_or_sort).unwrap_or_default());
            known.zone.clone_from(name);
        }
        self.zones_named = true;
        self.forget_matches();
    }

    /// The entry a new quest gets: the highest of the reserved base, one
    /// past what the table holds, and one past what this project has claimed.
    pub fn next_entry(&self, edits: &Edits) -> u32 {
        let in_database = self.max_entry.map(|highest| highest + 1).unwrap_or(0);
        let claimed = edits
            .rows()
            .filter(|(table, _, row)| *table == quest::TEMPLATE && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .map(|highest| highest as u32 + 1)
            .unwrap_or(0);
        quest::RESERVED_ENTRY_BASE.max(in_database).max(claimed)
    }

    /// Whether an entry is already a quest, by the entry the project gives
    /// each row and by the one the database has it at — see
    /// `crate::tools::items::Items::entry_taken`, where the reason for both is.
    pub fn entry_taken(&self, entry: u32) -> bool {
        self.all
            .iter()
            .chain(self.created.iter())
            .any(|known| known.entry == entry || known.read_entry == entry)
    }

    /// Move a quest to another entry.
    ///
    /// The project's claim on the row is re-keyed, recording where the database
    /// has it; the project's relation rows for the quest and its other claims
    /// that name it (`PrevQuestId`, `NextQuestId`, `NextQuestInChain`, an
    /// item's `start_quest`) follow. The database's rows follow at the apply —
    /// see `vale_mangos::quest::REFERENCES`, and
    /// `crate::tools::items::Items::rekey`, which is the same operation.
    ///
    /// A quest this project removes cannot be moved: there is no row the move
    /// would be of.
    pub fn rekey(
        &mut self,
        session: &mut EditSession,
        known: &Known,
        to: u32,
        now: f64,
    ) -> Result<(), String> {
        if to == known.entry {
            return Ok(());
        }
        if to == 0 {
            return Err("0 is not a quest entry".to_string());
        }
        if to > quest::MAX_ENTRY {
            return Err(format!(
                "{to} is past {}, which is all `entry` can hold",
                quest::MAX_ENTRY
            ));
        }
        if known.claim == Life::Delete {
            return Err("this project removes that quest; take the removal back first".to_string());
        }
        if self.entry_taken(to) && to != known.read_entry {
            return Err(format!("entry {to} is already a quest"));
        }
        let subject = format!("quest {} entry", known.entry);
        let gesture = crate::session::Gesture {
            label: "Move quest entry",
            subject: &subject,
            now,
        };
        crate::server::follow::rekey(
            session,
            quest::TEMPLATE,
            &known.key(),
            &quest::template_key(to, known.patch),
            Some(&quest::template_key(known.read_entry, known.patch)),
            &quest::REFERENCES,
            gesture,
        )?;
        self.open = Some(to);
        self.row = None;
        self.created_for = None;
        self.forget_matches();
        Ok(())
    }

    /// Make a new quest, and open it. Every column is written at once, at
    /// the server's own patch — see `vale_mangos::quest::new_quest`.
    pub fn create(&mut self, session: &mut EditSession, title: &str, patch: u32, now: f64) -> u32 {
        let entry = self.next_entry(&session.server_edits);
        let key = quest::template_key(entry, patch);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in quest::new_quest(title) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        let subject = format!("quest {entry}");
        session.set_server_row(
            quest::TEMPLATE,
            &key,
            Some(&row),
            Some(crate::session::Gesture {
                label: "New quest",
                subject: &subject,
                now,
            }),
        );
        self.open = Some(entry);
        self.row = None;
        self.forget_matches();
        entry
    }

    /// A copy of the open quest under a new entry, with every column as it
    /// is drawn. `None` until the whole row has been read.
    ///
    /// The relations are not copied, because who gives the copy is a separate
    /// decision. Copying them would make the original's giver offer a second
    /// quest as a side effect of the copy button.
    pub fn duplicate(&mut self, session: &mut EditSession, patch: u32, now: f64) -> Option<u32> {
        let from = self
            .row
            .as_ref()
            .filter(|held| Some(held.entry) == self.open)?;
        let source = quest::template_key(from.entry, from.patch);
        let entry = self.next_entry(&session.server_edits);
        let key = quest::template_key(entry, patch);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for column in quest::TEMPLATE_COLUMNS
            .iter()
            .filter(|column| column.editable())
        {
            let value = session
                .server_edits
                .get(quest::TEMPLATE, &source, column.name)
                .map(str::to_string)
                .or_else(|| column.literal(&from.row))
                // A text column the database holds as NULL is copied as empty:
                // a creation names every column, and `NULL` is not a value a
                // form can show.
                .unwrap_or_else(|| match column.kind.is_text() {
                    true => vale_mangos::sql::text(""),
                    false => "0".to_string(),
                });
            row.columns.insert(column.name.to_string(), value);
        }
        let titled = row
            .columns
            .get("Title")
            .map(|literal| crate::ui::rowform::unquote(literal))
            .unwrap_or_default();
        row.columns.insert(
            "Title".to_string(),
            vale_mangos::sql::text(&format!("{titled} (copy)")),
        );
        let subject = format!("quest {entry}");
        session.set_server_row(
            quest::TEMPLATE,
            &key,
            Some(&row),
            Some(crate::session::Gesture {
                label: "Copy quest",
                subject: &subject,
                now,
            }),
        );
        self.open = Some(entry);
        self.row = None;
        self.forget_matches();
        Some(entry)
    }

    /// Mark a quest for removal, or give up one this project created.
    ///
    /// A quest in the database becomes a [`Life::Delete`] row, which stays in
    /// the list struck through until it is applied. One this project created is
    /// in no database, so its claim is taken back and it leaves the list — and
    /// the relations this project added to it go with it, because a relation to
    /// a quest that does not exist is a row the server drops at load.
    pub fn remove(&mut self, session: &mut EditSession, known: &Known, now: f64) {
        let subject = format!("quest {} remove", known.entry);
        let gesture = crate::session::Gesture {
            label: "Remove quest",
            subject: &subject,
            now,
        };
        match known.claim {
            Life::Insert => {
                let added: Vec<(String, Key)> = session
                    .server_edits
                    .rows()
                    .filter(|(table, key, _)| {
                        quest::is_relation(table)
                            && key
                                .0
                                .get(1)
                                .and_then(|(_, quest)| quest.parse::<u32>().ok())
                                == Some(known.entry)
                    })
                    .map(|(table, key, _)| (table.to_string(), key.clone()))
                    .collect();
                for (table, key) in added {
                    session.set_server_row(&table, &key, None, Some(gesture));
                }
                session.set_server_row(quest::TEMPLATE, &known.key(), None, Some(gesture));
                if self.open == Some(known.entry) {
                    self.open = None;
                    self.row = None;
                }
            }
            _ => {
                let row = RowEdit {
                    life: Life::Delete,
                    ..RowEdit::default()
                };
                session.set_server_row(quest::TEMPLATE, &known.key(), Some(&row), Some(gesture));
            }
        }
        self.forget_matches();
    }

    /// Keep a quest that was marked for removal: the claim is taken back,
    /// and with it any column edit made before the mark, which a `Delete` row
    /// does not carry.
    pub fn keep(&mut self, session: &mut EditSession, known: &Known, now: f64) {
        let subject = format!("quest {} keep", known.entry);
        session.set_server_row(
            quest::TEMPLATE,
            &known.key(),
            None,
            Some(crate::session::Gesture {
                label: "Keep quest",
                subject: &subject,
                now,
            }),
        );
        self.forget_matches();
    }

    /// Every relation of one quest, the database's with the project's over
    /// them, each with what the project says is to become of it:
    /// [`Life::Update`] for a row left as it is, [`Life::Insert`] for one this
    /// project adds and [`Life::Delete`] for one it removes.
    pub fn relations_of_quest(&self, entry: u32, edits: &Edits) -> Vec<(Relation, Life)> {
        self.relations_where(edits, |relation| relation.quest == entry)
    }

    /// Every relation of one creature or game object, on the same terms as
    /// [`Self::relations_of_quest`].
    pub fn relations_of_holder(
        &self,
        holder: Holder,
        id: u32,
        edits: &Edits,
    ) -> Vec<(Relation, Life)> {
        self.relations_where(edits, |relation| {
            relation.holder == holder && relation.id == id
        })
    }

    fn relations_where(
        &self,
        edits: &Edits,
        wanted: impl Fn(&Relation) -> bool,
    ) -> Vec<(Relation, Life)> {
        let mut out: Vec<(Relation, Life)> = self
            .relations
            .iter()
            .filter(|relation| wanted(relation))
            .map(|relation| (*relation, edits.life(relation.table(), &relation.key())))
            .collect();
        for (table, key, row) in edits.rows() {
            if row.life != Life::Insert {
                continue;
            }
            let Some((holder, role)) = relation_of_table(table) else {
                continue;
            };
            let part = |at: usize| {
                key.0
                    .get(at)
                    .and_then(|(_, value)| value.parse::<u32>().ok())
            };
            let (Some(id), Some(quest)) = (part(0), part(1)) else {
                continue;
            };
            let relation = Relation {
                holder,
                role,
                id,
                quest,
            };
            if wanted(&relation) && !out.iter().any(|(had, _)| *had == relation) {
                out.push((relation, Life::Insert));
            }
        }
        out.sort_by_key(|(relation, _)| {
            (relation.role == Role::Takes, relation.quest, relation.id)
        });
        out
    }

    /// Add a relation. One that is in the database and marked for removal
    /// is kept instead; one that is already there is left alone.
    pub fn relate(&mut self, session: &mut EditSession, relation: Relation, now: f64) {
        let (table, key) = (relation.table(), relation.key());
        let subject = format!("{table} {}", key.text());
        let gesture = crate::session::Gesture {
            label: "Add quest relation",
            subject: &subject,
            now,
        };
        let in_database = self.relations.contains(&relation);
        match (in_database, session.server_edits.life(table, &key)) {
            (true, Life::Delete) => session.set_server_row(table, &key, None, Some(gesture)),
            (true, _) => {}
            (false, _) => {
                let mut row = RowEdit {
                    life: Life::Insert,
                    ..RowEdit::default()
                };
                for change in quest::new_relation() {
                    row.columns.insert(change.column.to_string(), change.value);
                }
                session.set_server_row(table, &key, Some(&row), Some(gesture));
            }
        }
        self.forget_matches();
    }

    /// Remove a relation: a `Delete` row for one in the database, and the
    /// claim taken back for one this project added.
    pub fn unrelate(&mut self, session: &mut EditSession, relation: Relation, now: f64) {
        let (table, key) = (relation.table(), relation.key());
        let subject = format!("{table} {}", key.text());
        let gesture = crate::session::Gesture {
            label: "Remove quest relation",
            subject: &subject,
            now,
        };
        match self.relations.contains(&relation) {
            true => {
                let row = RowEdit {
                    life: Life::Delete,
                    ..RowEdit::default()
                };
                session.set_server_row(table, &key, Some(&row), Some(gesture));
            }
            false => session.set_server_row(table, &key, None, Some(gesture)),
        }
        self.forget_matches();
    }

    /// What an item is called, its quality and its display id, or `None`
    /// while it is being fetched and for an entry neither the project nor the
    /// database holds; see [`Self::item_known`] for which.
    ///
    /// The project's own claim on the item's template row comes first, column
    /// by column, over what the database answered: an item this project
    /// creates has its name before it is applied, and one it renames shows the
    /// new name. The database's answer is kept only until the database is next
    /// written; see `crate::server::fresh`.
    pub fn item(&mut self, entry: u32, edits: &Edits) -> Option<ItemName> {
        let name = claimed(edits, vale_mangos::item::TEMPLATE, entry, "name");
        let quality = claimed(edits, vale_mangos::item::TEMPLATE, entry, "quality").and_then(|v| v.parse().ok());
        let display = claimed(edits, vale_mangos::item::TEMPLATE, entry, "display_id").and_then(|v| v.parse().ok());
        let read = match self.items.get(&entry) {
            Some(found) => found.clone(),
            None => {
                self.wanted_items.insert(entry);
                None
            }
        };
        match (read, name) {
            (Some(read), name) => Some(ItemName {
                name: name.unwrap_or(read.name),
                quality: quality.unwrap_or(read.quality),
                display_id: display.unwrap_or(read.display_id),
            }),
            (None, Some(name)) => Some(ItemName {
                name,
                quality: quality.unwrap_or(0),
                display_id: display.unwrap_or(0),
            }),
            (None, None) => None,
        }
    }

    /// Whether an item's name is settled: the project names it, or the
    /// database has answered, with a row or without one.
    pub fn item_known(&self, entry: u32, edits: &Edits) -> bool {
        self.items.contains_key(&entry) || claimed(edits, vale_mangos::item::TEMPLATE, entry, "name").is_some()
    }

    /// What a creature or game object is called, on [`Self::item`]'s terms:
    /// the project's claim on its template's `name` first, the database's
    /// answer after.
    pub fn holder(&mut self, holder: Holder, id: u32, edits: &Edits) -> Option<String> {
        if let Some(name) = claimed(edits, holder.template(), id, "name") {
            return Some(name);
        }
        match self.holders.get(&(holder, id)) {
            Some(found) => found.clone(),
            None => {
                self.wanted_holders.insert((holder, id));
                None
            }
        }
    }

    /// Whether a creature's or game object's name is settled, on
    /// [`Self::item_known`]'s terms.
    pub fn holder_known(&self, holder: Holder, id: u32, edits: &Edits) -> bool {
        self.holders.contains_key(&(holder, id)) || claimed(edits, holder.template(), id, "name").is_some()
    }

    /// What one id of a [`Target::List`] table holds, as the picker lists it:
    /// the database's answer, or a line saying the project creates rows under
    /// it when the database has none. Asked for when it is not known yet.
    pub fn listed(&mut self, table: &'static str, id: u32, edits: &Edits) -> Option<Hit> {
        match self.lists.get(&(table, id)) {
            Some(Some(found)) => Some(found.clone()),
            Some(None) => created_list(table, id, edits),
            None => {
                self.wanted_lists.insert((table, id));
                None
            }
        }
    }

    /// Whether an id of a [`Target::List`] table has been answered, with rows or
    /// without.
    pub fn listed_known(&self, table: &'static str, id: u32) -> bool {
        self.lists.contains_key(&(table, id))
    }

    /// Remember a name a picker's own search answered, so the row that was just
    /// chosen is drawn with its name on the frame it is written.
    pub fn learn(&mut self, target: Target, hit: &Hit) {
        match target {
            Target::List(table) => {
                self.lists.insert((table, hit.id), Some(hit.clone()));
            }
            Target::Creature => {
                self.holders
                    .insert((Holder::Creature, hit.id), Some(hit.title.clone()));
            }
            Target::Object => {
                self.holders
                    .insert((Holder::Object, hit.id), Some(hit.title.clone()));
            }
            // An item's quality and display id are not in a hit, so it is
            // fetched like any other.
            _ => {}
        }
    }

    /// Open the workspace on one creature's or game object's quests.
    pub fn show_quests_of(&mut self, holder: Holder, id: u32) {
        self.of_holder = Some((holder, id));
        self.forget_matches();
    }
}

/// Read the templates and the relations, on a task, when an apply has moved
/// what is in them.
fn read_the_tables(
    mut quests: ResMut<Quests>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
) {
    if let Some(task) = quests.task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            quests.task = None;
            match done {
                Ok(read) => {
                    info!(
                        "quests: {} quest(s) and {} relation(s) read",
                        read.quests.len(),
                        read.relations.len()
                    );
                    quests.all = read.quests;
                    quests.max_entry = read.highest;
                    quests.relations = read.relations.clone();
                    quests.relations_read = read.relations;
                    // The open row was read out of the table this replaces.
                    quests.row = None;
                    for (key, name) in read.holders {
                        quests.holders.insert(key, Some(name));
                    }
                    quests.trouble = None;
                    quests.created_for = None;
                    quests.zones_named = false;
                    quests.forget_matches();
                }
                Err(e) => {
                    warn!("quests: {e}");
                    quests.all.clear();
                    quests.relations.clear();
                    quests.trouble = Some(e);
                }
            }
        }
        return;
    }
    // Read for every tool that shows quests: the quest workspace, and the
    // creature and game object tools while their Quests window is open.
    // Without this read, that window would open on an empty list when the
    // Quests row had never been pressed, and show the creature as giving no
    // quests.
    //
    // The item workspace reads it too: it shows `start_quest` as the quest's
    // title rather than its number, and its picker searches this list.
    let wanted = matches!(*tool, Tool::Quests | Tool::Items)
        || (matches!(*tool, Tool::Creatures | Tool::GameObjects) && quests.window_for.is_some());
    if !wanted {
        return;
    }
    let Some(session) = session else { return };
    let key = session.database_writes;
    if quests.loaded == Some(key) {
        return;
    }
    // Set before the read, so a failed read does not ask again every frame.
    quests.loaded = Some(key);
    let Some((at, _source)) = settings.resolve() else {
        quests.all.clear();
        quests.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = super::creatures::server_patch(&settings);
    quests.trouble = None;
    quests.task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let rows = db.rows(&quest::all_quests_query(patch))?;
        let highest = db
            .row(quest::MAX_ENTRY_QUERY)?
            .and_then(|row| row.integer("entry"))
            .map(|entry| entry as u32);
        let mut relations = Vec::new();
        for table in quest::RELATIONS {
            let Some((holder, role)) = relation_of_table(table) else {
                continue;
            };
            for row in db.rows(&quest::relations_query(table, patch))? {
                let (Some(id), Some(quest)) = (row.integer("id"), row.integer("quest")) else {
                    continue;
                };
                relations.push(Relation {
                    holder,
                    role,
                    id: id as u32,
                    quest: quest as u32,
                });
            }
        }
        let mut holders = Vec::new();
        for (creatures, holder) in [(true, Holder::Creature), (false, Holder::Object)] {
            for row in db.rows(&quest::relation_names_query(creatures, patch))? {
                if let (Some(entry), Some(name)) = (row.integer("entry"), row.text("name")) {
                    holders.push(((holder, entry as u32), name.to_string()));
                }
            }
        }
        Ok(TableRead {
            quests: rows.iter().filter_map(read_known).collect(),
            highest,
            relations,
            holders,
        })
    }));
}

/// Read the open quest's whole row, on a task, when the open quest changes.
fn read_the_row(
    mut quests: ResMut<Quests>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
) {
    if let Some(task) = quests.row_task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            quests.row_task = None;
            match done {
                Ok(row) => quests.row = row,
                Err(e) => warn!("quest row: {e}"),
            }
        }
        return;
    }
    if *tool != Tool::Quests {
        return;
    }
    let Some(entry) = quests.open else { return };
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    let created_now = quests
        .by_entry(entry)
        .is_some_and(|known| known.claim == Life::Insert);
    if quests.row.as_ref().is_some_and(|held| held.entry == entry) && quests.row_at == Some((writes, created_now)) {
        return;
    }
    quests.row_at = Some((writes, created_now));
    // A row this project created has no row in the database: its columns are
    // the store's own and the form reads them from there.
    let known = quests.by_entry(entry).cloned();
    if let Some(known) = known.filter(|known| known.claim == Life::Insert) {
        quests.row = Some(QuestRow {
            entry,
            patch: known.patch,
            row: quest::Row::new(),
        });
        return;
    }
    let Some((at, _)) = settings.resolve() else {
        return;
    };
    let patch = super::creatures::server_patch(&settings);
    // Read from where the database has it — see [`Known::read_entry`].
    let read_at = quests
        .by_entry(entry)
        .map(|known| known.read_entry)
        .unwrap_or(entry);
    quests.row_task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let Some(row) = db.row(&quest::winning_template_query(read_at, patch))? else {
            return Ok(None);
        };
        let at_patch = row.integer("patch").unwrap_or(0) as u32;
        Ok(Some(QuestRow {
            entry,
            patch: at_patch,
            row,
        }))
    }));
}

/// Give every quest that was read, and every relation of it, the entry the
/// project says the quest has. This is `crate::tools::items::fold_the_store_in`
/// for quests, for the same reason: a quest whose entry the project changes is
/// read at its database entry until the move is applied, while its claim is
/// keyed under the new entry.
///
/// The list is sorted again afterwards, because [`Quests::by_entry`] is a
/// binary search over it.
pub fn fold_the_store_in(
    all: &mut Vec<Known>,
    relations: &mut Vec<Relation>,
    relations_read: &[Relation],
    edits: &Edits,
) {
    let moves: Vec<(u32, u32)> = edits
        .moves(quest::TEMPLATE)
        .filter_map(|(from, to)| Some((from.first()? as u32, to.first()? as u32)))
        .collect();
    let to = |read: u32| {
        moves
            .iter()
            .find(|(from, _)| *from == read)
            .map(|(_, to)| *to)
            .unwrap_or(read)
    };
    for known in all.iter_mut() {
        known.entry = to(known.read_entry);
    }
    all.sort_by_key(|known| known.entry);
    relations.clear();
    relations.extend(relations_read.iter().map(|relation| Relation {
        quest: to(relation.quest),
        ..*relation
    }));
}

/// Rebuild the quests this project creates, from the store, when it moves.
fn rebuild_created(mut quests: ResMut<Quests>, session: Option<Res<EditSession>>) {
    let Some(session) = session else { return };
    if quests.created_for == Some(session.server_edit_revision) {
        return;
    }
    quests.created_for = Some(session.server_edit_revision);
    {
        let quests = &mut *quests;
        fold_the_store_in(
            &mut quests.all,
            &mut quests.relations,
            &quests.relations_read,
            &session.server_edits,
        );
    }
    quests.created.clear();
    for (table, key, row) in session.server_edits.rows() {
        if table != quest::TEMPLATE || row.life != Life::Insert {
            continue;
        }
        let number = |column: &str| {
            row.columns
                .get(column)
                .and_then(|value| value.trim().parse::<i64>().ok())
                .unwrap_or(0)
        };
        let Some(entry) = key.first().map(|entry| entry as u32) else {
            continue;
        };
        // Once it has been applied it is in the table that was read, and
        // listing it from both would draw it twice and count it twice. The
        // project still claims it as a creation, which `Known::with_edits`
        // reads off the store for the row that is in [`Quests::all`].
        if quests
            .all
            .binary_search_by_key(&entry, |known| known.entry)
            .is_ok()
        {
            continue;
        }
        let patch = key
            .0
            .iter()
            .find(|(column, _)| column == "patch")
            .and_then(|(_, value)| value.parse::<u32>().ok())
            .unwrap_or(0);
        quests.created.push(Known {
            entry,
            read_entry: entry,
            patch,
            title: row
                .columns
                .get("Title")
                .map(|literal| crate::ui::rowform::unquote(literal))
                .unwrap_or_default(),
            method: number("Method") as u32,
            zone_or_sort: number("ZoneOrSort") as i32,
            min_level: number("MinLevel") as u32,
            level: number("QuestLevel") as u32,
            kind: number("Type") as u32,
            special_flags: number("SpecialFlags") as u32,
            prev: number("PrevQuestId") as i32,
            next: number("NextQuestId") as i32,
            next_in_chain: number("NextQuestInChain") as u32,
            zone: String::new(),
            claim: Life::Insert,
        });
    }
    quests.created.sort_by_key(|known| known.entry);
    // The headings of the created rows are named again with the rest.
    quests.zones_named = false;
    quests.forget_matches();
}

/// Turn the names that were asked for and not known into one query a kind.
///
/// Both name caches are emptied when `EditSession::database_writes` moves, so
/// a name is never drawn from a row an apply or a put back has changed; see
/// `crate::server::fresh`.
fn fetch_the_names(
    mut quests: ResMut<Quests>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    let quests = &mut *quests;
    let emptied =
        quests.holders.renew(writes) | quests.items.renew(writes) | quests.lists.renew(writes);
    if emptied {
        // What was asked for before is asked for again by whoever draws it.
        quests.wanted_items.clear();
        quests.wanted_holders.clear();
        quests.wanted_lists.clear();
    }
    if let Some((started, task)) = quests.names_task.as_mut() {
        let started = *started;
        if let Some(done) = block_on(future::poll_once(task)) {
            quests.names_task = None;
            match done {
                Ok(_) if started != writes => {}
                Ok(read) => {
                    for (entry, found) in read.items {
                        quests.items.insert(entry, found);
                    }
                    for (key, found) in read.holders {
                        quests.holders.insert(key, found);
                    }
                    for (key, found) in read.lists {
                        quests.lists.insert(key, found);
                    }
                }
                Err(e) => warn!("quest names: {e}"),
            }
        }
        return;
    }
    if quests.wanted_items.is_empty()
        && quests.wanted_holders.is_empty()
        && quests.wanted_lists.is_empty()
    {
        return;
    }
    let Some((at, _)) = settings.resolve() else {
        // No database: everything asked for is absent, so it is not asked again.
        let items: Vec<u32> = quests.wanted_items.drain().collect();
        for entry in items {
            quests.items.insert(entry, None);
        }
        let holders: Vec<(Holder, u32)> = quests.wanted_holders.drain().collect();
        for key in holders {
            quests.holders.insert(key, None);
        }
        let lists: Vec<(&'static str, u32)> = quests.wanted_lists.drain().collect();
        for key in lists {
            quests.lists.insert(key, None);
        }
        return;
    };
    let patch = super::creatures::server_patch(&settings);
    let items: Vec<u32> = quests.wanted_items.drain().collect();
    let holders: Vec<(Holder, u32)> = quests.wanted_holders.drain().collect();
    let lists: Vec<(&'static str, u32)> = quests.wanted_lists.drain().collect();
    quests.names_task = Some((writes, crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let mut out = NamesRead {
            // Everything asked for starts absent, and what the query answers
            // replaces it — so an id the database does not hold is cached too.
            items: items.iter().map(|entry| (*entry, None)).collect(),
            holders: holders.iter().map(|key| (*key, None)).collect(),
            lists: lists.iter().map(|key| (*key, None)).collect(),
        };
        // One query a table, for the ids of it that were asked for.
        let mut tables: Vec<&'static str> = lists.iter().map(|(table, _)| *table).collect();
        tables.sort_unstable();
        tables.dedup();
        for table in tables {
            let Some(list) = vale_mangos::lists::list(table) else {
                continue;
            };
            let ids: Vec<u32> = lists
                .iter()
                .filter(|(had, _)| *had == table)
                .map(|(_, id)| *id)
                .collect();
            let filter = vale_mangos::lists::Filter::Ids(&ids);
            for row in db.rows(&vale_mangos::lists::query(list, filter, patch, ids.len()))? {
                let Some(found) = vale_mangos::lists::Listed::read(list, &row) else {
                    continue;
                };
                if let Some(slot) = out.lists.iter_mut().find(|(key, _)| *key == (table, found.id)) {
                    slot.1 = Some(Hit {
                        id: found.id,
                        title: found.title,
                        sub: found.sub,
                    });
                }
            }
        }
        if let Some(sql) = quest::item_names_query(&items, patch) {
            for row in db.rows(&sql)? {
                let Some(entry) = row.integer("entry") else {
                    continue;
                };
                let found = ItemName {
                    name: row.text("name").unwrap_or_default().to_string(),
                    quality: row.integer("quality").unwrap_or(0) as u32,
                    display_id: row.integer("display_id").unwrap_or(0) as u32,
                };
                if let Some(slot) = out.items.iter_mut().find(|(had, _)| *had == entry as u32) {
                    slot.1 = Some(found);
                }
            }
        }
        for (creatures, which) in [(true, Holder::Creature), (false, Holder::Object)] {
            let ids: Vec<u32> = holders
                .iter()
                .filter(|(holder, _)| *holder == which)
                .map(|(_, id)| *id)
                .collect();
            let Some(sql) = quest::holder_names_query(creatures, &ids, patch) else {
                continue;
            };
            for row in db.rows(&sql)? {
                let (Some(entry), Some(name)) = (row.integer("entry"), row.text("name")) else {
                    continue;
                };
                if let Some(slot) = out
                    .holders
                    .iter_mut()
                    .find(|(key, _)| *key == (which, entry as u32))
                {
                    slot.1 = Some(name.to_string());
                }
            }
        }
        Ok(out)
    })));
}

/// …and the same for a [`Target::List`] table: an id under which the project
/// creates rows and the database holds none is listed first, when the box is
/// empty or names it.
fn fold_the_projects_lists(hits: &mut Vec<Hit>, table: &'static str, query: &str, edits: &Edits) {
    let by_id: Option<u32> = query.parse().ok();
    let mut created: Vec<Hit> = Vec::new();
    for (claimed_table, key, row) in edits.rows() {
        if claimed_table != table || row.life != Life::Insert {
            continue;
        }
        let Some(id) = key.first().map(|id| id as u32) else {
            continue;
        };
        if hits.iter().chain(created.iter()).any(|hit| hit.id == id) {
            continue;
        }
        if query.is_empty() || by_id == Some(id) {
            if let Some(hit) = created_list(table, id, edits) {
                created.push(hit);
            }
        }
    }
    created.sort_by_key(|hit| hit.id);
    hits.splice(0..0, created);
    hits.truncate(PICK_LIMIT);
}

/// An id of a [`Target::List`] table the project creates rows under, as a
/// picker row, or `None` when it creates none.
fn created_list(table: &'static str, id: u32, edits: &Edits) -> Option<Hit> {
    let rows = edits
        .rows()
        .filter(|(had, key, row)| {
            *had == table && key.first() == Some(u64::from(id)) && row.life == Life::Insert
        })
        .count();
    (rows > 0).then(|| Hit {
        id,
        title: "created by this project".to_string(),
        sub: format!("{rows} row(s) not yet applied"),
    })
}

/// Put the project's own rows into a search the database answered: a
/// creature, game object or item the project names is listed under that name,
/// and one it creates that matches the search is listed first. The database
/// has never held a created row, so without this it could not be chosen.
fn fold_the_projects_rows(hits: &mut Vec<Hit>, target: Target, query: &str, edits: &Edits) {
    let table = match target {
        Target::Item => vale_mangos::item::TEMPLATE,
        Target::Creature => vale_mangos::creature::TEMPLATE,
        Target::Object => vale_mangos::gameobject::TEMPLATE,
        Target::List(table) => return fold_the_projects_lists(hits, table, query, edits),
        _ => return,
    };
    for hit in hits.iter_mut() {
        if let Some(name) = claimed(edits, table, hit.id, "name") {
            hit.title = name;
        }
    }
    let by_entry: Option<u32> = query.parse().ok();
    let mut created: Vec<Hit> = Vec::new();
    for (claimed_table, key, row) in edits.rows() {
        if claimed_table != table || row.life != Life::Insert {
            continue;
        }
        let Some(entry) = key.first().map(|entry| entry as u32) else {
            continue;
        };
        if hits.iter().chain(created.iter()).any(|hit| hit.id == entry) {
            continue;
        }
        let name = claimed(edits, table, entry, "name").unwrap_or_default();
        if by_entry == Some(entry) || name.to_ascii_lowercase().contains(query) {
            created.push(Hit {
                id: entry,
                title: name,
                sub: "created by this project".to_string(),
            });
        }
    }
    created.sort_by_key(|hit| hit.id);
    hits.splice(0..0, created);
    hits.truncate(PICK_LIMIT);
}

/// Run the picker's search, for the targets whose rows are in the database:
/// one query per change to the box, on a task. A quest is searched in memory
/// and a client table by the panel.
fn search_the_picker(
    mut quests: ResMut<Quests>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    let edits = session.as_ref().map(|session| &session.server_edits);
    let quests = &mut *quests;
    let Some(picker) = quests.picker.as_mut() else {
        return;
    };
    if let Some(task) = picker.task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            picker.task = None;
            match done {
                Ok(mut hits) => {
                    if let (Some(edits), Some((target, query))) = (edits, picker.built.as_ref()) {
                        fold_the_projects_rows(&mut hits, *target, query, edits);
                    }
                    picker.hits = hits;
                }
                Err(e) => warn!("quest picker: {e}"),
            }
        }
        return;
    }
    let asked = (picker.target, picker.query.trim().to_ascii_lowercase());
    // A search answered before the database was written is sent again, and
    // the project's own rows are folded over what the database answers; see
    // `fold_the_projects_rows`.
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    if picker.built.as_ref() == Some(&asked) && picker.built_at == Some(writes) {
        return;
    }
    picker.built_at = Some(writes);
    picker.task = None;
    let query = asked.1.clone();
    match picker.target {
        // The panel's — see `crate::ui::quests::search_a_table`.
        Target::Dbc(_) => {}
        // Every id when the box is empty, unlike the templates: these tables
        // are hundreds or a few thousand ids, listed in order, and the one
        // wanted is often the creature's own entry or near it.
        Target::List(table) => {
            picker.built = Some(asked);
            let Some(list) = vale_mangos::lists::list(table) else {
                picker.hits.clear();
                return;
            };
            let Some((at, _)) = settings.resolve() else {
                return;
            };
            let patch = super::creatures::server_patch(&settings);
            picker.task = Some(crate::server::queue::read(async move {
                let mut db = vale_mangos::conn::Db::open(&at)?;
                let filter = vale_mangos::lists::Filter::Search(&query);
                let sql = vale_mangos::lists::query(list, filter, patch, PICK_LIMIT);
                Ok(db
                    .rows(&sql)?
                    .iter()
                    .filter_map(|row| vale_mangos::lists::Listed::read(list, row))
                    .map(|found| Hit {
                        id: found.id,
                        title: found.title,
                        sub: found.sub,
                    })
                    .collect())
            }));
        }
        Target::Quest => {
            picker.built = Some(asked);
            let by_entry: Option<u32> = query.parse().ok();
            let default = Edits::default();
            let edits = edits.unwrap_or(&default);
            picker.hits = quests
                .all
                .iter()
                .chain(quests.created.iter())
                .map(|known| known.with_edits(edits).unwrap_or_else(|| known.clone()))
                .filter(|known| {
                    query.is_empty()
                        || by_entry == Some(known.entry)
                        || known.title.to_ascii_lowercase().contains(&query)
                })
                .take(PICK_LIMIT)
                .map(|known| Hit {
                    id: known.entry,
                    sub: known.sub(),
                    title: known.title,
                })
                .collect();
        }
        Target::Item | Target::Creature | Target::Object => {
            picker.built = Some(asked);
            // An empty box lists nothing. The first two hundred rows of a table
            // of twenty thousand are an arbitrary selection.
            if query.is_empty() {
                picker.hits.clear();
                return;
            }
            let Some((at, _)) = settings.resolve() else {
                return;
            };
            let patch = super::creatures::server_patch(&settings);
            let target = picker.target;
            picker.task = Some(crate::server::queue::read(async move {
                let mut db = vale_mangos::conn::Db::open(&at)?;
                let sql = match target {
                    Target::Item => quest::item_search_query(&query, patch, PICK_LIMIT),
                    Target::Creature => quest::holder_search_query(true, &query, patch, PICK_LIMIT),
                    _ => quest::holder_search_query(false, &query, patch, PICK_LIMIT),
                };
                Ok(db
                    .rows(&sql)?
                    .iter()
                    .filter_map(|row| {
                        Some(Hit {
                            id: row.integer("entry")? as u32,
                            title: row.text("name").unwrap_or_default().to_string(),
                            sub: match target {
                                Target::Item => vale_mangos::item::value_word(
                                    &vale_mangos::item::QUALITIES,
                                    row.integer("quality").unwrap_or(0) as u32,
                                ),
                                Target::Creature => "creature".to_string(),
                                _ => "game object".to_string(),
                            },
                        })
                    })
                    .collect())
            }));
        }
    }
}

/// The command line's quest flags, acted on once the table has come back —
/// see [`crate::Args`].
fn on_the_command_line(
    args: Res<crate::Args>,
    mut quests: ResMut<Quests>,
    mut session: Option<ResMut<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    time: Res<Time>,
) {
    if quests.seeded {
        return;
    }
    let wanted = args.quest.is_some()
        || args.quest_new
        || args.quests_of.is_some()
        || args.quest_entry.is_some();
    if !wanted {
        quests.seeded = true;
        quests.scripted_done = true;
        return;
    }
    // Wait for the table, so `--quest <title>` can be a title and `--quest-new`
    // can number itself above what the table holds.
    if quests.task.is_some() || (quests.all.is_empty() && quests.trouble.is_none()) {
        return;
    }
    quests.seeded = true;
    let Some(session) = session.as_mut() else {
        return;
    };
    if let Some(entry) = args.quests_of {
        quests.show_quests_of(Holder::Creature, entry);
        let edits = &session.server_edits;
        let first = quests
            .relations_of_holder(Holder::Creature, entry, edits)
            .first()
            .map(|(relation, _)| relation.quest);
        info!(
            "--quests-of {entry}: {} relation(s)",
            quests
                .relations_of_holder(Holder::Creature, entry, edits)
                .len()
        );
        quests.open = first;
    }
    if args.quest_new {
        let patch = super::creatures::server_patch(&settings);
        let entry = quests.create(session, "New Quest", patch, time.elapsed_secs_f64());
        info!("--quest-new: quest {entry} created at patch {patch}");
    }
    if let Some(which) = args.quest.clone() {
        match which.parse::<u32>() {
            Ok(entry) => quests.open = Some(entry),
            Err(_) => {
                let wanted = which.to_ascii_lowercase();
                let found = quests
                    .all
                    .iter()
                    .find(|known| known.title.to_ascii_lowercase() == wanted)
                    .or_else(|| {
                        quests
                            .all
                            .iter()
                            .find(|known| known.title.to_ascii_lowercase().contains(&wanted))
                    })
                    .map(|known| known.entry);
                match found {
                    Some(entry) => quests.open = Some(entry),
                    None => warn!("--quest {which}: no quest of that title"),
                }
            }
        }
    }
    // Not finished while a move is still to make. [`scripted_entry`] makes it,
    // in the same way as `crate::tools::items::scripted_entry`.
    quests.scripted_done = args.quest_entry.is_none();
}

/// `--quest-entry <n>`: move the open quest to entry `n` without user input.
/// It retries on later frames, because a quest `--quest-new` has just created
/// is not in [`Quests::created`] until [`rebuild_created`] has run.
fn scripted_entry(
    args: Res<crate::Args>,
    mut quests: ResMut<Quests>,
    mut session: Option<ResMut<EditSession>>,
    time: Res<Time>,
    mut done: Local<bool>,
) {
    let Some(entry) = args.quest_entry else {
        return;
    };
    if *done || !quests.seeded {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    let Some(known) = quests.open.and_then(|open| quests.by_entry(open)).cloned() else {
        return;
    };
    *done = true;
    match quests.rekey(session, &known, entry, time.elapsed_secs_f64()) {
        Ok(()) => info!("--quest-entry {entry}: quest {} moved", known.entry),
        Err(why) => warn!("--quest-entry {entry}: {why}"),
    }
    quests.scripted_done = true;
}

pub struct QuestToolPlugin;

impl Plugin for QuestToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Quests>().add_systems(
            Update,
            (
                read_the_tables,
                rebuild_created,
                read_the_row,
                fetch_the_names,
                search_the_picker,
                on_the_command_line,
                scripted_entry,
            )
                .chain(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_quest(entry: u32) -> Known {
        Known {
            entry,
            read_entry: entry,
            patch: 0,
            title: "A Threat Within".to_string(),
            method: 2,
            zone_or_sort: 9,
            min_level: 1,
            level: 1,
            kind: 0,
            special_flags: 0,
            prev: 0,
            next: 0,
            next_in_chain: 7,
            zone: "Northshire Valley".to_string(),
            claim: Life::Update,
        }
    }

    fn held(relations: &[Relation]) -> Quests {
        Quests {
            all: vec![a_quest(7), a_quest(783)],
            relations: relations.to_vec(),
            ..Quests::default()
        }
    }

    const WILLEM_GIVES: Relation = Relation {
        holder: Holder::Creature,
        role: Role::Gives,
        id: 823,
        quest: 783,
    };
    const MCBRIDE_TAKES: Relation = Relation {
        holder: Holder::Creature,
        role: Role::Takes,
        id: 197,
        quest: 783,
    };

    /// The list shows what the project says: a title typed into the form is the
    /// title in the list, and an untouched row is not rebuilt.
    #[test]
    fn a_row_is_drawn_with_the_projects_edits_over_it() {
        let base = a_quest(783);
        let mut edits = Edits::default();
        assert!(base.with_edits(&edits).is_none());
        edits.set(
            quest::TEMPLATE,
            &base.key(),
            "Title",
            Some(vale_mangos::sql::text("A Threat Without")),
        );
        edits.set(
            quest::TEMPLATE,
            &base.key(),
            "QuestLevel",
            Some("12".to_string()),
        );
        let shown = base.with_edits(&edits).expect("edited");
        assert_eq!(shown.title, "A Threat Without");
        assert_eq!(shown.level, 12);
        assert_eq!(
            shown.zone, "Northshire Valley",
            "an unchanged heading keeps its name"
        );
    }

    /// A heading that was changed loses its name until the panel names it
    /// again, rather than showing the old zone beside the new number.
    #[test]
    fn a_changed_heading_is_named_again() {
        let base = a_quest(783);
        let mut edits = Edits::default();
        edits.set(
            quest::TEMPLATE,
            &base.key(),
            "ZoneOrSort",
            Some("-22".to_string()),
        );
        let shown = base.with_edits(&edits).expect("edited");
        assert_eq!(shown.zone_or_sort, -22);
        assert!(shown.zone.is_empty());
    }

    /// The second line says what kind of quest it is, and says nothing for the
    /// ordinary values.
    #[test]
    fn the_second_line_names_what_is_unusual() {
        let mut known = a_quest(783);
        assert_eq!(known.sub(), "level 1 \u{b7} Northshire Valley");
        known.kind = 81;
        known.special_flags = 1;
        known.method = 0;
        assert_eq!(
            known.sub(),
            "level 1 \u{b7} Northshire Valley \u{b7} Dungeon \u{b7} auto-complete \u{b7} repeatable"
        );
    }

    /// The four tables are the two holders by the two roles, each way.
    #[test]
    fn a_relation_names_its_own_table() {
        for table in quest::RELATIONS {
            let (holder, role) = relation_of_table(table).expect(table);
            assert_eq!(table_of(holder, role), table);
        }
        assert_eq!(WILLEM_GIVES.table(), "creature_questrelation");
        assert_eq!(MCBRIDE_TAKES.table(), "creature_involvedrelation");
        assert_eq!(WILLEM_GIVES.key().text(), "id=823;quest=783");
    }

    /// Both directions read one list, with the project's rows over it: a
    /// relation the project adds is listed, and one it removes is still listed
    /// and says so.
    #[test]
    fn relations_are_the_databases_with_the_projects_over_them() {
        let quests = held(&[WILLEM_GIVES, MCBRIDE_TAKES]);
        let mut edits = Edits::default();
        assert_eq!(quests.relations_of_quest(783, &edits).len(), 2);
        assert_eq!(
            quests
                .relations_of_holder(Holder::Creature, 823, &edits)
                .len(),
            1
        );

        // Removed: still in the list, marked.
        edits.set_life(WILLEM_GIVES.table(), &WILLEM_GIVES.key(), Life::Delete);
        let of = quests.relations_of_quest(783, &edits);
        assert_eq!(of.len(), 2);
        assert_eq!(of[0], (WILLEM_GIVES, Life::Delete));

        // Added: a game object takes it as well.
        let chest = Relation {
            holder: Holder::Object,
            role: Role::Takes,
            id: 55,
            quest: 783,
        };
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        row.columns.insert("patch_min".to_string(), "0".to_string());
        row.columns
            .insert("patch_max".to_string(), "10".to_string());
        edits.set_row_line(chest.table(), &chest.key(), Some(&row.to_line()));
        let of = quests.relations_of_quest(783, &edits);
        assert_eq!(of.len(), 3);
        assert!(of.contains(&(chest, Life::Insert)));
        assert_eq!(
            quests.relations_of_holder(Holder::Object, 55, &edits).len(),
            1
        );
        // The givers come before the takers.
        assert_eq!(of[0].0.role, Role::Gives);
    }

    /// The list narrows to one creature's quests, and the search applies inside
    /// the narrowing.
    #[test]
    fn the_list_narrows_to_one_holders_quests() {
        let mut quests = held(&[WILLEM_GIVES]);
        let edits = Edits::default();
        assert_eq!(quests.matches(&edits, 0).len(), 2);
        quests.show_quests_of(Holder::Creature, 823);
        assert_eq!(quests.matches(&edits, 0), &[1]);
        quests.show_quests_of(Holder::Creature, 1);
        assert!(quests.matches(&edits, 0).is_empty());
    }

    /// A new quest is numbered above the reserved base, the table and the
    /// project's own, whichever is highest.
    #[test]
    fn a_new_entry_clears_all_three() {
        let mut quests = held(&[]);
        let mut edits = Edits::default();
        assert_eq!(quests.next_entry(&edits), quest::RESERVED_ENTRY_BASE);
        quests.max_entry = Some(quest::RESERVED_ENTRY_BASE + 4);
        assert_eq!(quests.next_entry(&edits), quest::RESERVED_ENTRY_BASE + 5);
        edits.set_life(
            quest::TEMPLATE,
            &quest::template_key(quest::RESERVED_ENTRY_BASE + 9, 10),
            Life::Insert,
        );
        assert_eq!(quests.next_entry(&edits), quest::RESERVED_ENTRY_BASE + 10);
    }

    /// A name that has not been fetched is asked for once, and one the database
    /// does not hold is remembered as absent rather than asked for again.
    #[test]
    fn a_missing_name_is_asked_for_once() {
        let mut quests = Quests::default();
        let none = Edits::default();
        assert!(quests.item(2589, &none).is_none());
        assert!(!quests.item_known(2589, &none));
        assert!(quests.wanted_items.contains(&2589));
        quests.wanted_items.clear();
        quests.items.insert(2589, None);
        assert!(quests.item(2589, &none).is_none());
        assert!(quests.item_known(2589, &none));
        assert!(
            quests.wanted_items.is_empty(),
            "an absent item is not asked for again"
        );
    }

    /// The case that was reported: a creature was created at entry 2000000,
    /// applied, put back and discarded, and a new creature made at the same
    /// entry was drawn under the first one's name in the form, because the
    /// name the database answered while the first was applied was kept for
    /// the session. The cache is emptied when the database is written, and
    /// the project's own row names its creature before the database does.
    #[test]
    fn a_name_follows_the_database_and_the_projects_own_row() {
        let mut quests = Quests::default();
        let none = Edits::default();
        // The first creature, applied: the database names it.
        quests.holders.renew(4);
        quests.holders.insert((Holder::Creature, 2_000_000), Some("Hamfort Gaga".into()));
        assert_eq!(quests.holder(Holder::Creature, 2_000_000, &none).as_deref(), Some("Hamfort Gaga"));
        // Put back: the counter moves and the name goes with it.
        assert!(quests.holders.renew(5));
        assert_eq!(quests.holder(Holder::Creature, 2_000_000, &none), None);
        assert!(!quests.holder_known(Holder::Creature, 2_000_000, &none), "asked for again");
        // A new creature at the same entry, which the database has never held.
        let mut created = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        created.columns.insert("name".into(), "'Hobart Stefa'".into());
        let mut edits = Edits::default();
        let key = vale_mangos::creature::template_key(2_000_000, 10);
        edits.set_row_line(vale_mangos::creature::TEMPLATE, &key, Some(&created.to_line()));
        assert_eq!(quests.holder(Holder::Creature, 2_000_000, &edits).as_deref(), Some("Hobart Stefa"));
        assert!(quests.holder_known(Holder::Creature, 2_000_000, &edits));
        // …and the project's rename is shown over a name the database holds.
        quests.holders.insert((Holder::Creature, 68), Some("Stormwind City Guard".into()));
        let mut renamed = Edits::default();
        renamed.set(vale_mangos::creature::TEMPLATE, &vale_mangos::creature::template_key(68, 0), "name", Some("'Gate Guard'".into()));
        assert_eq!(quests.holder(Holder::Creature, 68, &renamed).as_deref(), Some("Gate Guard"));
    }

    /// A search lists a creature the project creates, which the database has
    /// never held, and shows a rename the project makes.
    #[test]
    fn a_search_lists_the_projects_own_rows() {
        let mut edits = Edits::default();
        let mut created = RowEdit { life: Life::Insert, ..RowEdit::default() };
        created.columns.insert("name".into(), "'Hobart Stefa'".into());
        edits.set_row_line(vale_mangos::creature::TEMPLATE, &vale_mangos::creature::template_key(2_000_000, 10), Some(&created.to_line()));
        edits.set(vale_mangos::creature::TEMPLATE, &vale_mangos::creature::template_key(68, 0), "name", Some("'Gate Guard'".into()));
        let mut hits = vec![Hit { id: 68, title: "Stormwind City Guard".into(), sub: String::new() }];
        fold_the_projects_rows(&mut hits, Target::Creature, "g", &edits);
        assert_eq!(hits[0].title, "Gate Guard");
        assert!(hits.iter().all(|hit| hit.id != 2_000_000), "\"g\" is not in Hobart Stefa");
        fold_the_projects_rows(&mut hits, Target::Creature, "hobart", &edits);
        assert_eq!((hits[0].id, hits[0].title.as_str()), (2_000_000, "Hobart Stefa"));
        let mut by_entry = Vec::new();
        fold_the_projects_rows(&mut by_entry, Target::Creature, "2000000", &edits);
        assert_eq!(by_entry.len(), 1);
    }

    /// An item the project creates is named, coloured and pictured from its
    /// own row, and a column the project leaves alone comes from the database.
    #[test]
    fn an_item_is_named_from_the_projects_row_first() {
        let mut quests = Quests::default();
        quests.items.renew(0);
        quests.items.insert(2589, Some(ItemName { name: "Linen Cloth".into(), quality: 1, display_id: 7090 }));
        let mut edits = Edits::default();
        let key = vale_mangos::item::template_key(2589, 0);
        edits.set(vale_mangos::item::TEMPLATE, &key, "name", Some("'Fine Linen'".into()));
        let found = quests.item(2589, &edits).expect("named");
        assert_eq!((found.name.as_str(), found.quality, found.display_id), ("Fine Linen", 1, 7090));
    }
}

#[cfg(test)]
mod keystroke {
    use super::*;

    /// What one keystroke in the quest search costs, over the reference
    /// install's 4,433 quests. `--ignored --nocapture` prints it.
    #[test]
    #[ignore]
    fn bench_one_keystroke_over_the_whole_list() {
        let mut quests = Quests::default();
        quests.all = (1..=4_433)
            .map(|entry| Known {
                entry,
                read_entry: entry,
                patch: 0,
                title: format!("Quest number {entry}"),
                method: 2,
                zone_or_sort: 12,
                min_level: 1,
                level: 5,
                kind: 0,
                special_flags: 0,
                prev: 0,
                next: 0,
                next_in_chain: 0,
                zone: "Elwynn Forest".to_string(),
                claim: Life::Update,
            })
            .collect();
        let edits = Edits::default();
        let mut worst = std::time::Duration::ZERO;
        for (revision, query) in ["q", "qu", "que", "quest"].iter().enumerate() {
            quests.query = query.to_string();
            let started = std::time::Instant::now();
            let found = quests.matches(&edits, revision as u64).len();
            worst = worst.max(started.elapsed());
            println!("{query:>6}: {found} rows in {:?}", started.elapsed());
        }
        println!("worst keystroke: {worst:?}");
    }
}
