//! The client's own tables: which row is open, and what an edit to a field is.
//!
//! ## Why this tool does not use the pointer
//!
//! Every other file in this directory states what a left-drag in the viewport
//! does. This one does not use the pointer: a table has no place in the world,
//! so the workspace replaces the viewport. [`crate::ui::data`] draws the
//! workspace, and the rail's Data group holds the subjects that work this way.
//!
//! The lights and the flight path tables are edited through the same functions
//! and are not in the Data group, because a `Light` row and a `TaxiNodes` row
//! each have a place in the world and are picked there; see
//! [`crate::ui::rail`]. The group is decided by the surface a subject is edited
//! on, not by its file format. [`crate::tools::flightpaths`] writes the flight
//! path tables' fields through [`set_fields`] and [`set_text`] here, and adds
//! and removes their rows through `vale_edit::dbc::taxi`.
//!
//! This file is in `tools/` rather than `ui/` for the same reason `tiles.rs`
//! is: the open table, the selected row and the effect of an edit are editor
//! state, and the panel is a view of that state. `--tool spells` must be able
//! to set this state before anything is drawn.
//!
//! ## One browser for every table
//!
//! [`Browser`] holds a table name; it is not specific to spells. A reference
//! column such as `SpellVisual`, `SpellIcon` or `EffectTriggerSpell` is
//! followed by opening the table it names at the row it names. Following
//! references is what makes this a data editor rather than a row viewer, and
//! the browser supports it for every table because it holds no knowledge of
//! any one table.
//!
//! A table that `vale_assets::tables::schema` describes is drawn as named,
//! grouped, typed fields. A table without a schema is drawn as numbered fields
//! with guessed types, which is what `vale dbc <Table>` prints. That is less
//! useful than a schema, and more useful than refusing to open the table.
//!
//! ## Which workspace reaches which table
//!
//! ```text
//! Spells   the spell chain's four tables, and under them the three skill
//!          tables: SkillLine, SkillLineAbility, SkillRaceClassInfo
//! Sets     ItemSet, as a part of the Items workspace
//! Zones    AreaTable: the zones and the sub-areas inside them
//! Tables   any file of DBFilesClient\, chosen from a list of all of them
//! ```
//!
//! The Tables workspace starts on no table ([`ANY`]): its list is the tables
//! themselves, and choosing one points the browser at it. A table is named
//! by one spelling whatever case it is asked for in
//! (`schema::table_name`), because the archives list their files in lower
//! case and the session holds each open table under its name.
//!
//! ## Row labels come from the rows that reference them
//!
//! A table with a schema is named by its first text column. A table without
//! one is named by a column guessed from the file ([`guess_name_field`]): the
//! first in which the rows hold the offsets of strings.
//!
//! `SpellVisual` row 67 has no name column, and a list of 2,167 bare ids cannot
//! be searched by meaning. The label is built from the tables around the row:
//! that visual fills the precast, cast and impact slots, throws a missile, and
//! is referenced by nine spells named Fireball. [`describe`] builds that label
//! for each table, and the search index is built from the labels, so the query
//! `fireball` finds the visual as well as the spell.
//!
//! ## Reverse references
//!
//! [`Browser::used_by`] lists the rows that reference a given row. It reads an
//! index built once per target table from the reference columns of every open
//! table. The index is keyed by the session's table revision, so an edit that
//! changes a reference is reflected on the next call.
//!
//! ## Adding, copying and removing rows
//!
//! [`add_row`], [`clone_row`] and [`delete_row`] each make one entry on the
//! undo stack, and each new row takes the next id above the table's largest.
//! Two operations used from a form are built on them: [`link_new`] makes a
//! blank row in the table a reference names and points the reference at it,
//! and [`unshare`] copies the row a reference names and points the reference
//! at the copy, so a kit that forty spells share becomes one visual's own.
//! [`clone_chain`] does the same for a whole `SpellVisual` and everything it
//! names, through `vale_edit::dbc::chain`.
//!
//! ## Rows of one table edited from another table's form
//!
//! Some rows exist only for a row of another table and have no name of
//! their own: a `SkillLineAbility` row is a spell's place in a skill line,
//! a `SkillRaceClassInfo` row is who has a skill line, and a teaching spell
//! is a `Spell` row whose only job is to teach another. The form of the
//! row they are about lists and edits them (`crate::ui::data`), and the
//! functions here find and make them: [`abilities_of`], [`teachers_of`],
//! [`race_class_rows_of`], [`add_ability`], [`add_teaching_spell`],
//! [`add_race_class_row`]. Each make is one entry on the undo stack. None
//! of them is required: most spells are in no skill line and have no
//! teaching spell.
//!
//! A row that many rows share (a kit, a cast time, a range, an icon) is
//! not edited this way. It stays a reference with a picker, because an edit
//! made from one row's form would change every row that names it.
//!
//! An item set's item list and an item's `set_id` are one fact stored
//! twice, in a DBC table and in a server row. [`move_between_sets`] is
//! the table half of keeping them in step.
//!
//! ## Zones and sub-areas
//!
//! `AreaTable` is two levels: a zone, and the sub-areas that name it as
//! their parent. A zone's form lists its sub-areas ([`sub_areas_of`]).
//! [`add_zone`] and [`add_sub_area`] make a row with the columns a blank row
//! would get wrong: the map, the parent, and an explore bit no other row
//! holds ([`next_explore_bit`]). A sub-area also starts with its zone's flags,
//! reverb, ambience and music. A row is put on the ground by the Areas tool,
//! which [`Browser::paint_area`] asks the shell to switch to.
//!
//! ## Commands on a row
//!
//! What is done to a row other than editing a field is a [`Command`]:
//! clone, delete, copy the id, and a table's own, such as making a spell's
//! teaching spell. [`commands`] lists a table's and [`command_label`] words
//! one for a row, or says it does not apply to that row. The panel's buttons
//! and its right-click menus read both, so the two cannot differ.
//!
//! ## A row of the spell chain is found and pictured by its models
//!
//! A kit's label names its first model and a visual's the first spell that
//! uses it. The search index also holds every model a kit hangs, by effect
//! name and by file name, and every spell that uses a visual
//! ([`search_words`]), and [`first_model`] is the model a list draws beside
//! the row.
//!
//! The contents of a blank row are the only table-specific rule here: a kit's
//! effect and procedural slots are `-1` for none ([`blank_defaults`]), which is
//! the convention the shipped rows use.

use crate::session::EditSession;
use vale_assets::tables::schema::{self, Kind, Schema};
use vale_client::assets::GameAssets;
use vale_edit::dbc::{cell, chain, Cell, Row};
use bevy::prelude::*;
use std::collections::HashMap;

/// What a row is called in a list and at the head of its form.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowLabel {
    /// The name, or the sentence that stands in for one.
    pub title: String,
    /// A second, quieter line: a rank, a file name, an animation.
    pub sub: String,
    pub id: u32,
}

impl RowLabel {
    /// The label on one line, for a place with room for only one.
    pub fn line(&self) -> String {
        match self.title.is_empty() {
            true => format!("{}", self.id),
            false => format!("{}  [{}]", self.title, self.id),
        }
    }
}

/// One reference to a row: the table, record and field that hold it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    pub table: String,
    pub record: usize,
    pub field: usize,
}

/// A dialog over the form. At most one is open, and `ui::data` draws it once
/// per frame regardless of which field opened it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    /// Choose a row of `points_at` for the reference at `(record, field)` of
    /// `table`. The table is named because the record is not always a row of
    /// the open table: a spell's form edits the `SkillLineAbility` rows that
    /// name the spell.
    Pick {
        table: String,
        record: usize,
        field: usize,
        points_at: &'static str,
    },
    /// Choose a model path from the archives for the text at `(record, field)`.
    Models { record: usize, field: usize },
    /// Choose a row of `table` and open it. No reference is written.
    ///
    /// The counterpart of [`Modal::Pick`] for a subject whose panel is not a
    /// list. Lights are picked in the viewport, so this list exists to find a
    /// light that is not on screen, and choosing a row opens it instead of
    /// writing it into a column.
    Rows { table: &'static str },
    /// Set and clear the bits of the mask at `(record, field)` of `table`.
    ///
    /// This is the third way to edit a column without typing its value.
    /// `Spell.dbc` has twelve mask columns with about 240 named bits between
    /// them, which otherwise have to be entered as hexadecimal numbers. `bits`
    /// is the column's own list: `vale_assets::tables::schema::Kind::Flags`
    /// carries it, and the names come from `vale_assets::tables::spellbits`.
    Bits {
        table: String,
        record: usize,
        field: usize,
        /// The column's name, for the dialog's heading. The schema is not
        /// available where the dialog is drawn.
        column: &'static str,
        bits: &'static [(u32, &'static str, &'static str)],
    },
    /// Choose the visual of the `Spell` row at `record` by another spell,
    /// which is played, and either share that spell's visual or clone it.
    LooksLike { record: usize },
}

/// The folders the model browser can be narrowed to, as path prefixes; the
/// first is everything.
pub const MODEL_FOLDERS: [(&str, &str); 5] = [
    ("All", ""),
    ("Spells", "spells\\"),
    ("Particles", "particles\\"),
    ("World", "world\\"),
    ("Creature", "creature\\"),
];

/// A per-target reverse index, and what it was built against.
struct Backrefs {
    revision: u64,
    open_tables: usize,
    by_id: HashMap<u32, Vec<Use>>,
}

/// What table is open, at which row, and what has been typed into it.
#[derive(Resource, Default)]
pub struct Browser {
    /// The table's bare name, as [`schema::for_table`] spells it.
    pub table: String,
    /// What is in the search box.
    pub query: String,
    /// The rows that match the query, as record indices.
    matches: Vec<usize>,
    /// The table, query and revision `matches` was built for, so an unchanged
    /// query is not searched again on every frame the panel is drawn.
    built: Option<(String, String, u64)>,
    /// Every row's searchable text, lower case, built once per table and
    /// revision. The 22,360 spell names take about a megabyte; without this
    /// cache every keystroke would decode all of them.
    names: HashMap<String, (u64, usize, Vec<String>)>,
    /// The reverse index, per target table. See the module comment.
    backrefs: HashMap<String, Backrefs>,
    /// The open row, as a record index.
    pub open: Option<usize>,
    /// The table and row the list last brought into view, or was clicked on.
    /// The list scrolls to the open row when it is another. See
    /// `crate::ui::theme::list_area`.
    pub revealed: Option<(String, usize)>,
    /// Which view of the open row is drawn; see `crate::ui::data::View`.
    pub view: crate::ui::data::View,
    /// The table and record each follow started from, so the browser can go
    /// back. Following a reference column moves to another table, and this
    /// stack is the way back to the one it came from.
    pub back: Vec<(String, usize)>,
    /// The text of the open row's string fields while it is being typed.
    ///
    /// A `TextEdit` needs to hold partly typed text, and the file cannot hold
    /// it: the string block is append-only, so writing on every keystroke
    /// would append one string per character. The text is kept here and
    /// written to the file when the box loses focus or Enter is pressed; see
    /// [`commit_text`].
    buffers: HashMap<usize, String>,
    /// Which row the buffers belong to.
    buffered: Option<(String, usize, u64)>,
    /// What is typed into a reference picker's search box.
    pub pick_query: String,
    /// The rows that match the picker's query, as record indices of the target
    /// table, and the table, query and revision they were built for; see
    /// [`Self::picks`].
    pick_hits: Vec<usize>,
    pick_built: Option<(String, String, u64)>,
    /// Which page of the hits the picker shows. It is reset to zero when the
    /// hits are rebuilt, because a page number into a changed list may point
    /// past its end.
    pub pick_page: usize,
    /// The model browser's page, and the path it previews on the stage while
    /// it is open; see `crate::lab::Lab::preview`.
    pub model_page: usize,
    pub model_preview: Option<String>,
    /// Whether the picker's box should take the keyboard on the next frame,
    /// which is the frame after it opens.
    pub pick_focus: bool,
    /// The row a picker that plays its rows has selected, by its id. A
    /// press selects a row and plays it, and a second action chooses it;
    /// see `ui::data`.
    pub pick_selected: Option<u32>,
    /// The open dialog, if any; see [`Modal`].
    pub modal: Option<Modal>,
    /// Every `.m2` the archives list, lower case, read once for the model
    /// browser; the text typed into the browser's search box; and the index of
    /// the folder it is narrowed to.
    pub models: Option<Vec<String>>,
    pub model_query: String,
    pub model_folder: usize,
    /// `SoundEntries` ids a panel has asked to play.
    ///
    /// A kit names a `SoundEntries` id, and outside a playtest the only way to
    /// identify the sound is to play it. The panel pushes an id here and
    /// [`audition`] plays it through the client's own mixer on the next frame.
    /// The panel cannot play it directly because it draws inside one system and
    /// the mixer is a separate `SystemParam`.
    pub audition: Vec<u32>,
    /// What a play or stop button beside a sound has asked for, carried out by
    /// [`listen`] on the next frame. See [`Heard`].
    pub listen: Option<Listen>,
    /// The sound [`listen`] is playing, until it ends or is stopped, so the
    /// button that started it can be drawn as a stop button.
    pub hearing: Option<Heard>,
    /// Whether `--row` has been applied. It names a row of a table that is not
    /// yet open on the frame the flag is read, so it is applied when the table
    /// has been opened rather than at startup.
    seeded: bool,
    /// Whether the browser was sent to a table from outside the data tools, on
    /// the frame the rail moved to one; for example, a reference on the item
    /// form followed to `SkillLine`. [`open_tables`] points the browser at the
    /// rail's table when the rail changes, which would move it back to `Spell`
    /// and lose the row. This flag tells it the change came with a
    /// destination.
    pub followed_in: bool,
    /// The table the Tables workspace last showed, which it returns to. Empty
    /// for the list of tables. See [`ANY`].
    pub any: String,
    /// What is typed into the Tables workspace's search over table names.
    pub table_query: String,
    /// Every file of `DBFilesClient\` the archives list, by its one name,
    /// sorted; read once for the Tables workspace's list.
    pub table_names: Option<Vec<String>>,
    /// Each open table's records by id, and the revision it was built at. See
    /// [`Self::record_of`].
    ids: HashMap<String, (u64, HashMap<u32, usize>)>,
    /// Which columns of the open table with no schema hold strings, and the
    /// table and revision that was worked out for. See [`Self::text_fields`].
    text_columns: Option<(String, u64, Vec<bool>)>,
    /// The name column guessed for each table with no schema, kept because
    /// the guess reads up to 64 rows and a label is asked for per row.
    guessed: HashMap<String, Option<usize>>,
    /// A tool a panel asks the shell to switch to, taken by the shell after
    /// everything is drawn: the strip at the head of the Sets list asking
    /// for Items. The panel cannot switch itself, because the tool is
    /// borrowed while it draws.
    pub switch_to: Option<super::Tool>,
    /// An area the form asks the shell to paint with: the shell gives its id
    /// to the Areas tool's brush and switches to that tool. See
    /// [`Command::PaintArea`].
    pub paint_area: Option<u32>,
}

impl Browser {
    /// Point the browser at a table, leaving everything else alone if it is
    /// already there.
    pub fn look_at(&mut self, table: &str) {
        if self.table != table {
            self.table = table.to_string();
            self.open = None;
            self.query.clear();
            self.built = None;
        }
    }

    /// Follow a reference: open `table` at the row holding `id`, and push the
    /// current table and row onto [`Self::back`].
    ///
    /// Returns `false` when the table has no such row. That is a normal value
    /// for a reference column: a `SpellVisual` of 0 means a spell with no
    /// visual, not a dangling reference.
    pub fn follow(&mut self, session: &EditSession, table: &str, id: u32) -> bool {
        let Some(at) = session.table(table).and_then(|open| open.row_of(id)) else {
            return false;
        };
        if let (Some(from), false) = (self.open, self.table.is_empty()) {
            self.back.push((self.table.clone(), from));
        }
        self.table = table.to_string();
        self.open = Some(at);
        self.query.clear();
        self.built = None;
        true
    }

    /// Open a row of the current table by its index, and clear the search so
    /// the row is in the list. Used for a row that was just added or copied.
    pub fn open_row(&mut self, record: usize) {
        self.open = Some(record);
        self.query.clear();
        self.built = None;
        self.forget_buffers();
    }

    /// Close the open row. Used after a removal, because the removed row's
    /// index now names the row after it.
    pub fn close_row(&mut self) {
        self.open = None;
        self.built = None;
        self.forget_buffers();
    }

    /// Return to the table and row the last follow started from.
    pub fn go_back(&mut self) -> bool {
        let Some((table, record)) = self.back.pop() else {
            return false;
        };
        self.table = table;
        self.open = Some(record);
        self.built = None;
        true
    }

    /// The rows matching the query, in table order.
    ///
    /// A query matches in two ways, and a row matching either is included: a
    /// number matches the row with that id, and any text matches a row whose
    /// label contains it. So `133` finds Fireball, and `fireball` finds all
    /// nine ranks of it and the visual they share, because a visual's label
    /// names the spells that use it.
    pub fn matches(&mut self, session: &EditSession) -> &[usize] {
        let asked = (
            self.table.clone(),
            self.query.clone(),
            session.table_revision,
        );
        if self.built.as_ref() == Some(&asked) {
            return &self.matches;
        }
        self.built = Some(asked);
        self.matches.clear();
        let table_name = self.table.clone();
        self.index(session, &table_name);
        let Some(table) = session.table(&table_name) else {
            return &self.matches;
        };
        let query = self.query.trim().to_ascii_lowercase();
        let by_id: Option<u32> = query.parse().ok();
        let names = self.names.get(&table_name).map(|(_, _, rows)| rows);
        for record in 0..table.record_count() {
            let hit = query.is_empty()
                || by_id.is_some_and(|id| table.u32_at(record, 0) == Some(id))
                || names
                    .and_then(|all| all.get(record))
                    .is_some_and(|name| name.contains(&query));
            if hit {
                self.matches.push(record);
            }
        }
        &self.matches
    }

    /// The rows of any open table that the picker's query matches, searched
    /// through the same index the list uses. The result covers the whole
    /// table, not only the first few hundred rows, so a picker can page
    /// through all of it.
    ///
    /// The result is cached until the query, the table or the revision
    /// changes, and the page is reset to the first when it is rebuilt. Returns
    /// `None` for a table that is not open.
    pub fn picks(&mut self, session: &EditSession, table: &str) -> Option<&[usize]> {
        let query = self.pick_query.trim().to_ascii_lowercase();
        let asked = (table.to_string(), query.clone(), session.table_revision);
        if self.pick_built.as_ref() == Some(&asked) {
            return Some(&self.pick_hits);
        }
        let open = session.table(table)?;
        self.index(session, table);
        self.pick_built = Some(asked);
        self.pick_page = 0;
        self.pick_hits.clear();
        let by_id: Option<u32> = query.parse().ok();
        let names = self.names.get(table).map(|(_, _, rows)| rows);
        for record in 0..open.record_count() {
            let hit = query.is_empty()
                || by_id.is_some_and(|id| open.u32_at(record, 0) == Some(id))
                || names
                    .and_then(|all| all.get(record))
                    .is_some_and(|name| name.contains(&query));
            if hit {
                self.pick_hits.push(record);
            }
        }
        Some(&self.pick_hits)
    }

    /// What to call one row in a list, on one line: its title and its id.
    pub fn label(&mut self, session: &EditSession, record: usize) -> String {
        let table = self.table.clone();
        self.describe(session, &table, record).line()
    }

    /// The label of a row of any open table; see the module comment.
    pub fn describe(&mut self, session: &EditSession, table: &str, record: usize) -> RowLabel {
        describe(self, session, table, record)
    }

    /// The record holding `id` in an open table, through an index kept per
    /// table and revision. `DbcFile::row_of` is a scan: asked for the spell of
    /// each of the 5,072 abilities over `Spell`'s 22,360 rows it is 56 million
    /// comparisons for one rebuild of the abilities' labels. Where two records
    /// hold one id the first is answered, as `row_of` answers.
    pub fn record_of(&mut self, session: &EditSession, table: &str, id: u32) -> Option<usize> {
        let open = session.table(table)?;
        let fresh = self
            .ids
            .get(table)
            .is_some_and(|(revision, _)| *revision == session.table_revision);
        if !fresh {
            let mut records: HashMap<u32, usize> = HashMap::new();
            for record in 0..open.record_count() {
                if let Some(id) = open.u32_at(record, 0) {
                    records.entry(id).or_insert(record);
                }
            }
            self.ids
                .insert(table.to_string(), (session.table_revision, records));
        }
        self.ids.get(table)?.1.get(&id).copied()
    }

    /// Which columns of a table with no schema hold strings, worked out once
    /// per revision, since the form asks once per field per frame.
    pub fn text_fields(&mut self, session: &EditSession, table: &str) -> &[bool] {
        let fresh = self.text_columns.as_ref().is_some_and(|(had, revision, _)| {
            had == table && *revision == session.table_revision
        });
        if !fresh {
            let columns = session.table(table).map(text_fields).unwrap_or_default();
            self.text_columns = Some((table.to_string(), session.table_revision, columns));
        }
        self.text_columns
            .as_ref()
            .map(|(_, _, columns)| columns.as_slice())
            .unwrap_or(&[])
    }

    /// The column a table's rows are named by: the schema's first text column,
    /// or the guess for a table with no schema. `None` for a table that is
    /// not open and for one with no such column.
    pub fn name_field(&mut self, session: &EditSession, table: &str) -> Option<usize> {
        if let Some(schema) = schema::for_table(table) {
            return label_field(schema);
        }
        if let Some(known) = self.guessed.get(table) {
            return *known;
        }
        let guess = guess_name_field(session.table(table)?);
        self.guessed.insert(table.to_string(), guess);
        guess
    }

    /// The label of a row by id rather than by record, for a reference column.
    /// Returns `None` when the table is not open or has no such row.
    pub fn describe_id(&mut self, session: &EditSession, table: &str, id: u32) -> Option<RowLabel> {
        let record = self.record_of(session, table, id)?;
        Some(self.describe(session, table, record))
    }

    /// The buffer for one string field of the open row, seeded from the file.
    pub fn buffer(&mut self, session: &EditSession, record: usize, field: usize) -> &mut String {
        // The buffers are also keyed by the table revision, so a table
        // restored by a discard or a project switch does not keep showing text
        // typed against the previous one. A commit also increments the
        // revision, and the buffer is then re-seeded from the file, which holds
        // the committed text.
        let here = Some((self.table.clone(), record, session.table_revision));
        if self.buffered != here {
            self.buffered = here;
            self.buffers.clear();
        }
        self.buffers.entry(field).or_insert_with(|| {
            session
                .table(&self.table)
                .and_then(|table| table.string_at(record, field))
                .unwrap_or_default()
        })
    }

    /// Forget the buffers, for a caller that has just written them or moved on.
    pub fn forget_buffers(&mut self) {
        self.buffers.clear();
        self.buffered = None;
    }

    /// Build a table's search text, once per revision.
    ///
    /// The index is rebuilt when a table is opened as well as when one is
    /// edited, because a visual's label names the spells that use it and the
    /// spell table may be opened a frame after the visual table.
    pub fn index(&mut self, session: &EditSession, table_name: &str) {
        let open_tables = session.tables.len();
        if self
            .names
            .get(table_name)
            .is_some_and(|(revision, tables, _)| {
                *revision == session.table_revision && *tables == open_tables
            })
        {
            return;
        }
        let Some(count) = session.table(table_name).map(|table| table.record_count()) else {
            return;
        };
        let rows = (0..count)
            .map(|record| {
                let label = describe(self, session, table_name, record);
                let words = search_words(self, session, table_name, record);
                format!("{} {}{words}", label.title, label.sub).to_ascii_lowercase()
            })
            .collect();
        self.names.insert(
            table_name.to_string(),
            (session.table_revision, open_tables, rows),
        );
    }

    /// Every reference to a row, over every open table.
    pub fn used_by(&mut self, session: &EditSession, target: &str, id: u32) -> Vec<Use> {
        self.backrefs_for(session, target)
            .get(&id)
            .cloned()
            .unwrap_or_default()
    }

    /// The reverse index for one target table, built if it is stale.
    fn backrefs_for(&mut self, session: &EditSession, target: &str) -> &HashMap<u32, Vec<Use>> {
        let open_tables = session.tables.len();
        let fresh = self.backrefs.get(target).is_some_and(|index| {
            index.revision == session.table_revision && index.open_tables == open_tables
        });
        if !fresh {
            self.backrefs.insert(
                target.to_string(),
                Backrefs {
                    revision: session.table_revision,
                    open_tables,
                    by_id: build_backrefs(session, target),
                },
            );
        }
        &self.backrefs[target].by_id
    }
}

/// Walk every open table's reference columns that point at `target`.
///
/// Only tables with a schema are read, because the schema is what declares a
/// column to be a reference; a table without one contributes no references to
/// this index. Zero and `-1` both mean "no row" and neither is counted as a
/// reference.
fn build_backrefs(session: &EditSession, target: &str) -> HashMap<u32, Vec<Use>> {
    let mut by_id: HashMap<u32, Vec<Use>> = HashMap::new();
    for (name, table) in &session.tables {
        let Some(schema) = schema::for_table(name) else {
            continue;
        };
        let fields = schema.references_to(target);
        if fields.is_empty() {
            continue;
        }
        for record in 0..table.record_count() {
            for &field in &fields {
                let Some(id) = table.u32_at(record, field) else {
                    continue;
                };
                if id == 0 || id == u32::MAX {
                    continue;
                }
                by_id.entry(id).or_default().push(Use {
                    table: name.clone(),
                    record,
                    field,
                });
            }
        }
    }
    by_id
}

/// The five kit slots of a `SpellVisual`, in the order they play, with the
/// word a label uses for each.
const VISUAL_SLOTS: [(usize, &str); 5] = [
    (
        vale_assets::tables::spell::fields::PRECAST_KIT,
        "Precast",
    ),
    (vale_assets::tables::spell::fields::CAST_KIT, "Cast"),
    (vale_assets::tables::spell::fields::IMPACT_KIT, "Impact"),
    (
        vale_assets::tables::spell::fields::CHANNEL_KIT,
        "Channel",
    ),
    (vale_assets::tables::spell::fields::STATE_KIT, "State"),
];

/// The kit's model columns, in the order a label reads them.
const KIT_MODELS: [usize; 9] = [6, 7, 3, 4, 5, 12, 8, 9, 10];

/// The label of one row, with a rule per table; see the module comment.
///
/// This is a free function rather than a method so that [`Browser::index`] can
/// call it while it holds the browser mutably: the reverse index it reads is
/// stored on the same struct.
pub fn describe(
    browser: &mut Browser,
    session: &EditSession,
    table_name: &str,
    record: usize,
) -> RowLabel {
    let Some(table) = session.table(table_name) else {
        return RowLabel::default();
    };
    let id = table.u32_at(record, 0).unwrap_or_default();
    let text = |field: usize| table.string_at(record, field).unwrap_or_default();
    let num = |field: usize| table.u32_at(record, field).unwrap_or_default();
    let float = |field: usize| table.f32_at(record, field).unwrap_or_default();

    let (title, sub) = match table_name {
        "Spell" => (
            text(vale_assets::tables::spellbook::spell_fields::NAME),
            text(vale_assets::tables::spellbook::spell_fields::RANK),
        ),
        "SpellVisual" => {
            let filled: Vec<&str> = VISUAL_SLOTS
                .iter()
                .filter(|(field, _)| {
                    let kit = num(*field);
                    kit != 0 && kit != u32::MAX
                })
                .map(|(_, word)| *word)
                .collect();
            let mut title = match filled.is_empty() {
                true => "empty visual".to_string(),
                false => filled.join(" · "),
            };
            if num(vale_assets::tables::spell::fields::HAS_MISSILE) != 0 {
                title.push_str(" · missile");
            }
            if num(vale_assets::tables::spell::fields::AREA_FLAG) != 0 {
                title.push_str(" · area");
            }
            (title, spells_using(browser, session, "SpellVisual", id))
        }
        "SpellVisualKit" => {
            let names = session.table("SpellVisualEffectName");
            let models: Vec<String> = KIT_MODELS
                .iter()
                .filter_map(|&field| {
                    let effect = num(field);
                    if effect == 0 || effect == u32::MAX {
                        return None;
                    }
                    let name = names
                        .and_then(|names| names.row_of(effect))
                        .and_then(|row| {
                            names?
                                .string_at(row, vale_assets::tables::spell::fields::EFFECT_NAME)
                        })
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| format!("effect {effect}"));
                    Some(name)
                })
                .collect();
            let title = match models.len() {
                0 => "no models".to_string(),
                1 => models[0].clone(),
                n => format!("{} +{}", models[0], n - 1),
            };
            let anim = num(vale_assets::tables::spell::fields::ANIMATION);
            let mut sub = match anim {
                0 | u32::MAX => String::new(),
                anim => session
                    .table("AnimationData")
                    .and_then(|data| data.row_of(anim))
                    .and_then(|row| session.table("AnimationData")?.string_at(row, 1))
                    .unwrap_or_else(|| format!("anim {anim}")),
            };
            // The visual this kit is in, and the spell that visual belongs to:
            // `Fireball · Cast` states which spell and slot the kit serves.
            if let Some(slot) = kit_slot(browser, session, id) {
                if !sub.is_empty() {
                    sub.push_str(" · ");
                }
                sub.push_str(&slot);
            }
            (title, sub)
        }
        "SpellVisualEffectName" => {
            let path = text(vale_assets::tables::spell::fields::EFFECT_MODEL);
            (
                text(vale_assets::tables::spell::fields::EFFECT_NAME),
                basename(&path).to_string(),
            )
        }
        "SpellIcon" => (basename(&text(1)).to_string(), String::new()),
        "AnimationData" => (text(1), String::new()),
        "SoundEntries" => {
            let files = (0..vale_assets::tables::sound::fields::entry::FILE_COUNT)
                .map(|i| text(vale_assets::tables::sound::fields::entry::FILE + i))
                .filter(|file| !file.is_empty())
                .count();
            let first = text(vale_assets::tables::sound::fields::entry::FILE);
            let sub = match files {
                0 => "no files".to_string(),
                1 => first,
                n => format!("{first} +{}", n - 1),
            };
            (
                text(vale_assets::tables::sound::fields::entry::NAME),
                sub,
            )
        }
        // An ambience row has no name, so it is called by its sounds' names:
        // the day one, and the night one under it when it differs.
        "SoundAmbience" => {
            use vale_assets::tables::sound::fields::{ambience, entry};
            let name = |sound: u32| {
                session
                    .table("SoundEntries")
                    .and_then(|sounds| sounds.string_at(sounds.row_of(sound)?, entry::NAME))
                    .filter(|name| !name.is_empty())
            };
            let (day, night) = (num(ambience::SOUND), num(ambience::SOUND + 1));
            let title = name(day).unwrap_or_default();
            let sub = match night != day {
                true => name(night).map(|name| format!("night: {name}")),
                false => None,
            };
            (title, sub.unwrap_or_default())
        }
        "SpellCastTimes" => (
            format!("{} ms", table.i32_at(record, 1).unwrap_or(0)),
            String::new(),
        ),
        "SpellDuration" => {
            let ms = table.i32_at(record, 1).unwrap_or(0);
            let title = match ms {
                -1 => "for ever".to_string(),
                ms if ms % 1000 == 0 => format!("{} s", ms / 1000),
                ms => format!("{ms} ms"),
            };
            (title, String::new())
        }
        "SpellRadius" => (format!("{:.0} yd", float(1)), String::new()),
        "SpellRange" => {
            let (min, max) = (float(1), float(2));
            let span = match (min, max) {
                (_, max) if max <= 0.0 => "self".to_string(),
                (min, max) if min <= 0.0 => format!("{max:.0} yd"),
                (min, max) => format!("{min:.0}–{max:.0} yd"),
            };
            (text(4), span)
        }
        "SpellEffectCameraShakes" => (
            format!("shakes {}, {}, {}", num(1), num(2), num(3)),
            String::new(),
        ),
        "SpellChainEffects" => (basename(&text(7)).to_string(), String::new()),
        // A light is labelled by its map and position. The table has no name
        // column, and a list of 374 bare ids cannot be searched by meaning;
        // `SpellVisual` has the same problem and is labelled the same way.
        "Light" => {
            use vale_assets::tables::light::{light_field as lf, YARDS_PER_UNIT};
            let map = num(lf::MAP);
            // The map's folder name if `Map.dbc` is open, and its id otherwise.
            // `Map.dbc` is not opened here, because `describe` runs for each
            // visible row on each frame and takes the session by shared
            // reference.
            let where_ = session
                .table("Map")
                .and_then(|maps| maps.row_of(map))
                .and_then(|row| session.table("Map")?.string_at(row, 1))
                .filter(|dir| !dir.is_empty())
                .unwrap_or_else(|| format!("map {map}"));
            let end = float(lf::FALLOFF_END) * YARDS_PER_UNIT;
            // A falloff of zero marks the map's default light. It applies
            // everywhere and its position is not read, so the label omits the
            // coordinates.
            if end <= 0.0 {
                (format!("{where_} default"), String::new())
            } else {
                let start = float(lf::FALLOFF_START) * YARDS_PER_UNIT;
                let at = vale_assets::world::adt::placement_to_world([
                    float(lf::INTERNAL_X) * YARDS_PER_UNIT,
                    float(lf::INTERNAL_Y) * YARDS_PER_UNIT,
                    float(lf::INTERNAL_Z) * YARDS_PER_UNIT,
                ]);
                (
                    format!("{where_}  {start:.0}..{end:.0} yd"),
                    format!("{:.0}, {:.0}, {:.0}", at[0], at[1], at[2]),
                )
            }
        }
        // A params row has no name of its own, so it is labelled by the lights
        // that use it. This is the light chain's equivalent of `spells_using`.
        "LightParams" => {
            let used = lights_using(browser, session, id);
            let title = match used.is_empty() {
                true => "unused".to_string(),
                false => used,
            };
            let sky = match num(1) {
                0 => "",
                _ => " · highlight sky",
            };
            (title, format!("glow {:.2}{sky}", float(4)))
        }
        // A band is labelled from its id, because the id's position is the
        // only thing that says which of the eighteen bands it is. The row holds
        // a count, times and colours, and nothing that names the band.
        "LightIntBand" | "LightFloatBand" => {
            use vale_assets::tables::light::{
                float_band_of, int_band_of, FLOAT_BAND_NAMES, INT_BAND_NAMES,
            };
            let ints = table_name == "LightIntBand";
            let named = match ints {
                true => int_band_of(id).map(|(p, b)| (p, INT_BAND_NAMES[b as usize])),
                false => float_band_of(id).map(|(p, b)| (p, FLOAT_BAND_NAMES[b as usize])),
            };
            match named {
                Some((params, band)) => (
                    band.to_string(),
                    format!("params {params} · {} keys", num(1)),
                ),
                None => (String::new(), format!("{} keys", num(1))),
            }
        }
        "LightSkybox" => (basename(&text(1)).to_string(), String::new()),
        // A category has no name column. `SpellCategory.dbc` is 166 records of
        // two fields, an id and a flags word, so [`label_field`] finds nothing
        // and the list would read `row 1`, `row 2`, `row 4`. `vale dbc
        // SpellCategory` confirms the file has no name to read.
        //
        // A category is defined by the set of spells that share it: vmangos
        // keys its shared cooldowns off this column (`sSpellsByCategory`). So
        // the label is the spells using it, the same rule `SpellVisual` uses.
        "SpellCategory" => {
            let used = spells_using(browser, session, "SpellCategory", id);
            let title = match used.is_empty() {
                // "unused" is stated rather than left blank. Of 166 categories
                // far fewer are in use, so an unused category is a normal
                // result and not a failed lookup.
                true => "unused".to_string(),
                false => used,
            };
            let sub = match num(1) {
                0 => String::new(),
                flags => format!("flags 0x{flags:08X}"),
            };
            (title, sub)
        }
        // `Map` has no schema but is still labelled. It is referenced by
        // `Light.Map` and read for a light's own label, and without this arm
        // every light's map field would show `none`. Field 1 is the folder
        // under `World\Maps\`, the column `tables::dbc::map_directories`
        // reads, and field 4 is the first of the eight locale columns holding
        // the map's display name.
        "Map" => (text(1), text(4)),
        // A skill line is named by its own name, and its heading on the
        // skills panel says what kind of skill it is.
        "SkillLine" => {
            let category = session
                .table("SkillLineCategory")
                .and_then(|categories| categories.string_at(categories.row_of(num(1))?, 1))
                .unwrap_or_default();
            (text(3), category)
        }
        // An ability has no name of its own. It is a spell in a skill line
        // for some classes, so the label is those three: `Fireball`, then
        // `Rank 1 · Fire · Mage`.
        "SkillLineAbility" => {
            let (skill, spell) = (num(1), num(2));
            let named = spell_label(browser, session, spell);
            let title = match &named {
                Some((name, _)) => name.clone(),
                None => format!("spell {spell}"),
            };
            let mut parts: Vec<String> = Vec::new();
            parts.extend(named.map(|(_, rank)| rank).filter(|rank| !rank.is_empty()));
            parts.push(skill_name(session, skill).unwrap_or_else(|| format!("skill {skill}")));
            let who = mask_words(&schema::SKILL_LINE_ABILITY, 4, num(4));
            parts.extend((!who.is_empty()).then_some(who));
            (title, parts.join(" · "))
        }
        // A row of this table gives a skill line to some races and classes,
        // so it is the line's name and who gets it.
        "SkillRaceClassInfo" => {
            let skill = num(1);
            let title = skill_name(session, skill).unwrap_or_else(|| format!("skill {skill}"));
            let who: Vec<String> = [
                mask_words(&schema::SKILL_RACE_CLASS_INFO, 3, num(3)),
                mask_words(&schema::SKILL_RACE_CLASS_INFO, 2, num(2)),
            ]
            .into_iter()
            .filter(|words| !words.is_empty())
            .collect();
            let sub = match who.is_empty() {
                true => "every race and class".to_string(),
                false => who.join(" · "),
            };
            (title, sub)
        }
        // A set is its name, and how much of the row is used: seventeen
        // item columns and eight bonus columns, most of them empty.
        "ItemSet" => {
            let count = |columns: std::ops::Range<usize>| columns.filter(|&field| num(field) != 0).count();
            let (items, bonuses) = (count(10..27), count(27..35));
            (text(1), format!("{items} items · {bonuses} bonuses"))
        }
        // An area is its name, and where it is: `zone · Azeroth` for a zone
        // and `in Elwynn Forest · Azeroth` for a sub-area. The search index
        // is built from the label, so a zone's name finds its sub-areas and
        // the word `zone` finds the zones.
        "AreaTable" => {
            let map = num(area::MAP);
            let map_name = session
                .table("Map")
                .and_then(|maps| maps.string_at(maps.row_of(map)?, 1))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("map {map}"));
            let sub = match num(area::PARENT) {
                0 => format!("zone \u{b7} {map_name}"),
                parent => {
                    let zone = table
                        .row_of(parent)
                        .and_then(|row| table.string_at(row, area::NAME))
                        .unwrap_or_else(|| format!("area {parent}"));
                    format!("in {zone} \u{b7} {map_name}")
                }
            };
            (text(area::NAME), sub)
        }
        _ => (
            browser
                .name_field(session, table_name)
                .map(text)
                .unwrap_or_default(),
            String::new(),
        ),
    };
    RowLabel { title, sub, id }
}

/// A spell's name and rank out of the open `Spell` table.
fn spell_label(browser: &mut Browser, session: &EditSession, id: u32) -> Option<(String, String)> {
    use vale_assets::tables::spellbook::spell_fields;
    let record = browser.record_of(session, "Spell", id)?;
    let spells = session.table("Spell")?;
    Some((
        spells.string_at(record, spell_fields::NAME).unwrap_or_default(),
        spells.string_at(record, spell_fields::RANK).unwrap_or_default(),
    ))
}

/// A skill line's name out of the open `SkillLine` table.
fn skill_name(session: &EditSession, id: u32) -> Option<String> {
    let lines = session.table("SkillLine")?;
    lines
        .string_at(lines.row_of(id)?, 3)
        .filter(|name| !name.is_empty())
}

/// A race or class mask in words: the names of its set bits, or nothing
/// for a mask that leaves nobody out. Zero and every named bit both mean
/// everyone.
fn mask_words(table: &Schema, field: usize, value: u32) -> String {
    let Some(Kind::Flags(bits)) = table.column(field).map(|column| column.kind) else {
        return String::new();
    };
    let every = bits.iter().fold(0u32, |all, (bit, _, _)| all | bit);
    if value == 0 || value & every == every {
        return String::new();
    }
    schema::named_bits(value, bits).join(", ")
}

/// The string a field's value starts, when the value is the offset of a
/// string's first byte in the table's string block: past the block's
/// opening NUL, directly after another NUL, and printable. `None` for a
/// value that is a number.
pub fn string_started_at(table: &vale_edit::dbc::DbcFile, raw: u32) -> Option<String> {
    let offset = raw as usize;
    if offset == 0 || offset >= table.string_size() {
        return None;
    }
    if table.text_at(offset - 1).as_deref() != Some("") {
        return None;
    }
    table
        .text_at(offset)
        .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

/// How many rows the guesses about a table with no schema read.
const GUESS_ROWS: usize = 64;

/// Whether a column of a table with no schema holds strings, and in how
/// many of the sampled rows: every sampled row holds zero or the offset of
/// a string, and the strings are not all one string.
///
/// The test is on the column and not on one value. A small number can be
/// the offset of a string's first byte: `Faction.dbc`'s field 1 holds 1 on
/// some rows, which is where the block's first string starts. A column of
/// such numbers fails on the rows that hold any other number, and a
/// column that holds one number throughout fails the last condition.
fn strings_in(table: &vale_edit::dbc::DbcFile, field: usize) -> Option<usize> {
    let rows = table.record_count().min(GUESS_ROWS);
    let mut texts: Vec<String> = Vec::new();
    for record in 0..rows {
        match table.u32_at(record, field).unwrap_or(0) {
            0 => {}
            raw => texts.push(string_started_at(table, raw)?),
        }
    }
    let held = texts.len();
    texts.sort();
    texts.dedup();
    (texts.len() > 1 || (rows == 1 && held == 1)).then_some(held)
}

/// Which columns of a table with no schema hold strings, by field. Field 0
/// is the id and never does.
pub fn text_fields(table: &vale_edit::dbc::DbcFile) -> Vec<bool> {
    (0..table.field_count())
        .map(|field| field > 0 && strings_in(table, field).is_some())
        .collect()
}

/// The column a table with no schema is named by, guessed from the file:
/// the first column of strings in which at least half the sampled rows
/// hold one.
pub fn guess_name_field(table: &vale_edit::dbc::DbcFile) -> Option<usize> {
    let rows = table.record_count().min(GUESS_ROWS);
    (1..table.field_count())
        .find(|&field| strings_in(table, field).is_some_and(|held| held * 2 >= rows))
}

/// The spells that reference a row of `target`, as a phrase such as
/// `Fireball (+8)`.
fn spells_using(browser: &mut Browser, session: &EditSession, target: &str, id: u32) -> String {
    let uses = browser.used_by(session, target, id);
    let spells: Vec<usize> = uses
        .iter()
        .filter(|at| at.table == "Spell")
        .map(|at| at.record)
        .collect();
    let Some(&first) = spells.first() else {
        return String::new();
    };
    let name = session
        .table("Spell")
        .and_then(|spell| {
            spell.string_at(first, vale_assets::tables::spellbook::spell_fields::NAME)
        })
        .unwrap_or_default();
    match spells.len() {
        1 => name,
        n => format!("{name} (+{})", n - 1),
    }
}

/// The lights that reference a `LightParams` row, as a phrase such as
/// `Azeroth 533..718 yd (+2)`.
///
/// This is the light chain's equivalent of [`spells_using`], and it cannot
/// share code with it: [`spells_using`] reads the referring row's name column,
/// and a `Light` row has none, so the referring row is labelled through
/// [`describe`] instead.
fn lights_using(browser: &mut Browser, session: &EditSession, id: u32) -> String {
    let uses = browser.used_by(session, "LightParams", id);
    let lights: Vec<usize> = uses
        .iter()
        .filter(|at| at.table == "Light")
        .map(|at| at.record)
        .collect();
    let Some(&first) = lights.first() else {
        return String::new();
    };
    let name = describe(browser, session, "Light", first).title;
    match lights.len() {
        1 => name,
        n => format!("{name} (+{})", n - 1),
    }
}

/// Which slot of which visual a kit fills, and which spells use that visual.
fn kit_slot(browser: &mut Browser, session: &EditSession, kit: u32) -> Option<String> {
    let uses = browser.used_by(session, "SpellVisualKit", kit);
    let at = uses.iter().find(|at| at.table == "SpellVisual")?;
    let slot = VISUAL_SLOTS
        .iter()
        .find(|(field, _)| *field == at.field)
        .map(|(_, word)| *word)
        .unwrap_or("Area");
    let visual = session.table("SpellVisual")?.u32_at(at.record, 0)?;
    let spell = spells_using(browser, session, "SpellVisual", visual);
    Some(match spell.is_empty() {
        true => format!("{slot} of visual {visual}"),
        false => format!("{spell} · {slot}"),
    })
}

/// The last component of an archive path.
pub fn basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// The tables a subject needs in addition to its own root table.
///
/// There are three chains (the spell chain, the light chain and the flight
/// path tables) and the tables a skill table or an item set is labelled
/// from. A spell's form can be drawn from `Spell.dbc` alone, but its
/// storyboard cannot, and neither can a followed reference or a row labelled
/// from the rows it references. These tables are opened one per frame after
/// the subject's own; together they are smaller than `Spell.dbc`'s string
/// block, so opening them has no noticeable cost.
pub fn chain_for(table: &str) -> &'static [&'static str] {
    match table {
        "Spell" => &[
            "SpellVisual",
            "SpellVisualKit",
            "SpellVisualEffectName",
            "AnimationData",
            "SpellIcon",
            "SpellCastTimes",
            "SpellDuration",
            "SpellRadius",
            "SpellRange",
            "SoundEntries",
            "SpellEffectCameraShakes",
            // The skill tables, which are tabs of the spell workspace. With
            // `SkillLineAbility` open, a spell's reverse references list the
            // skill lines it is in.
            "SkillLine",
            "SkillLineAbility",
            "SkillRaceClassInfo",
            "SkillLineCategory",
        ],
        "Light" => &[
            "LightParams",
            "LightIntBand",
            "LightFloatBand",
            "LightSkybox",
            // `Map` is not part of the light chain but is opened with it. It is
            // not edited here and has no tab; it is read so a light's label
            // can show `Azeroth` instead of `map 0`. `Light` rows have no
            // names, so the map is most of what the label shows.
            "Map",
        ],
        // The paths and their points, which a node's form lists, and `Map` for
        // the same reason the light chain opens it.
        "TaxiNodes" => &["TaxiPath", "TaxiPathNode", "Map"],
        // A set's bonuses are spells and its requirement is a skill line.
        "ItemSet" => &["Spell", "SkillLine"],
        // The skill tables, for when one is opened from the Tables
        // workspace and the spell chain is not open: each is labelled from
        // the others.
        "SkillLine" => &["SkillLineCategory", "SpellIcon"],
        "SkillLineAbility" | "SkillRaceClassInfo" => &["SkillLine", "Spell"],
        // `Map`, which an area's label names, and the five tables its sound
        // and liquid columns refer to, so each reference is drawn as a name.
        //
        // `SoundEntries` as well, which the music, intro and ambience rows
        // name, so a sound's play button says which sound it plays.
        "AreaTable" => &[
            "Map",
            "ZoneMusic",
            "ZoneIntroMusicTable",
            "SoundAmbience",
            "SoundProviderPreferences",
            "LiquidType",
            "SoundEntries",
        ],
        _ => &[],
    }
}

/// The flight path tables, in the order a flight is read: the node, the path
/// from it, the points of the path.
pub const TAXI_TABS: [(&str, &str); 3] = [
    ("Nodes", "TaxiNodes"),
    ("Paths", "TaxiPath"),
    ("Points", "TaxiPathNode"),
];

/// The trigger tool's one table. It keeps the viewport, so no tab is drawn.
pub const TRIGGER_TABS: [(&str, &str); 1] = [("Triggers", "AreaTrigger")];

/// …and the graveyard tool's.
pub const SAFE_LOC_TABS: [(&str, &str); 1] = [("Graveyards", "WorldSafeLocs")];

/// The spell workspace's tabs: the spell chain in the order a cast reads it,
/// then the three skill tables. The workspace draws them [`TAB_ROW`] to a
/// row, so the chain is the first row and the skill tables the second.
/// Callers read the tabs through [`super::Tool::tabs`], which chooses
/// between this, [`LIGHT_TABS`], [`TAXI_TABS`] and [`SET_TABS`].
pub const SPELL_TABS: [(&str, &str); 7] = [
    ("Spells", "Spell"),
    ("Visuals", "SpellVisual"),
    ("Kits", "SpellVisualKit"),
    ("Effects", "SpellVisualEffectName"),
    ("Skill lines", "SkillLine"),
    ("Abilities", "SkillLineAbility"),
    ("Race & class", "SkillRaceClassInfo"),
];

/// How many tabs the middle workspace draws to a row.
pub const TAB_ROW: usize = 4;

/// The Sets part of the Items workspace: one table. A single tab is not
/// drawn; the strip above it switches between the workspace's parts.
pub const SET_TABS: [(&str, &str); 1] = [("Sets", "ItemSet")];

/// The Zones workspace: one table, so no tab is drawn. The zones and their
/// sub-areas are rows of the same table.
pub const ZONE_TABS: [(&str, &str); 1] = [("Areas", "AreaTable")];

/// The table the Tables workspace starts on: none. Its list is the tables
/// themselves until one is chosen. See [`super::Tool::Tables`].
pub const ANY: &str = "";

/// The one name of the table `asked` names in any case, or `None` when it is
/// neither described nor a file of `DBFilesClient\`.
pub fn table_named(asked: &str) -> Option<&'static str> {
    schema::for_table(asked).map(|schema| schema.table).or_else(|| {
        schema::TABLE_NAMES
            .iter()
            .find(|known| known.eq_ignore_ascii_case(asked.trim()))
            .copied()
    })
}

/// The light chain's tabs, in the order it resolves: a light names params, and
/// params own bands.
///
/// `LightIntBand` and `LightFloatBand` have tabs although no column references
/// them. A band's id is computed from a params row rather than stored in a
/// reference column, so the browser cannot follow one, and the tab is the only
/// way to open those tables.
pub const LIGHT_TABS: [(&str, &str); 5] = [
    ("Lights", "Light"),
    ("Params", "LightParams"),
    ("Colours", "LightIntBand"),
    ("Numbers", "LightFloatBand"),
    ("Skyboxes", "LightSkybox"),
];

/// The column a row is named by: the first string in the table's own locale.
pub fn label_field(schema: &Schema) -> Option<usize> {
    schema
        .columns
        .iter()
        .find(|column| column.kind == Kind::Text)
        .map(|column| column.field)
}

/// Write one field and record it on the undo stack.
///
/// The gesture key is the field, so a number dragged through forty values in
/// one motion is one entry, and the same field changed again half a second
/// later is a second entry. That is `History::begin_gesture`'s rule, which the
/// panels also use for a dragged rotation.
pub fn set_field(
    session: &mut EditSession,
    table_name: &str,
    record: usize,
    field: usize,
    value: u32,
    label: &str,
    now: f64,
) {
    let Some(table) = session.tables.get_mut(table_name) else {
        return;
    };
    let Some(edit) = Cell::new(table, record, field, value) else {
        return;
    };
    if !edit.moves() {
        return;
    }
    edit.apply(table);
    session
        .history
        .begin_gesture(label, format!("{table_name} {record} field {field}"), now);
    session.history.record_cell(table_name, edit);
    session.history.end();
    session.table_edited(table_name);
}

/// Write several fields of one row as a single gesture.
///
/// [`set_field`] keys its gesture on the field, so a drag that changes two
/// columns would make two entries on the stack, and one `Ctrl+Z` would undo
/// half of the movement. A light dragged across the map changes `InternalX`
/// and `InternalZ`. A dragged radius changes one column, and writing it through
/// this function as well keeps both drags on the same key, so a drag that
/// changes from one to the other is still one entry.
///
/// `subject` is the gesture key. Pass a name for the movement rather than for
/// the columns; [`vale_edit::undo::History::begin_gesture`] merges writes
/// with the same key inside its time window into one change.
pub fn set_fields(
    session: &mut EditSession,
    table_name: &str,
    record: usize,
    fields: &[(usize, u32)],
    label: &str,
    subject: &str,
    now: f64,
) {
    let Some(table) = session.tables.get_mut(table_name) else {
        return;
    };
    let edits: Vec<Cell> = fields
        .iter()
        .filter_map(|&(field, value)| Cell::new(table, record, field, value))
        .filter(|edit| edit.moves())
        .collect();
    // Return without opening a gesture when nothing changes. A drag reports a
    // position every frame, and on most frames the movement is too small to
    // change a float's bits. Opening a gesture for each such frame would push
    // an empty change and end the run of entries the next real change should
    // merge into.
    if edits.is_empty() {
        return;
    }
    session.history.begin_gesture(label, subject, now);
    for edit in edits {
        edit.apply(table_mut(session, table_name));
        session.history.record_cell(table_name, edit);
    }
    session.history.end();
    session.table_edited(table_name);
}

/// The open table, looked up again on each iteration of [`set_fields`]' loop:
/// `apply` needs it mutably, and so does the history between iterations.
fn table_mut<'a>(
    session: &'a mut EditSession,
    table_name: &str,
) -> &'a mut vale_edit::dbc::DbcFile {
    session
        .tables
        .get_mut(table_name)
        .expect("checked by the caller")
}

/// Write one string field and record it on the undo stack, as [`set_field`]
/// does for a number. A string write appends to the string block and then
/// writes the field.
pub fn set_text(
    session: &mut EditSession,
    table_name: &str,
    record: usize,
    field: usize,
    text: &str,
    label: &str,
    now: f64,
) {
    let Some(table) = session.tables.get_mut(table_name) else {
        return;
    };
    let Some(edit) = cell::set_text(table, record, field, text) else {
        return;
    };
    if !edit.moves() {
        return;
    }
    session
        .history
        .begin_gesture(label, format!("{table_name} {record} field {field}"), now);
    session.history.record_cell(table_name, edit);
    session.history.end();
    session.table_edited(table_name);
}

// ---------------------------------------------------------------------------
// Rows of one table edited from another table's form
// ---------------------------------------------------------------------------

/// `SkillLineAbility.dbc`'s fields, as the spell form's Learning section and
/// the skill line form read them. The indices are
/// `schema::SKILL_LINE_ABILITY`'s.
pub mod ability {
    pub const TABLE: &str = "SkillLineAbility";
    pub const SKILL: usize = 1;
    pub const SPELL: usize = 2;
    pub const RACES: usize = 3;
    pub const CLASSES: usize = 4;
    pub const REQ_SKILL_VALUE: usize = 7;
    pub const SUPERSEDED_BY: usize = 8;
    pub const LEARN_ON_GET_SKILL: usize = 9;
    pub const MAX_VALUE: usize = 10;
    pub const MIN_VALUE: usize = 11;
    pub const REQ_TRAIN_POINTS: usize = 14;
}

/// `SkillRaceClassInfo.dbc`'s fields, as the skill line form reads them. The
/// indices are `schema::SKILL_RACE_CLASS_INFO`'s.
pub mod race_class {
    pub const TABLE: &str = "SkillRaceClassInfo";
    pub const SKILL: usize = 1;
    pub const RACES: usize = 2;
    pub const CLASSES: usize = 3;
    pub const FLAGS: usize = 4;
    pub const MIN_LEVEL: usize = 5;
    pub const SKILL_TIER: usize = 6;
    /// Every race: 134 of the 201 shipped rows hold it.
    pub const EVERY_RACE: u32 = 511;
    /// Every class: 60 of the shipped rows hold it.
    pub const EVERY_CLASS: u32 = 1503;
}

/// `ItemSet.dbc`'s seventeen item columns.
pub const SET_ITEMS: std::ops::Range<usize> = 10..27;

/// The records of `table` whose reference column `field` names row `id` of
/// `target`, in file order, through the reverse index.
fn rows_naming(
    browser: &mut Browser,
    session: &EditSession,
    target: &str,
    id: u32,
    table: &str,
    field: usize,
) -> Vec<usize> {
    let mut rows: Vec<usize> = browser
        .used_by(session, target, id)
        .into_iter()
        .filter(|at| at.table == table && at.field == field)
        .map(|at| at.record)
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// The `SkillLineAbility` rows that put `spell` in a skill line. Most spells
/// have none: 4,753 of the shipped spells are named by a row.
pub fn abilities_of(browser: &mut Browser, session: &EditSession, spell: u32) -> Vec<usize> {
    rows_naming(browser, session, "Spell", spell, ability::TABLE, ability::SPELL)
}

/// The `SkillLineAbility` rows of one skill line.
pub fn abilities_in(browser: &mut Browser, session: &EditSession, skill: u32) -> Vec<usize> {
    rows_naming(browser, session, "SkillLine", skill, ability::TABLE, ability::SKILL)
}

/// The `SkillRaceClassInfo` rows that give `skill` to races and classes.
pub fn race_class_rows_of(browser: &mut Browser, session: &EditSession, skill: u32) -> Vec<usize> {
    rows_naming(browser, session, "SkillLine", skill, race_class::TABLE, race_class::SKILL)
}

/// `SpellCastTimes.dbc`'s row for an instant cast, which is what both mage
/// trainer lists' teaching spells use; see `super::services::teaching_spell`.
const INSTANT_CAST: u32 = 1;

/// The teaching spells of `spell`, as the record of each and whether it is
/// instant: the rows of `Spell.dbc` whose first effect is `LEARN_SPELL` and
/// whose `EffectTriggerSpell 1` is `spell`. A trainer's list names one of
/// these and not the spell itself.
///
/// 4,183 of the shipped spells have one and 511 of those have more than one,
/// usually an instant one a trainer's list names beside one with a cast time
/// that a book casts.
pub fn teachers_of(browser: &mut Browser, session: &EditSession, spell: u32) -> Vec<(usize, bool)> {
    use vale_assets::tables::spellbook::spell_fields;
    let rows = rows_naming(
        browser,
        session,
        "Spell",
        spell,
        "Spell",
        spell_fields::EFFECT_TRIGGER_SPELL,
    );
    let Some(spells) = session.table("Spell") else {
        return Vec::new();
    };
    rows.into_iter()
        .filter(|&record| {
            spells.u32_at(record, spell_fields::EFFECT) == Some(vale_mangos::trainer::LEARN_SPELL)
        })
        .map(|record| {
            let instant = spells.u32_at(record, spell_fields::CASTING_TIME_INDEX) == Some(INSTANT_CAST);
            (record, instant)
        })
        .collect()
}

/// A new row of `table_name` under the next id with `numbers` and `texts`
/// written, recorded in the history entry that is open. Returns its record.
fn push_row(
    session: &mut EditSession,
    table_name: &str,
    numbers: &[(usize, u32)],
    texts: &[(usize, String)],
) -> Option<usize> {
    let table = session.tables.get_mut(table_name)?;
    let id = table.max_id().checked_add(1)?;
    let bytes = table.blank_record(id);
    let at = table.push_record(&bytes)?;
    let row = Row::added(table, at)?;
    session.history.record_row(table_name, row);
    for &(field, value) in numbers {
        let table = session.tables.get_mut(table_name)?;
        if let Some(edit) = Cell::new(table, at, field, value) {
            edit.apply(table);
            session.history.record_cell(table_name, edit);
        }
    }
    for (field, text) in texts {
        let table = session.tables.get_mut(table_name)?;
        if let Some(edit) = cell::set_text(table, at, *field, text) {
            session.history.record_cell(table_name, edit);
        }
    }
    session.table_edited(table_name);
    Some(at)
}

/// Put `spell` in a skill line: a new `SkillLineAbility` row naming it, as
/// one undo entry. The skill line is not chosen here; the form opens the
/// picker on the new row. `ReqSkillValue` starts at 1, which 5,026 of the
/// 5,072 shipped rows hold.
pub fn add_ability(session: &mut EditSession, spell: u32) -> Option<usize> {
    session.history.begin("Add to a skill line");
    let at = push_row(
        session,
        ability::TABLE,
        &[(ability::SPELL, spell), (ability::REQ_SKILL_VALUE, 1)],
        &[],
    );
    session.history.end();
    at
}

/// Give `skill` to races and classes: a new `SkillRaceClassInfo` row naming
/// it, for every race and every class, as one undo entry. The flags start at
/// zero, which lists the line normally and gives it a spellbook tab.
pub fn add_race_class_row(session: &mut EditSession, skill: u32) -> Option<usize> {
    session.history.begin("Give the skill line to races and classes");
    let at = push_row(
        session,
        race_class::TABLE,
        &[
            (race_class::SKILL, skill),
            (race_class::RACES, race_class::EVERY_RACE),
            (race_class::CLASSES, race_class::EVERY_CLASS),
        ],
        &[],
    );
    session.history.end();
    at
}

/// `AreaTable.dbc`'s fields, as the Zones workspace reads and writes them.
/// The indices are `vale_assets::tables::area::fields`'.
pub mod area {
    use vale_assets::tables::area::fields;
    pub const TABLE: &str = "AreaTable";
    pub const MAP: usize = fields::MAP_ID;
    pub const PARENT: usize = fields::PARENT;
    pub const EXPLORE_BIT: usize = fields::AREA_BIT;
    pub const FLAGS: usize = fields::FLAGS;
    pub const NAME: usize = fields::NAME;
    pub const NAME_FLAGS: usize = fields::NAME_FLAGS;
    /// `AREA_FLAG_DUEL`, which 969 of the 1,081 shipped rows carry and a new
    /// zone starts with.
    pub const DUELS: u32 = 0x40;
    /// The word after the name columns on 724 of the 1,081 shipped rows; the
    /// other 357 hold 8,323,198. No source describes the column, so a new row
    /// takes the commoner shipped value and not zero, which no shipped row
    /// holds.
    pub const SHIPPED_NAME_FLAGS: u32 = 4_128_894;
    /// What a new sub-area takes from its zone: the flags, the two reverb
    /// presets, the ambience and the music. Northshire Valley holds Elwynn
    /// Forest's values in all five.
    pub const FROM_ZONE: [usize; 5] = [4, 5, 6, 7, 8];
    pub const NEW_ZONE: &str = "New zone";
    pub const NEW_SUB_AREA: &str = "New area";
}

/// The lowest explore bit above every one the table uses. The shipped rows
/// use 0 to 1076 with no gap, so a new area takes 1077. Two areas that share
/// a bit are discovered together, which is why a new row does not reuse one.
pub fn next_explore_bit(areas: &vale_edit::dbc::DbcFile) -> u32 {
    (0..areas.record_count())
        .filter_map(|record| areas.u32_at(record, area::EXPLORE_BIT))
        .max()
        .map_or(0, |highest| highest.saturating_add(1))
}

/// The sub-areas of a zone: the rows whose parent is `zone`, in file order.
pub fn sub_areas_of(browser: &mut Browser, session: &EditSession, zone: u32) -> Vec<usize> {
    rows_naming(browser, session, area::TABLE, zone, area::TABLE, area::PARENT)
}

/// A new zone on `map`, as one undo entry: no parent, the next explore bit,
/// duels allowed, and a name to be replaced. Returns its record.
pub fn add_zone(session: &mut EditSession, map: u32) -> Option<usize> {
    let bit = next_explore_bit(session.table(area::TABLE)?);
    session.history.begin("Add zone");
    let at = push_row(
        session,
        area::TABLE,
        &[
            (area::MAP, map),
            (area::EXPLORE_BIT, bit),
            (area::FLAGS, area::DUELS),
            (area::NAME_FLAGS, area::SHIPPED_NAME_FLAGS),
        ],
        &[(area::NAME, area::NEW_ZONE.to_string())],
    );
    session.history.end();
    at
}

/// A new `Light` row on `map`, as one undo entry. At `centre`, in the world's
/// axes, it is a sphere with the two falloff radii in yards; with `None` it is
/// the map's default light, whose falloff is zero and whose position is not
/// read. Its five `LightParams` ids are copied from the `Light` row at
/// `template`, so it is lit and fogged like that light until its own are
/// chosen. Returns the new row's record.
pub fn add_light(
    session: &mut EditSession,
    map: u32,
    centre: Option<[f32; 3]>,
    radii: (f32, f32),
    template: Option<usize>,
) -> Option<usize> {
    use vale_assets::tables::light::{light_field as lf, YARDS_PER_UNIT};
    let lights = session.table("Light")?;
    let params: Vec<(usize, u32)> = (lf::PARAMS_CLEAR..lf::PARAMS_CLEAR + 5)
        .filter_map(|field| Some((field, lights.u32_at(template?, field)?)))
        .collect();
    let mut numbers = vec![(lf::MAP, map)];
    if let Some(centre) = centre {
        let placement = vale_assets::world::adt::placement_from_world(centre);
        numbers.extend([
            (lf::INTERNAL_X, (placement[0] / YARDS_PER_UNIT).to_bits()),
            (lf::INTERNAL_Y, (placement[1] / YARDS_PER_UNIT).to_bits()),
            (lf::INTERNAL_Z, (placement[2] / YARDS_PER_UNIT).to_bits()),
            (lf::FALLOFF_START, (radii.0 / YARDS_PER_UNIT).to_bits()),
            (lf::FALLOFF_END, (radii.1 / YARDS_PER_UNIT).to_bits()),
        ]);
    }
    numbers.extend(params);
    session.history.begin("Add light");
    let at = push_row(session, "Light", &numbers, &[]);
    session.history.end();
    at
}

/// A new sub-area of the zone `of` is, or is in, as one undo entry. It takes
/// the zone's map and [`area::FROM_ZONE`]'s columns, the next explore bit, and
/// a name to be replaced. Asked of a sub-area, it makes a sibling: the table
/// is two levels, and no shipped row's parent has a parent. Returns the new
/// row's record.
pub fn add_sub_area(session: &mut EditSession, of: u32) -> Option<usize> {
    let areas = session.table(area::TABLE)?;
    let asked = areas.row_of(of)?;
    let zone = match areas.u32_at(asked, area::PARENT)? {
        0 => of,
        parent => parent,
    };
    // A parent the table does not hold: the row asked of stands in for it.
    let from = areas.row_of(zone).unwrap_or(asked);
    let mut numbers = vec![
        (area::MAP, areas.u32_at(from, area::MAP)?),
        (area::PARENT, zone),
        (area::EXPLORE_BIT, next_explore_bit(areas)),
        (area::NAME_FLAGS, area::SHIPPED_NAME_FLAGS),
    ];
    for field in area::FROM_ZONE {
        numbers.push((field, areas.u32_at(from, field)?));
    }
    session.history.begin("Add sub-area");
    let at = push_row(
        session,
        area::TABLE,
        &numbers,
        &[(area::NAME, area::NEW_SUB_AREA.to_string())],
    );
    session.history.end();
    at
}

/// What a new teaching spell holds besides its name, rank, icon and the spell
/// it teaches, as `(field, value)`. Each value is the one most of the 3,408
/// shipped instant teaching spells hold, with the count beside it; a field
/// not listed is zero on most of them.
const TEACHING_SPELL: [(usize, u32); 12] = [
    // Attributes: 3,265.
    (6, 0x0004_0100),
    // Targets: 3,194.
    (13, 0x100),
    // CastingTimeIndex, instant: all of them, by the population's definition.
    (18, INSTANT_CAST),
    // ProcChance: 3,381.
    (25, 101),
    // RangeIndex: 3,350.
    (36, 6),
    // EquippedItemClass, none: all 3,408.
    (58, u32::MAX),
    // EquippedItemSubClassMask: 2,959.
    (59, u32::MAX),
    // Effect 1.
    (61, vale_mangos::trainer::LEARN_SPELL),
    // SpellVisual, the learning visual: 3,390.
    (115, 107),
    // StanceBarOrder, none: 3,397.
    (166, u32::MAX),
    // DmgMultiplier 1 and 2, 1.0: 3,408 and 3,397.
    (167, 0x3F80_0000),
    (168, 0x3F80_0000),
];

/// `DmgMultiplier 3`, which is 1.0 on 3,397 of the shipped instant teaching
/// spells, as the other two are.
const TEACHING_SPELL_MULTIPLIER_3: (usize, u32) = (169, 0x3F80_0000);

/// Make the spell that teaches `spell`: a new `Spell.dbc` row whose first
/// effect is `LEARN_SPELL` and whose `EffectTriggerSpell 1` is `spell`, as one
/// undo entry. Returns the new spell's id, or `None` when `spell` is not a row
/// of the open table.
///
/// The row takes the taught spell's name, rank and icon, which is what the
/// shipped ones do (2,958, 3,126 and 2,748 of the 3,392 whose taught spell
/// exists), and [`TEACHING_SPELL`] for the rest.
pub fn add_teaching_spell(session: &mut EditSession, spell: u32) -> Option<u32> {
    use vale_assets::tables::spellbook::spell_fields;
    let spells = session.table("Spell")?;
    let taught = spells.row_of(spell)?;
    let name = spells.string_at(taught, spell_fields::NAME).unwrap_or_default();
    let rank = spells.string_at(taught, spell_fields::RANK).unwrap_or_default();
    let icon = spells.u32_at(taught, spell_fields::ICON_ID).unwrap_or(0);
    // The locale flag words after the name and the rank, copied so the new
    // row marks the same locales as present.
    let name_flags = spells.u32_at(taught, spell_fields::NAME + 8).unwrap_or(0);
    let rank_flags = spells.u32_at(taught, spell_fields::RANK + 8).unwrap_or(0);

    let mut numbers: Vec<(usize, u32)> = TEACHING_SPELL.to_vec();
    numbers.push(TEACHING_SPELL_MULTIPLIER_3);
    numbers.push((spell_fields::EFFECT_TRIGGER_SPELL, spell));
    numbers.push((spell_fields::ICON_ID, icon));
    numbers.push((spell_fields::NAME + 8, name_flags));
    numbers.push((spell_fields::RANK + 8, rank_flags));
    let mut texts = vec![(spell_fields::NAME, name)];
    if !rank.is_empty() {
        texts.push((spell_fields::RANK, rank));
    }

    session.history.begin("Create teaching spell");
    let at = push_row(session, "Spell", &numbers, &texts);
    session.history.end();
    session.table("Spell")?.u32_at(at?, 0)
}

/// Which item column of an `ItemSet` row lists `item`, if one does.
pub fn set_lists(sets: &vale_edit::dbc::DbcFile, record: usize, item: u32) -> Option<usize> {
    let mut fields = SET_ITEMS;
    fields.find(|&field| item != 0 && sets.u32_at(record, field) == Some(item))
}

/// The first item column of an `ItemSet` row that holds no item.
fn empty_item_column(sets: &vale_edit::dbc::DbcFile, record: usize) -> Option<usize> {
    let mut fields = SET_ITEMS;
    fields.find(|&field| sets.u32_at(record, field) == Some(0))
}

/// Move `item` between two sets' item lists in the open `ItemSet` table: it
/// is taken out of every item column of set `from` that lists it and put in
/// the first empty item column of set `to`, where `to` does not list it
/// already. Zero for either is no set. Returns one sentence per change made
/// or refused.
///
/// The edits are recorded under `subject`, so that when the caller has just
/// written the item's `set_id` under the same subject the two are one undo
/// entry: the item's column and the set's list are one fact stored twice.
pub fn move_between_sets(
    session: &mut EditSession,
    item: u32,
    from: u32,
    to: u32,
    subject: &str,
    now: f64,
) -> Vec<String> {
    let mut said = Vec::new();
    if item == 0 || from == to {
        return said;
    }
    let Some(sets) = session.table("ItemSet") else {
        return said;
    };
    let mut edits: Vec<(usize, usize, u32)> = Vec::new();
    if let Some(record) = sets.row_of(from).filter(|_| from != 0) {
        for field in SET_ITEMS.filter(|&field| sets.u32_at(record, field) == Some(item)) {
            edits.push((record, field, 0));
        }
        if !edits.is_empty() {
            said.push(format!("item {item} taken out of set {from}'s items"));
        }
    }
    if to != 0 {
        match sets.row_of(to) {
            None => said.push(format!("ItemSet has no row {to}; item {item} is listed nowhere")),
            Some(record) if set_lists(sets, record, item).is_some() => {}
            Some(record) => match empty_item_column(sets, record) {
                Some(field) => {
                    edits.push((record, field, item));
                    said.push(format!("item {item} added to set {to}'s items"));
                }
                None => said.push(format!(
                    "set {to} has no empty item column; item {item} is not listed in it"
                )),
            },
        }
    }
    if edits.is_empty() {
        return said;
    }
    session.history.begin_gesture("Edit item", subject, now);
    for (record, field, value) in edits {
        let table = table_mut(session, "ItemSet");
        if let Some(edit) = Cell::new(table, record, field, value) {
            edit.apply(table);
            session.history.record_cell("ItemSet", edit);
        }
    }
    session.history.end();
    session.table_edited("ItemSet");
    said
}

/// The tables a deep copy of a `SpellVisual` follows into: its kits, and the
/// effects those kits and the visual's own missile and area columns name.
pub const CHAIN_DEEP: [&str; 2] = ["SpellVisualKit", "SpellVisualEffectName"];

/// The suffix appended to a copied row's name, so it can be told from the
/// original.
pub const COPY_SUFFIX: &str = " (copy)";

/// What a blank row of a table holds beyond zeros: `(field, value)` pairs.
///
/// A kit's model columns and its four procedural slots are `-1` for none in
/// the shipped rows (`build_backrefs` treats 0 and -1 alike), and a blank kit
/// with 0 in `Animation` would resolve to Stand; see the schema's note on that
/// column. In every other table, zero means none.
pub fn blank_defaults(table: &str) -> &'static [(usize, u32)] {
    const KIT: &[(usize, u32)] = &[
        (vale_assets::tables::spell::fields::ANIMATION, u32::MAX),
        (3, u32::MAX),
        (4, u32::MAX),
        (5, u32::MAX),
        (6, u32::MAX),
        (7, u32::MAX),
        (8, u32::MAX),
        (9, u32::MAX),
        (10, u32::MAX),
        (12, u32::MAX),
        (15, u32::MAX),
        (16, u32::MAX),
        (17, u32::MAX),
        (18, u32::MAX),
    ];
    match table {
        "SpellVisualKit" => KIT,
        _ => &[],
    }
}

/// Add a blank row to an open table under the next id, as one undo entry.
/// Returns its record index.
pub fn add_row(session: &mut EditSession, table_name: &str) -> Option<usize> {
    let table = session.tables.get_mut(table_name)?;
    let id = table.max_id().checked_add(1)?;
    let bytes = table.blank_record(id);
    let at = table.push_record(&bytes)?;
    let row = Row::added(table, at)?;
    session.history.begin("Add row");
    session.history.record_row(table_name, row);
    for &(field, value) in blank_defaults(table_name) {
        let table = session.tables.get_mut(table_name)?;
        if let Some(edit) = Cell::new(table, at, field, value) {
            edit.apply(table);
            session.history.record_cell(table_name, edit);
        }
    }
    session.history.end();
    session.table_edited(table_name);
    Some(at)
}

/// Copy one row under the next id, with its name suffixed, as one undo entry.
/// Returns the copy's record index.
pub fn clone_row(session: &mut EditSession, table_name: &str, record: usize) -> Option<usize> {
    let id = session.table(table_name)?.u32_at(record, 0)?;
    let done = chain::clone_row(&mut session.tables, table_name, id, Some(COPY_SUFFIX))?;
    let new_id = done.new_id(table_name, id)?;
    session.history.begin("Clone row");
    record_cloned(session, done);
    session.history.end();
    session.table_edited(table_name);
    session.table(table_name)?.row_of(new_id)
}

/// Remove one row, as one undo entry. Every record index after it shifts down
/// by one, so a caller holding one must look its row up again by id.
pub fn delete_row(session: &mut EditSession, table_name: &str, record: usize) -> bool {
    let Some(table) = session.tables.get_mut(table_name) else {
        return false;
    };
    let Some(row) = Row::removed(table, record) else {
        return false;
    };
    table.remove_record(record);
    session.history.begin("Remove row");
    session.history.record_row(table_name, row);
    session.history.end();
    session.table_edited(table_name);
    true
}

/// Copy a visual with every kit and effect it names, with the copies'
/// references pointed at the other copies, as one undo entry. When `assign`
/// names a reference, that reference is pointed at the copied visual in the
/// same entry. Returns what was copied.
///
/// `assign` is `(table, record, field)`, typically a spell's `SpellVisual`
/// column, so that spell casts the copy while every other spell keeps casting
/// the original.
pub fn clone_chain(
    session: &mut EditSession,
    visual: u32,
    assign: Option<(&str, usize, usize)>,
) -> Option<chain::Cloned> {
    let done = chain::clone_chain(
        &mut session.tables,
        "SpellVisual",
        visual,
        &CHAIN_DEEP,
        Some(COPY_SUFFIX),
    )?;
    let new_id = done.new_id("SpellVisual", visual)?;
    session.history.begin("Clone chain");
    let touched: Vec<String> = done.ids.iter().map(|(table, _, _)| table.clone()).collect();
    record_cloned(session, done.clone());
    if let Some((table_name, record, field)) = assign {
        if let Some(table) = session.tables.get_mut(table_name) {
            if let Some(edit) = Cell::new(table, record, field, new_id) {
                edit.apply(table);
                session.history.record_cell(table_name, edit);
                session.table_edited(table_name);
            }
        }
    }
    session.history.end();
    for table in touched {
        session.table_edited(&table);
    }
    Some(done)
}

/// Add a blank row to the table a reference names and point the reference at
/// it, as one undo entry. Returns the new row's id.
pub fn link_new(
    session: &mut EditSession,
    table_name: &str,
    record: usize,
    field: usize,
    target: &str,
) -> Option<u32> {
    let target_table = session.tables.get_mut(target)?;
    let id = target_table.max_id().checked_add(1)?;
    let bytes = target_table.blank_record(id);
    let at = target_table.push_record(&bytes)?;
    let row = Row::added(target_table, at)?;
    session.history.begin("New linked row");
    session.history.record_row(target, row);
    for &(blank_field, value) in blank_defaults(target) {
        let target_table = session.tables.get_mut(target)?;
        if let Some(edit) = Cell::new(target_table, at, blank_field, value) {
            edit.apply(target_table);
            session.history.record_cell(target, edit);
        }
    }
    let table = session.tables.get_mut(table_name)?;
    if let Some(edit) = Cell::new(table, record, field, id) {
        edit.apply(table);
        session.history.record_cell(table_name, edit);
    }
    session.history.end();
    session.table_edited(target);
    session.table_edited(table_name);
    Some(id)
}

/// Copy the row a reference names and point the reference at the copy, as one
/// undo entry. Returns the copy's id.
///
/// This makes a shared kit one visual's own: the other visuals keep naming the
/// original.
pub fn unshare(
    session: &mut EditSession,
    table_name: &str,
    record: usize,
    field: usize,
    target: &str,
) -> Option<u32> {
    let old = session.table(table_name)?.u32_at(record, field)?;
    let done = chain::clone_row(&mut session.tables, target, old, Some(COPY_SUFFIX))?;
    let new_id = done.new_id(target, old)?;
    session.history.begin("Copy linked row");
    record_cloned(session, done);
    let table = session.tables.get_mut(table_name)?;
    if let Some(edit) = Cell::new(table, record, field, new_id) {
        edit.apply(table);
        session.history.record_cell(table_name, edit);
    }
    session.history.end();
    session.table_edited(target);
    session.table_edited(table_name);
    Some(new_id)
}

/// Put a copy's edits on the open history entry, rows first.
fn record_cloned(session: &mut EditSession, done: chain::Cloned) {
    for (table, row) in done.rows {
        session.history.record_row(&table, row);
    }
    for (table, cell) in done.cells {
        session.history.record_cell(&table, cell);
    }
}

/// A command on one row of a table.
///
/// The list's buttons, the form's buttons and a row's right-click menu all
/// take a row's commands from [`commands`] and word them through
/// [`command_label`], so a command reads the same wherever it is pressed,
/// and one that does not apply to a row is offered in none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    AddToSkillLine,
    CreateTeachingSpell,
    LookLike,
    CloneChain,
    GiveToRacesAndClasses,
    /// A new sub-area of a zone. Offered on a zone's row.
    AddSubArea,
    /// Give the area to the Areas tool's brush and switch to that tool.
    PaintArea,
    Clone,
    Delete,
    CopyId,
}

/// The commands the rows of a table take, in the order a menu lists them:
/// the table's own first, then the three every row has.
pub fn commands(table: &str) -> Vec<Command> {
    let mut all = match table {
        "Spell" => vec![
            Command::AddToSkillLine,
            Command::CreateTeachingSpell,
            Command::LookLike,
        ],
        "SpellVisual" => vec![Command::CloneChain],
        "SkillLine" => vec![Command::GiveToRacesAndClasses],
        "AreaTable" => vec![Command::AddSubArea, Command::PaintArea],
        _ => Vec::new(),
    };
    all.extend([Command::Clone, Command::Delete, Command::CopyId]);
    all
}

/// What a command is called for one row, or `None` when it does not apply to
/// that row. A command with a consequence states it in its name: a menu acts
/// on a row that has not been opened, so there is no form to read it from.
pub fn command_label(
    browser: &mut Browser,
    session: &EditSession,
    table: &str,
    record: usize,
    command: Command,
) -> Option<String> {
    let id = session.table(table)?.u32_at(record, 0)?;
    Some(match command {
        Command::AddToSkillLine => "Add to a skill line".to_string(),
        // Offered while the spell has no instant teaching spell, which is the
        // one a trainer's list names.
        Command::CreateTeachingSpell => {
            let teachers = teachers_of(browser, session, id);
            if teachers.iter().any(|&(_, instant)| instant) {
                return None;
            }
            match teachers.is_empty() {
                true => "Create a teaching spell".to_string(),
                false => "Create an instant teaching spell".to_string(),
            }
        }
        Command::LookLike => "Look like a spell\u{2026}".to_string(),
        Command::CloneChain => "Clone chain".to_string(),
        Command::GiveToRacesAndClasses => "Give it to races and classes".to_string(),
        // Offered on a zone. A sub-area's own sub-area would be a third
        // level, which the table does not have.
        Command::AddSubArea => {
            if session.table(table)?.u32_at(record, area::PARENT)? != 0 {
                return None;
            }
            "Add a sub-area".to_string()
        }
        Command::PaintArea => "Paint it on the map".to_string(),
        Command::Clone => "Clone".to_string(),
        Command::Delete => match browser.used_by(session, table, id).len() {
            0 => "Delete".to_string(),
            1 => "Delete: 1 reference will point at nothing".to_string(),
            n => format!("Delete: {n} references will point at nothing"),
        },
        Command::CopyId => format!("Copy id {id}"),
    })
}

/// What a command does, for the tooltip on its button and its menu entry.
pub fn command_about(command: Command) -> &'static str {
    match command {
        Command::AddToSkillLine => {
            "A new SkillLineAbility row naming this spell, and the picker for its skill \
             line. One undo entry. A mask left at zero is every class or every race."
        }
        Command::CreateTeachingSpell => {
            "A new spell under the next id that teaches this one: Learn Spell as its first \
             effect, an instant cast, and this spell's name, rank and icon. One undo entry. \
             The Trainer window can then add it to a trainer's list."
        }
        Command::LookLike => {
            "Choose this spell's visual by another spell, with that spell playing: share \
             its visual, or clone it."
        }
        Command::CloneChain => {
            "Copy this visual with every kit and effect it names, each rewired to the \
             copies, and open the copy. The original and the spells that share it are \
             untouched."
        }
        Command::GiveToRacesAndClasses => {
            "A new SkillRaceClassInfo row naming this line, for every race and every class, \
             with no flag set. One undo entry."
        }
        Command::AddSubArea => {
            "A new area inside this zone, on the zone's map, with the zone's flags, reverb, \
             ambience and music and an explore bit no other area holds. One undo entry."
        }
        Command::PaintArea => {
            "Switch to the Areas tool with this area on the brush. An area is on the map \
             where chunks carry its id, and nowhere until some do."
        }
        Command::Clone => "A copy of the row under the next id; a copied name gets \" (copy)\".",
        Command::Delete => "Remove the row. One undo entry.",
        Command::CopyId => "Put the row's id on the clipboard.",
    }
}

/// The model that pictures a row of the spell chain in a list: an effect's
/// own, a kit's first, and for a visual the first model of the first kit
/// that has one, then its missile's and its area's. `None` for a row with no
/// model and for a row of any other table.
pub fn first_model(
    browser: &mut Browser,
    session: &EditSession,
    table: &str,
    record: usize,
) -> Option<String> {
    use vale_assets::tables::spell::fields;
    let named = |id: &u32| *id != 0 && *id != u32::MAX;
    match table {
        "SpellVisualEffectName" => session
            .table(table)?
            .string_at(record, fields::EFFECT_MODEL)
            .filter(|path| !path.is_empty()),
        "SpellVisualKit" => {
            let kits = session.table(table)?;
            let effects: Vec<u32> = KIT_MODELS
                .iter()
                .filter_map(|&field| kits.u32_at(record, field))
                .filter(named)
                .collect();
            effects.into_iter().find_map(|effect| {
                let row = browser.record_of(session, "SpellVisualEffectName", effect)?;
                first_model(browser, session, "SpellVisualEffectName", row)
            })
        }
        "SpellVisual" => {
            let visuals = session.table(table)?;
            let kits: Vec<u32> = VISUAL_SLOTS
                .iter()
                .filter_map(|&(field, _)| visuals.u32_at(record, field))
                .filter(named)
                .collect();
            let of_a_kit = kits.into_iter().find_map(|kit| {
                let row = browser.record_of(session, "SpellVisualKit", kit)?;
                first_model(browser, session, "SpellVisualKit", row)
            });
            if of_a_kit.is_some() {
                return of_a_kit;
            }
            [fields::MISSILE_MODEL, fields::AREA_MODEL]
                .into_iter()
                .filter_map(|field| visuals.u32_at(record, field))
                .filter(named)
                .find_map(|effect| {
                    let row = browser.record_of(session, "SpellVisualEffectName", effect)?;
                    first_model(browser, session, "SpellVisualEffectName", row)
                })
        }
        _ => None,
    }
}

/// What a row of the spell chain is found by in a search and is not labelled
/// with: every model a kit hangs, by its effect's name and its file's name,
/// and for a visual the models of each of its kits, its missile and its area,
/// and the name of every spell that uses it.
///
/// A label has room for one model and one spell. A person looking for a kit
/// remembers a model, such as a file with `frost` in its name, and the kit
/// that hangs it third is the one they want as often as the kit that hangs
/// it first.
fn search_words(
    browser: &mut Browser,
    session: &EditSession,
    table: &str,
    record: usize,
) -> String {
    use vale_assets::tables::spell::fields;
    let mut words = String::new();
    match table {
        "SpellVisualKit" => kit_words(browser, session, &mut words, record),
        "SpellVisual" => {
            let Some(visuals) = session.table(table) else {
                return words;
            };
            for &(field, _) in &VISUAL_SLOTS {
                let row = visuals
                    .u32_at(record, field)
                    .filter(|kit| *kit != 0 && *kit != u32::MAX)
                    .and_then(|kit| browser.record_of(session, "SpellVisualKit", kit));
                if let Some(row) = row {
                    kit_words(browser, session, &mut words, row);
                }
            }
            for field in [fields::MISSILE_MODEL, fields::AREA_MODEL] {
                if let Some(effect) = visuals.u32_at(record, field) {
                    effect_words(browser, session, &mut words, effect);
                }
            }
            // Each name once: the nine ranks of a spell share a visual.
            let id = visuals.u32_at(record, 0).unwrap_or_default();
            let mut seen: Vec<String> = Vec::new();
            for at in browser.used_by(session, "SpellVisual", id) {
                if at.table != "Spell" {
                    continue;
                }
                let name = session
                    .table("Spell")
                    .and_then(|spells| {
                        spells.string_at(
                            at.record,
                            vale_assets::tables::spellbook::spell_fields::NAME,
                        )
                    })
                    .unwrap_or_default();
                if !name.is_empty() && !seen.contains(&name) {
                    words.push(' ');
                    words.push_str(&name);
                    seen.push(name);
                }
            }
        }
        _ => {}
    }
    words
}

/// Add the name and the model file's name of every effect a kit hangs.
fn kit_words(browser: &mut Browser, session: &EditSession, words: &mut String, record: usize) {
    let Some(kits) = session.table("SpellVisualKit") else {
        return;
    };
    for &field in &KIT_MODELS {
        if let Some(effect) = kits.u32_at(record, field) {
            effect_words(browser, session, words, effect);
        }
    }
}

/// Add an effect's name and its model file's name. An id that is no row,
/// which includes the two values for none, adds nothing.
fn effect_words(browser: &mut Browser, session: &EditSession, words: &mut String, effect: u32) {
    use vale_assets::tables::spell::fields;
    let Some(names) = session.table("SpellVisualEffectName") else {
        return;
    };
    let Some(row) = browser.record_of(session, "SpellVisualEffectName", effect) else {
        return;
    };
    for field in [fields::EFFECT_NAME, fields::EFFECT_MODEL] {
        let text = names.string_at(row, field).unwrap_or_default();
        words.push(' ');
        words.push_str(basename(&text));
    }
}

/// The kit column that names an effect, if exactly one kit column does.
///
/// Returns `(record, field)` of the `SpellVisualKit` row and the kit's own id.
/// The attachment an effect hangs from is not a field of the effect or of the
/// kit: it is determined by which column of the kit names the effect, and
/// `spell::fields::EFFECTS` is that mapping. Moving an effect to the head is
/// therefore a move between two columns of one row.
///
/// Returns `None` when no kit names the effect, or when more than one does.
/// In the second case a move would change several spells, and the choice of
/// which is left to the user.
pub fn effect_slot(
    browser: &mut Browser,
    session: &EditSession,
    effect: u32,
) -> Option<(usize, usize, u32)> {
    let mut found: Option<(usize, usize, u32)> = None;
    for at in browser.used_by(session, "SpellVisualEffectName", effect) {
        if at.table != "SpellVisualKit" {
            continue;
        }
        if found.is_some() {
            return None;
        }
        let id = session.table("SpellVisualKit")?.u32_at(at.record, 0)?;
        found = Some((at.record, at.field, id));
    }
    found
}

/// The kit column that hangs a model from this attachment point, if there is
/// one. Only the six points in [`vale_assets::tables::spell::fields::EFFECTS`]
/// have a column. Every other point a body carries can hold a model in a
/// preview, but no column names it.
pub fn slot_for_point(point: u32) -> Option<usize> {
    vale_assets::tables::spell::fields::EFFECTS
        .iter()
        .find(|(_, at)| *at == point)
        .map(|(field, _)| *field)
}

/// The attachment point a kit column hangs its model from, if it has one.
pub fn point_for_slot(field: usize) -> Option<u32> {
    vale_assets::tables::spell::fields::EFFECTS
        .iter()
        .find(|(column, _)| *column == field)
        .map(|(_, point)| *point)
}

/// Move an effect from one column of a kit to another, which changes its
/// attachment point. One entry on the undo stack.
///
/// The old column is written with `-1` rather than 0. Both mean no effect
/// (see `SpellVisuals::build`, "the table's two ways of saying nothing"), and
/// `-1` is what the shipped rows and [`blank_defaults`] use.
///
/// Returns `false` when the two columns are the same or the table is not open.
pub fn move_effect_slot(
    session: &mut EditSession,
    record: usize,
    from: usize,
    to: usize,
    effect: u32,
) -> bool {
    if from == to {
        return false;
    }
    let Some(table) = session.tables.get_mut("SpellVisualKit") else {
        return false;
    };
    let Some(clear) = Cell::new(table, record, from, u32::MAX) else {
        return false;
    };
    let Some(set) = Cell::new(table, record, to, effect) else {
        return false;
    };
    clear.apply(table);
    set.apply(table);
    session.history.begin("Move effect to another attachment");
    session.history.record_cell("SpellVisualKit", clear);
    session.history.record_cell("SpellVisualKit", set);
    session.history.end();
    session.table_edited("SpellVisualKit");
    true
}

/// Where an effect is hung, from its use in the tables: a sentence for a card,
/// and the attachment ids to try in order.
///
/// A kit column's attachment is the pairing in `spell::fields::EFFECTS`, which
/// is the point the client hangs the model from. A missile model is aimed at
/// the chest, and an area model stands at the feet. An effect that nothing
/// names defaults to the chest, and the sentence says so.
pub fn effect_anchor(
    browser: &mut Browser,
    session: &EditSession,
    effect: u32,
) -> (String, &'static [u32]) {
    use vale_assets::world::m2::attach;
    const CHEST: &[u32] = &[attach::CHEST, 15];
    const FEET: &[u32] = &[attach::BASE, 0];
    const HEAD: &[u32] = &[attach::HEAD, attach::HELM];
    const LEFT: &[u32] = &[attach::SPELL_HAND_LEFT, attach::HAND_LEFT];
    const RIGHT: &[u32] = &[attach::SPELL_HAND_RIGHT, attach::HAND_RIGHT];
    const BREATH: &[u32] = &[17, attach::HEAD];
    let uses = browser.used_by(session, "SpellVisualEffectName", effect);
    for at in &uses {
        if at.table == "SpellVisualKit" {
            let kit = session
                .table("SpellVisualKit")
                .and_then(|kits| kits.u32_at(at.record, 0))
                .unwrap_or(0);
            let column = schema::for_table("SpellVisualKit")
                .and_then(|schema| schema.column(at.field))
                .map(|column| column.name)
                .unwrap_or("a");
            let ids: &'static [u32] = match at.field {
                3 => HEAD,
                4 => CHEST,
                5 | 12 => FEET,
                6 => LEFT,
                7 => RIGHT,
                8 => BREATH,
                _ => CHEST,
            };
            return (
                format!(
                    "Anchor: this effect is the {column} of kit {kit}, so the point defaulted to match. \
                     The exported model bakes only the offset, so in game it lands the same way relative \
                     to that attachment."
                ),
                ids,
            );
        }
    }
    for at in &uses {
        if at.table == "SpellVisual" {
            let visual = session
                .table("SpellVisual")
                .and_then(|visuals| visuals.u32_at(at.record, 0))
                .unwrap_or(0);
            let (word, ids): (&str, &'static [u32]) = match at.field {
                vale_assets::tables::spell::fields::MISSILE_MODEL => ("missile model", CHEST),
                _ => ("area model", FEET),
            };
            return (
                format!(
                    "Anchor: this effect is the {word} of visual {visual}, so the point defaulted to match. \
                     The exported model bakes only the offset."
                ),
                ids,
            );
        }
    }
    (
        "No kit or visual names this effect yet, so the point defaulted to the chest. The exported model \
         bakes only the offset relative to the chosen point."
            .to_string(),
        CHEST,
    )
}

/// Play the sounds the panels queued, through the client's own voices; see
/// [`Browser::audition`].
fn audition(
    browser: Option<ResMut<Browser>>,
    assets: Res<GameAssets>,
    mut voices: vale_client::sound::mixer::Voices,
) {
    let Some(mut browser) = browser else { return };
    if browser.audition.is_empty() {
        return;
    }
    let bank = assets.sounds();
    for id in browser.audition.drain(..) {
        voices.play(&bank, id, vale_client::sound::mixer::Place::Flat);
    }
}

/// One sound a panel can play and then stop: a row of `SoundEntries`,
/// `ZoneMusic`, `ZoneIntroMusicTable` or `SoundAmbience`, and for a row with
/// a day and a night sound, which of the two.
///
/// A kit's sound is short and [`Browser::audition`] plays it to its end. A
/// zone's music is a track of minutes and its ambience is a loop, so these
/// are played one at a time and can be stopped: a second play replaces the
/// first, and the button that started one stops it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heard {
    pub table: &'static str,
    pub id: u32,
    pub night: bool,
}

/// A request to [`listen`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Listen {
    /// Play the `SoundEntries` row `entry` on `channel`, as `heard`. The
    /// ambience channel loops, as the game loops it; the other two play once.
    Play {
        heard: Heard,
        entry: u32,
        channel: vale_client::sound::mixer::Channel,
    },
    Stop,
}

/// How long a stopped sound takes to fade, which is short enough to be a stop
/// and long enough not to click.
const STOP_SECS: f32 = 0.25;

/// Play or stop what a panel asked for in [`Browser::listen`], through the
/// client's mixer and at its channel volumes.
///
/// The sound stops by itself when the browser's tools are left or a
/// playtest begins, since a looping ambience has no other end.
fn listen(
    browser: Option<ResMut<Browser>>,
    tool: Res<super::Tool>,
    playtest: Res<crate::playtest::Playtest>,
    assets: Res<GameAssets>,
    mut voices: vale_client::sound::mixer::Voices,
    alive: Query<(), With<bevy::audio::AudioPlayer>>,
    // The voice and the volume it plays at, which a fade starts from.
    mut playing: Local<Option<(Entity, f32)>>,
) {
    use vale_client::sound::mixer::{Channel, Place};
    let Some(mut browser) = browser else { return };
    if let Some((entity, _)) = *playing {
        if alive.get(entity).is_err() {
            *playing = None;
            browser.hearing = None;
        }
    }
    let away = tool.table().is_none() || !playtest.editing();
    let ask = match away {
        true => {
            browser.listen = None;
            playing.is_some().then_some(Listen::Stop)
        }
        false => browser.listen.take(),
    };
    let Some(ask) = ask else { return };
    if let Some((entity, volume)) = playing.take() {
        voices.fade_out(entity, volume, STOP_SECS);
    }
    browser.hearing = None;
    let Listen::Play {
        heard,
        entry,
        channel,
    } = ask
    else {
        return;
    };
    let bank = assets.sounds();
    let Some((path, volume)) = voices.pick(&bank, entry) else {
        return;
    };
    let settings = match channel {
        Channel::Ambience => bevy::audio::PlaybackSettings::LOOP,
        _ => bevy::audio::PlaybackSettings::DESPAWN,
    };
    if let Some(entity) = voices.play_file(&path, volume, channel, Place::Flat, settings) {
        *playing = Some((entity, volume * voices.gain(channel)));
        browser.hearing = Some(heard);
    }
}

/// Open the tables the chosen subject needs, one table per frame.
///
/// This follows the same pattern as [`super::open_tiles`]. `Spell.dbc` is
/// 16 MB, and parsing it on the frame the Spells tool is chosen causes a
/// visible stall, so each table is requested once and the panel draws whatever
/// is open.
pub fn open_tables(
    tool: Res<super::Tool>,
    assets: Res<GameAssets>,
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    browser: Option<ResMut<Browser>>,
    mut stage: ResMut<crate::stage::Stage>,
    mut lab: ResMut<crate::lab::Lab>,
    mut pointed: Local<Option<String>>,
    time: Res<Time>,
    // When `--row` opened its row, and whether `--set` has been written.
    mut scripted: Local<(Option<f64>, bool)>,
) {
    let (Some(mut session), Some(mut browser)) = (session, browser) else {
        return;
    };
    let Some(wanted) = tool.table() else {
        // Clear the stage here when no table tool is chosen. The panel sets
        // `showing`, and the panel is not drawn once the rail moves to a World
        // tool, so without this the stage stayed up, the actors stayed in the
        // world and the camera stayed locked on the storyboard's characters.
        stage.close();
        return;
    };
    // The rail's table is where the browser starts, not a table it is held on.
    // Pointing the browser at it on every frame would undo a followed
    // reference on the next frame: `SpellVisual` is opened, then the next run
    // of this system points the browser back at `Spell` and loses the row. So
    // the rail's choice is applied only when it changes, and the browser moves
    // freely after that.
    // A follow from outside the data tools arrives together with the rail's
    // change and is not undone by it; see [`Browser::followed_in`].
    //
    // The Tables workspace has no table of its own: it starts on the table
    // it last showed, or on the list of tables.
    let any = wanted == ANY;
    let start = match any {
        true => browser.any.clone(),
        false => wanted.to_string(),
    };
    let followed_in = std::mem::take(&mut browser.followed_in);
    // The back stack belongs to the workspace it was made in. Carried into
    // another, `< back` opened the last workspace's tables under this one's
    // tool: Zones showing SkillLineAbility, with no tab to leave by, since
    // Zones has one table and draws no tabs. A follow from outside pushes the
    // table the browser last showed, which is no more a way back, so it is
    // cleared too.
    if pointed.as_deref() != Some(wanted) {
        browser.back.clear();
    }
    if pointed.as_deref() != Some(wanted) && followed_in {
        *pointed = Some(wanted.to_string());
    }
    if pointed.as_deref() != Some(wanted) {
        *pointed = Some(wanted.to_string());
        // `--table` names the table `--row` refers to, and is applied once.
        //
        // It accepts any table, not only the tool's own tabs. The tabs are a
        // few of the tables; a table reached by following a reference
        // (`SpellCategory`, `SpellIcon`, `SpellRange`) has no tab but can be
        // opened, and so can one with no schema. `--table` on one of those
        // was once accepted and then ignored, which left the run on `Spell`.
        // A name that is not a table logs a warning, as `--without` does.
        match (args.table.as_deref(), browser.seeded) {
            (Some(table), false) => match table_named(table) {
                Some(name) => browser.look_at(name),
                None => {
                    warn!("--table {table}: no table by that name");
                    browser.look_at(&start);
                }
            },
            _ => browser.look_at(&start),
        }
    }
    if any {
        browser.any = browser.table.clone();
    }

    // Open the table the browser is now pointed at. On the list of tables
    // there is none.
    let open = browser.table.clone();
    if open.is_empty() {
        stage.close();
        return;
    }
    if !session.open_table(&assets, &open) {
        // A file that does not open, such as one of the four zero-byte
        // ones, returns the Tables workspace to its list. The session's
        // status line says why.
        if any {
            browser.look_at(ANY);
            browser.any.clear();
        }
        return;
    }

    // Then open the subject's root table and its chain, one per frame, so no
    // single frame waits on all of them.
    //
    // The root table is included deliberately. `chain_for` lists the tables a
    // subject references and never the subject itself, so a browser opened
    // directly on a table the root references (`--table SpellCategory`, or a
    // followed reference) left `Spell` closed. Nothing appears broken in that
    // state: the row and its fields draw, but the reverse index is built over
    // the open tables only and finds nothing, so every category read `unused`
    // while 136 of the 166 are used by 22,360 spells. The reverse index is
    // only complete when every table that references the target is open.
    //
    // In the Tables workspace the root is the open table itself.
    let root: &str = match any {
        true => &open,
        false => wanted,
    };
    if let Some(next) = std::iter::once(root)
        .chain(chain_for(root).iter().copied())
        .find(|name| !session.tables.contains_key(*name))
    {
        session.open_table(&assets, next);
        return;
    }
    browser.index(&session, &open);

    // `--row <id>`, once the table it names is here. See `crate::Args::row`.
    if let (Some(id), false) = (args.row, browser.seeded) {
        browser.seeded = true;
        if args.story {
            browser.view = crate::ui::data::View::Storyboard;
        }
        // The stage holds the seek until the actors are spawned, then pauses
        // at that time; see `Stage::seek`.
        if let Some(seek) = args.seek {
            stage.seek(seek);
        }
        match session.table(&open).and_then(|table| table.row_of(id)) {
            Some(record) => {
                browser.open = Some(record);
                // `--lab` on an effect row opens the attachment lab on it,
                // which otherwise requires a press on the form.
                if args.lab && open == "SpellVisualEffectName" {
                    crate::ui::lab::open_on(&mut lab, &mut stage, &mut browser, &session, record);
                }
                // `--bits <column>` opens the mask dialog on that column. The
                // dialog is otherwise opened by a control a scripted run
                // cannot press. A name that is not a mask column of this table
                // logs a warning instead of opening nothing, as `--without`
                // does.
                if let Some(wanted) = args.bits.as_deref() {
                    match schema::for_table(&open).and_then(|schema| {
                        schema.columns.iter().find(|column| {
                            column.name.eq_ignore_ascii_case(wanted.trim())
                                && matches!(column.kind, Kind::Flags(bits) if !bits.is_empty())
                        })
                    }) {
                        Some(column) => {
                            if let Kind::Flags(bits) = column.kind {
                                browser.modal = Some(Modal::Bits {
                                    table: open.clone(),
                                    record,
                                    field: column.field,
                                    column: column.name,
                                    bits,
                                });
                            }
                        }
                        None => warn!("--bits {wanted}: {open} has no named mask by that name"),
                    }
                }
                // `--choose <column>` opens the reference picker on that
                // column, for the same reason. The picker selects the row
                // the column holds, so one over the spell chain opens
                // playing it.
                if let Some(wanted) = args.choose.as_deref() {
                    match schema::for_table(&open).and_then(|schema| {
                        schema.columns.iter().find_map(|column| match column.kind {
                            Kind::Reference(points_at)
                                if column.name.eq_ignore_ascii_case(wanted.trim()) =>
                            {
                                Some((column.field, points_at))
                            }
                            _ => None,
                        })
                    }) {
                        Some((field, points_at)) => {
                            browser.modal = Some(Modal::Pick {
                                table: open.clone(),
                                record,
                                field,
                                points_at,
                            });
                            browser.pick_query.clear();
                            browser.pick_focus = true;
                        }
                        None => warn!("--choose {wanted}: {open} has no reference by that name"),
                    }
                }
                // `--like <spell id>` opens the dialog that takes another
                // spell's look, with that spell selected.
                if let (Some(like), true) = (args.like, open == "Spell") {
                    browser.modal = Some(Modal::LooksLike { record });
                    browser.pick_query.clear();
                    browser.pick_selected = Some(like);
                }
                // `--browse` opens the model browser over the row. It is
                // otherwise two presses in, so a scripted `--shot` could not
                // reach it.
                if args.browse && open == "SpellVisualEffectName" {
                    browser.modal = Some(Modal::Models {
                        record,
                        field: vale_assets::tables::spell::fields::EFFECT_MODEL,
                    });
                    browser.model_query.clear();
                    browser.pick_focus = true;
                }
            }
            None => session.status = format!("{open} has no row {id}"),
        }
    }

    // `--set <column>=<value>`, once the row has been open for
    // [`SET_AFTER`] seconds: long enough for a preview beside the form to be
    // playing, so the write is an edit made under a standing preview.
    /// How long after the row opens `--set` writes its field.
    const SET_AFTER: f64 = 6.0;
    if let (Some((wanted, value)), Some(record), false) =
        (args.set.as_ref(), browser.open, scripted.1)
    {
        let now = time.elapsed_secs_f64();
        let since = *scripted.0.get_or_insert(now);
        if browser.seeded && now - since >= SET_AFTER {
            scripted.1 = true;
            let column = schema::for_table(&open).and_then(|schema| {
                schema
                    .columns
                    .iter()
                    .find(|column| column.name.eq_ignore_ascii_case(wanted))
            });
            match column {
                Some(column) => {
                    set_field(
                        &mut session,
                        &open,
                        record,
                        column.field,
                        *value,
                        &format!("Edit {}", column.name),
                        now,
                    );
                }
                None => warn!("--set {wanted}: {open} has no column by that name"),
            }
        }
    }
}

pub struct TableToolPlugin;

impl Plugin for TableToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Browser>()
            .init_resource::<crate::ui::storyboard::Storyboard>()
            .add_systems(Update, (open_tables, audition, listen));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of the six kit columns round-trips through its point. The
    /// lab's drop-down depends on this: it chooses a point, and
    /// `slot_for_point` must return the column. A point that returned `None`
    /// would make a choice that changed nothing, which is the fault the
    /// drop-down was reported with.
    #[test]
    fn the_six_kit_columns_and_their_points_agree_both_ways() {
        use vale_assets::tables::spell::fields::EFFECTS;
        use vale_assets::world::m2::attach;
        for (field, point) in EFFECTS {
            assert_eq!(point_for_slot(field), Some(point), "column {field}");
            assert!(slot_for_point(point).is_some(), "point {point}");
        }
        // The five named points, listed explicitly, so a reordering of the
        // table fails here instead of moving an effect to the wrong limb.
        assert_eq!(slot_for_point(attach::HEAD), Some(3));
        assert_eq!(slot_for_point(attach::CHEST), Some(4));
        assert_eq!(slot_for_point(attach::SPELL_HAND_LEFT), Some(6));
        assert_eq!(slot_for_point(attach::SPELL_HAND_RIGHT), Some(7));
        // The base point has two columns, `BaseEffect` and `GroundEffect`, and
        // the point maps to the first. For that reason `point_combo` refuses
        // a move whose source column already hangs from the chosen point;
        // otherwise choosing the base for a ground effect would move it from
        // column 12 to column 5 with no change requested.
        assert_eq!(slot_for_point(attach::BASE), Some(5));
        assert_eq!(point_for_slot(12), Some(attach::BASE));
        // A point that no column names has no slot. That is most of the
        // thirty points a body carries.
        assert_eq!(slot_for_point(attach::HAND_LEFT), None);
        assert_eq!(slot_for_point(attach::HELM), None);
        assert_eq!(point_for_slot(2), None);
    }

    #[test]
    fn a_basename_is_the_last_component_whichever_way_the_slashes_go() {
        assert_eq!(
            basename("Spells\\Fireball_Cast_Hand.mdx"),
            "Fireball_Cast_Hand.mdx"
        );
        assert_eq!(
            basename("Interface/Icons/Spell_Fire_FlameBolt"),
            "Spell_Fire_FlameBolt"
        );
        assert_eq!(basename("bare"), "bare");
    }

    /// Which column of a kit hangs from which attachment, checked by name
    /// rather than by position in the table this reads.
    ///
    /// A wrong mapping writes an effect into the wrong column and nothing
    /// reports it: the row is valid, the spell still casts, and the model
    /// hangs off the wrong part of the body. The six columns are
    /// `SpellVisualKit`'s `HeadEffect`, `ChestEffect`, `BaseEffect`,
    /// `LeftHandEffect`, `RightHandEffect` and the second base column.
    #[test]
    fn a_point_maps_to_the_kit_column_that_hangs_from_it() {
        use vale_assets::world::m2::attach;
        assert_eq!(slot_for_point(attach::HEAD), Some(3));
        assert_eq!(slot_for_point(attach::CHEST), Some(4));
        assert_eq!(slot_for_point(attach::SPELL_HAND_LEFT), Some(6));
        assert_eq!(slot_for_point(attach::SPELL_HAND_RIGHT), Some(7));
        // Two columns hang from the feet and the first is the one written, so
        // a move to the ground always targets the same column.
        assert_eq!(slot_for_point(attach::BASE), Some(5));
        assert_eq!(point_for_slot(12), Some(attach::BASE));

        // Every other point a body carries is used only by the preview.
        assert_eq!(slot_for_point(attach::HELM), None);
        assert_eq!(
            point_for_slot(2),
            None,
            "the animation column hangs nothing"
        );
        assert_eq!(point_for_slot(0), None, "nor does the id");
    }

    /// Every column the mapping offers is one the client reads, and every
    /// point it offers maps back to a column, so the drop-down cannot offer a
    /// move that writes to a column the client ignores.
    #[test]
    fn every_mapped_column_is_one_the_client_reads() {
        for (field, point) in vale_assets::tables::spell::fields::EFFECTS {
            assert_eq!(point_for_slot(field), Some(point), "column {field}");
            let back = slot_for_point(point).expect("a point with a column");
            assert_eq!(
                point_for_slot(back),
                Some(point),
                "point {point} maps to column {back}, which must hang from it"
            );
        }
    }

    /// An empty table of `fields` columns, for a test to add rows to.
    fn empty(fields: u32) -> vale_edit::dbc::DbcFile {
        let mut bytes = b"WDBC".to_vec();
        for word in [0u32, fields, fields * 4, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0);
        vale_edit::dbc::DbcFile::parse(&bytes).expect("an empty table")
    }

    /// Add a row with `id`, the numbers in `set` and the strings in `text`.
    fn add(table: &mut vale_edit::dbc::DbcFile, id: u32, set: &[(usize, u32)], text: &[(usize, &str)]) {
        let blank = table.blank_record(id);
        let record = table.push_record(&blank).expect("a record");
        for &(field, value) in set {
            table.set_u32(record, field, value);
        }
        for &(field, value) in text {
            table.set_string(record, field, value);
        }
    }

    /// A session with no archives, in a project folder of its own.
    fn session(name: &str) -> (EditSession, std::path::PathBuf) {
        let install =
            std::env::temp_dir().join(format!("vale-tables-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = vale_edit::project::Project::open(&install, "default").unwrap();
        (EditSession::for_tests(project), install)
    }

    /// Undo the last entry on every table it names.
    fn undo(session: &mut EditSession) {
        let change = session.history.undo().expect("an entry");
        for name in change.tables() {
            if let Some(table) = session.tables.get_mut(&name) {
                change.revert_table(&name, table);
            }
        }
        session.table_revision += 1;
    }

    /// A spell is put in a skill line by a new ability row naming it, found
    /// again through the reverse index, as one entry that one undo takes out.
    /// A spell with no row has none, which is the common case.
    #[test]
    fn a_spell_joins_a_skill_line_through_a_row_of_its_own() {
        let (mut session, install) = session("ability");
        let mut spells = empty(173);
        add(&mut spells, 133, &[], &[]);
        add(&mut spells, 143, &[], &[]);
        let mut abilities = empty(15);
        add(&mut abilities, 69, &[(ability::SKILL, 6), (ability::SPELL, 143)], &[]);
        session.tables.insert("Spell".to_string(), spells);
        session.tables.insert(ability::TABLE.to_string(), abilities);
        session.tables.insert("SkillLine".to_string(), empty(22));
        let mut browser = Browser::default();
        assert!(abilities_of(&mut browser, &session, 133).is_empty());
        assert_eq!(abilities_of(&mut browser, &session, 143), vec![0]);

        let depth = session.history.depth_done();
        let at = add_ability(&mut session, 133).expect("a row");
        assert_eq!(session.history.depth_done(), depth + 1);
        let table = &session.tables[ability::TABLE];
        assert_eq!(table.u32_at(at, 0), Some(70), "the next id");
        assert_eq!(table.u32_at(at, ability::SPELL), Some(133));
        assert_eq!(table.u32_at(at, ability::SKILL), Some(0), "the line is chosen after");
        assert_eq!(table.u32_at(at, ability::REQ_SKILL_VALUE), Some(1));
        assert_eq!(abilities_of(&mut browser, &session, 133), vec![at]);

        undo(&mut session);
        assert_eq!(session.tables[ability::TABLE].record_count(), 1);
        assert!(abilities_of(&mut browser, &session, 133).is_empty());
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A teaching spell is a new spell whose first effect is Learn Spell and
    /// which names the taught spell, with that spell's name, rank and icon
    /// and an instant cast. It is found as the spell's teacher, it is one
    /// entry, and a spell that only has a teacher with a cast time is not
    /// counted as having an instant one.
    #[test]
    fn a_teaching_spell_is_made_for_a_spell_and_found_again() {
        use vale_assets::tables::spellbook::spell_fields;
        let (mut session, install) = session("teach");
        let mut spells = empty(173);
        add(
            &mut spells,
            143,
            &[(spell_fields::ICON_ID, 185)],
            &[(spell_fields::NAME, "Fireball"), (spell_fields::RANK, "Rank 2")],
        );
        // A book's teaching spell: Learn Spell with a cast time.
        add(
            &mut spells,
            483,
            &[
                (spell_fields::EFFECT, vale_mangos::trainer::LEARN_SPELL),
                (spell_fields::EFFECT_TRIGGER_SPELL, 143),
                (spell_fields::CASTING_TIME_INDEX, 14),
            ],
            &[],
        );
        // A spell that triggers 143 without teaching it is not a teacher.
        add(&mut spells, 500, &[(spell_fields::EFFECT, 64), (spell_fields::EFFECT_TRIGGER_SPELL, 143)], &[]);
        session.tables.insert("Spell".to_string(), spells);
        let mut browser = Browser::default();
        assert_eq!(teachers_of(&mut browser, &session, 143), vec![(1, false)]);
        assert!(teachers_of(&mut browser, &session, 500).is_empty());

        let depth = session.history.depth_done();
        let id = add_teaching_spell(&mut session, 143).expect("a spell");
        assert_eq!(id, 501, "the next id");
        assert_eq!(session.history.depth_done(), depth + 1);
        let table = &session.tables["Spell"];
        let at = table.row_of(id).unwrap();
        assert_eq!(table.u32_at(at, spell_fields::EFFECT), Some(vale_mangos::trainer::LEARN_SPELL));
        assert_eq!(table.u32_at(at, spell_fields::EFFECT_TRIGGER_SPELL), Some(143));
        assert_eq!(table.u32_at(at, spell_fields::CASTING_TIME_INDEX), Some(INSTANT_CAST));
        assert_eq!(table.u32_at(at, spell_fields::ICON_ID), Some(185));
        assert_eq!(table.string_at(at, spell_fields::NAME).as_deref(), Some("Fireball"));
        assert_eq!(table.string_at(at, spell_fields::RANK).as_deref(), Some("Rank 2"));
        assert_eq!(table.f32_at(at, 167), Some(1.0));
        assert_eq!(teachers_of(&mut browser, &session, 143), vec![(1, false), (at, true)]);
        // What `services::teaching_spell` answers for a trainer's list is the
        // instant one.
        assert_eq!(super::super::services::teaching_spell(table, 143), Some(id));
        assert_eq!(add_teaching_spell(&mut session, 9999), None, "no such spell");

        undo(&mut session);
        assert_eq!(session.tables["Spell"].record_count(), 3);
        assert_eq!(teachers_of(&mut browser, &session, 143), vec![(1, false)]);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A new light copies its five weather rows from the template light,
    /// stands where it was put in the table's own units, and is one undo
    /// entry. A default light writes no position.
    #[test]
    fn a_new_light_copies_its_weather_and_is_placed_in_the_tables_units() {
        use vale_assets::tables::light::light_field as lf;
        let (mut session, install) = session("lights");
        let mut lights = empty(12);
        add(&mut lights, 1, &[(lf::MAP, 0), (lf::PARAMS_CLEAR, 11), (lf::PARAMS_CLEAR + 4, 15)], &[]);
        session.tables.insert("Light".to_string(), lights);

        let at = add_light(&mut session, 0, Some([100.0, 200.0, 30.0]), (50.0, 120.0), Some(0)).expect("a light");
        let table = &session.tables["Light"];
        assert_eq!(table.u32_at(at, 0), Some(2));
        assert_eq!(table.u32_at(at, lf::PARAMS_CLEAR), Some(11));
        assert_eq!(table.u32_at(at, lf::PARAMS_CLEAR + 4), Some(15));
        let float = |field| table.u32_at(at, field).map(f32::from_bits);
        let placement = vale_assets::world::adt::placement_from_world([100.0, 200.0, 30.0]);
        assert_eq!(float(lf::INTERNAL_Y), Some(30.0 * 36.0));
        assert_eq!(float(lf::INTERNAL_X), Some(placement[0] * 36.0));
        assert_eq!(float(lf::FALLOFF_END), Some(120.0 * 36.0));

        let default = add_light(&mut session, 1, None, (50.0, 120.0), Some(0)).expect("a light");
        let table = &session.tables["Light"];
        assert_eq!(table.u32_at(default, lf::MAP), Some(1));
        assert_eq!(table.u32_at(default, lf::FALLOFF_END), Some(0));
        assert_eq!(table.u32_at(default, lf::PARAMS_CLEAR), Some(11));

        undo(&mut session);
        undo(&mut session);
        assert_eq!(session.tables["Light"].record_count(), 1);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A new zone has no parent, is on the map asked for and takes an explore
    /// bit no row holds. A new sub-area names its zone, is on the zone's map,
    /// carries the zone's flags and music, and takes the next bit after that.
    /// Asked of a sub-area, a sibling is made. Each is one undo entry.
    #[test]
    fn a_zone_and_its_sub_areas_are_rows_with_their_own_explore_bits() {
        let (mut session, install) = session("areas");
        let mut areas = empty(25);
        add(
            &mut areas,
            12,
            &[(area::MAP, 0), (area::EXPLORE_BIT, 126), (area::FLAGS, 0x40), (7, 35), (8, 1)],
            &[(area::NAME, "Elwynn Forest")],
        );
        add(
            &mut areas,
            9,
            &[(area::MAP, 0), (area::PARENT, 12), (area::EXPLORE_BIT, 125)],
            &[(area::NAME, "Northshire Valley")],
        );
        session.tables.insert(area::TABLE.to_string(), areas);
        let mut browser = Browser::default();
        assert_eq!(next_explore_bit(&session.tables[area::TABLE]), 127);
        assert_eq!(sub_areas_of(&mut browser, &session, 12), vec![1]);

        let zone = add_zone(&mut session, 1).expect("a zone");
        let table = &session.tables[area::TABLE];
        assert_eq!(table.u32_at(zone, area::MAP), Some(1));
        assert_eq!(table.u32_at(zone, area::PARENT), Some(0));
        assert_eq!(table.u32_at(zone, area::EXPLORE_BIT), Some(127));
        assert_eq!(table.u32_at(zone, area::FLAGS), Some(area::DUELS));
        assert_eq!(table.string_at(zone, area::NAME).as_deref(), Some(area::NEW_ZONE));

        let inside = add_sub_area(&mut session, 12).expect("a sub-area");
        let table = &session.tables[area::TABLE];
        assert_eq!(table.u32_at(inside, area::PARENT), Some(12));
        assert_eq!(table.u32_at(inside, area::MAP), Some(0));
        assert_eq!(table.u32_at(inside, area::EXPLORE_BIT), Some(128));
        assert_eq!(table.u32_at(inside, 7), Some(35), "the zone's ambience");
        assert_eq!(table.u32_at(inside, 8), Some(1), "the zone's music");
        let sibling = add_sub_area(&mut session, 9).expect("a sibling");
        assert_eq!(session.tables[area::TABLE].u32_at(sibling, area::PARENT), Some(12));
        assert_eq!(sub_areas_of(&mut browser, &session, 12), vec![1, inside, sibling]);
        assert_eq!(session.history.depth_done(), 3);

        // A sub-area is offered to a zone and not to a sub-area.
        let label = |browser: &mut Browser, record| {
            command_label(browser, &session, area::TABLE, record, Command::AddSubArea)
        };
        assert!(label(&mut browser, 0).is_some());
        assert!(label(&mut browser, 1).is_none());
        assert!(commands(area::TABLE).contains(&Command::PaintArea));
        // The label says which of the two a row is.
        assert_eq!(describe(&mut browser, &session, area::TABLE, 0).sub, "zone \u{b7} map 0");
        assert_eq!(
            describe(&mut browser, &session, area::TABLE, 1).sub,
            "in Elwynn Forest \u{b7} map 0"
        );
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A skill line is given to races and classes by a row naming it, for
    /// everyone by default.
    #[test]
    fn a_skill_line_is_given_to_races_and_classes_by_a_row() {
        let (mut session, install) = session("raceclass");
        session.tables.insert("SkillLine".to_string(), empty(22));
        session.tables.insert(race_class::TABLE.to_string(), empty(8));
        let mut browser = Browser::default();
        assert!(race_class_rows_of(&mut browser, &session, 8).is_empty());
        let at = add_race_class_row(&mut session, 8).expect("a row");
        let table = &session.tables[race_class::TABLE];
        assert_eq!(table.u32_at(at, race_class::SKILL), Some(8));
        assert_eq!(table.u32_at(at, race_class::RACES), Some(race_class::EVERY_RACE));
        assert_eq!(table.u32_at(at, race_class::CLASSES), Some(race_class::EVERY_CLASS));
        assert_eq!(table.u32_at(at, race_class::FLAGS), Some(0));
        assert_eq!(race_class_rows_of(&mut browser, &session, 8), vec![at]);
        assert_eq!(session.history.depth_done(), 1);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// An item moved between sets leaves every column of the old set that
    /// lists it and takes the first empty column of the new one, as one
    /// entry. A set that lists it already is left alone, and a set with no
    /// empty column refuses and says so.
    #[test]
    fn an_item_moves_between_two_sets_item_lists() {
        let (mut session, install) = session("sets");
        let mut sets = empty(45);
        add(&mut sets, 1, &[(10, 11729), (11, 11726)], &[(1, "The Gladiator")]);
        add(&mut sets, 41, &[(10, 12940)], &[(1, "Dal'Rend's Arms")]);
        let full: Vec<(usize, u32)> = SET_ITEMS.map(|field| (field, 900 + field as u32)).collect();
        add(&mut sets, 65, &full, &[(1, "A full set")]);
        session.tables.insert("ItemSet".to_string(), sets);
        let listed = |session: &EditSession, set: u32, item: u32| {
            let sets = &session.tables["ItemSet"];
            set_lists(sets, sets.row_of(set).unwrap(), item)
        };

        let said = move_between_sets(&mut session, 11726, 1, 41, "item 11726 set_id", 0.0);
        assert_eq!(said.len(), 2, "{said:?}");
        assert_eq!(listed(&session, 1, 11726), None);
        assert_eq!(listed(&session, 41, 11726), Some(11), "the first empty column");
        assert_eq!(listed(&session, 1, 11729), Some(10), "the other piece stays");
        assert_eq!(session.history.depth_done(), 1);

        // Already listed, or no change of set: nothing is written.
        assert!(move_between_sets(&mut session, 11726, 0, 41, "item 11726 set_id", 10.0).is_empty());
        assert!(move_between_sets(&mut session, 11726, 41, 41, "item 11726 set_id", 10.0).is_empty());
        assert_eq!(session.history.depth_done(), 1);

        let said = move_between_sets(&mut session, 11729, 0, 65, "item 11729 set_id", 20.0);
        assert!(said[0].contains("no empty item column"), "{said:?}");
        let said = move_between_sets(&mut session, 11729, 0, 777, "item 11729 set_id", 20.0);
        assert!(said[0].contains("no row 777"), "{said:?}");
        assert_eq!(session.history.depth_done(), 1);

        undo(&mut session);
        assert_eq!(listed(&session, 1, 11726), Some(11));
        assert_eq!(listed(&session, 41, 11726), None);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// The skill tables and the item sets are labelled from the rows they
    /// name: an ability by its spell, its skill line and its classes, a
    /// race-and-class row by its skill line and who gets it, a set by its
    /// name and how many of its item and bonus columns are used.
    #[test]
    fn the_skill_tables_and_the_sets_are_labelled_from_what_they_name() {
        use vale_assets::tables::spellbook::spell_fields;
        let (mut session, install) = session("labels");
        let mut spells = empty(173);
        add(&mut spells, 116, &[], &[(spell_fields::NAME, "Frostbolt"), (spell_fields::RANK, "Rank 1")]);
        add(&mut spells, 133, &[], &[(spell_fields::NAME, "Fireball"), (spell_fields::RANK, "Rank 1")]);
        let mut lines = empty(22);
        add(&mut lines, 6, &[(1, 7)], &[(3, "Frost")]);
        add(&mut lines, 8, &[(1, 7)], &[(3, "Fire")]);
        let mut categories = empty(11);
        add(&mut categories, 7, &[], &[(1, "Class Skills")]);
        let mut abilities = empty(15);
        add(&mut abilities, 69, &[(1, 6), (2, 116), (4, 128)], &[]);
        add(&mut abilities, 70, &[(1, 8), (2, 999)], &[]);
        let mut infos = empty(8);
        add(&mut infos, 57, &[(1, 6), (2, 511), (3, 128)], &[]);
        add(&mut infos, 58, &[(1, 8), (2, 511), (3, 1503)], &[]);
        let mut sets = empty(45);
        add(
            &mut sets,
            1,
            &[(10, 11729), (11, 11726), (12, 11728), (27, 7514), (28, 9761), (35, 3), (36, 2)],
            &[(1, "The Gladiator")],
        );
        for (name, table) in [
            ("Spell", spells),
            ("SkillLine", lines),
            ("SkillLineCategory", categories),
            ("SkillLineAbility", abilities),
            ("SkillRaceClassInfo", infos),
            ("ItemSet", sets),
        ] {
            session.tables.insert(name.to_string(), table);
        }
        let mut browser = Browser::default();
        let label = |browser: &mut Browser, table: &str, record: usize| {
            let label = describe(browser, &session, table, record);
            (label.title, label.sub, label.id)
        };
        assert_eq!(
            label(&mut browser, "SkillLineAbility", 0),
            ("Frostbolt".to_string(), "Rank 1 · Frost · Mage".to_string(), 69)
        );
        // A spell the table does not hold is its number, and a mask of zero
        // is every class and says nothing.
        assert_eq!(
            label(&mut browser, "SkillLineAbility", 1),
            ("spell 999".to_string(), "Fire".to_string(), 70)
        );
        assert_eq!(
            label(&mut browser, "SkillLine", 1),
            ("Fire".to_string(), "Class Skills".to_string(), 8)
        );
        assert_eq!(
            label(&mut browser, "SkillRaceClassInfo", 0),
            ("Frost".to_string(), "Mage".to_string(), 57)
        );
        assert_eq!(
            label(&mut browser, "SkillRaceClassInfo", 1),
            ("Fire".to_string(), "every race and class".to_string(), 58)
        );
        assert_eq!(
            label(&mut browser, "ItemSet", 0),
            ("The Gladiator".to_string(), "3 items · 2 bonuses".to_string(), 1)
        );
        // An ability is found by its spell's name, since the index is built
        // from the labels.
        browser.look_at("SkillLineAbility");
        browser.query = "frostbolt".to_string();
        assert_eq!(browser.matches(&session), &[0]);
        // The reverse index lists the ability under its spell and its line.
        assert_eq!(browser.used_by(&session, "Spell", 116).len(), 1);
        assert_eq!(browser.used_by(&session, "SkillLine", 6).len(), 2);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A record is found by id through the index, the first of two records
    /// sharing one, and the index follows the table's revision.
    #[test]
    fn a_record_is_found_by_its_id_through_the_index() {
        let (mut session, install) = session("ids");
        let mut table = empty(2);
        add(&mut table, 7, &[(1, 1)], &[]);
        add(&mut table, 9, &[(1, 2)], &[]);
        add(&mut table, 7, &[(1, 3)], &[]);
        session.tables.insert("Lock".to_string(), table);
        let mut browser = Browser::default();
        assert_eq!(browser.record_of(&session, "Lock", 7), Some(0));
        assert_eq!(browser.record_of(&session, "Lock", 9), Some(1));
        assert_eq!(browser.record_of(&session, "Lock", 8), None);
        assert_eq!(browser.record_of(&session, "Faction", 7), None, "not open");
        let blank = session.tables["Lock"].blank_record(8);
        session.tables.get_mut("Lock").unwrap().push_record(&blank);
        session.table_revision += 1;
        assert_eq!(browser.record_of(&session, "Lock", 8), Some(3));
        let _ = std::fs::remove_dir_all(&install);
    }

    /// The name column of a table with no schema is the first whose values
    /// are the offsets of differing strings. A column of ones is the offset
    /// of the block's first string on every row and is not a name.
    #[test]
    fn a_table_with_no_schema_is_named_by_its_first_column_of_strings() {
        let mut table = empty(4);
        add(&mut table, 1, &[(1, 1), (2, 40)], &[(3, "Alpha")]);
        add(&mut table, 2, &[(1, 1), (2, 41)], &[(3, "Beta")]);
        add(&mut table, 3, &[(1, 1), (2, 42)], &[(3, "Gamma")]);
        assert_eq!(string_started_at(&table, 1).as_deref(), Some("Alpha"));
        assert_eq!(string_started_at(&table, 2), None, "inside a string");
        assert_eq!(string_started_at(&table, 0), None, "the empty string");
        assert_eq!(string_started_at(&table, 4000), None, "past the block");
        assert_eq!(guess_name_field(&table), Some(3));
        assert_eq!(guess_name_field(&empty(4)), None);
        // Field 1 holds 1 on every row, the offset of the first string, and
        // field 2 holds numbers that are no string's offset.
        assert_eq!(text_fields(&table), vec![false, false, false, true]);
        // A column that holds a string's offset on one row and another
        // number on the next is numbers.
        let mut mixed = empty(3);
        add(&mut mixed, 1, &[(1, 1)], &[(2, "Alpha")]);
        add(&mut mixed, 2, &[(1, 3)], &[(2, "Beta")]);
        assert_eq!(text_fields(&mixed), vec![false, false, true]);

        let (mut session, install) = session("guess");
        session.tables.insert("Lock".to_string(), table);
        let mut browser = Browser::default();
        assert_eq!(browser.name_field(&session, "Lock"), Some(3));
        assert_eq!(describe(&mut browser, &session, "Lock", 1).title, "Beta");
        // A described table is named by its schema, not by a guess.
        assert_eq!(browser.name_field(&session, "ItemSet"), Some(1));
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A table is opened under one name whatever case it is asked for in,
    /// and a name that is no table is refused.
    #[test]
    fn a_table_is_named_in_any_case() {
        assert_eq!(table_named("itemset"), Some("ItemSet"));
        assert_eq!(table_named("Faction"), Some("Faction"));
        assert_eq!(table_named("skilllineability"), Some("SkillLineAbility"));
        assert_eq!(table_named("creature_template"), None);
        // Every table the server reads as a file is spelled as the list
        // spells it, so a copied file and an opened table are one name.
        for name in vale_mangos::datadir::SERVER_DBCS {
            assert_eq!(table_named(name), Some(name), "{name}");
        }
        for (dbc, _) in crate::server::rows::MAPPED {
            assert_eq!(table_named(dbc), Some(dbc));
        }
    }

    /// A row with a title is `title  [id]`; one without is the id alone, which
    /// is what a table with no schema falls back to.
    #[test]
    fn a_label_line_carries_the_id_and_nothing_twice() {
        let named = RowLabel {
            title: "Fireball".into(),
            sub: "Rank 1".into(),
            id: 133,
        };
        assert_eq!(named.line(), "Fireball  [133]");
        let bare = RowLabel {
            id: 12,
            ..RowLabel::default()
        };
        assert_eq!(bare.line(), "12");
    }

    /// A table's commands are its own and then the three every row has, and
    /// a command that does not apply to a row has no label: a spell with an
    /// instant teaching spell is not offered another. A delete says how many
    /// references it leaves pointing at nothing.
    #[test]
    fn a_row_s_commands_are_its_table_s_and_are_worded_for_the_row() {
        use vale_assets::tables::spell::fields::SPELL_VISUAL;
        assert_eq!(
            commands("Spell"),
            vec![
                Command::AddToSkillLine,
                Command::CreateTeachingSpell,
                Command::LookLike,
                Command::Clone,
                Command::Delete,
                Command::CopyId,
            ]
        );
        assert_eq!(commands("SpellVisual")[0], Command::CloneChain);
        assert_eq!(commands("SkillLine")[0], Command::GiveToRacesAndClasses);
        assert_eq!(
            commands("AreaTable"),
            vec![
                Command::AddSubArea,
                Command::PaintArea,
                Command::Clone,
                Command::Delete,
                Command::CopyId,
            ]
        );
        assert_eq!(
            commands("Faction"),
            vec![Command::Clone, Command::Delete, Command::CopyId]
        );

        let (mut session, install) = session("commands");
        let mut spells = empty(173);
        add(&mut spells, 133, &[(SPELL_VISUAL, 7)], &[]);
        add(&mut spells, 143, &[(SPELL_VISUAL, 7)], &[]);
        let mut visuals = empty(15);
        add(&mut visuals, 7, &[], &[]);
        add(&mut visuals, 8, &[], &[]);
        session.tables.insert("Spell".to_string(), spells);
        session.tables.insert("SpellVisual".to_string(), visuals);
        let mut browser = Browser::default();
        let mut label = |table: &str, record: usize, command: Command| {
            command_label(&mut browser, &session, table, record, command)
        };
        assert_eq!(
            label("SpellVisual", 0, Command::Delete).as_deref(),
            Some("Delete: 2 references will point at nothing")
        );
        assert_eq!(label("SpellVisual", 1, Command::Delete).as_deref(), Some("Delete"));
        assert_eq!(label("SpellVisual", 1, Command::CopyId).as_deref(), Some("Copy id 8"));
        assert_eq!(
            label("Spell", 0, Command::CreateTeachingSpell).as_deref(),
            Some("Create a teaching spell")
        );
        assert_eq!(label("Spell", 9, Command::Clone), None, "no such row");

        let made = add_teaching_spell(&mut session, 133).expect("a teaching spell");
        assert!(session.tables["Spell"].row_of(made).is_some());
        let mut browser = Browser::default();
        assert_eq!(
            command_label(&mut browser, &session, "Spell", 0, Command::CreateTeachingSpell),
            None,
            "an instant teaching spell exists"
        );
        assert!(
            command_label(&mut browser, &session, "Spell", 1, Command::CreateTeachingSpell)
                .is_some()
        );
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A kit is pictured by the first model it hangs and a visual by its
    /// first kit's, or by its missile's when no kit has one. Both are found
    /// by the name of a model's file wherever in the kit it hangs, and a
    /// visual by the name of a spell that uses it.
    #[test]
    fn a_kit_and_a_visual_are_pictured_and_found_by_their_models() {
        use vale_assets::tables::spell::fields;
        let (mut session, install) = session("models");
        let mut effects = empty(5);
        add(
            &mut effects,
            11,
            &[],
            &[
                (fields::EFFECT_NAME, "Fire hand"),
                (fields::EFFECT_MODEL, "Spells\\Fire_Hand.mdx"),
            ],
        );
        add(
            &mut effects,
            12,
            &[],
            &[
                (fields::EFFECT_NAME, "Frost base"),
                (fields::EFFECT_MODEL, "Spells\\Frost_Nova_Base.mdx"),
            ],
        );
        let mut kits = empty(35);
        // The right hand and then the base, in the order a label reads them.
        add(&mut kits, 21, &[(7, 11), (5, 12)], &[]);
        add(&mut kits, 22, &[], &[]);
        let mut visuals = empty(15);
        add(&mut visuals, 31, &[(fields::CAST_KIT, 21)], &[]);
        add(&mut visuals, 32, &[(fields::MISSILE_MODEL, 12)], &[]);
        add(&mut visuals, 33, &[(fields::IMPACT_KIT, 22)], &[]);
        let mut spells = empty(173);
        add(
            &mut spells,
            133,
            &[(fields::SPELL_VISUAL, 31)],
            &[(vale_assets::tables::spellbook::spell_fields::NAME, "Fireball")],
        );
        session.tables.insert("SpellVisualEffectName".to_string(), effects);
        session.tables.insert("SpellVisualKit".to_string(), kits);
        session.tables.insert("SpellVisual".to_string(), visuals);
        session.tables.insert("Spell".to_string(), spells);
        let mut browser = Browser::default();

        let model = |browser: &mut Browser, table: &str, record: usize| {
            first_model(browser, &session, table, record)
        };
        assert_eq!(
            model(&mut browser, "SpellVisualEffectName", 1).as_deref(),
            Some("Spells\\Frost_Nova_Base.mdx")
        );
        assert_eq!(
            model(&mut browser, "SpellVisualKit", 0).as_deref(),
            Some("Spells\\Fire_Hand.mdx")
        );
        assert_eq!(model(&mut browser, "SpellVisualKit", 1), None);
        assert_eq!(
            model(&mut browser, "SpellVisual", 0).as_deref(),
            Some("Spells\\Fire_Hand.mdx")
        );
        assert_eq!(
            model(&mut browser, "SpellVisual", 1).as_deref(),
            Some("Spells\\Frost_Nova_Base.mdx"),
            "the missile, when no kit has a model"
        );
        assert_eq!(model(&mut browser, "SpellVisual", 2), None);
        assert_eq!(model(&mut browser, "Spell", 0), None);

        let mut found = |table: &str, query: &str| -> Vec<usize> {
            browser.look_at(table);
            browser.query = query.to_string();
            browser.matches(&session).to_vec()
        };
        // The second model of the kit, which the kit's label does not name.
        assert_eq!(found("SpellVisualKit", "frost_nova"), vec![0]);
        assert_eq!(found("SpellVisual", "fire_hand"), vec![0]);
        assert_eq!(found("SpellVisual", "frost_nova"), vec![0, 1]);
        assert_eq!(found("SpellVisual", "fireball"), vec![0]);
        assert!(found("SpellVisual", "arcane").is_empty());
        let _ = std::fs::remove_dir_all(&install);
    }
}
