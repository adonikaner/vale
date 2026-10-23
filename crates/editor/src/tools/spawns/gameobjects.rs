//! The server's game-object spawns, drawn on the map and picked with the
//! pointer.
//!
//! This tool does for the `gameobject` table what [`super::creatures`] does for
//! `creature`. A `gameobject` row carries a map and three coordinates, so it is
//! drawn where it stands, picked, dragged, and opened into a form over it and
//! the `gameobject_template` behind it. What a row means is
//! [`vale_mangos::gameobject`] and what a save does with one is
//! [`crate::server::gameobjects`].
//!
//! ## What is the same as the creature tool
//!
//! The mechanism, which is [`super::spawn`]: the whole map is read in one
//! query, once per map and again after an apply or a put-back. The nearest
//! [`SpawnTool::model_budget`] spawns are drawn as models through the client's
//! own entity pipeline, as a synthetic [`WorldEntity`] of kind `GameObject`,
//! and the rest as markers. A click selects; a drag on what is already
//! selected moves it. Place mode searches the templates and puts a new row
//! where the ground is clicked. Spawns this project creates are rebuilt from
//! the store and drawn beside the database's.
//!
//! ## What differs from the creature tool
//!
//! [`GameObjectSubject`] holds the differences:
//!
//! * There are no waypoints. A game object does not move.
//! * A spawn's facing is three columns: `orientation`, and the rotation
//!   quaternion's `z` and `w` beside it. See [`GameObjects::turn`] and
//!   `vale_mangos::gameobject::facing`.
//! * A drag keeps the height the object stood above the ground.
//! * The marker is a box, and the ghost is drawn solid with a mark under it.
//! * The state is drawn. `gameobject.state` is what the client poses a door
//!   open or shut by, so the preview body carries it.
//! * The picker narrows by type. 2,332 of the templates are spell focuses and
//!   2,297 are chairs, so a search for a vein is narrowed to the chests.
//! * The template's `data` columns are named by its type. The form that does
//!   this is in [`crate::ui::tools::spawns::gameobjects`].
//!
//! The tool is inactive during a playtest, for the creature tool's reason: the
//! server spawns the real objects then.

use super::spawn::{Flags, Script, SpawnSubject, SpawnTool, SpawnToolPlugin, Spawned, Template, Words};
use crate::marks::{Look, Marks};
use crate::session::EditSession;
use crate::subjects::listing::Listed;
use crate::tools::Tool;
use bevy::prelude::*;
use std::collections::HashMap;
use vale_client::world::entity::{ObjectType, WorldEntity};
use vale_mangos::gameobject::{self, RowValue};
use vale_mangos::row::{Edits, Key, Life, RowEdit};

/// What the pointer is doing with the game objects on the open map.
pub type GameObjects = SpawnTool<GameObjectSubject>;

/// The game object subject: `gameobject` and `gameobject_template`.
#[derive(Debug)]
pub struct GameObjectSubject;

/// What only the game object tool keeps.
#[derive(Debug, Default)]
pub struct GameObjectOwn {
    /// The model path a display id resolves to, worked out once per id,
    /// because the picker asks for every row on every frame it draws. `None` is
    /// an id `GameObjectDisplayInfo` does not resolve.
    pub models: HashMap<u32, Option<String>>,
    /// The `GameAssets::tables_generation` [`Self::models`] was resolved at.
    /// See `tools::spawns::creatures::CreatureOwn::worn_at`.
    pub models_at: u64,
    /// `Lock.dbc` joined to `LockType.dbc`, read by the panel the first time a
    /// lock is drawn. See `crate::ui::tools::spawns::gameobjects`. It is what
    /// identifies a chest as a mining vein: the server's row holds the lock's
    /// id and the client's file holds what the lock needs.
    pub locks: Option<vale_assets::tables::lock::Locks>,
}

/// A game-object entry: the part of its template that a marker, a model and a
/// panel heading need.
///
/// The map query carries these for every spawn, the picker's search for every
/// match, and [`learn_the_created`] for an entry neither has met.
#[derive(Debug, Clone, PartialEq)]
pub struct Known {
    /// The entry the project says the template has, which the list shows and
    /// the store keys its claim under.
    pub entry: u32,
    /// The entry the database has the row at, which differs from
    /// [`Self::entry`] while the project moves the template and has not
    /// applied it. A template the project creates has no row and the two are
    /// equal. See [`GameObjects::fold_template_moves`].
    pub read_entry: u32,
    /// [`Life::Insert`] for a template this project creates, which is in no
    /// database; [`Life::Update`] for one the database holds.
    pub claim: Life,
    /// Which content patch of the template the server would load.
    pub patch: u32,
    pub name: String,
    /// `gameobject_template.type`, which decides what the `data` columns mean.
    pub kind: u32,
    pub display_id: u32,
    /// `size`, the model's multiplier.
    pub size: f32,
    pub faction: u32,
    pub flags: u32,
    /// `data0`, `data1` and `data4`: the three columns a lock can be in, the
    /// second of which is also a chest's loot. See [`Known::lock`].
    pub data: [i64; 3],
}

impl Known {
    /// What to call it in a list.
    pub fn label(&self) -> String {
        format!("{} ({})", self.name, self.entry)
    }

    /// The key an edit to its template row is written under.
    pub fn key(&self) -> Key {
        gameobject::template_key(self.entry, self.patch)
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
        let number = |column: &str| -> i64 {
            row.columns
                .get(column)
                .and_then(|value| value.trim().parse::<f64>().ok())
                .unwrap_or(0.0) as i64
        };
        Some(Known {
            entry,
            read_entry: entry,
            claim: Life::Insert,
            patch,
            name: row
                .columns
                .get("name")
                .map(|literal| vale_mangos::sql::unquote(literal))
                .unwrap_or_default(),
            kind: number("type") as u32,
            display_id: number("displayId") as u32,
            size: row
                .columns
                .get("size")
                .and_then(|value| value.trim().parse::<f32>().ok())
                .unwrap_or(1.0),
            faction: number("faction") as u32,
            flags: number("flags") as u32,
            data: [number("data0"), number("data1"), number("data4")],
        })
    }

    /// What the type is called.
    pub fn type_word(&self) -> String {
        gameobject::value_word(&gameobject::TYPES, self.kind)
    }

    /// One of the three data columns this carries, by its index in the table.
    fn data_at(&self, index: usize) -> Option<i64> {
        match index {
            0 => Some(self.data[0]),
            1 => Some(self.data[1]),
            4 => Some(self.data[2]),
            _ => None,
        }
    }

    /// The `Lock.dbc` row this object is opened through, or `None` for a type
    /// with no lock or a lock of 0.
    ///
    /// The column depends on the type: `data0` on a chest and a goober, `data1`
    /// on a door, `data4` on a fishing hole.
    pub fn lock(&self) -> Option<u32> {
        let index = gameobject::lock_column(self.kind)?;
        self.data_at(index).filter(|lock| *lock > 0).map(|lock| lock as u32)
    }

    /// A chest's or a fishing pool's `gameobject_loot_template` entry. For a
    /// vein or a herb it is what is gathered from it. See
    /// `vale_mangos::gameobject::loot_column`, which is the server's own
    /// `GetLootId`.
    pub fn loot(&self) -> Option<u32> {
        let column = gameobject::loot_column(self.kind)?;
        self.data_at(column)
            .filter(|loot| *loot > 0)
            .map(|loot| loot as u32)
    }

    /// Whether the server takes loot from this type at all, whether or not
    /// the column names a set yet.
    pub fn is_looted(&self) -> bool {
        gameobject::loot_column(self.kind).is_some()
    }
}

/// An entry with no template row at this patch, which is a spawn the server
/// skips with one log line. Held so the read that failed to find it is not
/// started again on the next frame.
fn unknown_object(entry: u32) -> Known {
    Known {
        entry,
        read_entry: entry,
        claim: Life::Update,
        patch: 0,
        name: format!("entry {entry} — no template row"),
        kind: 0,
        display_id: 0,
        size: 1.0,
        faction: 0,
        flags: 0,
        data: [0; 3],
    }
}

/// One row of the brief template query.
pub(crate) fn read_known(row: &gameobject::Row) -> Option<Known> {
    let integer = |column: &str| row.integer(column).unwrap_or(0);
    Some(Known {
        entry: row.integer("entry")? as u32,
        read_entry: row.integer("entry")? as u32,
        claim: Life::Update,
        patch: integer("patch") as u32,
        name: row.text("name").unwrap_or_default().to_string(),
        kind: integer("type") as u32,
        display_id: integer("displayId") as u32,
        size: row.number("size").unwrap_or(1.0) as f32,
        faction: integer("faction") as u32,
        flags: integer("flags") as u32,
        data: [integer("data0"), integer("data1"), integer("data4")],
    })
}

/// One `gameobject` row, as the viewport needs it.
#[derive(Debug, Clone)]
pub struct Spawn {
    pub guid: u64,
    /// Where it is, in the world's own axes.
    pub at: Vec3,
    /// Radians it faces, as the row states it.
    pub orientation: f32,
    /// Whether `rotation0` or `rotation1` leans it off the vertical, so its
    /// quaternion is not a function of the facing. See
    /// `vale_mangos::gameobject::is_tilted`.
    pub tilted: bool,
    /// `gameobject.state`: 1 is ready, which a door draws as shut.
    pub state: u32,
    /// What stands here. `None` when the join found no template row, which is
    /// a spawn vmangos skips.
    pub known: Option<Known>,
    /// `gameobject.id`, kept beside [`Self::known`] because a spawn with no
    /// template row still has one. It is the entry the project says the
    /// template has; see [`Self::read_entry`].
    pub entry: u32,
    /// The template entry the map query read for this spawn, which is where
    /// the database has the template's row until a move of it is applied. See
    /// [`GameObjects::fold_template_moves`].
    pub read_entry: u32,
    /// What this project says is to become of the row. See
    /// `tools::spawns::creatures::Spawn::claim`.
    pub claim: Life,
}

impl Spawn {
    /// The key an edit to this spawn's `gameobject` row is written under.
    pub fn key(&self) -> Key {
        gameobject::spawn_key(self.guid)
    }

    /// The key an edit to this spawn's template is written under. The
    /// template is shared by every object of this kind.
    pub fn template_key(&self) -> Key {
        gameobject::template_key(self.entry, self.patch())
    }

    pub fn patch(&self) -> u32 {
        self.known.as_ref().map(|known| known.patch).unwrap_or(0)
    }

    pub fn display_id(&self) -> u32 {
        self.known.as_ref().map(|known| known.display_id).unwrap_or(0)
    }

    /// What to call it in a list.
    pub fn label(&self) -> String {
        match &self.known {
            Some(known) => known.label(),
            None => format!("entry {} — no template row", self.entry),
        }
    }

    pub fn is_new(&self) -> bool {
        self.claim == Life::Insert
    }

    pub fn is_removed(&self) -> bool {
        self.claim == Life::Delete
    }

    /// This row with the project's own edits laid over it, or `None` when the
    /// project says nothing about either of its two rows. The reason the
    /// drawing reads the store is given at
    /// `tools::spawns::creatures::Spawn::with_edits`.
    fn with_edits(&self, edits: &Edits) -> Option<Spawn> {
        let spawn_key = self.key();
        let template_key = self.template_key();
        if !edits.touches(gameobject::SPAWN, &spawn_key)
            && !edits.touches(gameobject::TEMPLATE, &template_key)
        {
            return None;
        }
        let mut out = self.clone();
        out.claim = edits.life(gameobject::SPAWN, &spawn_key);
        let number = |key: &Key, table: &'static str, column: &str| -> Option<f32> {
            edits.get(table, key, column)?.trim().parse().ok()
        };
        let spawn = |column: &str| number(&spawn_key, gameobject::SPAWN, column);
        if let Some(x) = spawn("position_x") {
            out.at.x = x;
        }
        if let Some(y) = spawn("position_y") {
            out.at.y = y;
        }
        if let Some(z) = spawn("position_z") {
            out.at.z = z;
        }
        if let Some(o) = spawn("orientation") {
            out.orientation = o;
        }
        if let Some(state) = spawn("state") {
            out.state = state as u32;
        }
        // Of the two tilt columns, only those the project has edited are known
        // here as numbers; an unedited one is known only through the bool the
        // read produced. So one edited column can add a lean but cannot remove
        // the lean the read found.
        out.tilted = match (spawn("rotation0"), spawn("rotation1")) {
            (Some(r0), Some(r1)) => gameobject::is_tilted(r0, r1),
            (Some(lean), None) | (None, Some(lean)) => {
                self.tilted || gameobject::is_tilted(lean, 0.0)
            }
            (None, None) => self.tilted,
        };
        if let Some(id) = spawn("id") {
            out.entry = id as u32;
        }
        if let Some(known) = out.known.as_mut() {
            let template = |column: &str| number(&template_key, gameobject::TEMPLATE, column);
            if let Some(display) = template("displayId") {
                known.display_id = display as u32;
            }
            if let Some(size) = template("size") {
                known.size = size;
            }
            if let Some(kind) = template("type") {
                known.kind = kind as u32;
            }
            if let Some(faction) = template("faction") {
                known.faction = faction as u32;
            }
            if let Some(flags) = template("flags") {
                known.flags = flags as u32;
            }
            for (slot, column) in ["data0", "data1", "data4"].into_iter().enumerate() {
                if let Some(value) = template(column) {
                    known.data[slot] = value as i64;
                }
            }
            if let Some(name) = edits.get(gameobject::TEMPLATE, &template_key, "name") {
                known.name = vale_mangos::sql::unquote(name);
            }
        }
        Some(out)
    }
}

/// One row of the map query as the viewport needs it.
fn read_spawn(row: &gameobject::Row) -> Spawn {
    let number = |column: &str| row.number(column).unwrap_or(0.0) as f32;
    // The join is a LEFT one, so a spawn whose template is missing has NULLs
    // where the template's columns are. `name` is never NULL on a template
    // row that exists, so it is the column tested.
    let known = row.text("name").map(|name| Known {
        entry: row.integer("id").unwrap_or(0) as u32,
        read_entry: row.integer("id").unwrap_or(0) as u32,
        claim: Life::Update,
        patch: row.integer("patch").unwrap_or(0) as u32,
        name: name.to_string(),
        kind: row.integer("type").unwrap_or(0) as u32,
        display_id: row.integer("displayId").unwrap_or(0) as u32,
        size: row.number("size").unwrap_or(1.0) as f32,
        faction: row.integer("faction").unwrap_or(0) as u32,
        flags: row.integer("flags").unwrap_or(0) as u32,
        data: [
            row.integer("data0").unwrap_or(0),
            row.integer("data1").unwrap_or(0),
            row.integer("data4").unwrap_or(0),
        ],
    });
    Spawn {
        guid: row.integer("guid").unwrap_or(0) as u64,
        at: Vec3::new(number("position_x"), number("position_y"), number("position_z")),
        orientation: number("orientation"),
        tilted: gameobject::is_tilted(number("rotation0"), number("rotation1")),
        state: row.integer("state").unwrap_or(1) as u32,
        entry: row.integer("id").unwrap_or(0) as u32,
        read_entry: row.integer("id").unwrap_or(0) as u32,
        known,
        claim: Life::Update,
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
        Spawn::patch(self)
    }

    fn at(&self) -> Vec3 {
        self.at
    }

    fn orientation(&self) -> f32 {
        self.orientation
    }

    fn tilted(&self) -> bool {
        self.tilted
    }

    fn claim(&self) -> Life {
        self.claim
    }

    fn display_id(&self) -> u32 {
        Spawn::display_id(self)
    }

    fn label(&self) -> String {
        Spawn::label(self)
    }

    fn with_edits(&self, edits: &Edits) -> Option<Spawn> {
        Spawn::with_edits(self, edits)
    }

    fn fold_entry(&mut self, moved: &dyn Fn(u32, u32) -> u32) {
        self.entry = moved(self.read_entry, Spawn::patch(self));
        if let Some(known) = self.known.as_mut() {
            known.entry = moved(known.read_entry, known.patch);
        }
    }
}

impl Listed for Known {
    const TABLE: &'static str = gameobject::TEMPLATE;

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

    /// The name and the type's word, so `chest` finds every chest.
    fn haystack(&self) -> String {
        format!("{} {}", self.name, self.type_word()).to_lowercase()
    }

    fn with_edits(&self, edits: &Edits) -> Self {
        let key = Known::key(self);
        if !edits.touches(gameobject::TEMPLATE, &key) {
            return self.clone();
        }
        let get = |column: &str| edits.get(gameobject::TEMPLATE, &key, column);
        let number = |column: &str| get(column).and_then(|value| value.trim().parse::<f64>().ok());
        let mut out = self.clone();
        out.claim = edits.life(gameobject::TEMPLATE, &key);
        if let Some(name) = get("name") {
            out.name = vale_mangos::sql::unquote(name);
        }
        if let Some(kind) = number("type") {
            out.kind = kind as u32;
        }
        if let Some(id) = number("displayId") {
            out.display_id = id as u32;
        }
        if let Some(size) = number("size") {
            out.size = size as f32;
        }
        if let Some(faction) = number("faction") {
            out.faction = faction as u32;
        }
        if let Some(flags) = number("flags") {
            out.flags = flags as u32;
        }
        // `Known::data` holds the three data columns a lock or loot is read
        // from, in this order.
        for (at, column) in ["data0", "data1", "data4"].into_iter().enumerate() {
            if let Some(value) = number(column) {
                out.data[at] = value as i64;
            }
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
        self.size
    }
}

/// The guid a preview body is given: clear of the creature tool's previews
/// (`0x7000…`) and of anything a session puts in the world.
const PREVIEW_GUID_BASE: u64 = 0x7100_0000_0000_0000;

/// Half the side of the box drawn over a spawn with no model, in yards. See
/// `super::creatures::DOME_RADIUS`.
const BOX_RADIUS: f32 = 0.5;

/// The smallest box drawn around a spawn's model, in yards across: a vein or
/// a chest is about this size.
const BOX_AROUND_MODEL: f32 = 0.8;

impl SpawnSubject for GameObjectSubject {
    type Spawn = Spawn;
    type Known = Known;
    type Own = GameObjectOwn;
    /// The display id and the state, because the pose is chosen from the state.
    type Look = (u32, u32);

    const TOOL: Tool = Tool::GameObjects;
    const WORKSPACE: Tool = Tool::GameObjectData;
    const SPAWN_TABLE: vale_mangos::row::Table = gameobject::SPAWN_TABLE;
    const TEMPLATE_TABLE: vale_mangos::row::Table = gameobject::TEMPLATE_TABLE;
    const TABLES: &'static [vale_mangos::row::Table] = gameobject::TABLES;
    const NUMBERING: vale_mangos::row::Numbering = gameobject::NUMBERING;
    const RESERVED_GUID_BASE: u64 = gameobject::RESERVED_GUID_BASE;
    const MAX_SPAWN_GUID_QUERY: &'static str = gameobject::MAX_SPAWN_GUID_QUERY;
    const MAX_ENTRY_QUERY: &'static str = gameobject::MAX_ENTRY_QUERY;
    const WORDS: Words = Words {
        noun: "game object",
        nouns: "game objects",
        spawn: "game object",
        spawns: "game objects",
        new_name: "New Game Object",
        edit: "Edit game object",
    };
    const FLAGS: Flags = Flags {
        open: "--object",
        add: "--object-add",
        delete: "--object-delete",
        pick: "--object-pick",
        new: "--object-new",
        entry: "--object-entry",
    };
    const DISPLAY_COLUMN: &'static str = "displayId";
    const DISPLAY_TABLE: crate::subjects::displays::Table = crate::subjects::displays::Table::Object;
    const HOLDER: crate::subjects::quests::Holder = crate::subjects::quests::Holder::Object;
    // Half the creature tool's: a vein or a chest is knee high.
    const BODY: f32 = 1.0;
    const HANDLE_PIXELS: f32 = 16.0;
    const MARK_RADIUS: f32 = BOX_RADIUS;
    // Blue-grey, so a map carrying both tools' marks reads as two
    // populations: the creature tool's are green.
    const ORDINARY: Color = Color::srgb(0.55, 0.70, 0.90);
    const PREVIEW_GUID_BASE: u64 = PREVIEW_GUID_BASE;
    // A game object is a static model with no skeleton to pose and no
    // dressing to compose, so the budget is twice the creature tool's.
    const MODEL_BUDGET: usize = 400;
    // Shorter than the creature tool's 1200: the busiest tiles hold several
    // hundred objects each, and a marker a kilometre away is a pixel.
    const RANGE: f32 = 800.0;
    const DISCARDS_PATHS: bool = false;

    fn spawn_key(guid: u64) -> Key {
        gameobject::spawn_key(guid)
    }

    fn spawns_query(map: u32, patch: u32) -> String {
        gameobject::spawns_on_map_query(map, patch)
    }

    fn template_query(entry: u32, patch: u32) -> String {
        gameobject::template_query(entry, patch)
    }

    fn spawn_query(guid: u64) -> String {
        gameobject::spawn_query(guid)
    }

    fn templates_query(entries: &[u32], patch: u32) -> Option<String> {
        gameobject::templates_query(entries, patch)
    }

    fn search_query(term: &str, of_type: Option<u32>, patch: u32, limit: usize) -> String {
        gameobject::search_query(term, of_type, patch, limit)
    }

    fn all_templates_query(patch: u32) -> String {
        gameobject::all_templates_query(patch)
    }

    fn new_spawn(entry: u32, map: u32, at: (f32, f32, f32), orientation: f32) -> Vec<vale_mangos::row::Assignment> {
        gameobject::new_spawn(entry, map, at, orientation)
    }

    fn new_template(name: &str) -> Vec<vale_mangos::row::Assignment> {
        gameobject::new_template(name)
    }

    fn read_spawn(row: &gameobject::Row) -> Spawn {
        read_spawn(row)
    }

    fn read_known(row: &gameobject::Row) -> Option<Known> {
        read_known(row)
    }

    fn known_of(spawn: &Spawn) -> Option<Known> {
        spawn.known.clone()
    }

    fn unknown(entry: u32) -> Known {
        unknown_object(entry)
    }

    fn created(guid: u64, row: &RowEdit, known: Option<&Known>) -> Spawn {
        let number = |column: &str| super::spawn::stored_number(row, column);
        let entry = number("id").unwrap_or(0.0) as u32;
        Spawn {
            guid,
            at: Vec3::new(
                number("position_x").unwrap_or(0.0),
                number("position_y").unwrap_or(0.0),
                number("position_z").unwrap_or(0.0),
            ),
            orientation: number("orientation").unwrap_or(0.0),
            tilted: gameobject::is_tilted(number("rotation0").unwrap_or(0.0), number("rotation1").unwrap_or(0.0)),
            state: number("state").unwrap_or(1.0) as u32,
            entry,
            read_entry: entry,
            known: known.cloned(),
            claim: Life::Insert,
        }
    }

    fn script(args: &crate::args::Args) -> Script {
        let spawns = &args.spawns;
        Script {
            open: spawns.object,
            add: spawns.object_add,
            delete: spawns.object_delete,
            pick: spawns.object_pick,
            new: spawns.object_new,
            entry: spawns.object_entry,
        }
    }

    fn workspace_row(args: &crate::args::Args) -> Option<u32> {
        args.subjects.object_data
    }

    fn target(guid: u64) -> crate::context::Target {
        crate::context::Target::GameObject { guid }
    }

    /// The ground's plus the height the object already stood above it. A
    /// lantern on a post and a sign on a wall are placed above the terrain,
    /// and snapping one to the ground on the first frame of a drag would bury
    /// it; `grab.z` is that offset.
    fn drag_height(ground: Vec3, grab: Vec3) -> f32 {
        ground.z + grab.z
    }

    fn face(session: &mut EditSession, guid: u64, tilted: bool, orientation: f32, now: f64) {
        GameObjects::turn(session, guid, tilted, orientation, now);
    }

    /// A box rather than the creature tool's dome, so the two server tools'
    /// marks differ by shape as well as by colour, turned to the spawn's
    /// facing and standing on its position.
    fn mark(marks: &mut Marks, spawn: &Spawn, at: Vec3, radius: f32, has_model: bool, colour: Color, look: Look) {
        let (side, colour) = match has_model {
            true => (radius.max(BOX_AROUND_MODEL) * 2.0, colour.with_alpha(0.4)),
            false => (radius * 2.0, colour.with_alpha(0.85)),
        };
        let pose = Transform {
            translation: at + Vec3::Y * (side * 0.5),
            rotation: Quat::from_rotation_y(spawn.orientation),
            scale: Vec3::splat(side),
        };
        marks.cuboid(pose, colour, look);
    }

    /// Only a spawn whose template names a model: one with none is a marker
    /// whatever the budget.
    fn marks_as_model(spawn: &Spawn) -> bool {
        spawn.display_id() != 0
    }

    fn look(spawn: &Spawn) -> (u32, u32) {
        (spawn.display_id(), spawn.state)
    }

    fn body(guid: u64, spawn: &Spawn) -> WorldEntity {
        let name = spawn.known.as_ref().map(|known| known.name.as_str()).unwrap_or_default();
        body(guid, name, spawn.display_id(), spawn.state)
    }

    fn transform(spawn: &Spawn) -> Transform {
        transform_for(spawn)
    }

    /// An arrow and a ring in the new-spawn colour, which are there whether or
    /// not the display id resolves: the body is drawn solid, because the
    /// client's fade reads a unit's vis flags and a game object has none.
    fn ghost_mark(marks: &mut Marks, centre: Vec3, at: Vec3, facing: f32) {
        let nose = vale_client::render::axes::to_bevy(
            (at + Vec3::new(facing.cos(), facing.sin(), 0.0) * 1.5 + Vec3::Z * 0.2).to_array(),
        );
        let thickness = marks.line_radius(centre) * 1.6;
        marks.arrow(centre, nose, thickness, super::spawn::NEW, Look::Ghosted);
        marks.flat_ring(centre, 0.6, super::spawn::NEW, Look::Ghosted);
    }

    fn ghost_body(guid: u64, known: &Known) -> WorldEntity {
        body(guid, &known.name, known.display_id, 1)
    }

    fn writes() -> &'static dyn super::spawn::SpawnWrites {
        &GameObjectSubject
    }
}

impl GameObjects {
    /// Turn a spawn to face a direction, as one undo entry.
    ///
    /// An upright object gets the facing and the quaternion's `z` and `w`
    /// together (`vale_mangos::gameobject::facing`), because the client is
    /// sent both and `UpdateRotationFields` derives the pair only when both are
    /// zero. A tilted one gets the facing alone: its quaternion carries the
    /// lean, which is not a function of the facing.
    pub fn turn(session: &mut EditSession, guid: u64, tilted: bool, orientation: f32, now: f64) {
        let key = gameobject::spawn_key(guid);
        let subject = format!("gameobject {guid} facing");
        let written = gameobject::facing(orientation);
        let columns = match tilted {
            true => &written[..1],
            false => &written[..],
        };
        for change in columns {
            session.set_server_edit(
                gameobject::SPAWN,
                &key,
                change.column,
                Some(change.value.clone()),
                Some(crate::server::store::Gesture { label: "Turn game object", subject: &subject, now }),
            );
        }
    }

    /// The model a display id names, memoised. See [`GameObjectOwn::models`].
    pub fn model_of(&mut self, assets: &vale_client::assets::GameAssets, display_id: u32) -> Option<String> {
        let generation = assets.tables_generation();
        let own = &mut self.own;
        if own.models_at != generation {
            own.models.clear();
            own.models_at = generation;
        }
        if let Some(had) = own.models.get(&display_id) {
            return had.clone();
        }
        let found = match display_id {
            0 => None,
            id => assets.display_tables().ok().and_then(|tables| tables.game_object(id)).map(|display| display.path),
        };
        own.models.insert(display_id, found.clone());
        found
    }
}

/// Registers the game object tool: the shared mechanism over
/// [`GameObjectSubject`].
pub struct GameObjectToolPlugin;

impl Plugin for GameObjectToolPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SpawnToolPlugin::<GameObjectSubject>::default());
    }
}

/// Where a preview body stands, in Bevy's axes. The scale is the template's
/// `size`, in the transform for `tools::spawns::creatures::transform_for`'s reason.
///
/// The rotation is the facing alone, which is how the client places a game
/// object the server sent. See `vale_assets::world::collision::object_matrix`.
/// A tilted spawn is therefore drawn upright, here and in a session.
fn transform_for(spawn: &Spawn) -> Transform {
    let size = spawn.known.as_ref().map(|known| known.size).unwrap_or(1.0);
    Transform {
        translation: vale_client::render::axes::to_bevy(spawn.at.to_array()),
        rotation: vale_client::render::axes::body(spawn.orientation, 0.0),
        scale: Vec3::splat(match size > 0.01 {
            true => size,
            false => 1.0,
        }),
    }
}

/// The synthetic entity a preview or a ghost is: a game object with a display
/// id and a state, which `spawn_models` and `animate` build and pose.
fn body(guid: u64, name: &str, display_id: u32, state: u32) -> WorldEntity {
    WorldEntity {
        guid,
        kind: ObjectType::GameObject,
        name: name.to_string(),
        display_id: Some(display_id),
        // The scale is in the transform. See [`transform_for`].
        scale: Some(1.0),
        // What the pose is chosen from: ready is shut, anything else is open.
        object_state: Some(state.min(255) as u8),
        ..WorldEntity::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::spawn::TemplateSubject;
    use super::*;

    fn a_vein() -> Known {
        Known {
            entry: 1731,
            read_entry: 1731,
            claim: Life::Update,
            patch: 0,
            name: "Copper Vein".into(),
            kind: gameobject::TYPE_CHEST,
            display_id: 310,
            size: 1.0,
            faction: 0,
            flags: 0,
            data: [38, 1502, 0],
        }
    }

    fn a_spawn() -> Spawn {
        Spawn {
            guid: 29993,
            at: Vec3::new(-445.19, -1529.4, 71.0),
            orientation: -1.8675,
            tilted: false,
            state: 1,
            entry: 1731,
            read_entry: 1731,
            known: Some(a_vein()),
            claim: Life::Update,
        }
    }

    /// A new template's entry is above the reserved base, above what the
    /// table holds at any patch, and above what this project has already
    /// claimed.
    #[test]
    fn a_new_templates_entry_is_above_everything_it_can_see() {
        let mut objects = GameObjects::default();
        let mut edits = Edits::default();
        assert_eq!(objects.next_entry(&edits), gameobject::RESERVED_ENTRY_BASE);
        objects.max_entry = Some(gameobject::RESERVED_ENTRY_BASE + 40);
        assert_eq!(objects.next_entry(&edits), gameobject::RESERVED_ENTRY_BASE + 41);
        let key = gameobject::template_key(gameobject::RESERVED_ENTRY_BASE + 100, 10);
        edits.set_life(gameobject::TEMPLATE, &key, Life::Insert);
        assert_eq!(objects.next_entry(&edits), gameobject::RESERVED_ENTRY_BASE + 101);
    }

    /// A created template is one row of the store, and renumbering it re-keys
    /// that row: one claim afterwards, at the new entry, still a creation
    /// with no origin. A spawn this project placed of it follows.
    #[test]
    fn renumbering_a_created_template_moves_its_one_row() {
        let mut edits = Edits::default();
        let from = gameobject::template_key(2_000_000, 10);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in gameobject::new_template("Test Object") {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(gameobject::TEMPLATE, &from, Some(&row.to_line()));
        let known = Known::from_store(&from, &row).expect("a template row");
        assert_eq!((known.entry, known.patch, known.kind), (2_000_000, 10, 5));
        assert_eq!(known.name, "Test Object");
        assert_eq!(known.size, 1.0);
        assert!(known.is_new());
        let shown = TemplateSubject {
            entry: 2_000_000,
            read_entry: 2_000_000,
            patch: 10,
            claim: Life::Insert,
            label: known.label(),
        };
        let (was, to) =
            GameObjects::plan_template_move(&shown, 2_000_005, false).expect("allowed").expect("a move");
        assert_eq!(was, from);
        assert_eq!(to, gameobject::template_key(2_000_005, 10));
        assert!(edits.rekey(gameobject::TEMPLATE, &was, &to, Some(&from)));
        let rows: Vec<(&str, &Key, &RowEdit)> = edits
            .rows()
            .filter(|(table, _, _)| *table == gameobject::TEMPLATE)
            .collect();
        assert_eq!(rows.len(), 1, "one claim, not two: {rows:?}");
        let (_, key, moved) = rows[0];
        assert_eq!(*key, to);
        assert_eq!(moved.life, Life::Insert);
        assert_eq!(moved.from, None);
        assert_eq!(moved.columns.len(), row.columns.len());
        let spawn = gameobject::spawn_key(10_000_000);
        edits.set(gameobject::SPAWN, &spawn, "id", Some("2000005".into()));
        let hits = crate::server::follow::hits(
            &edits,
            &gameobject::TEMPLATE_REFERENCES,
            2_000_005,
            2_000_009,
        );
        assert_eq!(hits.len(), 1, "{hits:?}");
    }

    /// The moves a template may not make: to 0, past the column, onto an
    /// entry that is already an object; and back to where the database has
    /// it, which is allowed although the entry is taken.
    #[test]
    fn a_template_move_is_refused_where_the_creature_tool_refuses_one() {
        let shown = TemplateSubject {
            entry: 1731,
            read_entry: 1731,
            patch: 0,
            claim: Life::Update,
            label: "Copper Vein (1731)".into(),
        };
        assert_eq!(GameObjects::plan_template_move(&shown, 1731, true).unwrap(), None);
        assert!(GameObjects::plan_template_move(&shown, 0, false).is_err());
        assert!(GameObjects::plan_template_move(&shown, gameobject::MAX_ENTRY + 1, false).is_err());
        assert!(GameObjects::plan_template_move(&shown, 1, true).is_err());
        assert!(GameObjects::plan_template_move(&shown, 2_000_000, false).unwrap().is_some());
        let moved = TemplateSubject { entry: 2_000_000, ..shown };
        assert!(GameObjects::plan_template_move(&moved, 1731, true).unwrap().is_some());
    }

    /// A template whose entry the project changes is shown at the new entry
    /// on every spawn of it, on the picker's matches and in the known table,
    /// and reset when the move is taken back.
    #[test]
    fn a_moved_template_is_folded_into_what_is_drawn() {
        let mut objects = GameObjects::default();
        objects.spawns = vec![a_spawn()];
        objects.known.insert(1731, a_vein());
        objects.new_spawn.matches = vec![a_vein()];
        objects.new_spawn.chosen = Some(a_vein());
        let mut edits = Edits::default();
        let from = gameobject::template_key(1731, 0);
        let to = gameobject::template_key(2_001_731, 0);
        edits.set(gameobject::TEMPLATE, &from, "size", Some("2".into()));
        assert!(edits.rekey(gameobject::TEMPLATE, &from, &to, None));
        objects.fold_template_moves(&edits);
        assert_eq!(objects.spawns[0].entry, 2_001_731);
        assert_eq!(objects.spawns[0].read_entry, 1731);
        assert_eq!(objects.spawns[0].template_key(), to);
        assert_eq!(objects.spawns[0].known.as_ref().map(|known| known.entry), Some(2_001_731));
        assert_eq!(objects.new_spawn.matches[0].entry, 2_001_731);
        assert_eq!(objects.new_spawn.chosen.as_ref().map(|known| known.entry), Some(2_001_731));
        assert!(objects.known.contains_key(&2_001_731));
        assert!(!objects.known.contains_key(&1731));
        assert_eq!(edits.get(gameobject::TEMPLATE, &objects.spawns[0].template_key(), "size"), Some("2"));
        assert!(edits.rekey(gameobject::TEMPLATE, &to, &from, None));
        objects.fold_template_moves(&edits);
        assert_eq!(objects.spawns[0].entry, 1731);
        assert!(objects.known.contains_key(&1731));
    }

    /// The window is about the picker's object in Place and the selected
    /// spawn's template in Select; a created template's claim reads as a
    /// creation, its chosen copy follows the store, and a discarded one is
    /// dropped.
    #[test]
    fn the_template_subject_follows_the_mode() {
        let mut objects = GameObjects::default();
        objects.spawns = vec![a_spawn()];
        objects.select_only(Some(29993));
        let edits = Edits::default();
        let shown = objects.template_subject(&edits).expect("the spawn's");
        assert_eq!((shown.entry, shown.patch, shown.claim), (1731, 0, Life::Update));
        assert_eq!(shown.key(), a_spawn().template_key());

        objects.mode = crate::tools::placements::place::Mode::Place;
        assert!(objects.template_subject(&edits).is_none());
        let mut edits = Edits::default();
        let key = gameobject::template_key(2_000_000, 10);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in gameobject::new_template("Test Object") {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(gameobject::TEMPLATE, &key, Some(&row.to_line()));
        objects.rebuild_templates(&edits);
        assert_eq!(objects.templates.len(), 1);
        assert!(objects.known.get(&2_000_000).is_some_and(Known::is_new));
        objects.new_spawn.chosen = objects.templates.first().cloned();
        let shown = objects.template_subject(&edits).expect("the picker's");
        assert_eq!((shown.entry, shown.patch, shown.claim), (2_000_000, 10, Life::Insert));
        assert_eq!(shown.label, "Test Object (2000000)");
        edits.set(gameobject::TEMPLATE, &key, "name", Some("'Renamed'".into()));
        objects.rebuild_templates(&edits);
        assert_eq!(
            objects.new_spawn.chosen.as_ref().map(|known| known.name.as_str()),
            Some("Renamed")
        );
        edits.set_row_line(gameobject::TEMPLATE, &key, None);
        objects.rebuild_templates(&edits);
        assert!(objects.templates.is_empty());
        assert!(objects.new_spawn.chosen.is_none());
        assert!(!objects.known.contains_key(&2_000_000));
    }

    /// A chest's lock is `data0` and its loot `data1`; a door's lock is
    /// `data1`; a fishing hole's is `data4`. The three columns the map query
    /// carries are exactly the three a lock can be in.
    #[test]
    fn the_lock_is_read_from_the_types_own_column() {
        let vein = a_vein();
        assert_eq!(vein.lock(), Some(38));
        assert_eq!(vein.loot(), Some(1502));

        let door = Known { kind: 0, data: [0, 85, 0], ..a_vein() };
        assert_eq!(door.lock(), Some(85));
        assert_eq!(door.loot(), None, "a door's data1 is its lock, not loot");

        let pool = Known { kind: 25, data: [5, 17280, 1628], ..a_vein() };
        assert_eq!(pool.lock(), Some(1628));

        let chair = Known { kind: 7, data: [3, 1, 0], ..a_vein() };
        assert_eq!(chair.lock(), None);
        // Every lock column is one of the three the queries read.
        for named in gameobject::TYPES {
            if let Some(index) = gameobject::lock_column(named.value) {
                assert!(a_vein().data_at(index).is_some(), "{} keeps its lock in data{index}", named.name);
            }
        }
    }

    /// A new guid clears the base, the table and the project. The table's own
    /// highest is 3,998,644 on the reference install.
    #[test]
    fn a_new_guid_clears_the_base_the_table_and_the_project() {
        let mut objects = GameObjects::default();
        let empty = Edits::default();
        assert_eq!(objects.next_guid(&empty), gameobject::RESERVED_GUID_BASE);
        objects.max_guid = Some(3_998_644);
        assert_eq!(objects.next_guid(&empty), gameobject::RESERVED_GUID_BASE);
        objects.max_guid = Some(gameobject::RESERVED_GUID_BASE + 4);
        assert_eq!(objects.next_guid(&empty), gameobject::RESERVED_GUID_BASE + 5);
        let mut edits = Edits::default();
        edits.set_life(
            gameobject::SPAWN,
            &gameobject::spawn_key(gameobject::RESERVED_GUID_BASE + 9),
            Life::Insert,
        );
        assert_eq!(objects.next_guid(&edits), gameobject::RESERVED_GUID_BASE + 10);
    }

    /// A creature spawn the project creates does not take a game-object guid,
    /// and the other way round: the two tables number separately.
    #[test]
    fn the_two_spawn_tables_number_separately() {
        let objects = GameObjects::default();
        let mut edits = Edits::default();
        edits.set_life(
            vale_mangos::creature::SPAWN,
            &vale_mangos::creature::spawn_key(gameobject::RESERVED_GUID_BASE + 50),
            Life::Insert,
        );
        assert_eq!(objects.next_guid(&edits), gameobject::RESERVED_GUID_BASE);
    }

    /// What the project says about a row reaches the drawn spawn: a removal, a
    /// move, a state, and a template edit of the columns the drawing reads.
    #[test]
    fn the_projects_edits_reach_the_drawn_spawn() {
        let spawn = a_spawn();
        assert!(spawn.with_edits(&Edits::default()).is_none());

        let mut edits = Edits::default();
        edits.set_life(gameobject::SPAWN, &spawn.key(), Life::Delete);
        assert!(spawn.with_edits(&edits).expect("claimed").is_removed());

        let mut edits = Edits::default();
        edits.set(gameobject::SPAWN, &spawn.key(), "state", Some("0".into()));
        edits.set(gameobject::TEMPLATE, &spawn.template_key(), "displayId", Some("311".into()));
        edits.set(gameobject::TEMPLATE, &spawn.template_key(), "data0", Some("39".into()));
        edits.set(gameobject::TEMPLATE, &spawn.template_key(), "name", Some("'Tin Vein'".into()));
        let drawn = spawn.with_edits(&edits).expect("claimed");
        assert_eq!(drawn.state, 0);
        assert_eq!(drawn.display_id(), 311);
        let known = drawn.known.expect("a template");
        assert_eq!(known.lock(), Some(39));
        assert_eq!(known.name, "Tin Vein");
    }

    /// The two keys a click reaches are different rows, and the template's
    /// carries the patch.
    #[test]
    fn a_click_reaches_a_spawn_key_and_a_template_key() {
        let spawn = a_spawn();
        assert_eq!(spawn.key().where_clause(), "`guid` = 29993");
        assert_eq!(spawn.template_key().where_clause(), "`entry` = 1731 AND `patch` = 0");
    }

    /// A `size` of 0 is drawn at 1, and the body carries the state the pose is
    /// chosen from.
    #[test]
    fn a_preview_carries_its_size_and_its_state() {
        let mut spawn = a_spawn();
        assert_eq!(transform_for(&spawn).scale, Vec3::splat(1.0));
        spawn.known.as_mut().expect("a template").size = 0.0;
        assert_eq!(transform_for(&spawn).scale, Vec3::splat(1.0));
        spawn.known.as_mut().expect("a template").size = 2.5;
        assert_eq!(transform_for(&spawn).scale, Vec3::splat(2.5));

        let built = body(PREVIEW_GUID_BASE | 7, "Door", 411, 0);
        assert_eq!(built.kind, ObjectType::GameObject);
        assert_eq!(built.object_state, Some(0));
        assert!(PREVIEW_GUID_BASE > u64::from(u32::MAX));
    }

    /// A spawn with no template row is drawn and named as one.
    #[test]
    fn a_spawn_with_no_template_says_so() {
        let mut spawn = a_spawn();
        assert_eq!(spawn.label(), "Copper Vein (1731)");
        spawn.known = None;
        assert_eq!(spawn.label(), "entry 1731 — no template row");
        assert_eq!(spawn.display_id(), 0);
    }
}
