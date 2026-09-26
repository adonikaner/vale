//! **What a loot set holds**, and what an edit to it is.
//!
//! ## Not a tool of its own
//!
//! A loot set has no place in the world and no list of its own worth
//! browsing: `creature_loot_template` has 4,074 entries and every one of them
//! is named by a creature. So there is no rail entry. The **Loot** button on a
//! selected creature, a selected game object and an open item opens one
//! window, [`crate::ui::loot::window`], over whatever that thing's loot columns
//! name — and the window follows the selection, as the quest window does.
//!
//! ## One set at a time is read
//!
//! A set is a few rows for most things and a few hundred for a raid boss, so
//! it is read on demand, one `(table, entry)` per query
//! (`vale_mangos::loot::rows_query`), and kept for the session. Every kept
//! set is dropped when an apply lands (`EditSession::loot_writes`), because the
//! apply may have written any of them.
//!
//! ## A loot row is a row of the project's store like any other
//!
//! Its key is `vale_mangos::loot::key`'s five columns. Adding one is a
//! [`Life::Insert`] row carrying every editable column, removing one in the
//! database is a [`Life::Delete`] row, removing one this project added takes
//! the claim back, and a column edit is a value under the row's key.
//! [`Loot::rows_of`] answers the database's rows with the project's over them.
//!
//! **A group is not a column edit.** It is part of the key, so moving a row
//! between groups is a removal and a creation under one gesture — see
//! [`Loot::regroup`], and `vale_mangos::loot`'s module comment for why the
//! key is shaped that way.

use crate::session::EditSession;
use vale_mangos::loot::{self, Entry, Table};
use vale_mangos::row::{Edits, Key, Life, RowEdit};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::HashMap;

/// **One loot set**: a table, and an entry of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Set {
    pub table: &'static str,
    pub entry: u32,
}

impl Set {
    pub fn new(table: &'static str, entry: u32) -> Set {
        Set { table, entry }
    }

    /// What the table is, for its heading and what it is keyed to.
    pub fn table(&self) -> Table {
        loot::table(self.table).unwrap_or(loot::ALL[0])
    }
}

/// **What the window is about**: a thing in the world and the sets its
/// columns name, one tab each.
///
/// Rebuilt by the shell every frame from the selection while the window is
/// open, so it is a description and never a copy: the entries come from the
/// creature's template row with the project's edits over it, and a `loot_id`
/// typed into the template form changes the tab on the next frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The creature, object or item, as its panel labels it.
    pub title: String,
    /// The template the sets hang off, for the sentence under the title.
    pub about: String,
    /// The sets, in the order the tabs are drawn. An entry of 0 is a tab that
    /// says the column names nothing.
    pub sets: Vec<Set>,
}

/// One row as the window draws it: the database's reading with the project's
/// edits over it, and what the project says is to become of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub entry: Entry,
    pub life: Life,
    /// The row as the database has it, for an edit to be cleared when it is
    /// typed back to what the database holds. `None` for a row this project
    /// creates.
    pub in_database: Option<Entry>,
}

/// **What one roll of a group comes to**, as `LootGroup::Roll`
/// (`LootMgr.cpp:1094`) decides it.
///
/// A group drops at most one of its rows per roll. The rows with a chance are
/// tried in turn against one roll of 0..100; if none is hit, one of the rows
/// whose chance is 0 is taken with equal odds. So a 0 in a group means *an
/// equal share of what the stated chances leave*, and a group of nothing but
/// 0s always drops exactly one of them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Odds {
    /// The stated chances summed, quest drops left out, as
    /// `LootGroup::RawTotalChance` sums them.
    pub stated: f32,
    /// How many rows have a chance of 0.
    pub equal: usize,
    /// …and the chance each of those comes to, per roll of the group.
    pub share: f32,
    /// The chance a roll drops nothing: what the stated chances leave when
    /// there is no 0 row to take it.
    pub nothing: f32,
}

/// The odds of one group among a set's rows, rows marked for removal left out.
pub fn odds(rows: &[Shown], group: u32) -> Odds {
    let members = rows
        .iter()
        .filter(|shown| shown.entry.group == group && shown.life != Life::Delete);
    let mut stated = 0.0;
    let mut equal = 0;
    for shown in members {
        match shown.entry.chance == 0.0 {
            true => equal += 1,
            false if !shown.entry.quest => stated += shown.entry.chance,
            false => {}
        }
    }
    let left = (100.0 - stated).max(0.0);
    Odds {
        stated,
        equal,
        share: match equal {
            0 => 0.0,
            n => left / n as f32,
        },
        nothing: match equal {
            0 => left,
            _ => 0.0,
        },
    }
}

/// What the loot window is holding.
#[derive(Resource, Default)]
pub struct Loot {
    /// Whether the window is shown. What it shows is [`Self::window`], which
    /// the shell fills from the selection each frame this is on.
    pub open: bool,
    pub window: Option<Window>,
    /// Which of the window's sets is shown.
    pub tab: usize,
    /// **A reference set followed from a row**, drawn in place of the tab's set
    /// until *back* is pressed. The rows of a boss's set are mostly references
    /// and the references are where the items are.
    pub reference: Option<u32>,
    /// Every set read so far, as the database has it.
    rows: HashMap<Set, Vec<Entry>>,
    /// What those were read at: `EditSession::loot_writes`.
    loaded_for: Option<u64>,
    reading: Option<(Set, Task<Result<Vec<Entry>, String>>)>,
    /// Why the last read answered nothing, when it answered nothing.
    pub trouble: Option<String>,
    /// What is typed into the *add reference* box.
    pub reference_box: String,
    /// Whether the scripted flags have been acted on — see
    /// `crate::server::loot::on_the_command_line`, which waits on this.
    pub scripted_done: bool,
}

impl Loot {
    /// **The set the window shows now**: the followed reference, or the tab's.
    pub fn showing(&self) -> Option<Set> {
        if let Some(reference) = self.reference {
            return Some(Set::new(loot::REFERENCE, reference));
        }
        let window = self.window.as_ref()?;
        window.sets.get(self.tab.min(window.sets.len().saturating_sub(1))).copied()
    }

    /// Whether a set's rows are in hand.
    pub fn is_read(&self, set: Set) -> bool {
        self.rows.contains_key(&set)
    }

    /// Whether a set is being read now.
    pub fn is_reading(&self, set: Set) -> bool {
        self.reading.as_ref().is_some_and(|(wanted, _)| *wanted == set)
    }

    /// **Every row of one set**, the database's with the project's over them,
    /// grouped as the server groups them: group 0 first, then each group in
    /// order, and by item inside a group.
    pub fn rows_of(&self, set: Set, edits: &Edits) -> Vec<Shown> {
        let mut out: Vec<Shown> = self
            .rows
            .get(&set)
            .map(|rows| rows.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|entry| Shown {
                entry: with_edits(entry, set.table, edits),
                life: edits.life(set.table, &entry.key()),
                in_database: Some(entry.clone()),
            })
            .collect();
        for (table, key, row) in edits.rows() {
            if row.life != Life::Insert || table != set.table {
                continue;
            }
            let Some(entry) = entry_of(key, row) else {
                continue;
            };
            if entry.entry != set.entry || out.iter().any(|shown| shown.entry.key() == *key) {
                continue;
            }
            out.push(Shown {
                entry,
                life: Life::Insert,
                in_database: None,
            });
        }
        out.sort_by_key(|shown| (shown.entry.group, shown.entry.item));
        out
    }

    /// **Add an item to a set**: one of it, at full chance, in no group. A row
    /// that is in the database and marked for removal is kept instead; one
    /// already there is left alone.
    pub fn add_item(&mut self, session: &mut EditSession, set: Set, item: u32, now: f64) {
        self.add(session, set, Entry::item(set.entry, item), "Add loot item", now);
    }

    /// …and a reference to another set, at full chance.
    pub fn add_reference(&mut self, session: &mut EditSession, set: Set, reference: u32, now: f64) {
        self.add(session, set, Entry::reference(set.entry, reference), "Add loot reference", now);
    }

    fn add(&mut self, session: &mut EditSession, set: Set, entry: Entry, label: &'static str, now: f64) {
        let key = entry.key();
        let subject = format!("{} {}", set.table, key.text());
        let gesture = crate::session::Gesture {
            label,
            subject: &subject,
            now,
        };
        let in_database = self
            .rows
            .get(&set)
            .is_some_and(|rows| rows.iter().any(|had| had.key() == key));
        match (in_database, session.server_edits.life(set.table, &key)) {
            (true, Life::Delete) => session.set_server_row(set.table, &key, None, Some(gesture)),
            (true, _) => {}
            (false, _) => {
                let mut row = RowEdit {
                    life: Life::Insert,
                    ..RowEdit::default()
                };
                for change in entry.assignments() {
                    row.columns.insert(change.column.to_string(), change.value);
                }
                session.set_server_row(set.table, &key, Some(&row), Some(gesture));
            }
        }
    }

    /// **Remove a row**: a `Delete` claim for one in the database, and the
    /// claim taken back for one this project added.
    pub fn remove(&mut self, session: &mut EditSession, set: Set, shown: &Shown, now: f64) {
        let key = shown.entry.key();
        let subject = format!("{} {}", set.table, key.text());
        let gesture = crate::session::Gesture {
            label: "Remove loot row",
            subject: &subject,
            now,
        };
        match shown.in_database.is_some() {
            true => {
                let row = RowEdit {
                    life: Life::Delete,
                    ..RowEdit::default()
                };
                session.set_server_row(set.table, &key, Some(&row), Some(gesture));
            }
            false => session.set_server_row(set.table, &key, None, Some(gesture)),
        }
    }

    /// **Keep a row that was marked for removal**: the claim is taken back,
    /// and with it any column edit made before the mark.
    pub fn keep(&mut self, session: &mut EditSession, set: Set, shown: &Shown, now: f64) {
        let key = shown.entry.key();
        let subject = format!("{} {}", set.table, key.text());
        session.set_server_row(
            set.table,
            &key,
            None,
            Some(crate::session::Gesture {
                label: "Keep loot row",
                subject: &subject,
                now,
            }),
        );
    }

    /// **Set one editable column of a row.** On a row the database holds the
    /// edit is cleared when it is what the database holds, so typing a value
    /// back leaves no claim; on a row this project creates the column is
    /// written into the creation.
    pub fn set_column(
        &mut self,
        session: &mut EditSession,
        set: Set,
        shown: &Shown,
        column: &'static str,
        value: String,
        now: f64,
    ) {
        let key = shown.entry.key();
        let subject = format!("{} {} {column}", set.table, key.text());
        let gesture = crate::session::Gesture {
            label: "Edit loot row",
            subject: &subject,
            now,
        };
        match &shown.in_database {
            Some(had) => {
                let held = had
                    .assignments()
                    .into_iter()
                    .find(|change| change.column == column)
                    .map(|change| change.value);
                let value = match held.as_ref() == Some(&value) {
                    true => None,
                    false => Some(value),
                };
                session.set_server_edit(set.table, &key, column, value, Some(gesture));
            }
            None => {
                let Some(mut row) = session.server_edits.row(set.table, &key).cloned() else {
                    return;
                };
                row.columns.insert(column.to_string(), value);
                session.set_server_row(set.table, &key, Some(&row), Some(gesture));
            }
        }
    }

    /// **Move a row to another group**, which is part of its key: the row is
    /// removed and created again under the new key with every column as it is
    /// drawn, as one entry on the undo stack. A row this project created is
    /// re-keyed and nothing else.
    pub fn regroup(&mut self, session: &mut EditSession, set: Set, shown: &Shown, group: u32, now: f64) {
        if group == shown.entry.group || group > loot::MAX_GROUP {
            return;
        }
        let moved = Entry {
            group,
            ..shown.entry.clone()
        };
        let (from, to) = (shown.entry.key(), moved.key());
        if session.server_edits.touches(set.table, &to) {
            return;
        }
        let subject = format!("{} {} group", set.table, from.text());
        let gesture = crate::session::Gesture {
            label: "Move loot row",
            subject: &subject,
            now,
        };
        match shown.in_database.is_some() {
            true => {
                let row = RowEdit {
                    life: Life::Delete,
                    ..RowEdit::default()
                };
                session.set_server_row(set.table, &from, Some(&row), Some(gesture));
            }
            false => session.set_server_row(set.table, &from, None, Some(gesture)),
        }
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in moved.assignments() {
            row.columns.insert(change.column.to_string(), change.value);
        }
        session.set_server_row(set.table, &to, Some(&row), Some(gesture));
    }

    /// **Follow a reference row** into the set it names.
    pub fn follow(&mut self, reference: u32) {
        self.reference = Some(reference);
    }

    /// …and come back to the tab's own set.
    pub fn back(&mut self) {
        self.reference = None;
    }

    /// **Open the window**, or shut it. Opening starts on the first tab with
    /// no reference followed, since what it is about is whatever is selected.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.tab = 0;
        self.reference = None;
    }
}

/// A database row with the project's column edits over it.
fn with_edits(entry: &Entry, table: &str, edits: &Edits) -> Entry {
    let key = entry.key();
    let mut out = entry.clone();
    let number = |column: &str| -> Option<f64> { edits.get(table, &key, column)?.trim().parse().ok() };
    if let Some(chance) = number("ChanceOrQuestChance") {
        out.chance = (chance as f32).abs();
        out.quest = chance < 0.0;
    }
    if let Some(min) = number("mincountOrRef") {
        out.min_or_ref = min as i32;
    }
    if let Some(max) = number("maxcount") {
        out.max = max as u32;
    }
    if let Some(condition) = number("condition_id") {
        out.condition = condition as u32;
    }
    out
}

/// A row this project creates, read back out of its claim.
fn entry_of(key: &Key, row: &RowEdit) -> Option<Entry> {
    let mut whole = loot::Row::new();
    for (column, value) in &key.0 {
        whole.insert(column.clone(), Some(value.clone()));
    }
    for (column, value) in &row.columns {
        whole.insert(column.clone(), Some(value.clone()));
    }
    Entry::from_row(&whole)
}

/// **Read the set the window shows**, on a task, when it has not been read —
/// and forget every set when an apply has moved the tables.
fn read_the_rows(
    mut loot: ResMut<Loot>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    if let Some((set, task)) = loot.reading.as_mut() {
        let set = *set;
        if let Some(done) = block_on(future::poll_once(task)) {
            loot.reading = None;
            match done {
                Ok(rows) => {
                    info!("loot: {} {}: {} row(s)", set.table, set.entry, rows.len());
                    loot.rows.insert(set, rows);
                    loot.trouble = None;
                }
                Err(e) => {
                    warn!("loot: {e}");
                    loot.trouble = Some(e);
                }
            }
        }
        return;
    }
    let Some(session) = session else { return };
    if loot.loaded_for != Some(session.loot_writes) {
        loot.loaded_for = Some(session.loot_writes);
        loot.rows.clear();
    }
    if !loot.open {
        return;
    }
    let Some(set) = loot.showing() else { return };
    if set.entry == 0 || loot.is_read(set) || loot.trouble.is_some() {
        return;
    }
    let Some((at, _source)) = settings.resolve() else {
        loot.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = super::creatures::server_patch(&settings);
    loot.reading = Some((
        set,
        crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let rows = db.rows(&loot::rows_query(set.table, set.entry, patch))?;
            Ok(rows.iter().filter_map(Entry::from_row).collect())
        }),
    ));
}

/// **`--loot`, and `--loot-add <item>`**: the window opened, and a row added to
/// the set it shows, with nobody at the keyboard. The second is the gesture a
/// scripted run cannot make, and the one every check downstream of it needs.
/// It waits for the window to be about something — `--spawn`, `--object` or
/// `--item` fires when its own table has been read — and for the set to be
/// read.
fn on_the_command_line(
    args: Res<crate::Args>,
    mut loot: ResMut<Loot>,
    mut session: Option<ResMut<EditSession>>,
    time: Res<Time>,
    mut opened: Local<bool>,
) {
    if !*opened {
        *opened = true;
        loot.open = args.loot_window;
    }
    if loot.scripted_done {
        return;
    }
    let Some(item) = args.loot_add else {
        loot.scripted_done = true;
        return;
    };
    let Some(set) = loot.showing() else { return };
    if set.entry == 0 {
        warn!("--loot-add {item}: the selection names no loot set");
        loot.scripted_done = true;
        return;
    }
    if !loot.is_read(set) {
        if loot.trouble.is_some() {
            loot.scripted_done = true;
        }
        return;
    }
    let Some(session) = session.as_mut() else { return };
    loot.add_item(session, set, item, time.elapsed_secs_f64());
    info!(
        "--loot-add {item}: added to {} {} — {} row(s) shown",
        set.table,
        set.entry,
        loot.rows_of(set, &session.server_edits).len()
    );
    loot.scripted_done = true;
}

pub struct LootToolPlugin;

impl Plugin for LootToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Loot>()
            .add_systems(Update, (read_the_rows, on_the_command_line).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(set: Set, rows: &[Entry]) -> Loot {
        let mut loot = Loot::default();
        loot.rows.insert(set, rows.to_vec());
        loot
    }

    /// The database's rows come back with the project's edits over them, the
    /// project's own creations beside them, and each marked with its life.
    #[test]
    fn a_set_is_drawn_with_the_projects_claims_over_it() {
        let set = Set::new(loot::CREATURE, 68);
        let loot = held(set, &[Entry::item(68, 2589), Entry::item(68, 25)]);
        let mut edits = Edits::default();
        edits.set(loot::CREATURE, &Entry::item(68, 2589).key(), "ChanceOrQuestChance", Some("-40".into()));
        edits.set_life(loot::CREATURE, &Entry::item(68, 25).key(), Life::Delete);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in loot::new_item(68, 929) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(loot::CREATURE, &Entry::item(68, 929).key(), Some(&row.to_line()));
        // A creation in another set is not this set's.
        edits.set_row_line(loot::CREATURE, &Entry::item(69, 929).key(), Some(&row.to_line()));

        let shown = loot.rows_of(set, &edits);
        assert_eq!(shown.len(), 3);
        let by_item = |item: u32| shown.iter().find(|shown| shown.entry.item == item).expect("shown");
        assert_eq!(by_item(2589).entry.chance, 40.0);
        assert!(by_item(2589).entry.quest);
        assert_eq!(by_item(2589).life, Life::Update);
        assert_eq!(by_item(25).life, Life::Delete);
        assert_eq!(by_item(929).life, Life::Insert);
        assert!(by_item(929).in_database.is_none());
    }

    /// **A group's 0s share what the stated chances leave**, and a group with
    /// no 0 drops nothing on the rest — `LootGroup::Roll`'s two branches. A
    /// quest drop is left out of the sum, and a row marked for removal out of
    /// both.
    #[test]
    fn a_groups_odds_are_the_servers_roll() {
        let in_group = |item: u32, chance: f32| Shown {
            entry: Entry { group: 1, chance, ..Entry::item(30038, item) },
            life: Life::Update,
            in_database: None,
        };
        let all_equal: Vec<Shown> = (0..57).map(|item| in_group(item, 0.0)).collect();
        let odds_of = odds(&all_equal, 1);
        assert_eq!(odds_of.equal, 57);
        assert!((odds_of.share - 100.0 / 57.0).abs() < 1e-4);
        assert_eq!(odds_of.nothing, 0.0);

        let mut mixed = vec![in_group(1, 40.0), in_group(2, 0.0), in_group(3, 0.0)];
        mixed.push(Shown { life: Life::Delete, ..in_group(4, 0.0) });
        mixed.push(Shown { entry: Entry { quest: true, ..in_group(5, 20.0).entry }, ..in_group(5, 20.0) });
        let odds_of = odds(&mixed, 1);
        assert_eq!((odds_of.stated, odds_of.equal, odds_of.share), (40.0, 2, 30.0));

        let stated_only = vec![in_group(1, 1.0), in_group(2, 2.0)];
        assert_eq!(odds(&stated_only, 1).nothing, 97.0);
        assert_eq!(odds(&stated_only, 2).equal, 0, "another group's rows are not counted");
    }

    /// The window shows the tab's set, or the reference followed from it.
    #[test]
    fn the_window_shows_the_tab_or_the_followed_reference() {
        let mut loot = Loot {
            open: true,
            window: Some(Window {
                title: "Kobold Vermin (6)".into(),
                about: String::new(),
                sets: vec![Set::new(loot::CREATURE, 6), Set::new(loot::PICKPOCKETING, 6), Set::new(loot::SKINNING, 0)],
            }),
            ..Loot::default()
        };
        assert_eq!(loot.showing(), Some(Set::new(loot::CREATURE, 6)));
        loot.tab = 1;
        assert_eq!(loot.showing(), Some(Set::new(loot::PICKPOCKETING, 6)));
        loot.follow(30001);
        assert_eq!(loot.showing(), Some(Set::new(loot::REFERENCE, 30001)));
        loot.back();
        loot.tab = 9;
        assert_eq!(loot.showing(), Some(Set::new(loot::SKINNING, 0)), "a tab past the end is the last");
    }
}
