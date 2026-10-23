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
//! which row is which before anything else; see [`crate::ui::tools::spawns::creatures`].
//!
//! ## What is here
//!
//! The mechanism is the game object tool's too and is in
//! [`super::spawn`]: the map read, the selection, the drag, the markers, the
//! models, the picker and the template lifecycle. This file is the creature
//! subject's judgements ([`CreatureSubject`]): its tables, its words, a
//! facing that is one column, a drag that stands the creature on the ground,
//! a dome for a marker, a unit faded by the client's own pass for a ghost, the
//! waypoint mode that takes a press first, and the service icons over the
//! creatures that offer one ([`mark_services`]).
//!
//! Nothing is drawn during a playtest. The server spawns the real creatures
//! then, and a preview in the same place would be a second copy of each one.

use super::spawn::{Flags, Script, SpawnSubject, SpawnTool, SpawnToolPlugin, Spawned, Template, Words};
use crate::marks::{Look, Marks};
use crate::session::EditSession;
use crate::subjects::listing::Listed;
use crate::tools::Tool;
use bevy::prelude::*;
use std::collections::HashMap;
use vale_client::world::camera::WorldCamera;
use vale_client::world::entity::{ObjectType, WorldEntity};
use vale_mangos::creature::{self, RowValue};
use vale_mangos::row::{Edits, Key, Life, RowEdit};
use vale_mangos::sql::unquote;

/// What the pointer is doing with the creatures on the open map.
pub type Creatures = SpawnTool<CreatureSubject>;

/// The creature subject: `creature` and `creature_template`.
#[derive(Debug)]
pub struct CreatureSubject;

/// What only the creature tool keeps.
#[derive(Debug)]
pub struct CreatureOwn {
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
    pub worn: HashMap<u32, Option<crate::preview::portraits::Worn>>,
    /// The `GameAssets::tables_generation` [`Self::worn`] was resolved at. A
    /// published display table is a new generation, and an edited or new
    /// display is resolved again.
    pub worn_at: u64,
    /// Draw a row of service icons over each near creature that offers one.
    /// See [`mark_services`].
    pub show_services: bool,
}

impl Default for CreatureOwn {
    fn default() -> CreatureOwn {
        CreatureOwn { worn: HashMap::new(), worn_at: 0, show_services: true }
    }
}

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
    /// template's entry and has not applied it; see
    /// [`SpawnTool::fold_template_moves`].
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

    /// This row with the project's own edits laid over it.
    ///
    /// The list in [`SpawnTool::spawns`] is what the database holds: it is
    /// read once per map and again when an apply changes something. A form
    /// edit changes neither. Without this function, typing a new
    /// `display_id1` or a new `orientation` changed nothing on screen until
    /// the rows had been applied and read back, so the edit appeared to be
    /// ignored.
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
        if !edits.touches(creature::SPAWN, &spawn_key) && !edits.touches(creature::TEMPLATE, &template_key) {
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

impl Spawned for Spawn {
    fn guid(&self) -> u64 {
        self.guid
    }

    fn entry(&self) -> u32 {
        self.entry
    }

    fn read_entry(&self) -> u32 {
        self.read_entry
    }

    fn patch(&self) -> u32 {
        self.patch
    }

    fn at(&self) -> Vec3 {
        self.at
    }

    fn orientation(&self) -> f32 {
        self.orientation
    }

    fn claim(&self) -> Life {
        self.claim
    }

    fn display_id(&self) -> u32 {
        self.display_id
    }

    fn label(&self) -> String {
        Spawn::label(self)
    }

    fn with_edits(&self, edits: &Edits) -> Option<Spawn> {
        Spawn::with_edits(self, edits)
    }

    fn fold_entry(&mut self, moved: &dyn Fn(u32, u32) -> u32) {
        self.entry = moved(self.read_entry, self.patch);
    }
}

/// The columns of a creature entry that a marker, a model and a panel heading
/// need.
///
/// The map query already carries these columns for every spawn on the map, so
/// most entries need no extra read. Two sources do: the picker's matches,
/// which may be creatures with no spawn on this map, and the entries a project
/// creates a spawn of after the map was read. All three fill the same table;
/// see [`SpawnTool::known`].
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
    pub(crate) fn from_store(key: &Key, row: &RowEdit) -> Option<Known> {
        let entry = key.first()? as u32;
        let patch = key.patch()?;
        let number = |column: &str| -> u32 {
            row.columns.get(column).and_then(|value| value.trim().parse::<f64>().ok()).unwrap_or(0.0) as u32
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

impl Listed for Known {
    const TABLE: &'static str = creature::TEMPLATE;

    fn entry(&self) -> u32 {
        self.entry
    }

    fn read_entry(&self) -> u32 {
        self.read_entry
    }

    fn patch(&self) -> u32 {
        self.patch
    }

    fn set_entry(&mut self, entry: u32) {
        self.entry = entry;
    }

    fn key(&self) -> Key {
        Known::key(self)
    }

    fn label(&self) -> String {
        Known::label(self)
    }

    /// The name and the subname, so `guard` and `<trade supplies>` both
    /// find a row.
    fn haystack(&self) -> String {
        match self.subname.as_deref() {
            Some(subname) => format!("{} <{subname}>", self.name).to_lowercase(),
            None => self.name.to_lowercase(),
        }
    }

    fn with_edits(&self, edits: &Edits) -> Self {
        let key = Known::key(self);
        if !edits.touches(creature::TEMPLATE, &key) {
            return self.clone();
        }
        let get = |column: &str| edits.get(creature::TEMPLATE, &key, column);
        let number = |column: &str| get(column).and_then(|value| value.trim().parse::<f64>().ok());
        let mut out = self.clone();
        out.claim = edits.life(creature::TEMPLATE, &key);
        if let Some(name) = get("name") {
            out.name = unquote(name);
        }
        if let Some(subname) = get("subname") {
            out.subname = Some(unquote(subname));
        }
        if let Some(id) = number("display_id1") {
            out.display_id = id as u32;
        }
        if let Some(scale) = number("display_scale1") {
            out.display_scale = scale as f32;
        }
        if let Some(level) = number("level_min") {
            out.level.0 = level as u32;
        }
        if let Some(level) = number("level_max") {
            out.level.1 = level as u32;
        }
        if let Some(faction) = number("faction") {
            out.faction = faction as u32;
        }
        if let Some(flags) = number("npc_flags") {
            out.npc_flags = flags as u32;
        }
        if let Some(rank) = number("rank") {
            out.rank = rank as u32;
        }
        out
    }

    fn from_claim(key: &Key, row: &RowEdit) -> Option<Self> {
        Known::from_store(key, row)
    }
}

impl Template for Known {
    fn claim(&self) -> Life {
        self.claim
    }

    fn set_read_entry(&mut self, entry: u32) {
        self.read_entry = entry;
    }

    fn display_id(&self) -> u32 {
        self.display_id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn scale(&self) -> f32 {
        self.display_scale
    }
}

/// One row of the brief template query.
pub(crate) fn read_known(row: &creature::Row) -> Option<Known> {
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

/// What the map query already says about a creature, which is everything a
/// created spawn of the same creature needs.
pub(crate) fn known_from(spawn: &Spawn) -> Option<Known> {
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
        at: Vec3::new(number("position_x"), number("position_y"), number("position_z")),
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

/// The guid a preview body is given.
///
/// High enough that it cannot collide with anything a session puts in the
/// world: a real creature guid is a `u32` with a type mask in its high word,
/// and nothing in a session reaches this range. The spawn's own guid is in the
/// low bits so the two can be matched back up.
const PREVIEW_GUID_BASE: u64 = 0x7000_0000_0000_0000;

/// The dome drawn over a spawn with no model, in yards: a size in the world,
/// drawn through `Marks::visible_radius` so a far one is still a dot. A spawn
/// is picked by its distance from the pointer's ray in pixels, so a far one is
/// as easy to click as a near one.
const DOME_RADIUS: f32 = 0.6;

/// The smallest dome drawn around a spawn's model, in yards: about the
/// height of a person's waist, so the dome reads as around the model.
const DOME_AROUND_MODEL: f32 = 1.2;

/// How far up from the row's position the marker's body reaches, in yards.
///
/// A creature model stands on its position, so the pointer is aimed at
/// something above the ground rather than at the ground itself.
const BODY: f32 = 2.0;

impl SpawnSubject for CreatureSubject {
    type Spawn = Spawn;
    type Known = Known;
    type Own = CreatureOwn;
    type Look = u32;

    const TOOL: Tool = Tool::Creatures;
    const WORKSPACE: Tool = Tool::CreatureData;
    const SPAWN_TABLE: vale_mangos::row::Table = creature::SPAWN_TABLE;
    const TEMPLATE_TABLE: vale_mangos::row::Table = creature::TEMPLATE_TABLE;
    const TABLES: &'static [vale_mangos::row::Table] = creature::TABLES;
    const NUMBERING: vale_mangos::row::Numbering = creature::NUMBERING;
    const RESERVED_GUID_BASE: u64 = creature::RESERVED_GUID_BASE;
    const MAX_SPAWN_GUID_QUERY: &'static str = creature::MAX_SPAWN_GUID_QUERY;
    const MAX_ENTRY_QUERY: &'static str = creature::MAX_ENTRY_QUERY;
    const WORDS: Words = Words {
        noun: "creature",
        nouns: "creatures",
        spawn: "spawn",
        spawns: "spawns",
        new_name: "New Creature",
        edit: "Edit creature",
    };
    const FLAGS: Flags = Flags {
        open: "--spawn",
        add: "--spawn-add",
        delete: "--spawn-delete",
        pick: "--pick",
        new: "--template-new",
        entry: "--template-entry",
    };
    const DISPLAY_COLUMN: &'static str = "display_id1";
    const DISPLAY_TABLE: crate::subjects::displays::Table = crate::subjects::displays::Table::Creature;
    const HOLDER: crate::subjects::quests::Holder = crate::subjects::quests::Holder::Creature;
    const BODY: f32 = BODY;
    // Wider than a light's twelve, because a creature marker is a body rather
    // than a dot and the whole body is the target.
    const HANDLE_PIXELS: f32 = 18.0;
    const MARK_RADIUS: f32 = DOME_RADIUS;
    const ORDINARY: Color = Color::srgb(0.45, 0.80, 0.55);
    const PREVIEW_GUID_BASE: u64 = PREVIEW_GUID_BASE;
    // 200 creatures is about what a busy city block holds, and is well inside
    // what the entity pass draws in a session. The panel changes it and shows
    // the count.
    const MODEL_BUDGET: usize = 200;
    // A little past the 7x7 block the editor streams at its default reach, so
    // a marker appears as its ground does.
    const RANGE: f32 = 1200.0;
    const DISCARDS_PATHS: bool = true;

    fn spawn_key(guid: u64) -> Key {
        creature::spawn_key(guid)
    }

    fn spawns_query(map: u32, patch: u32) -> String {
        creature::spawns_on_map_query(map, patch)
    }

    fn template_query(entry: u32, patch: u32) -> String {
        creature::template_query(entry, patch)
    }

    fn spawn_query(guid: u64) -> String {
        creature::spawn_query(guid)
    }

    fn templates_query(entries: &[u32], patch: u32) -> Option<String> {
        creature::templates_query(entries, patch)
    }

    /// A creature has no type the picker narrows by.
    fn search_query(term: &str, _of_type: Option<u32>, patch: u32, limit: usize) -> String {
        creature::search_query(term, patch, limit)
    }

    fn all_templates_query(patch: u32) -> String {
        creature::all_templates_query(patch)
    }

    fn new_spawn(entry: u32, map: u32, at: (f32, f32, f32), orientation: f32) -> Vec<vale_mangos::row::Assignment> {
        creature::new_spawn(entry, map, at, orientation)
    }

    fn new_template(name: &str) -> Vec<vale_mangos::row::Assignment> {
        creature::new_template(name)
    }

    fn read_spawn(row: &creature::Row) -> Spawn {
        read_spawn(row)
    }

    fn read_known(row: &creature::Row) -> Option<Known> {
        read_known(row)
    }

    fn known_of(spawn: &Spawn) -> Option<Known> {
        known_from(spawn)
    }

    /// A creature entry with no template row at this patch. vmangos refuses
    /// to load a spawn of it. Its name states the fault in the same words the
    /// map query uses for a spawn whose join found no row.
    fn unknown(entry: u32) -> Known {
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

    fn created(guid: u64, row: &RowEdit, known: Option<&Known>) -> Spawn {
        let number = |column: &str| super::spawn::stored_number(row, column);
        let entry = number("id").unwrap_or(0.0) as u32;
        Spawn {
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
        }
    }

    fn script(args: &crate::args::Args) -> Script {
        let spawns = &args.spawns;
        Script {
            open: spawns.spawn,
            add: spawns.spawn_add,
            delete: spawns.spawn_delete,
            pick: spawns.pick,
            new: spawns.template_new,
            entry: spawns.template_entry,
        }
    }

    fn workspace_row(args: &crate::args::Args) -> Option<u32> {
        args.subjects.creature_data
    }

    fn target(guid: u64) -> crate::context::Target {
        crate::context::Target::Creature { guid }
    }

    /// The ground's: a creature stands on the terrain, so a creature dragged
    /// across a hill follows the hill rather than flying at the height it was
    /// picked up from.
    fn drag_height(ground: Vec3, _grab: Vec3) -> f32 {
        ground.z
    }

    /// The one `orientation` column.
    fn face(session: &mut EditSession, guid: u64, _tilted: bool, orientation: f32, now: f64) {
        let subject = format!("{} {guid} facing", creature::SPAWN);
        session.set_server_edit(
            creature::SPAWN,
            &creature::spawn_key(guid),
            "orientation",
            Some(vale_mangos::sql::float(orientation)),
            Some(crate::server::store::Gesture { label: "Turn creature", subject: &subject, now }),
        );
    }

    /// A dome: translucent around a model's feet and never smaller than a
    /// person, or a smaller near-opaque one where a model would stand.
    fn mark(marks: &mut Marks, _spawn: &Spawn, at: Vec3, radius: f32, has_model: bool, colour: Color, look: Look) {
        match has_model {
            true => marks.dome(at, radius.max(DOME_AROUND_MODEL), colour.with_alpha(0.4), look),
            false => marks.dome(at, radius, colour.with_alpha(0.85), look),
        }
    }

    /// How far it wanders, which is a column of the row the form has open and
    /// hard to read as a number.
    fn mark_chosen(marks: &mut Marks, spawn: &Spawn, at: Vec3) {
        if spawn.wander > 0.1 {
            marks.flat_ring(at, spawn.wander, Color::srgba(1.0, 0.82, 0.25, 0.5), Look::Ghosted);
        }
    }

    fn look(spawn: &Spawn) -> u32 {
        spawn.display_id
    }

    fn body(guid: u64, spawn: &Spawn) -> WorldEntity {
        WorldEntity {
            guid,
            kind: ObjectType::Unit,
            name: spawn.name.clone().unwrap_or_default(),
            display_id: Some(spawn.display_id),
            // The scale is in the transform and not here. `place_entities`
            // reads this field, and it skips an entity with no entry in
            // `Motion`, which none of these has. See [`transform_for`].
            scale: Some(1.0),
            ..WorldEntity::default()
        }
    }

    fn transform(spawn: &Spawn) -> Transform {
        transform_for(spawn)
    }

    /// Only a template with a model: a creature's ghost is the model alone.
    fn ghost_wanted(known: &Known) -> bool {
        known.display_id != 0
    }

    /// A unit made translucent through the client's own pass:
    /// `tint::fade_models` fades a unit whose `UNIT_FIELD_BYTES_1` vis flags
    /// mark it as a ghost, and the ghost is a body that is not in the world.
    /// No seam is added, and the creature's appearance is decided only by the
    /// client.
    fn ghost_body(guid: u64, known: &Known) -> WorldEntity {
        WorldEntity {
            guid,
            kind: ObjectType::Unit,
            name: known.name.clone(),
            display_id: Some(known.display_id),
            scale: Some(1.0),
            vis_flags: vale_protocol::state::objects::UNIT_VIS_FLAGS_GHOST,
            ..WorldEntity::default()
        }
    }

    /// The waypoint mode takes the pointer when it is using it. It runs first,
    /// and without this check a click it took would also land on the tool: a
    /// press on a node would also select the creature under it, and with Add
    /// armed every point placed would re-select the creature it was placed
    /// over. The two conditions are the ones `waypoints::press` acts on, so a
    /// click that mode ignores still reaches the tool.
    ///
    /// This check is also ahead of the placer. That is the one state in which
    /// two modes both want the click. Arming Place disarms the waypoint mode,
    /// so the state is reached only by arming the waypoints afterwards, and
    /// then the open waypoint window takes the click.
    fn yields_press(waypoints: Option<&super::waypoints::Waypoints>) -> bool {
        waypoints.is_some_and(|waypoints| waypoints.open.is_some() && (waypoints.adding || waypoints.hovered.is_some()))
    }

    /// `--waypoints` presses the Waypoints button on the form just opened.
    fn opened(args: &crate::args::Args, spawn: &Spawn, waypoints: &mut super::waypoints::Waypoints) {
        if args.spawns.waypoints {
            waypoints.showing = true;
            waypoints.open_for(spawn.guid, spawn.entry, spawn.movement_type);
        }
    }

    fn writes() -> &'static dyn super::spawn::SpawnWrites {
        &CreatureSubject
    }
}

/// Registers the creature tool: the shared mechanism over
/// [`CreatureSubject`], and the service marks.
pub struct CreatureToolPlugin;

impl Plugin for CreatureToolPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SpawnToolPlugin::<CreatureSubject>::default())
            .init_resource::<ServiceMarks>()
            // After the client has put the camera where the rig says, so the
            // marks are projected through this frame's view and do not trail
            // a moving camera by a frame.
            .add_systems(
                Update,
                mark_services
                    .after(super::spawn::lay_the_edits::<CreatureSubject>)
                    .after(vale_client::world::camera::place),
            );
    }
}

/// Where a preview body stands, in Bevy's axes.
///
/// The scale is written here rather than onto the `WorldEntity`, because that
/// field is read by `world::entity::place_entities`, which skips an entity
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
/// by `crate::ui::shell::servicemarks`.
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
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    camera: Query<(&Camera, &Transform), With<WorldCamera>>,
    bodies: Query<(
        &super::spawn::Preview<CreatureSubject>,
        &Transform,
        &vale_client::world::entities::EntityModel,
    )>,
) {
    marks.0.clear();
    if !state.editing() || *tool != Tool::Creatures || !creatures.own.show_services {
        return;
    }
    let Ok((camera, camera_at)) = camera.single() else {
        return;
    };
    let view = GlobalTransform::from(*camera_at);
    let eye = camera_at.translation;
    let forward = *camera_at.forward();
    // The name point of each body that has its model, by the spawn's guid,
    // gathered when the first creature in range offers a service.
    let mut heads: Option<HashMap<u64, Vec3>> = None;
    for (rank, index) in creatures.near.iter().enumerate() {
        let Some(base) = creatures.base_spawn(*index) else {
            continue;
        };
        // The near list is sorted by distance from the focus, not from the
        // camera, so the whole list is walked; the cheap tests come first.
        let ground = vale_client::render::axes::to_bevy(base.at.to_array());
        if ground.distance(eye) > SERVICE_MARK_RANGE {
            continue;
        }
        // As drawn, with the edits `lay_the_edits` laid over it.
        let Some(spawn) = creatures.near_drawn(rank) else {
            continue;
        };
        if spawn.npc_flags & SERVICE_BITS == 0 || spawn.is_removed() {
            continue;
        }
        let heads = heads.get_or_insert_with(|| {
            bodies
                .iter()
                .map(|(preview, at, model)| {
                    let height = model.name_anchor * at.scale.y + SERVICE_MARK_LIFT;
                    (preview.guid, at.translation + Vec3::Y * height)
                })
                .collect()
        });
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

#[cfg(test)]
mod tests {
    use super::super::spawn::{SpawnRow, TemplateSubject};
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
        let (was, to) = Creatures::plan_template_move(&shown, 2_000_005, false)
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
        assert_eq!(Creatures::plan_template_move(&shown, 68, true).unwrap(), None);
        assert!(Creatures::plan_template_move(&shown, 0, false).is_err());
        assert!(Creatures::plan_template_move(&shown, creature::MAX_ENTRY + 1, false).is_err());
        assert!(Creatures::plan_template_move(&shown, 1, true).is_err());
        assert!(Creatures::plan_template_move(&shown, 2_000_000, false).unwrap().is_some());
        // Back to where the database has it is allowed although the entry is
        // taken, because it is taken by this row.
        let moved = TemplateSubject {
            entry: 2_000_000,
            ..shown
        };
        assert!(Creatures::plan_template_move(&moved, 68, true).unwrap().is_some());
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

        creatures.mode = crate::tools::placements::place::Mode::Place;
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
