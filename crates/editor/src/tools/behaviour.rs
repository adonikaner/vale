//! What a creature does, and what an edit to that is: its `creature_ai_events`
//! rows, the `creature_spells` list its template names, and the scripts
//! either of them runs.
//!
//! ## Why there is no rail entry
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
//! kept for the session until an apply lands (`EditSession::database_writes`).
//! Beside those, in batches: every script the shown events, the shown list
//! and the search results name, so each event can be drawn with its steps;
//! and the `broadcast_text` row of every Talk step in hand, so a step reads
//! as what is said. The highest id of a script table and of `creature_spells`
//! is read once, so a new script or list can be numbered above it. One read
//! runs at a time, in the order [`read_the_rows`] asks.
//!
//! ## Two stores
//!
//! An event, a list and a `broadcast_text` row are keyed rows of the
//! project's store, on
//! [`crate::tools::loot`]'s terms: a created row carries every column, a
//! removed one is a [`Life::Delete`] claim, and a column edit is a value under
//! the row's key. A script has no key of its own and is the session's second
//! store (`EditSession::server_scripts`): the whole script is set on every
//! edit, and the undo stack folds the edits made close together into one
//! entry. `vale_mangos::scripts`' module comment gives the reason for this
//! design.
//!
//! ## Existing rows as a starting point
//!
//! Four choosers draw on what the database already holds. Add an existing
//! event searches every creature's events by comment and copies one to this
//! creature under a new id, either with copies of its scripts under new ids
//! or naming the same scripts. Choose a script searches a script table by
//! comment and writes the chosen id into one column. Choose a list searches
//! `creature_spells` by name and points the template at the chosen list.
//! Choose a text searches `broadcast_text` by what is said and writes the
//! chosen entry into a Talk step. The
//! reference database writes the creature's name at the start of every event
//! and script comment, so a comment search is also a search by creature.

use crate::session::EditSession;
use crate::tools::quests::ColumnTarget;
use vale_mangos::broadcast::{self, Text};
use vale_mangos::creaturespells::{self, List};
use vale_mangos::eventai::{self, Event};
use vale_mangos::row::{Edits, Key, Life, RowEdit};
use vale_mangos::schema::RowValue;
use vale_mangos::scripts::{self, Script, ScriptRow};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::{HashMap, HashSet};

/// How many rows a search answers at most.
pub const SEARCH_LIMIT: usize = 40;

/// How many scripts or texts one batch read asks for at most.
const BATCH: usize = 60;

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

/// One event as a window draws it: the database's row with the project's
/// edits over it, and the project's [`Life`] for the row (update, insert or
/// delete).
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

/// One `broadcast_text` row as a window draws it, on [`ShownEvent`]'s terms.
#[derive(Debug, Clone, PartialEq)]
pub struct ShownText {
    pub text: Text,
    pub life: Life,
    pub in_database: Option<Text>,
}

/// A search typed into a chooser, and what it found.
///
/// The search runs as the text is typed: [`read_the_rows`] sends the typed
/// text when it differs from the last text sent, so at most one search is
/// running and the answer that lands is for the last text sent.
#[derive(Debug, Clone)]
pub struct Search<T> {
    /// What is typed.
    pub typed: String,
    /// The text of the search last sent.
    pub sent: Option<String>,
    /// What came back, with the text it came back for.
    pub found: Option<(String, Vec<T>)>,
    /// Why the last search answered nothing, when it failed.
    pub trouble: Option<String>,
}

impl<T> Default for Search<T> {
    fn default() -> Self {
        Search { typed: String::new(), sent: None, found: None, trouble: None }
    }
}

impl<T> Search<T> {
    /// A search that starts with `typed` in the box.
    pub fn of(typed: &str) -> Self {
        Search { typed: typed.to_string(), ..Search::default() }
    }

    /// The text to search for now, if any: what is typed, when it is three
    /// characters or a number and was not the last text sent.
    fn wanted(&self) -> Option<String> {
        let typed = self.typed.trim();
        let long_enough = typed.chars().count() >= 3 || typed.parse::<u32>().is_ok();
        (long_enough && self.sent.as_deref() != Some(typed)).then(|| typed.to_string())
    }

    /// Whether a search has been sent and its answer has not landed.
    pub fn searching(&self) -> bool {
        self.trouble.is_none()
            && self.sent.is_some()
            && self.found.as_ref().map(|(text, _)| text) != self.sent.as_ref()
    }

    /// What the last answer found, or nothing.
    pub fn hits(&self) -> &[T] {
        self.found.as_ref().map(|(_, hits)| hits.as_slice()).unwrap_or(&[])
    }
}

/// Where a script chosen in the script chooser is written.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptAnswer {
    /// A column of a keyed row: an event's `action1_script` or a slot's
    /// `scriptId_n`.
    Column(ColumnTarget),
    /// One cell of one row of a script, which is written whole.
    Cell {
        table: &'static str,
        id: u32,
        row: usize,
        column: &'static str,
    },
}

/// The chooser that is open, at most one.
#[derive(Debug, Clone)]
pub enum Chooser {
    /// Add an existing event to the creature: every creature's events,
    /// searched by comment, id or creature id.
    Events(Search<Event>),
    /// Choose an existing script of one table for one column: its scripts,
    /// searched by comment or id. The hits are ids; the rows are in
    /// [`Behaviour`]'s script cache.
    Script {
        table: &'static str,
        answer: ScriptAnswer,
        search: Search<u32>,
    },
    /// Choose an existing spell list for the creature: `creature_spells`,
    /// searched by name or entry. The hits are entries; the lists are in the
    /// list cache.
    List(Search<u32>),
    /// Choose an existing `broadcast_text` row for one cell of a Talk step:
    /// the table, searched by what is said or by entry. The hits are entries;
    /// the rows are in the text cache.
    Text {
        answer: ScriptAnswer,
        search: Search<u32>,
    },
}

/// How a copied event's scripts are carried over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyScripts {
    /// Each script is copied under a new id, so an edit to the copy changes
    /// nothing else.
    Copy,
    /// The copy names the same scripts, so an edit to one of them changes
    /// every event that runs it.
    Share,
}

/// What is being read.
enum Reading {
    Events(u32, Task<Result<Vec<Event>, String>>),
    List(u32, Task<Result<Option<List>, String>>),
    Script(&'static str, u32, Task<Result<Vec<ScriptRow>, String>>),
    MaxId(&'static str, Task<Result<u32, String>>),
    Scripts(&'static str, Vec<u32>, Task<Result<Vec<(u32, ScriptRow)>, String>>),
    Texts(Vec<u32>, Task<Result<Vec<Text>, String>>),
    Users(u32, Task<Result<Vec<(u32, String)>, String>>),
    Search(String, Task<Result<Found, String>>),
}

/// What a chooser's search found.
enum Found {
    Events(Vec<Event>),
    Scripts(&'static str, Vec<(u32, ScriptRow)>),
    Lists(Vec<List>),
    Texts(Vec<Text>),
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
    /// The chooser open over the windows, if any.
    pub chooser: Option<Chooser>,
    /// The events whose form is unfolded in the events window, by id.
    pub unfolded_events: HashSet<u32>,
    /// The steps whose form is unfolded in the script window, by index in
    /// the script's step order. Cleared when another script is opened.
    pub unfolded_steps: HashSet<usize>,
    /// The slots whose form is unfolded in the spells window, 0-based.
    pub unfolded_slots: HashSet<usize>,
    /// Every creature's events read so far, as the database has them.
    events: HashMap<u32, Vec<Event>>,
    /// Every list read so far; `None` for an entry the table does not hold.
    lists: HashMap<u32, Option<List>>,
    /// Every script read so far, as the database has it; no rows for an id
    /// the table does not hold.
    scripts: HashMap<(&'static str, u32), Vec<ScriptRow>>,
    /// Every `broadcast_text` row read so far, by entry; `None` for an entry
    /// the table does not hold, so a missing one is asked for once.
    texts: HashMap<u32, Option<Text>>,
    /// The creature templates that name each list read so far, by entry and name.
    users: HashMap<u32, Vec<(u32, String)>>,
    /// The highest id each table holds, read once.
    max_ids: HashMap<&'static str, u32>,
    reading: Option<Reading>,
    /// The `EditSession::database_writes` value the caches above were read
    /// at.
    loaded_for: Option<u64>,
    /// Why the last read answered nothing, when it answered nothing.
    pub trouble: Option<String>,
    /// Whether the scripted flags have been acted on; see
    /// `crate::server::behaviour::on_the_command_line`, which waits on this.
    pub scripted_done: bool,
    /// The texts another window shows, which this tool reads with its own:
    /// the gossip window's lines and option labels. Written by that window
    /// each frame.
    pub other_texts: Vec<u32>,
    /// Whether another window makes new texts and gossip scripts (the gossip
    /// window), so the two tables' highest ids are read to number them.
    pub numbering_texts: bool,
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
        self.add_event_of(session, creature, 0, now)
    }

    /// Add an event of one trigger type to a creature, with no script, at the
    /// next free id. See `vale_mangos::eventai::Event::of_type` for what each
    /// type starts as. Answers the id.
    pub fn add_event_of(&mut self, session: &mut EditSession, creature: u32, event_type: u32, now: f64) -> u32 {
        let id = self.next_event_id(creature, &session.server_edits);
        let event = Event::of_type(id, creature, event_type);
        create_event(session, &event, "Add event", now);
        self.unfolded_events.insert(id);
        id
    }

    /// Copy an event, from this creature or any other, to `creature` under
    /// the next free id. With [`CopyScripts::Copy`] every script it runs is
    /// copied under a new id first; that needs the scripts in hand, and an
    /// `Err` says so while they are read. Answers the new event's id.
    pub fn copy_event(
        &mut self,
        session: &mut EditSession,
        creature: u32,
        label: &str,
        from: &Event,
        scripts: CopyScripts,
        now: f64,
    ) -> Result<u32, String> {
        let id = self.next_event_id(creature, &session.server_edits);
        let mut copy = from.copied_for(id, creature, label);
        if scripts == CopyScripts::Copy {
            let mut copied: Vec<Script> = Vec::new();
            for (slot, script) in from.scripts.iter().enumerate() {
                if *script == 0 {
                    continue;
                }
                let Some(rows) = self.script_of(scripts::CREATURE_AI, *script, session) else {
                    return Err(format!("reading creature_ai_scripts {script}\u{2026}"));
                };
                let preferred = (slot == 0).then_some(id);
                let new_id = self
                    .new_script_id(scripts::CREATURE_AI, preferred, &session.server_scripts, &copied)
                    .ok_or_else(|| "reading creature_ai_scripts' highest id\u{2026}".to_string())?;
                let mut rows = rows.sorted();
                for row in rows.iter_mut() {
                    row.comments = eventai::renamed(&row.comments, label);
                }
                copied.push(Script { table: scripts::CREATURE_AI, id: new_id, rows });
                copy.scripts[slot] = new_id;
            }
            for script in &copied {
                self.set_script(session, script, now);
            }
        }
        create_event(session, &copy, "Copy event", now);
        self.unfolded_events.insert(id);
        Ok(id)
    }

    /// The next id an event of `creature` may take, on the reference
    /// database's numbering, among the events shown for it.
    fn next_event_id(&self, creature: u32, edits: &Edits) -> u32 {
        let taken: Vec<u32> = self.events_of(creature, edits).iter().map(|shown| shown.event.id).collect();
        eventai::next_id(creature, &taken)
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

    /// How many creature templates name a list, once read.
    pub fn users_of(&self, entry: u32, edits: &Edits) -> Option<u32> {
        let from_database = self.users.get(&entry)?;
        Some(crate::server::fresh::naming(from_database, edits, "spell_list_id", entry).len() as u32)
    }

    /// Create a list at an entry, with nothing in it.
    pub fn create_list(&mut self, session: &mut EditSession, entry: u32, name: &str, now: f64) {
        self.create_list_as(session, &List::new(entry, name), "Create spell list", now);
    }

    /// Create a list as given: a new list, or a copy of one.
    pub fn create_list_as(&mut self, session: &mut EditSession, list: &List, label: &'static str, now: f64) {
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
            Some(crate::session::Gesture { label, subject: &subject, now }),
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
    /// holds and the highest this project creates, once the table's has been
    /// read.
    pub fn next_list_entry(&self, session: &EditSession) -> Option<u32> {
        let database = *self.max_ids.get(creaturespells::TABLE)?;
        let created = session
            .server_edits
            .rows()
            .filter(|(table, _, row)| *table == creaturespells::TABLE && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .unwrap_or(0) as u32;
        Some(database.max(created) + 1)
    }

    // --- scripts --------------------------------------------------------

    /// Open the script window on one script.
    pub fn open_script(&mut self, table: &'static str, id: u32) {
        if self.script != Some((table, id)) {
            self.unfolded_steps.clear();
        }
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
    /// the table holds and the highest this project claims, once the table's
    /// has been read.
    pub fn next_script_id(&self, table: &'static str, session: &EditSession) -> Option<u32> {
        self.new_script_id(table, None, &session.server_scripts, &[])
    }

    /// The id a new script takes: `preferred` when it is above every id the
    /// table holds and nothing claims it, which is the reference database's
    /// convention of numbering an event's script as the event; otherwise the
    /// next free id. `claimed` is the project's script store; `pending` are
    /// scripts about to be written in the same gesture, whose ids are taken
    /// too.
    fn new_script_id(
        &self,
        table: &'static str,
        preferred: Option<u32>,
        claimed: &scripts::Scripts,
        pending: &[Script],
    ) -> Option<u32> {
        let database = *self.max_ids.get(table)?;
        let claimed = claimed
            .iter()
            .chain(pending.iter().cloned())
            .filter(|script| script.table == table)
            .map(|script| script.id);
        let taken: HashSet<u32> = claimed.collect();
        if let Some(preferred) = preferred {
            if preferred > database && !taken.contains(&preferred) {
                return Some(preferred);
            }
        }
        let highest = taken.iter().copied().max().unwrap_or(0).max(database);
        Some(highest + 1)
    }

    // --- texts ----------------------------------------------------------

    /// Whether a `broadcast_text` row has been asked for.
    pub fn text_read(&self, entry: u32) -> bool {
        self.texts.contains_key(&entry)
    }

    /// One text, the database's with the project's over it, or `None` when
    /// neither has it.
    pub fn text_of(&self, entry: u32, edits: &Edits) -> Option<ShownText> {
        let key = broadcast::key(entry);
        if let Some(Some(text)) = self.texts.get(&entry) {
            let mut shown = text.clone();
            for column in broadcast::COLUMNS.iter() {
                if let Some(value) = edits.get(broadcast::TABLE, &key, column.name) {
                    shown.set(column.name, value);
                }
            }
            return Some(ShownText { text: shown, life: edits.life(broadcast::TABLE, &key), in_database: Some(text.clone()) });
        }
        let row = edits.row(broadcast::TABLE, &key)?;
        if row.life != Life::Insert {
            return None;
        }
        let mut whole = broadcast::Row::new();
        for (column, value) in &key.0 {
            whole.insert(column.clone(), Some(value.clone()));
        }
        for (column, value) in &row.columns {
            whole.insert(column.clone(), Some(unquoted(column, value)));
        }
        let text = Text::from_row(&whole)?;
        Some(ShownText { text, life: Life::Insert, in_database: None })
    }

    /// What a text says, with the project's edits, once read.
    pub fn said(&self, entry: u32, edits: &Edits) -> Option<String> {
        self.text_of(entry, edits).map(|shown| shown.text.said().to_string())
    }

    /// Create a text at the next free entry, saying `said`. Answers the entry,
    /// or `None` while the table's highest entry is read.
    pub fn create_text(&mut self, session: &mut EditSession, said: &str, chat_type: u32, now: f64) -> Option<u32> {
        let entry = self.next_text_entry(session)?;
        let text = Text { chat_type, ..Text::new(entry, said) };
        let key = text.key();
        let subject = format!("{} {}", broadcast::TABLE, key.text());
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in text.assignments() {
            row.columns.insert(change.column.to_string(), change.value);
        }
        session.set_server_row(
            broadcast::TABLE,
            &key,
            Some(&row),
            Some(crate::session::Gesture { label: "Create text", subject: &subject, now }),
        );
        Some(entry)
    }

    /// Set one column of a text. The two text columns take the text itself
    /// and are stored as SQL literals.
    pub fn set_text(&mut self, session: &mut EditSession, shown: &ShownText, column: &'static str, value: String, now: f64) {
        let literal = |text: String| match matches!(column, "male_text" | "female_text") {
            true => vale_mangos::sql::text(&text),
            false => text,
        };
        let held = shown.in_database.as_ref().map(|had| literal(had.get(column)));
        set_column(session, broadcast::TABLE, &shown.text.key(), column, literal(value), held, "Edit text", now);
    }

    /// Remove a text: a `Delete` claim for one in the database, and the claim
    /// taken back for one this project added.
    pub fn remove_text(&mut self, session: &mut EditSession, shown: &ShownText, now: f64) {
        remove_row(session, broadcast::TABLE, &shown.text.key(), shown.in_database.is_some(), "Remove text", now);
    }

    /// The next entry a new text may take: one above the highest the table
    /// holds and the highest this project creates, once the table's has been
    /// read.
    pub fn next_text_entry(&self, session: &EditSession) -> Option<u32> {
        let database = *self.max_ids.get(broadcast::TABLE)?;
        let created = session
            .server_edits
            .rows()
            .filter(|(table, _, row)| *table == broadcast::TABLE && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .unwrap_or(0) as u32;
        Some(database.max(created) + 1)
    }

    /// A list the cache holds, as the database has it.
    pub fn cached_list(&self, entry: u32) -> Option<&List> {
        self.lists.get(&entry).and_then(Option::as_ref)
    }

    /// Forget every read, which the next frame reads again.
    pub fn forget(&mut self) {
        self.events.clear();
        self.lists.clear();
        self.scripts.clear();
        self.users.clear();
        self.texts.clear();
        self.max_ids.clear();
        self.trouble = None;
    }

    /// The scripts the open windows and chooser draw and the cache does not
    /// hold, as one table's ids.
    fn scripts_wanted(&self, session: &EditSession) -> Option<(&'static str, Vec<u32>)> {
        let mut wanted: Vec<(&'static str, u32)> = Vec::new();
        if let (true, Some(about)) = (self.events_open, self.about.as_ref()) {
            for shown in self.events_of(about.entry, &session.server_edits) {
                wanted.extend(shown.event.scripts.iter().map(|id| (scripts::CREATURE_AI, *id)));
            }
        }
        if let Some(Chooser::Events(search)) = &self.chooser {
            for event in search.hits() {
                wanted.extend(event.scripts.iter().map(|id| (scripts::CREATURE_AI, *id)));
            }
        }
        if let (true, Some(about)) = (self.spells_open, self.about.as_ref()) {
            if let Some(shown) = self.list_of(about.spell_list_id, &session.server_edits) {
                wanted.extend(shown.list.slots.iter().map(|slot| (scripts::CREATURE_SPELLS, slot.script)));
            }
        }
        let table = wanted
            .iter()
            .find(|(table, id)| *id != 0 && !self.script_read(table, *id))
            .map(|(table, _)| *table)?;
        let mut ids: Vec<u32> = wanted
            .into_iter()
            .filter(|(of, id)| *of == table && *id != 0 && !self.script_read(table, *id))
            .map(|(_, id)| id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.truncate(BATCH);
        Some((table, ids))
    }

    /// The texts of every Talk step in hand, read or claimed, that have not
    /// been asked for.
    fn texts_wanted(&self, session: &EditSession) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .scripts
            .values()
            .flatten()
            .chain(session.server_scripts.iter().flat_map(|script| script.rows).collect::<Vec<_>>().iter())
            .flat_map(scripts::texts_of)
            .chain(self.other_texts.iter().copied())
            .filter(|id| *id != 0 && !self.texts.contains_key(id))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.truncate(BATCH);
        ids
    }

    /// The tables whose highest id the open windows and chooser number new
    /// rows above, which have not been read.
    fn max_wanted(&self) -> Option<&'static str> {
        let mut wanted: Vec<&'static str> = Vec::new();
        if self.events_open || matches!(self.chooser, Some(Chooser::Events(_))) {
            wanted.push(scripts::CREATURE_AI);
        }
        if self.spells_open {
            wanted.extend([creaturespells::TABLE, scripts::CREATURE_SPELLS]);
        }
        if self.script_open {
            wanted.extend([scripts::GENERIC, broadcast::TABLE]);
        }
        if self.numbering_texts {
            wanted.extend([broadcast::TABLE, scripts::GOSSIP]);
        }
        wanted.into_iter().find(|table| !self.max_ids.contains_key(table))
    }
}

/// Write a created event into the project's store.
fn create_event(session: &mut EditSession, event: &Event, label: &'static str, now: f64) {
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
        Some(crate::session::Gesture { label, subject: &subject, now }),
    );
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

/// Set one column on a keyed row: the edit is cleared when the value is what
/// the database holds, and written into the creation for a created row.
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

/// The store holds a text column as a quoted SQL literal; a row read from the
/// database holds the plain text. This turns a stored value of a text column
/// (`comment`, `name`, `male_text`, `female_text`) back into plain text and
/// returns any other column's value unchanged.
fn unquoted(column: &str, value: &str) -> String {
    match column {
        "comment" | "name" | "male_text" | "female_text" => value
            .strip_prefix('\'')
            .and_then(|rest| rest.strip_suffix('\''))
            .map(|inner| inner.replace("\\'", "'").replace("\\\\", "\\"))
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

/// Rows of several scripts, each with the id it is under.
fn rows_with_ids(rows: &[vale_mangos::schema::Row]) -> Vec<(u32, ScriptRow)> {
    rows.iter()
        .filter_map(|row| Some((row.integer("id")? as u32, ScriptRow::from_row(row)?)))
        .collect()
}

/// Put rows read with their ids into the cache, one script per id. An id
/// in `asked` with no rows is a script the table does not hold.
fn cache_scripts(behaviour: &mut Behaviour, table: &'static str, asked: &[u32], rows: Vec<(u32, ScriptRow)>) {
    for id in asked {
        behaviour.scripts.insert((table, *id), Vec::new());
    }
    for (id, row) in rows {
        behaviour.scripts.entry((table, id)).or_default().push(row);
    }
}

/// Take what the running read answered, if it has. `true` while it is still
/// running.
fn land(behaviour: &mut Behaviour) -> bool {
    let Some(mut reading) = behaviour.reading.take() else {
        return false;
    };
    // `None`: the read is still running. `Some(Ok(()))`: the answer was
    // stored. A search's failure also gives `Some(Ok(()))`, because
    // `land_search` shows it in the chooser rather than in every window.
    // `Some(Err(_))`: the read failed.
    let done: Option<Result<(), String>> = match &mut reading {
        Reading::Events(creature, task) => block_on(future::poll_once(task)).map(|done| {
            done.map(|rows| {
                info!("behaviour: creature {creature}: {} event(s)", rows.len());
                behaviour.events.insert(*creature, rows);
            })
        }),
        Reading::List(entry, task) => block_on(future::poll_once(task)).map(|done| {
            done.map(|list| {
                behaviour.lists.insert(*entry, list);
            })
        }),
        Reading::Script(table, id, task) => block_on(future::poll_once(task)).map(|done| {
            done.map(|rows| {
                behaviour.scripts.insert((*table, *id), rows);
            })
        }),
        Reading::MaxId(table, task) => block_on(future::poll_once(task)).map(|done| {
            done.map(|max| {
                behaviour.max_ids.insert(*table, max);
            })
        }),
        Reading::Scripts(table, ids, task) => {
            let (table, ids) = (*table, ids.clone());
            block_on(future::poll_once(task)).map(|done| done.map(|rows| cache_scripts(behaviour, table, &ids, rows)))
        }
        Reading::Texts(ids, task) => {
            let ids = ids.clone();
            block_on(future::poll_once(task)).map(|done| {
                done.map(|texts| {
                    for entry in ids {
                        behaviour.texts.insert(entry, None);
                    }
                    for text in texts {
                        behaviour.texts.insert(text.entry, Some(text));
                    }
                })
            })
        }
        Reading::Users(entry, task) => block_on(future::poll_once(task)).map(|done| {
            done.map(|users| {
                behaviour.users.insert(*entry, users);
            })
        }),
        Reading::Search(text, task) => {
            let text = text.clone();
            match block_on(future::poll_once(task)) {
                None => None,
                Some(answer) => {
                    land_search(behaviour, text, answer);
                    Some(Ok(()))
                }
            }
        }
    };
    match done {
        None => {
            behaviour.reading = Some(reading);
            true
        }
        Some(Ok(())) => {
            behaviour.trouble = None;
            false
        }
        Some(Err(e)) => {
            warn!("behaviour: {e}");
            behaviour.trouble = Some(e);
            false
        }
    }
}

/// A search's answer, stored in the cache and in the chooser it was for. A
/// chooser closed or replaced since the search was sent takes nothing.
fn land_search(behaviour: &mut Behaviour, text: String, answer: Result<Found, String>) {
    let answer = match answer {
        Ok(found) => found,
        Err(e) => {
            warn!("behaviour: search {text:?}: {e}");
            match behaviour.chooser.as_mut() {
                Some(Chooser::Events(search)) => search.trouble = Some(e),
                Some(Chooser::Script { search, .. }) => search.trouble = Some(e),
                Some(Chooser::List(search)) => search.trouble = Some(e),
                Some(Chooser::Text { search, .. }) => search.trouble = Some(e),
                None => {}
            }
            return;
        }
    };
    match (answer, behaviour.chooser.as_mut()) {
        (Found::Events(events), Some(Chooser::Events(search))) => {
            search.found = Some((text, events));
        }
        (Found::Scripts(table, rows), Some(Chooser::Script { table: open, .. })) if table == *open => {
            let mut ids: Vec<u32> = rows.iter().map(|(id, _)| *id).collect();
            ids.dedup();
            let mut by_id: HashMap<u32, Vec<ScriptRow>> = HashMap::new();
            for (id, row) in rows {
                by_id.entry(id).or_default().push(row);
            }
            for (id, rows) in by_id {
                behaviour.scripts.insert((table, id), rows);
            }
            if let Some(Chooser::Script { search, .. }) = behaviour.chooser.as_mut() {
                search.found = Some((text, ids));
            }
        }
        (Found::Lists(lists), Some(Chooser::List(_))) => {
            let entries: Vec<u32> = lists.iter().map(|list| list.entry).collect();
            for list in lists {
                behaviour.lists.insert(list.entry, Some(list));
            }
            if let Some(Chooser::List(search)) = behaviour.chooser.as_mut() {
                search.found = Some((text, entries));
            }
        }
        (Found::Texts(texts), Some(Chooser::Text { .. })) => {
            let entries: Vec<u32> = texts.iter().map(|found| found.entry).collect();
            for found in texts {
                behaviour.texts.insert(found.entry, Some(found));
            }
            if let Some(Chooser::Text { search, .. }) = behaviour.chooser.as_mut() {
                search.found = Some((text, entries));
            }
        }
        _ => {}
    }
}

/// The search the open chooser wants sent, marked as sent.
fn search_wanted(behaviour: &mut Behaviour) -> Option<(String, String, Option<&'static str>)> {
    match behaviour.chooser.as_mut()? {
        Chooser::Events(search) => {
            let text = search.wanted()?;
            search.sent = Some(text.clone());
            search.trouble = None;
            Some((eventai::search_query(&text, SEARCH_LIMIT), text, None))
        }
        Chooser::Script { table, search, .. } => {
            let text = search.wanted()?;
            search.sent = Some(text.clone());
            search.trouble = None;
            Some((scripts::search_query(table, &text, SEARCH_LIMIT), text, Some(*table)))
        }
        Chooser::List(search) => {
            let text = search.wanted()?;
            search.sent = Some(text.clone());
            search.trouble = None;
            Some((creaturespells::search_query(&text, SEARCH_LIMIT), text, Some(creaturespells::TABLE)))
        }
        Chooser::Text { search, .. } => {
            let text = search.wanted()?;
            search.sent = Some(text.clone());
            search.trouble = None;
            Some((broadcast::search_query(&text, SEARCH_LIMIT), text, Some(broadcast::TABLE)))
        }
    }
}

/// Read what the open windows and chooser show and do not have, one query at
/// a time, and forget every read when an apply has moved the tables.
///
/// The order is what a window needs first: the creature's events, its list
/// and the open script, which the windows cannot draw without; the highest
/// ids and a list's users, which buttons wait on; a chooser's search; and
/// last the scripts and texts that fill in the steps.
fn read_the_rows(
    mut behaviour: ResMut<Behaviour>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    if land(&mut behaviour) {
        return;
    }
    let Some(session) = session else { return };
    if behaviour.loaded_for != Some(session.database_writes) {
        behaviour.loaded_for = Some(session.database_writes);
        behaviour.forget();
    }
    if behaviour.trouble.is_some() {
        return;
    }
    let about = behaviour.about.clone();
    let want_events = about
        .as_ref()
        .filter(|about| behaviour.events_open && !behaviour.events_read(about.entry))
        .map(|about| about.entry);
    let want_list = about
        .as_ref()
        .filter(|about| behaviour.spells_open && about.spell_list_id != 0 && !behaviour.list_read(about.spell_list_id))
        .map(|about| about.spell_list_id);
    let want_users = about
        .as_ref()
        .filter(|about| behaviour.spells_open && about.spell_list_id != 0)
        .map(|about| about.spell_list_id)
        .filter(|entry| behaviour.cached_list(*entry).is_some() && !behaviour.users.contains_key(entry));
    let want_script = behaviour
        .script
        .filter(|_| behaviour.script_open)
        .filter(|(table, id)| !behaviour.script_read(table, *id));
    let want_max = behaviour.max_wanted();
    let has_search = behaviour
        .chooser
        .as_ref()
        .is_some_and(|chooser| match chooser {
            Chooser::Events(search) => search.wanted().is_some(),
            Chooser::Script { search, .. } => search.wanted().is_some(),
            Chooser::List(search) => search.wanted().is_some(),
            Chooser::Text { search, .. } => search.wanted().is_some(),
        });
    let want_scripts = behaviour.scripts_wanted(&session);
    let want_texts = behaviour.texts_wanted(&session);
    let wanted = want_events.is_some()
        || want_list.is_some()
        || want_script.is_some()
        || want_max.is_some()
        || want_users.is_some()
        || has_search
        || want_scripts.is_some()
        || !want_texts.is_empty();
    if !wanted {
        return;
    }
    let Some((at, _source)) = settings.resolve() else {
        behaviour.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let open = move || vale_mangos::conn::Db::open(&at);
    let reading = if let Some(creature) = want_events {
        let open = open.clone();
        Reading::Events(
            creature,
            crate::server::queue::read(async move {
                let rows = open()?.rows(&eventai::events_query(creature))?;
                Ok(rows.iter().filter_map(Event::from_row).collect())
            }),
        )
    } else if let Some(entry) = want_list {
        let open = open.clone();
        Reading::List(
            entry,
            crate::server::queue::read(async move {
                Ok(open()?.row(&creaturespells::row_query(entry))?.as_ref().and_then(List::from_row))
            }),
        )
    } else if let Some((table, id)) = want_script {
        let open = open.clone();
        Reading::Script(
            table,
            id,
            crate::server::queue::read(async move {
                let rows = open()?.rows(&scripts::rows_query(table, id))?;
                Ok(rows.iter().filter_map(ScriptRow::from_row).collect())
            }),
        )
    } else if let Some(table) = want_max {
        let (sql, column) = match table {
            creaturespells::TABLE => (creaturespells::max_entry_query(), "entry"),
            broadcast::TABLE => (broadcast::max_entry_query(), "entry"),
            _ => (scripts::max_id_query(table), "id"),
        };
        let open = open.clone();
        Reading::MaxId(
            table,
            crate::server::queue::read(async move {
                Ok(open()?.row(&sql)?.and_then(|row| row.integer(column)).unwrap_or(0).max(0) as u32)
            }),
        )
    } else if let Some(entry) = want_users {
        let open = open.clone();
        Reading::Users(
            entry,
            crate::server::queue::read(async move {
                let sql = creaturespells::users_query(entry);
                Ok(open()?
                    .rows(&sql)?
                    .iter()
                    .filter_map(|row| Some((row.integer("entry")?.max(0) as u32, String::new())))
                    .collect())
            }),
        )
    } else if let Some((sql, text, table)) = search_wanted(&mut behaviour) {
        let open = open.clone();
        Reading::Search(
            text,
            crate::server::queue::read(async move {
                let rows = open()?.rows(&sql)?;
                Ok(match table {
                    None => Found::Events(rows.iter().filter_map(Event::from_row).collect()),
                    Some(creaturespells::TABLE) => Found::Lists(rows.iter().filter_map(List::from_row).collect()),
                    Some(broadcast::TABLE) => Found::Texts(rows.iter().filter_map(Text::from_row).collect()),
                    Some(table) => Found::Scripts(table, rows_with_ids(&rows)),
                })
            }),
        )
    } else if let Some((table, ids)) = want_scripts {
        let Some(sql) = scripts::rows_of_query(table, &ids) else { return };
        let open = open.clone();
        Reading::Scripts(
            table,
            ids,
            crate::server::queue::read(async move { Ok(rows_with_ids(&open()?.rows(&sql)?)) }),
        )
    } else {
        let Some(sql) = broadcast::rows_query(&want_texts) else { return };
        Reading::Texts(
            want_texts,
            crate::server::queue::read(async move {
                Ok(open()?.rows(&sql)?.iter().filter_map(Text::from_row).collect())
            }),
        )
    };
    behaviour.reading = Some(reading);
}

/// `--events`, `--event-add`, `--spells`, `--script` and `--find-event` open
/// the windows and the chooser and add an event to the creature without user
/// input. `--event-add` waits for the creature's events to be read, since the
/// new id is numbered among them.
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
        if let Some(text) = &args.find_event {
            behaviour.chooser = Some(Chooser::Events(Search::of(text)));
        }
        if let Some((table, id)) = args.script_window {
            behaviour.open_script(table, id);
            behaviour.unfolded_steps.extend(0..16);
        }
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

    /// A new script takes the event's own id when that is above every id the
    /// table holds and unclaimed, and otherwise one above the highest id held
    /// or claimed, counting the scripts about to be written with it.
    #[test]
    fn a_new_script_id_is_never_one_already_taken() {
        let mut behaviour = Behaviour::default();
        let mut claimed = scripts::Scripts::default();
        assert_eq!(behaviour.new_script_id(scripts::CREATURE_AI, None, &claimed, &[]), None);
        behaviour.max_ids.insert(scripts::CREATURE_AI, 1_000);
        assert_eq!(behaviour.new_script_id(scripts::CREATURE_AI, Some(500), &claimed, &[]), Some(1_001));
        assert_eq!(behaviour.new_script_id(scripts::CREATURE_AI, Some(2_001), &claimed, &[]), Some(2_001));
        let pending = [Script::empty(scripts::CREATURE_AI, 2_001)];
        assert_eq!(behaviour.new_script_id(scripts::CREATURE_AI, Some(2_001), &claimed, &pending), Some(2_002));
        assert_eq!(behaviour.new_script_id(scripts::CREATURE_AI, None, &claimed, &pending), Some(2_002));
        claimed.set(&Script::empty(scripts::CREATURE_AI, 5_000));
        assert_eq!(behaviour.new_script_id(scripts::CREATURE_AI, None, &claimed, &[]), Some(5_001));
        assert_eq!(behaviour.new_script_id(scripts::GENERIC, None, &claimed, &[]), None);
    }

    /// A search is sent once per text, for three characters or a number, and
    /// is running until the answer for that text lands.
    #[test]
    fn a_search_is_sent_once_per_text() {
        let mut search: Search<u32> = Search::of("Fl");
        assert_eq!(search.wanted(), None);
        search.typed = "Flee".into();
        assert_eq!(search.wanted().as_deref(), Some("Flee"));
        search.sent = Some("Flee".into());
        assert_eq!(search.wanted(), None);
        assert!(search.searching());
        search.found = Some(("Flee".into(), vec![1]));
        assert!(!search.searching());
        assert_eq!(search.hits(), &[1]);
        search.typed = "68".into();
        assert_eq!(search.wanted().as_deref(), Some("68"));
    }
}
