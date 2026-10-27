//! The mechanism the creature and game object tools share, written once over
//! the subject: [`SpawnSubject`] is what the two differ in, and
//! [`SpawnTool`] is the state and the operations that follow from it.
//!
//! ```text
//! mod.rs      SpawnSubject and the two row traits under it, SpawnTool (the
//!             spawns on the open map, the selection, the picker, the two
//!             whole rows a form draws) and its operations: create, duplicate,
//!             remove, keep, and a template's create, copy, renumber and discard
//! systems.rs  the systems over a SpawnTool: the map read, the created rows,
//!             the picker's search, the near list and its tags, the pointer's
//!             aim, press, rectangle and drag, a press on the other tool's
//!             spawns, Go To, the markers, the models and the ghost, and the
//!             two whole rows a form reads
//! script.rs   the command-line flags that select, place, remove, create and
//!             renumber, and the display picker flag
//! keys.rs     the handles and keys for a selected spawn, which has a position
//!             and a facing and no other angle: the reader and writer the
//!             handles use, and the keys that move, turn, remove and
//!             duplicate it
//! ```
//!
//! ## What a subject decides
//!
//! The two tools are one mechanism over two pairs of tables (`creature` and
//! `creature_template`, `gameobject` and `gameobject_template`). The store, the
//! undo stack, the map read, the selection rules, the picker, the template
//! lifecycle and the command-line flags are the same. A subject decides:
//!
//! * its tables, its entry numbering and the references a renumber follows;
//! * the words its labels and status lines use;
//! * the drag height ([`SpawnSubject::drag_height`]): a creature stands on the
//!   ground, and a game object keeps the height it had above the ground when it
//!   was picked up;
//! * the facing writer ([`SpawnSubject::face`]): a creature's facing is one
//!   column, and an upright game object's is `orientation` with the two
//!   quaternion columns beside it;
//! * the marker ([`SpawnSubject::mark`]): a dome for a creature, a box for a
//!   game object, so the two populations differ by shape;
//! * the preview body and the ghost: a unit, faded by the client's own pass,
//!   or a game object with a state, drawn solid;
//! * what one spawn and one template row carry beyond the shared columns
//!   ([`Spawned`], [`Template`]).
//!
//! What a game object's form adds (the `data` columns named by the row's type,
//! a lock resolved through `Lock.dbc`) is in the subject's own view and its
//! `Known`, and does not reach this module.
//!
//! ## Both kinds at once
//!
//! Pools and game events hold creatures and game objects in one list, so a
//! few things here name a spawn of either kind:
//!
//! * [`SpawnKind`] and [`SpawnRef`] name a spawn by its table and guid, since
//!   the two tables number their guids independently.
//! * [`GoTo`] asks a tool to select a spawn and fly the camera to it, opening
//!   its map first when it stands on another one.
//! * [`Role`] says how a tool's systems run on a frame. A tool reads and draws
//!   its spawns when it is chosen, and also when the other spawn tool is
//!   chosen with [`OtherSpawns`] on. In the second case it draws faded markers
//!   only, and a click on one selects the spawn and switches to its tool.
//! * [`SpawnHover`] and the [`SpawnPointer`] sets run every tool's aim before
//!   any tool's press, so a press goes to the nearer spawn of either kind.
//! * [`Tag`] is what a spawn's pool and event rows say about drawing it:
//!   hidden by the view bar's event filter, existing only during an event,
//!   pooled, or in the pool that is open. The tags are built for the near list
//!   only, and rebuilt when the filter, the events or the pools change.

mod keys;
mod script;
mod systems;

pub(crate) use keys::{chosen, members, turned, SpawnKeysPlugin, Standing};
pub(crate) use systems::{fetch_rows, menu_target, press, Preview};
pub(crate) use systems::{lay_the_edits, NEW};

use crate::server::fresh::Fresh;
use crate::session::EditSession;
use crate::subjects::listing::{Listed, Listing};
use crate::tools::Tool;
use bevy::prelude::*;
use bevy::tasks::Task;
use std::collections::HashMap;
use std::marker::PhantomData;
use vale_mangos::row::{Assignment, Edits, Key, Life, Numbering, RowEdit, Table};

/// One spawn row, as the viewport needs it: the columns every spawn has, read
/// through one interface. The subject's own type carries the rest.
pub(crate) trait Spawned: Clone + std::fmt::Debug + Send + Sync + 'static {
    fn guid(&self) -> u64;
    /// The template entry the project says the spawn has.
    fn entry(&self) -> u32;
    /// The template entry the map query read, which is where the database has
    /// the template's row until a move of it is applied.
    fn read_entry(&self) -> u32;
    /// The content patch of the template row the server would load.
    fn patch(&self) -> u32;
    /// Where it is, in the world's own axes.
    fn at(&self) -> Vec3;
    /// Radians it faces, as the row states it.
    fn orientation(&self) -> f32;
    /// Whether the facing is not the whole of the row's turn. Only a game
    /// object leans; see `vale_mangos::gameobject::is_tilted`.
    fn tilted(&self) -> bool {
        false
    }
    /// What this project says is to become of the row.
    fn claim(&self) -> Life;
    /// The template's display id, 0 for none.
    fn display_id(&self) -> u32;
    /// What to call it in a list.
    fn label(&self) -> String;
    /// This row with the project's own edits laid over it, or `None` when the
    /// project changes neither of its two rows.
    fn with_edits(&self, edits: &Edits) -> Option<Self>;
    /// Give the spawn the entry the project says its template has: `moved`
    /// takes a read entry and a patch to the entry the project gives it.
    fn fold_entry(&mut self, moved: &dyn Fn(u32, u32) -> u32);
    fn is_new(&self) -> bool {
        self.claim() == Life::Insert
    }
    fn is_removed(&self) -> bool {
        self.claim() == Life::Delete
    }
}

/// One template row's brief columns: what a marker, a model, a heading and
/// the workspace's list need.
pub(crate) trait Template: Listed + PartialEq {
    /// [`Life::Insert`] for a template this project creates.
    fn claim(&self) -> Life;
    /// Give the row the entry the database has it at. Only a created row,
    /// which has no row in the database, moves its read entry with it.
    fn set_read_entry(&mut self, entry: u32);
    fn display_id(&self) -> u32;
    fn name(&self) -> &str;
    /// The model's size multiplier as the row states it. 0 means the model's
    /// own size, and is drawn at 1.
    fn scale(&self) -> f32;
    fn is_new(&self) -> bool {
        self.claim() == Life::Insert
    }
}

/// The words a subject's labels, status lines and log lines are made of.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Words {
    /// One row of the subject: `creature`, `game object`.
    pub(crate) noun: &'static str,
    /// More than one: `creatures`, `game objects`.
    pub(crate) nouns: &'static str,
    /// One spawn, in the spawn's own undo labels and status lines: `spawn`
    /// for a creature, `game object` for a game object.
    pub(crate) spawn: &'static str,
    /// More than one spawn.
    pub(crate) spawns: &'static str,
    /// The name a new template starts with.
    pub(crate) new_name: &'static str,
    /// The undo label of a column edit to either row: `Edit creature`.
    pub(crate) edit: &'static str,
}

/// The command-line flags a subject's scripted gestures are spelled with.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Flags {
    /// Select a spawn and fly to it.
    pub(crate) open: &'static str,
    /// Place a spawn of an entry.
    pub(crate) add: &'static str,
    /// Remove a spawn.
    pub(crate) delete: &'static str,
    /// Choose an entry in the picker and place nothing.
    pub(crate) pick: &'static str,
    /// Make a new template.
    pub(crate) new: &'static str,
    /// Renumber the template the window is about.
    pub(crate) entry: &'static str,
}

/// The values of a subject's [`Flags`] on this run.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Script {
    pub(crate) open: Option<u64>,
    pub(crate) add: Option<u32>,
    pub(crate) delete: Option<u64>,
    pub(crate) pick: Option<u32>,
    pub(crate) new: bool,
    pub(crate) entry: Option<u32>,
}

/// What the creature tool and the game object tool differ in. The rest of the
/// mechanism is [`SpawnTool`] and its systems; see the module comment.
pub(crate) trait SpawnSubject: std::fmt::Debug + Send + Sync + Sized + 'static {
    /// One spawn row as the viewport needs it.
    type Spawn: Spawned;
    /// One template row's brief columns.
    type Known: Template;
    /// State only this subject's tool keeps.
    type Own: Default + std::fmt::Debug + Send + Sync;
    /// What a preview body is built from. A body whose spawn no longer
    /// matches is despawned and spawned again, because a `WorldEntity`'s model
    /// is resolved once.
    type Look: Copy + PartialEq + std::fmt::Debug + Send + Sync + 'static;

    /// The rail tool.
    const TOOL: Tool;
    /// The workspace that lists every template.
    const WORKSPACE: Tool;
    const SPAWN_TABLE: Table;
    const TEMPLATE_TABLE: Table;
    const NUMBERING: Numbering;
    const RESERVED_GUID_BASE: u64;
    const MAX_SPAWN_GUID_QUERY: &'static str;
    const MAX_ENTRY_QUERY: &'static str;
    const WORDS: Words;
    const FLAGS: Flags;
    /// The template's first display column, which `--pick-display` opens.
    const DISPLAY_COLUMN: &'static str;
    const DISPLAY_TABLE: crate::subjects::displays::Table;
    /// Which holder a quest relation names this subject by.
    const HOLDER: crate::subjects::quests::Holder;
    /// Which spawn table this subject's spawns are rows of.
    const KIND: SpawnKind;
    /// How far up from the row's position the marker reaches, in yards.
    const BODY: f32;
    /// The pick radius of a marker, in pixels.
    const HANDLE_PIXELS: f32;
    /// The marker of a spawn with no model, in yards: a dome's radius or half
    /// a box's side.
    const MARK_RADIUS: f32;
    /// The marker colour of a spawn nothing else colours.
    const ORDINARY: Color;
    /// The high bits of a preview body's guid, clear of anything a session
    /// puts in the world and of the other subject's.
    const PREVIEW_GUID_BASE: u64;
    /// How many models are drawn at first.
    const MODEL_BUDGET: usize;
    /// How far out a spawn is drawn at first, in yards.
    const RANGE: f32;

    /// The table names, from the two tables.
    const SPAWN: &'static str = Self::SPAWN_TABLE.name;
    const TEMPLATE: &'static str = Self::TEMPLATE_TABLE.name;
    /// Other tables whose rows are keyed by the spawn's guid first and change
    /// how the spawn is drawn, so an edit to one lays the project's edits
    /// over that spawn again ([`SpawnTool::lay_edits`]). A creature's
    /// `creature_addon` row decides what it holds.
    const SPAWN_ROWS: &'static [&'static str] = &[];

    fn spawn_key(guid: u64) -> Key;
    fn template_key(entry: u32, patch: u32) -> Key {
        Self::NUMBERING.key(entry, patch)
    }
    fn spawns_query(map: u32, patch: u32) -> String;
    fn template_query(entry: u32, patch: u32) -> String;
    fn spawn_query(guid: u64) -> String;
    fn templates_query(entries: &[u32], patch: u32) -> Option<String>;
    /// The picker's search. `of_type` narrows it where the subject has types.
    fn search_query(term: &str, of_type: Option<u32>, patch: u32, limit: usize) -> String;
    /// Every template row the server would load, for the workspace's list.
    fn all_templates_query(patch: u32) -> String;
    /// Every column of a new spawn.
    fn new_spawn(entry: u32, map: u32, at: (f32, f32, f32), orientation: f32) -> Vec<Assignment>;
    /// Every column of a new template.
    fn new_template(name: &str) -> Vec<Assignment>;
    /// One row of the map query.
    fn read_spawn(row: &vale_mangos::schema::Row) -> Self::Spawn;
    /// One row of the brief template query.
    fn read_known(row: &vale_mangos::schema::Row) -> Option<Self::Known>;
    /// What the map query already says about a spawn's template.
    fn known_of(spawn: &Self::Spawn) -> Option<Self::Known>;
    /// An entry with no template row at this patch, held so the read that did
    /// not find it is not started again.
    fn unknown(entry: u32) -> Self::Known;
    /// A spawn this project creates, out of its row in the store, labelled
    /// from `known` when its template is known.
    fn created(guid: u64, row: &RowEdit, known: Option<&Self::Known>) -> Self::Spawn;
    /// The values of the subject's flags.
    fn script(args: &crate::args::Args) -> Script;
    /// The template the command line opens in the subject's workspace.
    fn workspace_row(args: &crate::args::Args) -> Option<u32>;
    /// What a right click on a spawn landed on.
    fn target(guid: u64) -> crate::context::Target;

    /// The height a drag puts the spawn at, from the surface under the
    /// pointer and the offset measured when the drag began.
    fn drag_height(ground: Vec3, grab: Vec3) -> f32;
    /// Write the facing, in radians in `0..2π`, as one undo entry.
    fn face(session: &mut EditSession, guid: u64, tilted: bool, orientation: f32, now: f64);
    /// Draw one spawn's marker at `at`, in Bevy's axes. `radius` is
    /// [`Self::MARK_RADIUS`] as it is drawn at this distance.
    #[allow(clippy::too_many_arguments)]
    fn mark(
        marks: &mut crate::marks::Marks,
        spawn: &Self::Spawn,
        at: Vec3,
        radius: f32,
        has_model: bool,
        colour: Color,
        look: crate::marks::Look,
    );
    /// What the selected spawn draws beside its facing arrow.
    fn mark_chosen(_marks: &mut crate::marks::Marks, _spawn: &Self::Spawn, _at: Vec3) {}
    /// Whether a spawn inside the model budget is marked as having a model.
    fn marks_as_model(_spawn: &Self::Spawn) -> bool {
        true
    }
    /// What a preview body is built from.
    fn look(spawn: &Self::Spawn) -> Self::Look;
    /// A preview body for a spawn, given its guid.
    fn body(guid: u64, spawn: &Self::Spawn) -> vale_client::world::entity::WorldEntity;
    /// The sheath state a preview body starts with: 0, stowed, unless the
    /// subject says the spawn stands with a weapon in hand. It seeds the
    /// client's `Sheath`, so a body spawned holding its weapon does not play
    /// the draw on its first frame.
    fn sheath(_spawn: &Self::Spawn) -> u8 {
        0
    }
    /// Where a preview body stands, in Bevy's axes.
    fn transform(spawn: &Self::Spawn) -> Transform;
    /// Whether the chosen template is put on the cursor at all.
    fn ghost_wanted(_known: &Self::Known) -> bool {
        true
    }
    /// What is drawn under the ghost: `centre` in Bevy's axes, `at` in the
    /// world's.
    fn ghost_mark(_marks: &mut crate::marks::Marks, _centre: Vec3, _at: Vec3, _facing: f32) {}
    /// The body on the cursor before it is placed.
    fn ghost_body(guid: u64, known: &Self::Known) -> vale_client::world::entity::WorldEntity;
    /// Whether another mode of the tool takes this press.
    fn yields_press(_waypoints: Option<&crate::tools::spawns::waypoints::Waypoints>) -> bool {
        false
    }
    /// What `--spawn` or `--object` does beside selecting the spawn.
    fn opened(_args: &crate::args::Args, _spawn: &Self::Spawn, _waypoints: &mut crate::tools::spawns::waypoints::Waypoints) {}
    /// The subject's writes, for a selection held apart from its tool. See
    /// [`Standing`].
    fn writes() -> &'static dyn SpawnWrites;
}

/// The writes a [`Standing`] makes, through the subject it was read from.
pub(crate) trait SpawnWrites: std::fmt::Debug + Send + Sync {
    /// One row, in an undo label and a status line.
    fn noun(&self) -> &'static str;
    /// More than one.
    fn nouns(&self) -> &'static str;
    /// The spawn table, which names a group gesture.
    fn table(&self) -> &'static str;
    fn move_to(&self, session: &mut EditSession, guid: u64, to: Vec3, now: f64);
    fn face(&self, session: &mut EditSession, guid: u64, tilted: bool, orientation: f32, now: f64);
}

impl<S: SpawnSubject> SpawnWrites for S {
    fn noun(&self) -> &'static str {
        S::WORDS.noun
    }

    fn nouns(&self) -> &'static str {
        S::WORDS.nouns
    }

    fn table(&self) -> &'static str {
        S::SPAWN
    }

    fn move_to(&self, session: &mut EditSession, guid: u64, to: Vec3, now: f64) {
        move_one::<S>(session, guid, to, now);
    }

    fn face(&self, session: &mut EditSession, guid: u64, tilted: bool, orientation: f32, now: f64) {
        S::face(session, guid, tilted, orientation, now);
    }
}

/// Write one spawn's three position columns under one gesture subject, so a
/// move is one entry on the undo stack. Keyed on the guid rather than on the
/// column, as the form keys it.
pub(crate) fn move_one<S: SpawnSubject>(session: &mut EditSession, guid: u64, to: Vec3, now: f64) {
    let key = S::spawn_key(guid);
    let subject = format!("{} {guid} position", S::SPAWN);
    let label = format!("Move {}", S::WORDS.noun);
    for (column, value) in [("position_x", to.x), ("position_y", to.y), ("position_z", to.z)] {
        session.set_server_edit(
            S::SPAWN,
            &key,
            column,
            Some(vale_mangos::sql::float(value)),
            Some(crate::server::store::Gesture { label: &label, subject: &subject, now }),
        );
    }
}

/// A number out of a column of a row in the store.
pub(crate) fn stored_number(row: &RowEdit, column: &str) -> Option<f32> {
    row.columns.get(column)?.trim().parse::<f32>().ok()
}

/// What the template window is about: the template chosen in the picker while
/// the tool is in Place, or the selected spawn's template in Select.
///
/// Both name a template row by entry and patch. The picker's template may be
/// one this project creates, which is in no database and whose columns are
/// the store's own; a spawn's template is always a row the database holds.
/// See [`SpawnTool::template_subject`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TemplateSubject {
    /// The entry the project says the template has.
    pub(crate) entry: u32,
    /// The entry the database has it at, which differs while the project moves
    /// it and has not applied the move.
    pub(crate) read_entry: u32,
    pub(crate) patch: u32,
    /// What this project says of the row: [`Life::Insert`] for one it creates,
    /// [`Life::Update`] otherwise.
    pub(crate) claim: Life,
    /// The template's name and entry, for the window's title.
    pub(crate) label: String,
}

impl TemplateSubject {
    /// The key an edit to the row is written under. Both template tables are
    /// keyed by `entry` and `patch`.
    pub(crate) fn key(&self) -> Key {
        Key::two(("entry", u64::from(self.entry)), ("patch", u64::from(self.patch)))
    }

    /// Whether this project creates the template.
    pub(crate) fn is_new(&self) -> bool {
        self.claim == Life::Insert
    }
}

/// One template row, whole.
#[derive(Debug, Clone)]
pub(crate) struct TemplateRow {
    pub(crate) entry: u32,
    pub(crate) row: vale_mangos::schema::Row,
}

/// One spawn row, whole.
#[derive(Debug, Clone)]
pub(crate) struct SpawnRow {
    pub(crate) guid: u64,
    pub(crate) row: vale_mangos::schema::Row,
}

/// The picker behind Place, and the template a click on the ground will
/// place.
///
/// A spawn's first column is a template entry. The template is chosen by
/// name, and the search also matches the entry, because `vale spawns` and the
/// template window both print entries and a user who has one can type it.
///
/// The mode decides what a click does, as in the placement tools: in Place, a
/// chosen template stands under the pointer and the next click on the ground
/// creates the spawn; in Select, a click picks what is there. See
/// [`SpawnTool::mode`].
#[derive(Debug)]
pub(crate) struct NewSpawn<K> {
    /// What has been typed.
    pub(crate) search: String,
    /// Which type the matches are narrowed to, or `None` for every type. Only
    /// the game object picker sets it.
    pub(crate) of_type: Option<u32>,
    /// The search and type last sent, so a redraw does not re-query every
    /// frame.
    pub(crate) ran: Option<(String, Option<u32>)>,
    pub(crate) matches: Vec<K>,
    task: Option<Task<Result<Vec<K>, String>>>,
    /// The template a click will place. The ghost on the cursor and the
    /// preview pane both show it. `None` means nothing is chosen, which is the
    /// state `Escape` leaves.
    pub(crate) chosen: Option<K>,
    /// Why the search found nothing, when it is not that nothing matched.
    pub(crate) trouble: Option<String>,
    /// The `EditSession::database_writes` the matches were read at.
    searched_at: Option<u64>,
}

impl<K> Default for NewSpawn<K> {
    fn default() -> NewSpawn<K> {
        NewSpawn {
            search: String::new(),
            of_type: None,
            ran: None,
            matches: Vec::new(),
            task: None,
            chosen: None,
            trouble: None,
            searched_at: None,
        }
    }
}

impl<K> NewSpawn<K> {
    /// Whether a search is in flight, for the panel.
    pub(crate) fn searching(&self) -> bool {
        self.task.is_some()
    }

    /// Forget the matches, which a map change or an apply makes stale.
    pub(crate) fn forget(&mut self) {
        self.ran = None;
        self.matches.clear();
    }
}

/// How many entries the picker lists at once: fifty.
///
/// A `LIKE '%a%'` over ten thousand templates matches thousands. A longer list
/// is not scrolled through; the user narrows it by typing more letters into
/// the search box.
const MATCHES: usize = 50;

/// What a held left button is moving, once it has moved far enough to be a
/// drag.
///
/// The spawn's own position, across the surface under the pointer. By this
/// crate's rule a drag belongs to the object it began on: once the button is
/// down, the pointer leaving the marker still moves that spawn.
///
/// Armed only by a press on the already-selected spawn, and it moves nothing
/// until the pointer has travelled `DRAG_PIXELS`. See [`press`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Drag {
    pub(crate) guid: u64,
    /// Where the spawn was when the button went down, so the status line can
    /// say how far it moved.
    pub(crate) from: Vec3,
    /// Where the pointer was when the button went down.
    pub(crate) pressed_at: Vec2,
    /// How far the spawn is from the pointer, so a press near the edge of the
    /// marker does not snap it under the cursor on the first frame. `None`
    /// until the first frame the drag is actually moving.
    pub(crate) grab: Option<Vec3>,
    /// Whether the pointer has travelled far enough to be a drag rather than
    /// a click. While it is false the drag moves nothing.
    pub(crate) moving: bool,
    /// Whether the press landed on a spawn in a group of several. Released
    /// without moving, it selects that spawn alone; see
    /// [`crate::tools::placements::group`].
    pub(crate) collapse: bool,
}

/// The result of one read of the map: its spawns, the highest guid the whole
/// spawn table holds, and the highest entry the template table holds.
///
/// Neither maximum is specific to the map, since guids and entries are global.
/// They are read here because the read uses the same connection, and because
/// placing a spawn or creating a template, which need them, is only possible
/// once the map has been read.
#[derive(Debug)]
pub(crate) struct MapRead<P> {
    pub(crate) spawns: Vec<P>,
    pub(crate) highest: Option<u64>,
    pub(crate) highest_entry: Option<u32>,
}

/// The near spawns as they are drawn, built when what they depend on changes
/// rather than on every frame. See [`SpawnTool::lay_edits`].
///
/// Laying the store over every near spawn per frame built two keys and looked
/// both up in the store per spawn, which at a few thousand near spawns was
/// most of the frame. All but a few spawns are not edited, so the store is
/// walked once instead, and only the spawns it names are laid over.
#[derive(Debug)]
struct Overlay<P> {
    /// What it was built for: `ServerRows::revision` (`None` with no
    /// session) and [`SpawnTool::near`]'s generation.
    at: Option<(Option<u64>, u64)>,
    /// One per entry of [`SpawnTool::near`]: 0 for a spawn the project does
    /// not change, otherwise one past its place in [`Self::edited`].
    slots: Vec<u32>,
    /// One per entry of [`SpawnTool::near`]: whether the store holds the
    /// spawn's own row, which draws it in the edited colour.
    touched: Vec<bool>,
    /// The near spawns the project changes, with its edits over them.
    edited: Vec<P>,
}

impl<P> Default for Overlay<P> {
    fn default() -> Overlay<P> {
        Overlay { at: None, slots: Vec::new(), touched: Vec::new(), edited: Vec::new() }
    }
}

/// Which of the two spawn tables a spawn is a row of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SpawnKind {
    Creature,
    GameObject,
}

impl SpawnKind {
    /// The rail tool that edits a spawn of this kind.
    pub(crate) fn tool(self) -> Tool {
        match self {
            SpawnKind::Creature => Tool::Creatures,
            SpawnKind::GameObject => Tool::GameObjects,
        }
    }

    /// The workspace that edits this kind's templates, and what the top bar
    /// calls it.
    pub(crate) fn workspace(self) -> (Tool, &'static str) {
        match self {
            SpawnKind::Creature => (Tool::CreatureData, "Creatures"),
            SpawnKind::GameObject => (Tool::GameObjectData, "Game Objects"),
        }
    }

    /// The kind of a quest relation's holder, which is a template of one.
    pub(crate) fn of_holder(holder: crate::subjects::quests::Holder) -> SpawnKind {
        match holder {
            crate::subjects::quests::Holder::Creature => SpawnKind::Creature,
            crate::subjects::quests::Holder::Object => SpawnKind::GameObject,
        }
    }

    /// The other kind.
    pub(crate) fn other(self) -> SpawnKind {
        match self {
            SpawnKind::Creature => SpawnKind::GameObject,
            SpawnKind::GameObject => SpawnKind::Creature,
        }
    }

    /// What a list calls a spawn of this kind.
    pub(crate) fn word(self) -> &'static str {
        match self {
            SpawnKind::Creature => "creature",
            SpawnKind::GameObject => "game object",
        }
    }

    /// Where this kind's entry in a per-kind array is.
    fn index(self) -> usize {
        match self {
            SpawnKind::Creature => 0,
            SpawnKind::GameObject => 1,
        }
    }
}

/// One spawn named across both spawn tables. Creature guid 5000 and game
/// object guid 5000 are two different rows, so a list that names spawns of
/// both kinds names each by its kind and guid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SpawnRef {
    pub(crate) kind: SpawnKind,
    pub(crate) guid: u64,
}

/// A spawn a tool is asked to select and put the camera over: Go To on a list
/// that names spawns on any map. `map` is where the spawn stands, when it is
/// known: a spawn on another map opens that map first, with the camera over
/// the spawn, and is selected once the map's spawns are read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GoTo {
    pub(crate) guid: u64,
    pub(crate) map: Option<(u32, Vec3)>,
}

/// What a spawn's other rows say about it, for drawing it: whether the open
/// event filter hides it, and the marks it is drawn with. See
/// [`crate::subjects::game_events::Phase`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Tag {
    /// The event filter leaves it out of the world.
    pub(crate) hidden: bool,
    /// It exists only while an event is on.
    pub(crate) event_only: bool,
    /// It is in a pool, by its guid or by its entry.
    pub(crate) pooled: bool,
    /// It is in the pool the Pool window or workspace has open.
    pub(crate) in_open_pool: bool,
}

/// Whether each spawn tool draws the other tool's spawns while it is the one
/// chosen: dimmed markers that select the spawn under the other tool on a
/// click. One switch for both tools, set on either inspector.
#[derive(Resource, Debug, Default)]
pub(crate) struct OtherSpawns {
    pub(crate) shown: bool,
}

/// The spawn under the pointer for each kind this frame, with its distance
/// along the pointer's ray, so a press goes to the nearer of a creature and a
/// game object whichever tool is chosen. Written by each tool's
/// [`systems::aim`], read by the presses.
#[derive(Resource, Debug, Default)]
pub(crate) struct SpawnHover {
    under: [Option<(f32, u64)>; 2],
}

impl SpawnHover {
    pub(crate) fn set(&mut self, kind: SpawnKind, under: Option<(f32, u64)>) {
        self.under[kind.index()] = under;
    }

    /// Whether a spawn of `kind` is under the pointer and nearer than any of
    /// the other kind.
    pub(crate) fn nearest_is(&self, kind: SpawnKind) -> bool {
        match (self.under[kind.index()], self.under[kind.other().index()]) {
            (Some((mine, _)), Some((theirs, _))) => mine <= theirs,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// Whether anything of either kind is under the pointer.
    pub(crate) fn any(&self) -> bool {
        self.under.iter().any(Option::is_some)
    }
}

/// How a spawn tool's systems run this frame: its own tool is chosen; the
/// other spawn tool is chosen and draws this one's spawns; or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Own,
    Other,
    Idle,
}

impl Role {
    pub(crate) fn of<S: SpawnSubject>(chosen: Tool, others: &OtherSpawns, editing: bool) -> Role {
        match (editing, chosen == S::TOOL, others.shown && chosen == S::KIND.other().tool()) {
            (false, _, _) => Role::Idle,
            (true, true, _) => Role::Own,
            (true, false, true) => Role::Other,
            (true, false, false) => Role::Idle,
        }
    }

    /// Whether the spawns are read, laid out and drawn.
    pub(crate) fn draws(self) -> bool {
        self != Role::Idle
    }
}

/// What the pointer is doing with one subject's spawns on the open map.
#[derive(Resource, Debug)]
pub(crate) struct SpawnTool<S: SpawnSubject> {
    /// Every spawn on the open map, as the database has it.
    pub(crate) spawns: Vec<S::Spawn>,
    /// Where each of [`Self::spawns`] is, by guid, built with each read. See
    /// [`SpawnTool::read_one`].
    spawn_index: HashMap<u64, usize>,
    /// The template moves [`Self::spawns`] were last folded through, so a
    /// store change that moves no template does not walk the whole map. `None`
    /// after a read, whose rows have not been folded.
    folded_for: Option<Vec<(u32, u32, u32)>>,
    /// The spawns this project creates. They are in no database and are
    /// rebuilt from the store whenever it changes.
    ///
    /// A second list rather than rows appended to [`Self::spawns`], because
    /// that list is what a read returned and the next read replaces it whole.
    /// Everything that draws, picks or counts walks both; see
    /// [`SpawnTool::base_spawn`].
    pub(crate) created: Vec<S::Spawn>,
    /// Which `ServerRows::revision` [`Self::created`] and
    /// [`Self::templates`] were built for.
    created_for: Option<u64>,
    /// The templates this project creates, rebuilt from the store whenever it
    /// changes, sorted by entry. They are in no database, so no query returns
    /// them; the picker lists them above its search, and a spawn of one is
    /// labelled and drawn from here through [`Self::known`].
    pub(crate) templates: Vec<S::Known>,
    /// The highest entry the template table holds, at any patch, read with
    /// the map. One of the inputs to a new template's entry; see
    /// [`SpawnTool::next_entry`]. `None` until the read completes.
    pub(crate) max_entry: Option<u32>,
    /// Whether the template flags have been acted on, or were not asked for.
    /// The scripted apply waits on it beside [`Self::scripted_done`].
    pub(crate) scripted_template_done: bool,
    /// The highest guid the spawn table holds, read with the map.
    ///
    /// One of the inputs to a new spawn's guid; see
    /// [`SpawnTool::next_guid`]. `None` until the read completes, so placing
    /// waits for it.
    pub(crate) max_guid: Option<u64>,
    /// What an entry is, for the spawns of it this project creates.
    ///
    /// Filled from the map query, which already carries these columns for
    /// every spawn on the map; from the picker's matches; and, for an entry
    /// neither knows, from a read of its own.
    pub(crate) known: Fresh<HashMap<u32, S::Known>>,
    /// The read of [`Self::known`]'s own, while it is running.
    known_task: Option<Task<Result<Vec<S::Known>, String>>>,
    /// The display id picker, while it is open on one of the template's
    /// display columns. See `crate::subjects::displays`.
    pub(crate) display_pick: Option<crate::subjects::displays::DisplayPick>,
    /// Which half of the panel is showing, which also decides what a click in
    /// the world does.
    ///
    /// This is `place::Mode`, the placement tools' own two values, rather than
    /// a third enumeration with the same meaning: choosing what to put down and
    /// editing what is there are one subject, and the user switches between
    /// them often. See `crate::tools::placements::place::Mode`.
    pub(crate) mode: crate::tools::placements::place::Mode,
    /// The picker shown in the Place half, and what a click will put down.
    pub(crate) new_spawn: NewSpawn<S::Known>,
    /// The direction the template on the cursor faces, in radians, which is
    /// what a click writes as `orientation`. `None` while nothing is on the
    /// cursor. Set toward the camera when the placer is armed, kept from one
    /// placement to the next, and turned by [`Self::turn_ghost`].
    pub(crate) ghost_facing: Option<f32>,
    /// The read, while it is running. `None` when there is nothing in flight.
    task: Option<Task<Result<MapRead<S::Spawn>, String>>>,
    /// What [`Self::spawns`] was read for: the map, and what the last apply
    /// put in the database.
    ///
    /// Keyed on `EditSession::database_writes`, which counts every apply and
    /// every Put back, and not on the project's edit counter, which changes
    /// on every keystroke; a map query per typed character is 24,610 rows per
    /// keystroke. A row this project has edited and not yet applied is
    /// therefore drawn from the store over the database's reading.
    loaded: Option<(u32, u64)>,
    /// Why there is nothing, when there is nothing: no database, a refused
    /// connection, a query that failed.
    pub(crate) trouble: Option<String>,
    /// Indices over both lists that are near enough to draw, nearest first.
    /// Rebuilt when the focus crosses a tile, not per frame.
    pub(crate) near: Vec<usize>,
    /// The tile [`Self::near`] was built around.
    built_at: Option<(u32, u32)>,
    /// How many times [`Self::near`] has been built, which the overlay is
    /// keyed on.
    near_generation: u64,
    /// Each near spawn's place in [`Self::near`], by guid, built with it.
    near_rank: HashMap<u64, usize>,
    /// The near spawns as they are drawn.
    overlay: Overlay<S::Spawn>,
    /// The spawn under the pointer, by guid.
    pub(crate) hovered: Option<u64>,
    /// The spawn whose form is open, by guid: the primary of the selection.
    pub(crate) selected: Option<u64>,
    /// The rest of the selection, by guid — see
    /// [`crate::tools::placements::group`]. The form is the primary's; a move,
    /// a turn or a removal acts on every one.
    ///
    /// A guid here whose spawn is no longer on the map (a created spawn whose
    /// creation was undone) is skipped by everything that reads the group, as
    /// [`SpawnTool::edited`] finds nothing for it. It is not pruned every
    /// frame, because pruning is a walk of every spawn on the map.
    pub(crate) also: Vec<u64>,
    /// Where each member stood when a drag's button went down, so each moves
    /// by the drag's step from where it began, as the primary does.
    group_from: Vec<(u64, Vec3)>,
    /// The selected spawn's whole template row, read on demand.
    pub(crate) template: Option<TemplateRow>,
    /// The template read, while it is running.
    template_task: Option<Task<Result<Option<TemplateRow>, String>>>,
    /// What [`Self::template`] was made at: `EditSession::database_writes`,
    /// and whether the project created the row, in which case it is an empty
    /// row holding the key. Either changing makes it be made again: an apply
    /// or a put back changes what the database holds, and a discard turns a
    /// created row into one the database may hold.
    template_at: Option<(u64, bool)>,
    /// The selected spawn's whole spawn row. The map query reads only the
    /// columns the drawing needs.
    pub(crate) spawn_row: Option<SpawnRow>,
    spawn_task: Option<Task<Result<Option<SpawnRow>, String>>>,
    /// …and the same for [`Self::spawn_row`].
    spawn_at: Option<(u64, bool)>,
    /// Draw the near ones as models, or as markers only.
    pub(crate) show_models: bool,
    /// Mark every near spawn, or only the selected, hovered and grouped ones.
    /// Off is for a place with so many spawns near the camera that their
    /// markers slow the frame. Picking does not depend on it: the tool picks
    /// by arithmetic on the pointer's ray, so an unmarked spawn can still be
    /// clicked.
    pub(crate) show_markers: bool,
    /// How many models at once.
    pub(crate) model_budget: usize,
    /// How far out a spawn is drawn at all, in yards.
    pub(crate) range: f32,
    /// Whether the template window is open.
    pub(crate) template_window: bool,
    /// Where a drag is moving the selected spawn to, and where it began.
    pub(crate) drag: Option<Drag>,
    /// Whether the scripted placements and removals have finished.
    ///
    /// The scripted apply runs on the first frame there is a session and the
    /// scripted placement several seconds later, once the map read, the
    /// search and the tile under the camera have all completed. Without this
    /// flag the apply ran against an empty store and reported "nothing to
    /// apply", and the edit arrived afterwards.
    pub(crate) scripted_done: bool,
    /// Every template row, for the subject's workspace. See
    /// [`crate::subjects::templates`].
    pub(crate) listed: Listing<S::Known>,
    /// What only this subject's tool keeps.
    pub(crate) own: S::Own,
    /// A spawn to select and frame once its map is read; see [`GoTo`].
    pub(crate) go_to: Option<GoTo>,
    /// Each near spawn's [`Tag`], by guid, and what it was worked out for:
    /// [`Self::near`]'s generation, the store's revision and the event
    /// filter, pool and event reads. See [`systems::tag_the_near`].
    tags: HashMap<u64, Tag>,
    tags_for: Option<TagsFor>,
    /// The task working out the tags for a newer key, while [`Self::tags`]
    /// are still drawn.
    tagging: Option<(TagsFor, Task<HashMap<u64, Tag>>)>,
}

/// What a spawn tool's tags were worked out for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TagsFor {
    pub(crate) near: u64,
    pub(crate) revision: Option<u64>,
    pub(crate) phase: crate::subjects::game_events::Phase,
    pub(crate) events: u64,
    pub(crate) pools: u64,
    pub(crate) open_pool: Option<u32>,
}

impl<S: SpawnSubject> Default for SpawnTool<S> {
    fn default() -> SpawnTool<S> {
        SpawnTool {
            spawns: Vec::new(),
            spawn_index: HashMap::new(),
            folded_for: None,
            created: Vec::new(),
            created_for: None,
            templates: Vec::new(),
            max_entry: None,
            scripted_template_done: false,
            max_guid: None,
            known: Fresh::default(),
            known_task: None,
            display_pick: None,
            mode: crate::tools::placements::place::Mode::default(),
            new_spawn: NewSpawn::default(),
            ghost_facing: None,
            task: None,
            loaded: None,
            trouble: None,
            near: Vec::new(),
            built_at: None,
            near_generation: 0,
            near_rank: HashMap::new(),
            overlay: Overlay::default(),
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
            model_budget: S::MODEL_BUDGET,
            range: S::RANGE,
            template_window: false,
            drag: None,
            scripted_done: false,
            listed: Listing::default(),
            own: S::Own::default(),
            go_to: None,
            tags: HashMap::new(),
            tags_for: None,
            tagging: None,
        }
    }
}

/// How far from the original a duplicate is placed, in yards: two.
///
/// Far enough that the two are separate marks at editing distance, and near
/// enough that the copy is clearly next to its original.
const APART: f32 = 2.0;

impl<S: SpawnSubject> SpawnTool<S> {
    /// How many spawns there are to draw: the database's and this project's
    /// own.
    pub(crate) fn spawn_count(&self) -> usize {
        self.spawns.len() + self.created.len()
    }

    /// One spawn, by an index over both lists, the database's first.
    ///
    /// Everything that walks the spawns goes through this rather than through
    /// [`Self::spawns`], which holds only what the last read returned. This
    /// keeps a created spawn drawable, pickable and counted on the same terms
    /// as a read one.
    pub(crate) fn base_spawn(&self, index: usize) -> Option<&S::Spawn> {
        match self.spawns.get(index) {
            Some(spawn) => Some(spawn),
            None => self.created.get(index - self.spawns.len()),
        }
    }

    /// The chosen spawn, if it is still on this map.
    pub(crate) fn chosen(&self) -> Option<&S::Spawn> {
        self.find(self.selected?)
    }

    /// Any spawn on this map by guid, as read or as created.
    fn find(&self, guid: u64) -> Option<&S::Spawn> {
        self.read_one(guid).or_else(|| self.created.iter().find(|spawn| spawn.guid() == guid))
    }

    /// One of [`Self::spawns`] by guid, through [`Self::spawn_index`]. A list
    /// the index was not built for, which only a test makes, is walked.
    fn read_one(&self, guid: u64) -> Option<&S::Spawn> {
        if self.spawn_index.len() == self.spawns.len() {
            match self.spawn_index.get(&guid) {
                None => return None,
                Some(&at) => {
                    if let Some(spawn) = self.spawns.get(at).filter(|spawn| spawn.guid() == guid) {
                        return Some(spawn);
                    }
                }
            }
        }
        self.spawns.iter().find(|spawn| spawn.guid() == guid)
    }

    /// Take a read of the map: its spawns, indexed by guid. The near list and
    /// the overlay are built again from it.
    pub(crate) fn take_read(&mut self, spawns: Vec<S::Spawn>) {
        self.spawn_index = spawns.iter().enumerate().map(|(at, spawn)| (spawn.guid(), at)).collect();
        self.spawns = spawns;
        self.folded_for = None;
        self.built_at = None;
        self.overlay.at = None;
    }

    /// Build [`Self::near`] around `eye`: every spawn of both lists within
    /// [`Self::range`], nearest first.
    pub(crate) fn build_near(&mut self, eye: Vec3) {
        let range = self.range;
        let mut near: Vec<(i64, usize)> = (0..self.spawn_count())
            .filter_map(|index| {
                let away = self.base_spawn(index)?.at().distance(eye);
                (away <= range).then_some((away as i64, index))
            })
            .collect();
        near.sort_unstable();
        self.near = near.into_iter().map(|(_, index)| index).collect();
        self.near_rank = self
            .near
            .iter()
            .enumerate()
            .filter_map(|(rank, index)| Some((self.base_spawn(*index)?.guid(), rank)))
            .collect();
        self.near_generation += 1;
    }

    /// Lay the project's edits over the near spawns, when the store or the
    /// near list has changed since they were last laid. What
    /// [`Self::near_drawn`] and [`Self::near_touched`] answer from.
    ///
    /// The store is walked once for the rows of this subject's two tables. A
    /// near spawn whose own row or whose template's row is among them gets
    /// [`Spawned::with_edits`], which is the same test the drawing made per
    /// spawn, made here for those spawns alone.
    pub(crate) fn lay_edits(&mut self, session: Option<&EditSession>) {
        let at = (session.map(|session| session.rows.revision), self.near_generation);
        if self.overlay.at == Some(at) {
            return;
        }
        self.overlay.at = Some(at);
        let count = self.near.len();
        self.overlay.slots.clear();
        self.overlay.slots.resize(count, 0);
        self.overlay.touched.clear();
        self.overlay.touched.resize(count, false);
        self.overlay.edited.clear();
        let Some(edits) = session.map(|session| &session.rows.edits) else {
            return;
        };
        // The store's keys are compared by `Key`'s own order, which is the
        // store's, so a row counts here exactly when a lookup would find it.
        let mut ranks: Vec<usize> = Vec::new();
        let mut templates: Vec<(u32, u32)> = Vec::new();
        for (table, key, _) in edits.rows() {
            if table == S::SPAWN {
                let Some(guid) = key.first() else { continue };
                if key.cmp(&S::spawn_key(guid)).is_ne() {
                    continue;
                }
                if let Some(&rank) = self.near_rank.get(&guid) {
                    self.overlay.touched[rank] = true;
                    ranks.push(rank);
                }
            } else if table == S::TEMPLATE {
                let (Some(entry), Some(patch)) = (key.first(), key.patch()) else { continue };
                let entry = entry as u32;
                if key.cmp(&S::template_key(entry, patch)).is_eq() {
                    templates.push((entry, patch));
                }
            } else if S::SPAWN_ROWS.contains(&table) {
                let Some(guid) = key.first() else { continue };
                if let Some(&rank) = self.near_rank.get(&guid) {
                    ranks.push(rank);
                }
            }
        }
        if !templates.is_empty() {
            for (rank, index) in self.near.iter().enumerate() {
                let Some(spawn) = self.base_spawn(*index) else { continue };
                if templates.contains(&(spawn.entry(), spawn.patch())) {
                    ranks.push(rank);
                }
            }
        }
        ranks.sort_unstable();
        ranks.dedup();
        for rank in ranks {
            let Some(base) = self.base_spawn(self.near[rank]) else { continue };
            if let Some(spawn) = base.with_edits(edits) {
                self.overlay.edited.push(spawn);
                self.overlay.slots[rank] = self.overlay.edited.len() as u32;
            }
        }
    }

    /// The near spawn at `rank` in [`Self::near`] as it is drawn: with the
    /// project's edits over it where it changes it. Borrowed, as nothing is
    /// built here; see [`Self::lay_edits`].
    ///
    /// `None` for a spawn the event filter hides, so nothing draws, picks or
    /// encloses it.
    pub(crate) fn near_drawn(&self, rank: usize) -> Option<&S::Spawn> {
        let base = self.base_spawn(*self.near.get(rank)?)?;
        if self.tags.get(&base.guid()).is_some_and(|tag| tag.hidden) {
            return None;
        }
        let edited = match self.overlay.slots.get(rank) {
            Some(&slot) if slot > 0 => self.overlay.edited.get(slot as usize - 1),
            _ => None,
        };
        Some(edited.filter(|edited| edited.guid() == base.guid()).unwrap_or(base))
    }

    /// [`Self::near_drawn`] by the spawn's guid, for a pass over the preview
    /// bodies, which know their spawn by guid.
    pub(crate) fn near_drawn_by_guid(&self, guid: u64) -> Option<&S::Spawn> {
        let rank = *self.near_rank.get(&guid)?;
        self.near_drawn(rank).filter(|spawn| spawn.guid() == guid)
    }

    /// A near spawn's [`Tag`], or the empty tag.
    pub(crate) fn tag(&self, guid: u64) -> Tag {
        self.tags.get(&guid).copied().unwrap_or_default()
    }

    /// Take the tags a finished task worked out. Tags for a key that has
    /// changed since are still taken: they are nearer the truth than the ones
    /// they replace, and the next call to [`Self::retag`] starts a task for
    /// the new key.
    pub(crate) fn land_tags(&mut self) {
        let Some((_, task)) = self.tagging.as_mut() else { return };
        let Some(tags) = bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(task)) else { return };
        if let Some((key, _)) = self.tagging.take() {
            self.tags = tags;
            self.tags_for = Some(key);
        }
    }

    /// Whether a task should start for `key`: the tags are not for it and no
    /// task is running.
    pub(crate) fn wants_tags(&self, key: &TagsFor) -> bool {
        self.tags_for.as_ref() != Some(key) && self.tagging.is_none()
    }

    /// Start a task that works out each near spawn's tag for `key`, unless
    /// [`Self::wants_tags`] says no. `tag` runs on the task, once per near
    /// spawn, with the spawn's guid and the entry the database has it at.
    pub(crate) fn retag(&mut self, key: TagsFor, tag: impl Fn(u64, u32) -> Tag + Send + 'static) {
        if !self.wants_tags(&key) {
            return;
        }
        let near: Vec<(u64, u32)> = self
            .near
            .iter()
            .filter_map(|at| self.base_spawn(*at))
            .map(|spawn| (spawn.guid(), spawn.entry()))
            .collect();
        let pool = bevy::tasks::AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::default);
        let task = pool.spawn(async move { near.into_iter().map(|(guid, entry)| (guid, tag(guid, entry))).collect() });
        self.tagging = Some((key, task));
    }

    /// The generation [`Self::near`] was built at.
    pub(crate) fn near_generation(&self) -> u64 {
        self.near_generation
    }

    /// Whether the open map's spawns have been read for `map` at `writes`,
    /// with no read in flight.
    pub(crate) fn read_for(&self, map: u32, writes: u64) -> bool {
        self.loaded == Some((map, writes)) && self.task.is_none()
    }

    /// Whether the store holds the own row of the near spawn at `rank`.
    pub(crate) fn near_touched(&self, rank: usize) -> bool {
        self.overlay.touched.get(rank).copied().unwrap_or(false)
    }

    /// One spawn as it should be drawn: the database's reading with the
    /// project's edits over it. See [`Spawned::with_edits`].
    ///
    /// Returns an owned spawn rather than a reference, because an edited one
    /// is built here and there is nothing to borrow it from.
    pub(crate) fn spawn_at(&self, index: usize, edits: Option<&Edits>) -> Option<S::Spawn> {
        let base = self.base_spawn(index)?;
        Some(edits.and_then(|edits| base.with_edits(edits)).unwrap_or_else(|| base.clone()))
    }

    /// The chosen spawn, with the project's edits over it.
    pub(crate) fn chosen_edited(&self, edits: Option<&Edits>) -> Option<S::Spawn> {
        let base = self.chosen()?;
        Some(edits.and_then(|edits| base.with_edits(edits)).unwrap_or_else(|| base.clone()))
    }

    /// The spawn under the pointer, with the project's edits over it.
    pub(crate) fn hovered_edited(&self, edits: Option<&Edits>) -> Option<S::Spawn> {
        self.edited(self.hovered?, edits)
    }

    /// Any spawn on this map by guid, with the project's edits over it.
    pub(crate) fn edited(&self, guid: u64, edits: Option<&Edits>) -> Option<S::Spawn> {
        let base = self.find(guid)?;
        Some(edits.and_then(|edits| base.with_edits(edits)).unwrap_or_else(|| base.clone()))
    }

    /// The chosen spawn as a [`Standing`], as the project's edits leave it.
    pub(crate) fn standing(&self, edits: Option<&Edits>) -> Option<Standing> {
        self.chosen_edited(edits).map(|spawn| Standing::of::<S>(&spawn))
    }

    /// The rest of the selection as [`Standing`]s. A member no longer on the
    /// map is left out.
    pub(crate) fn member_standings(&self, edits: Option<&Edits>) -> Vec<Standing> {
        self.also
            .iter()
            .filter_map(|&guid| self.edited(guid, edits))
            .map(|spawn| Standing::of::<S>(&spawn))
            .collect()
    }

    /// Whether a spawn is selected, as the primary or as a member.
    pub(crate) fn holds(&self, guid: u64) -> bool {
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
    pub(crate) fn select_only(&mut self, guid: Option<u64>) {
        self.make_primary(guid);
        self.also.clear();
    }

    /// Shift+click: take a spawn out of the group, or add one and make it the
    /// primary. See [`crate::tools::placements::group`].
    pub(crate) fn toggle(&mut self, guid: u64) {
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
    pub(crate) fn promote(&mut self, guid: u64) {
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
    /// The highest of three: the reserved base, one past what the spawn table
    /// holds, and one past the highest this project has already claimed. The
    /// second is needed because the reserved base is only a floor: once a
    /// project has put a spawn at ten million, the table's own maximum is ten
    /// million and the next one has to go above it. The third is needed
    /// because several spawns can be placed between two reads of the map.
    ///
    /// See `vale_mangos::creature::RESERVED_GUID_BASE` for where the number
    /// comes from and what it costs.
    pub(crate) fn next_guid(&self, edits: &Edits) -> u64 {
        let in_database = self.max_guid.map(|highest| highest + 1).unwrap_or(0);
        let claimed = edits
            .rows()
            .filter(|(table, _, row)| *table == S::SPAWN && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .map(|highest| highest + 1)
            .unwrap_or(0);
        S::RESERVED_GUID_BASE.max(in_database).max(claimed)
    }

    /// Create a new spawn of `known` on the ground at `at`.
    ///
    /// Every column of the row is written at once, as one claim and one undo
    /// entry; the values come from [`SpawnSubject::new_spawn`]. The guid is
    /// [`Self::next_guid`]'s and is returned, because the caller selects what
    /// it just placed.
    pub(crate) fn create(&mut self, session: &mut EditSession, known: &S::Known, at: Vec3, orientation: f32, now: f64) -> u64 {
        let guid = self.next_guid(&session.rows.edits);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in S::new_spawn(known.entry(), session.map_id, (at.x, at.y, at.z), orientation) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        let subject = format!("{} {guid}", S::SPAWN);
        let label = format!("New {}", S::WORDS.spawn);
        session.set_server_row(
            S::SPAWN,
            &S::spawn_key(guid),
            Some(&row),
            Some(crate::server::store::Gesture { label: &label, subject: &subject, now }),
        );
        // So a spawn of it is labelled and drawn before the map is read
        // again. `next_guid` reads the store, which now holds the guid.
        self.known.entry(known.entry()).or_insert_with(|| known.clone());
        self.select_only(Some(guid));
        guid
    }

    /// Create another spawn of the same template beside the selected one.
    ///
    /// A second guard on the other side of a gate is the first one's row with
    /// a new guid. Every column is copied: the database's value where the
    /// project has not changed it and the project's where it has, so the copy
    /// matches what is drawn, not what the table holds.
    ///
    /// The copy is offset by [`APART`] yards rather than placed at the same
    /// point, because two spawns at one point look like one and the copy
    /// could not be clicked.
    ///
    /// `None` when the whole row has not been read yet, which is the first
    /// frame or two after a spawn is clicked. Copying from a partly read row
    /// would take the table's defaults for the columns not yet read.
    pub(crate) fn duplicate(&mut self, session: &mut EditSession, now: f64) -> Option<u64> {
        let spawn = self.chosen_edited(Some(&session.rows.edits))?;
        let from = self.spawn_row.as_ref().filter(|held| held.guid == spawn.guid())?;
        let key = S::spawn_key(spawn.guid());
        let guid = self.next_guid(&session.rows.edits);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        // The defaults first, so a column that neither the row nor the project
        // supplies gets a value this crate chose rather than the table's.
        for change in S::new_spawn(spawn.entry(), session.map_id, spawn.at().into(), spawn.orientation()) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        for column in S::SPAWN_TABLE.columns {
            if !column.editable() {
                continue;
            }
            let value = session
                .rows
                .edits
                .get(S::SPAWN, &key, column.name)
                .map(str::to_string)
                .or_else(|| column.literal(&from.row));
            if let Some(value) = value {
                row.columns.insert(column.name.to_string(), value);
            }
        }
        // Then the position: beside the original, and on this map whatever
        // map the copied row names.
        row.columns.insert("map".to_string(), session.map_id.to_string());
        row.columns.insert("position_x".to_string(), vale_mangos::sql::float(spawn.at().x + APART));
        let subject = format!("{} {guid}", S::SPAWN);
        let label = format!("Duplicate {}", S::WORDS.spawn);
        session.set_server_row(
            S::SPAWN,
            &S::spawn_key(guid),
            Some(&row),
            Some(crate::server::store::Gesture { label: &label, subject: &subject, now }),
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
    pub(crate) fn remove(session: &mut EditSession, guid: u64, now: f64) {
        let key = S::spawn_key(guid);
        let held = session.rows.edits.row(S::SPAWN, &key).cloned();
        let subject = format!("{} {guid} removal", S::SPAWN);
        let label = format!("Remove {}", S::WORDS.spawn);
        let wanted = match held {
            Some(row) if row.life == Life::Insert => None,
            held => {
                let mut row = held.unwrap_or_default();
                row.life = Life::Delete;
                Some(row)
            }
        };
        session.set_server_row(
            S::SPAWN,
            &key,
            wanted.as_ref(),
            Some(crate::server::store::Gesture { label: &label, subject: &subject, now }),
        );
    }

    /// Cancel a removal. The column edits the spawn had are kept.
    pub(crate) fn keep(session: &mut EditSession, guid: u64, now: f64) {
        let key = S::spawn_key(guid);
        let mut row = session.rows.edits.row(S::SPAWN, &key).cloned().unwrap_or_default();
        row.life = Life::Update;
        let subject = format!("{} {guid} removal", S::SPAWN);
        let label = format!("Cancel {} removal", S::WORDS.spawn);
        let wanted = (!row.is_empty()).then_some(row);
        session.set_server_row(
            S::SPAWN,
            &key,
            wanted.as_ref(),
            Some(crate::server::store::Gesture { label: &label, subject: &subject, now }),
        );
    }

    /// Ask for the map to be read again on the next frame.
    ///
    /// The two whole rows the form draws go with it, so after an apply or a
    /// revert the form and the markers show the same reading, and the
    /// picker's matches, which an apply can change.
    pub(crate) fn forget(&mut self) {
        self.loaded = None;
        self.built_at = None;
        self.template = None;
        self.spawn_row = None;
        self.new_spawn.forget();
    }

    /// The entry a new template gets.
    ///
    /// The highest of three: the reserved base, one past what the table holds
    /// at any patch, and one past the highest this project has already
    /// claimed. See [`Numbering::next_entry`].
    pub(crate) fn next_entry(&self, edits: &Edits) -> u32 {
        S::NUMBERING.next_entry(self.max_entry, edits)
    }

    /// Make a new template, choose it in the picker and open the template
    /// window on it.
    ///
    /// Every column is written at once, as one claim and one undo entry; the
    /// values come from [`SpawnSubject::new_template`]. The patch is the
    /// server's own rather than 0: a row at patch 0 would be beaten by any
    /// later content-patch row of the same entry.
    ///
    /// The tool is put into Place with the new template chosen, and the
    /// template window is opened on it: a new template is named and given a
    /// model in the window, then placed in the world with a click.
    pub(crate) fn create_template(&mut self, session: &mut EditSession, name: &str, patch: u32, now: f64) -> u32 {
        let (entry, known) = self.write_new_template(session, name, patch, now);
        if let Some(known) = known {
            self.open_template(known);
        }
        entry
    }

    /// Make a new template and open it in the subject's workspace. The tool's
    /// picker and window are left as they are.
    pub(crate) fn create_listed_template(&mut self, session: &mut EditSession, name: &str, patch: u32, now: f64) -> u32 {
        let (entry, _) = self.write_new_template(session, name, patch, now);
        self.open_listed(entry);
        entry
    }

    /// The claim a new template is: every column at once, as one undo entry.
    /// Answers its entry and the row as a list shows it.
    fn write_new_template(
        &mut self,
        session: &mut EditSession,
        name: &str,
        patch: u32,
        now: f64,
    ) -> (u32, Option<S::Known>) {
        let entry = self.next_entry(&session.rows.edits);
        let key = S::template_key(entry, patch);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in S::new_template(name) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        let subject = format!("{} {entry}", S::TEMPLATE);
        let label = format!("New {}", S::WORDS.noun);
        session.set_server_row(
            S::TEMPLATE,
            &key,
            Some(&row),
            Some(crate::server::store::Gesture { label: &label, subject: &subject, now }),
        );
        (entry, S::Known::from_claim(&key, &row))
    }

    /// A copy of the template the window shows, under a new entry.
    ///
    /// Every column comes across: the database's value where the project has
    /// not changed it and the project's where it has, so the copy is what is
    /// shown rather than what the table holds. The name gets " (copy)" so the
    /// two can be told apart in a list.
    ///
    /// `None` when the whole row has not been read yet, which is the frame or
    /// two after a template is chosen. Copying from a half-read row would take
    /// the table's defaults for the columns that had not arrived.
    pub(crate) fn copy_template(
        &mut self,
        session: &mut EditSession,
        subject: &TemplateSubject,
        patch: u32,
        now: f64,
    ) -> Option<u32> {
        let (entry, known) = self.write_copy(session, subject, patch, now)?;
        if let Some(known) = known {
            self.open_template(known);
        }
        Some(entry)
    }

    /// A copy of the open row in the subject's workspace, opened there. `None`
    /// on the terms of [`Self::copy_template`].
    pub(crate) fn copy_listed_template(
        &mut self,
        session: &mut EditSession,
        subject: &TemplateSubject,
        patch: u32,
        now: f64,
    ) -> Option<u32> {
        let (entry, _) = self.write_copy(session, subject, patch, now)?;
        self.open_listed(entry);
        Some(entry)
    }

    /// The claim a copy is, as one undo entry. Answers its entry and the row
    /// as a list shows it.
    fn write_copy(
        &mut self,
        session: &mut EditSession,
        subject: &TemplateSubject,
        patch: u32,
        now: f64,
    ) -> Option<(u32, Option<S::Known>)> {
        let from = self.template.as_ref().filter(|held| held.entry == subject.entry)?;
        let source = subject.key();
        let entry = self.next_entry(&session.rows.edits);
        let key = S::template_key(entry, patch);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for column in S::TEMPLATE_TABLE.columns.iter().filter(|column| column.editable()) {
            let value = session
                .rows
                .edits
                .get(S::TEMPLATE, &source, column.name)
                .map(str::to_string)
                .or_else(|| column.literal(&from.row));
            if let Some(value) = value {
                row.columns.insert(column.name.to_string(), value);
            }
        }
        let named = row.columns.get("name").map(|literal| vale_mangos::sql::unquote(literal)).unwrap_or_default();
        row.columns.insert("name".to_string(), vale_mangos::sql::text(&format!("{named} (copy)")));
        let gesture = format!("{} {entry}", S::TEMPLATE);
        let label = format!("Copy {}", S::WORDS.noun);
        session.set_server_row(
            S::TEMPLATE,
            &key,
            Some(&row),
            Some(crate::server::store::Gesture { label: &label, subject: &gesture, now }),
        );
        Some((entry, S::Known::from_claim(&key, &row)))
    }

    /// Open a template in the subject's workspace's form.
    pub(crate) fn open_listed(&mut self, entry: u32) {
        self.listed.open = Some(entry);
        self.template = None;
        self.created_for = None;
    }

    /// Choose a template in the picker and open the window on it.
    fn open_template(&mut self, known: S::Known) {
        self.mode = crate::tools::placements::place::Mode::Place;
        self.new_spawn.chosen = Some(known);
        self.template_window = true;
        self.template = None;
        self.created_for = None;
    }

    /// Give up a template this project created. It is in no database, so
    /// giving it up removes nothing there. A spawn of it this project created
    /// keeps its entry and is a spawn the server refuses to load until the
    /// template exists; the panel says so through the spawn's label.
    pub(crate) fn discard_template(&mut self, session: &mut EditSession, entry: u32, now: f64) {
        let Some(known) = self.templates.iter().find(|known| known.entry() == entry).cloned() else {
            return;
        };
        let subject = format!("{} {entry}", S::TEMPLATE);
        let label = format!("Discard new {}", S::WORDS.noun);
        session.set_server_row(
            S::TEMPLATE,
            &known.key(),
            None,
            Some(crate::server::store::Gesture { label: &label, subject: &subject, now }),
        );
        if self.new_spawn.chosen.as_ref().is_some_and(|chosen| chosen.entry() == entry) {
            self.new_spawn.chosen = None;
        }
        if self.listed.open == Some(entry) {
            self.listed.open = None;
        }
        self.template = None;
        self.created_for = None;
    }

    /// Whether an entry is already a template, as far as this tool can see:
    /// the templates behind the map's spawns, the picker's matches, the
    /// templates read for created spawns, the ones this project creates, and
    /// the whole table once the workspace has read it. Asked by the entry the
    /// project gives each row and by the entry the database has it at,
    /// because the database is where an apply runs.
    ///
    /// An entry with no spawn on this map and never searched for is not seen
    /// here and is caught at the apply, which asks the database itself.
    pub(crate) fn entry_taken(&self, entry: u32) -> bool {
        self.spawns.iter().any(|spawn| spawn.entry() == entry || spawn.read_entry() == entry)
            || self
                .known
                .values()
                .chain(self.new_spawn.matches.iter())
                .chain(self.templates.iter())
                .any(|known| known.entry() == entry || known.read_entry() == entry)
            || self.listed.holds(entry)
    }

    /// Move a template to another entry.
    ///
    /// Whether it may happen is [`Self::plan_template_move`]; this performs
    /// it, and it is one operation whatever the row is: the project's claim on
    /// the row is re-keyed, so the template is one row before and after, and
    /// every later edit lands on the same claim. A row the database holds
    /// records where the database has it, which the `UPDATE` at the apply
    /// names.
    ///
    /// The project's own rows that name the entry follow it here, and the
    /// database's rows follow at the apply. See `crate::server::follow` and
    /// the template table's references.
    pub(crate) fn rekey_template(
        &mut self,
        session: &mut EditSession,
        subject: &TemplateSubject,
        to: u32,
        now: f64,
    ) -> Result<(), String> {
        let Some((from, to_key)) = Self::plan_template_move(subject, to, self.entry_taken(to))? else {
            return Ok(());
        };
        let at_base = S::template_key(subject.read_entry, subject.patch);
        let gesture_subject = format!("{} {} entry", S::TEMPLATE, subject.entry);
        let label = format!("Move {} entry", S::WORDS.noun);
        let gesture = crate::server::store::Gesture { label: &label, subject: &gesture_subject, now };
        crate::server::follow::rekey(
            session,
            S::TEMPLATE,
            &from,
            &to_key,
            Some(&at_base),
            S::TEMPLATE_TABLE.references,
            gesture,
        )?;
        // The picker's chosen template follows the row, so the window and the
        // ghost stay on it. A created template has no database row, so its
        // read entry moves with it.
        if let Some(chosen) = self.new_spawn.chosen.as_mut() {
            if chosen.entry() == subject.entry {
                chosen.set_entry(to);
                if chosen.is_new() {
                    chosen.set_read_entry(to);
                }
            }
        }
        // The workspace's open row follows it on the same terms.
        if self.listed.open == Some(subject.entry) {
            self.listed.open = Some(to);
        }
        self.template = None;
        self.created_for = None;
        Ok(())
    }

    /// Whether a template may move to `to`, and the two keys of the move when
    /// it may: the claim's key now, and the key it is re-keyed under.
    ///
    /// The decision apart from the writes, so it can be checked with no
    /// session. `taken` is [`Self::entry_taken`]'s answer for `to`. Moving a
    /// row back to where the database has it is always allowed: the entry is
    /// taken, and by this row. `Ok(None)` is a move to the entry the row
    /// already has.
    pub(crate) fn plan_template_move(subject: &TemplateSubject, to: u32, taken: bool) -> Result<Option<(Key, Key)>, String> {
        let row = vale_mangos::row::Numbered {
            entry: subject.entry,
            read_entry: subject.read_entry,
            patch: subject.patch,
            life: subject.claim,
        };
        vale_mangos::row::plan_rekey(&S::NUMBERING, row, to, taken)
    }

    /// What the template window is about; see [`TemplateSubject`].
    pub(crate) fn template_subject(&self, edits: &Edits) -> Option<TemplateSubject> {
        match self.mode {
            crate::tools::placements::place::Mode::Place => {
                let known = self.new_spawn.chosen.as_ref()?;
                Some(TemplateSubject {
                    entry: known.entry(),
                    read_entry: known.read_entry(),
                    patch: known.patch(),
                    claim: edits.life(S::TEMPLATE, &known.key()),
                    label: known.label(),
                })
            }
            crate::tools::placements::place::Mode::Select => {
                let spawn = self.chosen_edited(Some(edits))?;
                Some(TemplateSubject {
                    entry: spawn.entry(),
                    read_entry: spawn.read_entry(),
                    patch: spawn.patch(),
                    claim: edits.life(S::TEMPLATE, &S::template_key(spawn.entry(), spawn.patch())),
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
    pub(crate) fn fold_template_moves(&mut self, edits: &Edits) {
        let moves: Vec<(u32, u32, u32)> = edits
            .moves(S::TEMPLATE)
            .filter_map(|(from, to)| {
                let patch = from.patch()?;
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
        // The read's rows only when the moves differ from the ones they were
        // last folded through: folding is a walk of the whole map, and the
        // store changes on every frame of a drag.
        if self.folded_for.as_ref() != Some(&moves) {
            for spawn in self.spawns.iter_mut() {
                spawn.fold_entry(&moved);
            }
            // A changed entry changes which template a near spawn is laid
            // over with.
            self.overlay.at = None;
        }
        for known in self.new_spawn.matches.iter_mut() {
            known.set_entry(moved(known.read_entry(), known.patch()));
        }
        if let Some(chosen) = self.new_spawn.chosen.as_mut() {
            chosen.set_entry(moved(chosen.read_entry(), chosen.patch()));
        }
        let known: Vec<S::Known> = self
            .known
            .drain()
            .map(|(_, mut known)| {
                known.set_entry(moved(known.read_entry(), known.patch()));
                known
            })
            .collect();
        for known in known {
            self.known.insert(known.entry(), known);
        }
        self.folded_for = Some(moves);
    }

    /// Rebuild the templates this project creates from the store, and put
    /// them where a spawn of one and the picker find them.
    pub(crate) fn rebuild_templates(&mut self, edits: &Edits) {
        self.known.retain(|_, known| !known.is_new());
        let mut templates: Vec<S::Known> = edits
            .rows()
            .filter(|(table, _, row)| *table == S::TEMPLATE && row.life == Life::Insert)
            .filter_map(|(_, key, row)| S::Known::from_claim(key, row))
            .collect();
        templates.sort_by_key(|known| known.entry());
        for known in &templates {
            self.known.insert(known.entry(), known.clone());
        }
        // The picker's chosen template is a copy taken when it was clicked, so
        // a created one is replaced by its current row: the window renames it
        // and the picker shows the new name on the same frame. One whose row
        // was discarded or undone is dropped.
        let chosen = match self.new_spawn.chosen.as_ref() {
            Some(chosen) if chosen.is_new() => templates.iter().find(|known| known.entry() == chosen.entry()).cloned(),
            other => other.cloned(),
        };
        self.new_spawn.chosen = chosen;
        self.templates = templates;
    }

    /// Whether a read is in flight, for the panel.
    pub(crate) fn reading(&self) -> bool {
        self.task.is_some()
    }

    /// Whether a click in the world would place a spawn: the panel is on its
    /// Place half and a template has been chosen.
    ///
    /// The counterpart of `Placing::armed()`, with the same two conditions:
    /// switching to Select or pressing `Escape` makes it false.
    pub(crate) fn placing(&self) -> bool {
        self.mode == crate::tools::placements::place::Mode::Place && self.new_spawn.chosen.is_some()
    }

    /// Turn the template on the cursor by `degrees` about up, snapped to the
    /// placer's step when `snap`. Does nothing before the ghost has a facing.
    /// `,` and `.` call it from `keys::keys` and `Alt` with the mouse from
    /// `crate::tools::placements::gizmo::spin`.
    pub(crate) fn turn_ghost(&mut self, degrees: f32, snap: bool) {
        if let Some(facing) = self.ghost_facing {
            self.ghost_facing = Some(turned(facing, degrees, snap));
        }
    }
}

/// Registers one subject's [`SpawnTool`] and its systems.
pub(crate) struct SpawnToolPlugin<S>(PhantomData<fn() -> S>);

impl<S> Default for SpawnToolPlugin<S> {
    fn default() -> SpawnToolPlugin<S> {
        SpawnToolPlugin(PhantomData)
    }
}

/// The two halves of a press on a spawn, across both spawn tools: every
/// tool's aim, then every tool's press, so a press can go to the nearer of a
/// creature and a game object whichever tool is chosen. See [`SpawnHover`].
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SpawnPointer {
    Aim,
    Press,
}

impl<S: SpawnSubject> Plugin for SpawnToolPlugin<S> {
    fn build(&self, app: &mut App) {
        use script::{display_on_the_command_line, on_the_command_line, placed_on_the_command_line, template_on_the_command_line};
        use systems::{
            aim, collect_the_near, drag, draw, enclose, ghost, go_to, learn_the_created, press_across, read_the_map,
            rebuild_created, search, stand_them_up, tag_the_near,
        };
        app.init_resource::<SpawnTool<S>>()
            .init_resource::<OtherSpawns>()
            .init_resource::<SpawnHover>()
            .configure_sets(Update, SpawnPointer::Aim.before(SpawnPointer::Press))
            .add_systems(
                Update,
                // After the pick, like every tool here: reading the pointer
                // before this frame's ray is built uses last frame's ray.
                (
                    read_the_map::<S>,
                    // Before the near list, so a spawn placed this frame is
                    // in it rather than one frame late.
                    rebuild_created::<S>,
                    learn_the_created::<S>,
                    search::<S>,
                    collect_the_near::<S>,
                    // Before the aim, which picks only what the event filter
                    // leaves in the world.
                    tag_the_near::<S>,
                    aim::<S>.in_set(SpawnPointer::Aim),
                    menu_target::<S>,
                    press::<S>.in_set(SpawnPointer::Press),
                    press_across::<S>.in_set(SpawnPointer::Press),
                    enclose::<S>,
                    drag::<S>,
                    // After the drag, which writes the store on every frame
                    // it moves, and before everything that draws.
                    lay_the_edits::<S>,
                    fetch_rows::<S>,
                )
                    .chain()
                    .after(crate::pick::aim),
            )
            .add_systems(Update, (draw::<S>, stand_them_up::<S>, ghost::<S>).after(lay_the_edits::<S>))
            // `--spawn` runs once and is ordered only after the read it waits
            // for. The two that create and remove a row run after it, so
            // `--spawn-delete` acts on the map `--spawn` has already opened.
            .add_systems(
                Update,
                (
                    on_the_command_line::<S>,
                    placed_on_the_command_line::<S>,
                    template_on_the_command_line::<S>,
                    display_on_the_command_line::<S>,
                    go_to::<S>,
                )
                    .chain()
                    .after(read_the_map::<S>),
            );
    }
}
