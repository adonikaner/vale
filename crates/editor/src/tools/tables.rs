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
//! ## Row labels come from the rows that reference them
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
    /// Choose a row of `points_at` for the reference at `(record, field)`.
    Pick {
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
    /// Set and clear the bits of the mask at `(record, field)`.
    ///
    /// This is the third way to edit a column without typing its value.
    /// `Spell.dbc` has twelve mask columns with about 240 named bits between
    /// them, which otherwise have to be entered as hexadecimal numbers. `bits`
    /// is the column's own list: `vale_assets::tables::schema::Kind::Flags`
    /// carries it, and the names come from `vale_assets::tables::spellbits`.
    Bits {
        record: usize,
        field: usize,
        /// The column's name, for the dialog's heading. The schema is not
        /// available where the dialog is drawn.
        column: &'static str,
        bits: &'static [(u32, &'static str, &'static str)],
    },
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

    /// The label of a row by id rather than by record, for a reference column.
    /// Returns `None` when the table is not open or has no such row.
    pub fn describe_id(&mut self, session: &EditSession, table: &str, id: u32) -> Option<RowLabel> {
        let record = session.table(table)?.row_of(id)?;
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
                format!("{} {}", label.title, label.sub).to_ascii_lowercase()
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
        _ => (
            schema::for_table(table_name)
                .and_then(label_field)
                .map(text)
                .unwrap_or_default(),
            String::new(),
        ),
    };
    RowLabel { title, sub, id }
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
/// There are three chains: the spell chain, the light chain and the flight
/// path tables. A spell's form can be drawn from `Spell.dbc` alone, but its
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

/// The spell chain's tabs, in the order a cast reads them. Callers read the
/// tabs through [`super::Tool::tabs`], which chooses between this,
/// [`LIGHT_TABS`] and [`TAXI_TABS`].
pub const SPELL_TABS: [(&str, &str); 4] = [
    ("Spells", "Spell"),
    ("Visuals", "SpellVisual"),
    ("Kits", "SpellVisualKit"),
    ("Effects", "SpellVisualEffectName"),
];

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
) {
    let (Some(mut session), Some(mut browser)) = (session, browser) else {
        return;
    };
    let Some(wanted) = tool.table() else {
        // Clear the stage here when no table tool is chosen. The panel sets
        // `showing`, and the panel is not drawn once the rail moves to a World
        // tool, so without this the stage stayed up, the actors stayed in the
        // world and the camera stayed locked on the storyboard's characters.
        stage.showing = None;
        stage.lab = false;
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
    let followed_in = std::mem::take(&mut browser.followed_in);
    if pointed.as_deref() != Some(wanted) && followed_in {
        *pointed = Some(wanted.to_string());
    }
    if pointed.as_deref() != Some(wanted) {
        *pointed = Some(wanted.to_string());
        // `--table` names the table `--row` refers to, and is applied once.
        //
        // It accepts any table with a schema, not only the tool's own tabs.
        // The tabs are four of the twenty-two tables; a table reached by
        // following a reference (`SpellCategory`, `SpellIcon`, `SpellRange`)
        // has no tab but can be opened. `--table` on one of those was once
        // accepted and then ignored, which left the run on `Spell`. A name
        // that is not a table logs a warning, as `--without` does.
        match (args.table.as_deref(), browser.seeded) {
            (Some(table), false) => match schema::for_table(table) {
                Some(schema) => browser.look_at(schema.table),
                None => {
                    warn!("--table {table}: no table by that name");
                    browser.look_at(wanted);
                }
            },
            _ => browser.look_at(wanted),
        }
    }

    // Open the table the browser is now pointed at.
    let open = browser.table.clone();
    if !session.open_table(&assets, &open) {
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
    if let Some(next) = std::iter::once(&wanted)
        .chain(chain_for(wanted).iter())
        .find(|name| !session.tables.contains_key(**name))
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
}

pub struct TableToolPlugin;

impl Plugin for TableToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Browser>()
            .init_resource::<crate::ui::storyboard::Storyboard>()
            .add_systems(Update, (open_tables, audition));
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
}
