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
//! On `crate::subjects::loot`'s terms: adding one is a [`Life::Insert`] row
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
//! ([`teaching::teaching_spell`]), since that is what the loader's log line asks for.

use crate::server::store::Gesture;
use crate::session::EditSession;
use vale_assets::tables::talent::Talents;
use vale_assets::tables::teaching::{self, LineGroup};
use vale_mangos::row::{Assignment, Edits, Key, Life, RowEdit};
use vale_mangos::trainer::{self, Lesson, Taught};
use vale_mangos::vendor::{self, Ware};
use bevy::prelude::*;
use crate::server::fresh::FreshReads;
use std::collections::HashSet;

/// Which of the two windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Vendor,
    Trainer,
}

impl Kind {
    /// The creature's own table and the template table, in the order the
    /// tabs are drawn.
    pub(crate) fn tables(self) -> [&'static str; 2] {
        match self {
            Kind::Vendor => [vendor::VENDOR, vendor::TEMPLATE],
            Kind::Trainer => [trainer::TRAINER, trainer::TEMPLATE],
        }
    }

    /// The `npc_flags` bit the client reads to offer the window at all:
    /// `UNIT_NPC_FLAG_VENDOR` and `UNIT_NPC_FLAG_TRAINER`, from
    /// `UnitDefines.h`.
    pub(crate) fn flag(self) -> (u32, &'static str) {
        match self {
            Kind::Vendor => (0x4, "VENDOR"),
            Kind::Trainer => (0x10, "TRAINER"),
        }
    }

    /// The template column that names the shared list.
    pub(crate) fn template_column(self) -> &'static str {
        match self {
            Kind::Vendor => "vendor_id",
            Kind::Trainer => "trainer_id",
        }
    }
}

/// One list: a table, and an entry of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct List {
    pub(crate) table: &'static str,
    pub(crate) entry: u32,
}

impl List {
    pub(crate) fn new(table: &'static str, entry: u32) -> List {
        List { table, entry }
    }

    pub(crate) fn kind(&self) -> Kind {
        match vale_mangos::row::table_named(vendor::TABLES, self.table) {
            Some(_) => Kind::Vendor,
            None => Kind::Trainer,
        }
    }

    /// Whether this is a template list, shared by every creature that names it.
    pub(crate) fn is_template(&self) -> bool {
        self.table == vendor::TEMPLATE || self.table == trainer::TEMPLATE
    }
}

/// What the windows are about: the selected creature, with the project's
/// edits over its template row. Rebuilt by the shell every frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct About {
    pub(crate) entry: u32,
    pub(crate) label: String,
    /// The template row's key, for the offers that write a template column.
    pub(crate) template_key: Key,
    pub(crate) npc_flags: u32,
    pub(crate) vendor_id: u32,
    pub(crate) trainer_id: u32,
    pub(crate) trainer_type: u32,
    pub(crate) trainer_class: u32,
    pub(crate) trainer_race: u32,
    pub(crate) trainer_spell: u32,
}

impl About {
    /// The windows' subject for the creature in focus, read from `template`,
    /// its template row as read, with this project's edits over it: so a
    /// `vendor_id`, a `trainer_id` or `npc_flags` typed into the template form
    /// changes the window on the next frame. `None` while the row is being
    /// read.
    pub(crate) fn from_focus(
        focus: &crate::subjects::windows::Focus,
        template: Option<&vale_mangos::schema::Row>,
        edits: &Edits,
    ) -> Option<About> {
        let template = template?;
        let column = |name: &str| -> u32 {
            edits
                .shown_integer(vale_mangos::creature::TEMPLATE, &focus.template_key, name, Some(template))
                .map_or(0, |value| value.max(0) as u32)
        };
        Some(About {
            entry: focus.entry,
            label: focus.label.clone(),
            template_key: focus.template_key.clone(),
            npc_flags: column("npc_flags"),
            vendor_id: column("vendor_id"),
            trainer_id: column("trainer_id"),
            trainer_type: column("trainer_type"),
            trainer_class: column("trainer_class"),
            trainer_race: column("trainer_race"),
            trainer_spell: column("trainer_spell"),
        })
    }

    /// The creature's two lists of a kind: its own, then its template's. The
    /// template list's entry is 0 when the column names none.
    pub(crate) fn lists(&self, kind: Kind) -> [List; 2] {
        let [own, template] = kind.tables();
        let shared = match kind {
            Kind::Vendor => self.vendor_id,
            Kind::Trainer => self.trainer_id,
        };
        [List::new(own, self.entry), List::new(template, shared)]
    }

    /// Whether `npc_flags` carries the kind's bit.
    pub(crate) fn has_flag(&self, kind: Kind) -> bool {
        self.npc_flags & kind.flag().0 != 0
    }
}

/// One vendor row as the window draws it: the database's reading with the
/// project's edits over it, and what the project says is to become of it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShownWare {
    pub(crate) ware: Ware,
    pub(crate) life: Life,
    /// The row as the database has it, for an edit typed back to that value
    /// to be cleared. `None` for a row this project creates.
    pub(crate) in_database: Option<Ware>,
    /// Whether `forbidden_items` leaves the row out at the server's patch.
    pub(crate) forbidden: bool,
}

/// …and one trainer row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShownLesson {
    pub(crate) lesson: Lesson,
    pub(crate) life: Life,
    pub(crate) in_database: Option<Lesson>,
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

/// One list as the database has it, and for a template list the creatures
/// that name it, as entry and name.
struct Held {
    read: Read,
    users: Option<Vec<(u32, String)>>,
}

/// What the two windows are holding.
#[derive(Resource, Default)]
pub(crate) struct Services {
    pub(crate) vendor_open: bool,
    pub(crate) trainer_open: bool,
    /// The selected creature, which the shell writes each frame; `None`
    /// under another tool, or with nothing selected.
    pub(crate) about: Option<About>,
    /// Which list each window shows: 0 the creature's own, 1 the template's.
    /// `None` until a tab is pressed; see [`Self::showing`].
    pub(crate) vendor_tab: Option<usize>,
    pub(crate) trainer_tab: Option<usize>,
    /// Every list read so far, kept until an apply moves
    /// `EditSession::database_writes`.
    lists: FreshReads<List, Held>,
    /// Every spell id that is a rank of a talent, out of `Talent.dbc`; `None`
    /// until the trainer window has opened the table.
    pub(crate) talents: Option<HashSet<u32>>,
    /// Every row of `SkillLine.dbc` in the groups the `reqskill` menu lists;
    /// `None` until the trainer window has opened the table.
    pub(crate) skill_lines: Option<Vec<LineGroup>>,
    /// Whether the scripted flags have been acted on; see
    /// `crate::server::subject::flags::on_the_command_line`, which waits on this.
    pub(crate) scripted_done: bool,
    /// `--vendor-find` or `--trainer-find`: the window whose add-picker is
    /// to be opened, and the text to search, taken by the window the first
    /// time it draws that list.
    pub(crate) find: Option<(Kind, String)>,
}

impl Services {
    /// Whether a window is open.
    pub(crate) fn is_open(&self, kind: Kind) -> bool {
        match kind {
            Kind::Vendor => self.vendor_open,
            Kind::Trainer => self.trainer_open,
        }
    }

    /// Open a window, or shut it. Opening leaves the tab to [`Self::showing`].
    pub(crate) fn toggle(&mut self, kind: Kind) {
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
    pub(crate) fn set_tab(&mut self, kind: Kind, tab: usize) {
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
    pub(crate) fn tab(&self, kind: Kind) -> usize {
        let pressed = match kind {
            Kind::Vendor => self.vendor_tab,
            Kind::Trainer => self.trainer_tab,
        };
        if let Some(tab) = pressed {
            return tab;
        }
        let Some(about) = self.about.as_ref() else { return 0 };
        let [own, template] = about.lists(kind);
        let own_rows = self.wares(own).map(Vec::len).or_else(|| self.lessons(own).map(Vec::len));
        match (own_rows, template.entry) {
            (Some(0), entry) if entry != 0 => 1,
            _ => 0,
        }
    }

    /// The list a window shows now.
    pub(crate) fn showing(&self, kind: Kind) -> Option<List> {
        let about = self.about.as_ref()?;
        Some(about.lists(kind)[self.tab(kind)])
    }

    /// Whether a list's rows are in hand. A list with entry 0 is never read
    /// and counts as read, with no rows.
    pub(crate) fn is_read(&self, list: List) -> bool {
        list.entry == 0 || self.lists.is_read(&list)
    }

    /// Why the last read answered nothing, when it answered nothing.
    pub(crate) fn trouble(&self) -> Option<&String> {
        self.lists.trouble()
    }

    /// A vendor list's rows as the database has them, once read.
    fn wares(&self, list: List) -> Option<&Vec<HeldWare>> {
        match &self.lists.get(&list)?.read {
            Read::Wares(rows) => Some(rows),
            Read::Lessons(_) => None,
        }
    }

    /// A trainer list's rows as the database has them, once read.
    fn lessons(&self, list: List) -> Option<&Vec<Lesson>> {
        match &self.lists.get(&list)?.read {
            Read::Lessons(rows) => Some(rows),
            Read::Wares(_) => None,
        }
    }

    /// The creatures that name a template list, once it has been read: the
    /// database's, with the project's `vendor_id` or `trainer_id` edits and
    /// its created templates over them.
    pub(crate) fn users(&self, list: List, edits: &Edits) -> Option<Vec<(u32, String)>> {
        let from_database = self.lists.get(&list)?.users.as_ref()?;
        Some(crate::server::fresh::naming(from_database, edits, list.kind().template_column(), list.entry))
    }

    /// Every vendor row of one list, the database's with the project's over
    /// them, in the order the server sends them: by slot, then by item.
    pub(crate) fn wares_of(&self, list: List, edits: &Edits) -> Vec<ShownWare> {
        let mut out: Vec<ShownWare> = self
            .wares(list)
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
            let Some(ware) = Ware::from_row(&row.whole(key)) else {
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
    pub(crate) fn lessons_of(&self, list: List, edits: &Edits) -> Vec<ShownLesson> {
        let mut out: Vec<ShownLesson> = self
            .lessons(list)
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
            let Some(lesson) = Lesson::from_row(&row.whole(key)) else {
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
    pub(crate) fn add_item(
        &mut self,
        session: &mut EditSession,
        list: List,
        other: Option<List>,
        item: u32,
        now: f64,
    ) -> Result<String, String> {
        let shown = self.wares_of(list, &session.rows.edits);
        let kept: Vec<&ShownWare> = shown.iter().filter(|s| s.life != Life::Delete).collect();
        if kept.iter().any(|s| s.ware.item == item) {
            return Err(format!("item {item} is already in {} {}", list.table, list.entry));
        }
        let other_rows = match other {
            Some(other) if other.entry != 0 => self.wares_of(other, &session.rows.edits),
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
    pub(crate) fn add_spell(
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
        let shown = self.lessons_of(list, &session.rows.edits);
        if shown.iter().any(|s| s.life != Life::Delete && s.lesson.spell == spell) {
            return Err(format!("spell {spell} is already in {} {}", list.table, list.entry));
        }
        if let Some(other) = other.filter(|other| other.entry != 0 && !list.is_template()) {
            let taught_there = self
                .lessons_of(other, &session.rows.edits)
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
    pub(crate) fn remove(&mut self, session: &mut EditSession, table: &'static str, key: &Key, in_database: bool, now: f64) {
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
    pub(crate) fn keep(&mut self, session: &mut EditSession, table: &'static str, key: &Key, now: f64) {
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
    pub(crate) fn set_column(
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
    pub(crate) fn move_ware(&mut self, session: &mut EditSession, list: List, item: u32, by: i32, now: f64) {
        let mut order: Vec<ShownWare> = self
            .wares_of(list, &session.rows.edits)
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
    let creating = session.rows.edits.row(table, key).is_some_and(|row| row.life == Life::Insert);
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
            let Some(mut row) = session.rows.edits.row(table, key).cloned() else {
                return;
            };
            row.columns.insert(column.to_string(), value);
            session.set_server_row(table, key, Some(&row), Some(gesture));
        }
    }
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
// The reads
// ---------------------------------------------------------------------------

/// Read the lists of every open window that have not been read, one at a
/// time on a task, and forget every list when an apply has moved the tables.
fn read_the_rows(
    mut services: ResMut<Services>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    let Some(session) = session else { return };
    let writes = session.database_writes;
    services.lists.renew(writes);
    match services.lists.poll() {
        Some(Ok(list)) => {
            let count = services.wares(list).map(Vec::len).or_else(|| services.lessons(list).map(Vec::len));
            info!("services: {} {}: {} row(s)", list.table, list.entry, count.unwrap_or(0));
        }
        Some(Err(e)) => warn!("services: {e}"),
        None => {}
    }
    if services.lists.is_busy() || services.trouble().is_some() {
        return;
    }
    let Some(about) = services.about.clone() else { return };
    let wanted = [Kind::Vendor, Kind::Trainer]
        .into_iter()
        .filter(|kind| services.is_open(*kind))
        .flat_map(|kind| about.lists(kind))
        .find(|list| !services.is_read(*list));
    let Some(list) = wanted else { return };
    let patch = crate::server::settings::server_patch(&settings);
    services.lists.want(writes, list, || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
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
                (false, _) => None,
                (true, kind) => {
                    let sql = match kind {
                        Kind::Vendor => vendor::users_query(list.entry),
                        Kind::Trainer => trainer::users_query(list.entry),
                    };
                    let users = db
                        .rows(&sql)?
                        .iter()
                        .filter_map(|row| {
                            use vale_mangos::schema::RowValue;
                            Some((row.integer("entry")? as u32, row.text("name").unwrap_or("").to_string()))
                        })
                        .collect();
                    Some(users)
                }
            };
            Ok(Held { read, users })
        }))
    });
}

/// `--vendor`, `--trainer`, `--vendor-add <item>`, `--trainer-add <spell>`,
/// `--vendor-find <text>` and `--trainer-find <text>`: the windows opened, a
/// row added to the creature's own list, and a window's add-picker opened,
/// with nobody at the keyboard. An add waits for `--spawn`'s creature and for both
/// lists of its kind to be read.
fn on_the_command_line(
    args: Res<crate::args::Args>,
    mut services: ResMut<Services>,
    mut session: Option<ResMut<EditSession>>,
    assets: Option<Res<vale_client::assets::GameAssets>>,
    time: Res<Time>,
    mut opened: Local<bool>,
    mut pending: Local<Option<Vec<(Kind, u32)>>>,
) {
    if !*opened {
        *opened = true;
        services.vendor_open = args.subjects.vendor_window;
        services.trainer_open = args.subjects.trainer_window;
        services.find = args
            .subjects.vendor_find
            .clone()
            .map(|text| (Kind::Vendor, text))
            .or_else(|| args.subjects.trainer_find.clone().map(|text| (Kind::Trainer, text)));
        let mut adds = Vec::new();
        adds.extend(args.subjects.vendor_add.map(|item| (Kind::Vendor, item)));
        adds.extend(args.subjects.trainer_add.map(|spell| (Kind::Trainer, spell)));
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
        if services.trouble().is_some() {
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
                    (Some(talent), Some(tab)) => {
                        Talents::parse(&talent.write(), &tab.write()).map(|t| teaching::talent_spells(&t))
                    }
                    _ => None,
                };
            }
            let talents = services.talents.clone().unwrap_or_default();
            let Some(spells) = session.table("Spell") else { return };
            let (spell, replaced) = teaching::resolve_teaching(spells, value);
            let facts = teaching::spell_facts(spells, spell).map(|f| Taught {
                effect: f.effect,
                teaches: f.teaches,
                level: f.level,
                talent: talents.contains(&spell),
            });
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

pub(crate) struct ServicesToolPlugin;

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
        hold(services, list, Read::Wares(rows));
    }

    fn hold(services: &mut Services, list: List, read: Read) {
        services.lists.insert(list, Held { read, users: None });
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
        hold(&mut services, List::new(trainer::TRAINER, 3044), Read::Lessons(Vec::new()));
        assert_eq!(services.tab(Kind::Trainer), 1);
        services.set_tab(Kind::Trainer, 0);
        assert_eq!(services.tab(Kind::Trainer), 0);
        hold(&mut services, List::new(vendor::VENDOR, 3044), Read::Wares(Vec::new()));
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
        hold(&mut services, list, Read::Lessons(vec![Lesson::new(3044, 1173)]));
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
