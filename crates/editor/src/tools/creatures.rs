//! The server's creature spawns: drawn on the map where they stand and picked
//! with the pointer.
//!
//! The document here is a row in vmangos' database rather than a file in the
//! archives. A `creature` row carries a map and three coordinates, so it has a
//! place in the world in the same way a `Light` row does. It is handled the
//! same way: drawn where it stands, picked with the pointer, and edited
//! through its form in the inspector.
//!
//! The data source differs from every other tool here. There is no file to
//! open and no overlay to publish into: the rows are read over a database
//! connection, and an edit is SQL. See [`vale_mangos::creature`] for what a
//! row means and [`crate::server::rows`] for what a save does with one.
//!
//! ## A click reaches two rows: the spawn and the template
//!
//! Clicking a creature reaches two rows, and an edit to each has a different
//! scope:
//!
//! ```text
//! creature           one spawn. Moving it moves one guard.
//! creature_template  what it is. Changing it changes every guard in the world.
//! ```
//!
//! Both are in the inspector, the spawn above the template, because the click
//! landed on the spawn and the template is looked up from it. The form states
//! which row is which before anything else; see [`crate::ui::creatures`].
//!
//! ## The whole map is read in one query, once
//!
//! Map 0 has 24,610 spawns and map 1 has 29,010, and one statement reads either
//! in about 0.6 s (`vale spawns Azeroth`). A query per block as the camera
//! moves would put a database round trip on the frame the camera pans. So the
//! map is read once, on a task, and camera movement only changes which of the
//! rows are drawn.
//!
//! The map is read again when the map changes, when a save applies something,
//! and when the Reload button is pressed. It is not polled: this does not
//! handle another editor writing the same database, and a re-read every few
//! seconds would discard a selection whose form is being typed into.
//!
//! ## Near spawns are drawn as models, the rest as markers
//!
//! The busiest tile on map 0 holds 288 spawns and the editor streams a 7x7
//! block, so drawing every spawn in the block as a model is up to several
//! thousand creatures. [`Creatures::model_budget`] is the cap: the nearest N
//! get a model through the client's own entity pipeline, and every other spawn
//! in range gets a marker. Both are pickable, so the budget changes how a spawn
//! is drawn and never whether it can be picked.
//!
//! A model is a synthetic [`WorldEntity`] (a guid, a display id and a scale)
//! standing at the row's position, built the same way [`crate::stage`] and
//! [`crate::lab`] build theirs. It goes through `spawn_models`, `dress` and
//! `animate`, so what is on screen is what the game would draw, not a second
//! reading of the same tables in this crate.
//!
//! Nothing is drawn during a playtest. The server spawns the real creatures
//! then, and a preview in the same place would be a second copy of each one.

use super::Tool;
use crate::server::fresh::Fresh;
use crate::marks::{Look, Marks};
use crate::session::EditSession;
use vale_client::render::focus::WorldFocus;
use vale_client::world::camera::WorldCamera;
use vale_client::world::session::{ObjectType, WorldEntity};
use vale_mangos::creature::{self, RowValue};
use vale_mangos::row::{Edits, Key, Life, RowEdit};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::{HashMap, HashSet};

/// One `creature` row, as the viewport needs it.
///
/// The columns of the join in [`vale_mangos::creature::spawns_on_map_query`],
/// read into numbers once so the drawing does not parse text every frame.
#[derive(Debug, Clone)]
pub struct Spawn {
    pub guid: u64,
    /// `creature.id`: which template stands here, as the project says it.
    ///
    /// Differs from [`Self::read_entry`] while the project changes the
    /// template's entry and has not applied it; see [`Creatures::fold_template_moves`].
    pub entry: u32,
    /// The template entry the map query read for this spawn, which is where
    /// the database has the template's row until a move of it is applied.
    pub read_entry: u32,
    /// Where it is, in the world's own axes.
    pub at: Vec3,
    /// Radians it faces, as the row states it.
    pub orientation: f32,
    pub wander: f32,
    /// `creature.movement_type`: what this spawn does with a waypoint path.
    ///
    /// Read here rather than from the whole row, because the Waypoints
    /// button needs it before the path has been read: a path on a creature
    /// whose movement type walks none is an edit that applies, reloads and has
    /// no effect, and the window states this on the frame it opens.
    pub movement_type: u32,
    /// The template's name, or `None` when the join found no row. vmangos
    /// refuses to load a spawn with no template row.
    pub name: Option<String>,
    pub subname: Option<String>,
    pub display_id: u32,
    /// `display_scale1`, which is 0 in most rows and means "the model's own".
    pub display_scale: f32,
    pub level: (u32, u32),
    pub faction: u32,
    pub npc_flags: u32,
    pub rank: u32,
    /// Which content patch of the template the server would load. It is the
    /// other half of the key an edit to the template is written under.
    pub patch: u32,
    /// What this project does to the row.
    ///
    /// [`Life::Update`] for a spawn as the database has it, whether or not this
    /// project edits a column of it. [`Life::Insert`] for one the project
    /// creates, which has no row in the database at all. [`Life::Delete`] for
    /// one it removes. A removed spawn is still drawn, so it can be selected
    /// and restored; nothing reaches the database until Apply.
    pub claim: Life,
}

impl Spawn {
    /// The key an edit to this spawn is written under.
    pub fn key(&self) -> Key {
        creature::spawn_key(self.guid)
    }

    /// The key an edit to its template is written under. The template is a
    /// different row, and an edit to it affects every spawn of the entry.
    pub fn template_key(&self) -> Key {
        creature::template_key(self.entry, self.patch)
    }

    /// What to call it in a list.
    pub fn label(&self) -> String {
        match &self.name {
            Some(name) => format!("{name} ({})", self.entry),
            None => format!("entry {} — no template row", self.entry),
        }
    }

    /// Whether this project creates this spawn, so there is no row behind it.
    pub fn is_new(&self) -> bool {
        self.claim == Life::Insert
    }

    /// Whether this project removes this spawn.
    pub fn is_removed(&self) -> bool {
        self.claim == Life::Delete
    }
}

impl Spawn {
    /// This row with the project's own edits laid over it.
    ///
    /// The list in [`Creatures::spawns`] is what the database holds: it is
    /// read once per map and again when an apply changes something. A form edit
    /// changes neither. Without this function, typing a new `display_id1` or a
    /// new `orientation` changed nothing on screen until the rows had been
    /// applied and read back, so the edit appeared to be ignored.
    ///
    /// Laying the store over the reading makes the drawing follow an edit and
    /// an undo on the same frame, and an edit still lives in one place only.
    /// Only the columns the drawing uses are read; the form reads the store
    /// itself.
    ///
    /// `None` when the project changes neither of this spawn's two rows, which
    /// is true of all but a few spawns on the map. The caller then keeps the
    /// base reading rather than cloning it.
    fn with_edits(&self, edits: &Edits) -> Option<Spawn> {
        let spawn_key = self.key();
        let template_key = self.template_key();
        if !edits.touches(creature::SPAWN, &spawn_key)
            && !edits.touches(creature::TEMPLATE, &template_key)
        {
            return None;
        }
        let mut out = self.clone();
        out.claim = edits.life(creature::SPAWN, &spawn_key);
        let number = |key: &Key, table: &'static str, column: &str| -> Option<f32> {
            edits.get(table, key, column)?.trim().parse().ok()
        };
        let text = |key: &Key, table: &'static str, column: &str| -> Option<String> {
            Some(unquote(edits.get(table, key, column)?))
        };

        if let Some(x) = number(&spawn_key, creature::SPAWN, "position_x") {
            out.at.x = x;
        }
        if let Some(y) = number(&spawn_key, creature::SPAWN, "position_y") {
            out.at.y = y;
        }
        if let Some(z) = number(&spawn_key, creature::SPAWN, "position_z") {
            out.at.z = z;
        }
        if let Some(o) = number(&spawn_key, creature::SPAWN, "orientation") {
            out.orientation = o;
        }
        if let Some(w) = number(&spawn_key, creature::SPAWN, "wander_distance") {
            out.wander = w;
        }
        if let Some(id) = number(&spawn_key, creature::SPAWN, "id") {
            out.entry = id as u32;
        }
        if let Some(display) = number(&template_key, creature::TEMPLATE, "display_id1") {
            out.display_id = display as u32;
        }
        if let Some(scale) = number(&template_key, creature::TEMPLATE, "display_scale1") {
            out.display_scale = scale;
        }
        if let Some(name) = text(&template_key, creature::TEMPLATE, "name") {
            out.name = Some(name);
        }
        if let Some(subname) = text(&template_key, creature::TEMPLATE, "subname") {
            out.subname = Some(subname);
        }
        if let Some(flags) = number(&template_key, creature::TEMPLATE, "npc_flags") {
            out.npc_flags = flags as u32;
        }
        if let Some(rank) = number(&template_key, creature::TEMPLATE, "rank") {
            out.rank = rank as u32;
        }
        if let Some(faction) = number(&template_key, creature::TEMPLATE, "faction") {
            out.faction = faction as u32;
        }
        if let Some(low) = number(&template_key, creature::TEMPLATE, "level_min") {
            out.level.0 = low as u32;
        }
        if let Some(high) = number(&template_key, creature::TEMPLATE, "level_max") {
            out.level.1 = high as u32;
        }
        Some(out)
    }
}

/// The inside of a SQL string literal. The store keeps a name as a literal.
///
/// `vale_mangos::sql::text` is the one place a value is escaped, and this is
/// the one place the drawing reads one back. A value that is not a quoted
/// literal is returned as it is.
fn unquote(literal: &str) -> String {
    let Some(inner) = literal
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
    else {
        return literal.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        match (ch, chars.clone().next()) {
            ('\\', Some('n')) => {
                chars.next();
                out.push('\n');
            }
            ('\\', Some(next)) => {
                chars.next();
                out.push(next);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// The columns of a creature entry that a marker, a model and a panel heading
/// need.
///
/// The map query already carries these columns for every spawn on the map, so
/// most entries need no extra read. Two sources do: the picker's matches,
/// which may be creatures with no spawn on this map, and the entries a project
/// creates a spawn of after the map was read. All three fill the same table;
/// see [`Creatures::known`].
#[derive(Debug, Clone, PartialEq)]
pub struct Known {
    /// The entry the project says the template has, which the list shows and
    /// the store keys its claim under.
    pub entry: u32,
    /// The entry the database has the row at, which differs from
    /// [`Self::entry`] while the project moves the template and has not
    /// applied it. A template the project creates has no row and the two are
    /// equal.
    pub read_entry: u32,
    /// Which content patch of the template the server would load.
    pub patch: u32,
    pub name: String,
    pub subname: Option<String>,
    pub display_id: u32,
    pub display_scale: f32,
    pub level: (u32, u32),
    pub faction: u32,
    pub npc_flags: u32,
    pub rank: u32,
    /// [`Life::Insert`] for a template this project creates, which is in no
    /// database; [`Life::Update`] for one the database holds.
    pub claim: Life,
}

impl Known {
    /// What to call it in a list.
    pub fn label(&self) -> String {
        match self.subname.as_deref().filter(|text| !text.is_empty()) {
            Some(subname) => format!("{} <{subname}> ({})", self.name, self.entry),
            None => format!("{} ({})", self.name, self.entry),
        }
    }

    /// The key an edit to its template row is written under.
    pub fn key(&self) -> Key {
        creature::template_key(self.entry, self.patch)
    }

    /// Whether this project creates the template, so there is no row behind it.
    pub fn is_new(&self) -> bool {
        self.claim == Life::Insert
    }

    /// A template this project creates, read out of its row in the store:
    /// the columns a marker, a model and a heading need, under the key's
    /// entry and patch.
    fn from_store(key: &Key, row: &RowEdit) -> Option<Known> {
        let entry = key.first()? as u32;
        let patch = key
            .0
            .iter()
            .find(|(column, _)| column == "patch")
            .and_then(|(_, value)| value.parse::<u32>().ok())?;
        let number = |column: &str| -> u32 {
            row.columns
                .get(column)
                .and_then(|value| value.trim().parse::<f64>().ok())
                .unwrap_or(0.0) as u32
        };
        let text = |column: &str| -> Option<String> { row.columns.get(column).map(|literal| unquote(literal)) };
        Some(Known {
            entry,
            read_entry: entry,
            patch,
            name: text("name").unwrap_or_default(),
            subname: text("subname"),
            display_id: number("display_id1"),
            display_scale: row
                .columns
                .get("display_scale1")
                .and_then(|value| value.trim().parse::<f32>().ok())
                .unwrap_or(0.0),
            level: (number("level_min"), number("level_max")),
            faction: number("faction"),
            npc_flags: number("npc_flags"),
            rank: number("rank"),
            claim: Life::Insert,
        })
    }
}

/// One row of the brief template query.
fn read_known(row: &creature::Row) -> Option<Known> {
    let integer = |column: &str| row.integer(column).unwrap_or(0) as u32;
    Some(Known {
        entry: row.integer("entry")? as u32,
        read_entry: row.integer("entry")? as u32,
        claim: Life::Update,
        patch: integer("patch"),
        name: row.text("name").unwrap_or_default().to_string(),
        subname: row.text("subname").map(str::to_string),
        display_id: integer("display_id1"),
        display_scale: row.number("display_scale1").unwrap_or(0.0) as f32,
        level: (integer("level_min"), integer("level_max")),
        faction: integer("faction"),
        npc_flags: integer("npc_flags"),
        rank: integer("rank"),
    })
}

/// The picker behind New spawn, and the creature a click on the ground will
/// place.
///
/// A spawn is a `creature` row and its first column is a template entry
/// number. The creature is chosen by name, and the search also matches the
/// entry, because `vale spawns` and the template window both print entries
/// and a user who has one can type it.
///
/// The mode decides what a click does, as in the placement tools: in Place, a
/// chosen creature stands translucent under the pointer and the next click on
/// the ground creates the spawn; in Select, a click picks what is there. See
/// [`Creatures::mode`].
#[derive(Debug, Default)]
pub struct NewSpawn {
    /// What has been typed.
    pub search: String,
    /// The search last sent, so a redraw does not re-query every frame.
    ran: String,
    pub matches: Vec<Known>,
    task: Option<Task<Result<Vec<Known>, String>>>,
    /// The creature a click will place. The ghost on the cursor and the
    /// preview pane both show it. `None` means nothing is chosen, which is the
    /// state `Escape` leaves.
    pub chosen: Option<Known>,
    /// Why the search found nothing, when it is not that nothing matched.
    pub trouble: Option<String>,
    /// The `EditSession::database_writes` the matches were read at.
    searched_at: Option<u64>,
}

impl NewSpawn {
    /// Whether a search is in flight, for the panel.
    pub fn searching(&self) -> bool {
        self.task.is_some()
    }

    /// Forget the matches, which a map change or an apply makes stale.
    pub fn forget(&mut self) {
        self.ran.clear();
        self.matches.clear();
    }
}

/// How many entries the picker lists at once: fifty.
///
/// A `LIKE '%a%'` over 10,390 templates matches thousands. A longer list is
/// not scrolled through; the user narrows it by typing more letters into the
/// search box.
const MATCHES: usize = 50;

/// What the pointer is doing with the creatures on the open map.
#[derive(Resource, Debug)]
pub struct Creatures {
    /// Every spawn on the open map, as the database has it.
    pub spawns: Vec<Spawn>,
    /// The spawns this project creates. They are in no database and are
    /// rebuilt from the store whenever it changes.
    ///
    /// A second list rather than rows appended to [`Self::spawns`], because
    /// that list is what a read returned and the next read replaces it whole.
    /// Everything that draws, picks or counts walks both; see
    /// [`Creatures::base_spawn`].
    pub created: Vec<Spawn>,
    /// Which `EditSession::server_edit_revision` [`Self::created`] and
    /// [`Self::templates`] were built for.
    created_for: Option<u64>,
    /// The templates this project creates, rebuilt from the store whenever it
    /// changes, sorted by entry. They are in no database, so no query returns
    /// them; the picker lists them above its search, and a spawn of one is
    /// labelled and drawn from here through [`Self::known`].
    pub templates: Vec<Known>,
    /// The highest entry the `creature_template` table holds, at any patch,
    /// read with the map. One of the inputs to a new template's entry; see
    /// [`Creatures::next_entry`]. `None` until the read completes.
    pub max_entry: Option<u32>,
    /// Whether `--template-new` and `--template-entry` have been acted on, or
    /// were not asked for. `--apply-creatures` waits on it beside
    /// [`Self::scripted_done`].
    pub scripted_template_done: bool,
    /// The highest guid the `creature` table holds, read with the map.
    ///
    /// One of the inputs to a new spawn's guid; see
    /// [`Creatures::next_guid`]. `None` until the read completes, so placing
    /// waits for it.
    pub max_guid: Option<u64>,
    /// What a creature entry is, for the spawns of it this project creates.
    ///
    /// Filled from the map query, which already carries these columns for
    /// every spawn on the map; from the picker's matches; and, for an entry
    /// neither knows, from a read of its own.
    pub known: Fresh<HashMap<u32, Known>>,
    /// The read of [`Self::known`]'s own, while it is running.
    known_task: Option<Task<Result<Vec<Known>, String>>>,
    /// What a display id looks like, resolved once per id.
    ///
    /// The join behind a picture (`DisplayTables::creature` and
    /// `look::dress::dress`) is a DBC lookup, a string build and a
    /// `Dressing`, and the picker asks for one for every row on every frame it
    /// draws. Fifty rows at sixty frames a second is three thousand lookups a
    /// second for an answer that cannot change while the archives are open.
    ///
    /// `None` is a display id the tables do not resolve, kept so it is not
    /// asked again.
    pub worn: HashMap<u32, Option<crate::portraits::Worn>>,
    /// The display id picker, while it is open on one of the template's five
    /// display columns. See `crate::tools::displays`.
    pub display_pick: Option<super::displays::DisplayPick>,
    /// Which half of the panel is showing, which also decides what a click in
    /// the world does.
    ///
    /// This is `place::Mode`, the placement tools' own two values, rather than
    /// a third enumeration with the same meaning: choosing what to put down and
    /// editing what is there are one subject, and the user switches between
    /// them often. See `crate::tools::place::Mode` for the reasoning.
    pub mode: super::place::Mode,
    /// The picker shown in the Place half, and what a click will put down.
    pub new_spawn: NewSpawn,
    /// The direction the creature on the cursor faces, in radians, which is
    /// what a click writes as `orientation`. `None` while nothing is on the
    /// cursor. Set toward the camera when the placer is armed, kept from one
    /// placement to the next, and turned by [`Self::turn_ghost`]. See
    /// [`ghost`].
    ghost_facing: Option<f32>,
    /// The read, while it is running. `None` when there is nothing in flight.
    task: Option<Task<Result<MapRead, String>>>,
    /// What [`Self::spawns`] was read for: the map, and what the last apply
    /// put in the database.
    ///
    /// Keyed on `EditSession::database_writes`, which counts every apply and
    /// every Put back, and not on the project's edit counter, which changes
    /// on every keystroke; a map query per typed character is 24,610 rows per
    /// keystroke. A row this project has edited and not yet applied is
    /// therefore drawn as the database still has it. The form shows both, and a
    /// drag moves the marker in memory so the marker follows the pointer during
    /// the drag.
    loaded: Option<(u32, u64)>,
    /// Why there is nothing, when there is nothing: no database, a refused
    /// connection, a query that failed.
    pub trouble: Option<String>,
    /// Indices into [`Self::spawns`] that are near enough to draw, nearest
    /// first. Rebuilt when the focus crosses a tile, not per frame.
    pub near: Vec<usize>,
    /// The tile [`Self::near`] was built around.
    built_at: Option<(u32, u32)>,
    /// The spawn under the pointer, by guid.
    pub hovered: Option<u64>,
    /// The spawn whose form is open, by guid: the primary of the selection.
    pub selected: Option<u64>,
    /// The rest of the selection, by guid — see [`super::group`]. The form is
    /// the primary's; a move, a turn or a removal acts on every one.
    ///
    /// A guid here whose spawn is no longer on the map (a created spawn whose
    /// creation was undone) is skipped by everything that reads the group,
    /// as [`Creatures::edited`] finds nothing for it. It is not pruned every
    /// frame, because pruning is a walk of every spawn on the map.
    pub also: Vec<u64>,
    /// Where each member stood when a drag's button went down, so each moves
    /// by the drag's step from where it began, as the primary does.
    group_from: Vec<(u64, Vec3)>,
    /// The selected spawn's whole template row, read on demand.
    pub template: Option<TemplateRow>,
    /// The template read, while it is running.
    template_task: Option<Task<Result<Option<TemplateRow>, String>>>,
    /// What [`Self::template`] was made at: `EditSession::database_writes`,
    /// and whether the project created the row, in which case it is an empty
    /// row holding the key. Either changing makes it be made again: an apply
    /// or a put back changes what the database holds, and a discard turns a
    /// created row into one the database may hold.
    template_at: Option<(u64, bool)>,
    /// The selected spawn's whole spawn row. The map query reads only ten of
    /// its columns.
    pub spawn_row: Option<SpawnRow>,
    spawn_task: Option<Task<Result<Option<SpawnRow>, String>>>,
    /// …and the same for [`Self::spawn_row`].
    spawn_at: Option<(u64, bool)>,
    /// Draw the near ones as models, or as markers only.
    pub show_models: bool,
    /// How many models at once — see the module comment.
    pub model_budget: usize,
    /// Draw a row of service icons over each near creature that offers one.
    /// See [`mark_services`].
    pub show_services: bool,
    /// How far out a spawn is drawn at all, in yards.
    pub range: f32,
    /// Whether the template window is open. [`crate::ui::creatures`] states
    /// why the template is a window and the spawn is not.
    pub template_window: bool,
    /// Where a drag is moving the selected spawn to, and where it began.
    pub drag: Option<Drag>,
    /// A fly-to asked for by the panel, consumed by [`fly`].
    pub fly_to: Option<Vec3>,
    /// Whether the scripted placements and removals have finished.
    ///
    /// `--apply-creatures` runs on the first frame there is a session and
    /// `--spawn-add` runs several seconds later, once the map read, the search
    /// and the tile under the camera have all completed. Without this flag the
    /// apply ran against an empty store and reported "nothing to apply", and
    /// the edit arrived afterwards. `Waypoints::scripted_done` is the same
    /// flag for the same reason, and the apply waits on both.
    pub scripted_done: bool,
}

impl Default for Creatures {
    fn default() -> Creatures {
        Creatures {
            spawns: Vec::new(),
            created: Vec::new(),
            created_for: None,
            templates: Vec::new(),
            max_entry: None,
            scripted_template_done: false,
            max_guid: None,
            known: Fresh::default(),
            known_task: None,
            worn: HashMap::new(),
            display_pick: None,
            mode: super::place::Mode::default(),
            new_spawn: NewSpawn::default(),
            ghost_facing: None,
            task: None,
            loaded: None,
            trouble: None,
            near: Vec::new(),
            built_at: None,
            hovered: None,
            selected: None,
            also: Vec::new(),
            group_from: Vec::new(),
            template: None,
            template_task: None,
            template_at: None,
            spawn_row: None,
            spawn_task: None,
            spawn_at: None,
            show_models: true,
            // 200 creatures is about what a busy city block holds, and is
            // well inside what the entity pass draws in a session. The panel
            // changes it and shows the count.
            model_budget: 200,
            show_services: true,
            // A little past the 7x7 block the editor streams at its default
            // reach, so a marker appears as its ground does.
            range: 1200.0,
            template_window: false,
            drag: None,
            fly_to: None,
            scripted_done: false,
        }
    }
}

/// What the template window is about: the creature chosen in the picker while
/// the tool is in Place, or the selected spawn's template in Select.
///
/// Both name a `creature_template` row by entry and patch. The picker's
/// creature may be one this project creates, which is in no database and
/// whose columns are the store's own; a spawn's template is always a row the
/// database holds. See [`Creatures::template_subject`].
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateSubject {
    /// The entry the project says the template has.
    pub entry: u32,
    /// The entry the database has it at, which differs while the project moves
    /// it and has not applied the move.
    pub read_entry: u32,
    pub patch: u32,
    /// What this project says of the row: [`Life::Insert`] for one it creates,
    /// [`Life::Update`] otherwise.
    pub claim: Life,
    /// The creature's name and entry, for the window's title.
    pub label: String,
}

impl TemplateSubject {
    /// The key an edit to the row is written under.
    pub fn key(&self) -> Key {
        creature::template_key(self.entry, self.patch)
    }

    /// Whether this project creates the template.
    pub fn is_new(&self) -> bool {
        self.claim == Life::Insert
    }
}

/// One row of `creature_template`, whole.
#[derive(Debug, Clone)]
pub struct TemplateRow {
    pub entry: u32,
    pub patch: u32,
    pub row: creature::Row,
}

/// One row of `creature`, whole.
#[derive(Debug, Clone)]
pub struct SpawnRow {
    pub guid: u64,
    pub row: creature::Row,
}

/// What a held left button is moving, once it has moved far enough to be a drag.
///
/// The spawn's own position, across the horizontal plane at the height of the
/// ground under the pointer. By this crate's rule a drag belongs to the object
/// it began on: once the button is down, the pointer leaving the marker still
/// moves that spawn.
///
/// Armed only by a press on the already-selected creature, and it moves
/// nothing until the pointer has travelled [`DRAG_PIXELS`]. See [`press`].
#[derive(Debug, Clone, Copy)]
pub struct Drag {
    pub guid: u64,
    /// Where the spawn was when the button went down, so the status line can
    /// say how far it moved.
    pub from: Vec3,
    /// Where the pointer was when the button went down. [`DRAG_PIXELS`] is
    /// measured from here.
    pub pressed_at: Vec2,
    /// How far the spawn is from the pointer, so a press near the edge of the
    /// marker does not snap it under the cursor on the first frame. `None`
    /// until the first frame the drag is actually moving.
    pub grab: Option<Vec3>,
    /// Whether the pointer has travelled far enough to be a drag rather than
    /// a click. While it is false the drag moves nothing.
    pub moving: bool,
    /// Whether the press landed on a spawn in a group of several. Released
    /// without moving, it selects that spawn alone — see [`super::group`].
    pub collapse: bool,
}

impl Creatures {
    /// How many spawns there are to draw: the database's and this project's
    /// own.
    pub fn spawn_count(&self) -> usize {
        self.spawns.len() + self.created.len()
    }

    /// One spawn, by an index over both lists, the database's first.
    ///
    /// Everything that walks the spawns goes through this rather than through
    /// [`Self::spawns`], which holds only what the last read returned. This
    /// keeps a created spawn drawable, pickable and counted on the same terms
    /// as a read one.
    pub fn base_spawn(&self, index: usize) -> Option<&Spawn> {
        match self.spawns.get(index) {
            Some(spawn) => Some(spawn),
            None => self.created.get(index - self.spawns.len()),
        }
    }

    /// The chosen spawn, if it is still on this map.
    pub fn chosen(&self) -> Option<&Spawn> {
        let guid = self.selected?;
        self.spawns
            .iter()
            .chain(self.created.iter())
            .find(|spawn| spawn.guid == guid)
    }

    /// One spawn as it should be drawn: the database's reading with the
    /// project's edits over it. See [`Spawn::with_edits`].
    ///
    /// Returns an owned `Spawn` rather than a reference, because an edited one
    /// is built here and there is nothing to borrow it from. That is one
    /// clone per drawn marker per frame for the handful the project has
    /// touched, and a clone of the base for the rest; a `Spawn` is a dozen
    /// numbers and two `Option<String>`s.
    pub fn spawn_at(&self, index: usize, edits: Option<&Edits>) -> Option<Spawn> {
        let base = self.base_spawn(index)?;
        match edits.and_then(|edits| base.with_edits(edits)) {
            Some(edited) => Some(edited),
            None => Some(base.clone()),
        }
    }

    /// The chosen spawn, with the project's edits over it.
    pub fn chosen_edited(&self, edits: Option<&Edits>) -> Option<Spawn> {
        let base = self.chosen()?;
        Some(
            edits
                .and_then(|edits| base.with_edits(edits))
                .unwrap_or_else(|| base.clone()),
        )
    }

    /// The spawn under the pointer, with the project's edits over it.
    pub fn hovered_edited(&self, edits: Option<&Edits>) -> Option<Spawn> {
        self.edited(self.hovered?, edits)
    }

    /// Any spawn on this map by guid, with the project's edits over it.
    pub fn edited(&self, guid: u64, edits: Option<&Edits>) -> Option<Spawn> {
        let base = self
            .spawns
            .iter()
            .chain(self.created.iter())
            .find(|spawn| spawn.guid == guid)?;
        Some(
            edits
                .and_then(|edits| base.with_edits(edits))
                .unwrap_or_else(|| base.clone()),
        )
    }

    /// Whether a spawn is selected, as the primary or as a member.
    pub fn holds(&self, guid: u64) -> bool {
        self.selected == Some(guid) || self.also.contains(&guid)
    }

    /// Make `guid` the primary, and drop the two whole rows the form draws so
    /// the new primary's are read. The group is left as it is.
    fn make_primary(&mut self, guid: Option<u64>) {
        if self.selected != guid {
            self.selected = guid;
            self.template = None;
            self.spawn_row = None;
        }
    }

    /// Select one spawn, or none, and drop the rest of the group.
    pub fn select_only(&mut self, guid: Option<u64>) {
        self.make_primary(guid);
        self.also.clear();
    }

    /// Shift+click: take a spawn out of the group, or add one and make it the
    /// primary. See [`super::group`].
    pub fn toggle(&mut self, guid: u64) {
        if self.selected == Some(guid) {
            let next = self.also.pop();
            self.make_primary(next);
            return;
        }
        if let Some(at) = self.also.iter().position(|&had| had == guid) {
            self.also.remove(at);
            return;
        }
        if let Some(was) = self.selected {
            self.also.push(was);
        }
        self.make_primary(Some(guid));
    }

    /// Make a member the primary, keeping the group.
    pub fn promote(&mut self, guid: u64) {
        let Some(at) = self.also.iter().position(|&had| had == guid) else {
            return;
        };
        match self.selected {
            Some(was) => self.also[at] = was,
            None => {
                self.also.remove(at);
            }
        }
        self.make_primary(Some(guid));
    }

    /// The guid a new spawn gets.
    ///
    /// The highest of three: the reserved base, one past what the `creature`
    /// table holds, and one past the highest this project has already claimed.
    /// The second is needed because the reserved base is only a floor: once a
    /// project has put a spawn at ten million, the table's own maximum is ten
    /// million and the next one has to go above it. The third is needed
    /// because several spawns can be placed between two reads of the map.
    ///
    /// See `vale_mangos::creature::RESERVED_GUID_BASE` for where the number
    /// comes from and what it costs.
    pub fn next_guid(&self, edits: &Edits) -> u64 {
        let in_database = self.max_guid.map(|highest| highest + 1).unwrap_or(0);
        let claimed = edits
            .rows()
            .filter(|(table, _, row)| *table == creature::SPAWN && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .map(|highest| highest + 1)
            .unwrap_or(0);
        creature::RESERVED_GUID_BASE.max(in_database).max(claimed)
    }

    /// Create a new spawn of `known` on the ground at `at`.
    ///
    /// Every column of the row is written at once, as one claim and one undo
    /// entry; the values come from `vale_mangos::creature::new_spawn`. The
    /// guid is [`Self::next_guid`]'s and is returned, because the caller
    /// selects what it just placed.
    pub fn create(
        &mut self,
        session: &mut EditSession,
        known: &Known,
        at: Vec3,
        orientation: f32,
        now: f64,
    ) -> u64 {
        let guid = self.next_guid(&session.server_edits);
        let key = creature::spawn_key(guid);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in
            creature::new_spawn(known.entry, session.map_id, (at.x, at.y, at.z), orientation)
        {
            row.columns.insert(change.column.to_string(), change.value);
        }
        let subject = format!("creature {guid}");
        session.set_server_row(
            creature::SPAWN,
            &key,
            Some(&row),
            Some(crate::session::Gesture {
                label: "New spawn",
                subject: &subject,
                now,
            }),
        );
        // So the next one placed before the map is read again does not reuse
        // this guid — `next_guid` reads the store, which now holds it.
        self.known
            .entry(known.entry)
            .or_insert_with(|| known.clone());
        self.select_only(Some(guid));
        guid
    }

    /// Create another spawn of the same creature beside the selected one.
    ///
    /// A second guard on the other side of a gate is the first one's row with
    /// a new guid. Every column is copied: the database's value where the
    /// project has not changed it and the project's where it has, so the copy
    /// matches what is drawn, not what the table holds.
    ///
    /// The copy is offset by [`APART`] yards rather than placed at the same
    /// point, because two creatures at one point look like one and the copy
    /// could not be clicked.
    ///
    /// `None` when the whole row has not been read yet, which is the first
    /// frame or two after a creature is clicked. Copying from a partly read
    /// row would take the table's defaults for the columns not yet read.
    pub fn duplicate(&mut self, session: &mut EditSession, now: f64) -> Option<u64> {
        let spawn = self.chosen_edited(Some(&session.server_edits))?;
        let from = self
            .spawn_row
            .as_ref()
            .filter(|held| held.guid == spawn.guid)?;
        let key = spawn.key();
        let guid = self.next_guid(&session.server_edits);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        // The defaults first, so a column that neither the row nor the project
        // supplies gets a value this crate chose rather than the table's.
        for change in creature::new_spawn(
            spawn.entry,
            session.map_id,
            spawn.at.into(),
            spawn.orientation,
        ) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        for column in creature::columns_of(creature::SPAWN) {
            if !column.editable() {
                continue;
            }
            let value = session
                .server_edits
                .get(creature::SPAWN, &key, column.name)
                .map(str::to_string)
                .or_else(|| column.literal(&from.row));
            if let Some(value) = value {
                row.columns.insert(column.name.to_string(), value);
            }
        }
        // Then the position: beside the original, and on this map whatever
        // map the copied row names.
        row.columns
            .insert("map".to_string(), session.map_id.to_string());
        row.columns.insert(
            "position_x".to_string(),
            vale_mangos::sql::float(spawn.at.x + APART),
        );
        let subject = format!("creature {guid}");
        session.set_server_row(
            creature::SPAWN,
            &key_for(guid),
            Some(&row),
            Some(crate::session::Gesture {
                label: "Duplicate spawn",
                subject: &subject,
                now,
            }),
        );
        self.select_only(Some(guid));
        Some(guid)
    }

    /// Mark a spawn for removal.
    ///
    /// The edit depends on what the project already claims for the row:
    ///
    /// * A spawn this project created is not marked for removal; the claim is
    ///   dropped entirely. A `DELETE` for a row that was never inserted would
    ///   act on a row this project does not own.
    /// * A spawn the database holds is marked [`Life::Delete`], keeping the
    ///   columns the project had edited, so undoing the removal restores
    ///   those edits instead of losing them.
    pub fn remove(session: &mut EditSession, guid: u64, now: f64) {
        let key = creature::spawn_key(guid);
        let held = session.server_edits.row(creature::SPAWN, &key).cloned();
        let subject = format!("creature {guid} removal");
        let wanted = match held {
            Some(row) if row.life == Life::Insert => None,
            held => {
                let mut row = held.unwrap_or_default();
                row.life = Life::Delete;
                Some(row)
            }
        };
        session.set_server_row(
            creature::SPAWN,
            &key,
            wanted.as_ref(),
            Some(crate::session::Gesture {
                label: "Remove spawn",
                subject: &subject,
                now,
            }),
        );
    }

    /// Cancel a removal. The column edits the spawn had are kept.
    pub fn keep(session: &mut EditSession, guid: u64, now: f64) {
        let key = creature::spawn_key(guid);
        let mut row = session
            .server_edits
            .row(creature::SPAWN, &key)
            .cloned()
            .unwrap_or_default();
        row.life = Life::Update;
        let subject = format!("creature {guid} removal");
        let wanted = match row.is_empty() {
            true => None,
            false => Some(row),
        };
        session.set_server_row(
            creature::SPAWN,
            &key,
            wanted.as_ref(),
            Some(crate::session::Gesture {
                label: "Cancel spawn removal",
                subject: &subject,
                now,
            }),
        );
    }

    /// Ask for the map to be read again on the next frame.
    pub fn forget(&mut self) {
        self.loaded = None;
        self.built_at = None;
        // Also the two whole rows the form draws. They are fetched once per
        // selection, and were previously re-fetched only when the selection
        // changed, so after an apply or a revert the form showed the values
        // from when the creature was first clicked while the markers showed
        // the re-read ones. Everything is re-read together now.
        self.template = None;
        self.spawn_row = None;
        // Also the picker's matches. An apply can change them and a map change
        // does not, but refilling them costs one keystroke.
        self.new_spawn.forget();
    }
    /// Discard every edit this project makes to creatures.
    ///
    /// The store is the project's claim on those rows, and it survives both an
    /// apply and a revert: reverting restores the database and leaves the
    /// project still changing the same columns, so the next Apply writes them
    /// again. A revert undoes what reached the server and keeps the work; this
    /// function abandons the work in one action rather than a field at a time.
    ///
    /// Not an undo step. It is one action over many rows and the history is
    /// per-column; `History::forget_server_rows` drops the entries instead, for
    /// the reason `forget_table` drops a discarded table's: an entry addressed
    /// against a store that has been emptied would put a value back into a row
    /// the project no longer claims.
    pub fn discard(session: &mut EditSession) -> (usize, usize) {
        // Only this subject's tables, not the whole store. The item workspace
        // keeps its rows in the same file, so a discard that emptied the store
        // discarded item edits and counted them as creatures. The waypoint
        // paths are discarded too, since only this subject has any; see
        // `EditSession::forget_server_edits`.
        session.forget_server_edits(|table| creature::table_named(table).is_some(), true)
    }

    /// The entry a new template gets.
    ///
    /// The highest of three: the reserved base, one past what the table holds
    /// at any patch, and one past the highest this project has already
    /// claimed. The second is needed because the reserved base is only a
    /// floor: once a project has applied a template at two million, the
    /// table's own maximum is two million. The third is needed because several
    /// templates can be created between two reads of the map. See
    /// `vale_mangos::creature::RESERVED_ENTRY_BASE`.
    pub fn next_entry(&self, edits: &Edits) -> u32 {
        let in_database = self.max_entry.map(|highest| highest + 1).unwrap_or(0);
        let claimed = edits
            .rows()
            .filter(|(table, _, row)| *table == creature::TEMPLATE && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .map(|highest| highest as u32 + 1)
            .unwrap_or(0);
        creature::RESERVED_ENTRY_BASE.max(in_database).max(claimed)
    }

    /// Make a new template, choose it in the picker and open the template
    /// window on it.
    ///
    /// Every column is written at once, as one claim and one undo entry; the
    /// values come from `vale_mangos::creature::new_template`. The patch is
    /// the server's own rather than 0: a row at patch 0 would be beaten by any
    /// later content-patch row of the same entry.
    ///
    /// The tool is put into Place with the new creature chosen, and the
    /// template window is opened on it. A new creature is named and given a
    /// model in the window, then placed in the world with a click.
    pub fn create_template(
        &mut self,
        session: &mut EditSession,
        name: &str,
        patch: u32,
        now: f64,
    ) -> u32 {
        let entry = self.next_entry(&session.server_edits);
        let key = creature::template_key(entry, patch);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in creature::new_template(name) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        let subject = format!("creature_template {entry}");
        session.set_server_row(
            creature::TEMPLATE,
            &key,
            Some(&row),
            Some(crate::session::Gesture {
                label: "New creature",
                subject: &subject,
                now,
            }),
        );
        if let Some(known) = Known::from_store(&key, &row) {
            self.open_template(known);
        }
        entry
    }

    /// A copy of the template the window shows, under a new entry.
    ///
    /// Every column comes across: the database's value where the project has
    /// not changed it and the project's where it has, so the copy is what is
    /// shown rather than what the table holds. The name gets " (copy)" so the
    /// two can be told apart in a list.
    ///
    /// `None` when the whole row has not been read yet, which is the frame or
    /// two after a creature is chosen. Copying from a half-read row would take
    /// the table's defaults for the columns that had not arrived.
    pub fn copy_template(
        &mut self,
        session: &mut EditSession,
        subject: &TemplateSubject,
        patch: u32,
        now: f64,
    ) -> Option<u32> {
        let from = self
            .template
            .as_ref()
            .filter(|held| held.entry == subject.entry)?;
        let source = subject.key();
        let entry = self.next_entry(&session.server_edits);
        let key = creature::template_key(entry, patch);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for column in creature::TEMPLATE_COLUMNS
            .iter()
            .filter(|column| column.editable())
        {
            let value = session
                .server_edits
                .get(creature::TEMPLATE, &source, column.name)
                .map(str::to_string)
                .or_else(|| column.literal(&from.row));
            if let Some(value) = value {
                row.columns.insert(column.name.to_string(), value);
            }
        }
        let named = row
            .columns
            .get("name")
            .map(|literal| unquote(literal))
            .unwrap_or_default();
        row.columns.insert(
            "name".to_string(),
            vale_mangos::sql::text(&format!("{named} (copy)")),
        );
        let gesture = format!("creature_template {entry}");
        session.set_server_row(
            creature::TEMPLATE,
            &key,
            Some(&row),
            Some(crate::session::Gesture {
                label: "Copy creature",
                subject: &gesture,
                now,
            }),
        );
        if let Some(known) = Known::from_store(&key, &row) {
            self.open_template(known);
        }
        Some(entry)
    }

    /// Choose a template in the picker and open the window on it.
    fn open_template(&mut self, known: Known) {
        self.mode = super::place::Mode::Place;
        self.new_spawn.chosen = Some(known);
        self.template_window = true;
        self.template = None;
        self.created_for = None;
    }

    /// Give up a template this project created. It is in no database, so
    /// giving it up removes nothing there. A spawn of it this project created
    /// keeps its `id` and is a spawn the server refuses to load until the
    /// template exists; the panel says so through `Spawn::label`.
    pub fn discard_template(&mut self, session: &mut EditSession, entry: u32, now: f64) {
        let Some(known) = self
            .templates
            .iter()
            .find(|known| known.entry == entry)
            .cloned()
        else {
            return;
        };
        let subject = format!("creature_template {entry}");
        session.set_server_row(
            creature::TEMPLATE,
            &known.key(),
            None,
            Some(crate::session::Gesture {
                label: "Discard new creature",
                subject: &subject,
                now,
            }),
        );
        if self
            .new_spawn
            .chosen
            .as_ref()
            .is_some_and(|chosen| chosen.entry == entry)
        {
            self.new_spawn.chosen = None;
        }
        self.template = None;
        self.created_for = None;
    }

    /// Whether an entry is already a creature, as far as this tool can see:
    /// the templates behind the map's spawns, the picker's matches, the
    /// templates read for created spawns, and the ones this project creates.
    /// Asked by the entry the project gives each row and by the entry the
    /// database has it at, because the database is where an apply runs.
    ///
    /// An entry with no spawn on this map and never searched for is not seen
    /// here and is caught at the apply, which asks the database itself.
    pub fn entry_taken(&self, entry: u32) -> bool {
        self.spawns
            .iter()
            .any(|spawn| spawn.entry == entry || spawn.read_entry == entry)
            || self
                .known
                .values()
                .chain(self.new_spawn.matches.iter())
                .chain(self.templates.iter())
                .any(|known| known.entry == entry || known.read_entry == entry)
    }

    /// Move a template to another entry.
    ///
    /// Whether it may happen is [`plan_template_move`]; this performs it, and
    /// it is one operation whatever the row is: the project's claim on the row
    /// is re-keyed, so the template is one row before and after, and every
    /// later edit lands on the same claim. A row the database holds records
    /// where the database has it, which the `UPDATE` at the apply names.
    ///
    /// The project's own rows that name the entry follow it here, and the
    /// database's rows follow at the apply. See `crate::server::follow` and
    /// `vale_mangos::creature::TEMPLATE_REFERENCES`.
    pub fn rekey_template(
        &mut self,
        session: &mut EditSession,
        subject: &TemplateSubject,
        to: u32,
        now: f64,
    ) -> Result<(), String> {
        let Some((from, to_key)) = plan_template_move(subject, to, self.entry_taken(to))? else {
            return Ok(());
        };
        let at_base = creature::template_key(subject.read_entry, subject.patch);
        let gesture_subject = format!("creature_template {} entry", subject.entry);
        let gesture = crate::session::Gesture {
            label: "Move creature entry",
            subject: &gesture_subject,
            now,
        };
        crate::server::follow::rekey(
            session,
            creature::TEMPLATE,
            &from,
            &to_key,
            Some(&at_base),
            &creature::TEMPLATE_REFERENCES,
            gesture,
        )?;
        // The picker's chosen creature follows the row, so the window and the
        // ghost stay on it. A created template has no database row, so its
        // read entry moves with it.
        if let Some(chosen) = self.new_spawn.chosen.as_mut() {
            if chosen.entry == subject.entry {
                chosen.entry = to;
                if chosen.is_new() {
                    chosen.read_entry = to;
                }
            }
        }
        self.template = None;
        self.created_for = None;
        Ok(())
    }

    /// What the template window is about; see [`TemplateSubject`].
    pub fn template_subject(&self, edits: &Edits) -> Option<TemplateSubject> {
        match self.mode {
            super::place::Mode::Place => {
                let known = self.new_spawn.chosen.as_ref()?;
                Some(TemplateSubject {
                    entry: known.entry,
                    read_entry: known.read_entry,
                    patch: known.patch,
                    claim: edits.life(creature::TEMPLATE, &known.key()),
                    label: known.label(),
                })
            }
            super::place::Mode::Select => {
                let spawn = self.chosen_edited(Some(edits))?;
                Some(TemplateSubject {
                    entry: spawn.entry,
                    read_entry: spawn.read_entry,
                    patch: spawn.patch,
                    claim: edits.life(creature::TEMPLATE, &spawn.template_key()),
                    label: spawn.label(),
                })
            }
        }
    }

    /// Give every spawn and every known template the entry the project says
    /// its template has.
    ///
    /// A template whose entry the project changes is read at the entry the
    /// database has it at until the move is applied. The store keys its claim
    /// under the new entry, so what is drawn has to show it there or the row
    /// and its claim are two things: an unedited template at the old entry and
    /// a claim nobody can open. Every row is reset first, so a move taken back
    /// returns the row to the entry it was read at.
    pub fn fold_template_moves(&mut self, edits: &Edits) {
        let moves: Vec<(u32, u32, u32)> = edits
            .moves(creature::TEMPLATE)
            .filter_map(|(from, to)| {
                let patch = from
                    .0
                    .iter()
                    .find(|(column, _)| column == "patch")
                    .and_then(|(_, value)| value.parse::<u32>().ok())?;
                Some((from.first()? as u32, patch, to.first()? as u32))
            })
            .collect();
        let moved = |read_entry: u32, patch: u32| -> u32 {
            moves
                .iter()
                .find(|(from, at, _)| *from == read_entry && *at == patch)
                .map(|(_, _, to)| *to)
                .unwrap_or(read_entry)
        };
        for spawn in self.spawns.iter_mut() {
            spawn.entry = moved(spawn.read_entry, spawn.patch);
        }
        for known in self.new_spawn.matches.iter_mut() {
            known.entry = moved(known.read_entry, known.patch);
        }
        if let Some(chosen) = self.new_spawn.chosen.as_mut() {
            chosen.entry = moved(chosen.read_entry, chosen.patch);
        }
        let known: Vec<Known> = self
            .known
            .drain()
            .map(|(_, mut known)| {
                known.entry = moved(known.read_entry, known.patch);
                known
            })
            .collect();
        for known in known {
            self.known.insert(known.entry, known);
        }
    }

    /// Rebuild the templates this project creates from the store, and put
    /// them where a spawn of one and the picker find them.
    fn rebuild_templates(&mut self, edits: &Edits) {
        self.known.retain(|_, known| !known.is_new());
        let mut templates: Vec<Known> = edits
            .rows()
            .filter(|(table, _, row)| *table == creature::TEMPLATE && row.life == Life::Insert)
            .filter_map(|(_, key, row)| Known::from_store(key, row))
            .collect();
        templates.sort_by_key(|known| known.entry);
        for known in &templates {
            self.known.insert(known.entry, known.clone());
        }
        // The picker's chosen creature is a copy taken when it was clicked, so
        // a created one is replaced by its current row: the window renames it
        // and the picker shows the new name on the same frame. One whose row
        // was discarded or undone is dropped.
        let chosen = match self.new_spawn.chosen.as_ref() {
            Some(chosen) if chosen.is_new() => templates
                .iter()
                .find(|known| known.entry == chosen.entry)
                .cloned(),
            other => other.cloned(),
        };
        self.new_spawn.chosen = chosen;
        self.templates = templates;
    }

    /// Ask the camera to go and look at a spawn.
    pub fn fly_to(&mut self, at: Vec3) {
        self.fly_to = Some(at);
    }

    /// Whether a read is in flight, for the panel.
    pub fn reading(&self) -> bool {
        self.task.is_some()
    }

    /// Whether a click in the world would place a creature: the panel is on
    /// its Place half and a creature has been chosen.
    ///
    /// The counterpart of `Placing::armed()`, with the same two conditions:
    /// switching to Select or pressing `Escape` makes it false.
    pub fn placing(&self) -> bool {
        self.mode == super::place::Mode::Place && self.new_spawn.chosen.is_some()
    }

    /// Turn the creature on the cursor by `degrees` about up, snapped to the
    /// placer's step when `snap`. Does nothing before the ghost has a facing.
    /// `,` and `.` call it from `super::spawn::keys` and `Alt` with the mouse
    /// from `super::gizmo::spin`.
    pub fn turn_ghost(&mut self, degrees: f32, snap: bool) {
        if let Some(facing) = self.ghost_facing {
            self.ghost_facing = Some(super::spawn::turned(facing, degrees, snap));
        }
    }
}

/// Whether a template may move to `to`, and the two keys of the move when it
/// may: the claim's key now, and the key it is re-keyed under.
///
/// The decision apart from the writes, so it can be checked with no session.
/// `taken` is [`Creatures::entry_taken`]'s answer for `to`. Moving a row back
/// to where the database has it is always allowed: the entry is taken, and by
/// this row. `Ok(None)` is a move to the entry the row already has.
pub fn plan_template_move(
    subject: &TemplateSubject,
    to: u32,
    taken: bool,
) -> Result<Option<(Key, Key)>, String> {
    if to == subject.entry {
        return Ok(None);
    }
    if to == 0 {
        return Err("0 is not a creature entry".to_string());
    }
    if to > creature::MAX_ENTRY {
        return Err(format!(
            "{to} is above {}, the largest value `entry` can hold",
            creature::MAX_ENTRY
        ));
    }
    if taken && to != subject.read_entry {
        return Err(format!("creature_template entry {to} already exists"));
    }
    Ok(Some((
        subject.key(),
        creature::template_key(to, subject.patch),
    )))
}

/// The content patch this machine's server reads its rows at, found from the
/// same settings that locate the server.
///
/// The patch is half the key a template edit is written under. Reading it
/// from the wrong place produces an edit that applies cleanly to a row the
/// running server does not use. It is read from the same `mangosd.conf` the
/// connection is; with only `VALE_WORLDDB` set there is no conf and the
/// default is used.
pub fn server_patch(settings: &crate::server::settings::ServerSettings) -> u32 {
    use vale_mangos::conn;
    let conf = match settings.resolve() {
        Some((_, crate::server::settings::Source::Env("VALE_MANGOSD"))) => {
            std::env::var("VALE_MANGOSD").ok()
        }
        Some((_, crate::server::settings::Source::Panel)) => Some(settings.conf.clone()),
        _ => None,
    };
    let Some(conf) = conf else {
        return conn::DEFAULT_WOW_PATCH;
    };
    let at = std::path::PathBuf::from(conf);
    let file = match at.is_dir() {
        true => at.join("mangosd.conf"),
        false => at,
    };
    conn::wow_patch_from_conf(file).unwrap_or(conn::DEFAULT_WOW_PATCH)
}

pub struct CreatureToolPlugin;

impl Plugin for CreatureToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Creatures>()
            .init_resource::<ServiceMarks>()
            // After the client has put the camera where the rig says, so the
            // marks are projected through this frame's view and do not trail
            // a moving camera by a frame.
            .add_systems(
                Update,
                mark_services
                    .after(drag)
                    .after(vale_client::world::camera::place),
            )
            .add_systems(
                Update,
                // After the pick, like every tool here: reading the pointer
                // before this frame's ray is built uses last frame's ray.
                (
                    read_the_map,
                    // Before the near list, so a spawn placed this frame is
                    // in it rather than one frame late.
                    rebuild_created,
                    learn_the_created,
                    search_creatures,
                    collect_the_near,
                    aim,
                    press,
                    enclose,
                    drag,
                    fetch_rows,
                )
                    .chain()
                    .after(crate::pick::aim),
            )
            .add_systems(Update, (draw, stand_them_up, ghost, fly).after(drag))
            // `--spawn` runs once and is ordered only after the read it waits
            // for. The two that create and remove a row run after it, so
            // `--spawn-delete` acts on the map `--spawn` has already opened.
            .add_systems(
                Update,
                (
                    on_the_command_line,
                    placed_on_the_command_line,
                    template_on_the_command_line,
                    display_on_the_command_line,
                )
                    .chain()
                    .after(read_the_map),
            );
    }
}

/// Marks one of this tool's preview bodies, so they can be despawned together
/// and so nothing else mistakes one for an entity the server sent.
#[derive(Component)]
pub struct Preview {
    pub guid: u64,
    /// The display id this body was built from. A `WorldEntity`'s model is
    /// resolved once, when `spawn_models` first sees it, so a body whose
    /// display id has since changed cannot be corrected in place; it is
    /// despawned and spawned again. Without this field, editing `display_id1`
    /// left the old model standing until something else despawned it.
    pub display_id: u32,
}

/// The guid a preview body is given.
///
/// High enough that it cannot collide with anything a session puts in the
/// world: a real creature guid is a `u32` with a type mask in its high word,
/// and nothing in a session reaches this range. The spawn's own guid is in the
/// low bits so the two can be matched back up.
const PREVIEW_GUID_BASE: u64 = 0x7000_0000_0000_0000;

/// Build the spawns this project creates from the store, whenever the store
/// changes.
///
/// They are not in any database, so no query returns them. The store's own
/// columns are turned into the same `Spawn` the map query produces, and from
/// there they are drawn, picked, dressed and edited by the same code as the
/// others.
///
/// Keyed on `EditSession::server_edit_revision`, which changes on every edit,
/// unlike the map read, which is keyed on the last apply. The work is a walk
/// of the store, which holds only the project's edits rather than a whole map,
/// so a walk per edit covers a few rows.
fn rebuild_created(
    mut creatures: ResMut<Creatures>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !state.editing() || *tool != Tool::Creatures {
        return;
    }
    let Some(session) = session else { return };
    if creatures.created_for == Some(session.server_edit_revision) {
        return;
    }
    creatures.created_for = Some(session.server_edit_revision);
    // The templates first: a created spawn below is labelled from `known`,
    // which the created templates are put into here, and every entry is
    // folded through the project's moves before anything reads it.
    creatures.fold_template_moves(&session.server_edits);
    creatures.rebuild_templates(&session.server_edits);

    let mut out: Vec<Spawn> = Vec::new();
    for (table, key, row) in session.server_edits.rows() {
        if table != creature::SPAWN || row.life != Life::Insert {
            continue;
        }
        let Some(guid) = key.first() else { continue };
        // Skip a spawn the read already returned. Once a created spawn has
        // been applied it is a row in the database, so the next read of the
        // map includes it. Listing it here as well would draw the creature
        // twice, in two places once one copy was dragged. The read's copy is
        // drawn; `with_edits` lays the project's own columns over it as it
        // does for any other row.
        if creatures.spawns.iter().any(|spawn| spawn.guid == guid) {
            continue;
        }
        let number =
            |column: &str| -> Option<f32> { row.columns.get(column)?.trim().parse::<f32>().ok() };
        // Only this map's spawns. A project can carry spawns on several maps
        // and the viewport shows one; a row for another map drawn here would
        // put a creature at another map's coordinates.
        if number("map").map(|map| map as u32) != Some(session.map_id) {
            continue;
        }
        let entry = number("id").unwrap_or(0.0) as u32;
        let known = creatures.known.get(&entry);
        out.push(Spawn {
            guid,
            entry,
            read_entry: entry,
            at: Vec3::new(
                number("position_x").unwrap_or(0.0),
                number("position_y").unwrap_or(0.0),
                number("position_z").unwrap_or(0.0),
            ),
            orientation: number("orientation").unwrap_or(0.0),
            wander: number("wander_distance").unwrap_or(0.0),
            movement_type: number("movement_type").unwrap_or(0.0) as u32,
            // Until the template has been read the row is labelled by its
            // entry and has no model, so it is drawn as a dome.
            // See [`learn_the_created`].
            name: known.map(|known| known.name.clone()),
            subname: known.and_then(|known| known.subname.clone()),
            display_id: known.map(|known| known.display_id).unwrap_or(0),
            display_scale: known.map(|known| known.display_scale).unwrap_or(0.0),
            level: known.map(|known| known.level).unwrap_or((0, 0)),
            faction: known.map(|known| known.faction).unwrap_or(0),
            npc_flags: known.map(|known| known.npc_flags).unwrap_or(0),
            rank: known.map(|known| known.rank).unwrap_or(0),
            patch: known.map(|known| known.patch).unwrap_or(0),
            claim: Life::Insert,
        });
    }
    creatures.created = out;
    // The near list indexes both lists, so it is out of date as soon as
    // either changes.
    creatures.built_at = None;
}

/// Read the templates of creatures this project has spawns of and the map
/// does not.
///
/// Everything the map query returns is already in [`Creatures::known`], and
/// the picker adds whatever was searched for, so this covers one case: a
/// project opened in a later session whose created spawns name a creature
/// with no other spawn on this map. Without it those spawns are drawn as a
/// ring labelled by an entry and never get a model.
///
/// One query for every entry that is missing rather than one each, and it does
/// not run again while a read is in flight.
fn learn_the_created(
    mut creatures: ResMut<Creatures>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    session: Option<Res<EditSession>>,
) {
    // The templates held were read from the database as it stood at the
    // counter; after an apply or a put back they are read again, and a read
    // still running is dropped with them.
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    if creatures.known.renew(writes) {
        creatures.known_task = None;
        creatures.created_for = None;
    }
    if let Some(task) = creatures.known_task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            creatures.known_task = None;
            match done {
                Ok(rows) => {
                    for known in rows {
                        creatures.known.insert(known.entry, known);
                    }
                    // What was read changes what a created spawn is drawn as.
                    creatures.created_for = None;
                }
                Err(e) => warn!("creature templates: {e}"),
            }
        }
        return;
    }
    if !state.editing() || *tool != Tool::Creatures {
        return;
    }
    let missing: Vec<u32> = creatures
        .created
        .iter()
        .map(|spawn| spawn.entry)
        .filter(|entry| *entry != 0 && !creatures.known.contains_key(entry))
        .collect::<HashSet<u32>>()
        .into_iter()
        .collect();
    if missing.is_empty() {
        return;
    }
    let Some((at, _)) = settings.resolve() else {
        return;
    };
    let patch = server_patch(&settings);
    // Recorded before the read, not after. Otherwise two frames with a read
    // in flight would start two reads, and an entry the database has no row
    // for would start a read on every frame.
    for entry in &missing {
        creatures.known.insert(*entry, unknown_creature(*entry));
    }
    creatures.known_task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let Some(sql) = creature::templates_query(&missing, patch) else {
            return Ok(Vec::new());
        };
        Ok(db.rows(&sql)?.iter().filter_map(read_known).collect())
    }));
}

/// A creature entry with no template row at this patch. vmangos refuses to
/// load a spawn of it.
///
/// Held in the table rather than left absent, so the read that failed to find
/// it is not started again on the next frame. Its name states the fault in the
/// same words the map query uses for a spawn whose join found no row.
fn unknown_creature(entry: u32) -> Known {
    Known {
        entry,
        read_entry: entry,
        claim: Life::Update,
        patch: 0,
        name: format!("entry {entry} — no template row"),
        subname: None,
        display_id: 0,
        display_scale: 0.0,
        level: (0, 0),
        faction: 0,
        npc_flags: 0,
        rank: 0,
    }
}

/// Run the picker's search when the typed text differs from the last search
/// sent.
///
/// One query per change of the box rather than per frame: `ran` holds what
/// was sent, so a frame that redraws the same text sends nothing. A term of
/// fewer than two characters is not sent at all: `%a%` matches most of the
/// table and returns fifty arbitrary rows.
fn search_creatures(
    mut creatures: ResMut<Creatures>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    session: Option<Res<EditSession>>,
) {
    // The matches were read from the database as it stood at the counter; an
    // apply or a put back empties them, drops a search still running, and
    // sends the typed text again.
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    if creatures.new_spawn.searched_at != Some(writes) {
        creatures.new_spawn.searched_at = Some(writes);
        creatures.new_spawn.task = None;
        creatures.new_spawn.forget();
    }
    if let Some(task) = creatures.new_spawn.task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            creatures.new_spawn.task = None;
            match done {
                Ok(rows) => {
                    for known in &rows {
                        creatures.known.insert(known.entry, known.clone());
                    }
                    creatures.new_spawn.matches = rows;
                    creatures.new_spawn.trouble = None;
                    // The matches are read at the entries the database has,
                    // and a template this project moves is listed at its new
                    // one; see `fold_template_moves`.
                    creatures.created_for = None;
                }
                Err(e) => {
                    creatures.new_spawn.matches.clear();
                    creatures.new_spawn.trouble = Some(e);
                }
            }
        }
        return;
    }
    if !state.editing() || *tool != Tool::Creatures || creatures.mode != super::place::Mode::Place {
        return;
    }
    let term = creatures.new_spawn.search.trim().to_string();
    if term == creatures.new_spawn.ran {
        return;
    }
    creatures.new_spawn.ran = term.clone();
    if term.len() < 2 {
        creatures.new_spawn.matches.clear();
        return;
    }
    let Some((at, _)) = settings.resolve() else {
        creatures.new_spawn.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = server_patch(&settings);
    creatures.new_spawn.task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let sql = creature::search_query(&term, patch, MATCHES);
        Ok(db.rows(&sql)?.iter().filter_map(read_known).collect())
    }));
}

/// Read the open map's spawns, on a task, when the map or the project's
/// applied edits have changed.
fn read_the_map(
    mut creatures: ResMut<Creatures>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    // Collect a finished read whatever the tool is, so switching away and back
    // does not discard a read that has already run.
    if let Some(task) = creatures.task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            creatures.task = None;
            match done {
                Ok(read) => {
                    info!("creatures: {} spawn(s) read", read.spawns.len());
                    // What the map query already says about each creature, so
                    // a spawn this project creates of one of them is drawn
                    // without a read of its own.
                    for spawn in &read.spawns {
                        if let Some(known) = known_from(spawn) {
                            creatures.known.insert(known.entry, known);
                        }
                    }
                    creatures.spawns = read.spawns;
                    creatures.max_guid = read.highest;
                    creatures.max_entry = read.highest_entry;
                    creatures.trouble = None;
                    creatures.built_at = None;
                    creatures.created_for = None;
                }
                Err(e) => {
                    warn!("creatures: {e}");
                    creatures.spawns.clear();
                    creatures.trouble = Some(e);
                }
            }
        }
        return;
    }
    if !state.editing() || *tool != Tool::Creatures {
        return;
    }
    let Some(session) = session else { return };
    // Keyed on `EditSession::database_writes`, which every apply and every
    // Put back of any subject increments. It was keyed on
    // `EditSession::applied_creatures`, which a Put back sets to `None`; in a
    // session that did not make the apply it was already `None`, so the key
    // did not change and spawns the Put back had deleted stayed on the map.
    // It is not keyed on the spell tables' signature either: a spell save
    // would re-read every creature on the map. The game-object tool keys the
    // same way.
    let key = (session.map_id, session.database_writes);
    if creatures.loaded == Some(key) {
        return;
    }
    // Set before the read, not after, so a failed read is not retried on every
    // frame; with no server running that would be a connection attempt sixty
    // times a second.
    creatures.loaded = Some(key);
    let Some((at, _source)) = settings.resolve() else {
        creatures.spawns.clear();
        creatures.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = server_patch(&settings);
    let map = session.map_id;
    creatures.trouble = None;
    creatures.task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let rows = db.rows(&creature::spawns_on_map_query(map, patch))?;
        // The highest guid in the whole table, not this map's. Guids are
        // global, so a new spawn numbered from one map's highest would collide
        // with a row on another. It adds one statement to a read that already
        // takes half a second.
        let highest = db
            .row(creature::MAX_SPAWN_GUID_QUERY)?
            .and_then(|row| row.integer("guid"))
            .map(|guid| guid as u64);
        // The highest template entry at any patch, for a new template's
        // number. One numbered from the rows the server loads would collide
        // with a row at a patch the server is not loading.
        let highest_entry = db
            .row(creature::MAX_ENTRY_QUERY)?
            .and_then(|row| row.integer("entry"))
            .map(|entry| entry as u32);
        Ok(MapRead {
            spawns: rows.iter().map(read_spawn).collect(),
            highest,
            highest_entry,
        })
    }));
}

/// The result of one read of the map: its spawns, the highest guid the whole
/// `creature` table holds, and the highest entry `creature_template` holds.
///
/// Neither maximum is specific to the map, since guids and entries are global.
/// They are read here because the read uses the same connection, and because
/// placing a spawn or creating a template, which need them, is only possible
/// once the map has been read.
#[derive(Debug)]
pub struct MapRead {
    pub spawns: Vec<Spawn>,
    pub highest: Option<u64>,
    pub highest_entry: Option<u32>,
}

/// What the map query already says about a creature, which is everything a
/// created spawn of the same creature needs.
fn known_from(spawn: &Spawn) -> Option<Known> {
    Some(Known {
        entry: spawn.entry,
        read_entry: spawn.read_entry,
        claim: Life::Update,
        patch: spawn.patch,
        name: spawn.name.clone()?,
        subname: spawn.subname.clone(),
        display_id: spawn.display_id,
        display_scale: spawn.display_scale,
        level: spawn.level,
        faction: spawn.faction,
        npc_flags: spawn.npc_flags,
        rank: spawn.rank,
    })
}

/// One row of the map query as the viewport needs it.
fn read_spawn(row: &creature::Row) -> Spawn {
    let number = |column: &str| row.number(column).unwrap_or(0.0) as f32;
    let integer = |column: &str| row.integer(column).unwrap_or(0) as u32;
    Spawn {
        guid: row.integer("guid").unwrap_or(0) as u64,
        entry: integer("id"),
        read_entry: integer("id"),
        at: Vec3::new(
            number("position_x"),
            number("position_y"),
            number("position_z"),
        ),
        orientation: number("orientation"),
        wander: number("wander_distance"),
        movement_type: integer("movement_type"),
        name: row.text("name").map(str::to_string),
        subname: row.text("subname").map(str::to_string),
        display_id: integer("display_id1"),
        display_scale: number("display_scale1"),
        level: (integer("level_min"), integer("level_max")),
        faction: integer("faction"),
        npc_flags: integer("npc_flags"),
        rank: integer("rank"),
        patch: integer("patch"),
        // A row read from the database exists, so it is an update. Any other
        // state is the project's claim, laid over by `with_edits`.
        claim: Life::Update,
    }
}

/// Collect the spawns near enough to draw, nearest first.
///
/// Rebuilt when the focus crosses a tile rather than per frame: it walks 24,610
/// rows and sorts what it keeps, which is too much to do every frame. A tile is
/// 533 yards, so the list is never more than that out of date.
fn collect_the_near(
    mut creatures: ResMut<Creatures>,
    focus: Res<WorldFocus>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !state.editing() || *tool != Tool::Creatures {
        return;
    }
    if !focus.present {
        return;
    }
    let tile = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    if creatures.built_at == Some(tile) {
        return;
    }
    creatures.built_at = Some(tile);
    let eye = focus.position;
    let range = creatures.range;
    // Both lists, the database's spawns and the ones this project creates,
    // under one index space. See [`Creatures::base_spawn`].
    let mut near: Vec<(i64, usize)> = (0..creatures.spawn_count())
        .filter_map(|index| {
            let away = creatures.base_spawn(index)?.at.distance(eye);
            (away <= range).then_some((away as i64, index))
        })
        .collect();
    near.sort_unstable();
    creatures.near = near.into_iter().map(|(_, index)| index).collect();
}

/// The pick radius of a spawn's marker, in pixels.
///
/// The light tool does the same, for the same reason: a position is a point,
/// and a radius in yards is too small to click far away and covers the screen
/// close up. Wider than a light's twelve, because a creature marker is a body
/// rather than a dot and the whole body is the target.
const HANDLE_PIXELS: f32 = 18.0;

/// How far from the original a duplicate is placed, in yards: two.
///
/// Far enough that the two are separate marks at editing distance, and near
/// enough that the copy is clearly next to its original.
const APART: f32 = 2.0;

/// The key a spawn's edits are written under. A local name because
/// [`Creatures::duplicate`] needs the new row's key while it is still holding
/// the old one's.
fn key_for(guid: u64) -> Key {
    creature::spawn_key(guid)
}

/// The marker colour of a spawn this project creates, which is in no database
/// yet.
const NEW: Color = Color::srgb(0.45, 0.85, 1.0);

/// The marker colour of a spawn this project removes. It is still drawn until
/// Apply.
const REMOVED: Color = Color::srgb(0.95, 0.30, 0.30);

/// The marker colour of a selected spawn that is not the primary: the
/// primary's gold, paler. `pub(crate)` because the game-object tool draws its
/// members in it too.
pub(crate) const MEMBER: Color = Color::srgb(0.95, 0.78, 0.50);

/// The smallest dome drawn around a spawn's model, in yards: about the
/// height of a person's waist, so the dome reads as around the model.
const DOME_AROUND_MODEL: f32 = 1.2;

/// How far up from the row's position the marker's body reaches, in yards.
///
/// A creature model stands on its position, so the pointer is aimed at
/// something above the ground rather than at the ground itself.
const BODY: f32 = 2.0;

/// Find the spawn the pointer is over.
fn aim(
    mut creatures: ResMut<Creatures>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !state.editing() || *tool != Tool::Creatures {
        creatures.hovered = None;
        return;
    }
    // A held drag keeps what it grabbed.
    if creatures.drag.is_some() {
        return;
    }
    // Nothing is hovered while a creature is on the cursor. A highlighted
    // marker under a ghost would indicate that a click selects it, and the
    // click places instead.
    if creatures.placing() {
        creatures.hovered = None;
        return;
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        creatures.hovered = None;
        return;
    }
    let Some((origin, direction)) = crate::pick::ray(&windows, &camera) else {
        creatures.hovered = None;
        return;
    };
    let Ok(window) = windows.single() else { return };
    let Ok((camera, _)) = camera.single() else {
        return;
    };
    let height = window.height().max(1.0);
    // The projection's [1][1] is `1 / tan(fov / 2)`, which turns a pixel height
    // into an angle without asking what kind of projection it is.
    let fov = match camera.clip_from_view().to_cols_array()[5] {
        y if y.abs() > f32::EPSILON => y,
        _ => return,
    };

    // The pointer aims at where a creature is drawn, which is where the
    // project's edits put it rather than where the database does.
    let edits = session.as_ref().map(|session| &session.server_edits);
    let mut best: Option<(f32, u64)> = None;
    for index in &creatures.near {
        let Some(spawn) = creatures.spawn_at(*index, edits) else {
            continue;
        };
        let middle = spawn.at + Vec3::Z * (BODY / 2.0);
        let to = middle - origin;
        let along = to.dot(direction);
        if along <= 0.0 {
            continue;
        }
        // The handle's radius in yards at the spawn's own distance, so it is
        // the same size on screen at any distance. The body's half height is
        // the minimum, so a creature close to the camera is picked by its body
        // rather than by a dot at its feet.
        let grabbable = ((HANDLE_PIXELS / height) * 2.0 * along / fov).max(BODY / 2.0);
        if (to - direction * along).length() > grabbable {
            continue;
        }
        if best.is_none_or(|(nearest, _)| along < nearest) {
            best = Some((along, spawn.guid));
        }
    }
    creatures.hovered = best.map(|(_, guid)| guid);
}

/// How far the pointer must travel before a drag moves anything, in pixels.
///
/// A click on a creature selects it; moving it is a second gesture. Without a
/// threshold every click moved the spawn a fraction of a yard, because the
/// pointer moves between the press and the release of an ordinary click and
/// the ground under it is a different point.
///
/// Six is about the travel of an ordinary click, and well under the travel of
/// an intended drag.
const DRAG_PIXELS: f32 = 6.0;

/// Place the chosen creature on the ground under the pointer.
///
/// This is the only action in this tool that creates a row rather than
/// changing one. It is armed by the panel and acts on the next click, the same
/// two-gesture rule a move follows: arming is deliberate, and until it has
/// happened a click on the world selects.
///
/// The creature faces the camera. vmangos' own `.npc add` gives the creature
/// the player's orientation, so it faces the way the player placing it is
/// looking. The editor's camera looks at the spot rather than from it, so the
/// equivalent here is the creature facing the camera. The facing is the
/// `orientation` column, and a drag on that field changes it.
#[allow(clippy::too_many_arguments)]
fn place_one(
    creatures: &mut Creatures,
    session: &mut Option<ResMut<EditSession>>,
    cursor: &crate::pick::Cursor,
    viewport: &crate::ui::Viewport,
    wants: &bevy_egui::input::EguiWantsInput,
    windows: &Query<&Window>,
    camera: &Query<&GlobalTransform, With<WorldCamera>>,
    time: &Time,
) {
    if !crate::ui::over_the_world(viewport, wants, windows) {
        return;
    }
    let Some(known) = creatures.new_spawn.chosen.clone() else {
        return;
    };
    let Some(session) = session.as_mut() else {
        return;
    };
    // Without a surface under the pointer nothing is placed. A height guessed
    // from the camera would put the creature inside a hill or in the air, and
    // the server drops a spawn with a wrong `position_z` to the floor of
    // whatever it is over. The click is ignored and the placer stays armed,
    // so the next click on something solid places.
    //
    // The surface and not the ground, so that clicking a bridge, a dock or an
    // inn's upper floor stands the creature on it. See [`crate::pick`].
    let Some(ground) = cursor.surface else {
        session.status = "no surface under the pointer; nothing placed".into();
        return;
    };
    // The ghost's own facing, so the row written matches the model that was
    // on screen. The camera fallback covers a click before the ghost has been
    // given a facing, which is the frame the placer is armed on.
    let facing = creatures
        .ghost_facing
        .unwrap_or_else(|| match camera.single() {
            Ok(camera) => {
                let eye = Vec3::from(vale_client::render::axes::to_wow(camera.translation()));
                (eye.y - ground.y).atan2(eye.x - ground.x)
            }
            Err(_) => 0.0,
        });
    let guid = creatures.create(session, &known, ground, facing, time.elapsed_secs_f64());
    creatures.template_window = false;
    session.status = format!("{} placed as guid {guid}", known.label());
    info!(
        "creatures: placed {} as guid {guid} at {:.1}, {:.1}, {:.1}",
        known.label(),
        ground.x,
        ground.y,
        ground.z
    );
}

/// Select the spawn under the pointer.
///
/// A press on a creature that is not selected selects it and does nothing else.
/// A press on one that is already selected arms a drag, which moves nothing
/// until the pointer has travelled [`DRAG_PIXELS`]. Moving a creature is
/// therefore two gestures, click then drag, and the first cannot move it.
#[allow(clippy::too_many_arguments)]
pub fn press(
    mut creatures: ResMut<Creatures>,
    mut session: Option<ResMut<EditSession>>,
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<crate::pick::Cursor>,
    time: Res<Time>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    waypoints: Res<super::waypoints::Waypoints>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
    gizmo: Res<super::gizmo::Gizmo>,
    keys: Res<ButtonInput<KeyCode>>,
    mut marquee: ResMut<super::group::Marquee>,
) {
    if !state.editing() || *tool != Tool::Creatures {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    // A press on a gizmo handle is the gizmo's, which runs first; it must
    // not also select or arm a drag on what is behind the handle.
    if gizmo.holding() {
        return;
    }
    // The waypoint mode takes the pointer when it is using it. It runs first,
    // and without this check a click it took would also land here: a press on
    // a node would also select the creature under it, and with Add armed every
    // point placed would re-select the creature it was placed over. The two
    // conditions are the ones `waypoints::press` acts on, so a click that mode
    // ignores still reaches this one.
    //
    // This check is also ahead of the placer below. That is the one state in
    // which two modes both want the click. Arming Place disarms the waypoint
    // mode, so the state is reached only by arming the waypoints afterwards,
    // and then the open waypoint window takes the click.
    if waypoints.open.is_some() && (waypoints.adding || waypoints.hovered.is_some()) {
        return;
    }
    // A click in Place creates a spawn and does nothing else. It does not
    // also select whatever was under the pointer, which would open the form
    // on a creature the user did not click.
    if creatures.placing() {
        place_one(
            &mut creatures,
            &mut session,
            &cursor,
            &viewport,
            &wants,
            &windows,
            &camera,
            &time,
        );
        return;
    }
    let adding = super::group::shift(&keys);
    let pointer = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position())
        .unwrap_or_default();
    let Some(guid) = creatures.hovered else {
        // Nothing under the pointer: a rectangle begins. A click on nothing
        // leaves the selection alone, unlike the doodad tool's: the form,
        // the template window and the waypoint window all follow the selected
        // spawn, and an empty click is how a waypoint node is let go of.
        if crate::ui::over_the_world(&viewport, &wants, &windows) {
            marquee.begin(Tool::Creatures, pointer, adding);
        }
        return;
    };
    if adding {
        creatures.toggle(guid);
        return;
    }
    if !creatures.holds(guid) {
        // The form draws the two whole rows, and they belong to the selection
        // rather than to the frame. `select_only` clears them, which makes
        // `fetch_rows` read the new selection's rows.
        //
        // No drag is armed. The press that selects a creature does not move
        // it; see this function's doc.
        creatures.select_only(Some(guid));
        return;
    }
    // A press on a member of the group keeps the group, so the drag that
    // follows moves all of it.
    let collapse = !creatures.also.is_empty();
    creatures.promote(guid);
    // Where each is drawn, which is where the project's edits put it. See
    // [`Spawn::with_edits`].
    let edits = session.as_ref().map(|session| &session.server_edits);
    let from = creatures
        .chosen_edited(edits)
        .map(|spawn| spawn.at)
        .unwrap_or_default();
    let group_from: Vec<(u64, Vec3)> = creatures
        .also
        .iter()
        .filter_map(|&guid| creatures.edited(guid, edits).map(|spawn| (guid, spawn.at)))
        .collect();
    creatures.group_from = group_from;
    creatures.drag = Some(Drag {
        guid,
        from,
        pressed_at: pointer,
        grab: None,
        moving: false,
        collapse,
    });
}

/// Select every drawn spawn whose marker is inside a finished rectangle.
///
/// The spawns considered are the ones drawn — [`Creatures::near`] — for the
/// reason the doodad tool encloses only what is drawn: what is not on screen
/// cannot be seen to be inside the rectangle. See [`super::group`].
fn enclose(
    mut creatures: ResMut<Creatures>,
    mut marquee: ResMut<super::group::Marquee>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if *tool != Tool::Creatures {
        return;
    }
    let Some(done) = marquee.finished(Tool::Creatures) else {
        return;
    };
    let Some((camera, eye, window)) = super::group::view(&camera, &windows) else {
        return;
    };
    let edits = session.as_ref().map(|session| &session.server_edits);
    let mut found: Vec<(f32, u64)> = Vec::new();
    for index in &creatures.near {
        let Some(spawn) = creatures.spawn_at(*index, edits) else {
            continue;
        };
        let middle = spawn.at + Vec3::Z * (BODY / 2.0);
        let Some(point) = super::group::on_screen(camera, eye, window, middle) else {
            continue;
        };
        if done.rect.contains(point) {
            found.push((point.distance(done.rect.center()), spawn.guid));
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    if !done.adding {
        creatures.select_only(None);
    }
    for (_, guid) in found {
        if creatures.holds(guid) {
            continue;
        }
        match creatures.selected {
            None => creatures.make_primary(Some(guid)),
            Some(_) => creatures.also.push(guid),
        }
    }
}

/// Move the held spawn, once the pointer has travelled far enough.
///
/// The three position columns are written to the project's store on every
/// frame of the drag, under one gesture. This is what makes the creature
/// follow the pointer: the drawing reads the store over the database's
/// reading (see [`Spawn::with_edits`]), so a drag that wrote only at the end
/// would move nothing until the button was released.
///
/// It adds nothing to the file or the undo stack. The store is a map keyed
/// by column, so sixty writes to `position_x` are one entry; the gesture names
/// the row, so the three columns fold into one undo entry rather than three;
/// and `set_server_edit` returns early when the value has not changed, so a
/// held pointer that is not moving writes nothing.
fn drag(
    mut creatures: ResMut<Creatures>,
    mut session: Option<ResMut<EditSession>>,
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<crate::pick::Cursor>,
    time: Res<Time>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    windows: Query<&Window>,
) {
    if !state.editing() || *tool != Tool::Creatures {
        creatures.drag = None;
        return;
    }
    let Some(held) = creatures.drag else { return };
    let Some(session) = session.as_mut() else {
        return;
    };

    if !buttons.pressed(MouseButton::Left) {
        creatures.drag = None;
        creatures.group_from.clear();
        session.history.release();
        if held.moving {
            let now = creatures
                .chosen_edited(Some(&session.server_edits))
                .map(|spawn| spawn.at)
                .unwrap_or(held.from);
            session.status = match creatures.also.len() {
                0 => format!(
                    "moved spawn {} by {:.1} yards",
                    held.guid,
                    now.distance(held.from)
                ),
                more => format!(
                    "moved {} spawns by {:.1} yards",
                    more + 1,
                    now.distance(held.from)
                ),
            };
        } else if held.collapse {
            // A press on a member that never moved was a click on it.
            creatures.also.clear();
        }
        return;
    }

    // Nothing moves until the pointer has travelled from the press point. See
    // [`DRAG_PIXELS`]: without it a click moved the spawn a fraction of a yard.
    if !held.moving {
        let at = windows
            .single()
            .ok()
            .and_then(|window| window.cursor_position())
            .unwrap_or(held.pressed_at);
        if at.distance(held.pressed_at) < DRAG_PIXELS {
            return;
        }
        creatures.drag = Some(Drag {
            moving: true,
            ..held
        });
        return;
    }

    // The surface under the pointer: the ground, or a building or scenery
    // model standing on it. Without a surface nothing moves; a height guessed
    // from the camera would drop the creature through the floor of whatever
    // it is standing in.
    let Some(ground) = cursor.surface else { return };
    let grab = match held.grab {
        Some(grab) => grab,
        None => {
            // Measured on the first frame the drag is moving, and nothing is
            // written on that frame. The offset is the difference between the
            // creature's position and the point where the pointer met the
            // ground; keeping it stops the creature jumping under the cursor.
            let grab = held.from - ground;
            creatures.drag = Some(Drag {
                grab: Some(grab),
                ..held
            });
            return;
        }
    };
    // The horizontal offset is kept and the height is the ground's, so a
    // creature dragged across a hill follows the hill rather than flying at the
    // height it was picked up from.
    let want = Vec3::new(ground.x + grab.x, ground.y + grab.y, ground.z);
    let now = time.elapsed_secs_f64();
    // One entry from the first write to the release, whatever pauses the
    // pointer makes on the way. See `vale_edit::undo::History::hold`.
    session.history.hold(now);
    if creatures.group_from.is_empty() {
        move_one(session, held.guid, want, now);
        return;
    }
    // The rest of the group by the same step across the ground, each keeping
    // its own height above the terrain — see [`super::group::carried`].
    let step = want - held.from;
    let label = format!("Move {} creatures", creatures.group_from.len() + 1);
    let subject = format!("creature group {} position", held.guid);
    let members = creatures.group_from.clone();
    session.as_one(&label, &subject, |session| {
        move_one(session, held.guid, want, now);
        for (guid, from) in members {
            let to = super::group::carried(session, from, from + step);
            move_one(session, guid, to, now);
        }
    });
}

/// Write one spawn's three position columns under one gesture subject, so a
/// move is one entry on the undo stack. Keyed on the guid rather than on the
/// column, as the form keys it.
fn move_one(session: &mut EditSession, guid: u64, to: Vec3, now: f64) {
    let key = creature::spawn_key(guid);
    let subject = format!("creature {guid} position");
    for (column, value) in [
        ("position_x", to.x),
        ("position_y", to.y),
        ("position_z", to.z),
    ] {
        session.set_server_edit(
            creature::SPAWN,
            &key,
            column,
            Some(sql_float(value)),
            Some(crate::session::Gesture {
                label: "Move creature",
                subject: &subject,
                now,
            }),
        );
    }
}

/// A float as this crate writes it into a statement: `vale_mangos::sql`'s
/// own rule, so a position and a name are escaped in one place.
fn sql_float(value: f32) -> String {
    vale_mangos::sql::float(value)
}

/// Draw a dome over every near spawn, and the selected one's facing and wander
/// circle.
///
/// A spawn drawn as a model gets a translucent dome around its feet; a spawn
/// with no model gets a smaller, near-opaque dome where the model would
/// stand.
///
/// A mark that is being aimed at is [`Look::Ghosted`], so it shows faintly
/// through a wall. Every other mark is [`Look::Solid`] and hidden by whatever is
/// nearer the camera: the tool marks up to a thousand spawns, and in a forest
/// ghosts of all of them would cover the trees.
fn draw(
    mut marks: ResMut<Marks>,
    creatures: Res<Creatures>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
) {
    if !state.editing() || *tool != Tool::Creatures {
        return;
    }
    let Ok(camera) = camera.single() else { return };
    let eye = Vec3::from(vale_client::render::axes::to_wow(camera.translation()));

    let edits = session.as_ref().map(|session| &session.server_edits);
    // Sorted once and searched per marker — see `super::doodads::draw_marker`.
    let members = super::group::sorted(creatures.also.iter().copied());
    for (rank, index) in creatures.near.iter().enumerate() {
        // The spawn as drawn, not as read. A spawn this project creates is in
        // neither the database nor `spawns`, and one it removes has to be
        // drawn as removed.
        //
        // `with_edits` returns `None` for a row the project does not edit,
        // which is all but a few spawns on the map, and the base is borrowed
        // in that case. This runs over up to a thousand markers a frame, and
        // cloning each would clone two `String`s per marker to no purpose.
        let Some(base) = creatures.base_spawn(*index) else {
            continue;
        };
        let with_edits = edits.and_then(|edits| base.with_edits(edits));
        let spawn = with_edits.as_ref().unwrap_or(base);
        let chosen = creatures.selected == Some(spawn.guid);
        let member = super::group::holds(&members, spawn.guid);
        let under = creatures.hovered == Some(spawn.guid);
        // A spawn drawn as a model gets a foot ring rather than a full marker:
        // the model shows where it is, and a marker line would be drawn
        // through the creature.
        let has_model = creatures.show_models && rank < creatures.model_budget;
        // Edited spawns are drawn in the warning colour whether or not they are
        // chosen, so the user can see which spawns have been changed. Nothing
        // else in the tool shows that.
        let edited = edits.is_some_and(|edits| edits.touches(creature::SPAWN, &spawn.key()));
        // A removal is red in every state, including while it is selected.
        // The other colours all mean the project changes the spawn, and a
        // removal is a different kind of claim. A creation has its own colour
        // for the same reason, at lower priority.
        let colour = match (spawn.claim, chosen, under, edited) {
            (Life::Delete, _, _, _) => REMOVED,
            (_, true, _, _) => Color::srgb(1.0, 0.82, 0.25),
            (_, _, true, _) => Color::WHITE,
            // The rest of a group, paler than the primary.
            _ if member => MEMBER,
            (Life::Insert, _, _, _) => NEW,
            (_, _, _, true) => Color::srgb(0.95, 0.55, 0.20),
            _ => Color::srgb(0.45, 0.80, 0.55),
        };
        let chosen_or_member = chosen || member;
        let at = vale_client::render::axes::to_bevy(spawn.at.to_array());
        let radius = ((spawn.at.distance(eye)) * 0.012).clamp(0.35, 4.0);
        let look = match chosen_or_member || under {
            true => Look::Ghosted,
            false => Look::Solid,
        };
        match has_model {
            // Around the model's feet, never smaller than a person.
            true => marks.dome(at, radius.max(DOME_AROUND_MODEL), colour.with_alpha(0.4), look),
            false => marks.dome(at, radius, colour.with_alpha(0.85), look),
        }
        if spawn.is_removed() {
            // A cross through it, because the colour alone has to be learned
            // and this is the one state in which a mistake deletes a row.
            // Ghosted, so a creature marked for removal still shows through a
            // wall in front of it.
            let arm = radius * 1.4;
            for (dx, dy) in [(1.0, 1.0), (1.0, -1.0)] {
                let from = vale_client::render::axes::to_bevy(
                    (spawn.at + Vec3::new(arm * dx, arm * dy, BODY / 2.0)).to_array(),
                );
                let to = vale_client::render::axes::to_bevy(
                    (spawn.at - Vec3::new(arm * dx, arm * dy, -BODY / 2.0)).to_array(),
                );
                let thickness = marks.line_radius(from) * 1.6;
                marks.tube(from, to, thickness, REMOVED, Look::Ghosted);
            }
        }
        if chosen {
            // Which way it faces, and how far it wanders. Both are columns of
            // the row the form has open, and both are hard to read as numbers:
            // 4.71 radians does not show a direction.
            let facing = Vec3::new(spawn.orientation.cos(), spawn.orientation.sin(), 0.0);
            let from = vale_client::render::axes::to_bevy((spawn.at + Vec3::Z * 0.2).to_array());
            let nose = vale_client::render::axes::to_bevy(
                (spawn.at + facing * (radius * 3.0) + Vec3::Z * 0.2).to_array(),
            );
            let thickness = marks.line_radius(from) * 1.6;
            marks.arrow(from, nose, thickness, Color::srgb(1.0, 0.95, 0.6), Look::Ghosted);
            if spawn.wander > 0.1 {
                marks.flat_ring(at, spawn.wander, Color::srgba(1.0, 0.82, 0.25, 0.5), Look::Ghosted);
            }
        }
    }
}

/// Where one creature's service icons are drawn this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ServiceMark {
    /// The point over the creature's head, in physical pixels: the world
    /// camera draws into a target sized in physical pixels, and that is what
    /// its projection answers in.
    pub at: Vec2,
    /// The creature's `npc_flags`, with this project's edits applied.
    pub npc_flags: u32,
    /// Yards from the camera, which the painter fades the far ones by.
    pub away: f32,
}

/// The service marks to draw this frame, nearest last so a near creature's
/// icons are painted over a far one's. Written by [`mark_services`] and drawn
/// by `crate::ui::servicemarks`.
///
/// A resource of its own rather than a field of [`Creatures`]: it is written
/// every frame, and [`Creatures`] is read by passes that would then see it as
/// changed every frame.
#[derive(Resource, Default)]
pub struct ServiceMarks(pub Vec<ServiceMark>);

/// How far from the camera a creature's services are still marked, in yards.
///
/// The icons are a fixed size on screen, so past this distance they are
/// larger than the creature they mark and a town reads as a block of icons.
pub const SERVICE_MARK_RANGE: f32 = 150.0;

/// How far above the name point, or above a marker's top, the icons sit, in
/// yards.
const SERVICE_MARK_LIFT: f32 = 0.35;

/// The `npc_flags` bits that are services. `GOSSIP` (0x1) is left out: nearly
/// every flagged creature carries it and it names no service.
pub const SERVICE_BITS: u32 = 0x0000_7FFE;

/// Project a point over the head of every near creature that offers a service.
///
/// A creature drawn as a model is marked over the model's name point, which is
/// where the client draws a unit's name, scaled as the body is. One drawn as a
/// ring is marked over the top of its marker line.
///
/// Nothing is tested against the world: a mark is drawn for a creature behind
/// a wall, as its ring is while it is selected.
fn mark_services(
    mut marks: ResMut<ServiceMarks>,
    creatures: Res<Creatures>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    camera: Query<(&Camera, &Transform), With<WorldCamera>>,
    bodies: Query<(
        &Preview,
        &Transform,
        &vale_client::world::entities::EntityModel,
    )>,
) {
    marks.0.clear();
    if !state.editing() || *tool != Tool::Creatures || !creatures.show_services {
        return;
    }
    let Ok((camera, camera_at)) = camera.single() else {
        return;
    };
    let view = GlobalTransform::from(*camera_at);
    let eye = camera_at.translation;
    let forward = *camera_at.forward();
    // The name point of each body that has its model, by the spawn's guid.
    let heads: HashMap<u64, Vec3> = bodies
        .iter()
        .map(|(preview, at, model)| {
            let height = model.name_anchor * at.scale.y + SERVICE_MARK_LIFT;
            (preview.guid, at.translation + Vec3::Y * height)
        })
        .collect();
    let edits = session.as_ref().map(|session| &session.server_edits);
    for index in &creatures.near {
        let Some(base) = creatures.base_spawn(*index) else {
            continue;
        };
        // The near list is sorted by distance from the focus, not from the
        // camera, so the whole list is walked; the cheap tests come first.
        let ground = vale_client::render::axes::to_bevy(base.at.to_array());
        if ground.distance(eye) > SERVICE_MARK_RANGE {
            continue;
        }
        let with_edits = edits.and_then(|edits| base.with_edits(edits));
        let spawn = with_edits.as_ref().unwrap_or(base);
        if spawn.npc_flags & SERVICE_BITS == 0 || spawn.is_removed() {
            continue;
        }
        let over = heads.get(&spawn.guid).copied().unwrap_or_else(|| {
            let top = spawn.at + Vec3::Z * (BODY + SERVICE_MARK_LIFT);
            vale_client::render::axes::to_bevy(top.to_array())
        });
        // In front of the camera only: a point behind it projects to a
        // mirrored place on the screen.
        if (over - eye).dot(forward) <= 0.0 {
            continue;
        }
        let Ok(at) = camera.world_to_viewport(&view, over) else {
            continue;
        };
        marks.0.push(ServiceMark {
            at,
            npc_flags: spawn.npc_flags,
            away: over.distance(eye),
        });
    }
    marks
        .0
        .sort_unstable_by(|a, b| b.away.total_cmp(&a.away));
}

/// Spawn a model at each of the nearest spawns, and despawn the models of
/// spawns that have left the list.
///
/// The model is the client's own: a [`WorldEntity`] with a display id goes
/// through `spawn_models`, `dress` and `animate` as an entity the server sent
/// does. The module comment states why this is used rather than a second
/// reading of the same tables.
fn stand_them_up(
    mut commands: Commands,
    creatures: Res<Creatures>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    mut standing: Query<(Entity, &Preview, &mut Transform)>,
) {
    // The project's edits over the database's reading, so a display id typed
    // into the form changes the model on the next frame rather than after an
    // apply and a re-read. See [`Spawn::with_edits`].
    let edits = session.as_ref().map(|session| &session.server_edits);
    let wanted: std::collections::HashMap<u64, usize> = match (
        state.editing(),
        *tool == Tool::Creatures,
        creatures.show_models,
    ) {
        (true, true, true) => creatures
            .near
            .iter()
            .take(creatures.model_budget)
            .filter_map(|index| Some((creatures.base_spawn(*index)?.guid, *index)))
            .collect(),
        // Every other state wants none of them: another tool, a playtest, or
        // the switch off. The models are despawned rather than hidden, so no
        // dressing is held for a tool that is not in use.
        _ => std::collections::HashMap::new(),
    };

    let mut present = std::collections::HashSet::new();
    for (entity, preview, mut transform) in &mut standing {
        match wanted.get(&preview.guid) {
            Some(index) => {
                // A changed display id is a different model, and the body was
                // built from the old one, so it is despawned and spawned again
                // rather than moved.
                match creatures.spawn_at(*index, edits) {
                    Some(spawn) if spawn.display_id == preview.display_id => {
                        present.insert(preview.guid);
                        *transform = transform_for(&spawn);
                    }
                    _ => commands.entity(entity).despawn(),
                }
            }
            None => commands.entity(entity).despawn(),
        }
    }

    for (guid, index) in &wanted {
        if present.contains(guid) {
            continue;
        }
        let Some(spawn) = creatures.spawn_at(*index, edits) else {
            continue;
        };
        if spawn.display_id == 0 {
            continue;
        }
        commands.spawn((
            WorldEntity {
                guid: PREVIEW_GUID_BASE | guid,
                kind: ObjectType::Unit,
                name: spawn.name.clone().unwrap_or_default(),
                display_id: Some(spawn.display_id),
                // The scale is in the transform and not here.
                // `place_entities` reads this field, and it skips an entity
                // with no entry in `Motion`, which none of these has. See
                // `transform_for`.
                scale: Some(1.0),
                ..WorldEntity::default()
            },
            transform_for(&spawn),
            Visibility::default(),
            // Without a `Sheath` the unit is never drawn and no warning is
            // logged: `spawn_models` requires it in its query rather than as
            // an option, so an entity without one does not match, and it gets
            // no model, no fallback box and no warning line.
            vale_client::world::entities::Sheath::seeded(0),
            Preview {
                guid: *guid,
                display_id: spawn.display_id,
            },
        ));
    }
}

/// Marks the creature on the cursor before it is placed. Nothing else in this
/// crate uses this component.
///
/// Not [`Preview`]: `stand_them_up` despawns every `Preview` that is not a
/// spawn on the map, and a ghost is not a spawn.
#[derive(Component)]
pub struct Ghost {
    /// The display id this body was built from. A `WorldEntity`'s model is
    /// resolved once, so a ghost whose creature has changed is despawned and
    /// spawned again rather than corrected in place. This is [`Preview`]'s
    /// rule, for the same reason.
    display_id: u32,
}

/// The guid the ghost is given, clear of the previews' range and of anything a
/// session puts in the world.
const GHOST_GUID: u64 = PREVIEW_GUID_BASE | 0xFFFF_FFFF;

/// Keep the creature to be placed standing translucent under the pointer.
///
/// ## The ghost is a client entity faded by the client's own pass
///
/// The placement tool's ghost is the model's batches spawned by hand with
/// `Materials::with_opacity` over each one, because a doodad is only a static
/// mesh. A creature has a skeleton, a dressing composed from three tables, an
/// idle animation and, for a character-model NPC, a hairstyle and a set of
/// geosets. Spawning its batches by hand would be a second dressing pass
/// beside the client's, which this crate may not have.
///
/// So the ghost is a synthetic [`WorldEntity`], as a preview body is, and the
/// client's own `spawn_models`, `dress`, `pose` and `animate` build it. The
/// client's own `world::entities::tint::fade_models` makes it translucent: it
/// fades any unit whose `UNIT_FIELD_BYTES_1` vis flags mark it as creeping or
/// a ghost, so setting that flag is all that is needed. No seam is added, the
/// creature's appearance is decided only by the client, and the ghost is posed
/// and animated, where the doodad ghost is a still mesh.
///
/// ## The ghost follows the ground and keeps its facing
///
/// The height is the ground's under the pointer, because the server stands a
/// creature at its `position_z` and a guessed one drops it through the floor.
///
/// The facing is the doodad placer's turn. It starts toward the camera, on
/// the frame the ghost first has a surface under it, and from then on it is
/// held: moving the pointer or the camera does not turn the ghost. `,` and
/// `.` and `Alt` with the mouse turn it ([`Creatures::turn_ghost`]), and it is
/// kept from one placement to the next. Leaving Place or pressing `Escape`
/// forgets it, so the next creature armed starts toward the camera again.
#[allow(clippy::too_many_arguments)]
fn ghost(
    mut commands: Commands,
    mut creatures: ResMut<Creatures>,
    cursor: Res<crate::pick::Cursor>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
    mut standing: Query<(Entity, &Ghost, &mut Transform)>,
) {
    // Escape clears the chosen creature, with `place::aim`'s key and rule:
    // the mode stays, the creature on the cursor is removed. Not while a text
    // box has the keyboard, or the `Escape` used to leave the search box
    // would also disarm the placer.
    if creatures.placing() && keys.just_pressed(KeyCode::Escape) && !wants.wants_keyboard_input() {
        creatures.new_spawn.chosen = None;
    }
    // What the ghost should be, or nothing when another tool is active, the
    // mode is Select, no creature is chosen, the pointer is over a panel, or
    // there is no surface under it. The last is the usual case at the edge of
    // the streamed block, and the ghost is despawned rather than left where
    // the pointer last met the world.
    let wanted = match state.editing()
        && *tool == Tool::Creatures
        && creatures.placing()
        && crate::ui::over_the_world(&viewport, &wants, &windows)
    {
        true => creatures
            .new_spawn
            .chosen
            .clone()
            .zip(cursor.surface)
            .filter(|(known, _)| known.display_id != 0),
        false => None,
    };

    let Some((known, at)) = wanted else {
        for (entity, _, _) in &standing {
            commands.entity(entity).despawn();
        }
        // The facing is kept while the placer is armed and the pointer is
        // only off the world, and forgotten once nothing is on the cursor.
        if !creatures.placing() && creatures.ghost_facing.is_some() {
            creatures.ghost_facing = None;
        }
        return;
    };

    // Toward the camera the first time, and what it was left at after that.
    // The click reads the same value, so the model on screen and the row
    // written face the same way.
    let facing = match creatures.ghost_facing {
        Some(facing) => facing,
        None => {
            let toward = match camera.single() {
                Ok(camera) => {
                    let eye = Vec3::from(vale_client::render::axes::to_wow(camera.translation()));
                    (eye.y - at.y).atan2(eye.x - at.x)
                }
                Err(_) => 0.0,
            };
            creatures.ghost_facing = Some(toward);
            toward
        }
    };

    let scale = match known.display_scale > 0.01 {
        true => known.display_scale,
        false => 1.0,
    };
    let stand = Transform {
        translation: vale_client::render::axes::to_bevy(at.to_array()),
        rotation: vale_client::render::axes::body(facing, 0.0),
        scale: Vec3::splat(scale),
    };

    let mut found = false;
    for (entity, ghost, mut transform) in &mut standing {
        match ghost.display_id == known.display_id && !found {
            true => {
                found = true;
                *transform = stand;
            }
            // A second one, or one built from a display id that has changed.
            false => commands.entity(entity).despawn(),
        }
    }
    if found {
        return;
    }
    commands.spawn((
        WorldEntity {
            guid: GHOST_GUID,
            kind: ObjectType::Unit,
            name: known.name.clone(),
            display_id: Some(known.display_id),
            scale: Some(1.0),
            // This flag makes it translucent, through the client's own pass:
            // `tint::fade_models` fades a unit the server marks as hidden, and
            // the ghost is a body that is not in the world.
            vis_flags: vale_protocol::state::objects::UNIT_VIS_FLAGS_GHOST,
            ..WorldEntity::default()
        },
        stand,
        Visibility::default(),
        // Without a `Sheath` the unit is never drawn and no warning is logged.
        // See [`stand_them_up`], which needs it for the same reason.
        vale_client::world::entities::Sheath::seeded(0),
        Ghost {
            display_id: known.display_id,
        },
    ));
}

/// Where a preview body stands, in Bevy's axes.
///
/// The scale is written here rather than onto the `WorldEntity`, because that
/// field is read by `world::session::place_entities`, which skips an entity
/// with no interpolated position. No preview has one, since none came from the
/// server.
///
/// A `display_scale1` of 0 means "the model's own", which
/// `CreatureDisplayInfo` already applied through `DisplayModel::scale`, so a
/// zero becomes 1.0 here rather than drawing the creature at zero size.
fn transform_for(spawn: &Spawn) -> Transform {
    let scale = match spawn.display_scale > 0.01 {
        true => spawn.display_scale,
        false => 1.0,
    };
    Transform {
        translation: vale_client::render::axes::to_bevy(spawn.at.to_array()),
        rotation: vale_client::render::axes::body(spawn.orientation, 0.0),
        scale: Vec3::splat(scale),
    }
}

/// `--spawn <guid>`: select one spawn and fly to it, once the map has been
/// read.
///
/// Selecting a spawn is the one gesture in this tool a scripted run cannot
/// otherwise make. Everything else here is a checkbox or a number; selecting
/// a spawn is a click on a creature in the viewport, and `--hover` belongs to
/// the client, not to this tool. See [`crate::Args::spawn`].
///
/// Runs once, on the first frame the map's spawns are available. Running
/// repeatedly would move the selection back from whatever the user had
/// clicked.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    mut creatures: ResMut<Creatures>,
    mut waypoints: ResMut<super::waypoints::Waypoints>,
    mut quests: ResMut<super::quests::Quests>,
    mut session: Option<ResMut<EditSession>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(guid) = args.spawn else {
        *done = true;
        return;
    };
    // Nothing to choose from yet: the read is still in flight, or there is no
    // database and `trouble` already reports it.
    if creatures.spawns.is_empty() {
        if creatures.trouble.is_some() {
            *done = true;
        }
        return;
    }
    *done = true;
    let Some(spawn) = creatures
        .spawns
        .iter()
        .find(|spawn| spawn.guid == guid)
        .cloned()
    else {
        warn!("--spawn: no creature with guid {guid} on this map");
        if let Some(session) = session.as_mut() {
            session.status = format!("no creature spawn {guid} on this map");
        }
        return;
    };
    info!(
        "--spawn: {} at {:.1}, {:.1}, {:.1}",
        spawn.label(),
        spawn.at.x,
        spawn.at.y,
        spawn.at.z
    );
    creatures.selected = Some(guid);
    // `--waypoints` presses the Waypoints button on the form just opened.
    if args.waypoints {
        waypoints.showing = true;
        waypoints.open_for(guid, spawn.entry, spawn.movement_type);
    }
    creatures.template = None;
    creatures.spawn_row = None;
    // The template window, which is otherwise opened by a button. See
    // [`crate::Args::template`].
    creatures.template_window = args.template;
    // The quest window beside it. See [`crate::Args::creature_quests`].
    if args.creature_quests {
        quests.window_for =
            Some((super::quests::Holder::Creature, spawn.entry, spawn.label()));
    }
    creatures.fly_to(spawn.at);
    if let Some(session) = session.as_mut() {
        session.status = format!("{} — guid {guid}", spawn.label());
    }
}

/// `--spawn-add <entry>` and `--spawn-delete <guid>`: add a creature spawn to
/// the map, and remove one, from a scripted run.
///
/// A scripted run cannot otherwise make either gesture. Placing is a click on
/// the ground with the placer armed and removing is a press on a button under
/// a selection, so without these flags nothing downstream of either (the
/// store, the undo, the SQL, the apply, the revert) could be checked. They
/// exist for the same reason as `--waypoint-add`, for the `creature` table.
///
/// `--spawn-add` goes through the picker rather than around it, so the same
/// run checks the search: the entry is typed into the box, the match is taken
/// from the results, and the placement is the ordinary one.
///
/// It waits for three things; without any one of them the placement fails or
/// lands in the wrong place:
///
/// * the map read, because the guid comes from the highest the table holds;
/// * the search, because the creature has to be known before it can be drawn
///   or placed;
/// * the tile under the camera, because a placement takes the ground's height
///   and a tile that is not open returns none, which leaves the spawn buried
///   in the slope it was meant to stand on.
pub fn placed_on_the_command_line(
    args: Res<crate::Args>,
    mut creatures: ResMut<Creatures>,
    mut session: Option<ResMut<EditSession>>,
    focus: Res<WorldFocus>,
    time: Res<Time>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    if args.spawn_add.is_none() && args.spawn_delete.is_none() && args.pick.is_none() {
        *done = true;
        creatures.scripted_done = true;
        return;
    }
    // The map has to have been read: `--spawn-delete` needs the spawn and
    // `--spawn-add` needs the highest guid the table holds.
    if creatures.reading() || creatures.loaded.is_none() {
        return;
    }
    if creatures.trouble.is_some() {
        *done = true;
        creatures.scripted_done = true;
        warn!("--spawn-add/--spawn-delete: there is no database to work against");
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    let now = time.elapsed_secs_f64();

    if let Some(guid) = args.spawn_delete {
        match creatures.spawns.iter().any(|spawn| spawn.guid == guid) {
            true => {
                Creatures::remove(session, guid, now);
                creatures.selected = Some(guid);
                info!("--spawn-delete: spawn {guid} is marked for removal");
            }
            false => warn!("--spawn-delete: no creature with guid {guid} on this map"),
        }
    }

    // `--pick` is `--spawn-add` without the placement; see
    // [`crate::Args::pick`]. It takes the same steps up to choosing the
    // creature and then stops, which is the state a screenshot of the picker
    // needs.
    let Some(entry) = args.spawn_add.or(args.pick) else {
        *done = true;
        creatures.scripted_done = true;
        return;
    };
    // Through the picker, so the search is checked by the same run.
    if creatures.new_spawn.search.is_empty() {
        creatures.mode = super::place::Mode::Place;
        creatures.new_spawn.search = entry.to_string();
        return;
    }
    if creatures.new_spawn.searching() {
        return;
    }
    let Some(known) = creatures
        .new_spawn
        .matches
        .iter()
        .find(|known| known.entry == entry)
        .cloned()
    else {
        *done = true;
        creatures.scripted_done = true;
        warn!("--spawn-add: no creature template with entry {entry}");
        return;
    };
    creatures.new_spawn.chosen = Some(known.clone());
    if args.spawn_add.is_none() {
        // `--pick`: chosen, armed, and nothing placed.
        *done = true;
        creatures.scripted_done = true;
        info!("--pick: {} is on the cursor", known.label());
        return;
    }
    // The ground at the camera's focus. A scripted run has no pointer, so
    // there is no surface under one to use.
    if !focus.present {
        return;
    }
    let tile = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    let Some(ground) = session.tiles.get(&tile).and_then(|open| {
        vale_edit::adt::heights::height_at(open, focus.position.x, focus.position.y)
    }) else {
        return;
    };
    *done = true;
    let at = Vec3::new(focus.position.x, focus.position.y, ground);
    let guid = creatures.create(session, &known, at, 0.0, now);
    // Back to Select. A click does not do this, but a script must: nothing
    // presses `Escape` afterwards, and a run that ended armed would show a
    // ghost in its screenshot standing over the spawn it had just placed.
    creatures.mode = super::place::Mode::Select;
    creatures.new_spawn.chosen = None;
    creatures.scripted_done = true;
    creatures.fly_to(at);
    info!(
        "--spawn-add: {} placed as guid {guid} at {:.2}, {:.2}, {:.2}",
        known.label(),
        at.x,
        at.y,
        at.z
    );
    session.status = format!("{} placed as guid {guid}", known.label());
}

/// `--template-new`: make a new creature template, choose it in the picker and
/// open the template window on it. `--template-entry <n>`: renumber the
/// template the window is about, which is `--template-new`'s or `--spawn`'s.
///
/// A scripted run cannot otherwise make either gesture: creating is a press on
/// the picker and renumbering is typing into the window's entry box. Without
/// these flags nothing downstream of either could be checked: the store, the
/// undo, the SQL, the apply and the revert.
///
/// It waits for the map read, because a new entry is numbered from the highest
/// the table holds. The renumber waits for a subject: `--spawn` opens its
/// creature on the frame the map lands, and `--template-new` chooses its own,
/// so the retry is at most a frame.
fn template_on_the_command_line(
    args: Res<crate::Args>,
    mut creatures: ResMut<Creatures>,
    mut session: Option<ResMut<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    time: Res<Time>,
    mut done: Local<bool>,
    mut made: Local<bool>,
) {
    if *done {
        return;
    }
    if !args.template_new && args.template_entry.is_none() {
        *done = true;
        creatures.scripted_template_done = true;
        return;
    }
    if creatures.reading() || creatures.loaded.is_none() {
        return;
    }
    if creatures.trouble.is_some() {
        *done = true;
        creatures.scripted_template_done = true;
        warn!("--template-new/--template-entry: there is no database to work against");
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    if args.template_new && !*made {
        *made = true;
        let patch = server_patch(&settings);
        let entry = creatures.create_template(session, "New Creature", patch, now);
        info!("--template-new: creature_template {entry} created at patch {patch}");
        session.status = format!("creature_template {entry} created");
    }
    if let Some(to) = args.template_entry {
        if !args.template_new && args.spawn.is_none() {
            *done = true;
            creatures.scripted_template_done = true;
            warn!("--template-entry: nothing names a template; give --template-new or --spawn");
            return;
        }
        let Some(subject) = creatures.template_subject(&session.server_edits) else {
            return;
        };
        match creatures.rekey_template(session, &subject, to, now) {
            Ok(()) => info!(
                "--template-entry {to}: creature_template {} moved",
                subject.entry
            ),
            Err(why) => warn!("--template-entry {to}: {why}"),
        }
    }
    *done = true;
    creatures.scripted_template_done = true;
}

/// `--pick-display [id]`: open the display picker on the template window's
/// `display_id1`, and with an id choose it. The picker is a grid of pictures
/// two clicks in; the bare flag opens it for a screenshot, and an id also
/// writes the column, which checks the whole path with nobody at the keyboard.
///
/// It runs after the template flags and waits for the template's row, because
/// the column's target carries the database's value: a chosen value equal to
/// it takes the edit off rather than writing it. The window is opened so that
/// the row is fetched (`fetch_rows` follows the window's subject) and so that
/// a shot shows the column the choice lands in.
fn display_on_the_command_line(
    args: Res<crate::Args>,
    mut creatures: ResMut<Creatures>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    time: Res<Time>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    if !args.pick_display || *tool != Tool::Creatures {
        *done = true;
        return;
    }
    if !creatures.scripted_template_done {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    let Some(subject) = creatures.template_subject(&session.server_edits) else {
        if creatures.scripted_done || creatures.trouble.is_some() {
            *done = true;
            warn!("--pick-display: nothing names a template; give --spawn or --template-new");
        }
        return;
    };
    creatures.template_window = true;
    let Some(row) = creatures
        .template
        .as_ref()
        .filter(|held| held.entry == subject.entry)
    else {
        return;
    };
    const COLUMN: &str = "display_id1";
    let column = creature::column(creature::TEMPLATE, COLUMN).expect("a template column");
    let key = subject.key();
    let in_database = match subject.is_new() {
        true => None,
        false => column.literal(&row.row),
    };
    let showing: u32 = session
        .server_edits
        .get(creature::TEMPLATE, &key, COLUMN)
        .map(str::to_string)
        .or_else(|| in_database.clone())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let target = super::quests::ColumnTarget {
        table: creature::TEMPLATE,
        subject: format!("{} {} {COLUMN}", creature::TEMPLATE, key.text()),
        key,
        column: COLUMN,
        in_database,
        label: "Edit creature",
    };
    match args.display_id {
        Some(id) => {
            target.write(session, id.to_string(), time.elapsed_secs_f64());
            info!("--pick-display {id}: written to creature_template {} {COLUMN}", subject.entry);
        }
        None => {
            creatures.display_pick = Some(super::displays::DisplayPick::open(
                super::displays::Table::Creature,
                target,
                showing,
            ));
            info!("--pick-display: the picker is open on creature_template {} {COLUMN}", subject.entry);
        }
    }
    *done = true;
}

/// Take the camera to wherever the panel asked.
fn fly(mut creatures: ResMut<Creatures>, mut camera: ResMut<crate::camera::EditorCamera>) {
    let Some(at) = creatures.fly_to.take() else {
        return;
    };
    camera.target = at;
    camera.wants_the_ground = false;
}

/// Read the selected spawn's two whole rows, on a task.
///
/// The map query reads ten columns, which is what the drawing needs. A form
/// needs all 78 columns of the template and all 21 of the spawn, for one row
/// rather than 24,610, so they are read when a selection is made rather than
/// carried for every spawn on the map.
pub fn fetch_rows(
    mut creatures: ResMut<Creatures>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if let Some(task) = creatures.template_task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            creatures.template_task = None;
            match done {
                Ok(row) => creatures.template = row,
                Err(e) => warn!("creature template: {e}"),
            }
        }
    }
    if let Some(task) = creatures.spawn_task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            creatures.spawn_task = None;
            match done {
                Ok(row) => creatures.spawn_row = row,
                Err(e) => warn!("creature spawn: {e}"),
            }
        }
    }
    if !state.editing() || *tool != Tool::Creatures {
        return;
    }
    // A row is made again when the database has been written since it was
    // made, or when the project has started or stopped creating it; see
    // `crate::server::fresh`.
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    let Some((at, _)) = settings.resolve() else {
        return;
    };
    // The template half follows the window's subject, which in Place is the
    // picker's creature and in Select the selected spawn's template.
    let subject = session
        .as_ref()
        .and_then(|session| creatures.template_subject(&session.server_edits));
    if let Some(subject) = subject {
        let wants_template = creatures.template_task.is_none()
            && (creatures.template.as_ref().is_none_or(|had| had.entry != subject.entry)
                || creatures.template_at != Some((writes, subject.is_new())));
        if wants_template {
            creatures.template_at = Some((writes, subject.is_new()));
            if subject.is_new() {
                // A template this project creates has no row to read. Its
                // whole row is in the store, so the form is given an empty
                // row holding only the key, and every column shows the
                // project's own value.
                let mut row = creature::Row::new();
                row.insert("entry".to_string(), Some(subject.entry.to_string()));
                creatures.template = Some(TemplateRow {
                    entry: subject.entry,
                    patch: subject.patch,
                    row,
                });
            } else {
                // Read from where the database has it, which is not the
                // entry the window shows while the project moves the
                // template and has not applied it.
                let (entry, read_entry, patch) = (subject.entry, subject.read_entry, subject.patch);
                let at = at.clone();
                creatures.template_task = Some(crate::server::queue::read(async move {
                    let mut db = vale_mangos::conn::Db::open(&at)?;
                    // The patch the map query already resolved, so the form
                    // and the drawing show the same row. A row the database
                    // does not hold is an empty row, which the form shows as
                    // NULL in every column; answering nothing instead left
                    // the row unread and asked for again on every frame.
                    let sql = format!(
                        "SELECT * FROM `creature_template` WHERE `entry` = {read_entry} AND `patch` = {patch}"
                    );
                    let row = db.row(&sql)?.unwrap_or_default();
                    Ok(Some(TemplateRow { entry, patch, row }))
                }));
            }
        }
    }
    let Some(spawn) = creatures.chosen().cloned() else {
        return;
    };
    // A spawn this project creates has no row to read. Its whole row is in
    // the store, so the form is given an empty row holding only the key:
    // every column then shows the project's own value, and the `guid` field,
    // which is never editable and so never in the store, shows the guid
    // rather than `NULL`.
    if spawn.is_new() {
        if creatures.spawn_row.as_ref().is_none_or(|had| had.guid != spawn.guid)
            || creatures.spawn_at != Some((writes, true))
        {
            let mut row = creature::Row::new();
            row.insert("guid".to_string(), Some(spawn.guid.to_string()));
            creatures.spawn_at = Some((writes, true));
            creatures.spawn_row = Some(SpawnRow {
                guid: spawn.guid,
                row,
            });
        }
    }
    let wants_spawn = !spawn.is_new()
        && creatures.spawn_task.is_none()
        && (creatures.spawn_row.as_ref().is_none_or(|had| had.guid != spawn.guid)
            || creatures.spawn_at != Some((writes, false)));

    if wants_spawn {
        creatures.spawn_at = Some((writes, false));
        let guid = spawn.guid;
        creatures.spawn_task = Some(crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            Ok(db
                .row(&creature::spawn_query(guid))?
                .map(|row| SpawnRow { guid, row }))
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows the form draws belong to the primary, so changing the
    /// primary drops them and changing only the group does not. A shift+click
    /// makes the clicked spawn the primary, and taking the primary out
    /// promotes the member added last — the doodad tool's rules, by guid.
    #[test]
    fn a_group_of_spawns_follows_the_doodad_rules() {
        let mut creatures = Creatures::default();
        let row = |guid| SpawnRow {
            guid,
            row: creature::Row::default(),
        };
        creatures.select_only(Some(1));
        creatures.spawn_row = Some(row(1));
        // Adding one makes it the primary, so the held row is dropped.
        creatures.toggle(2);
        assert_eq!(creatures.selected, Some(2));
        assert_eq!(creatures.also, vec![1]);
        assert!(creatures.spawn_row.is_none());

        // Taking a member out keeps the primary and its row.
        creatures.spawn_row = Some(row(2));
        creatures.toggle(3);
        creatures.spawn_row = Some(row(3));
        creatures.toggle(1);
        assert_eq!(creatures.selected, Some(3));
        assert_eq!(creatures.also, vec![2]);
        assert!(creatures.spawn_row.is_some());

        // A press on a member makes it the primary and keeps the group.
        creatures.promote(2);
        assert_eq!(creatures.selected, Some(2));
        assert_eq!(creatures.also, vec![3]);
        assert!(creatures.holds(3) && !creatures.holds(1));

        // Taking the primary out promotes the member added last.
        creatures.toggle(2);
        assert_eq!(creatures.selected, Some(3));
        assert!(creatures.also.is_empty());

        creatures.toggle(4);
        creatures.select_only(Some(5));
        assert_eq!((creatures.selected, creatures.also.len()), (Some(5), 0));
    }

    fn a_spawn() -> Spawn {
        Spawn {
            guid: 19272,
            entry: 68,
            read_entry: 68,
            at: Vec3::new(-8817.3, 809.0, 98.7),
            orientation: 1.5,
            wander: 5.0,
            movement_type: 2,
            name: Some("Stormwind City Guard".into()),
            subname: None,
            display_id: 3167,
            display_scale: 0.0,
            level: (55, 55),
            faction: 11,
            npc_flags: 1,
            rank: 0,
            patch: 0,
            claim: Life::Update,
        }
    }

    /// A new spawn's guid is above the reserved base, above what the table
    /// holds, and above what this project has already claimed.
    ///
    /// The second condition matters because once a project has placed a spawn
    /// at ten million the table's own maximum is ten million, and a guid taken
    /// from the base alone would collide with it on the next apply. The third
    /// stops two spawns placed between two reads of the map from getting the
    /// same row.
    #[test]
    fn a_new_guid_clears_the_base_the_table_and_the_project() {
        let mut creatures = Creatures::default();
        let empty = Edits::default();
        // Nothing read and nothing claimed: the reserved base.
        assert_eq!(creatures.next_guid(&empty), creature::RESERVED_GUID_BASE);
        // A database whose highest is below the base does not move it.
        creatures.max_guid = Some(303_733);
        assert_eq!(creatures.next_guid(&empty), creature::RESERVED_GUID_BASE);
        // A database whose highest is above the base raises it.
        creatures.max_guid = Some(creature::RESERVED_GUID_BASE + 4);
        assert_eq!(
            creatures.next_guid(&empty),
            creature::RESERVED_GUID_BASE + 5
        );

        // A spawn this project has claimed and not yet applied counts too.
        let mut edits = Edits::default();
        edits.set_life(
            creature::SPAWN,
            &creature::spawn_key(creature::RESERVED_GUID_BASE + 9),
            Life::Insert,
        );
        assert_eq!(
            creatures.next_guid(&edits),
            creature::RESERVED_GUID_BASE + 10
        );
        // A row the project only edits does not count: that guid already
        // exists in the database.
        let mut edits = Edits::default();
        edits.set(
            creature::SPAWN,
            &creature::spawn_key(creature::RESERVED_GUID_BASE + 900),
            "position_z",
            Some("1".into()),
        );
        assert_eq!(
            creatures.next_guid(&edits),
            creature::RESERVED_GUID_BASE + 5
        );
    }

    /// One index space covers both lists, the database's first.
    ///
    /// Everything that draws, picks or counts walks it, so a created spawn is
    /// drawn, picked and counted on the same terms as a read one.
    #[test]
    fn the_index_space_covers_the_read_and_the_projects_own() {
        let mut creatures = Creatures::default();
        creatures.spawns = vec![a_spawn()];
        let mut mine = a_spawn();
        mine.guid = 10_000_001;
        mine.claim = Life::Insert;
        creatures.created = vec![mine];

        assert_eq!(creatures.spawn_count(), 2);
        assert_eq!(creatures.base_spawn(0).map(|spawn| spawn.guid), Some(19272));
        assert_eq!(
            creatures.base_spawn(1).map(|spawn| spawn.guid),
            Some(10_000_001)
        );
        assert!(creatures.base_spawn(2).is_none());

        // The selection reaches both lists, so the form can open on a spawn
        // just placed.
        creatures.selected = Some(10_000_001);
        assert!(creatures.chosen().is_some_and(Spawn::is_new));
    }

    /// The project's claim on a row reaches the drawn spawn. Otherwise a
    /// removed creature would be drawn as unchanged until the removal was
    /// applied.
    #[test]
    fn a_removed_spawn_is_drawn_as_one() {
        let spawn = a_spawn();
        let mut edits = Edits::default();
        edits.set_life(creature::SPAWN, &spawn.key(), Life::Delete);
        let drawn = spawn.with_edits(&edits).expect("the store claims it");
        assert!(drawn.is_removed());
        assert!(!drawn.is_new());
        // A spawn with no claim is neither.
        assert!(spawn.with_edits(&Edits::default()).is_none());
        assert_eq!(spawn.claim, Life::Update);
    }

    /// A new template's entry is above the reserved base, above what the
    /// table holds at any patch, and above what this project has already
    /// claimed. The third is what stops two templates created between two
    /// reads of the map from sharing an entry.
    #[test]
    fn a_new_templates_entry_is_above_everything_it_can_see() {
        let mut creatures = Creatures::default();
        let mut edits = Edits::default();
        assert_eq!(creatures.next_entry(&edits), creature::RESERVED_ENTRY_BASE);
        creatures.max_entry = Some(creature::RESERVED_ENTRY_BASE + 40);
        assert_eq!(creatures.next_entry(&edits), creature::RESERVED_ENTRY_BASE + 41);
        let key = creature::template_key(creature::RESERVED_ENTRY_BASE + 100, 10);
        edits.set_life(creature::TEMPLATE, &key, Life::Insert);
        assert_eq!(creatures.next_entry(&edits), creature::RESERVED_ENTRY_BASE + 101);
        // What the table holds at another patch still counts, because the
        // key is `(entry, patch)` and an `INSERT` at a taken entry would
        // collide at the apply.
        creatures.max_entry = Some(creature::RESERVED_ENTRY_BASE + 500);
        assert_eq!(creatures.next_entry(&edits), creature::RESERVED_ENTRY_BASE + 501);
    }

    /// A created template is one row of the store, and renumbering it re-keys
    /// that row: after the move there is one `creature_template` claim, at the
    /// new entry, still a creation, with no origin recorded. A spawn this
    /// project created of it follows.
    #[test]
    fn renumbering_a_created_template_moves_its_one_row() {
        let mut edits = Edits::default();
        let from = creature::template_key(2_000_000, 10);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in creature::new_template("Test Subject") {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(creature::TEMPLATE, &from, Some(&row.to_line()));
        let known = Known::from_store(&from, &row).expect("a template row");
        assert_eq!(known.entry, 2_000_000);
        assert_eq!(known.patch, 10);
        assert_eq!(known.name, "Test Subject");
        assert!(known.is_new());
        let shown = TemplateSubject {
            entry: 2_000_000,
            read_entry: 2_000_000,
            patch: 10,
            claim: Life::Insert,
            label: known.label(),
        };
        let (was, to) = plan_template_move(&shown, 2_000_005, false)
            .expect("allowed")
            .expect("a move");
        assert_eq!(was, from);
        assert_eq!(to, creature::template_key(2_000_005, 10));
        assert!(edits.rekey(creature::TEMPLATE, &was, &to, Some(&from)));
        let rows: Vec<(&str, &Key, &RowEdit)> = edits
            .rows()
            .filter(|(table, _, _)| *table == creature::TEMPLATE)
            .collect();
        assert_eq!(rows.len(), 1, "one claim, not two: {rows:?}");
        let (_, key, moved) = rows[0];
        assert_eq!(*key, to);
        assert_eq!(moved.life, Life::Insert);
        assert_eq!(moved.from, None, "a created row has no origin to record");
        assert_eq!(moved.columns.len(), row.columns.len());
        // The project's own spawn of it names the new entry after the follow.
        let spawn = creature::spawn_key(10_000_000);
        edits.set(creature::SPAWN, &spawn, "id", Some("2000005".into()));
        let hits = crate::server::follow::hits(
            &edits,
            &creature::TEMPLATE_REFERENCES,
            2_000_005,
            2_000_009,
        );
        assert_eq!(hits.len(), 1, "{hits:?}");
    }

    /// The moves a template may not make: to 0, past the column, onto an
    /// entry that is already a creature, and to where it already is, which
    /// is not a move.
    #[test]
    fn a_template_move_is_refused_where_the_item_tool_refuses_one() {
        let shown = TemplateSubject {
            entry: 68,
            read_entry: 68,
            patch: 0,
            claim: Life::Update,
            label: "Stormwind City Guard (68)".into(),
        };
        assert_eq!(plan_template_move(&shown, 68, true).unwrap(), None);
        assert!(plan_template_move(&shown, 0, false).is_err());
        assert!(plan_template_move(&shown, creature::MAX_ENTRY + 1, false).is_err());
        assert!(plan_template_move(&shown, 1, true).is_err());
        assert!(plan_template_move(&shown, 2_000_000, false).unwrap().is_some());
        // Back to where the database has it is allowed although the entry is
        // taken, because it is taken by this row.
        let moved = TemplateSubject {
            entry: 2_000_000,
            ..shown
        };
        assert!(plan_template_move(&moved, 68, true).unwrap().is_some());
    }

    /// A template whose entry the project changes is shown at the new entry
    /// on every spawn of it, on the picker's matches and in the known table,
    /// and reset when the move is taken back.
    #[test]
    fn a_moved_template_is_folded_into_what_is_drawn() {
        let mut creatures = Creatures::default();
        creatures.spawns = vec![a_spawn()];
        let known = known_from(&a_spawn()).expect("named");
        creatures.known.insert(68, known.clone());
        creatures.new_spawn.matches = vec![known.clone()];
        creatures.new_spawn.chosen = Some(known);
        let mut edits = Edits::default();
        edits.set(
            creature::TEMPLATE,
            &creature::template_key(68, 0),
            "level_min",
            Some("57".into()),
        );
        assert!(edits.rekey(
            creature::TEMPLATE,
            &creature::template_key(68, 0),
            &creature::template_key(2_000_068, 0),
            None
        ));
        creatures.fold_template_moves(&edits);
        assert_eq!(creatures.spawns[0].entry, 2_000_068);
        assert_eq!(creatures.spawns[0].read_entry, 68);
        assert_eq!(
            creatures.spawns[0].template_key(),
            creature::template_key(2_000_068, 0)
        );
        assert_eq!(creatures.new_spawn.matches[0].entry, 2_000_068);
        assert_eq!(
            creatures.new_spawn.chosen.as_ref().map(|known| known.entry),
            Some(2_000_068)
        );
        assert!(creatures.known.contains_key(&2_000_068));
        assert!(!creatures.known.contains_key(&68));
        // The edit made before the move is on the claim at the new key.
        assert_eq!(
            edits.get(creature::TEMPLATE, &creatures.spawns[0].template_key(), "level_min"),
            Some("57")
        );
        // Taken back, everything returns to the entry it was read at.
        assert!(edits.rekey(
            creature::TEMPLATE,
            &creature::template_key(2_000_068, 0),
            &creature::template_key(68, 0),
            None
        ));
        creatures.fold_template_moves(&edits);
        assert_eq!(creatures.spawns[0].entry, 68);
        assert!(creatures.known.contains_key(&68));
    }

    /// The window is about the picker's creature in Place and the selected
    /// spawn's template in Select, and a created template's claim reads as
    /// a creation.
    #[test]
    fn the_template_subject_follows_the_mode() {
        let mut creatures = Creatures::default();
        creatures.spawns = vec![a_spawn()];
        creatures.selected = Some(19272);
        let edits = Edits::default();
        let shown = creatures.template_subject(&edits).expect("the spawn's");
        assert_eq!((shown.entry, shown.patch, shown.claim), (68, 0, Life::Update));
        assert_eq!(shown.key(), a_spawn().template_key());

        creatures.mode = super::super::place::Mode::Place;
        assert!(creatures.template_subject(&edits).is_none());
        let mut edits = Edits::default();
        let key = creature::template_key(2_000_000, 10);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in creature::new_template("Test Subject") {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(creature::TEMPLATE, &key, Some(&row.to_line()));
        creatures.rebuild_templates(&edits);
        assert_eq!(creatures.templates.len(), 1);
        assert!(creatures.known.get(&2_000_000).is_some_and(Known::is_new));
        creatures.new_spawn.chosen = creatures.templates.first().cloned();
        let shown = creatures.template_subject(&edits).expect("the picker's");
        assert_eq!((shown.entry, shown.patch, shown.claim), (2_000_000, 10, Life::Insert));
        assert!(shown.is_new());
        assert_eq!(shown.label, "Test Subject (2000000)");
        // The chosen copy follows the store: a rename in the window shows in
        // the picker on the same frame.
        edits.set(creature::TEMPLATE, &key, "name", Some("'Renamed'".into()));
        creatures.rebuild_templates(&edits);
        assert_eq!(
            creatures.new_spawn.chosen.as_ref().map(|known| known.name.as_str()),
            Some("Renamed")
        );
        // …and a discarded one is dropped from it.
        edits.set_row_line(creature::TEMPLATE, &key, None);
        creatures.rebuild_templates(&edits);
        assert!(creatures.templates.is_empty());
        assert!(creatures.new_spawn.chosen.is_none());
        assert!(!creatures.known.contains_key(&2_000_000));
    }

    /// The two keys a click reaches name different rows, and the template's
    /// key includes the patch, so an edit does not change every content-patch
    /// version of a creature.
    #[test]
    fn a_click_reaches_a_spawn_key_and_a_template_key() {
        let spawn = a_spawn();
        assert_eq!(spawn.key().where_clause(), "`guid` = 19272");
        assert_eq!(
            spawn.template_key().where_clause(),
            "`entry` = 68 AND `patch` = 0"
        );
    }

    /// A `display_scale1` of 0 means "the model's own scale", which
    /// `CreatureDisplayInfo` has already applied. Passing the 0 through would
    /// draw the creature at zero size.
    #[test]
    fn a_zero_display_scale_is_one() {
        let mut spawn = a_spawn();
        assert_eq!(transform_for(&spawn).scale, Vec3::splat(1.0));
        spawn.display_scale = 2.5;
        assert_eq!(transform_for(&spawn).scale, Vec3::splat(2.5));
    }

    /// A preview's guid cannot collide with one the server sent: a real guid is
    /// a `u32` with a type mask above it and nothing reaches this range.
    #[test]
    fn a_preview_guid_is_out_of_the_servers_range() {
        assert!(PREVIEW_GUID_BASE > u64::from(u32::MAX));
        assert_eq!(PREVIEW_GUID_BASE | 19272, PREVIEW_GUID_BASE + 19272);
    }

    /// A spawn with no template row is drawn and labelled as such rather than
    /// left out. vmangos refuses to load it, and the editor is where it can be
    /// found.
    #[test]
    fn a_spawn_with_no_template_says_so() {
        let mut spawn = a_spawn();
        assert_eq!(spawn.label(), "Stormwind City Guard (68)");
        spawn.name = None;
        assert_eq!(spawn.label(), "entry 68 — no template row");
    }
}
