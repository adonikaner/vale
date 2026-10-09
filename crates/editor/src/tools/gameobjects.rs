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
//! The whole map is read in one query, once per map and again after an apply or
//! a put-back. The nearest [`GameObjects::model_budget`] spawns are drawn as
//! models through the client's own entity pipeline, as a synthetic
//! [`WorldEntity`] of kind `GameObject`, and the rest as rings. A click
//! selects; a drag on what is already selected moves it. Place mode searches
//! the templates and puts a new row where the ground is clicked. Spawns this
//! project creates are rebuilt from the store and drawn beside the database's.
//! The reasons for each of those are in that module and are not repeated here.
//!
//! ## What differs from the creature tool
//!
//! * There are no waypoints. A game object does not move.
//! * A spawn's facing is three columns: `orientation`, and the rotation
//!   quaternion's `z` and `w` beside it. See [`GameObjects::turn`] and
//!   `vale_mangos::gameobject::facing`.
//! * The state is drawn. `gameobject.state` is what the client poses a door
//!   open or shut by, so the preview body carries it.
//! * The picker narrows by type. 2,332 of the templates are spell focuses and
//!   2,297 are chairs, so a search for a vein is narrowed to the chests.
//! * The template's `data` columns are named by its type. The form that does
//!   this is in [`crate::ui::gameobjects`].
//!
//! The tool is inactive during a playtest, for the creature tool's reason: the
//! server spawns the real objects then.

use super::creatures::TemplateSubject;
use super::Tool;
use crate::server::fresh::Fresh;
use crate::marks::{Look, Marks};
use crate::session::EditSession;
use vale_client::render::focus::WorldFocus;
use vale_client::world::camera::WorldCamera;
use vale_client::world::session::{ObjectType, WorldEntity};
use vale_mangos::gameobject::{self, RowValue};
use vale_mangos::row::{Edits, Key, Life, RowEdit};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::{HashMap, HashSet};

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
    fn from_store(key: &Key, row: &RowEdit) -> Option<Known> {
        let entry = key.first()? as u32;
        let patch = key
            .0
            .iter()
            .find(|(column, _)| column == "patch")
            .and_then(|(_, value)| value.parse::<u32>().ok())?;
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
                .map(|literal| crate::ui::rowform::unquote(literal))
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
fn read_known(row: &gameobject::Row) -> Option<Known> {
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
    /// `tools::creatures::Spawn::claim`.
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
    /// `tools::creatures::Spawn::with_edits`.
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
                known.name = crate::ui::rowform::unquote(name);
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

/// The picker behind Place mode, and what a click on the ground will place.
#[derive(Debug, Default)]
pub struct NewSpawn {
    /// What has been typed.
    pub search: String,
    /// Which type the matches are narrowed to, or `None` for every type.
    pub of_type: Option<u32>,
    /// The search and type last sent, so a redraw does not re-query every
    /// frame.
    ran: Option<(String, Option<u32>)>,
    pub matches: Vec<Known>,
    task: Option<Task<Result<Vec<Known>, String>>>,
    /// The object a click will place, which is also the ghost on the cursor.
    pub chosen: Option<Known>,
    /// Why the search found nothing, when it is not that nothing matched.
    pub trouble: Option<String>,
    /// The `EditSession::database_writes` the matches were read at.
    searched_at: Option<u64>,
}

impl NewSpawn {
    pub fn searching(&self) -> bool {
        self.task.is_some()
    }

    /// Forget the matches, which a map change or an apply makes stale.
    pub fn forget(&mut self) {
        self.ran = None;
        self.matches.clear();
    }
}

/// How many matches the picker offers at once: fifty, the same as the creature
/// picker.
const MATCHES: usize = 50;

/// One row of `gameobject_template`, whole.
#[derive(Debug, Clone)]
pub struct TemplateRow {
    pub entry: u32,
    pub patch: u32,
    pub row: gameobject::Row,
}

/// One row of `gameobject`, whole.
#[derive(Debug, Clone)]
pub struct SpawnRow {
    pub guid: u64,
    pub row: gameobject::Row,
}

/// What a held left button is moving, once it has moved far enough to be a
/// drag. See `tools::creatures::Drag`.
#[derive(Debug, Clone, Copy)]
pub struct Drag {
    pub guid: u64,
    pub from: Vec3,
    pub pressed_at: Vec2,
    pub grab: Option<Vec3>,
    pub moving: bool,
    /// Whether the press landed on a spawn in a group of several; released
    /// without moving, it selects that spawn alone.
    pub collapse: bool,
}

/// What the pointer is doing with the game objects on the open map.
#[derive(Resource, Debug)]
pub struct GameObjects {
    /// Every spawn on the open map, as the database has it.
    pub spawns: Vec<Spawn>,
    /// The spawns this project creates, rebuilt from the store whenever it
    /// changes. Everything that draws, picks or counts walks both lists. See
    /// [`GameObjects::base_spawn`].
    pub created: Vec<Spawn>,
    created_for: Option<u64>,
    /// The templates this project creates, rebuilt from the store whenever it
    /// changes, sorted by entry. They are in no database, so no query returns
    /// them; the picker lists them above its search, and a spawn of one is
    /// labelled and drawn from here through [`Self::known`].
    pub templates: Vec<Known>,
    /// The highest entry the `gameobject_template` table holds, at any patch,
    /// read with the map. One of the inputs to a new template's entry; see
    /// [`GameObjects::next_entry`]. `None` until the read completes.
    pub max_entry: Option<u32>,
    /// Whether `--object-new` and `--object-entry` have been acted on, or were
    /// not asked for. `--apply-gameobjects` waits on it beside
    /// [`Self::scripted_done`].
    pub scripted_template_done: bool,
    /// The highest guid the `gameobject` table holds, read with the map.
    pub max_guid: Option<u64>,
    /// What an entry is, for the spawns of it this project creates.
    pub known: Fresh<HashMap<u32, Known>>,
    known_task: Option<Task<Result<Vec<Known>, String>>>,
    /// The model path a display id resolves to, worked out once per id,
    /// because the picker asks for every row on every frame it draws. `None` is
    /// an id `GameObjectDisplayInfo` does not resolve.
    pub models: HashMap<u32, Option<String>>,
    /// The display id picker, while it is open on the template's `displayId`.
    /// See `crate::tools::displays`.
    pub display_pick: Option<super::displays::DisplayPick>,
    /// `Lock.dbc` joined to `LockType.dbc`, read by the panel the first time a
    /// lock is drawn. See `crate::ui::gameobjects`. It is what identifies a
    /// chest as a mining vein: the server's row holds the lock's id and the
    /// client's file holds what the lock needs.
    pub locks: Option<vale_assets::tables::lock::Locks>,
    /// Which half of the panel is showing, and with it what a click does.
    pub mode: super::place::Mode,
    pub new_spawn: NewSpawn,
    /// Which way the object on the cursor faces, in radians, which is what a
    /// click writes. `None` while nothing is on the cursor. Set toward the
    /// camera when the placer is armed, kept from one placement to the next,
    /// and turned by [`Self::turn_ghost`]. See `tools::creatures::ghost`.
    ghost_facing: Option<f32>,
    task: Option<Task<Result<MapRead, String>>>,
    /// What [`Self::spawns`] was read for: the map, and how many times this
    /// session has written the tables. See `EditSession::database_writes`.
    loaded: Option<(u32, u64)>,
    /// Why there is nothing, when there is nothing.
    pub trouble: Option<String>,
    /// Indices over both lists that are near enough to draw, nearest first.
    pub near: Vec<usize>,
    built_at: Option<(u32, u32)>,
    pub hovered: Option<u64>,
    /// The primary of the selection, whose form is open.
    pub selected: Option<u64>,
    /// The rest of the selection, by guid, on the same terms as
    /// `tools::creatures::Creatures::also`.
    pub also: Vec<u64>,
    /// Where each member stood when a drag's button went down.
    group_from: Vec<(u64, Vec3)>,
    /// The selected spawn's whole template row, read on demand.
    pub template: Option<TemplateRow>,
    template_task: Option<Task<Result<Option<TemplateRow>, String>>>,
    /// What [`Self::template`] was made at: `EditSession::database_writes`,
    /// and whether the project created the row, in which case it is an empty
    /// row holding the key. Either changing makes it be made again: an apply
    /// or a put back changes what the database holds, and a discard turns a
    /// created row into one the database may hold.
    template_at: Option<(u64, bool)>,
    /// The selected spawn's whole spawn row, read on demand.
    pub spawn_row: Option<SpawnRow>,
    spawn_task: Option<Task<Result<Option<SpawnRow>, String>>>,
    /// …and the same for [`Self::spawn_row`].
    spawn_at: Option<(u64, bool)>,
    pub show_models: bool,
    /// Mark every near spawn, or only the selected, hovered and grouped ones.
    /// See `super::creatures::Creatures::show_markers`.
    pub show_markers: bool,
    pub model_budget: usize,
    /// How far out a spawn is drawn at all, in yards.
    pub range: f32,
    /// Whether the template window is open.
    pub template_window: bool,
    pub drag: Option<Drag>,
    /// A fly-to asked for by the panel, consumed by [`fly`].
    pub fly_to: Option<Vec3>,
    /// Whether the scripted placements and removals have finished, which
    /// `--apply-gameobjects` waits on.
    pub scripted_done: bool,
}

impl Default for GameObjects {
    fn default() -> GameObjects {
        GameObjects {
            spawns: Vec::new(),
            created: Vec::new(),
            created_for: None,
            templates: Vec::new(),
            max_entry: None,
            scripted_template_done: false,
            max_guid: None,
            known: Fresh::default(),
            known_task: None,
            models: HashMap::new(),
            display_pick: None,
            locks: None,
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
            show_markers: true,
            // A game object is a static model with no skeleton to pose and no
            // dressing to compose, so the budget is twice the creature tool's.
            model_budget: 400,
            // Shorter than the creature tool's 1200: the busiest tiles hold
            // several hundred objects each, and a ring a kilometre away is a
            // pixel.
            range: 800.0,
            template_window: false,
            drag: None,
            fly_to: None,
            scripted_done: false,
        }
    }
}

impl GameObjects {
    /// The entry a new template gets: the highest of the reserved base, one
    /// past what the table holds at any patch, and one past the highest this
    /// project has already claimed. These are the same three as
    /// `tools::creatures::Creatures::next_entry`, for the reasons given there.
    pub fn next_entry(&self, edits: &Edits) -> u32 {
        let in_database = self.max_entry.map(|highest| highest + 1).unwrap_or(0);
        let claimed = edits
            .rows()
            .filter(|(table, _, row)| *table == gameobject::TEMPLATE && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .map(|highest| highest as u32 + 1)
            .unwrap_or(0);
        gameobject::RESERVED_ENTRY_BASE.max(in_database).max(claimed)
    }

    /// Make a new template, choose it in the picker and open the template
    /// window on it. Every column is written at once, from
    /// `vale_mangos::gameobject::new_template`, at the server's own patch.
    /// The tool is put into Place with the new object chosen, because the next
    /// thing done with a new object is putting one in the world, and the
    /// window is opened because the thing before that is naming it and giving
    /// it a model.
    pub fn create_template(
        &mut self,
        session: &mut EditSession,
        name: &str,
        patch: u32,
        now: f64,
    ) -> u32 {
        let entry = self.next_entry(&session.server_edits);
        let key = gameobject::template_key(entry, patch);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in gameobject::new_template(name) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        let subject = format!("gameobject_template {entry}");
        session.set_server_row(
            gameobject::TEMPLATE,
            &key,
            Some(&row),
            Some(crate::session::Gesture { label: "New game object", subject: &subject, now }),
        );
        if let Some(known) = Known::from_store(&key, &row) {
            self.open_template(known);
        }
        entry
    }

    /// A copy of the template the window shows, under a new entry: every
    /// column as it is shown, the name suffixed " (copy)". `None` until the
    /// whole row has been read, since a copy of a half-read row would take
    /// the table's defaults for the columns that had not arrived.
    pub fn copy_template(
        &mut self,
        session: &mut EditSession,
        subject: &TemplateSubject,
        patch: u32,
        now: f64,
    ) -> Option<u32> {
        let from = self.template.as_ref().filter(|held| held.entry == subject.entry)?;
        let source = subject.key();
        let entry = self.next_entry(&session.server_edits);
        let key = gameobject::template_key(entry, patch);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for column in gameobject::TEMPLATE_COLUMNS.iter().filter(|column| column.editable()) {
            let value = session
                .server_edits
                .get(gameobject::TEMPLATE, &source, column.name)
                .map(str::to_string)
                .or_else(|| column.literal(&from.row));
            if let Some(value) = value {
                row.columns.insert(column.name.to_string(), value);
            }
        }
        let named = row
            .columns
            .get("name")
            .map(|literal| crate::ui::rowform::unquote(literal))
            .unwrap_or_default();
        row.columns
            .insert("name".to_string(), vale_mangos::sql::text(&format!("{named} (copy)")));
        let gesture = format!("gameobject_template {entry}");
        session.set_server_row(
            gameobject::TEMPLATE,
            &key,
            Some(&row),
            Some(crate::session::Gesture { label: "Copy game object", subject: &gesture, now }),
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
    /// giving it up removes nothing there.
    pub fn discard_template(&mut self, session: &mut EditSession, entry: u32, now: f64) {
        let Some(known) = self.templates.iter().find(|known| known.entry == entry).cloned()
        else {
            return;
        };
        let subject = format!("gameobject_template {entry}");
        session.set_server_row(
            gameobject::TEMPLATE,
            &known.key(),
            None,
            Some(crate::session::Gesture {
                label: "Discard new game object",
                subject: &subject,
                now,
            }),
        );
        if self.new_spawn.chosen.as_ref().is_some_and(|chosen| chosen.entry == entry) {
            self.new_spawn.chosen = None;
        }
        self.template = None;
        self.created_for = None;
    }

    /// Whether an entry is already a game object, as far as this tool can
    /// see: the templates behind the map's spawns, the picker's matches, the
    /// templates read for created spawns, and the ones this project creates.
    /// Asked by both entries a row has, because the database is where an
    /// apply runs. An entry the tool has not met is caught at the apply.
    pub fn entry_taken(&self, entry: u32) -> bool {
        self.spawns.iter().any(|spawn| spawn.entry == entry || spawn.read_entry == entry)
            || self
                .known
                .values()
                .chain(self.new_spawn.matches.iter())
                .chain(self.templates.iter())
                .any(|known| known.entry == entry || known.read_entry == entry)
    }

    /// Move a template to another entry, by re-keying the project's one claim
    /// on the row, with the project's own rows that name the entry following.
    /// See `tools::creatures::Creatures::rekey_template`, which this is for
    /// the other table.
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
        let at_base = gameobject::template_key(subject.read_entry, subject.patch);
        let gesture_subject = format!("gameobject_template {} entry", subject.entry);
        let gesture = crate::session::Gesture {
            label: "Move game object entry",
            subject: &gesture_subject,
            now,
        };
        crate::server::follow::rekey(
            session,
            gameobject::TEMPLATE,
            &from,
            &to_key,
            Some(&at_base),
            &gameobject::TEMPLATE_REFERENCES,
            gesture,
        )?;
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

    /// What the template window is about: the picker's chosen object while
    /// the tool is in Place, the selected spawn's template in Select.
    pub fn template_subject(&self, edits: &Edits) -> Option<TemplateSubject> {
        match self.mode {
            super::place::Mode::Place => {
                let known = self.new_spawn.chosen.as_ref()?;
                Some(TemplateSubject {
                    entry: known.entry,
                    read_entry: known.read_entry,
                    patch: known.patch,
                    claim: edits.life(gameobject::TEMPLATE, &known.key()),
                    label: known.label(),
                })
            }
            super::place::Mode::Select => {
                let spawn = self.chosen_edited(Some(edits))?;
                Some(TemplateSubject {
                    entry: spawn.entry,
                    read_entry: spawn.read_entry,
                    patch: spawn.patch(),
                    claim: edits.life(gameobject::TEMPLATE, &spawn.template_key()),
                    label: spawn.label(),
                })
            }
        }
    }

    /// Give every spawn and every known template the entry the project says
    /// its template has, so a moved template is drawn and keyed where the
    /// project puts it while the database still has it where it was read.
    /// Every row is reset first, so a move taken back returns the row to the
    /// entry it was read at. See `tools::creatures::Creatures::fold_template_moves`.
    pub fn fold_template_moves(&mut self, edits: &Edits) {
        let moves: Vec<(u32, u32, u32)> = edits
            .moves(gameobject::TEMPLATE)
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
            spawn.entry = moved(spawn.read_entry, spawn.patch());
            if let Some(known) = spawn.known.as_mut() {
                known.entry = moved(known.read_entry, known.patch);
            }
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
    /// them where a spawn of one and the picker find them. The picker's
    /// chosen object is replaced by its current row, so a rename in the
    /// window shows in the picker on the same frame, and one whose row was
    /// discarded is dropped.
    fn rebuild_templates(&mut self, edits: &Edits) {
        self.known.retain(|_, known| !known.is_new());
        let mut templates: Vec<Known> = edits
            .rows()
            .filter(|(table, _, row)| *table == gameobject::TEMPLATE && row.life == Life::Insert)
            .filter_map(|(_, key, row)| Known::from_store(key, row))
            .collect();
        templates.sort_by_key(|known| known.entry);
        for known in &templates {
            self.known.insert(known.entry, known.clone());
        }
        let chosen = match self.new_spawn.chosen.as_ref() {
            Some(chosen) if chosen.is_new() => {
                templates.iter().find(|known| known.entry == chosen.entry).cloned()
            }
            other => other.cloned(),
        };
        self.new_spawn.chosen = chosen;
        self.templates = templates;
    }

    /// How many spawns there are to draw: the database's and this project's.
    pub fn spawn_count(&self) -> usize {
        self.spawns.len() + self.created.len()
    }

    /// One of them, by an index over both lists. The database's spawns come
    /// first.
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
    /// project's edits over it.
    pub fn spawn_at(&self, index: usize, edits: Option<&Edits>) -> Option<Spawn> {
        let base = self.base_spawn(index)?;
        Some(edits.and_then(|edits| base.with_edits(edits)).unwrap_or_else(|| base.clone()))
    }

    /// The chosen spawn, with the project's edits over it.
    pub fn chosen_edited(&self, edits: Option<&Edits>) -> Option<Spawn> {
        let base = self.chosen()?;
        Some(edits.and_then(|edits| base.with_edits(edits)).unwrap_or_else(|| base.clone()))
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
        Some(edits.and_then(|edits| base.with_edits(edits)).unwrap_or_else(|| base.clone()))
    }

    /// Select one spawn, or none, drop the rest of the group, and drop the two
    /// whole rows so the new one's are read.
    pub fn select(&mut self, guid: Option<u64>) {
        self.make_primary(guid);
        self.also.clear();
    }

    /// Make `guid` the primary and keep the group. The two whole rows belong
    /// to the primary and are dropped when it changes.
    fn make_primary(&mut self, guid: Option<u64>) {
        if self.selected != guid {
            self.selected = guid;
            self.template = None;
            self.spawn_row = None;
        }
    }

    /// Whether a spawn is selected, as the primary or as a member.
    pub fn holds(&self, guid: u64) -> bool {
        self.selected == Some(guid) || self.also.contains(&guid)
    }

    /// Shift+click: add the spawn to the selection or take it out, as
    /// `tools::creatures::Creatures::toggle` does for creatures.
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

    /// The guid a new spawn gets: the highest of the reserved base, one past
    /// what the table holds, and one past what this project has claimed. These
    /// are the same three as `tools::creatures::Creatures::next_guid`, for the
    /// reasons given there.
    pub fn next_guid(&self, edits: &Edits) -> u64 {
        let in_database = self.max_guid.map(|highest| highest + 1).unwrap_or(0);
        let claimed = edits
            .rows()
            .filter(|(table, _, row)| *table == gameobject::SPAWN && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .map(|highest| highest + 1)
            .unwrap_or(0);
        gameobject::RESERVED_GUID_BASE.max(in_database).max(claimed)
    }

    /// Put a new spawn of that object on the ground here, with every column
    /// written at once, as one claim and one undo entry.
    pub fn create(
        &mut self,
        session: &mut EditSession,
        known: &Known,
        at: Vec3,
        orientation: f32,
        now: f64,
    ) -> u64 {
        let guid = self.next_guid(&session.server_edits);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in
            gameobject::new_spawn(known.entry, session.map_id, (at.x, at.y, at.z), orientation)
        {
            row.columns.insert(change.column.to_string(), change.value);
        }
        let subject = format!("gameobject {guid}");
        session.set_server_row(
            gameobject::SPAWN,
            &gameobject::spawn_key(guid),
            Some(&row),
            Some(crate::session::Gesture { label: "New game object", subject: &subject, now }),
        );
        self.known.entry(known.entry).or_insert_with(|| known.clone());
        self.select(Some(guid));
        guid
    }

    /// Add another spawn of the same object, two yards north, carrying every
    /// column this one has: the database's value where the project has not
    /// changed it, and the project's where it has.
    ///
    /// `None` until the whole row has been read: copying from a half-read row
    /// would take the defaults for the columns that had not arrived, and the
    /// rotation is one of them.
    pub fn duplicate(&mut self, session: &mut EditSession, now: f64) -> Option<u64> {
        let spawn = self.chosen_edited(Some(&session.server_edits))?;
        let from = self.spawn_row.as_ref().filter(|held| held.guid == spawn.guid)?;
        let key = spawn.key();
        let guid = self.next_guid(&session.server_edits);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in
            gameobject::new_spawn(spawn.entry, session.map_id, spawn.at.into(), spawn.orientation)
        {
            row.columns.insert(change.column.to_string(), change.value);
        }
        for column in gameobject::columns_of(gameobject::SPAWN) {
            if !column.editable() {
                continue;
            }
            let value = session
                .server_edits
                .get(gameobject::SPAWN, &key, column.name)
                .map(str::to_string)
                .or_else(|| column.literal(&from.row));
            if let Some(value) = value {
                row.columns.insert(column.name.to_string(), value);
            }
        }
        row.columns.insert("map".to_string(), session.map_id.to_string());
        row.columns.insert(
            "position_x".to_string(),
            vale_mangos::sql::float(spawn.at.x + APART),
        );
        let subject = format!("gameobject {guid}");
        session.set_server_row(
            gameobject::SPAWN,
            &gameobject::spawn_key(guid),
            Some(&row),
            Some(crate::session::Gesture { label: "Duplicate game object", subject: &subject, now }),
        );
        self.select(Some(guid));
        Some(guid)
    }

    /// Mark the spawn for removal. A spawn this project created has its claim
    /// dropped; one the database holds is marked [`Life::Delete`] with its
    /// column edits kept. See `tools::creatures::Creatures::remove`.
    pub fn remove(session: &mut EditSession, guid: u64, now: f64) {
        let key = gameobject::spawn_key(guid);
        let held = session.server_edits.row(gameobject::SPAWN, &key).cloned();
        let subject = format!("gameobject {guid} removal");
        let wanted = match held {
            Some(row) if row.life == Life::Insert => None,
            held => {
                let mut row = held.unwrap_or_default();
                row.life = Life::Delete;
                Some(row)
            }
        };
        session.set_server_row(
            gameobject::SPAWN,
            &key,
            wanted.as_ref(),
            Some(crate::session::Gesture { label: "Remove game object", subject: &subject, now }),
        );
    }

    /// Clear a spawn's removal mark, leaving the column edits it had.
    pub fn keep(session: &mut EditSession, guid: u64, now: f64) {
        let key = gameobject::spawn_key(guid);
        let mut row = session
            .server_edits
            .row(gameobject::SPAWN, &key)
            .cloned()
            .unwrap_or_default();
        row.life = Life::Update;
        let subject = format!("gameobject {guid} removal");
        let wanted = (!row.is_empty()).then_some(row);
        session.set_server_row(
            gameobject::SPAWN,
            &key,
            wanted.as_ref(),
            Some(crate::session::Gesture { label: "Cancel game object removal", subject: &subject, now }),
        );
    }

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
                Some(crate::session::Gesture { label: "Turn game object", subject: &subject, now }),
            );
        }
    }

    /// Ask for the map to be read again on the next frame, with the two whole
    /// rows the form draws and the picker's matches.
    pub fn forget(&mut self) {
        self.loaded = None;
        self.built_at = None;
        self.template = None;
        self.spawn_row = None;
        self.new_spawn.forget();
    }

    /// Throw away every edit this project makes to game objects: this
    /// subject's tables only, not the whole store, which every subject shares.
    pub fn discard(session: &mut EditSession) -> (usize, usize) {
        session.forget_server_edits(|table| gameobject::table_named(table).is_some(), false)
    }

    pub fn fly_to(&mut self, at: Vec3) {
        self.fly_to = Some(at);
    }

    /// Whether a read is in flight, for the panel.
    pub fn reading(&self) -> bool {
        self.task.is_some()
    }

    /// Whether a click in the world would put an object down.
    pub fn placing(&self) -> bool {
        self.mode == super::place::Mode::Place && self.new_spawn.chosen.is_some()
    }

    /// Turn the object on the cursor by `degrees` about up, snapped to the
    /// placer's step when `snap`. See `Creatures::turn_ghost`.
    pub fn turn_ghost(&mut self, degrees: f32, snap: bool) {
        if let Some(facing) = self.ghost_facing {
            self.ghost_facing = Some(super::spawn::turned(facing, degrees, snap));
        }
    }

    /// The model a display id names, memoised. See [`Self::models`].
    pub fn model_of(
        &mut self,
        assets: &vale_client::assets::GameAssets,
        display_id: u32,
    ) -> Option<String> {
        if let Some(had) = self.models.get(&display_id) {
            return had.clone();
        }
        let found = match display_id {
            0 => None,
            id => assets
                .display_tables()
                .ok()
                .and_then(|tables| tables.game_object(id))
                .map(|display| display.path),
        };
        self.models.insert(display_id, found.clone());
        found
    }
}

/// Whether a template may move to `to`, and the two keys of the move when it
/// may: the claim's key now, and the key it is re-keyed under. `taken` is
/// [`GameObjects::entry_taken`]'s answer for `to`; moving a row back to where
/// the database has it is allowed, because the entry is taken by this row.
/// `Ok(None)` is a move to the entry the row already has.
pub fn plan_template_move(
    subject: &TemplateSubject,
    to: u32,
    taken: bool,
) -> Result<Option<(Key, Key)>, String> {
    if to == subject.entry {
        return Ok(None);
    }
    if to == 0 {
        return Err("0 is not a game object entry".to_string());
    }
    if to > gameobject::MAX_ENTRY {
        return Err(format!(
            "{to} is above {}, the largest value `entry` can hold",
            gameobject::MAX_ENTRY
        ));
    }
    if taken && to != subject.read_entry {
        return Err(format!("gameobject_template entry {to} already exists"));
    }
    Ok(Some((subject.key(), gameobject::template_key(to, subject.patch))))
}

pub struct GameObjectToolPlugin;

impl Plugin for GameObjectToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameObjects>()
            .add_systems(
                Update,
                (
                    read_the_map,
                    rebuild_created,
                    learn_the_created,
                    search_objects,
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

/// Marks one of this tool's preview bodies.
#[derive(Component)]
pub struct Preview {
    pub guid: u64,
    /// Which display id and state this body was built from. A `WorldEntity`'s
    /// model is resolved once and its pose is chosen from the state, so a body
    /// whose display id or state has changed is despawned and spawned again.
    pub display_id: u32,
    pub state: u32,
}

/// The guid a preview body is given: clear of the creature tool's previews
/// (`0x7000…`) and of anything a session puts in the world.
const PREVIEW_GUID_BASE: u64 = 0x7100_0000_0000_0000;

/// The guid the ghost is given.
const GHOST_GUID: u64 = PREVIEW_GUID_BASE | 0xFFFF_FFFF;

/// How wide a spawn's marker is to the pointer, in pixels.
const HANDLE_PIXELS: f32 = 16.0;

/// How far from the original a duplicate lands, in yards.
const APART: f32 = 2.0;

/// The smallest box drawn around a spawn's model, in yards across: a vein or
/// a chest is about this size.
const BOX_AROUND_MODEL: f32 = 0.8;

/// How far up from the row's position the marker reaches, in yards. Half the
/// creature tool's: a vein or a chest is knee high.
const BODY: f32 = 1.0;

/// How far the pointer must travel before a drag moves anything, in pixels.
const DRAG_PIXELS: f32 = 6.0;

/// A spawn this project creates, which is in no database yet.
const NEW: Color = Color::srgb(0.45, 0.85, 1.0);

/// A spawn this project removes, which is still standing there until Apply.
const REMOVED: Color = Color::srgb(0.95, 0.30, 0.30);

/// An ordinary spawn. Blue-grey, so a map carrying both tools' marks reads as
/// two populations: the creature tool's are green.
const ORDINARY: Color = Color::srgb(0.55, 0.70, 0.90);

/// What one read of the map answered.
#[derive(Debug)]
pub struct MapRead {
    pub spawns: Vec<Spawn>,
    /// The highest guid in the whole table, since guids are global.
    pub highest: Option<u64>,
    /// The highest `gameobject_template.entry` at any patch, for a new
    /// template's number.
    pub highest_entry: Option<u32>,
}

/// Read the open map's spawns, on a task, when the map changes or this
/// session writes the tables.
fn read_the_map(
    mut objects: ResMut<GameObjects>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if let Some(task) = objects.task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            objects.task = None;
            match done {
                Ok(read) => {
                    info!("game objects: {} spawn(s) read", read.spawns.len());
                    for spawn in &read.spawns {
                        if let Some(known) = &spawn.known {
                            objects.known.insert(known.entry, known.clone());
                        }
                    }
                    objects.spawns = read.spawns;
                    objects.max_guid = read.highest;
                    objects.max_entry = read.highest_entry;
                    objects.trouble = None;
                    objects.built_at = None;
                    objects.created_for = None;
                }
                Err(e) => {
                    warn!("game objects: {e}");
                    objects.spawns.clear();
                    objects.trouble = Some(e);
                }
            }
        }
        return;
    }
    if !state.editing() || *tool != Tool::GameObjects {
        return;
    }
    let Some(session) = session else { return };
    let key = (session.map_id, session.database_writes);
    if objects.loaded == Some(key) {
        return;
    }
    // Set before the read: a failed one must not ask again on every frame.
    objects.loaded = Some(key);
    let Some((at, _source)) = settings.resolve() else {
        objects.spawns.clear();
        objects.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = super::creatures::server_patch(&settings);
    let map = session.map_id;
    objects.trouble = None;
    objects.task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let rows = db.rows(&gameobject::spawns_on_map_query(map, patch))?;
        let highest = db
            .row(gameobject::MAX_SPAWN_GUID_QUERY)?
            .and_then(|row| row.integer("guid"))
            .map(|guid| guid as u64);
        // The highest template entry at any patch, for a new template's
        // number, on `tools::creatures::read_the_map`'s reason.
        let highest_entry = db
            .row(gameobject::MAX_ENTRY_QUERY)?
            .and_then(|row| row.integer("entry"))
            .map(|entry| entry as u32);
        Ok(MapRead { spawns: rows.iter().map(read_spawn).collect(), highest, highest_entry })
    }));
}

/// Build the spawns this project creates from the store, whenever it
/// changes. See `tools::creatures::rebuild_created`.
fn rebuild_created(
    mut objects: ResMut<GameObjects>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !state.editing() || *tool != Tool::GameObjects {
        return;
    }
    let Some(session) = session else { return };
    if objects.created_for == Some(session.server_edit_revision) {
        return;
    }
    objects.created_for = Some(session.server_edit_revision);
    // The templates first: a created spawn below is labelled from `known`,
    // which the created templates are put into here, and every entry is
    // folded through the project's moves before anything reads it.
    objects.fold_template_moves(&session.server_edits);
    objects.rebuild_templates(&session.server_edits);

    let mut out: Vec<Spawn> = Vec::new();
    for (table, key, row) in session.server_edits.rows() {
        if table != gameobject::SPAWN || row.life != Life::Insert {
            continue;
        }
        let Some(guid) = key.first() else { continue };
        // Once applied it is a row the read carries, and that copy is the one
        // to draw.
        if objects.spawns.iter().any(|spawn| spawn.guid == guid) {
            continue;
        }
        let number = |column: &str| -> Option<f32> {
            row.columns.get(column)?.trim().parse::<f32>().ok()
        };
        // Only this map's: a project can carry spawns on several.
        if number("map").map(|map| map as u32) != Some(session.map_id) {
            continue;
        }
        let entry = number("id").unwrap_or(0.0) as u32;
        out.push(Spawn {
            guid,
            at: Vec3::new(
                number("position_x").unwrap_or(0.0),
                number("position_y").unwrap_or(0.0),
                number("position_z").unwrap_or(0.0),
            ),
            orientation: number("orientation").unwrap_or(0.0),
            tilted: gameobject::is_tilted(
                number("rotation0").unwrap_or(0.0),
                number("rotation1").unwrap_or(0.0),
            ),
            state: number("state").unwrap_or(1.0) as u32,
            entry,
            read_entry: entry,
            known: objects.known.get(&entry).cloned(),
            claim: Life::Insert,
        });
    }
    objects.created = out;
    objects.built_at = None;
}

/// Read the templates of objects this project has spawns of and the map does
/// not. This happens when a project is opened in a later session and its
/// created spawns name an entry that nothing else on this map is a spawn of.
fn learn_the_created(
    mut objects: ResMut<GameObjects>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    session: Option<Res<EditSession>>,
) {
    // The templates held were read from the database as it stood at the
    // counter; after an apply or a put back they are read again, and a read
    // still running is dropped with them.
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    if objects.known.renew(writes) {
        objects.known_task = None;
        objects.created_for = None;
    }
    if let Some(task) = objects.known_task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            objects.known_task = None;
            match done {
                Ok(rows) => {
                    for known in rows {
                        objects.known.insert(known.entry, known);
                    }
                    objects.created_for = None;
                }
                Err(e) => warn!("game object templates: {e}"),
            }
        }
        return;
    }
    if !state.editing() || *tool != Tool::GameObjects {
        return;
    }
    let missing: Vec<u32> = objects
        .created
        .iter()
        .map(|spawn| spawn.entry)
        .filter(|entry| *entry != 0 && !objects.known.contains_key(entry))
        .collect::<HashSet<u32>>()
        .into_iter()
        .collect();
    if missing.is_empty() {
        return;
    }
    let Some((at, _)) = settings.resolve() else { return };
    let patch = super::creatures::server_patch(&settings);
    // Remembered before the read, so an entry with no row does not start one
    // on every frame.
    for entry in &missing {
        objects.known.insert(*entry, unknown_object(*entry));
    }
    objects.known_task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let Some(sql) = gameobject::templates_query(&missing, patch) else {
            return Ok(Vec::new());
        };
        Ok(db.rows(&sql)?.iter().filter_map(read_known).collect())
    }));
}

/// Run the picker's search when what is typed or the type filter no longer
/// matches what was last asked.
///
/// A term of fewer than two characters is sent only when a type is chosen:
/// `%a%` over every template is thousands of rows, while every template of one
/// type, such as every chest, is a list somebody asked for.
fn search_objects(
    mut objects: ResMut<GameObjects>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    session: Option<Res<EditSession>>,
) {
    // The matches were read from the database as it stood at the counter; an
    // apply or a put back empties them, drops a search still running, and
    // sends the typed text again.
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    if objects.new_spawn.searched_at != Some(writes) {
        objects.new_spawn.searched_at = Some(writes);
        objects.new_spawn.task = None;
        objects.new_spawn.forget();
    }
    if let Some(task) = objects.new_spawn.task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            objects.new_spawn.task = None;
            match done {
                Ok(rows) => {
                    for known in &rows {
                        objects.known.insert(known.entry, known.clone());
                    }
                    objects.new_spawn.matches = rows;
                    objects.new_spawn.trouble = None;
                    // The matches are read at the entries the database has;
                    // a template this project moves is listed at its new one.
                    // See `fold_template_moves`.
                    objects.created_for = None;
                }
                Err(e) => {
                    objects.new_spawn.matches.clear();
                    objects.new_spawn.trouble = Some(e);
                }
            }
        }
        return;
    }
    if !state.editing()
        || *tool != Tool::GameObjects
        || objects.mode != super::place::Mode::Place
    {
        return;
    }
    let asked = (objects.new_spawn.search.trim().to_string(), objects.new_spawn.of_type);
    if objects.new_spawn.ran.as_ref() == Some(&asked) {
        return;
    }
    objects.new_spawn.ran = Some(asked.clone());
    let (term, of_type) = asked;
    if term.len() < 2 && of_type.is_none() {
        objects.new_spawn.matches.clear();
        return;
    }
    let Some((at, _)) = settings.resolve() else {
        objects.new_spawn.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = super::creatures::server_patch(&settings);
    objects.new_spawn.task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let sql = gameobject::search_query(&term, of_type, patch, MATCHES);
        Ok(db.rows(&sql)?.iter().filter_map(read_known).collect())
    }));
}

/// Collect the spawns near enough to draw, nearest first, rebuilt when the
/// focus crosses a tile.
fn collect_the_near(
    mut objects: ResMut<GameObjects>,
    focus: Res<WorldFocus>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !state.editing() || *tool != Tool::GameObjects || !focus.present {
        return;
    }
    let tile = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    if objects.built_at == Some(tile) {
        return;
    }
    objects.built_at = Some(tile);
    let eye = focus.position;
    let range = objects.range;
    let mut near: Vec<(i64, usize)> = (0..objects.spawn_count())
        .filter_map(|index| {
            let away = objects.base_spawn(index)?.at.distance(eye);
            (away <= range).then_some((away as i64, index))
        })
        .collect();
    near.sort_unstable();
    objects.near = near.into_iter().map(|(_, index)| index).collect();
}

/// Find the spawn the pointer is over. This is `tools::creatures::aim` with a
/// smaller body.
#[allow(clippy::too_many_arguments)]
fn aim(
    mut objects: ResMut<GameObjects>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !state.editing() || *tool != Tool::GameObjects {
        objects.hovered = None;
        return;
    }
    if objects.drag.is_some() {
        return;
    }
    if objects.placing() || !crate::ui::over_the_world(&viewport, &wants, &windows) {
        objects.hovered = None;
        return;
    }
    let Some((origin, direction)) = crate::pick::ray(&windows, &camera) else {
        objects.hovered = None;
        return;
    };
    let Ok(window) = windows.single() else { return };
    let Ok((camera, _)) = camera.single() else { return };
    let height = window.height().max(1.0);
    let fov = match camera.clip_from_view().to_cols_array()[5] {
        y if y.abs() > f32::EPSILON => y,
        _ => return,
    };
    let edits = session.as_ref().map(|session| &session.server_edits);
    let mut best: Option<(f32, u64)> = None;
    for index in &objects.near {
        let Some(base) = objects.base_spawn(*index) else { continue };
        let edited = edits.and_then(|edits| base.with_edits(edits));
        let spawn = edited.as_ref().unwrap_or(base);
        let middle = spawn.at + Vec3::Z * (BODY / 2.0);
        let to = middle - origin;
        let along = to.dot(direction);
        if along <= 0.0 {
            continue;
        }
        let grabbable = ((HANDLE_PIXELS / height) * 2.0 * along / fov).max(BODY / 2.0);
        if (to - direction * along).length() > grabbable {
            continue;
        }
        if best.is_none_or(|(nearest, _)| along < nearest) {
            best = Some((along, spawn.guid));
        }
    }
    objects.hovered = best.map(|(_, guid)| guid);
}

/// Put the chosen object on the ground under the pointer, facing the way the
/// ghost does. See `tools::creatures::place_one`.
#[allow(clippy::too_many_arguments)]
fn place_one(
    objects: &mut GameObjects,
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
    let Some(known) = objects.new_spawn.chosen.clone() else { return };
    let Some(session) = session.as_mut() else { return };
    // Without a surface there is nothing to place on; the placer stays armed.
    // The surface is used rather than the ground, so a chest clicked onto a
    // dock or a brazier onto a balcony stands on it. See [`crate::pick`].
    let Some(ground) = cursor.surface else {
        session.status = "no surface under the pointer; nothing placed".into();
        return;
    };
    let facing = objects.ghost_facing.unwrap_or_else(|| match camera.single() {
        Ok(camera) => {
            let eye = Vec3::from(vale_client::render::axes::to_wow(camera.translation()));
            (eye.y - ground.y).atan2(eye.x - ground.x)
        }
        Err(_) => 0.0,
    });
    let guid = objects.create(session, &known, ground, facing, time.elapsed_secs_f64());
    objects.template_window = false;
    session.status = format!("{} placed as guid {guid}", known.label());
    info!(
        "game objects: placed {} as guid {guid} at {:.1}, {:.1}, {:.1}",
        known.label(),
        ground.x,
        ground.y,
        ground.z
    );
}

/// Choose the spawn under the pointer. A press on one that is not selected
/// selects it; a press on the selected one arms a drag.
#[allow(clippy::too_many_arguments)]
pub fn press(
    mut objects: ResMut<GameObjects>,
    mut session: Option<ResMut<EditSession>>,
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<crate::pick::Cursor>,
    time: Res<Time>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
    gizmo: Res<super::gizmo::Gizmo>,
    keys: Res<ButtonInput<KeyCode>>,
    mut marquee: ResMut<super::group::Marquee>,
) {
    if !state.editing() || *tool != Tool::GameObjects {
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
    if objects.placing() {
        place_one(&mut objects, &mut session, &cursor, &viewport, &wants, &windows, &camera, &time);
        return;
    }
    // The four cases below are the creature tool's, and so is the reason a
    // click on nothing leaves the selection alone. See `creatures::press`.
    let adding = super::group::shift(&keys);
    let pointer = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position())
        .unwrap_or_default();
    let Some(guid) = objects.hovered else {
        if crate::ui::over_the_world(&viewport, &wants, &windows) {
            marquee.begin(Tool::GameObjects, pointer, adding);
        }
        return;
    };
    if adding {
        objects.toggle(guid);
        return;
    }
    if !objects.holds(guid) {
        objects.select(Some(guid));
        return;
    }
    let collapse = !objects.also.is_empty();
    objects.promote(guid);
    let edits = session.as_ref().map(|session| &session.server_edits);
    let from = objects.chosen_edited(edits).map(|spawn| spawn.at).unwrap_or_default();
    let group_from: Vec<(u64, Vec3)> = objects
        .also
        .iter()
        .filter_map(|&guid| objects.edited(guid, edits).map(|spawn| (guid, spawn.at)))
        .collect();
    objects.group_from = group_from;
    objects.drag = Some(Drag {
        guid,
        from,
        pressed_at: pointer,
        grab: None,
        moving: false,
        collapse,
    });
}

/// Select every drawn spawn whose marker is inside a finished rectangle, as
/// `creatures::enclose` does for creatures.
fn enclose(
    mut objects: ResMut<GameObjects>,
    mut marquee: ResMut<super::group::Marquee>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if *tool != Tool::GameObjects {
        return;
    }
    let Some(done) = marquee.finished(Tool::GameObjects) else {
        return;
    };
    let Some((camera, eye, window)) = super::group::view(&camera, &windows) else {
        return;
    };
    let edits = session.as_ref().map(|session| &session.server_edits);
    let mut found: Vec<(f32, u64)> = Vec::new();
    for index in &objects.near {
        let Some(spawn) = objects.spawn_at(*index, edits) else {
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
        objects.select(None);
    }
    for (_, guid) in found {
        if objects.holds(guid) {
            continue;
        }
        match objects.selected {
            None => objects.make_primary(Some(guid)),
            Some(_) => objects.also.push(guid),
        }
    }
}

/// Move the held spawn once the pointer has travelled far enough, writing the
/// three position columns every frame under one gesture.
fn drag(
    mut objects: ResMut<GameObjects>,
    mut session: Option<ResMut<EditSession>>,
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<crate::pick::Cursor>,
    time: Res<Time>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    windows: Query<&Window>,
) {
    if !state.editing() || *tool != Tool::GameObjects {
        objects.drag = None;
        return;
    }
    let Some(held) = objects.drag else { return };
    let Some(session) = session.as_mut() else { return };

    if !buttons.pressed(MouseButton::Left) {
        objects.drag = None;
        objects.group_from.clear();
        session.history.release();
        if held.moving {
            let now = objects
                .chosen_edited(Some(&session.server_edits))
                .map(|spawn| spawn.at)
                .unwrap_or(held.from);
            session.status = match objects.also.len() {
                0 => format!(
                    "moved game object {} by {:.1} yards",
                    held.guid,
                    now.distance(held.from)
                ),
                more => format!(
                    "moved {} game objects by {:.1} yards",
                    more + 1,
                    now.distance(held.from)
                ),
            };
        } else if held.collapse {
            // A press on a member that never moved was a click on it.
            objects.also.clear();
        }
        return;
    }
    if !held.moving {
        let at = windows
            .single()
            .ok()
            .and_then(|window| window.cursor_position())
            .unwrap_or(held.pressed_at);
        if at.distance(held.pressed_at) >= DRAG_PIXELS {
            objects.drag = Some(Drag { moving: true, ..held });
        }
        return;
    }
    let Some(ground) = cursor.surface else { return };
    let grab = match held.grab {
        Some(grab) => grab,
        None => {
            objects.drag = Some(Drag { grab: Some(held.from - ground), ..held });
            return;
        }
    };
    // The height is the ground's plus the height the object already stood
    // above it. A lantern on a post and a sign on a wall are placed above the
    // terrain, and snapping one to the ground on the first frame of a drag
    // would bury it; `grab.z` is that offset.
    let want = Vec3::new(ground.x + grab.x, ground.y + grab.y, ground.z + grab.z);
    let now = time.elapsed_secs_f64();
    // One entry from the first write to the release, whatever pauses the
    // pointer makes on the way. See `vale_edit::undo::History::hold`.
    session.history.hold(now);
    if objects.group_from.is_empty() {
        move_one(session, held.guid, want, now);
        return;
    }
    // The rest of the group by the same step, each keeping its own height
    // above the terrain. See `super::group::carried`.
    let step = want - held.from;
    let label = format!("Move {} game objects", objects.group_from.len() + 1);
    let subject = format!("gameobject group {} position", held.guid);
    let members = objects.group_from.clone();
    session.as_one(&label, &subject, |session| {
        move_one(session, held.guid, want, now);
        for (guid, from) in members {
            let to = super::group::carried(session, from, from + step);
            move_one(session, guid, to, now);
        }
    });
}

/// Write one spawn's three position columns under one gesture subject, so a
/// move is one entry on the undo stack.
fn move_one(session: &mut EditSession, guid: u64, to: Vec3, now: f64) {
    let key = gameobject::spawn_key(guid);
    let subject = format!("gameobject {guid} position");
    for (column, value) in [("position_x", to.x), ("position_y", to.y), ("position_z", to.z)] {
        session.set_server_edit(
            gameobject::SPAWN,
            &key,
            column,
            Some(vale_mangos::sql::float(value)),
            Some(crate::session::Gesture { label: "Move game object", subject: &subject, now }),
        );
    }
}

/// Draw a box on every near spawn, and the selected one's facing.
///
/// A box rather than the creature tool's dome, so the two server tools' marks
/// differ by shape as well as by colour. A spawn drawn as a model gets a
/// translucent box around its base; one with no model a smaller, near-opaque
/// one. What is aimed at is [`Look::Ghosted`] and the rest [`Look::Solid`], for
/// the reason `super::creatures::draw` gives.
fn draw(
    mut marks: ResMut<Marks>,
    objects: Res<GameObjects>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
) {
    if !state.editing() || *tool != Tool::GameObjects {
        return;
    }
    let Ok(camera) = camera.single() else { return };
    let eye = Vec3::from(vale_client::render::axes::to_wow(camera.translation()));
    let edits = session.as_ref().map(|session| &session.server_edits);
    // Sorted once and searched per marker. See `super::doodads::draw_marker`.
    let members = super::group::sorted(objects.also.iter().copied());
    for (rank, index) in objects.near.iter().enumerate() {
        let Some(base) = objects.base_spawn(*index) else { continue };
        // With the markers off, only what is aimed at. See
        // [`GameObjects::show_markers`].
        let wanted = objects.selected == Some(base.guid)
            || objects.hovered == Some(base.guid)
            || super::group::holds(&members, base.guid);
        if !objects.show_markers && !wanted {
            continue;
        }
        let with_edits = edits.and_then(|edits| base.with_edits(edits));
        let spawn = with_edits.as_ref().unwrap_or(base);
        let chosen = objects.selected == Some(spawn.guid);
        let member = super::group::holds(&members, spawn.guid);
        let under = objects.hovered == Some(spawn.guid);
        let has_model =
            objects.show_models && rank < objects.model_budget && spawn.display_id() != 0;
        let edited = edits.is_some_and(|edits| edits.touches(gameobject::SPAWN, &spawn.key()));
        let colour = match (spawn.claim, chosen, under, edited) {
            (Life::Delete, _, _, _) => REMOVED,
            (_, true, _, _) => Color::srgb(1.0, 0.82, 0.25),
            (_, _, true, _) => Color::WHITE,
            // The rest of a group, paler than the primary.
            _ if member => super::creatures::MEMBER,
            (Life::Insert, _, _, _) => NEW,
            (_, _, _, true) => Color::srgb(0.95, 0.55, 0.20),
            _ => ORDINARY,
        };
        let aimed = chosen || member || under;
        let at = vale_client::render::axes::to_bevy(spawn.at.to_array());
        let radius = (spawn.at.distance(eye) * 0.010).clamp(0.25, 3.0);
        let look = match aimed {
            true => Look::Ghosted,
            false => Look::Solid,
        };
        let (side, colour) = match has_model {
            true => (radius.max(BOX_AROUND_MODEL) * 2.0, colour.with_alpha(0.4)),
            false => (radius * 2.0, colour.with_alpha(0.85)),
        };
        // Turned to the spawn's facing and standing on its position.
        let pose = Transform {
            translation: at + Vec3::Y * (side * 0.5),
            rotation: Quat::from_rotation_y(spawn.orientation),
            scale: Vec3::splat(side),
        };
        marks.cuboid(pose, colour, look);
        if spawn.is_removed() {
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
            // An arrow showing which way it faces, since a value such as 4.71
            // radians is hard to read as a direction.
            let facing = Vec3::new(spawn.orientation.cos(), spawn.orientation.sin(), 0.0);
            let from = vale_client::render::axes::to_bevy((spawn.at + Vec3::Z * 0.2).to_array());
            let nose = vale_client::render::axes::to_bevy(
                (spawn.at + facing * (radius * 3.0) + Vec3::Z * 0.2).to_array(),
            );
            let thickness = marks.line_radius(from) * 1.6;
            marks.arrow(from, nose, thickness, Color::srgb(1.0, 0.95, 0.6), Look::Ghosted);
        }
    }
}

/// Where a preview body stands, in Bevy's axes. The scale is the template's
/// `size`, in the transform for `tools::creatures::transform_for`'s reason.
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

/// Stand a model where each of the nearest spawns is, and take down the ones
/// that have left the list. See `tools::creatures::stand_them_up`.
fn stand_them_up(
    mut commands: Commands,
    objects: Res<GameObjects>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    mut standing: Query<(Entity, &Preview, &mut Transform)>,
) {
    let edits = session.as_ref().map(|session| &session.server_edits);
    let wanted: HashMap<u64, usize> =
        match (state.editing(), *tool == Tool::GameObjects, objects.show_models) {
            (true, true, true) => objects
                .near
                .iter()
                .take(objects.model_budget)
                .filter_map(|index| Some((objects.base_spawn(*index)?.guid, *index)))
                .collect(),
            _ => HashMap::new(),
        };

    let mut present = HashSet::new();
    for (entity, preview, mut transform) in &mut standing {
        let spawn = wanted.get(&preview.guid).and_then(|index| objects.spawn_at(*index, edits));
        match spawn {
            Some(spawn)
                if spawn.display_id() == preview.display_id && spawn.state == preview.state =>
            {
                present.insert(preview.guid);
                *transform = transform_for(&spawn);
            }
            _ => commands.entity(entity).despawn(),
        }
    }
    for (guid, index) in &wanted {
        if present.contains(guid) {
            continue;
        }
        let Some(spawn) = objects.spawn_at(*index, edits) else { continue };
        let display_id = spawn.display_id();
        if display_id == 0 {
            continue;
        }
        let name = spawn.known.as_ref().map(|known| known.name.as_str()).unwrap_or_default();
        commands.spawn((
            body(PREVIEW_GUID_BASE | guid, name, display_id, spawn.state),
            transform_for(&spawn),
            Visibility::default(),
            // `spawn_models` takes a `Sheath` in its query rather than as an
            // option, so an entity without one gets no model and no warning.
            vale_client::world::entities::Sheath::seeded(0),
            Preview { guid: *guid, display_id, state: spawn.state },
        ));
    }
}

/// The object on the cursor before it is placed.
#[derive(Component)]
pub struct Ghost {
    display_id: u32,
}

/// Keep the object to be placed standing under the pointer.
///
/// It is the client's own body, as a preview is. It is drawn solid: the
/// creature tool's ghost is faded by `tint::fade_models`, which reads a unit's
/// vis flags, and a game object has none. The square under it is drawn in the
/// new-spawn colour instead.
#[allow(clippy::too_many_arguments)]
fn ghost(
    mut commands: Commands,
    mut objects: ResMut<GameObjects>,
    mut marks: ResMut<Marks>,
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
    if objects.placing() && keys.just_pressed(KeyCode::Escape) && !wants.wants_keyboard_input() {
        objects.new_spawn.chosen = None;
    }
    let wanted = match state.editing()
        && *tool == Tool::GameObjects
        && objects.placing()
        && crate::ui::over_the_world(&viewport, &wants, &windows)
    {
        true => objects.new_spawn.chosen.clone().zip(cursor.surface),
        false => None,
    };
    let Some((known, at)) = wanted else {
        for (entity, _, _) in &standing {
            commands.entity(entity).despawn();
        }
        // Kept while the placer is armed, forgotten once nothing is on the
        // cursor. See `tools::creatures::ghost`.
        if !objects.placing() && objects.ghost_facing.is_some() {
            objects.ghost_facing = None;
        }
        return;
    };
    // Toward the camera the first time, and what it was left at after that.
    let facing = match objects.ghost_facing {
        Some(facing) => facing,
        None => {
            let toward = match camera.single() {
                Ok(camera) => {
                    let eye = Vec3::from(vale_client::render::axes::to_wow(camera.translation()));
                    (eye.y - at.y).atan2(eye.x - at.x)
                }
                Err(_) => 0.0,
            };
            objects.ghost_facing = Some(toward);
            toward
        }
    };

    // The mark, which is there whether or not the display id resolves.
    let centre = vale_client::render::axes::to_bevy(at.to_array());
    let nose = vale_client::render::axes::to_bevy(
        (at + Vec3::new(facing.cos(), facing.sin(), 0.0) * 1.5 + Vec3::Z * 0.2).to_array(),
    );
    let thickness = marks.line_radius(centre) * 1.6;
    marks.arrow(centre, nose, thickness, NEW, Look::Ghosted);
    marks.flat_ring(centre, 0.6, NEW, Look::Ghosted);

    let stand = Transform {
        translation: centre,
        rotation: vale_client::render::axes::body(facing, 0.0),
        scale: Vec3::splat(match known.size > 0.01 {
            true => known.size,
            false => 1.0,
        }),
    };
    let mut found = false;
    for (entity, ghost, mut transform) in &mut standing {
        match ghost.display_id == known.display_id && !found {
            true => {
                found = true;
                *transform = stand;
            }
            false => commands.entity(entity).despawn(),
        }
    }
    if found || known.display_id == 0 {
        return;
    }
    commands.spawn((
        body(GHOST_GUID, &known.name, known.display_id, 1),
        stand,
        Visibility::default(),
        vale_client::world::entities::Sheath::seeded(0),
        Ghost { display_id: known.display_id },
    ));
}

/// Take the camera to wherever the panel asked.
fn fly(mut objects: ResMut<GameObjects>, mut camera: ResMut<crate::camera::EditorCamera>) {
    let Some(at) = objects.fly_to.take() else { return };
    camera.target = at;
    camera.wants_the_ground = false;
}

/// Read the selected spawn's two whole rows, on a task. The map query reads
/// only the columns the drawing needs; the form needs all 34 columns of the
/// template and all 19 of the spawn, for one row.
pub fn fetch_rows(
    mut objects: ResMut<GameObjects>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if let Some(task) = objects.template_task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            objects.template_task = None;
            match done {
                Ok(row) => objects.template = row,
                Err(e) => warn!("game object template: {e}"),
            }
        }
    }
    if let Some(task) = objects.spawn_task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            objects.spawn_task = None;
            match done {
                Ok(row) => objects.spawn_row = row,
                Err(e) => warn!("game object spawn: {e}"),
            }
        }
    }
    if !state.editing() || *tool != Tool::GameObjects {
        return;
    }
    // A row is made again when the database has been written since it was
    // made, or when the project has started or stopped creating it; see
    // `crate::server::fresh`.
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    let Some((at, _)) = settings.resolve() else { return };
    // The template half follows the window's subject: the picker's object in
    // Place, the selected spawn's template in Select.
    let subject = session
        .as_ref()
        .and_then(|session| objects.template_subject(&session.server_edits));
    if let Some(subject) = subject {
        let wants_template = objects.template_task.is_none()
            && (objects.template.as_ref().is_none_or(|had| had.entry != subject.entry)
                || objects.template_at != Some((writes, subject.is_new())));
        if wants_template {
            objects.template_at = Some((writes, subject.is_new()));
            if subject.is_new() {
                // A template this project creates has no row to read; its
                // columns are the store's own, so the form is given an empty
                // row holding only the key.
                let mut row = gameobject::Row::new();
                row.insert("entry".to_string(), Some(subject.entry.to_string()));
                objects.template =
                    Some(TemplateRow { entry: subject.entry, patch: subject.patch, row });
            } else {
                // Read from where the database has it, which is not the entry
                // the window shows while the project moves the template. A
                // row the database does not hold is an empty row, so it is
                // not asked for again on every frame.
                let (entry, read_entry, patch) = (subject.entry, subject.read_entry, subject.patch);
                let at = at.clone();
                objects.template_task = Some(crate::server::queue::read(async move {
                    let mut db = vale_mangos::conn::Db::open(&at)?;
                    let row = db
                        .row(&gameobject::template_query(read_entry, patch))?
                        .unwrap_or_default();
                    Ok(Some(TemplateRow { entry, patch, row }))
                }));
            }
        }
    }
    let Some(spawn) = objects.chosen().cloned() else { return };
    // A spawn this project creates has no row to read: the form is given an
    // empty one carrying the key, and every column draws the store's value.
    if spawn.is_new() && (objects.spawn_row.as_ref().is_none_or(|had| had.guid != spawn.guid) || objects.spawn_at != Some((writes, true))) {
        let mut row = gameobject::Row::new();
        row.insert("guid".to_string(), Some(spawn.guid.to_string()));
        objects.spawn_at = Some((writes, true));
        objects.spawn_row = Some(SpawnRow { guid: spawn.guid, row });
    }
    let wants_spawn = !spawn.is_new()
        && objects.spawn_task.is_none()
        && (objects.spawn_row.as_ref().is_none_or(|had| had.guid != spawn.guid)
            || objects.spawn_at != Some((writes, false)));

    if wants_spawn {
        objects.spawn_at = Some((writes, false));
        let guid = spawn.guid;
        objects.spawn_task = Some(crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            Ok(db.row(&gameobject::spawn_query(guid))?.map(|row| SpawnRow { guid, row }))
        }));
    }
}

/// `--object-new`: make a new game object template, choose it in the picker
/// and open the template window on it. `--object-entry <n>`: renumber the
/// template the window is about, which is `--object-new`'s or `--object`'s.
/// These are `--template-new` and `--template-entry` for this table, for the
/// reasons given at `tools::creatures::template_on_the_command_line`.
fn template_on_the_command_line(
    args: Res<crate::Args>,
    mut objects: ResMut<GameObjects>,
    mut session: Option<ResMut<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    time: Res<Time>,
    mut done: Local<bool>,
    mut made: Local<bool>,
) {
    if *done {
        return;
    }
    if !args.object_new && args.object_entry.is_none() {
        *done = true;
        objects.scripted_template_done = true;
        return;
    }
    if objects.reading() || objects.loaded.is_none() {
        return;
    }
    if objects.trouble.is_some() {
        *done = true;
        objects.scripted_template_done = true;
        warn!("--object-new/--object-entry: there is no database to work against");
        return;
    }
    let Some(session) = session.as_mut() else { return };
    let now = time.elapsed_secs_f64();
    if args.object_new && !*made {
        *made = true;
        let patch = super::creatures::server_patch(&settings);
        let entry = objects.create_template(session, "New Game Object", patch, now);
        info!("--object-new: gameobject_template {entry} created at patch {patch}");
        session.status = format!("gameobject_template {entry} created");
    }
    if let Some(to) = args.object_entry {
        if !args.object_new && args.object.is_none() {
            *done = true;
            objects.scripted_template_done = true;
            warn!("--object-entry: nothing names a template; give --object-new or --object");
            return;
        }
        let Some(subject) = objects.template_subject(&session.server_edits) else { return };
        match objects.rekey_template(session, &subject, to, now) {
            Ok(()) => info!("--object-entry {to}: gameobject_template {} moved", subject.entry),
            Err(why) => warn!("--object-entry {to}: {why}"),
        }
    }
    *done = true;
    objects.scripted_template_done = true;
}

/// `--pick-display [id]`: open the display picker on the template window's
/// `displayId`, and with an id choose it. The creature tool's
/// `display_on_the_command_line` for this table, for the reasons given there.
fn display_on_the_command_line(
    args: Res<crate::Args>,
    mut objects: ResMut<GameObjects>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<super::Tool>,
    time: Res<Time>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    if !args.pick_display || *tool != super::Tool::GameObjects {
        *done = true;
        return;
    }
    if !objects.scripted_template_done {
        return;
    }
    let Some(session) = session.as_mut() else { return };
    let Some(subject) = objects.template_subject(&session.server_edits) else {
        if objects.scripted_done || objects.trouble.is_some() {
            *done = true;
            warn!("--pick-display: nothing names a template; give --object or --object-new");
        }
        return;
    };
    objects.template_window = true;
    let Some(row) = objects.template.as_ref().filter(|held| held.entry == subject.entry) else {
        return;
    };
    const COLUMN: &str = "displayId";
    let column = gameobject::column(gameobject::TEMPLATE, COLUMN).expect("a template column");
    let key = subject.key();
    let in_database = match subject.is_new() {
        true => None,
        false => column.literal(&row.row),
    };
    let showing: u32 = session
        .server_edits
        .get(gameobject::TEMPLATE, &key, COLUMN)
        .map(str::to_string)
        .or_else(|| in_database.clone())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let target = super::quests::ColumnTarget {
        table: gameobject::TEMPLATE,
        subject: format!("{} {} {COLUMN}", gameobject::TEMPLATE, key.text()),
        key,
        column: COLUMN,
        in_database,
        label: "Edit game object",
    };
    match args.display_id {
        Some(id) => {
            target.write(session, id.to_string(), time.elapsed_secs_f64());
            info!("--pick-display {id}: written to gameobject_template {} {COLUMN}", subject.entry);
        }
        None => {
            objects.display_pick = Some(super::displays::DisplayPick::open(
                super::displays::Table::Object,
                target,
                showing,
            ));
            info!("--pick-display: the picker is open on gameobject_template {} {COLUMN}", subject.entry);
        }
    }
    *done = true;
}

/// `--object <guid>`: open one spawn and fly to it once the map has been
/// read. Fires once.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    mut objects: ResMut<GameObjects>,
    mut quests: ResMut<super::quests::Quests>,
    mut session: Option<ResMut<EditSession>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(guid) = args.object else {
        *done = true;
        return;
    };
    if objects.spawns.is_empty() {
        if objects.trouble.is_some() {
            *done = true;
        }
        return;
    }
    *done = true;
    let Some(spawn) = objects.spawns.iter().find(|spawn| spawn.guid == guid).cloned() else {
        warn!("--object: no game object with guid {guid} on this map");
        if let Some(session) = session.as_mut() {
            session.status = format!("no game object spawn {guid} on this map");
        }
        return;
    };
    info!("--object: {} at {:.1}, {:.1}, {:.1}", spawn.label(), spawn.at.x, spawn.at.y, spawn.at.z);
    objects.select(Some(guid));
    objects.template_window = args.template;
    if args.creature_quests {
        quests.window_for =
            Some((super::quests::Holder::Object, spawn.entry, spawn.label()));
    }
    objects.fly_to(spawn.at);
    if let Some(session) = session.as_mut() {
        session.status = format!("{} — guid {guid}", spawn.label());
    }
}

/// `--object-add <entry>`, `--object-pick <entry>` and `--object-delete
/// <guid>`: the gestures of this tool a scripted run cannot otherwise make.
/// This follows `tools::creatures::placed_on_the_command_line` step for step,
/// including the three things it waits for.
pub fn placed_on_the_command_line(
    args: Res<crate::Args>,
    mut objects: ResMut<GameObjects>,
    mut session: Option<ResMut<EditSession>>,
    focus: Res<WorldFocus>,
    time: Res<Time>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    if args.object_add.is_none() && args.object_delete.is_none() && args.object_pick.is_none() {
        *done = true;
        objects.scripted_done = true;
        return;
    }
    if objects.reading() || objects.loaded.is_none() {
        return;
    }
    if objects.trouble.is_some() {
        *done = true;
        objects.scripted_done = true;
        warn!("--object-add/--object-delete: there is no database to work against");
        return;
    }
    let Some(session) = session.as_mut() else { return };
    let now = time.elapsed_secs_f64();

    if let Some(guid) = args.object_delete {
        if objects.selected != Some(guid) {
            match objects.spawns.iter().any(|spawn| spawn.guid == guid) {
                true => {
                    GameObjects::remove(session, guid, now);
                    objects.select(Some(guid));
                    info!("--object-delete: spawn {guid} is marked for removal");
                }
                false => warn!("--object-delete: no game object with guid {guid} on this map"),
            }
        }
    }

    let Some(entry) = args.object_add.or(args.object_pick) else {
        *done = true;
        objects.scripted_done = true;
        return;
    };
    // Through the picker, so the search is checked by the same run.
    if objects.new_spawn.search.is_empty() {
        objects.mode = super::place::Mode::Place;
        objects.new_spawn.search = entry.to_string();
        return;
    }
    if objects.new_spawn.searching() || objects.new_spawn.ran.is_none() {
        return;
    }
    let Some(known) = objects
        .new_spawn
        .matches
        .iter()
        .find(|known| known.entry == entry)
        .cloned()
    else {
        *done = true;
        objects.scripted_done = true;
        warn!("--object-add: no game object template with entry {entry}");
        return;
    };
    objects.new_spawn.chosen = Some(known.clone());
    if args.object_add.is_none() {
        *done = true;
        objects.scripted_done = true;
        info!("--object-pick: {} is on the cursor", known.label());
        return;
    }
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
    let guid = objects.create(session, &known, at, 0.0, now);
    objects.mode = super::place::Mode::Select;
    objects.new_spawn.chosen = None;
    objects.scripted_done = true;
    objects.fly_to(at);
    info!(
        "--object-add: {} placed as guid {guid} at {:.2}, {:.2}, {:.2}",
        known.label(),
        at.x,
        at.y,
        at.z
    );
    session.status = format!("{} placed as guid {guid}", known.label());
}

#[cfg(test)]
mod tests {
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
            plan_template_move(&shown, 2_000_005, false).expect("allowed").expect("a move");
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
        assert_eq!(plan_template_move(&shown, 1731, true).unwrap(), None);
        assert!(plan_template_move(&shown, 0, false).is_err());
        assert!(plan_template_move(&shown, gameobject::MAX_ENTRY + 1, false).is_err());
        assert!(plan_template_move(&shown, 1, true).is_err());
        assert!(plan_template_move(&shown, 2_000_000, false).unwrap().is_some());
        let moved = TemplateSubject { entry: 2_000_000, ..shown };
        assert!(plan_template_move(&moved, 1731, true).unwrap().is_some());
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
        objects.select(Some(29993));
        let edits = Edits::default();
        let shown = objects.template_subject(&edits).expect("the spawn's");
        assert_eq!((shown.entry, shown.patch, shown.claim), (1731, 0, Life::Update));
        assert_eq!(shown.key(), a_spawn().template_key());

        objects.mode = super::super::place::Mode::Place;
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
