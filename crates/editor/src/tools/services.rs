//! What a creature sells and teaches, and what an edit to either list is.
//!
//! ## Two windows over four tables
//!
//! A creature's Vendor window shows `npc_vendor` under its own entry and
//! `npc_vendor_template` under its `vendor_id`; its Trainer window shows
//! `npc_trainer` and `npc_trainer_template` the same way, by `trainer_id`.
//! Each window has a tab per list. There is no rail entry: a list is named by
//! a creature, and the windows follow the selection as the loot window does.
//! [`About`] is what the shell fills from the selection each frame, with the
//! project's edits over the template row, so a `vendor_id` typed into the
//! template form changes the tab on the next frame.
//!
//! ## Both lists of a kind are read together
//!
//! The server reads a creature's own list against its template list: an item
//! or a spell already in the template list is skipped in the own list, and the
//! two vendor lists together hold at most 254 items. So while a window is open
//! both of its lists are read, one query each (`vale_mangos::vendor::rows_query`,
//! `vale_mangos::trainer::rows_query`), and kept until an apply lands
//! (`EditSession::database_writes`). A template list is read with the
//! creatures that name it, since an edit to it changes every one of them.
//!
//! ## A row is a row of the project's store
//!
//! On `crate::tools::loot`'s terms: adding one is a [`Life::Insert`] row
//! carrying every column that is not the key, removing one in the database is
//! a [`Life::Delete`] row, removing one this project added takes the claim
//! back, and a column edit is a value under the row's key.
//!
//! ## What is refused before it is written
//!
//! An add is refused when the server would skip the row for a reason the
//! editor can see: an item or a spell already in the list or in the
//! creature's template list, a list already at 254 items, a spell that is not
//! in `Spell.dbc` or is a talent. A spell that is not a teaching spell is
//! replaced by the spell that teaches it when `Spell.dbc` has one
//! ([`teaching_spell`]), since that is what the loader's log line asks for.

use crate::session::{EditSession, Gesture};
use vale_edit::dbc::DbcFile;
use vale_mangos::row::{Assignment, Edits, Key, Life, RowEdit};
use vale_mangos::trainer::{self, Lesson, Taught};
use vale_mangos::vendor::{self, Ware};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::{HashMap, HashSet};

/// Which of the two windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Vendor,
    Trainer,
}

impl Kind {
    /// The creature's own table and the template table, in the order the
    /// tabs are drawn.
    pub fn tables(self) -> [&'static str; 2] {
        match self {
            Kind::Vendor => [vendor::VENDOR, vendor::TEMPLATE],
            Kind::Trainer => [trainer::TRAINER, trainer::TEMPLATE],
        }
    }

    /// The `npc_flags` bit the client reads to offer the window at all:
    /// `UNIT_NPC_FLAG_VENDOR` and `UNIT_NPC_FLAG_TRAINER`, from
    /// `UnitDefines.h`.
    pub fn flag(self) -> (u32, &'static str) {
        match self {
            Kind::Vendor => (0x4, "VENDOR"),
            Kind::Trainer => (0x10, "TRAINER"),
        }
    }

    /// The template column that names the shared list.
    pub fn template_column(self) -> &'static str {
        match self {
            Kind::Vendor => "vendor_id",
            Kind::Trainer => "trainer_id",
        }
    }
}

/// One list: a table, and an entry of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct List {
    pub table: &'static str,
    pub entry: u32,
}

impl List {
    pub fn new(table: &'static str, entry: u32) -> List {
        List { table, entry }
    }

    pub fn kind(&self) -> Kind {
        match vendor::table_named(self.table) {
            Some(_) => Kind::Vendor,
            None => Kind::Trainer,
        }
    }

    /// Whether this is a template list, shared by every creature that names it.
    pub fn is_template(&self) -> bool {
        self.table == vendor::TEMPLATE || self.table == trainer::TEMPLATE
    }
}

/// What the windows are about: the selected creature, with the project's
/// edits over its template row. Rebuilt by the shell every frame.
#[derive(Debug, Clone, PartialEq)]
pub struct About {
    pub entry: u32,
    pub label: String,
    /// The template row's key, for the offers that write a template column.
    pub template_key: Key,
    pub npc_flags: u32,
    pub vendor_id: u32,
    pub trainer_id: u32,
    pub trainer_type: u32,
    pub trainer_class: u32,
    pub trainer_race: u32,
    pub trainer_spell: u32,
}

impl About {
    /// The creature's two lists of a kind: its own, then its template's. The
    /// template list's entry is 0 when the column names none.
    pub fn lists(&self, kind: Kind) -> [List; 2] {
        let [own, template] = kind.tables();
        let shared = match kind {
            Kind::Vendor => self.vendor_id,
            Kind::Trainer => self.trainer_id,
        };
        [List::new(own, self.entry), List::new(template, shared)]
    }

    /// Whether `npc_flags` carries the kind's bit.
    pub fn has_flag(&self, kind: Kind) -> bool {
        self.npc_flags & kind.flag().0 != 0
    }
}

/// One vendor row as the window draws it: the database's reading with the
/// project's edits over it, and what the project says is to become of it.
#[derive(Debug, Clone, PartialEq)]
pub struct ShownWare {
    pub ware: Ware,
    pub life: Life,
    /// The row as the database has it, for an edit typed back to that value
    /// to be cleared. `None` for a row this project creates.
    pub in_database: Option<Ware>,
    /// Whether `forbidden_items` leaves the row out at the server's patch.
    pub forbidden: bool,
}

/// …and one trainer row.
#[derive(Debug, Clone, PartialEq)]
pub struct ShownLesson {
    pub lesson: Lesson,
    pub life: Life,
    pub in_database: Option<Lesson>,
}

/// A vendor row as the database answered it.
#[derive(Debug, Clone, PartialEq)]
struct HeldWare {
    ware: Ware,
    forbidden: bool,
}

/// What one read answered.
enum Read {
    Wares(Vec<HeldWare>),
    Lessons(Vec<Lesson>),
}

/// What the two windows are holding.
#[derive(Resource, Default)]
pub struct Services {
    pub vendor_open: bool,
    pub trainer_open: bool,
    /// The selected creature, which the shell writes each frame; `None`
    /// under another tool, or with nothing selected.
    pub about: Option<About>,
    /// Which list each window shows: 0 the creature's own, 1 the template's.
    /// `None` until a tab is pressed; see [`Self::showing`].
    pub vendor_tab: Option<usize>,
    pub trainer_tab: Option<usize>,
    wares: HashMap<List, Vec<HeldWare>>,
    lessons: HashMap<List, Vec<Lesson>>,
    /// The creatures that name each template list read so far, as entry and
    /// name.
    users: HashMap<List, Vec<(u32, String)>>,
    /// What those were read at: `EditSession::database_writes`.
    loaded_for: Option<u64>,
    reading: Option<(List, Task<Result<(Read, Vec<(u32, String)>), String>>)>,
    /// Why the last read answered nothing, when it answered nothing.
    pub trouble: Option<String>,
    /// Every spell id that is a rank of a talent, out of `Talent.dbc`; `None`
    /// until the trainer window has opened the table.
    pub talents: Option<HashSet<u32>>,
    /// Every row of `SkillLine.dbc` in the groups the `reqskill` menu lists;
    /// `None` until the trainer window has opened the table.
    pub skill_lines: Option<Vec<SkillGroup>>,
    /// Whether the scripted flags have been acted on; see
    /// `crate::server::services::on_the_command_line`, which waits on this.
    pub scripted_done: bool,
    /// `--vendor-find` or `--trainer-find`: the window whose add-picker is
    /// to be opened, and the text to search, taken by the window the first
    /// time it draws that list.
    pub find: Option<(Kind, String)>,
}

impl Services {
    /// Whether a window is open.
    pub fn is_open(&self, kind: Kind) -> bool {
        match kind {
            Kind::Vendor => self.vendor_open,
            Kind::Trainer => self.trainer_open,
        }
    }

    /// Open a window, or shut it. Opening leaves the tab to [`Self::showing`].
    pub fn toggle(&mut self, kind: Kind) {
        match kind {
            Kind::Vendor => {
                self.vendor_open = !self.vendor_open;
                self.vendor_tab = None;
            }
            Kind::Trainer => {
                self.trainer_open = !self.trainer_open;
                self.trainer_tab = None;
            }
        }
    }

    /// Show one tab of a window, as a press on it does.
    pub fn set_tab(&mut self, kind: Kind, tab: usize) {
        match kind {
            Kind::Vendor => self.vendor_tab = Some(tab.min(1)),
            Kind::Trainer => self.trainer_tab = Some(tab.min(1)),
        }
    }

    /// Which tab a window shows: the one pressed, or, until one is, the
    /// template list when the creature's own list has no rows in the
    /// database and a template list is named. Zaldimar Wefhellt (328), a mage
    /// trainer, is that case: his own list is empty, and `trainer_id` 1 names
    /// a template list of 146 spells at build 5875.
    pub fn tab(&self, kind: Kind) -> usize {
        let pressed = match kind {
            Kind::Vendor => self.vendor_tab,
            Kind::Trainer => self.trainer_tab,
        };
        if let Some(tab) = pressed {
            return tab;
        }
        let Some(about) = self.about.as_ref() else { return 0 };
        let [own, template] = about.lists(kind);
        let own_rows = self.wares.get(&own).map(Vec::len).or_else(|| self.lessons.get(&own).map(Vec::len));
        match (own_rows, template.entry) {
            (Some(0), entry) if entry != 0 => 1,
            _ => 0,
        }
    }

    /// The list a window shows now.
    pub fn showing(&self, kind: Kind) -> Option<List> {
        let about = self.about.as_ref()?;
        Some(about.lists(kind)[self.tab(kind)])
    }

    /// Whether a list's rows are in hand. A list with entry 0 is never read
    /// and counts as read, with no rows.
    pub fn is_read(&self, list: List) -> bool {
        list.entry == 0 || self.wares.contains_key(&list) || self.lessons.contains_key(&list)
    }

    /// The creatures that name a template list, once it has been read: the
    /// database's, with the project's `vendor_id` or `trainer_id` edits and
    /// its created templates over them.
    pub fn users(&self, list: List, edits: &Edits) -> Option<Vec<(u32, String)>> {
        let from_database = self.users.get(&list)?;
        Some(crate::server::fresh::naming(from_database, edits, list.kind().template_column(), list.entry))
    }

    /// Every vendor row of one list, the database's with the project's over
    /// them, in the order the server sends them: by slot, then by item.
    pub fn wares_of(&self, list: List, edits: &Edits) -> Vec<ShownWare> {
        let mut out: Vec<ShownWare> = self
            .wares
            .get(&list)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .map(|held| ShownWare {
                ware: ware_with_edits(&held.ware, list.table, edits),
                life: edits.life(list.table, &held.ware.key()),
                in_database: Some(held.ware.clone()),
                forbidden: held.forbidden,
            })
            .collect();
        for (table, key, row) in edits.rows() {
            if row.life != Life::Insert || table != list.table {
                continue;
            }
            let Some(ware) = Ware::from_row(&whole_row(key, row)) else {
                continue;
            };
            if ware.entry != list.entry || out.iter().any(|shown| shown.ware.key() == *key) {
                continue;
            }
            out.push(ShownWare {
                ware,
                life: Life::Insert,
                in_database: None,
                forbidden: false,
            });
        }
        out.sort_by_key(|shown| (shown.ware.slot, shown.ware.item));
        out
    }

    /// Every trainer row of one list, the database's with the project's over
    /// them, by spell.
    pub fn lessons_of(&self, list: List, edits: &Edits) -> Vec<ShownLesson> {
        let mut out: Vec<ShownLesson> = self
            .lessons
            .get(&list)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .map(|lesson| ShownLesson {
                lesson: lesson_with_edits(lesson, list.table, edits),
                life: edits.life(list.table, &lesson.key()),
                in_database: Some(lesson.clone()),
            })
            .collect();
        for (table, key, row) in edits.rows() {
            if row.life != Life::Insert || table != list.table {
                continue;
            }
            let Some(lesson) = Lesson::from_row(&whole_row(key, row)) else {
                continue;
            };
            if lesson.entry != list.entry || out.iter().any(|shown| shown.lesson.key() == *key) {
                continue;
            }
            out.push(ShownLesson {
                lesson,
                life: Life::Insert,
                in_database: None,
            });
        }
        out.sort_by_key(|shown| shown.lesson.spell);
        out
    }

    /// Add an item to a vendor list, after every row it already has. A row in
    /// the database that is marked for removal is kept instead. Refused, with
    /// the reason, when the server would skip it: see the module comment.
    /// `other` is the creature's other vendor list, for the duplicate and
    /// count checks.
    pub fn add_item(
        &mut self,
        session: &mut EditSession,
        list: List,
        other: Option<List>,
        item: u32,
        now: f64,
    ) -> Result<String, String> {
        let shown = self.wares_of(list, &session.server_edits);
        let kept: Vec<&ShownWare> = shown.iter().filter(|s| s.life != Life::Delete).collect();
        if kept.iter().any(|s| s.ware.item == item) {
            return Err(format!("item {item} is already in {} {}", list.table, list.entry));
        }
        let other_rows = match other {
            Some(other) if other.entry != 0 => self.wares_of(other, &session.server_edits),
            _ => Vec::new(),
        };
        // The count leaves out what `forbidden_items` removes, as the
        // loader's `SELECT` does.
        let counted = kept.iter().filter(|s| !s.forbidden).count()
            + other_rows.iter().filter(|s| s.life != Life::Delete && !s.forbidden).count();
        if !list.is_template() && other_rows.iter().any(|s| s.life != Life::Delete && s.ware.item == item) {
            return Err(format!(
                "item {item} is already in the template list {} {}, so the server would skip it in the \
                 creature's own list",
                vendor::TEMPLATE,
                other.map_or(0, |o| o.entry)
            ));
        }
        if counted + 1 >= vendor::MAX_ITEMS {
            return Err(format!(
                "the two lists would hold {} items; the server skips every row past {}",
                vendor::MAX_ITEMS,
                vendor::MAX_ITEMS - 1
            ));
        }
        let key = vendor::key(list.entry, item);
        let label = format!("{} {}", list.table, key.text());
        let gesture = Gesture {
            label: "Add vendor item",
            subject: &label,
            now,
        };
        if shown.iter().any(|s| s.ware.item == item && s.in_database.is_some()) {
            session.set_server_row(list.table, &key, None, Some(gesture));
            return Ok(format!("cancelled removal of item {item} in {} {}", list.table, list.entry));
        }
        let slot = kept.iter().map(|s| s.ware.slot).max().unwrap_or(0) + 1;
        let ware = Ware::new(list.entry, item, slot);
        session.set_server_row(list.table, &key, Some(&creation(&ware.assignments())), Some(gesture));
        Ok(format!("item {item} added to {} {} in slot {slot}", list.table, list.entry))
    }

    /// Add a teaching spell to a trainer list. Refused, with the reason, when
    /// the server would skip it; `taught` is what `Spell.dbc` and `Talent.dbc`
    /// say about the spell, and `other` is the creature's other trainer list.
    pub fn add_spell(
        &mut self,
        session: &mut EditSession,
        list: List,
        other: Option<List>,
        spell: u32,
        taught: Option<Taught>,
        now: f64,
    ) -> Result<String, String> {
        let faults = trainer::spell_faults(taught, None);
        let faults: Vec<String> = faults.into_iter().chain(Lesson::new(list.entry, spell).check()).collect();
        if !faults.is_empty() {
            return Err(format!("spell {spell}: {}", faults.join("; ")));
        }
        let shown = self.lessons_of(list, &session.server_edits);
        if shown.iter().any(|s| s.life != Life::Delete && s.lesson.spell == spell) {
            return Err(format!("spell {spell} is already in {} {}", list.table, list.entry));
        }
        if let Some(other) = other.filter(|other| other.entry != 0 && !list.is_template()) {
            let taught_there = self
                .lessons_of(other, &session.server_edits)
                .iter()
                .any(|s| s.life != Life::Delete && s.lesson.spell == spell);
            if taught_there {
                return Err(format!(
                    "spell {spell} is already in the template list {} {}, so the server would skip it \
                     in the creature's own list",
                    trainer::TEMPLATE,
                    other.entry
                ));
            }
        }
        let lesson = Lesson::new(list.entry, spell);
        let key = lesson.key();
        let label = format!("{} {}", list.table, key.text());
        let gesture = Gesture {
            label: "Add trainer spell",
            subject: &label,
            now,
        };
        if shown.iter().any(|s| s.lesson.key() == key && s.in_database.is_some()) {
            session.set_server_row(list.table, &key, None, Some(gesture));
            return Ok(format!("cancelled removal of spell {spell} in {} {}", list.table, list.entry));
        }
        session.set_server_row(list.table, &key, Some(&creation(&lesson.assignments())), Some(gesture));
        Ok(format!("spell {spell} added to {} {}", list.table, list.entry))
    }

    /// Remove a row: a `Delete` claim for one in the database, and the claim
    /// taken back for one this project added.
    pub fn remove(&mut self, session: &mut EditSession, table: &'static str, key: &Key, in_database: bool, now: f64) {
        let label = format!("{table} {}", key.text());
        let gesture = Gesture {
            label: "Remove vendor or trainer row",
            subject: &label,
            now,
        };
        match in_database {
            true => {
                let row = RowEdit {
                    life: Life::Delete,
                    ..RowEdit::default()
                };
                session.set_server_row(table, key, Some(&row), Some(gesture));
            }
            false => session.set_server_row(table, key, None, Some(gesture)),
        }
    }

    /// Keep a row that was marked for removal: the claim is taken back, and
    /// with it any column edit made before the mark.
    pub fn keep(&mut self, session: &mut EditSession, table: &'static str, key: &Key, now: f64) {
        let label = format!("{table} {}", key.text());
        session.set_server_row(
            table,
            key,
            None,
            Some(Gesture {
                label: "Keep vendor or trainer row",
                subject: &label,
                now,
            }),
        );
    }

    /// Set one column of a row. On a row the database holds, the edit is
    /// cleared when it is what the database holds; on a row this project
    /// creates, the column is written into the creation. `held` is the row's
    /// columns as the database has them, `None` for a created row.
    #[allow(clippy::too_many_arguments)]
    pub fn set_column(
        &mut self,
        session: &mut EditSession,
        table: &'static str,
        key: &Key,
        held: Option<Vec<Assignment>>,
        column: &'static str,
        value: String,
        now: f64,
    ) {
        let label = format!("{table} {} {column}", key.text());
        let gesture = Gesture {
            label: "Edit vendor or trainer row",
            subject: &label,
            now,
        };
        write_column(session, table, key, held.as_deref(), column, value, gesture);
    }

    /// Move a vendor row one place up (`-1`) or down (`1`) in its list. The
    /// list is renumbered from 1 in its new order, which changes two slots in
    /// a list already numbered that way, and every change is one entry on the
    /// undo stack.
    pub fn move_ware(&mut self, session: &mut EditSession, list: List, item: u32, by: i32, now: f64) {
        let mut order: Vec<ShownWare> = self
            .wares_of(list, &session.server_edits)
            .into_iter()
            .filter(|s| s.life != Life::Delete)
            .collect();
        let Some(at) = order.iter().position(|s| s.ware.item == item) else {
            return;
        };
        let to = at as i64 + by as i64;
        if to < 0 || to >= order.len() as i64 {
            return;
        }
        order.swap(at, to as usize);
        let label = format!("{} {} order", list.table, list.entry);
        let gesture = Gesture {
            label: "Move vendor item",
            subject: &label,
            now,
        };
        for (index, shown) in order.iter().enumerate() {
            let slot = index as u32 + 1;
            if shown.ware.slot == slot {
                continue;
            }
            let held = shown.in_database.as_ref().map(Ware::assignments);
            write_column(
                session,
                list.table,
                &shown.ware.key(),
                held.as_deref(),
                "slot",
                slot.to_string(),
                gesture,
            );
        }
    }
}

/// A created row's claim: every column it is written with.
pub(crate) fn creation(changes: &[Assignment]) -> RowEdit {
    let mut row = RowEdit {
        life: Life::Insert,
        ..RowEdit::default()
    };
    for change in changes {
        row.columns.insert(change.column.to_string(), change.value.clone());
    }
    row
}

/// One column written under a gesture; see [`Services::set_column`].
///
/// A row the project creates is written into its creation even when the
/// database also holds a row under that key, which it does after the creation
/// has been applied. Clearing a column of a creation because it matches the
/// database's left the creation without that column once the database's row
/// was put back, and the plan then refused the whole row.
pub(crate) fn write_column(
    session: &mut EditSession,
    table: &'static str,
    key: &Key,
    held: Option<&[Assignment]>,
    column: &'static str,
    value: String,
    gesture: Gesture<'_>,
) {
    let creating = session.server_edits.row(table, key).is_some_and(|row| row.life == Life::Insert);
    match held.filter(|_| !creating) {
        Some(held) => {
            let had = held.iter().find(|change| change.column == column).map(|change| &change.value);
            let value = match had == Some(&value) {
                true => None,
                false => Some(value),
            };
            session.set_server_edit(table, key, column, value, Some(gesture));
        }
        None => {
            let Some(mut row) = session.server_edits.row(table, key).cloned() else {
                return;
            };
            row.columns.insert(column.to_string(), value);
            session.set_server_row(table, key, Some(&row), Some(gesture));
        }
    }
}

/// A claim's key and columns as one row, for reading it back.
fn whole_row(key: &Key, row: &RowEdit) -> vale_mangos::schema::Row {
    let mut whole = vale_mangos::schema::Row::new();
    for (column, value) in &key.0 {
        whole.insert(column.clone(), Some(value.clone()));
    }
    for (column, value) in &row.columns {
        whole.insert(column.clone(), Some(value.clone()));
    }
    whole
}

/// A value of the store read as a number.
fn number(edits: &Edits, table: &str, key: &Key, column: &str) -> Option<u32> {
    let value: i64 = edits.get(table, key, column)?.trim().parse().ok()?;
    Some(value.max(0) as u32)
}

/// A database vendor row with the project's column edits over it.
fn ware_with_edits(ware: &Ware, table: &str, edits: &Edits) -> Ware {
    let key = ware.key();
    let at = |column: &str, had: u32| number(edits, table, &key, column).unwrap_or(had);
    Ware {
        slot: at("slot", ware.slot),
        maxcount: at("maxcount", ware.maxcount),
        incrtime: at("incrtime", ware.incrtime),
        flags: at("itemflags", ware.flags),
        condition: at("condition_id", ware.condition),
        ..ware.clone()
    }
}

/// …and a trainer row.
fn lesson_with_edits(lesson: &Lesson, table: &str, edits: &Edits) -> Lesson {
    let key = lesson.key();
    let at = |column: &str, had: u32| number(edits, table, &key, column).unwrap_or(had);
    Lesson {
        cost: at("spellcost", lesson.cost),
        skill: at("reqskill", lesson.skill),
        skill_value: at("reqskillvalue", lesson.skill_value),
        level: at("reqlevel", lesson.level),
        ..lesson.clone()
    }
}

// ---------------------------------------------------------------------------
// What Spell.dbc and Talent.dbc say about a spell
// ---------------------------------------------------------------------------

/// What `Spell.dbc` says about one spell, with whether it is a talent, or
/// `None` for a spell the table does not hold.
pub fn taught(spells: &DbcFile, talents: &HashSet<u32>, spell: u32) -> Option<Taught> {
    use vale_assets::tables::spellbook::spell_fields;
    let record = spells.row_of(spell)?;
    Some(Taught {
        effect: spells.u32_at(record, spell_fields::EFFECT).unwrap_or(0),
        teaches: spells.u32_at(record, spell_fields::EFFECT_TRIGGER_SPELL).unwrap_or(0),
        level: spells.u32_at(record, spell_fields::SPELL_LEVEL).unwrap_or(0),
        talent: talents.contains(&spell),
    })
}

/// `CastingTimeIndex` 1, `SpellCastTimes.dbc`'s row for an instant cast.
const INSTANT: u32 = 1;

/// The teaching spell for `spell`: a row of `Spell.dbc` whose first effect is
/// `LEARN_SPELL` and whose `EffectTriggerSpell[0]` is `spell`, which is the
/// search `LoadTrainers` makes for its log line.
///
/// 522 spells have more than one teaching spell. Fireball rank 2 (143) has
/// two: 483, which has a cast time and is the one a book casts, and 1173,
/// which is instant and is the one both mage trainer lists name. On the
/// reference database 1,821 of the 1,826 teaching spells that trainer lists
/// name are instant, and 468 of the 522 have exactly one instant teacher, so
/// an instant teacher is preferred, then the first in the file.
pub fn teaching_spell(spells: &DbcFile, spell: u32) -> Option<u32> {
    use vale_assets::tables::spellbook::spell_fields;
    let teachers: Vec<(u32, bool)> = (0..spells.record_count())
        .filter_map(|record| {
            let teaches = spells.u32_at(record, spell_fields::EFFECT_TRIGGER_SPELL)? == spell;
            let learns = spells.u32_at(record, spell_fields::EFFECT)? == trainer::LEARN_SPELL;
            let instant = spells.u32_at(record, spell_fields::CASTING_TIME_INDEX) == Some(INSTANT);
            (teaches && learns).then(|| Some((spells.u32_at(record, 0)?, instant))).flatten()
        })
        .collect();
    teachers
        .iter()
        .find(|(_, instant)| *instant)
        .or(teachers.first())
        .map(|(id, _)| *id)
}

/// The spell a trainer row should name for a spell a person chose: the spell
/// itself when it teaches, otherwise the spell that teaches it, when there is
/// one. Returns the spell and, when it was replaced, the spell chosen.
pub fn resolve_teaching(spells: &DbcFile, talents: &HashSet<u32>, chosen: u32) -> (u32, Option<u32>) {
    match taught(spells, talents, chosen) {
        Some(facts) if !facts.is_teaching() => match teaching_spell(spells, chosen) {
            Some(teacher) => (teacher, Some(chosen)),
            None => (chosen, None),
        },
        _ => (chosen, None),
    }
}

/// One skill line, as the `reqskill` menu lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillLine {
    pub id: u32,
    pub name: String,
}

impl SkillLine {
    /// Whether the line answers what is typed into the menu's filter: part of
    /// its name, in any case, or its id. An empty filter answers every line.
    pub fn matches(&self, filter: &str) -> bool {
        let filter = filter.trim();
        filter.is_empty()
            || self.id.to_string() == filter
            || self.name.to_ascii_lowercase().contains(&filter.to_ascii_lowercase())
    }
}

/// The skill lines of one `SkillLineCategory.dbc` row, under its name.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillGroup {
    pub category: u32,
    pub name: String,
    pub lines: Vec<SkillLine>,
}

/// `SkillLine.dbc`'s category and enUS name, and `SkillLineCategory.dbc`'s
/// enUS name and sort order: the fields `vale_assets::tables::skills` reads.
const LINE_CATEGORY: usize = 1;
const LINE_NAME: usize = 3;
const CATEGORY_NAME: usize = 1;
const CATEGORY_SORT: usize = 10;

/// `SkillLineCategory.dbc` rows 11, Professions, and 9, Secondary Skills:
/// the lines a trainer's `reqskill` names. They are listed first.
pub const TRAINED_CATEGORIES: [u32; 2] = [11, 9];

/// Every named skill line, grouped by category: the two trained categories
/// first, then the others in the category table's own sort order, and by name
/// inside a group. A category the table does not name is headed by its id.
pub fn skill_groups(lines: &DbcFile, categories: Option<&DbcFile>) -> Vec<SkillGroup> {
    let mut groups: Vec<SkillGroup> = Vec::new();
    for record in 0..lines.record_count() {
        let (Some(id), Some(name)) = (lines.u32_at(record, 0), lines.string_at(record, LINE_NAME)) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let category = lines.u32_at(record, LINE_CATEGORY).unwrap_or(0);
        let line = SkillLine { id, name };
        match groups.iter_mut().find(|group| group.category == category) {
            Some(group) => group.lines.push(line),
            None => groups.push(SkillGroup {
                category,
                name: String::new(),
                lines: vec![line],
            }),
        }
    }
    let category_row = |category: u32| categories.and_then(|table| Some((table, table.row_of(category)?)));
    for group in &mut groups {
        group.name = category_row(group.category)
            .and_then(|(table, record)| table.string_at(record, CATEGORY_NAME))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("category {}", group.category));
        group.lines.sort_by(|a, b| a.name.cmp(&b.name));
    }
    let sort = |category: u32| {
        let trained = TRAINED_CATEGORIES.iter().position(|c| *c == category);
        let order = category_row(category).and_then(|(table, record)| table.u32_at(record, CATEGORY_SORT));
        (trained.is_none(), trained, order.unwrap_or(u32::MAX), category)
    };
    groups.sort_by_key(|group| sort(group.category));
    groups
}

/// What a skill line id is called, out of the groups.
pub fn skill_name(groups: &[SkillGroup], id: u32) -> Option<&str> {
    groups
        .iter()
        .flat_map(|group| &group.lines)
        .find(|line| line.id == id)
        .map(|line| line.name.as_str())
}

/// Every spell id that is a rank of a talent, out of `Talent.dbc` and
/// `TalentTab.dbc` as `vale_assets::tables::talent` reads them. `None` when
/// either table is missing.
pub fn talent_spells(talent: &DbcFile, talent_tab: &DbcFile) -> Option<HashSet<u32>> {
    let talents = vale_assets::tables::talent::Talents::parse(&talent.write(), &talent_tab.write())?;
    Some(
        talents
            .talents()
            .iter()
            .flat_map(|row| row.ranks)
            .filter(|spell| *spell != 0)
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// The reads
// ---------------------------------------------------------------------------

/// Read the lists of every open window that have not been read, one at a
/// time on a task, and forget every list when an apply has moved the tables.
fn read_the_rows(
    mut services: ResMut<Services>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    if let Some((list, task)) = services.reading.as_mut() {
        let list = *list;
        if let Some(done) = block_on(future::poll_once(task)) {
            services.reading = None;
            match done {
                Ok((read, users)) => {
                    match read {
                        Read::Wares(rows) => {
                            info!("services: {} {}: {} row(s)", list.table, list.entry, rows.len());
                            services.wares.insert(list, rows);
                        }
                        Read::Lessons(rows) => {
                            info!("services: {} {}: {} row(s)", list.table, list.entry, rows.len());
                            services.lessons.insert(list, rows);
                        }
                    }
                    if list.is_template() {
                        services.users.insert(list, users);
                    }
                    services.trouble = None;
                }
                Err(e) => {
                    warn!("services: {e}");
                    services.trouble = Some(e);
                }
            }
        }
        return;
    }
    let Some(session) = session else { return };
    if services.loaded_for != Some(session.database_writes) {
        services.loaded_for = Some(session.database_writes);
        services.wares.clear();
        services.lessons.clear();
        services.users.clear();
        // A read that failed against the database as it was is tried again.
        services.trouble = None;
    }
    if services.trouble.is_some() {
        return;
    }
    let Some(about) = services.about.clone() else { return };
    let wanted = [Kind::Vendor, Kind::Trainer]
        .into_iter()
        .filter(|kind| services.is_open(*kind))
        .flat_map(|kind| about.lists(kind))
        .find(|list| !services.is_read(*list));
    let Some(list) = wanted else { return };
    let Some((at, _source)) = settings.resolve() else {
        services.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = super::creatures::server_patch(&settings);
    services.reading = Some((
        list,
        crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let read = match list.kind() {
                Kind::Vendor => {
                    let rows = db.rows(&vendor::rows_query(list.table, list.entry, patch))?;
                    Read::Wares(
                        rows.iter()
                            .filter_map(|row| {
                                use vale_mangos::schema::RowValue;
                                Some(HeldWare {
                                    ware: Ware::from_row(row)?,
                                    forbidden: row.integer("forbidden").unwrap_or(0) != 0,
                                })
                            })
                            .collect(),
                    )
                }
                Kind::Trainer => {
                    let rows = db.rows(&trainer::rows_query(list.table, list.entry))?;
                    Read::Lessons(rows.iter().filter_map(Lesson::from_row).collect())
                }
            };
            let users = match (list.is_template(), list.kind()) {
                (false, _) => Vec::new(),
                (true, kind) => {
                    let sql = match kind {
                        Kind::Vendor => vendor::users_query(list.entry),
                        Kind::Trainer => trainer::users_query(list.entry),
                    };
                    db.rows(&sql)?
                        .iter()
                        .filter_map(|row| {
                            use vale_mangos::schema::RowValue;
                            Some((row.integer("entry")? as u32, row.text("name").unwrap_or("").to_string()))
                        })
                        .collect()
                }
            };
            Ok((read, users))
        }),
    ));
}

/// `--vendor`, `--trainer`, `--vendor-add <item>`, `--trainer-add <spell>`,
/// `--vendor-find <text>` and `--trainer-find <text>`: the windows opened, a
/// row added to the creature's own list, and a window's add-picker opened,
/// with nobody at the keyboard. An add waits for `--spawn`'s creature and for both
/// lists of its kind to be read.
fn on_the_command_line(
    args: Res<crate::Args>,
    mut services: ResMut<Services>,
    mut session: Option<ResMut<EditSession>>,
    assets: Option<Res<vale_client::assets::GameAssets>>,
    time: Res<Time>,
    mut opened: Local<bool>,
    mut pending: Local<Option<Vec<(Kind, u32)>>>,
) {
    if !*opened {
        *opened = true;
        services.vendor_open = args.vendor_window;
        services.trainer_open = args.trainer_window;
        services.find = args
            .vendor_find
            .clone()
            .map(|text| (Kind::Vendor, text))
            .or_else(|| args.trainer_find.clone().map(|text| (Kind::Trainer, text)));
        let mut adds = Vec::new();
        adds.extend(args.vendor_add.map(|item| (Kind::Vendor, item)));
        adds.extend(args.trainer_add.map(|spell| (Kind::Trainer, spell)));
        *pending = Some(adds);
    }
    if services.scripted_done {
        return;
    }
    let Some(adds) = pending.as_mut() else { return };
    let Some((kind, value)) = adds.first().copied() else {
        services.scripted_done = true;
        return;
    };
    let Some(about) = services.about.clone() else { return };
    let [own, template] = about.lists(kind);
    if !services.is_read(own) || !services.is_read(template) {
        if services.trouble.is_some() {
            services.scripted_done = true;
        }
        return;
    }
    let Some(session) = session.as_mut() else { return };
    let now = time.elapsed_secs_f64();
    let done = match kind {
        Kind::Vendor => services.add_item(session, own, Some(template), value, now),
        Kind::Trainer => {
            let Some(assets) = assets.as_ref() else { return };
            if !(session.open_table(assets, "Spell")
                && session.open_table(assets, "Talent")
                && session.open_table(assets, "TalentTab"))
            {
                warn!("--trainer-add {value}: Spell.dbc or Talent.dbc is not in the archives");
                services.scripted_done = true;
                return;
            }
            if services.talents.is_none() {
                services.talents = match (session.table("Talent"), session.table("TalentTab")) {
                    (Some(talent), Some(tab)) => talent_spells(talent, tab),
                    _ => None,
                };
            }
            let talents = services.talents.clone().unwrap_or_default();
            let Some(spells) = session.table("Spell") else { return };
            let (spell, replaced) = resolve_teaching(spells, &talents, value);
            let facts = taught(spells, &talents, spell);
            if let Some(chosen) = replaced {
                info!("--trainer-add {chosen}: not a teaching spell; spell {spell} teaches it");
            }
            services.add_spell(session, own, Some(template), spell, facts, now)
        }
    };
    let flag = match kind {
        Kind::Vendor => "--vendor-add",
        Kind::Trainer => "--trainer-add",
    };
    match done {
        Ok(line) => info!("{flag} {value}: {line}"),
        Err(e) => warn!("{flag} {value}: {e}"),
    }
    adds.remove(0);
}

pub struct ServicesToolPlugin;

impl Plugin for ServicesToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Services>()
            .add_systems(Update, (read_the_rows, on_the_command_line).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn about() -> About {
        About {
            entry: 3044,
            label: "Miles Welsh (3044)".into(),
            template_key: Key::one("entry", 3044),
            npc_flags: 0x13,
            vendor_id: 0,
            trainer_id: 8,
            trainer_type: 0,
            trainer_class: 5,
            trainer_race: 0,
            trainer_spell: 0,
        }
    }

    fn held_wares(services: &mut Services, list: List, wares: &[Ware]) {
        let rows = wares.iter().map(|ware| HeldWare { ware: ware.clone(), forbidden: false }).collect();
        services.wares.insert(list, rows);
    }

    /// The filter answers part of a name in any case, or the whole id.
    #[test]
    fn a_skill_line_is_found_by_name_or_id() {
        let line = SkillLine { id: 165, name: "Leatherworking".into() };
        assert!(line.matches(""));
        assert!(line.matches("leather"));
        assert!(line.matches("WORK"));
        assert!(line.matches("165"));
        assert!(!line.matches("16"), "an id matches only in full");
        assert!(!line.matches("tailor"));
    }

    /// The tabs are the creature's own list and its template's, and a
    /// template column of 0 is a list that counts as read with no rows.
    #[test]
    fn a_creature_names_two_lists_of_each_kind() {
        let mut services = Services { about: Some(about()), ..Services::default() };
        assert_eq!(services.showing(Kind::Trainer), Some(List::new(trainer::TRAINER, 3044)));
        services.set_tab(Kind::Trainer, 1);
        assert_eq!(services.showing(Kind::Trainer), Some(List::new(trainer::TEMPLATE, 8)));
        let [_, template] = about().lists(Kind::Vendor);
        assert_eq!(template.entry, 0);
        assert!(services.is_read(template));
        assert!(about().has_flag(Kind::Trainer));
        assert!(!about().has_flag(Kind::Vendor));
    }

    /// Until a tab is pressed, a window whose own list is empty in the
    /// database opens on the template list, and one with no template list on
    /// the own list.
    #[test]
    fn an_empty_own_list_opens_on_the_template() {
        let mut services = Services { about: Some(about()), ..Services::default() };
        assert_eq!(services.tab(Kind::Trainer), 0, "nothing read yet");
        services.lessons.insert(List::new(trainer::TRAINER, 3044), Vec::new());
        assert_eq!(services.tab(Kind::Trainer), 1);
        services.set_tab(Kind::Trainer, 0);
        assert_eq!(services.tab(Kind::Trainer), 0);
        services.wares.insert(List::new(vendor::VENDOR, 3044), Vec::new());
        assert_eq!(services.tab(Kind::Vendor), 0, "vendor_id is 0: no template list");
    }

    /// The database's rows come back with the project's edits over them, a
    /// creation after them in slot order, and each with its life.
    #[test]
    fn a_list_is_drawn_with_the_projects_claims_over_it() {
        let list = List::new(vendor::VENDOR, 54);
        let mut services = Services::default();
        held_wares(&mut services, list, &[Ware::new(54, 2488, 1), Ware::new(54, 2489, 2)]);
        let mut edits = Edits::default();
        edits.set(vendor::VENDOR, &vendor::key(54, 2488), "slot", Some("3".into()));
        edits.set_life(vendor::VENDOR, &vendor::key(54, 2489), Life::Delete);
        let created = Ware::new(54, 2490, 2);
        edits.set_row_line(vendor::VENDOR, &created.key(), Some(&creation(&created.assignments()).to_line()));
        let shown = services.wares_of(list, &edits);
        let items: Vec<u32> = shown.iter().map(|s| s.ware.item).collect();
        assert_eq!(items, [2489, 2490, 2488]);
        assert_eq!(shown[0].life, Life::Delete);
        assert_eq!(shown[1].life, Life::Insert);
        assert_eq!(shown[2].ware.slot, 3);
    }

    /// A trainer row read back out of a creation, and one of another list
    /// left out.
    #[test]
    fn a_created_lesson_is_shown_in_its_own_list_only() {
        let list = List::new(trainer::TRAINER, 3044);
        let mut services = Services::default();
        services.lessons.insert(list, vec![Lesson::new(3044, 1173)]);
        let mut edits = Edits::default();
        for entry in [3044, 3045] {
            let lesson = Lesson { cost: 90, ..Lesson::new(entry, 5504) };
            edits.set_row_line(trainer::TRAINER, &lesson.key(), Some(&creation(&lesson.assignments()).to_line()));
        }
        let shown = services.lessons_of(list, &edits);
        assert_eq!(shown.len(), 2);
        assert_eq!(shown[1].lesson.cost, 90);
        assert!(shown[1].in_database.is_none());
    }
}
