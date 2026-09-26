//! What a creature does, and what an edit to that is: its `creature_ai_events`
//! rows, the `creature_spells` list its template names, and the scripts
//! either of them runs.
//!
//! ## Not a tool of its own
//!
//! An event, a spell list and a script have no place in the world and are
//! named by a creature, so there is no rail entry. The Events and Spells
//! buttons on a selected creature open two windows
//! ([`crate::ui::behaviour`]) that follow the selection as the loot window
//! does, and a script is opened from an event's action or a slot's script
//! into a third.
//!
//! ## What is read
//!
//! One creature's events, one list and one script at a time, on demand, and
//! kept for the session until an apply lands (`EditSession::behaviour_writes`).
//! The highest id of a script table is read once, so a new script can be
//! numbered above it.
//!
//! ## Two stores
//!
//! An event and a list are keyed rows of the project's store, on
//! [`crate::tools::loot`]'s terms: a created row carries every column, a
//! removed one is a [`Life::Delete`] claim, and a column edit is a value under
//! the row's key. A script has no key of its own and is the session's second
//! store (`EditSession::server_scripts`): the whole script is set on every
//! edit, and the undo stack folds the edits made close together into one
//! entry. `vale_mangos::scripts`' module comment is where that was decided.

use crate::session::EditSession;
use vale_mangos::creaturespells::{self, List};
use vale_mangos::eventai::{self, Event};
use vale_mangos::row::{Edits, Key, Life, RowEdit};
use vale_mangos::scripts::{self, Script, ScriptRow};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::HashMap;

/// The creature the windows are about, as the shell describes it each frame
/// from the selection with the project's edits over it.
#[derive(Debug, Clone, PartialEq)]
pub struct About {
    pub entry: u32,
    pub label: String,
    /// The template row's key, for the two columns the windows offer to set
    /// on it: `ai_name` and `spell_list_id`.
    pub template_key: Key,
    pub spell_list_id: u32,
    pub ai_name: String,
}

/// One event as a window draws it: the database's reading with the project's
/// edits over it, and what the project says is to become of it.
#[derive(Debug, Clone, PartialEq)]
pub struct ShownEvent {
    pub event: Event,
    pub life: Life,
    /// The row as the database has it, for an edit to be cleared when it is
    /// typed back to what the database holds. `None` for a row this project
    /// creates.
    pub in_database: Option<Event>,
}

/// One spell list as a window draws it, on [`ShownEvent`]'s terms.
#[derive(Debug, Clone, PartialEq)]
pub struct ShownList {
    pub list: List,
    pub life: Life,
    pub in_database: Option<List>,
}

/// What is being read.
enum Reading {
    Events(u32, Task<Result<Vec<Event>, String>>),
    List(u32, Task<Result<Option<List>, String>>),
    Script(&'static str, u32, Task<Result<Vec<ScriptRow>, String>>),
    MaxId(&'static str, Task<Result<u32, String>>),
}

/// What the three windows are holding.
#[derive(Resource, Default)]
pub struct Behaviour {
    pub about: Option<About>,
    pub events_open: bool,
    pub spells_open: bool,
    pub script_open: bool,
    /// The script the third window shows, as its table and id.
    pub script: Option<(&'static str, u32)>,
    /// Every creature's events read so far, as the database has them.
    events: HashMap<u32, Vec<Event>>,
    /// Every list read so far; `None` for an entry the table does not hold.
    lists: HashMap<u32, Option<List>>,
    /// Every script read so far, as the database has it.
    scripts: HashMap<(&'static str, u32), Vec<ScriptRow>>,
    /// The highest id each table holds, read once.
    max_ids: HashMap<&'static str, u32>,
    reading: Option<Reading>,
    /// What those were read at: `EditSession::behaviour_writes`.
    loaded_for: Option<u64>,
    /// Why the last read answered nothing, when it answered nothing.
    pub trouble: Option<String>,
    /// Whether the scripted flags have been acted on; see
    /// `crate::server::behaviour::on_the_command_line`, which waits on this.
    pub scripted_done: bool,
}

impl Behaviour {
    // --- events ---------------------------------------------------------

    /// Whether a creature's events are in hand.
    pub fn events_read(&self, creature: u32) -> bool {
        self.events.contains_key(&creature)
    }

    /// Every event of one creature, the database's with the project's over
    /// them, in id order.
    pub fn events_of(&self, creature: u32, edits: &Edits) -> Vec<ShownEvent> {
        let mut out: Vec<ShownEvent> = self
            .events
            .get(&creature)
            .map(|rows| rows.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|event| ShownEvent {
                event: event_with_edits(event, edits),
                life: edits.life(eventai::TABLE, &event.key()),
                in_database: Some(event.clone()),
            })
            .collect();
        for (table, key, row) in edits.rows() {
            if row.life != Life::Insert || table != eventai::TABLE {
                continue;
            }
            let Some(event) = event_of(key, row) else { continue };
            if event.creature_id != creature || out.iter().any(|shown| shown.event.id == event.id) {
                continue;
            }
            out.push(ShownEvent { event, life: Life::Insert, in_database: None });
        }
        out.sort_by_key(|shown| shown.event.id);
        out
    }

    /// Add an event to a creature: a repeating combat timer with no script,
    /// at the next free id. Answers the id.
    pub fn add_event(&mut self, session: &mut EditSession, creature: u32, now: f64) -> u32 {
        let taken: Vec<u32> = self
            .events_of(creature, &session.server_edits)
            .iter()
            .map(|shown| shown.event.id)
            .collect();
        let event = Event::new(eventai::next_id(creature, &taken), creature);
        let key = event.key();
        let subject = format!("{} {}", eventai::TABLE, key.text());
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in event.assignments() {
            row.columns.insert(change.column.to_string(), change.value);
        }
        session.set_server_row(
            eventai::TABLE,
            &key,
            Some(&row),
            Some(crate::session::Gesture { label: "Add event", subject: &subject, now }),
        );
        event.id
    }

    /// Remove an event: a `Delete` claim for one in the database, and the
    /// claim taken back for one this project added.
    pub fn remove_event(&mut self, session: &mut EditSession, shown: &ShownEvent, now: f64) {
        remove_row(session, eventai::TABLE, &shown.event.key(), shown.in_database.is_some(), "Remove event", now);
    }

    /// Keep an event that was marked for removal.
    pub fn keep_event(&mut self, session: &mut EditSession, shown: &ShownEvent, now: f64) {
        keep_row(session, eventai::TABLE, &shown.event.key(), "Keep event", now);
    }

    /// Set one column of an event, on [`crate::tools::loot::Loot::set_column`]'s
    /// terms: cleared when it is what the database holds, written into the
    /// creation for a row this project creates.
    pub fn set_event(
        &mut self,
        session: &mut EditSession,
        shown: &ShownEvent,
        column: &'static str,
        value: String,
        now: f64,
    ) {
        let held = shown.in_database.as_ref().map(|had| had.get(column));
        set_column(session, eventai::TABLE, &shown.event.key(), column, value, held, "Edit event", now);
    }

    // --- spell lists ----------------------------------------------------

    /// Whether a list has been asked for.
    pub fn list_read(&self, entry: u32) -> bool {
        self.lists.contains_key(&entry)
    }

    /// One list, the database's with the project's over it, or `None` when
    /// neither has it.
    pub fn list_of(&self, entry: u32, edits: &Edits) -> Option<ShownList> {
        let key = creaturespells::key(entry);
        if let Some(Some(list)) = self.lists.get(&entry) {
            return Some(ShownList {
                list: list_with_edits(list, edits),
                life: edits.life(creaturespells::TABLE, &key),
                in_database: Some(list.clone()),
            });
        }
        let row = edits.row(creaturespells::TABLE, &key)?;
        if row.life != Life::Insert {
            return None;
        }
        let list = list_of(&key, row)?;
        Some(ShownList { list, life: Life::Insert, in_database: None })
    }

    /// Create a list at an entry, with nothing in it.
    pub fn create_list(&mut self, session: &mut EditSession, entry: u32, name: &str, now: f64) {
        let list = List::new(entry, name);
        let key = list.key();
        let subject = format!("{} {}", creaturespells::TABLE, key.text());
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in list.assignments() {
            row.columns.insert(change.column.to_string(), change.value);
        }
        session.set_server_row(
            creaturespells::TABLE,
            &key,
            Some(&row),
            Some(crate::session::Gesture { label: "Create spell list", subject: &subject, now }),
        );
    }

    /// Set one column of a list.
    pub fn set_list(
        &mut self,
        session: &mut EditSession,
        shown: &ShownList,
        column: &'static str,
        value: String,
        now: f64,
    ) {
        let held = shown.in_database.as_ref().map(|had| match column {
            "name" => vale_mangos::sql::text(&had.name),
            _ => had.get(column),
        });
        set_column(session, creaturespells::TABLE, &shown.list.key(), column, value, held, "Edit spell list", now);
    }

    /// The next entry a new list may take: one above the highest the table
    /// holds, once that has been read.
    pub fn next_list_entry(&self) -> Option<u32> {
        self.max_ids.get(creaturespells::TABLE).map(|max| max + 1)
    }

    // --- scripts --------------------------------------------------------

    /// Open the script window on one script.
    pub fn open_script(&mut self, table: &'static str, id: u32) {
        self.script = Some((table, id));
        self.script_open = true;
    }

    /// Whether a script's rows are in hand.
    pub fn script_read(&self, table: &'static str, id: u32) -> bool {
        self.scripts.contains_key(&(table, id))
    }

    /// A script as the window draws it: the project's reading over the
    /// database's. `None` while the database's is not in hand and the project
    /// says nothing.
    pub fn script_of(&self, table: &'static str, id: u32, session: &EditSession) -> Option<Script> {
        if let Some(script) = session.server_scripts.get(table, id) {
            return Some(script);
        }
        let rows = self.scripts.get(&(table, id))?;
        Some(Script { table, id, rows: rows.clone() })
    }

    /// Whether the database holds rows of a script, once read.
    pub fn script_in_database(&self, table: &'static str, id: u32) -> bool {
        self.scripts.get(&(table, id)).is_some_and(|rows| !rows.is_empty())
    }

    /// Set what the project says a script is. A script equal to the
    /// database's takes the claim off.
    pub fn set_script(&mut self, session: &mut EditSession, script: &Script, now: f64) {
        let subject = format!("{} {}", script.table, script.id);
        let gesture = crate::session::Gesture { label: "Edit script", subject: &subject, now };
        let same = self
            .scripts
            .get(&(script.table, script.id))
            .is_some_and(|rows| *rows == script.sorted());
        match same {
            true => session.set_server_script(script.table, script.id, None, Some(gesture)),
            false => session.set_server_script(script.table, script.id, Some(script), Some(gesture)),
        }
    }

    /// The next id a new script of a table may take: one above the highest
    /// the table holds, once that has been read.
    pub fn next_script_id(&self, table: &'static str) -> Option<u32> {
        self.max_ids.get(table).map(|max| max + 1)
    }

    /// Forget every read, which the next frame reads again.
    pub fn forget(&mut self) {
        self.events.clear();
        self.lists.clear();
        self.scripts.clear();
        self.max_ids.clear();
        self.trouble = None;
    }
}

/// A `Delete` claim on a row in the database, or the claim taken back on one
/// this project added.
fn remove_row(session: &mut EditSession, table: &'static str, key: &Key, in_database: bool, label: &'static str, now: f64) {
    let subject = format!("{table} {}", key.text());
    let gesture = crate::session::Gesture { label, subject: &subject, now };
    match in_database {
        true => {
            let row = RowEdit { life: Life::Delete, ..RowEdit::default() };
            session.set_server_row(table, key, Some(&row), Some(gesture));
        }
        false => session.set_server_row(table, key, None, Some(gesture)),
    }
}

/// The removal mark taken off, and with it any column edit made before it.
fn keep_row(session: &mut EditSession, table: &'static str, key: &Key, label: &'static str, now: f64) {
    let subject = format!("{table} {}", key.text());
    session.set_server_row(table, key, None, Some(crate::session::Gesture { label, subject: &subject, now }));
}

/// One column set on a row of either table: cleared when it is what the
/// database holds, written into the creation for a created row.
#[allow(clippy::too_many_arguments)]
fn set_column(
    session: &mut EditSession,
    table: &'static str,
    key: &Key,
    column: &'static str,
    value: String,
    in_database: Option<String>,
    label: &'static str,
    now: f64,
) {
    let subject = format!("{table} {} {column}", key.text());
    let gesture = crate::session::Gesture { label, subject: &subject, now };
    match in_database {
        Some(held) => {
            let value = match held == value {
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

/// A database event with the project's column edits over it.
fn event_with_edits(event: &Event, edits: &Edits) -> Event {
    let key = event.key();
    let mut out = event.clone();
    for column in eventai::COLUMNS.iter() {
        if let Some(value) = edits.get(eventai::TABLE, &key, column.name) {
            out.set(column.name, value);
        }
    }
    out
}

/// An event this project creates, read back out of its claim.
fn event_of(key: &Key, row: &RowEdit) -> Option<Event> {
    let mut whole = eventai::Row::new();
    for (column, value) in &key.0 {
        whole.insert(column.clone(), Some(value.clone()));
    }
    for (column, value) in &row.columns {
        whole.insert(column.clone(), Some(unquoted(column, value)));
    }
    Event::from_row(&whole)
}

/// A database list with the project's column edits over it.
fn list_with_edits(list: &List, edits: &Edits) -> List {
    let key = list.key();
    let mut out = list.clone();
    for column in creaturespells::COLUMNS.iter() {
        if let Some(value) = edits.get(creaturespells::TABLE, &key, column.name) {
            out.set(column.name, value);
        }
    }
    out
}

/// A list this project creates, read back out of its claim.
fn list_of(key: &Key, row: &RowEdit) -> Option<List> {
    let mut whole = creaturespells::Row::new();
    for (column, value) in &key.0 {
        whole.insert(column.clone(), Some(value.clone()));
    }
    for (column, value) in &row.columns {
        whole.insert(column.clone(), Some(unquoted(column, value)));
    }
    List::from_row(&whole)
}

/// The store holds a text column as a quoted literal and a row from the
/// database holds the text; the two text columns are read back the second
/// way.
fn unquoted(column: &str, value: &str) -> String {
    match column {
        "comment" | "name" => value
            .strip_prefix('\'')
            .and_then(|rest| rest.strip_suffix('\''))
            .map(|inner| inner.replace("\\'", "'").replace("\\\\", "\\"))
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

/// Read what the open windows show and do not have, one query at a time, and
/// forget every read when an apply has moved the tables.
fn read_the_rows(
    mut behaviour: ResMut<Behaviour>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    if let Some(reading) = behaviour.reading.as_mut() {
        let done = match reading {
            Reading::Events(creature, task) => {
                let creature = *creature;
                block_on(future::poll_once(task)).map(|done| match done {
                    Ok(rows) => {
                        info!("behaviour: creature {creature}: {} event(s)", rows.len());
                        behaviour.events.insert(creature, rows);
                        Ok(())
                    }
                    Err(e) => Err(e),
                })
            }
            Reading::List(entry, task) => {
                let entry = *entry;
                block_on(future::poll_once(task)).map(|done| match done {
                    Ok(list) => {
                        behaviour.lists.insert(entry, list);
                        Ok(())
                    }
                    Err(e) => Err(e),
                })
            }
            Reading::Script(table, id, task) => {
                let (table, id) = (*table, *id);
                block_on(future::poll_once(task)).map(|done| match done {
                    Ok(rows) => {
                        behaviour.scripts.insert((table, id), rows);
                        Ok(())
                    }
                    Err(e) => Err(e),
                })
            }
            Reading::MaxId(table, task) => {
                let table = *table;
                block_on(future::poll_once(task)).map(|done| match done {
                    Ok(max) => {
                        behaviour.max_ids.insert(table, max);
                        Ok(())
                    }
                    Err(e) => Err(e),
                })
            }
        };
        match done {
            None => return,
            Some(Ok(())) => {
                behaviour.reading = None;
                behaviour.trouble = None;
            }
            Some(Err(e)) => {
                warn!("behaviour: {e}");
                behaviour.reading = None;
                behaviour.trouble = Some(e);
            }
        }
    }
    let Some(session) = session else { return };
    if behaviour.loaded_for != Some(session.behaviour_writes) {
        behaviour.loaded_for = Some(session.behaviour_writes);
        behaviour.forget();
    }
    if behaviour.trouble.is_some() {
        return;
    }
    let Some(about) = behaviour.about.clone() else { return };
    let want_events = behaviour.events_open && !behaviour.events_read(about.entry);
    let want_events_max = behaviour.events_open && !behaviour.max_ids.contains_key(scripts::CREATURE_AI);
    let want_list = behaviour.spells_open && about.spell_list_id != 0 && !behaviour.list_read(about.spell_list_id);
    let want_list_max = behaviour.spells_open && !behaviour.max_ids.contains_key(creaturespells::TABLE);
    let want_spell_scripts_max = behaviour.spells_open && !behaviour.max_ids.contains_key(scripts::CREATURE_SPELLS);
    let want_script = behaviour
        .script
        .filter(|_| behaviour.script_open)
        .filter(|(table, id)| !behaviour.script_read(table, *id));
    if !(want_events || want_events_max || want_list || want_list_max || want_spell_scripts_max || want_script.is_some()) {
        return;
    }
    let Some((at, _source)) = settings.resolve() else {
        behaviour.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let reading = if want_events {
        let creature = about.entry;
        Reading::Events(
            creature,
            crate::server::queue::read(async move {
                let mut db = vale_mangos::conn::Db::open(&at)?;
                let rows = db.rows(&eventai::events_query(creature))?;
                Ok(rows.iter().filter_map(Event::from_row).collect())
            }),
        )
    } else if want_list {
        let entry = about.spell_list_id;
        Reading::List(
            entry,
            crate::server::queue::read(async move {
                let mut db = vale_mangos::conn::Db::open(&at)?;
                Ok(db.row(&creaturespells::row_query(entry))?.as_ref().and_then(List::from_row))
            }),
        )
    } else if let Some((table, id)) = want_script {
        Reading::Script(
            table,
            id,
            crate::server::queue::read(async move {
                let mut db = vale_mangos::conn::Db::open(&at)?;
                let rows = db.rows(&scripts::rows_query(table, id))?;
                Ok(rows.iter().filter_map(ScriptRow::from_row).collect())
            }),
        )
    } else {
        let (table, sql) = match (want_events_max, want_list_max) {
            (true, _) => (scripts::CREATURE_AI, scripts::max_id_query(scripts::CREATURE_AI)),
            (_, true) => (creaturespells::TABLE, creaturespells::max_entry_query()),
            _ => (scripts::CREATURE_SPELLS, scripts::max_id_query(scripts::CREATURE_SPELLS)),
        };
        let column = match table == creaturespells::TABLE {
            true => "entry",
            false => "id",
        };
        Reading::MaxId(
            table,
            crate::server::queue::read(async move {
                let mut db = vale_mangos::conn::Db::open(&at)?;
                use vale_mangos::schema::RowValue;
                Ok(db.row(&sql)?.and_then(|row| row.integer(column)).unwrap_or(0).max(0) as u32)
            }),
        )
    };
    behaviour.reading = Some(reading);
}

/// `--events`, `--event-add` and `--spells`: the windows opened, and an event
/// added to the creature's, with nobody at the keyboard. Waits for the
/// creature's events to be read, since the new id is numbered among them.
fn on_the_command_line(
    args: Res<crate::Args>,
    mut behaviour: ResMut<Behaviour>,
    mut session: Option<ResMut<EditSession>>,
    time: Res<Time>,
    mut opened: Local<bool>,
) {
    if !*opened {
        *opened = true;
        behaviour.events_open = args.events_window || args.event_add;
        behaviour.spells_open = args.spells_window;
    }
    if behaviour.scripted_done {
        return;
    }
    if !args.event_add {
        behaviour.scripted_done = true;
        return;
    }
    let Some(about) = behaviour.about.clone() else { return };
    if !behaviour.events_read(about.entry) {
        if behaviour.trouble.is_some() {
            behaviour.scripted_done = true;
        }
        return;
    }
    let Some(session) = session.as_mut() else { return };
    let id = behaviour.add_event(session, about.entry, time.elapsed_secs_f64());
    info!(
        "--event-add: event {id} added to creature {} — {} event(s) shown",
        about.entry,
        behaviour.events_of(about.entry, &session.server_edits).len()
    );
    behaviour.scripted_done = true;
}

pub struct BehaviourToolPlugin;

impl Plugin for BehaviourToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Behaviour>()
            .add_systems(Update, (read_the_rows, on_the_command_line).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The database's events come back with the project's edits over them,
    /// the project's own creations beside them, and each marked with its
    /// life; another creature's creation is not this creature's.
    #[test]
    fn a_creatures_events_are_drawn_with_the_projects_claims_over_them() {
        let mut behaviour = Behaviour::default();
        behaviour.events.insert(68, vec![Event::new(6801, 68), Event::new(6802, 68)]);
        let mut edits = Edits::default();
        edits.set(eventai::TABLE, &eventai::key(6801), "event_chance", Some("50".into()));
        edits.set(eventai::TABLE, &eventai::key(6801), "comment", Some("'Half the time'".into()));
        edits.set_life(eventai::TABLE, &eventai::key(6802), Life::Delete);
        for (id, creature) in [(6803, 68), (6901, 69)] {
            let event = Event::new(id, creature);
            let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
            for change in event.assignments() {
                row.columns.insert(change.column.to_string(), change.value);
            }
            edits.set_row_line(eventai::TABLE, &event.key(), Some(&row.to_line()));
        }
        let shown = behaviour.events_of(68, &edits);
        assert_eq!(shown.iter().map(|s| s.event.id).collect::<Vec<_>>(), vec![6801, 6802, 6803]);
        assert_eq!(shown[0].event.chance, 50);
        assert_eq!(shown[0].event.comment, "Half the time");
        assert_eq!(shown[0].life, Life::Update);
        assert_eq!(shown[1].life, Life::Delete);
        assert_eq!(shown[2].life, Life::Insert);
        assert!(shown[2].in_database.is_none());
    }

    /// A list the project creates is read back out of its claim, slot by
    /// slot, and a list the database holds takes the project's edits.
    #[test]
    fn a_list_is_drawn_from_either_store() {
        let mut behaviour = Behaviour::default();
        let mut edits = Edits::default();
        assert!(behaviour.list_of(680, &edits).is_none());
        let mut list = List::new(680, "Guard");
        list.slots[0].spell = 6660;
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in list.assignments() {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(creaturespells::TABLE, &list.key(), Some(&row.to_line()));
        let shown = behaviour.list_of(680, &edits).expect("the created list");
        assert_eq!(shown.life, Life::Insert);
        assert_eq!(shown.list.name, "Guard");
        assert_eq!(shown.list.slots[0].spell, 6660);

        behaviour.lists.insert(681, Some(List::new(681, "Other")));
        edits.set(creaturespells::TABLE, &creaturespells::key(681), "spellId_2", Some("12169".into()));
        let shown = behaviour.list_of(681, &edits).expect("the database's list");
        assert_eq!(shown.list.slots[1].spell, 12169);
        assert_eq!(shown.life, Life::Update);
        behaviour.lists.insert(682, None);
        assert!(behaviour.list_of(682, &edits).is_none());
    }
}
