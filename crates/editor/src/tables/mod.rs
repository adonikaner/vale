//! The client's own tables: which row is open, and what an edit to a field is.
//!
//! ```text
//! browser.rs   the browser's state: the open table and row, the search, the
//!              text being typed, and the reverse references
//! labels.rs    what a row is called: `vale_assets::tables::labels`' rules,
//!              read against the session and the browser's reverse index
//! tabs.rs      the tables each workspace reaches, as tabs, and a table named
//!              in any case
//! fields.rs    writing a field of a row, as one undo entry
//! rows.rs      adding, copying and removing rows, the light chain's new rows,
//!              and an item's place in the item sets
//! learning.rs  a spell's skill line rows and teaching spell, and a skill line's
//!              race and class rows
//! zones.rs     `AreaTable`'s zones and sub-areas
//! commands.rs  what can be done to a row other than editing a field
//! visuals.rs   the spell chain's rows found and pictured by their models, and
//!              a kit's effect slots by attachment point
//! sounds.rs    playing a `SoundEntries` row for the panel
//! open.rs      opening the tables a workspace needs, one per frame
//! tests.rs     the rows each operation makes and the labels they get
//! ```
//!
//! ## Why this tool does not use the pointer
//!
//! Every file in `crate::tools` states what a left-drag in the viewport
//! does. This one does not use the pointer: a table has no place in the world,
//! so the workspace replaces the viewport. [`crate::ui::tables`] draws the
//! workspace, and the rail's Data group holds the subjects that work this way.
//!
//! The lights and the flight path tables are edited through the same functions
//! and are not in the Data group, because a `Light` row and a `TaxiNodes` row
//! each have a place in the world and are picked there; see
//! [`crate::ui::shell::rail`]. The group is decided by the surface a subject is
//! edited on, not by its file format. [`crate::tools::places::flightpaths`]
//! writes the flight path tables' fields through
//! [`set_fields`](fields::set_fields) and [`set_text`](fields::set_text) here,
//! and adds and removes their rows through `vale_edit::dbc::taxi`.
//!
//! This module is outside `ui/` for the same reason `map/tiles.rs` is: the
//! open table, the selected row and the effect of an edit are editor
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
//! with guessed types, which is what `vale dbc <Table>` prints. That shows
//! less than a schema does, but the table can still be opened.
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
//! The Tables workspace starts on no table ([`ANY`](tabs::ANY)): its list is
//! the tables themselves, and choosing one points the browser at it. A table is
//! named by one spelling whatever case it is asked for in
//! (`schema::table_name`), because the archives list their files in lower case
//! and the session holds each open table under its name.
//!
//! ## Row labels come from the rows that reference them
//!
//! A table with a schema is named by its first text column. A table without one
//! is named by a column guessed from the file
//! ([`guess_name_field`](vale_edit::dbc::names::guess_name_field)): the first in which the
//! rows hold the offsets of strings.
//!
//! `SpellVisual` row 67 has no name column, and a list of 2,167 bare ids cannot
//! be searched by meaning. The label is built from the tables around the row:
//! that visual fills the precast, cast and impact slots, throws a missile, and
//! is referenced by nine spells named Fireball. [`describe`](labels::describe)
//! builds that label for each table, and the search index is built from the
//! labels, so the query `fireball` finds the visual as well as the spell.
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
//! [`add_row`](rows::add_row), [`clone_row`](rows::clone_row) and
//! [`delete_row`](rows::delete_row) each make one entry on the undo stack, and
//! each new row takes the next id above the table's largest. Two operations
//! used from a form are built on them: [`link_new`](rows::link_new) makes a
//! blank row in the table a reference names and points the reference at it, and
//! [`unshare`](rows::unshare) copies the row a reference names and points the
//! reference at the copy, so a kit that forty spells share becomes one visual's
//! own. [`clone_chain`](rows::clone_chain) does the same for a whole
//! `SpellVisual` and everything it names, through `vale_edit::dbc::chain`.
//!
//! ## Rows of one table edited from another table's form
//!
//! Some rows exist only for a row of another table and have no name of their
//! own: a `SkillLineAbility` row is a spell's place in a skill line, a
//! `SkillRaceClassInfo` row is who has a skill line, and a teaching spell is a
//! `Spell` row whose only job is to teach another. The form of the row they are
//! about lists and edits them (`crate::ui::tables`), and the functions here
//! find and make them: [`abilities_of`](learning::abilities_of),
//! [`teachers_of`](learning::teachers_of),
//! [`race_class_rows_of`](learning::race_class_rows_of),
//! [`add_ability`](learning::add_ability),
//! [`add_teaching_spell`](learning::add_teaching_spell),
//! [`add_race_class_row`](learning::add_race_class_row). Each make is one entry
//! on the undo stack. None of them is required: most spells are in no skill
//! line and have no teaching spell.
//!
//! A row that many rows share (a kit, a cast time, a range, an icon) is
//! not edited this way. It stays a reference with a picker, because an edit
//! made from one row's form would change every row that names it.
//!
//! An item set's item list and an item's `set_id` are one fact stored twice, in
//! a DBC table and in a server row.
//! [`move_between_sets`](rows::move_between_sets) is the table half of keeping
//! them in step.
//!
//! ## Zones and sub-areas
//!
//! `AreaTable` is two levels: a zone, and the sub-areas that name it as their
//! parent. A zone's form lists its sub-areas
//! ([`sub_areas_of`](zones::sub_areas_of)). [`add_zone`](zones::add_zone) and
//! [`add_sub_area`](zones::add_sub_area) make a row with the columns a blank
//! row would get wrong: the map, the parent, and an explore bit no other row
//! holds ([`next_explore_bit`](vale_edit::dbc::places::next_explore_bit)). A sub-area also
//! starts with its zone's flags, reverb, ambience and music. A row is put on
//! the ground by the Areas tool, which the form asks the shell
//! to switch to with `crate::ui::shell::asks::ShellAsk::PaintArea`.
//!
//! ## Commands on a row
//!
//! What is done to a row other than editing a field is a
//! [`Command`](commands::Command): clone, delete, copy the id, and a table's
//! own, such as making a spell's teaching spell.
//! [`commands`](commands::commands) lists a table's and
//! [`command_label`](commands::command_label) words one for a row, or says it
//! does not apply to that row. The panel's buttons and its right-click menus
//! read both, so the two cannot differ.
//!
//! ## A row of the spell chain is found and pictured by its models
//!
//! A kit's label names its first model and a visual's the first spell that uses
//! it. The search index also holds every model a kit hangs, by effect name and
//! by file name, and every spell that uses a visual
//! ([`search_words`](visuals::search_words)), and
//! [`first_model`](visuals::first_model) is the model a list draws beside the
//! row.
//!
//! The contents of a blank row are the only table-specific rule here: a kit's
//! effect and procedural slots are `-1` for none
//! ([`blank_defaults`](vale_assets::tables::schema::blank_defaults)), which is the convention the
//! shipped rows use.

use bevy::prelude::*;

pub(crate) mod browser;
pub(crate) mod labels;
pub(crate) mod tabs;
pub(crate) mod fields;
pub(crate) mod learning;
pub(crate) mod zones;
pub(crate) mod rows;
pub(crate) mod commands;
pub(crate) mod visuals;
pub(crate) mod sounds;
pub(crate) mod open;
#[cfg(test)]
mod tests;

use browser::Browser;
use sounds::{audition, listen};
use open::open_tables;

/// What a row is called in a list and at the head of its form.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RowLabel {
    /// The name, or the sentence that stands in for one.
    pub(crate) title: String,
    /// A second, quieter line: a rank, a file name, an animation.
    pub(crate) sub: String,
    pub(crate) id: u32,
}

impl RowLabel {
    /// The label on one line, for a place with room for only one.
    pub(crate) fn line(&self) -> String {
        match self.title.is_empty() {
            true => format!("{}", self.id),
            false => format!("{}  [{}]", self.title, self.id),
        }
    }
}

/// One reference to a row: the table, record and field that hold it.
pub(crate) use vale_assets::tables::labels::Reference as Use;

/// A dialog over the form. At most one is open, and `ui::tables` draws it once
/// per frame regardless of which field opened it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Modal {
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
    /// Choose a texture for the display-table text field at `(record,
    /// field)`: a creature's skin, an item model's skin, a bag icon or a body
    /// texture, from the folder the field's file is found in, or a picture
    /// imported into it. See `ui::tables::appearance`.
    Textures { record: usize, field: usize },
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
pub(crate) const MODEL_FOLDERS: [(&str, &str); 5] = [
    ("All", ""),
    ("Spells", "spells\\"),
    ("Particles", "particles\\"),
    ("World", "world\\"),
    ("Creature", "creature\\"),
];

/// What a row of a display table looks like: the model it draws, dressed as
/// the row dresses it, and an item's bag icon.
#[derive(Debug, Clone, Default)]
pub(crate) struct Shown {
    /// The model, or for an item that has none, a body wearing it.
    pub(crate) worn: Option<crate::preview::portraits::Worn>,
    /// Whether [`Self::worn`] is a body wearing the row rather than the row's
    /// own model.
    pub(crate) on_a_body: bool,
    /// An `ItemDisplayInfo` row's bag icon, as an archive path.
    pub(crate) icon: Option<String>,
    /// Why nothing is drawn, when [`Self::worn`] is `None`.
    pub(crate) why: &'static str,
}

pub(crate) struct TableToolPlugin;

impl Plugin for TableToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Browser>()
            .init_resource::<crate::ui::tables::storyboard::Storyboard>()
            .add_systems(Update, (open_tables, audition, listen));
    }
}
