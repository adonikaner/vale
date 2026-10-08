//! What is open: the project, the tiles, the history, and the overlay the
//! archives are read through.
//!
//! ## How an unsaved edit reaches the renderer
//!
//! The renderer reads a tile by its virtual path, through
//! `vale_client::assets::GameAssets`, on the task pool. To draw an edit, that
//! path must answer with the edited bytes. The overlay installed here does
//! this: it looks in an in-memory map first, the project folder second, and
//! the archives last. Nothing else in the client changes, and a path with no
//! edit behind it costs one hash lookup.
//!
//! The order matters in both directions. In memory before the folder, so an
//! unsaved edit is what is drawn; the folder before the archives, so a project
//! opened in a new session draws what it saved last time without having to load
//! every tile it ever touched.
//!
//! ## Open tiles and the six sets of pending work
//!
//! * Open — parsed into an [`AdtFile`] in [`EditSession::tiles`], which is
//!   what a tool edits.
//! * Dirty — the ground on screen is behind the heights, per chunk.
//!   [`crate::tools::live_ground`] clears it by pushing the new vertices into
//!   the meshes already on the GPU, every frame of a stroke.
//! * Regrow — the foliage on that ground is behind it, per chunk. A lawn
//!   is a merged mesh and has to be built again rather than patched, so
//!   [`crate::tools::terrain::live_foliage`] drains this once, when the button
//!   comes up.
//! * Moved — the placements on it are behind their `MDDF` records, per
//!   entry. Nothing about a doodad is in the ground's mesh, so this is its own
//!   set and [`crate::tools::doodads::reconcile`] drains it.
//! * Repaint — the blend maps on the GPU are behind `MCAL`, per chunk.
//!   The paint is in a texture rather than in a mesh or a transform, so
//!   neither of the two sets above reaches it.
//!   [`crate::tools::textures::live_paint`] drains it.
//! * Stale — the whole tile has to be read again: what the live path could
//!   not reach. [`crate::tools::remesh`] forgets it and lets the streamer do it.
//! * Unsaved — its bytes have moved ahead of the project folder. Saving
//!   clears it. A save of a tile whose bytes are the archives' own removes
//!   the tile's file instead of writing it; see [`EditSession::save_one`].
//!
//! They are separate because they are cleared by different things and at
//! different rates. Merging any two breaks one of them: if the ground and the
//! grass shared one set, the grass would follow only the last frame's four
//! chunks, because the ground's set is emptied sixty times a second.
//!
//! A change goes in one of the live sets when it alters something the drawn
//! tile holds, and in stale when it alters something the drawn tile was built
//! from. A height is held in a vertex buffer and a blend weight in a texel, so
//! both can be written in place. Which textures a chunk names, and how many
//! placements a tile has, are decided when the tile is built, so a change to
//! either means the tile has to be read again.

use vale_client::assets::GameAssets;
use vale_client::render::focus::WorldFocus;
use vale_edit::adt::AdtFile;
use vale_edit::dbc::DbcFile;
use vale_edit::project::Project;
use vale_edit::undo::History;
use vale_edit::TileKey;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use std::sync::{Arc, RwLock};

/// The tiles this editor has changed and not yet saved, keyed by virtual path in
/// lower case.
///
/// Lower case because an archive lookup does not care and a caller may spell a
/// path either way; the client's own terrain loader spells it as
/// `vale_assets::adt_path` does, and this must answer whatever spelling
/// arrives.
type Edited = Arc<RwLock<HashMap<String, Vec<u8>>>>;

/// What is open.
#[derive(Resource)]
pub struct EditSession {
    pub project: Project,
    /// The map being edited, by its `World\Maps\` directory name.
    pub map: String,
    /// …and its id, which is what the light chain and the collision world are
    /// keyed by.
    pub map_id: u32,
    /// Every map the archives name, so a panel can offer them. Sorted by name.
    pub maps: Vec<(u32, String)>,
    /// Tiles parsed for editing.
    pub tiles: HashMap<(u32, u32), AdtFile>,
    /// …and client tables, by their bare name (`"Spell"`).
    ///
    /// The same arrangement as `tiles` and for the same reasons: opened on
    /// demand, edited in memory, written to the project folder and into the
    /// overlay. There is no 3x3 to stream, so a table stays open for the
    /// session once something has asked for it.
    pub tables: std::collections::HashMap<String, DbcFile>,
    pub history: History,
    /// The chunks whose ground on screen is behind their heights, per tile.
    ///
    /// Cleared by [`crate::tools::live_ground`], which pushes the new vertices
    /// straight into the meshes already on the GPU.
    pub dirty: HashMap<(u32, u32), HashSet<usize>>,
    /// The chunks whose foliage is behind their heights, per tile.
    ///
    /// Separate from `dirty` because the two are caught up at different rates
    /// and by different means. A vertex patch is cheap enough to run on every
    /// frame of a stroke, so `dirty` is cleared sixty times a second; a lawn is
    /// a merged mesh that has to be built again from scratch, so this
    /// accumulates for the whole stroke and is drained once, when the button
    /// comes up. Sharing one set would have left the grass following only the
    /// last frame's four chunks.
    ///
    /// Cleared by [`crate::tools::terrain::live_foliage`].
    pub regrow: HashMap<(u32, u32), HashSet<usize>>,
    /// The `MDDF` entries whose drawn placement is behind their record, per
    /// tile.
    ///
    /// A third set on the same argument as the second: a placement is not in the
    /// ground's mesh and is not a chunk, so neither of the other two names it.
    /// Cleared by [`crate::tools::doodads::reconcile`], every frame of a drag —
    /// which it can afford to be, because moving a placement is writing one
    /// transform where moving the ground is writing a buffer.
    pub moved: HashMap<(u32, u32), HashSet<usize>>,
    /// …and the `MODF` entries, which is a separate set because the two
    /// lists are numbered separately: `MDDF` entry 3 and `MODF` entry 3 are
    /// different objects, and one set of indices shared between them moves the
    /// wrong thing. Cleared by [`crate::tools::wmos::reconcile`].
    pub moved_buildings: HashMap<(u32, u32), HashSet<usize>>,
    /// The chunks whose blend maps on the GPU are behind their `MCAL`, per
    /// tile.
    ///
    /// A fourth set on the same argument as the third. What is behind here is a
    /// texture rather than a mesh or a transform, and the fix is a write into the
    /// tile's alpha atlas — see [`crate::tools::textures::live_paint`], which
    /// drains it every frame of a stroke because one chunk's cell is 16 KB
    /// against the ground's 81,840 vertices.
    ///
    /// A chunk whose set of textures changed is not in here; it is in `stale`.
    /// The set is part of the material a draw group was built with, so there is
    /// nothing to patch and the tile has to be read again.
    pub repaint: HashMap<(u32, u32), HashSet<usize>>,
    /// Tiles that need the whole tile read again rather than a vertex patch:
    /// what the live path could not do, and anything that changes more than
    /// heights.
    pub stale: HashSet<(u32, u32)>,
    /// Tiles whose bytes are ahead of the project folder.
    pub unsaved: HashSet<(u32, u32)>,
    /// Tables whose bytes are ahead of the project folder, by name.
    pub unsaved_tables: HashSet<String>,
    /// How many field edits have landed, for anything that caches a reading
    /// of a table rather than the table itself.
    ///
    /// The storyboard is the one reader: it walks four tables and asks the
    /// archives about every model, which is not a per-frame cost. A counter
    /// rather than a per-table flag because what it answers is "is what I built
    /// still true", and an edit to any table in a chain can make it false.
    pub table_revision: u64,
    /// Which tiles this map claims — the WDT's `MAIN` grid.
    ///
    /// Held here rather than asked for each time because it is the answer to a
    /// question the map window asks for all 4,096 cells every frame, and the
    /// file it comes from is 32 KB. `crate::tools::tiles` changes it at the same
    /// time it writes the file.
    pub claimed: HashSet<(u32, u32)>,
    /// Tiles whose existence changed, for the streamer to be told about.
    ///
    /// Made and unmade ground both land here. See
    /// `crate::tools::tiles::restream`, which is what acts on it, and the note
    /// there on why the streamer has to be told at all rather than noticing.
    pub restream: HashSet<(u32, u32)>,
    /// Paths whose bytes have changed under the overlay, for the caches
    /// that key on a path and keep what they read.
    ///
    /// The renderer's `ModelCache` is the one that matters: it holds every
    /// model it has read for the life of the process, so an edited `.m2`
    /// published here goes on drawing as it was. A baked attachment offset
    /// did not appear in the storyboard for this reason. A path that failed
    /// to read is remembered too, so a model written after something asked
    /// for it would never be looked at again.
    ///
    /// Recorded rather than acted on, because the cache is a Bevy resource
    /// and the things that publish (a bake, a save, the lab's own scratch
    /// copy) are panels with no `World` to hand. See
    /// [`forget_what_changed`], which drains it.
    pub republished: Vec<String>,
    /// Whether every path has changed under the overlay, which is what a
    /// project switch does: every path the overlay answers now comes out of a
    /// different folder.
    pub republished_all: bool,
    /// The edited tables have been put where the client reads them, and
    /// the banks that have already read them have to be told.
    ///
    /// The file half above is `ModelCache`; this is the other cache in the
    /// client, and it is two deep: `GameAssets` parses each table once, and
    /// `world::entities::DisplayCache` holds an `Arc` of that parse from the
    /// first entity of the session. Forgetting only the first leaves every
    /// pass reading the old parse, so an edited spell does not change in the
    /// storyboard or in a playtest.
    ///
    /// Set by [`publish_for_the_preview`] and by `crate::playtest::start`,
    /// drained by [`forget_what_changed`].
    pub tables_republished: bool,
    /// How many times a file has been forgotten, for anything holding a
    /// picture of a file rather than the file.
    ///
    /// The file counterpart of `table_revision`. The lab is the one reader: it
    /// compares what it has hanging against what it wants, and a re-bake over
    /// the same path has the same path. Without this counter the preview goes
    /// on showing the copy it hung the first time, even after the model cache
    /// has forgotten the path. Bumped by [`forget_what_changed`], which runs
    /// before the hang.
    pub republished_revision: u64,
    /// Everything drawn is from a project that is no longer open.
    ///
    /// Set by [`Self::switch_to`] and drained by [`reread_after_a_switch`],
    /// which is where the reason for the indirection is: the session has no
    /// list of what is on screen, and the tiles that are drawn are the
    /// streamer's business rather than the session's.
    pub reread_everything: bool,
    /// Which map [`Self::claimed`] is of.
    ///
    /// Without it the grid is the first map's for the whole session: switching
    /// maps clears the open tiles but the claims were read once at startup, so
    /// the map window drew Azeroth's claims over Development's empty grid. A
    /// cell claimed with no picture behind it looks the same as a real tile
    /// that has no minimap, so the fault was not visible as an error.
    /// [`refresh_claims`] puts it back in step for every route that changes
    /// the map, not only the drop-down.
    pub claims_for: String,
    /// The last thing worth telling the person at the keyboard.
    pub status: String,
    /// How many times each tile's bytes have been republished.
    ///
    /// A counter and not a flag, and it exists for one reader. A tile's
    /// replacement is a read of the overlay that starts on one frame and lands
    /// several frames later, and anything published in between is a change that
    /// read cannot have seen. The copy that arrives is then built from the older
    /// bytes and nothing corrects it: the live sets above were drained against
    /// the outgoing copy, which is despawned when the incoming one appears.
    ///
    /// So the number is what `crate::tools::terrain::remesh` compares. It
    /// remembers the revision each replacement was started at and asks for
    /// another read when the file has moved on since. See [`Self::publish`],
    /// which is the only thing that bumps it.
    ///
    /// Without it, an undo that landed during the re-read a drag had asked for
    /// left the building drawn where it had been dragged, while the file was
    /// already correct.
    revision: HashMap<(u32, u32), u64>,
    /// How many times each open tile has changed in memory: every
    /// [`Self::touched`], [`Self::shaded`], [`Self::moved`],
    /// [`Self::moved_building`], [`Self::repainted`] and [`Self::publish`].
    ///
    /// [`Self::revision`] counts publishes, and some edits are not published
    /// while they are being made: a selection of vertices dragged to a new
    /// height or tilt is published when the mouse button comes up. A tile
    /// re-read during such a drag is built from bytes the drag has moved on
    /// from, and when it replaces the drawn tile the live patches are lost,
    /// which drew the ground part old and part new. `remesh` compares this
    /// counter instead, so a tile that changed while its re-read was in flight
    /// is read again.
    changes: HashMap<(u32, u32), u64>,
    /// The open tiles whose in-memory bytes are ahead of what was last
    /// published. `remesh` publishes such a tile before it asks for a re-read,
    /// so the read sees every edit made so far.
    unpublished: HashSet<(u32, u32)>,
    edited: Edited,
    /// The rows this project changes in the server's database.
    ///
    /// Held here and not in the tool, for the reason the open tables are: a
    /// project's edits outlive whichever tool is chosen, and a save has to be
    /// able to write all of them without asking a panel what it is holding.
    ///
    /// Empty for every project that has not touched a server table, which is
    /// every terrain project. See [`vale_mangos::row::Edits`], and
    /// [`crate::server::rows`], which writes them.
    pub server_edits: vale_mangos::row::Edits,
    /// …and the waypoint paths it changes, which are the one server subject
    /// whose edit is a set of rows rather than a column.
    ///
    /// A second store rather than more entries in the first, because the two
    /// are written differently: a column edit is an `UPDATE` of a row that is
    /// already there, and a path is every row under a key replaced. See
    /// [`vale_mangos::path::Paths`], and the reason the unit is the whole
    /// path in that module's own comment.
    pub server_paths: vale_mangos::path::Paths,
    /// The scripts it changes, which are the other subject whose edit is a
    /// set of rows: a script's rows have no key of their own, so every row
    /// under an id is replaced. See [`vale_mangos::scripts::Scripts`].
    pub server_scripts: vale_mangos::scripts::Scripts,
    /// How many row edits have landed, for anything that cached a reading
    /// of the database.
    ///
    /// `crate::tools::creatures` is the one reader: it holds a map's worth of
    /// spawns read over a connection, and an edit to a position has to move the
    /// marker without a second round trip. A counter rather than a flag,
    /// because what it answers is "is what I read still true".
    pub server_edit_revision: u64,
    /// Whether [`Self::server_edits`] is ahead of the project folder.
    pub server_edits_unsaved: bool,
    /// …and whether [`Self::server_paths`] is.
    pub server_paths_unsaved: bool,
    /// …and whether [`Self::server_scripts`] is.
    pub server_scripts_unsaved: bool,
    /// What the last apply that succeeded in this process put in the
    /// database, as `crate::server::rows::Plan::signature` measures it.
    ///
    /// The statements a save emits are a diff of the project rather than of
    /// what changed since the last save, so a project carrying an edit from a
    /// previous session emits the same statements every time — on every save
    /// and at the start of every playtest. Re-applying them is harmless; the
    /// `.reload` that follows is not, because it reports a change as having
    /// gone live when nothing moved.
    ///
    /// Per process rather than kept in the project, deliberately: what is in
    /// the database between one launch and the next is not this editor's to
    /// know, since a row may have been changed by hand or by another project.
    pub applied_signature: Option<u64>,
    /// …and the same for the creature half, which is a different plan
    /// applied by a different gesture — see `crate::server::creatures`.
    ///
    /// Two fields and not one because the two are applied at different
    /// moments: a spell reaches the database on every save and a creature row
    /// when a person presses a button, so one number would report the other
    /// half's state. The panel uses it to choose between "applied" and
    /// "applied, and changed since", two states that look identical in a
    /// revert file.
    ///
    /// Per process, for [`Self::applied_signature`]'s reason. `None` at the
    /// start of a session means the panel falls back to what the revert file
    /// says, which is that the rows in it were applied — because what happened
    /// in an earlier session is not this editor's to know beyond that.
    pub applied_creatures: Option<u64>,
    /// …and the same for the item half, which is a third plan applied
    /// by a third gesture — see `crate::server::items`.
    ///
    /// Its own field for [`Self::applied_creatures`]' reason: the three are
    /// applied at three different moments, so one number would report
    /// another half's state on this half's panel.
    pub applied_items: Option<u64>,
    /// …and for the quest half — see `crate::server::quests`.
    pub applied_quests: Option<u64>,
    /// …and for the game-object half — see `crate::server::gameobjects`.
    pub applied_gameobjects: Option<u64>,
    /// …and for the loot half — see `crate::server::loot`.
    pub applied_loot: Option<u64>,
    /// …and for the behaviour half: events, scripts and spell lists. See
    /// `crate::server::behaviour`.
    pub applied_behaviour: Option<u64>,
    /// …and for what creatures sell and teach. See `crate::server::services`.
    pub applied_services: Option<u64>,
    /// …and of the five subjects of [`crate::server::places`], by
    /// `places::Group::index`: conditions, gossip, maps, area triggers,
    /// graveyards.
    pub applied_places: [Option<u64>; 5],
    /// How many times this session has written the world database, applying
    /// or putting back any subject, or pointed at a different database. Every
    /// cache of what the database holds is keyed on it; see
    /// [`crate::server::fresh`].
    ///
    /// One counter for every subject, not one each, because a write of one
    /// subject changes others' rows: an item renumber moves loot, vendor and
    /// quest columns, and a creature renumber moves quest relations, vendor
    /// and trainer lists and AI events. With a counter per subject, a list
    /// keyed on its own subject's counter went on showing the rows from before
    /// such a move. Bump it with [`Self::wrote_the_database`].
    ///
    /// Not [`Self::applied_quests`] and its siblings, which cannot tell a
    /// put-back from nothing in a session that did not make the apply.
    pub database_writes: u64,
    /// The undo entry every server-row write goes on while a group is being
    /// changed, as `(label, subject)`. See [`Self::as_one`].
    ///
    /// A spawn's writers each name their own row in their gesture subject, so a
    /// move of twelve creatures would be twelve entries on the stack. While this
    /// is set, every write that carries a gesture takes this one instead, and
    /// the twelve fold into one.
    one_gesture: Option<(String, String)>,
    /// The archives' own copy of a file, which [`Self::save`] compares a
    /// tile against. `None` in a test session, which has no archives; every
    /// tile is then written.
    pub shipped: Option<Shipped>,
    /// The check for tiles the project folder already holds unchanged,
    /// started when a project is opened. See [`drop_unchanged_tiles`].
    unchanged: Option<Task<Vec<Unchanged>>>,
    /// The locked vertices of each tile of the open map that has been asked
    /// about, read from the project on first use. See [`Self::locked`]. A tile
    /// with no entry has not been read yet; an empty set is a tile with no
    /// locks.
    locks: HashMap<(u32, u32), vale_edit::ops::vertices::Selected>,
    /// The locked doodads and WMOs of the open map, by `uniqueId`. `None`
    /// until the first tile of the map is opened, which reads them; see
    /// [`PlacementLocks`].
    placement_locks: Option<PlacementLocks>,
}

/// The doodads and WMOs of one map that are locked: no tool selects, moves or
/// deletes them, and moving the ground does not carry them. Kept by
/// `uniqueId`, which every copy of a placement in the tiles it crosses shares
/// and which does not change when other placements are added or removed.
///
/// Stored in the project as `editor\locks\<Map>.placements.txt`, one line per
/// placement: `doodad <uniqueId>` or `wmo <uniqueId>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlacementLocks {
    pub doodads: std::collections::BTreeSet<u32>,
    pub wmos: std::collections::BTreeSet<u32>,
}

impl PlacementLocks {
    /// The set for one kind of placement.
    pub fn of(&self, kind: crate::tools::place::Kind) -> &std::collections::BTreeSet<u32> {
        match kind {
            crate::tools::place::Kind::Doodad => &self.doodads,
            crate::tools::place::Kind::Wmo => &self.wmos,
        }
    }

    fn of_mut(&mut self, kind: crate::tools::place::Kind) -> &mut std::collections::BTreeSet<u32> {
        match kind {
            crate::tools::place::Kind::Doodad => &mut self.doodads,
            crate::tools::place::Kind::Wmo => &mut self.wmos,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.doodads.is_empty() && self.wmos.is_empty()
    }

    /// The stored form. See [`Self::from_text`].
    pub fn to_text(&self) -> String {
        let mut text = String::from(
            "# Locked doodads and WMOs of one map, by uniqueId, written by the world editor.\n",
        );
        for id in &self.doodads {
            text.push_str(&format!("doodad {id}\n"));
        }
        for id in &self.wmos {
            text.push_str(&format!("wmo {id}\n"));
        }
        text
    }

    /// Read [`Self::to_text`]'s form. Blank lines and `#` lines are skipped; a
    /// line that is neither `doodad <id>` nor `wmo <id>` is an error naming it.
    pub fn from_text(text: &str) -> Result<PlacementLocks, String> {
        let mut locks = PlacementLocks::default();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = || format!("line {}: {line:?} is not `doodad <id>` or `wmo <id>`", number + 1);
            let (kind, id) = line.split_once(char::is_whitespace).ok_or_else(bad)?;
            let id: u32 = id.trim().parse().map_err(|_| bad())?;
            match kind {
                "doodad" => locks.doodads.insert(id),
                "wmo" => locks.wmos.insert(id),
                _ => return Err(bad()),
            };
        }
        Ok(locks)
    }
}

/// What undo entry a server-row edit goes on, and what folds into it.
///
/// `subject` is the gesture key `vale_edit::undo::History::begin_gesture`
/// folds on, so it alone decides what becomes one entry. A form passes a
/// subject naming the column, so typing into a name box is one entry and
/// editing two columns is two. A drag passes one naming the row, so the
/// three position columns it writes are one entry — without that, moving a
/// creature would take three presses of `Ctrl+Z` to put back, one per axis.
#[derive(Debug, Clone, Copy)]
pub struct Gesture<'a> {
    /// What to call it in the history: "Move creature", "Edit creature".
    pub label: &'a str,
    /// What it is about. Two edits with the same subject close together in time
    /// are one entry; two with different subjects never are.
    pub subject: &'a str,
    /// The frame clock the fold window is measured on.
    pub now: f64,
}

impl EditSession {
    /// A session over `project` on Azeroth with nothing open, for a test that
    /// puts its own tiles in [`Self::tiles`]. Nothing here reads the archives.
    #[cfg(test)]
    pub(crate) fn for_tests(project: Project) -> EditSession {
        EditSession {
            status: String::new(),
            project,
            map: "Azeroth".to_string(),
            map_id: 0,
            maps: Vec::new(),
            tiles: HashMap::default(),
            tables: std::collections::HashMap::default(),
            history: History::new(),
            dirty: HashMap::default(),
            regrow: HashMap::default(),
            moved: HashMap::default(),
            moved_buildings: HashMap::default(),
            revision: HashMap::default(),
            changes: HashMap::default(),
            unpublished: HashSet::default(),
            repaint: HashMap::default(),
            stale: HashSet::default(),
            unsaved: HashSet::default(),
            unsaved_tables: HashSet::default(),
            table_revision: 0,
            restream: HashSet::default(),
            republished: Vec::new(),
            republished_all: false,
            tables_republished: false,
            republished_revision: 0,
            claimed: HashSet::default(),
            reread_everything: false,
            claims_for: "Azeroth".to_string(),
            edited: Edited::default(),
            server_edits: Default::default(),
            server_paths: Default::default(),
            server_scripts: Default::default(),
            server_edit_revision: 0,
            server_edits_unsaved: false,
            server_paths_unsaved: false,
            server_scripts_unsaved: false,
            applied_signature: None,
            applied_creatures: None,
            applied_items: None,
            applied_quests: None,
            applied_gameobjects: None,
            applied_loot: None,
            applied_behaviour: None,
            applied_services: None,
            applied_places: [None; 5],
            database_writes: 0,
            one_gesture: None,
            shipped: None,
            unchanged: None,
            locks: HashMap::default(),
            placement_locks: None,
        }
    }

    /// The key a tile is edited under.
    pub fn key(&self, coord: (u32, u32)) -> TileKey {
        TileKey::new(self.map.clone(), coord.0, coord.1)
    }

    /// Parse a tile for editing, if it is not open already.
    ///
    /// Reads it through the same chain the renderer does, so an already-edited
    /// tile comes back as edited and a fresh one comes out of the archives.
    /// Returns false for a tile the map does not have, which is most of the 64
    /// by 64 grid on every map.
    pub fn open(&mut self, assets: &GameAssets, coord: (u32, u32)) -> bool {
        if self.tiles.contains_key(&coord) {
            return true;
        }
        // The map's locked placements, read with its first tile so that every
        // reader after this can answer without the session borrowed mutably.
        self.read_placement_locks();
        let path = self.key(coord).vpath();
        let bytes = match assets.with_archive(|chain| Ok(chain.read(&path).ok())) {
            Ok(Some(bytes)) => bytes,
            _ => return false,
        };
        match AdtFile::parse(&bytes) {
            Ok(tile) => {
                self.tiles.insert(coord, tile);
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

    /// Where a tile's locked vertices are stored in the project, outside the
    /// game data: `editor\locks\<Map>_<x>_<y>.txt`.
    /// `editor` is one of `Project::NOT_GAME_DATA`, so a publish never packs
    /// it.
    fn locks_vpath(&self, coord: (u32, u32)) -> String {
        format!(r"editor\locks\{}_{}_{}.txt", self.map, coord.0, coord.1)
    }

    /// A tile's locked vertices: the ones no height operation may move. Read
    /// from the project the first time a tile is asked about. A file that
    /// cannot be read as locks is reported in the status line and treated as
    /// no locks.
    pub fn locked(&mut self, coord: (u32, u32)) -> &vale_edit::ops::vertices::Selected {
        if !self.locks.contains_key(&coord) {
            let vpath = self.locks_vpath(coord);
            let set = match self.project.read(&vpath) {
                None => Default::default(),
                Some(bytes) => {
                    match vale_edit::ops::vertices::Selected::from_text(&String::from_utf8_lossy(&bytes)) {
                        Ok(set) => set,
                        Err(e) => {
                            self.status = format!("{vpath}: {e}; the tile is treated as having no locked vertices");
                            Default::default()
                        }
                    }
                }
            };
            self.locks.insert(coord, set);
        }
        &self.locks[&coord]
    }

    /// Where the open map's locked placements are stored. See
    /// [`PlacementLocks`].
    fn placement_locks_vpath(&self) -> String {
        format!(r"editor\locks\{}.placements.txt", self.map)
    }

    /// Read the open map's locked placements, if they have not been read.
    fn read_placement_locks(&mut self) {
        if self.placement_locks.is_some() {
            return;
        }
        let vpath = self.placement_locks_vpath();
        let locks = match self.project.read(&vpath) {
            None => PlacementLocks::default(),
            Some(bytes) => match PlacementLocks::from_text(&String::from_utf8_lossy(&bytes)) {
                Ok(locks) => locks,
                Err(e) => {
                    self.status = format!("{vpath}: {e}; no placement is treated as locked");
                    PlacementLocks::default()
                }
            },
        };
        self.placement_locks = Some(locks);
    }

    /// The open map's locked placements. Empty before the first tile is open.
    pub fn placement_locks(&self) -> &PlacementLocks {
        static NONE: std::sync::OnceLock<PlacementLocks> = std::sync::OnceLock::new();
        self.placement_locks
            .as_ref()
            .unwrap_or_else(|| NONE.get_or_init(PlacementLocks::default))
    }

    /// Whether one placement is locked.
    pub fn placement_locked(&self, kind: crate::tools::place::Kind, unique_id: u32) -> bool {
        self.placement_locks().of(kind).contains(&unique_id)
    }

    /// Lock or unlock placements, and write the map's locks to the project at
    /// once. Returns how many changed. Like the vertex locks, these are not on
    /// the undo history.
    pub fn set_placements_locked(
        &mut self,
        kind: crate::tools::place::Kind,
        ids: impl IntoIterator<Item = u32>,
        locked: bool,
    ) -> usize {
        self.read_placement_locks();
        let mut locks = self.placement_locks.clone().unwrap_or_default();
        let set = locks.of_mut(kind);
        let mut changed = 0;
        for id in ids {
            let did = match locked {
                true => set.insert(id),
                false => set.remove(&id),
            };
            changed += usize::from(did);
        }
        if changed == 0 {
            return 0;
        }
        let vpath = self.placement_locks_vpath();
        let written = match locks.is_empty() {
            true => self.project.revert(&vpath).map(|_| ()),
            false => self.project.write(&vpath, locks.to_text().as_bytes()).map(|_| ()),
        };
        if let Err(e) = written {
            self.status = format!("{vpath}: the locked placements were not saved: {e}");
        }
        self.placement_locks = Some(locks);
        changed
    }

    /// A tile's locked vertices if they have been read, for a reader that
    /// cannot take the session mutably. The terrain tool reads every open
    /// tile's locks through [`Self::locked`] each frame it is chosen.
    pub fn locks_if_read(&self, coord: (u32, u32)) -> Option<&vale_edit::ops::vertices::Selected> {
        self.locks.get(&coord)
    }

    /// Replace a tile's locked vertices, and write them to the project at once.
    /// An empty set removes the file. Locks are not on the undo history: a
    /// lock changes no file the game reads.
    pub fn set_locked(&mut self, coord: (u32, u32), set: vale_edit::ops::vertices::Selected) {
        let vpath = self.locks_vpath(coord);
        let written = match set.is_empty() {
            true => self.project.revert(&vpath).map(|_| ()),
            false => self.project.write(&vpath, set.to_text().as_bytes()).map(|_| ()),
        };
        if let Err(e) = written {
            self.status = format!("{vpath}: the locked vertices were not saved: {e}");
        }
        self.locks.insert(coord, set);
    }

    /// Put a tile's locked vertices back where `edits` found them. Every
    /// operation that moves a tile's heights calls this after writing them and
    /// before recording the edits, so a lock holds against all of them. See
    /// `vale_edit::ops::vertices::Selected::hold`. Returns how many entries
    /// were held.
    pub fn hold_locked(&mut self, coord: (u32, u32), edits: &mut Vec<vale_edit::ops::Edit>) -> usize {
        if !edits.iter().any(|edit| matches!(edit, vale_edit::ops::Edit::Heights { .. })) {
            return 0;
        }
        self.locked(coord);
        let (Some(locks), Some(tile)) = (self.locks.get(&coord), self.tiles.get_mut(&coord)) else {
            return 0;
        };
        locks.hold(tile, edits)
    }

    /// Take a tile read and parsed elsewhere as open, unless one is already
    /// open at that place. `crate::tools::open_tiles` reads tiles on the task
    /// pool and hands them over here; [`Self::open`] is the same on the
    /// calling thread.
    pub fn insert_opened(&mut self, coord: (u32, u32), tile: AdtFile) {
        if self.tiles.contains_key(&coord) {
            return;
        }
        self.read_placement_locks();
        self.tiles.insert(coord, tile);
    }

    /// Let go of an open tile, so that a session that has flown across a
    /// continent is not holding every tile it flew over.
    ///
    /// Only the parsed copy is dropped. What was published stays in the
    /// overlay, so a tile opened again reads back as it was — see
    /// [`Self::tile_bytes`] — and the caller is what decides which tiles may
    /// go: `crate::tools::close_tiles`, which keeps anything unsaved or on
    /// the history.
    pub fn close(&mut self, coord: (u32, u32)) -> bool {
        match self.tiles.remove(&coord) {
            // A parsed tile is megabytes in many allocations, and a jump closes
            // a block of them at once; they are freed on the task pool rather
            // than in the frame that closes them.
            Some(tile) => {
                bevy::tasks::AsyncComputeTaskPool::get()
                    .spawn(async move { drop(tile) })
                    .detach();
                true
            }
            None => false,
        }
    }

    /// One tile's bytes, as they are now.
    ///
    /// An open tile answers from what is in hand, unsaved edits included; a tile
    /// that is not open is read through the same chain the renderer uses, so a
    /// tile edited in a previous session answers as edited. `None` for a tile
    /// the map does not have.
    ///
    /// This is what a copy reads — see `crate::tools::tiles::paste`. Copying
    /// from disk instead would silently take the version before whatever the
    /// person had just done to it.
    pub fn tile_bytes(&self, assets: &GameAssets, coord: (u32, u32)) -> Option<Vec<u8>> {
        if let Some(tile) = self.tiles.get(&coord) {
            return Some(tile.write());
        }
        let path = self.key(coord).vpath();
        assets
            .with_archive(|chain| Ok(chain.read(&path).ok()))
            .ok()
            .flatten()
    }

    /// Make a tile's path answer with nothing.
    ///
    /// The overlay is asked before the archives, so an entry of zero bytes is a
    /// path that reads back empty. `Adt::parse` of nothing fails, and the
    /// streamer treats that failure as "there is no tile here", so a deleted
    /// tile stops being drawn.
    ///
    /// A tombstone rather than a removal, because the tile is still in the
    /// archives: dropping the override would hand the shipped bytes straight
    /// back. It is undone by `create`, which writes real bytes over it.
    pub fn tombstone(&mut self, coord: (u32, u32)) {
        let path = self.key(coord).vpath().to_ascii_lowercase();
        if let Ok(mut edited) = self.edited.write() {
            edited.insert(path, Vec::new());
        }
        self.tiles.remove(&coord);
        self.restream.insert(coord);
    }

    /// Change which project this session writes to.
    ///
    /// A project is a folder of loose files and an overlay over the archives, so
    /// switching is four things: everything unsaved is written down first,
    /// because the alternative is losing it silently; the overlay is rebuilt
    /// over the new folder; everything read through the old one is forgotten —
    /// tiles, tables, the undo stack, the claims; and the world is told to read
    /// itself again, which is what puts the new project's ground on screen.
    ///
    /// The history goes. An undo entry holds bytes belonging to a file in
    /// the folder that was open; replaying one against another project's file
    /// would write bytes from one map's tile into another's. The stack is
    /// therefore dropped, so that `Ctrl+Z` cannot corrupt the new project.
    ///
    /// `false` when the folder cannot be made, which leaves the session exactly
    /// where it was.
    pub fn switch_to(&mut self, assets: &GameAssets, name: &str) -> bool {
        if name == self.project.name {
            return true;
        }
        let saved = self.save_all();
        let tables = self.save_all_tables();
        let project = match Project::open(&assets.root, name) {
            Ok(project) => project,
            Err(e) => {
                self.status = format!("could not open the project {name}: {e}");
                return false;
            }
        };

        self.adopt(assets, project);
        vale_edit::project::remember_opened(&assets.root, name);
        self.status = match (saved, tables) {
            (0, 0) => format!("opened {name}; the previous project had nothing unsaved"),
            (tiles, 0) => format!("opened {name}; saved {tiles} tiles of the previous project"),
            (0, count) => format!("opened {name}; saved {count} tables of the previous project"),
            (tiles, count) => format!("opened {name}; saved {tiles} tiles and {count} tables of the previous project"),
        };
        true
    }

    /// Take a project as the one this session writes to, forgetting
    /// everything read through whatever was open before.
    ///
    /// The half of [`Self::switch_to`] that is not about saving, so that
    /// [`Self::clear_open_project`] can reach it: emptying the open folder
    /// leaves the session holding tiles, tables and undo entries that came
    /// out of files which no longer exist, and every one of those has to go
    /// for the same reason a switch drops them.
    fn adopt(&mut self, assets: &GameAssets, project: Project) {
        self.edited = Arc::new(RwLock::new(HashMap::default()));
        assets.set_overlay(Some(overlay_over(&project, &self.edited)));
        self.project = project;
        self.tiles.clear();
        // The locks are the project's, read from its folder.
        self.locks.clear();
        self.placement_locks = None;
        self.tables.clear();
        self.unsaved.clear();
        self.unsaved_tables.clear();
        // …and every set naming work that is still owed on a tile. They are
        // keyed by coordinate and survived a project switch, so a tile marked
        // stale or unsaved under the old project was still named under the new
        // one — which for a project that had just been emptied meant the
        // session wrote its files back out again. Nothing in them can be true
        // of a project that has not been opened yet.
        self.dirty.clear();
        self.regrow.clear();
        self.moved.clear();
        self.moved_buildings.clear();
        self.repaint.clear();
        self.stale.clear();
        self.restream.clear();
        self.history = History::new();
        self.table_revision += 1;
        self.revision.clear();
        self.unpublished.clear();
        self.claimed = claimed_tiles(assets, &self.project, &self.map);
        self.claims_for = self.map.clone();
        // The maps are `Map.dbc`'s, which the project may change: a map the
        // old project made is not one this project has. Read again through
        // the new overlay, the list kept the old project's maps, and a new map
        // given the same id was listed twice.
        self.maps = map_directories(assets);
        // The server's half is a project's too. Its edits live in the
        // project folder, so a switch reads the new folder's and forgets the
        // old one's exactly as it does for the tiles and the tables — without
        // this, the rows one project changes would be applied under another
        // project's name on its first save.
        self.server_edits = read_server_edits(&self.project);
        self.server_paths = read_server_paths(&self.project);
        self.server_scripts = read_server_scripts(&self.project);
        self.server_edit_revision += 1;
        self.server_edits_unsaved = false;
        self.server_paths_unsaved = false;
        self.server_scripts_unsaved = false;
        // A different project's rows are a different set of statements, so
        // what the last one applied says nothing about whether this one has to.
        self.applied_signature = None;
        self.applied_creatures = None;
        self.applied_items = None;
        self.applied_quests = None;
        self.applied_gameobjects = None;
        self.applied_loot = None;
        self.applied_behaviour = None;
        self.applied_services = None;
        self.applied_places = [None; 5];
        // Every path the overlay answers now comes out of a different folder.
        self.republished_all = true;
        self.tables_republished = true;
        // Everything on screen came out of the old folder. See
        // [`Self::reread_everything`].
        self.reread_everything = true;
        // The new folder may hold tiles it does not change. A check still
        // running for the old folder is dropped with the task it is in.
        self.unchanged = self
            .shipped
            .as_ref()
            .map(|shipped| shipped.look_for_unchanged(&self.project));
    }

    /// Throw away every file in the open project, and everything this
    /// session read out of them.
    ///
    /// What `default` gets where another project gets deleted: the folder is
    /// the one edits land in when nobody has said otherwise, so it stays and
    /// its contents go — see `vale_edit::project::clear`.
    ///
    /// Nothing is saved first, unlike a switch, because a save would write
    /// the same files that are being thrown away. The caller must have asked
    /// the person before calling this.
    ///
    /// Returns how many files went, or `None` when the folder would not
    /// empty; the status line says which.
    pub fn clear_open_project(&mut self, assets: &GameAssets) -> Option<usize> {
        let name = self.project.name.clone();
        let went = match vale_edit::project::clear(&assets.root, &name) {
            Ok(went) => went,
            Err(e) => {
                self.status = format!("could not clear {name}: {e}");
                return None;
            }
        };
        // Re-opened rather than kept: the folder is the same one, and what
        // has to be dropped is everything this session read out of it.
        match Project::open(&assets.root, &name) {
            Ok(project) => self.adopt(assets, project),
            Err(e) => {
                self.status = format!("{name} was cleared but could not be reopened: {e}");
                return Some(went);
            }
        }
        self.status = match went {
            0 => format!("{name} was already empty"),
            1 => format!("{name}: 1 file permanently deleted"),
            n => format!("{name}: {n} files permanently deleted"),
        };
        Some(went)
    }

    /// Open a client table for editing, if it is not open already.
    ///
    /// Read through the same chain the renderer uses, so a table edited in an
    /// earlier session comes back edited. `false` for a name the archives do not
    /// answer — and for the four entries under `DBFilesClient\` that are zero
    /// bytes, which are not tables at all. See `vale dbc`.
    pub fn open_table(&mut self, assets: &GameAssets, name: &str) -> bool {
        if self.tables.contains_key(name) {
            return true;
        }
        let path = vale_assets::tables::dbc::dbc_path(name);
        let bytes = match assets.with_archive(|chain| Ok(chain.read(&path).ok())) {
            Ok(Some(bytes)) => bytes,
            _ => {
                self.status = format!("{path}: not in the archives");
                return false;
            }
        };
        match DbcFile::parse(&bytes) {
            Ok(table) => {
                self.tables.insert(name.to_string(), table);
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

    pub fn table(&self, name: &str) -> Option<&DbcFile> {
        self.tables.get(name)
    }

    pub fn table_mut(&mut self, name: &str) -> Option<&mut DbcFile> {
        self.tables.get_mut(name)
    }

    /// Open another map.
    ///
    /// Everything open belongs to the old map. A tile coordinate means a
    /// different place on each one, so carrying the parsed tiles across would
    /// edit Kalimdor's ground with Azeroth's bytes. The unsaved ones are saved
    /// first rather than dropped. The height the camera was at belonged to
    /// the old map's ground; asking for the new one's puts it back on the
    /// terrain as soon as the tile under it is parsed.
    ///
    /// The top bar's map list and a camera bookmark on another map both come
    /// through here. Nothing happens for the map already open.
    pub fn switch_map(&mut self, name: String, id: u32, camera: &mut crate::camera::EditorCamera) {
        if self.map == name {
            return;
        }
        self.save_all();
        self.tiles.clear();
        // Keyed by tile coordinate, which names a different tile on the next
        // map.
        self.locks.clear();
        self.placement_locks = None;
        self.unpublished.clear();
        self.stale.clear();
        self.map = name;
        self.map_id = id;
        camera.wants_the_ground = true;
    }

    /// Note that a table's bytes are ahead of what is written down.
    ///
    /// The table counterpart of the `unsaved` mark, and not of
    /// [`Self::publish`]: a field edit does not write the file anywhere.
    ///
    /// The reason is cost. `Spell.dbc` is 16 MB, an edit arrives on every
    /// frame a number is dragged, and [`Self::publish_table`] serialises the
    /// whole table — so publishing per edit is a gigabyte a second of copying
    /// for no reader at all. There is no reader because
    /// `GameAssets::display_tables` parses every DBC once and hands out what it
    /// parsed: nothing in a running session reads a table again. What a table
    /// edit has to reach is the project folder and the patch archive, and both
    /// of those are [`Self::save_table`].
    pub fn table_edited(&mut self, name: &str) {
        self.unsaved_tables.insert(name.to_string());
        self.table_revision += 1;
    }

    /// Write an open table's bytes into the overlay, so the next read of its
    /// path answers with them — the table counterpart of [`Self::publish`].
    ///
    /// Called when a table is saved rather than when it is edited, for the
    /// reason on [`Self::table_edited`]. What it is for is the next thing to
    /// read a table: a playtest logging in, or a relaunch.
    pub fn publish_table(&mut self, name: &str) {
        let Some(table) = self.tables.get(name) else {
            return;
        };
        let path = vale_assets::tables::dbc::dbc_path(name).to_ascii_lowercase();
        let bytes = table.write();
        if let Ok(mut edited) = self.edited.write() {
            edited.insert(path, bytes);
        }
    }

    /// Take a table's bytes back out of the overlay, so the project folder
    /// or the archives answer for its path again, and have the client read
    /// its tables again. The counterpart of [`Self::publish_table`].
    pub fn unpublish_table(&mut self, name: &str) {
        let path = vale_assets::tables::dbc::dbc_path(name).to_ascii_lowercase();
        let had = match self.edited.write() {
            Ok(mut edited) => edited.remove(&path).is_some(),
            Err(_) => false,
        };
        if had {
            self.tables_republished = true;
        }
    }

    /// Put any file's bytes into the overlay under a virtual path, so the
    /// next read of that path answers with them — a model just baked, before
    /// or instead of writing it anywhere.
    ///
    /// The file counterpart of [`Self::publish_table`], for a path that is
    /// neither a tile nor a table. Nothing on disk changes; see
    /// [`Self::save_bytes`].
    pub fn publish_bytes(&mut self, vpath: &str, bytes: Vec<u8>) {
        self.publish_scratch(vpath, bytes);
        self.republished.push(vpath.to_string());
    }

    /// …and take one back out, so the archives (or the project folder) answer
    /// for the path again.
    pub fn unpublish_bytes(&mut self, vpath: &str) {
        self.unpublish_scratch(vpath);
        self.republished.push(vpath.to_string());
    }

    /// The same two, for a path minted fresh on every publish — the lab's
    /// scratch copy, which is `custom\lab\<n>.m2` for a new `n` each time.
    ///
    /// They differ from [`Self::publish_bytes`] and [`Self::unpublish_bytes`]
    /// only in recording nothing in [`Self::republished`]. There is nothing to
    /// forget: no cache can be holding a reading of a path that did not exist
    /// a moment ago.
    ///
    /// Recording would also cause a loop. The lab publishes its copy and
    /// hangs it in one frame, and compares what it has hanging against
    /// [`Self::republished_revision`]; a scratch publish that bumped that
    /// counter would make the next frame's compare differ, which re-hangs,
    /// which publishes again, on every frame the lab is open.
    pub fn publish_scratch(&mut self, vpath: &str, bytes: Vec<u8>) {
        if let Ok(mut edited) = self.edited.write() {
            edited.insert(vpath.to_ascii_lowercase(), bytes);
        }
    }

    pub fn unpublish_scratch(&mut self, vpath: &str) {
        if let Ok(mut edited) = self.edited.write() {
            edited.remove(&vpath.to_ascii_lowercase());
        }
    }

    /// Write any file's bytes into the project folder under a virtual path,
    /// and publish them. `true` on success; the status line says otherwise.
    pub fn save_bytes(&mut self, vpath: &str, bytes: Vec<u8>) -> bool {
        match self.project.write(vpath, &bytes) {
            Ok(at) => {
                self.publish_bytes(vpath, bytes);
                self.status = format!("saved {}", at.display());
                true
            }
            Err(e) => {
                self.status = format!("{vpath}: {e}");
                false
            }
        }
    }

    /// Put a table back to what is written down, dropping every unsaved
    /// edit to it: the bytes are read again through the chain — the project
    /// folder's copy if one has been saved, else the archives' — and the
    /// undo stack forgets the table. `false` for a table that is not open
    /// or will not read again.
    ///
    /// The stack has to forget rather than keep, and the reason is on
    /// `History::forget_table`: its entries were made against the table as it
    /// was, and after the re-read that is a different table. The readers
    /// holding record indices — the browser, the reverse index — rebuild off
    /// the revision, which this bumps.
    pub fn discard_table(&mut self, assets: &GameAssets, name: &str) -> bool {
        if !self.tables.contains_key(name) {
            return false;
        }
        self.tables.remove(name);
        let dropped = self.history.forget_table(name);
        let had_edits = self.unsaved_tables.remove(name);
        self.table_revision += 1;
        // A preview puts the edited table in the overlay, and the overlay
        // answers a read before the project folder and the archives do.
        // Left there, the read below is answered with the edits it is meant
        // to drop, and the client goes on drawing them.
        self.unpublish_table(name);
        if !self.open_table(assets, name) {
            return false;
        }
        self.status = match (had_edits, dropped) {
            (false, _) => format!("{name}: nothing to discard"),
            (true, 0) => format!("{name}: unsaved edits discarded"),
            (true, 1) => format!("{name}: unsaved edits discarded, 1 undo entry dropped"),
            (true, n) => format!("{name}: unsaved edits discarded, {n} undo entries dropped"),
        };
        true
    }

    /// Run `write` with every server-row write in it on one undo entry.
    ///
    /// For a change to a group of spawns: each spawn's writer names its own row
    /// in its gesture subject, which is right for one spawn and makes a group
    /// move one entry per member. Inside this, every write that carries a
    /// gesture is given `label` and `subject` instead, so the members fold into
    /// one entry, and a drag that calls this on every frame keeps folding into
    /// the same entry because the subject does not change.
    ///
    /// Writes made by an undo carry no gesture and are unaffected.
    pub fn as_one<R>(
        &mut self,
        label: &str,
        subject: &str,
        write: impl FnOnce(&mut EditSession) -> R,
    ) -> R {
        let outer = self
            .one_gesture
            .replace((label.to_string(), subject.to_string()));
        let answer = write(self);
        self.one_gesture = outer;
        answer
    }

    /// Open the undo entry a server-row write goes on: its own gesture, or the
    /// group's while [`Self::as_one`] is running.
    fn open_gesture(&mut self, gesture: Gesture<'_>) {
        match &self.one_gesture {
            Some((label, subject)) => {
                self.history
                    .begin_gesture(label.clone(), subject.clone(), gesture.now)
            }
            None => self
                .history
                .begin_gesture(gesture.label, gesture.subject, gesture.now),
        }
    }

    /// Set one column of one row of one of the server's tables, or clear it.
    ///
    /// `value` is already a SQL literal — quoted and escaped by
    /// `vale_mangos::sql` at the moment it is typed, so there is one
    /// escaping rule and it is applied once. `None` takes the edit back, which
    /// removes the column from the project's file and so from the next save's
    /// statements.
    ///
    /// Nothing is sent anywhere here. The project's file is written and the
    /// database is reached by [`crate::server::rows::save`], which is what
    /// Save and a playtest call.
    pub fn set_server_edit(
        &mut self,
        table: &str,
        key: &vale_mangos::row::Key,
        column: &str,
        value: Option<String>,
        // …and the entry it goes on the undo stack as. `None` is an edit made
        // by an undo, which must not push one of its own.
        under: Option<Gesture<'_>>,
    ) {
        if self.server_edits.get(table, key, column) == value.as_deref() {
            // Nothing changed, so nothing is marked unsaved. A form redraws
            // every frame and writes what its widget holds; without this, a
            // panel merely being open would leave the project permanently
            // ahead of its folder and Save permanently lit — and every frame it
            // was drawn would be an entry on the undo stack.
            return;
        }
        if let Some(gesture) = under {
            let before = self
                .server_edits
                .get(table, key, column)
                .map(str::to_string);
            self.open_gesture(gesture);
            self.history
                .record_server_cell(vale_edit::undo::ServerCell {
                    table: table.to_string(),
                    key: key.text(),
                    column: column.to_string(),
                    before,
                    after: value.clone(),
                });
            // Closed at once, as `tables::set_fields` closes its own.
            // `History::begin_gesture` leaves the change open, and an open
            // change is not on the stack: `next_undo` reads `done` and the panel
            // greys its button off that, so the last edit made could not be
            // undone at all and the one before it came back instead. The next
            // gesture with the same subject inside the fold window pops this
            // straight back off and continues it, so folding still works.
            self.history.end();
        }
        self.server_edits.set(table, key, column, value);
        self.server_edits_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// Record that the world database has changed under every cache of it:
    /// an Apply or a Put back of any subject finished, whether or not it
    /// succeeded, or the editor was pointed at another database.
    pub fn wrote_the_database(&mut self) {
        self.database_writes += 1;
    }

    /// Record that this project creates a row, removes one, or says nothing
    /// about it at all.
    ///
    /// [`Self::set_server_edit`]'s counterpart for the claim that is not a
    /// column. Creating a spawn sets no column of a row that is already there,
    /// because there is no row; removing one sets no column at all. What moves
    /// in both cases is the project's whole claim, so that is what is written
    /// and what goes on the undo stack — see `vale_edit::undo::ServerRow`.
    ///
    /// `None` takes the claim back entirely. Undoing a creation does this, and
    /// so does keeping a spawn that was marked for removal.
    ///
    /// Nothing is sent anywhere. The project's file is written and the database
    /// is reached by [`crate::server::creatures::apply`], which is a button.
    pub fn set_server_row(
        &mut self,
        table: &str,
        key: &vale_mangos::row::Key,
        row: Option<&vale_mangos::row::RowEdit>,
        // …and the entry it goes on the undo stack as. `None` is an edit made
        // by an undo, which must not push one of its own.
        under: Option<Gesture<'_>>,
    ) {
        let before = self.server_edits.row_line(table, key);
        let after = row.map(vale_mangos::row::RowEdit::to_line);
        if before == after {
            // Nothing changed, so nothing is marked unsaved. See
            // [`Self::set_server_edit`], where the guard is and why.
            return;
        }
        if let Some(gesture) = under {
            self.open_gesture(gesture);
            self.history
                .record_server_row(vale_edit::undo::ServerRow {
                    table: table.to_string(),
                    key: key.text(),
                    before,
                    after: after.clone(),
                });
            // Closed at once, for `set_server_edit`'s reason: an open
            // change is not on the stack, so the last edit made would be the
            // one that could not be undone.
            self.history.end();
        }
        self.server_edits.set_row_line(table, key, after.as_deref());
        self.server_edits_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// Set what this project says a creature's waypoint path is, or stop
    /// saying anything about it.
    ///
    /// [`Self::set_server_edit`]'s counterpart for the subject whose edit is a
    /// set of rows. `None` takes the claim back entirely, which leaves the
    /// database's own path alone; a `Some` holding a path with no nodes is
    /// the edit that deletes the path. The two are kept distinct on purpose.
    ///
    /// Nothing is sent anywhere. The project's file is written and the database
    /// is reached by [`crate::server::creatures::apply`], which is a button.
    pub fn set_server_path(
        &mut self,
        which: vale_mangos::path::Which,
        owner: u64,
        path: Option<&vale_mangos::path::Path>,
        // …and the entry it goes on the undo stack as. `None` is an edit made
        // by an undo, which must not push one of its own.
        under: Option<Gesture<'_>>,
    ) {
        let before = self.server_paths.get(which, owner);
        let after = path.cloned();
        if before == after {
            // Nothing changed, so nothing is marked unsaved. The form
            // redraws every frame and writes what it holds; without this a
            // panel merely being open would leave Save permanently lit and put
            // an entry on the undo stack every frame. See
            // [`Self::set_server_edit`], where the same guard is and why.
            return;
        }
        if let Some(gesture) = under {
            self.open_gesture(gesture);
            self.history
                .record_server_path(vale_edit::undo::ServerPath {
                    table: which.table().to_string(),
                    key: vale_mangos::row::Key::one(which.key_column(), owner).text(),
                    before: before.as_ref().map(vale_mangos::path::Path::to_line),
                    after: after.as_ref().map(vale_mangos::path::Path::to_line),
                });
            // Closed at once, for `set_server_edit`'s reason: an open change
            // is not on the stack, so the last edit made would be the one that
            // could not be undone.
            self.history.end();
        }
        match path {
            Some(path) => self.server_paths.set(path),
            None => self.server_paths.forget(which, owner),
        }
        self.server_paths_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// Set what this project says a script is, or stop saying anything about
    /// it. [`Self::set_server_path`] for the other subject whose edit is a
    /// set of rows: `None` takes the claim back, and a script with no rows is
    /// the edit that removes it.
    ///
    /// The undo entry is a [`vale_edit::undo::ServerPath`] under the
    /// script table's name, which is how [`Self::step_server_paths`] tells
    /// the two stores apart.
    pub fn set_server_script(
        &mut self,
        table: &'static str,
        id: u32,
        script: Option<&vale_mangos::scripts::Script>,
        under: Option<Gesture<'_>>,
    ) {
        let before = self.server_scripts.get(table, id);
        let after = script.cloned();
        if before == after {
            return;
        }
        if let Some(gesture) = under {
            self.open_gesture(gesture);
            self.history
                .record_server_path(vale_edit::undo::ServerPath {
                    table: table.to_string(),
                    key: vale_mangos::row::Key::one("id", u64::from(id)).text(),
                    before: before.as_ref().map(vale_mangos::scripts::Script::to_line),
                    after: after.as_ref().map(vale_mangos::scripts::Script::to_line),
                });
            self.history.end();
        }
        match script {
            Some(script) => self.server_scripts.set(script),
            None => self.server_scripts.forget(table, id),
        }
        self.server_scripts_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// Put a change's server-row edits in, or take them out.
    ///
    /// The server-row counterpart of [`Self::step_tables`], and simpler:
    /// there is no file to be open or not, because the store is the session's
    /// own and is always there. What it cannot do is reach the database — an
    /// undone edit is written back to the server by the next save, exactly as
    /// the edit itself was, because the statements a save emits are a diff of
    /// the project rather than a log of what was typed.
    fn step_server(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        // The paths first, and outside the guard in `step_server_cells`. A
        // waypoint edit is a change with no `server_cells` at all, so when
        // this call came after an `is_empty` return on that list, Undo and
        // Redo did nothing to a path edit even though the entry was on the
        // stack and the button was lit. Each list of a change must be stepped
        // independently of the others being empty.
        self.step_server_paths(change, forward);
        // A created or removed row is stepped before its columns going
        // forward, and after them coming back. A change that creates a spawn
        // and then writes one of its columns has to have the row first, and
        // taking the row away has to come after the column — see
        // `vale_edit::undo::Change::server_rows`.
        if forward {
            self.step_server_rows(change, forward);
        }
        self.step_server_cells(change, forward);
        if !forward {
            self.step_server_rows(change, forward);
        }
    }

    /// …and the cells on their own, so [`Self::step_server`] can put the row
    /// claims on either side of them.
    fn step_server_cells(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        if change.server_cells.is_empty() {
            return;
        }
        for cell in &change.server_cells {
            let cell = match forward {
                true => cell.clone(),
                false => cell.reversed(),
            };
            let Some(key) = vale_mangos::row::Key::parse(&cell.key) else {
                // The key is written by `Key::text` and read by `Key::parse`,
                // so this cannot happen from anything this crate wrote. It is
                // logged rather than ignored, because otherwise an undo would
                // do nothing and give no reason.
                warn!("undo: {} is not a row key", cell.key);
                continue;
            };
            self.server_edits
                .set(&cell.table, &key, &cell.column, cell.after.clone());
        }
        self.server_edits_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// …and the same for the whole-row claims, which are a list of their own
    /// on the change for the paths' reason: a column and a claim about whether
    /// the row exists at all are addressed differently.
    fn step_server_rows(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        if change.server_rows.is_empty() {
            return;
        }
        for entry in &change.server_rows {
            let entry = match forward {
                true => entry.clone(),
                false => entry.reversed(),
            };
            let Some(key) = vale_mangos::row::Key::parse(&entry.key) else {
                warn!("undo: {} is not a row key", entry.key);
                continue;
            };
            self.server_edits
                .set_row_line(&entry.table, &key, entry.after.as_deref());
        }
        self.server_edits_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// …and the same for the paths, which are a list of their own on the change.
    ///
    /// Separate because the two are addressed differently — a column and a set
    /// of rows — and folding them would need one of them to pretend to be the
    /// other. See [`vale_edit::undo::ServerPath`].
    fn step_server_paths(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        if change.server_paths.is_empty() {
            return;
        }
        for entry in &change.server_paths {
            let entry = match forward {
                true => entry.clone(),
                false => entry.reversed(),
            };
            // A script's entry carries a script table's name; see
            // [`Self::set_server_script`].
            if let Some(table) = vale_mangos::scripts::table_named(&entry.table) {
                let Some(id) = vale_mangos::row::Key::parse(&entry.key).and_then(|key| key.first())
                else {
                    warn!("undo: {} is not a script key", entry.key);
                    continue;
                };
                let id = id as u32;
                match &entry.after {
                    Some(line) => self
                        .server_scripts
                        .set(&vale_mangos::scripts::Script::from_line(table, id, line)),
                    None => self.server_scripts.forget(table, id),
                }
                self.server_scripts_unsaved = true;
                continue;
            }
            let Some(which) = vale_mangos::path::Which::named(&entry.table) else {
                warn!("undo: {} is not a movement table", entry.table);
                continue;
            };
            let Some(owner) =
                vale_mangos::row::Key::parse(&entry.key).and_then(|key| key.first())
            else {
                warn!("undo: {} is not a path key", entry.key);
                continue;
            };
            match &entry.after {
                Some(line) => self
                    .server_paths
                    .set(&vale_mangos::path::Path::from_line(which, owner, line)),
                None => self.server_paths.forget(which, owner),
            }
        }
        self.server_paths_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// Throw away one subject's server-row edits, and the undo entries
    /// addressed against them. Answers `(rows, paths)`.
    ///
    /// `wanted` is asked each table's name, because the store holds every
    /// subject's rows together: a creature discard that emptied it would take
    /// the item workspace's rows with it and count them as creatures. See
    /// `vale_mangos::row::Edits::forget_where`.
    ///
    /// The entries go for the reason [`History::forget_table`]'s do: an entry's
    /// `before` and `after` are addressed against the store as it was when the
    /// entry was made, and after it is emptied there is nothing for them to put
    /// back — undoing would write a value into a row the project has stopped
    /// claiming.
    pub fn forget_server_edits(
        &mut self,
        mut wanted: impl FnMut(&str) -> bool,
        paths: bool,
    ) -> (usize, usize) {
        let rows = self.server_edits.forget_where(&mut wanted);
        // The paths go with them for the subject that has any, because
        // Discard is one gesture about one subject, and leaving the paths
        // behind would report "edits discarded" while the next save went on
        // writing a `DELETE` and a dozen `INSERT`s. The scripts go by table,
        // as the rows do.
        let mut dropped = match paths {
            true => {
                let had = self.server_paths.len();
                self.server_paths = vale_mangos::path::Paths::default();
                had
            }
            false => 0,
        };
        let scripts: Vec<(&'static str, u32)> = self
            .server_scripts
            .iter()
            .filter(|script| wanted(script.table))
            .map(|script| (script.table, script.id))
            .collect();
        for (table, id) in &scripts {
            self.server_scripts.forget(table, *id);
        }
        dropped += scripts.len();
        // The waypoint entries go with the creature discard by their table,
        // so the script entries, which share the undo's list, stay.
        self.history.forget_server_rows_where(
            |table| wanted(table) || (paths && vale_mangos::path::table_named(table).is_some()),
            false,
        );
        self.server_edits_unsaved = true;
        if paths {
            self.server_paths_unsaved = true;
        }
        if !scripts.is_empty() {
            self.server_scripts_unsaved = true;
        }
        self.server_edit_revision += 1;
        (rows, dropped)
    }

    /// Whether the server half of this project is ahead of its folder.
    ///
    /// The third thing a save writes, beside the tiles and the tables. When the
    /// Save button read only the tile and table lists, it showed "Saved" and
    /// disabled itself while a creature edit was outstanding, so the keystroke
    /// was the only way to write it. After a revert of the rows, the store
    /// still claimed them, `sql\creatures.sql` was still on disk, and nothing
    /// asked for a save.
    ///
    /// Both stores, because both are written by the same press and neither has
    /// a count worth showing — what a person needs to know is that a save is
    /// owed, not how many rows it is.
    pub fn server_unsaved(&self) -> bool {
        self.server_edits_unsaved || self.server_paths_unsaved || self.server_scripts_unsaved
    }

    /// Write the project's scripts to their own file, or remove the file when
    /// there are none left. [`Self::save_server_paths`]' sibling, for the
    /// same reason: a script is written from a different shape.
    pub fn save_server_scripts(&mut self) -> bool {
        let path = crate::server::behaviour::SCRIPTS_VPATH;
        if self.server_scripts.is_empty() {
            if let Some(disk) = self.project.path_for(path) {
                let _ = std::fs::remove_file(disk);
            }
            self.server_scripts_unsaved = false;
            return true;
        }
        let body = self.server_scripts.to_text(&self.project.name);
        match self.project.write(path, body.as_bytes()) {
            Ok(_) => {
                self.server_scripts_unsaved = false;
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

    /// Write the project's row edits to its own file, or remove the file when
    /// there are none left.
    ///
    /// Separate from [`Self::save_all_tables`] because it is a different
    /// container: a DBC is bytes in the overlay and this is a text file nothing
    /// but this editor and a person reads.
    pub fn save_server_edits(&mut self) -> bool {
        let path = crate::server::creatures::EDITS_VPATH;
        if self.server_edits.is_empty() {
            if let Some(disk) = self.project.path_for(path) {
                let _ = std::fs::remove_file(disk);
            }
            self.server_edits_unsaved = false;
            return true;
        }
        let body = self.server_edits.to_text(&self.project.name);
        match self.project.write(path, body.as_bytes()) {
            Ok(_) => {
                self.server_edits_unsaved = false;
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

    /// Write the project's waypoint paths to its own file, or remove the file
    /// when there are none left.
    ///
    /// [`Self::save_server_edits`]' sibling and a second file for the reason
    /// the store is a second store: the two are written from different shapes
    /// and one file holding both could not be read by a person or corrected by
    /// hand, which is the property that makes these text at all.
    pub fn save_server_paths(&mut self) -> bool {
        let path = crate::server::creatures::PATHS_VPATH;
        if self.server_paths.is_empty() {
            if let Some(disk) = self.project.path_for(path) {
                let _ = std::fs::remove_file(disk);
            }
            self.server_paths_unsaved = false;
            return true;
        }
        let body = self.server_paths.to_text(&self.project.name);
        match self.project.write(path, body.as_bytes()) {
            Ok(_) => {
                self.server_paths_unsaved = false;
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

    /// Write every table whose bytes are ahead of the project folder; how
    /// many were written.
    pub fn save_all_tables(&mut self) -> usize {
        let unsaved: Vec<String> = self.unsaved_tables.iter().cloned().collect();
        unsaved.iter().filter(|name| self.save_table(name)).count()
    }

    /// …and write it to the project folder.
    pub fn save_table(&mut self, name: &str) -> bool {
        let Some(table) = self.tables.get(name) else {
            return false;
        };
        let path = vale_assets::tables::dbc::dbc_path(name);
        match self.project.write(&path, &table.write()) {
            Ok(at) => {
                self.publish_table(name);
                self.unsaved_tables.remove(name);
                self.status = format!("saved {}", at.display());
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

    /// Write an open tile's bytes into the overlay, so the next read of its path
    /// answers with them.
    ///
    /// After this, everything that reads the path sees the edit. It does not
    /// touch the disk: [`EditSession::save`] does that.
    pub fn publish(&mut self, coord: (u32, u32)) {
        let Some(tile) = self.tiles.get(&coord) else {
            return;
        };
        let path = self.key(coord).vpath().to_ascii_lowercase();
        let bytes = tile.write();
        if let Ok(mut edited) = self.edited.write() {
            edited.insert(path, bytes);
        }
        self.unsaved.insert(coord);
        *self.revision.entry(coord).or_default() += 1;
        *self.changes.entry(coord).or_default() += 1;
        self.unpublished.remove(&coord);
    }

    /// How many times a tile has changed in memory or been published. See
    /// [`Self::changes`], the field.
    pub fn changes(&self, coord: (u32, u32)) -> u64 {
        self.changes.get(&coord).copied().unwrap_or_default()
    }

    /// Whether an open tile's bytes are ahead of what was last published.
    pub fn is_unpublished(&self, coord: (u32, u32)) -> bool {
        self.unpublished.contains(&coord)
    }

    /// Note an in-memory change to a tile, for [`Self::changes`] and
    /// [`Self::is_unpublished`].
    fn changed(&mut self, coord: (u32, u32)) {
        *self.changes.entry(coord).or_default() += 1;
        self.unpublished.insert(coord);
    }

    /// How many times this tile's bytes have been republished — see
    /// [`Self::revision`], the field, for what reads it and why.
    pub fn revision(&self, coord: (u32, u32)) -> u64 {
        self.revision.get(&coord).copied().unwrap_or_default()
    }

    /// Note that a chunk's heights have moved, so what stands on them can be
    /// caught up: the ground on screen this frame, and its lawn when the stroke
    /// is over.
    pub fn touched(&mut self, coord: (u32, u32), chunk: usize) {
        self.dirty.entry(coord).or_default().insert(chunk);
        self.regrow.entry(coord).or_default().insert(chunk);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// …and for one chunk's shading, which moves a vertex attribute and
    /// nothing else.
    ///
    /// `dirty` and not `regrow`: `MCCV` does not move the ground, so the grass
    /// standing on it is exactly where it was. Marking it would rebuild a
    /// chunk's whole lawn on every shading stroke for no change at all — which
    /// is what calling [`Self::touched`] here would have done, and is the only
    /// reason this is its own method rather than that one.
    pub fn shaded(&mut self, coord: (u32, u32), chunk: usize) {
        self.dirty.entry(coord).or_default().insert(chunk);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// …and the same for one `MDDF` entry, whose drawn copy has to be put where
    /// its record now says.
    pub fn moved(&mut self, coord: (u32, u32), index: usize) {
        self.moved.entry(coord).or_default().insert(index);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// …and the same for one `MODF` entry.
    pub fn moved_building(&mut self, coord: (u32, u32), index: usize) {
        self.moved_buildings.entry(coord).or_default().insert(index);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// …and for one chunk's paint, whose blend maps on the GPU are behind the
    /// file.
    pub fn repainted(&mut self, coord: (u32, u32), chunk: usize) {
        self.repaint.entry(coord).or_default().insert(chunk);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// Write one tile to the project folder, or take it out of the folder
    /// when its bytes are the archives' own. Answers whether the folder now
    /// matches the tile; see [`Self::save_one`].
    pub fn save(&mut self, coord: (u32, u32)) -> bool {
        self.save_one(coord).is_some()
    }

    /// [`Self::save`], answering which of the two it did.
    ///
    /// A tile is marked unsaved by any edit, including one that is undone and
    /// a drag that ends where it began, so a save can hold exactly what the
    /// archives hold. Kept, such a file is listed as a tile the project
    /// changes, goes into every patch archive, and makes a server tile
    /// regeneration rebuild the tile. It is removed from the folder instead,
    /// and a file an earlier save wrote is removed with it, so a tile whose
    /// edits have all been taken back leaves the project.
    ///
    /// The comparison is of bytes, with the archives read past this project's
    /// own archives in `Data\`. `vale_edit::adt::diff`, which the project
    /// dialog lists tiles by, does not compare every chunk, so a tile it calls
    /// the same can still differ.
    fn save_one(&mut self, coord: (u32, u32)) -> Option<Saved> {
        let tile = self.tiles.get(&coord)?;
        let key = self.key(coord);
        let vpath = key.vpath();
        let bytes = tile.write();
        let unchanged = self
            .shipped
            .as_ref()
            .is_some_and(|shipped| shipped.holds(&self.project, &vpath, &bytes));
        let done = match unchanged {
            true => self.project.revert(&vpath).map(|_| {
                self.status = format!("{key} matches the archives, so it is not kept in the project");
                Saved::Dropped
            }),
            false => self.project.write(&vpath, &bytes).map(|path| {
                self.status = format!("saved {}", path.display());
                Saved::Written
            }),
        };
        match done {
            Ok(saved) => {
                self.unsaved.remove(&coord);
                Some(saved)
            }
            Err(e) => {
                self.status = format!("{key}: {e}");
                None
            }
        }
    }

    /// Does the map claim this tile? — `MAIN`'s one bit, off
    /// [`EditSession::claimed`].
    pub fn wdt_claims(&self, coord: (u32, u32)) -> bool {
        self.claimed.contains(&coord)
    }

    /// Note that a tile's bytes are ahead of what is written down.
    ///
    /// The other edits reach this through `touched` and its neighbours, which
    /// also mark the ground or the foliage stale. A shadow rebake changes
    /// neither — nothing on screen is drawn from `MCSH` — so it needs the
    /// unsaved mark on its own.
    pub fn mark_unsaved(&mut self, coord: (u32, u32)) {
        self.unsaved.insert(coord);
    }

    /// Record that a tile has been claimed or unclaimed, beside the file.
    pub fn set_claimed(&mut self, coord: (u32, u32), claimed: bool) {
        match claimed {
            true => self.claimed.insert(coord),
            false => self.claimed.remove(&coord),
        };
    }

    /// …and all of them. Answers how many tiles the folder now matches,
    /// written or removed.
    pub fn save_all(&mut self) -> usize {
        let unsaved: Vec<(u32, u32)> = self.unsaved.iter().copied().collect();
        let (mut written, mut dropped) = (0, 0);
        for coord in unsaved {
            match self.save_one(coord) {
                Some(Saved::Written) => written += 1,
                Some(Saved::Dropped) => dropped += 1,
                None => {}
            }
        }
        let root = self.project.root.display();
        match (written, dropped) {
            (0, 0) => {}
            (written, 0) => self.status = format!("saved {written} tiles to {root}"),
            (0, dropped) => {
                self.status =
                    format!("{dropped} tiles match the archives, so they are not kept in {root}")
            }
            (written, dropped) => {
                self.status = format!(
                    "saved {written} tiles to {root}; {dropped} match the archives, so they are \
                     not kept"
                )
            }
        }
        written + dropped
    }

    /// Pack the project into a patch archive beside the game's own, replacing
    /// the one it published before — see `Project::publish_into`. Answers
    /// where it landed and how many files went in, or `None` when there was
    /// nothing to pack or the write failed; the status line says which.
    pub fn publish_archive(&mut self, data_dir: &str) -> Option<(std::path::PathBuf, usize)> {
        self.save_all();
        match self.project.publish_into(data_dir) {
            Ok(report) => {
                let mut line = match &report.to {
                    Some(to) => format!("{} files written to {}", report.files, to.display()),
                    None => "nothing to publish".to_string(),
                };
                if !report.removed.is_empty() {
                    line.push_str(&format!(
                        "; this project's earlier {} removed",
                        report.removed.join(", ")
                    ));
                }
                for (name, why) in &report.left {
                    warn!("publish: {name} could not be removed: {why}");
                    line.push_str(&format!(
                        "; {name} could not be removed ({why}) and still holds the \
                         project's older files — close the program that has it open and publish again"
                    ));
                }
                self.status = line;
                report.to.map(|to| (to, report.files))
            }
            Err(e) => {
                self.status = format!("publish: {e}");
                None
            }
        }
    }

    /// Undo or redo one change's table half, which is the same three lines
    /// in both directions.
    ///
    /// Separate from the tile loop rather than folded into it because the two
    /// targets are different objects — see `vale_edit::undo::Change`, where
    /// the argument for two lists on one stack is. What a person sees is one
    /// stack: `Ctrl+Z` takes back the last thing done, whether that was a wall
    /// or a spell's name.
    fn step_tables(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        for name in change.tables() {
            let Some(table) = self.tables.get_mut(&name) else {
                // The table is not open. Nothing to put back — and nothing is
                // lost, because opening it again reads the project folder,
                // which is where the change was written.
                continue;
            };
            match forward {
                true => change.apply_table(&name, table),
                false => change.revert_table(&name, table),
            }
            self.table_edited(&name);
        }
    }

    /// Undo the last change, publishing every tile it was about.
    ///
    /// One stack for every tile, which is what an undo key is expected to be:
    /// the last thing done, wherever it was done. A stroke that crossed a tile
    /// border is one entry naming two tiles and comes back off in one press.
    ///
    /// A tile the change names and this session does not have open is opened
    /// again, which is why this takes the archives. Skipping such a tile is
    /// safe only if [`crate::tools::close_tiles`] refuses to close anything
    /// the history names, and that rule does not scale: a find-and-replace
    /// over 687 tiles would pin every one of them in memory for the rest of
    /// the session. Opening on demand puts a change back where it was made,
    /// whether or not the camera has been there since. Opening reads through
    /// the overlay, so what comes back is the edited tile, which is the one
    /// the change applies to.
    pub fn undo(&mut self, assets: &GameAssets) -> Vec<(u32, u32)> {
        let Some(change) = self.history.undo() else {
            return Vec::new();
        };
        self.status = format!("undo {}", change.label);
        self.step_tables(&change, false);
        self.step_server(&change, false);
        let mut touched = Vec::new();
        for key in change.tiles() {
            let coord = (key.x, key.y);
            self.open(assets, coord);
            if let Some(tile) = self.tiles.get_mut(&coord) {
                change.revert(&key, tile);
                self.publish(coord);
                for chunk in change.chunks(&key) {
                    self.touched(coord, chunk);
                }
                // …and the placements, which are neither a chunk nor in one.
                // A change that renumbers the lists marks the whole tile stale
                // instead: no index survives it, so reconciling entry by entry
                // would move the wrong ones.
                match change.renumbers_placements(&key) {
                    true => {
                        self.stale.insert(coord);
                    }
                    false => {
                        for index in change.placements(&key) {
                            self.moved(coord, index);
                        }
                        for index in change.buildings(&key) {
                            self.moved_building(coord, index);
                        }
                    }
                }
                // …and the paint, on the same fork and for the same reason: a
                // change that gave a chunk a texture it did not have cannot be
                // put back by writing into the atlas, because what it changed is
                // the material the draw group was built with.
                match change.changes_the_texture_set(&key) {
                    true => {
                        self.stale.insert(coord);
                    }
                    false => {
                        for chunk in change.painted(&key) {
                            self.repainted(coord, chunk);
                        }
                    }
                }
                // …and the third fork, which has no `false` arm at all: a hole
                // changes how many vertices a chunk contributes, so there is
                // nothing on screen to patch back and the tile is read again.
                if change.remeshes(&key) {
                    self.stale.insert(coord);
                }
                touched.push(coord);
            }
        }
        touched
    }

    /// …and put it back, opening what it names on [`Self::undo`]'s own terms.
    pub fn redo(&mut self, assets: &GameAssets) -> Vec<(u32, u32)> {
        let Some(change) = self.history.redo() else {
            return Vec::new();
        };
        self.status = format!("redo {}", change.label);
        self.step_tables(&change, true);
        self.step_server(&change, true);
        let mut touched = Vec::new();
        for key in change.tiles() {
            let coord = (key.x, key.y);
            self.open(assets, coord);
            if let Some(tile) = self.tiles.get_mut(&coord) {
                change.apply(&key, tile);
                self.publish(coord);
                for chunk in change.chunks(&key) {
                    self.touched(coord, chunk);
                }
                match change.renumbers_placements(&key) {
                    true => {
                        self.stale.insert(coord);
                    }
                    false => {
                        for index in change.placements(&key) {
                            self.moved(coord, index);
                        }
                        for index in change.buildings(&key) {
                            self.moved_building(coord, index);
                        }
                    }
                }
                // …and the paint, on the same fork and for the same reason: a
                // change that gave a chunk a texture it did not have cannot be
                // put back by writing into the atlas, because what it changed is
                // the material the draw group was built with.
                match change.changes_the_texture_set(&key) {
                    true => {
                        self.stale.insert(coord);
                    }
                    false => {
                        for chunk in change.painted(&key) {
                            self.repainted(coord, chunk);
                        }
                    }
                }
                // …and the third fork, which has no `false` arm at all: a hole
                // changes how many vertices a chunk contributes, so there is
                // nothing on screen to patch back and the tile is read again.
                if change.remeshes(&key) {
                    self.stale.insert(coord);
                }
                touched.push(coord);
            }
        }
        touched
    }
}

/// Open the project, install the overlay, and point the world at the map.
pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, open)
            // After the client's own writer, and it never fights it — see
            // [`vale_client::render::focus`], which stands the client's
            // writer down while there is no session. Ordered anyway, because an
            // ordering that matters is stated: without it the two would take
            // turns on the frames a playtest starts and ends.
            .add_systems(
                Update,
                (
                    // Before anything reads the claims, since switching maps
                    // leaves them describing the old one — see [`refresh_claims`].
                    refresh_claims,
                    // …and the tiles on screen, which a project switch makes
                    // stale wholesale. Before the claims are read rather than
                    // after: both are about a folder that has changed.
                    reread_after_a_switch,
                    // As soon as the ground under the camera is open, for
                    // the start position and for every jump after it. Both name
                    // a place on the map and not a height. See
                    // [`drop_to_the_ground`].
                    drop_to_the_ground,
                    // …and the list of places a jump can name, which is a join
                    // over two tables and is rebuilt only when the map changes.
                    follow_the_map,
                    follow_the_camera,
                )
                    .chain()
                    .after(vale_client::world::session::follow_the_session),
            )
            // Before the lab hangs anything. The lab publishes a scratch copy
            // and asks for it in the same frame, so a drain that ran after it
            // would forget the path that had just been asked for and cost a
            // second read of it. Draining first means what is forgotten is
            // what changed before this frame, which covers every other
            // publish. See [`forget_what_changed`].
            //
            // …and the publish before the drain, so an edit that settles on
            // this frame is told about on this frame rather than the next.
            .add_systems(
                Update,
                (publish_for_the_preview, forget_what_changed)
                    .chain()
                    .before(crate::lab::hang_the_effect),
            )
            .add_systems(Update, drop_unchanged_tiles);
    }
}

/// What the archives are read through: the unsaved edits, then the project
/// folder, then the game's own files.
///
/// In memory first so an unsaved edit is what is drawn; the folder before the
/// archives so a project opened in a new session draws what it saved last time
/// without having to load every tile it ever touched. See the module comment.
///
/// A function rather than a closure written once at startup because the
/// project can be changed while the editor is running, and to everything
/// downstream the overlay is the project.
fn overlay_over(project: &Project, edited: &Edited) -> vale_assets::archive::Overlay {
    let edited = Arc::clone(edited);
    let project = project.clone();
    Arc::new(move |path: &str| -> Option<Vec<u8>> {
        let key = path.to_ascii_lowercase();
        if let Ok(map) = edited.read() {
            if let Some(bytes) = map.get(&key) {
                return Some(bytes.clone());
            }
        }
        project.read(path)
    })
}

/// What [`EditSession::save_one`] did with a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Saved {
    /// The tile differs from the archives' copy, and its bytes are in the
    /// project folder.
    Written,
    /// The tile is the archives' copy, and the project folder has no file
    /// for it.
    Dropped,
}

/// The archives' own files, read past this project's archives in `Data\`,
/// for telling a tile that changes something from one that changes nothing.
///
/// Reads through a chain borrowed from the client's pool (`GameAssets::chains`)
/// rather than through `GameAssets`, so the check [`Self::look_for_unchanged`]
/// starts can hold one on a worker. A pool chain has no overlay, so it answers
/// the archives' bytes whatever the project holds.
#[derive(Clone)]
pub struct Shipped {
    chains: Arc<vale_assets::archive::ChainPool>,
    /// The folder the pool is keyed on: `GameAssets::gamedata_dir` as it is
    /// written. A borrow under any other spelling empties the pool, and the
    /// tile loaders would open their chains again.
    gamedata_dir: String,
}

impl Shipped {
    pub fn new(assets: &GameAssets) -> Shipped {
        Shipped {
            chains: assets.chains(),
            gamedata_dir: assets.gamedata_dir.clone(),
        }
    }

    /// Whether `bytes` are, byte for byte, the archives' file at `vpath`.
    ///
    /// False when the archives do not have the file or cannot be read, so a
    /// tile is taken out of a project only on a positive answer.
    pub fn holds(&self, project: &Project, vpath: &str, bytes: &[u8]) -> bool {
        let Ok(mut chain) = self.chains.take(&self.gamedata_dir) else {
            return false;
        };
        chain
            .read_past_overlay_skipping(vpath, &own_archives(project))
            .is_ok_and(|shipped| shipped == bytes)
    }

    /// Start finding, on a worker, every tile file in `project` whose bytes
    /// are the archives' own.
    ///
    /// Save stops a tile like this from being written, but a project saved
    /// before it did may hold some. The check reads each tile the folder
    /// holds and the archives' copy, so its cost grows with the number of
    /// tiles, and it runs off the main thread. [`drop_unchanged_tiles`]
    /// collects the answer and removes the files.
    fn look_for_unchanged(&self, project: &Project) -> Task<Vec<Unchanged>> {
        let shipped = self.clone();
        let project = project.clone();
        AsyncComputeTaskPool::get().spawn(async move {
            let own = own_archives(&project);
            let Ok(mut chain) = shipped.chains.take(&shipped.gamedata_dir) else {
                return Vec::new();
            };
            project
                .files()
                .into_iter()
                .filter_map(|vpath| {
                    let key = TileKey::from_vpath(&vpath)?;
                    let path = project.path_for(&vpath)?;
                    // Read before the bytes, so a save that lands between
                    // the two leaves a record that no longer matches the file
                    // and the file is kept.
                    let meta = std::fs::metadata(&path).ok()?;
                    let bytes = std::fs::read(&path).ok()?;
                    let shipped = chain.read_past_overlay_skipping(&vpath, &own).ok()?;
                    (shipped == bytes).then(|| Unchanged {
                        vpath,
                        key,
                        len: meta.len(),
                        modified: meta.modified().ok(),
                    })
                })
                .collect()
        })
    }
}

/// This project's own archives in `Data\`, which a comparison with the
/// archives reads past. A chain that included them would compare an edit
/// with its own published copy.
fn own_archives(project: &Project) -> Vec<String> {
    project
        .published()
        .into_iter()
        .map(|archive| archive.name)
        .collect()
}

/// A tile file the open-time check found to be the archives' own, and the
/// size and time the file had when it was read.
struct Unchanged {
    vpath: String,
    key: TileKey,
    len: u64,
    modified: Option<std::time::SystemTime>,
}

/// Remove the tile files [`Shipped::look_for_unchanged`] found, once it has
/// finished.
///
/// A file is kept when its tile has unsaved edits, or when its size or time
/// has changed since the check read it: a save landed in between, and the
/// check's answer is about bytes that are no longer there.
fn drop_unchanged_tiles(session: Option<ResMut<EditSession>>) {
    let Some(mut session) = session else { return };
    if session.unchanged.is_none() {
        return;
    }
    let Some(found) = session
        .unchanged
        .as_mut()
        .and_then(|task| block_on(future::poll_once(task)))
    else {
        return;
    };
    session.unchanged = None;
    let mut dropped = 0;
    for one in found {
        let open_here = one.key.map.eq_ignore_ascii_case(&session.map);
        if open_here && session.unsaved.contains(&(one.key.x, one.key.y)) {
            continue;
        }
        let Some(path) = session.project.path_for(&one.vpath) else {
            continue;
        };
        let as_read = std::fs::metadata(&path)
            .is_ok_and(|meta| meta.len() == one.len && meta.modified().ok() == one.modified);
        if as_read && session.project.revert(&one.vpath).is_ok_and(|gone| gone) {
            dropped += 1;
        }
    }
    if dropped > 0 {
        let line = format!(
            "{dropped} tiles in {} match the archives, so they are no longer kept",
            session.project.name
        );
        info!("{line}");
        session.status = line;
    }
}

/// Build the session and hand the archive chain its overlay.
fn open(
    mut commands: Commands,
    args: Res<crate::Args>,
    assets: Res<GameAssets>,
    mut focus: ResMut<WorldFocus>,
    mut live: ResMut<vale_client::render::terrain::LiveEdits>,
    mut clock: ResMut<vale_client::render::sky::WorldClock>,
    // The camera is where the focus comes from from the next frame on, so
    // `--at` has to reach it here. Setting only the focus would put the world at
    // the asked-for place for one frame and then move it to the camera's own
    // origin, which is the middle of the map.
    mut camera: ResMut<crate::camera::EditorCamera>,
) {
    // `--project`, else the one opened last, else the default. The last
    // one is a file under `Edit\`, written here and on every switch — see
    // `vale_edit::project::last_opened`.
    let name = args
        .project
        .clone()
        .or_else(|| vale_edit::project::last_opened(&assets.root))
        .unwrap_or_else(|| vale_edit::project::DEFAULT.to_string());
    let project = match Project::open(&assets.root, &name) {
        Ok(project) => project,
        Err(e) => {
            error!("could not open the project {name}: {e}");
            return;
        }
    };
    vale_edit::project::remember_opened(&assets.root, &name);

    let edited: Edited = Arc::new(RwLock::new(HashMap::default()));
    assets.set_overlay(Some(overlay_over(&project, &edited)));

    let maps = map_directories(&assets);
    let map_id = maps
        .iter()
        .find(|(_, name)| name.eq_ignore_ascii_case(&args.map))
        .map(|(id, _)| *id)
        .unwrap_or(0);

    // A tile is built so its ground can be rewritten after it is on the GPU,
    // and this has to be set before any tile is streamed because it decides how
    // each one is built: the meshes stay in the main world so a stroke can write
    // to them, and each draw group remembers where its vertices came from. See
    // `vale_client::render::terrain::LiveEdits`.
    live.0 = true;
    // The world is lit by hand, because `render::sky::resolve` declines to
    // resolve at all while neither the server nor a person has said what time it
    // is, and only a server sends the clock. The same override `--hour` uses.
    //
    // The game's interface and the login screen's own 3D scene are switched
    // by [`crate::playtest::arrange`], not here. They are off while the editor
    // is editing and on while it is being played, which is one state and not
    // two defaults.
    let noon = 12 * 120;
    *clock = vale_client::render::sky::WorldClock {
        half_minutes: noon,
        from_server: false,
        override_half_minutes: Some(noon),
    };

    // What the map claims, from the project's own copy of the WDT if it has
    // one and the archives' otherwise — so a tile made in a previous session is
    // there when this one opens. See `EditSession::claimed`.
    let claimed = claimed_tiles(&assets, &project, &args.map);
    // …and what it changes on the server, which is a file of the project's own
    // because no client file carries a `creature_template` row.
    let server_edits = read_server_edits(&project);
    let server_paths = read_server_paths(&project);
    let server_scripts = read_server_scripts(&project);
    // …and which of its tiles are the archives' own, which it should not keep.
    // See [`Shipped::look_for_unchanged`].
    let shipped = Shipped::new(&assets);
    let unchanged = Some(shipped.look_for_unchanged(&project));

    let at = args.at.unwrap_or((0.0, 0.0));
    camera.go_to(Vec2::new(at.0, at.1));
    // …and `--view`, which is how far back and at what angle. Written after
    // `go_to` because that keeps the framing and only moves the focus.
    if let Some((distance, pitch, yaw)) = args.view {
        camera.distance = distance.max(1.0);
        if let Some(pitch) = pitch {
            camera.pitch = pitch.to_radians();
        }
        if let Some(yaw) = yaw {
            camera.yaw = yaw.to_radians();
        }
    }
    *focus = WorldFocus::at(map_id, args.map.clone(), camera.target);

    commands.insert_resource(EditSession {
        status: format!("{} in {}", args.map, project.root.display()),
        project,
        map: args.map.clone(),
        map_id,
        maps,
        tiles: HashMap::default(),
        tables: std::collections::HashMap::default(),
        history: History::new(),
        dirty: HashMap::default(),
        regrow: HashMap::default(),
        moved: HashMap::default(),
        moved_buildings: HashMap::default(),
        revision: HashMap::default(),
        changes: HashMap::default(),
        unpublished: HashSet::default(),
        repaint: HashMap::default(),
        stale: HashSet::default(),
        unsaved: HashSet::default(),
        unsaved_tables: HashSet::default(),
        table_revision: 0,
        restream: HashSet::default(),
        republished: Vec::new(),
        republished_all: false,
        tables_republished: false,
        republished_revision: 0,
        claimed,
        reread_everything: false,
        claims_for: args.map.clone(),
        edited,
        server_edits,
        server_paths,
        server_scripts,
        server_edit_revision: 0,
        server_edits_unsaved: false,
        server_paths_unsaved: false,
        server_scripts_unsaved: false,
        applied_signature: None,
        applied_creatures: None,
        applied_items: None,
        applied_quests: None,
        applied_gameobjects: None,
        applied_loot: None,
        applied_behaviour: None,
        applied_services: None,
        applied_places: [None; 5],
        database_writes: 0,
        one_gesture: None,
        shipped: Some(shipped),
        unchanged,
        locks: HashMap::default(),
        placement_locks: None,
    });
}

/// Tell every cache in the client what has changed under it.
///
/// Publishing bytes into the overlay is half of a change. The other half is
/// that the overlay is asked only when a path is read, and the client has four
/// banks that read a path once and keep what they read:
///
/// ```text
/// ModelCache             every model, dressing and skin, by archive path
/// GameAssets             the DBC banks, the strings, the bindings, the sounds
/// DisplayCache           an Arc of the parsed display tables, and what it
///                        resolved from them
/// UiTextures, Art        the two interface painters' minimap pictures, by
///                        the path the index names
/// ```
///
/// Each needs to be told. The third is easy to miss behind the second:
/// `GameAssets::forget_tables` drops the bank's copy, and `DisplayCache` goes
/// on holding the `Arc` it took at the first entity. Without the fourth, a
/// redrawn minimap reaches only the first playtest: the picture is read then
/// and answered from the cache at every playtest after.
///
/// None of this rebuilds what is drawn. What is standing keeps its
/// handles; the next thing to ask reads the file. For the lab that is the
/// next hang, for the storyboard the next loop of the timeline, and for a
/// playtest the login.
#[allow(clippy::too_many_arguments)]
pub fn forget_what_changed(
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    models: Option<ResMut<vale_client::render::models::ModelCache>>,
    displays: Option<ResMut<vale_client::world::entities::DisplayCache>>,
    mut stage: ResMut<crate::stage::Stage>,
    mut ui_textures: Option<ResMut<vale_client::ui::mesh::textures::UiTextures>>,
    mut art: Option<ResMut<vale_client::ui::framexml::Art>>,
) {
    let Some(mut session) = session else { return };

    // The minimap pictures first, since they are the one bank here that
    // is not the model cache and the paths are about to be drained.
    if session.republished_all {
        if let Some(textures) = ui_textures.as_mut() {
            textures.forget_all_minimaps();
        }
        if let Some(art) = art.as_mut() {
            art.forget_all_minimaps();
        }
    } else {
        for path in &session.republished {
            if let Some(textures) = ui_textures.as_mut() {
                textures.forget_minimap(path);
            }
            if let Some(art) = art.as_mut() {
                art.forget_minimap(path);
            }
        }
    }

    // The tables first, because a display the models are then asked for
    // is resolved through them.
    if std::mem::take(&mut session.tables_republished) {
        let began = std::time::Instant::now();
        assets.forget_tables();
        // Read again here, and not on the next entity that needs a display.
        // The stage's actors are already standing when a kit is edited
        // beside its preview, so there is no next entity: the effect and
        // pose passes would find no tables from then on, and the actors
        // would stand idle with nothing hung on them.
        //
        // The re-parse is 73 reads with `Spell.dbc` among them twice, about
        // a tenth of a second. Doing it in this call puts it where the
        // person has just made an edit, and gives it somewhere to be
        // measured.
        if let Some(mut displays) = displays {
            displays.reread(&assets);
        }
        // The lab's compare keys on this revision, so without the bump the
        // model view keeps the effect it hung off the tables from before.
        session.republished_revision += 1;
        // What is standing is shown the edit now. The effect pass hangs a
        // kit when a counter changes, so a playing stage starts its loop
        // again, and a paused one is put back at its own time, which moves
        // the counters of its phase again and leaves it paused.
        if stage.showing.is_some() || stage.pushing.is_some() {
            match stage.playing {
                true => stage.restart(),
                false => {
                    let at = stage.at;
                    stage.seek(at);
                }
            }
        }
        let ok = assets.display_tables().is_ok();
        info!(
            "tables re-read in {:?}{}",
            began.elapsed(),
            match ok {
                true => "",
                false => " (they did not parse)",
            }
        );
    }

    let Some(mut models) = models else { return };
    if session.republished_all {
        session.republished_all = false;
        session.republished.clear();
        session.republished_revision += 1;
        models.forget_all();
        return;
    }
    if session.republished.is_empty() {
        return;
    }
    session.republished_revision += 1;
    for path in std::mem::take(&mut session.republished) {
        models.forget(&path);
    }
}

/// Put the edited tables where the client reads them, while a preview is
/// open.
///
/// The storyboard's preview is the client's own passes (see `crate::stage`),
/// and those read `DisplayCache` — the client's parse of the archives — not
/// the editor's open `DbcFile`s. Without this, a kit edited on the form
/// changes the rail, which reads the edit, and not the picture, which does
/// not; a baked model then never appears in the storyboard.
///
/// Only while something is being previewed, and only once it has settled.
/// A field edit arrives on every frame of a drag and the re-read is not
/// cheap; [`SETTLE`] after the last one is one re-read per thing a person
/// does. With no preview open nothing here runs at all, which is every other
/// tool in the editor.
pub fn publish_for_the_preview(
    time: Res<Time>,
    stage: Res<crate::stage::Stage>,
    tool: Res<crate::tools::Tool>,
    session: Option<ResMut<EditSession>>,
    // The revision last seen, when it changed, the revision last published,
    // and whether the stage was open last frame. The third makes this run
    // once per change rather than once per settle: publishing does not
    // clear `unsaved_tables` — only saving does — so a re-arm on the clock
    // alone would put the tables where the client reads them every settle for
    // as long as the preview was open.
    mut seen: Local<(u64, f64, u64, bool)>,
) {
    /// How long a run of edits is left to settle before the tables are put
    /// where the client reads them. A field edit arrives on every frame of a
    /// drag and the re-read is about a tenth of a second, so it is not done
    /// per frame. It is kept short because a choice in a picker or a typed
    /// number is one edit, and the person is looking at the preview for it.
    const SETTLE: f64 = 0.15;

    let Some(mut session) = session else { return };
    let (had, changed_at, published, was_open) = &mut *seen;
    // Two things count as a preview being open.
    //
    // The stage is one: a spell played on two actors, which reads the tables
    // when it builds a loop. The lights tool is the other, and its preview is
    // the viewport itself — the world is drawn by the client's own passes
    // reading the client's own light chain, so an edited band shows in the
    // terrain, the fog and the sky with nothing else to open. Without this the
    // publish never ran for it and a colour edit reached nothing: the table
    // changed in the session and `GameAssets` went on holding the parse it
    // made at startup.
    let previewing = stage.is_open() || *tool == crate::tools::Tool::Lights;
    if !previewing {
        // Nothing is being previewed. The next thing that needs the tables is
        // a playtest, which publishes for itself.
        *was_open = false;
        return;
    }
    // The frame a preview opens publishes at once. Most edits are made
    // with the stage shut — a kit's form closes it — and the stage's first
    // loop begins on the frame it opens, so a publish that waited for the
    // settle hung the first cast off the tables from before the edit and the
    // person saw the change one loop later, or never on a stage that was
    // paused. The preview then showed an edit only after the person
    // navigated away and back.
    let opened = !*was_open;
    *was_open = true;
    let now = time.elapsed_secs_f64();
    let revision = session.table_revision;
    if *had != revision {
        *had = revision;
        *changed_at = now;
    }
    if *published == revision || (!opened && now - *changed_at < SETTLE) {
        return;
    }
    *published = revision;
    let unsaved: Vec<String> = session.unsaved_tables.iter().cloned().collect();
    if unsaved.is_empty() {
        return;
    }
    for name in &unsaved {
        session.publish_table(name);
    }
    session.tables_republished = true;
    session.status = match unsaved.len() {
        1 => format!("{} written to the overlay for the preview", unsaved[0]),
        n => format!("{n} tables written to the overlay for the preview"),
    };
}

/// Read every drawn tile again after the project changed.
///
/// The ground on screen was built from bytes the old project's overlay
/// answered with, and nothing about it is wrong in a way a patch could fix — a
/// different project is a different file. So every tile that is drawn is made
/// stale, which is the same route a texture edit takes: the streamer reads it
/// again and `terrain::swap` puts the new one up in one frame, leaving the old
/// one on screen until then.
pub fn reread_after_a_switch(
    session: Option<ResMut<EditSession>>,
    tiles: Query<&vale_client::render::terrain::TerrainTile>,
) {
    let Some(mut session) = session else { return };
    if !session.reread_everything {
        return;
    }
    session.reread_everything = false;
    let drawn: Vec<(u32, u32)> = tiles.iter().map(|tile| tile.coord).collect();
    for coord in drawn {
        session.stale.insert(coord);
    }
}

/// Put [`EditSession::claimed`] back in step when the map changes.
///
/// A comparison rather than an event, so it cannot be forgotten by whatever
/// changes the map next: the drop-down does not have the archives to hand and
/// should not have to.
pub fn refresh_claims(
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    mut view: ResMut<crate::ui::mapview::MapView>,
) {
    let Some(mut session) = session else { return };
    if session.claims_for == session.map {
        return;
    }
    let map = session.map.clone();
    session.claimed = claimed_tiles(&assets, &session.project, &map);
    session.claims_for = map;
    // And nothing stays selected. A tile coordinate names a different place
    // on every map, so carrying a selection across would point the next Delete
    // at whatever happens to sit at those coordinates now.
    view.selection.clear();
    view.clipboard.clear();
}

/// Read the project's row edits, or nothing.
///
/// A project with no server edits has no file, which is the ordinary case and
/// is not an error. A file that will not decode as text is reported and treated
/// as empty rather than failing the open: the alternative is a project that
/// cannot be opened at all because of a file no other tool reads.
fn read_server_edits(project: &vale_edit::project::Project) -> vale_mangos::row::Edits {
    let Some(bytes) = project.read(crate::server::creatures::EDITS_VPATH) else {
        return vale_mangos::row::Edits::default();
    };
    match String::from_utf8(bytes) {
        Ok(text) => {
            // A line that could not be read is said, one warning each: it
            // is an edit the project no longer carries, and the next save
            // rewrites the file without it.
            let (edits, refused) = vale_mangos::row::Edits::read(&text);
            for line in refused {
                warn!(
                    "{}: not read, because its key or value is not a SQL literal: {line}",
                    crate::server::creatures::EDITS_VPATH
                );
            }
            edits
        }
        Err(e) => {
            warn!("{}: {e}", crate::server::creatures::EDITS_VPATH);
            vale_mangos::row::Edits::default()
        }
    }
}

/// The scripts the project changes, read out of its own file, or none.
fn read_server_scripts(project: &vale_edit::project::Project) -> vale_mangos::scripts::Scripts {
    let Some(bytes) = project.read(crate::server::behaviour::SCRIPTS_VPATH) else {
        return vale_mangos::scripts::Scripts::default();
    };
    match String::from_utf8(bytes) {
        Ok(text) => vale_mangos::scripts::Scripts::from_text(&text),
        Err(e) => {
            warn!("{}: {e}", crate::server::behaviour::SCRIPTS_VPATH);
            vale_mangos::scripts::Scripts::default()
        }
    }
}

/// Read the project's waypoint paths, or nothing.
///
/// [`read_server_edits`]' sibling, on the same terms: no file is the ordinary
/// case, and a file that will not decode is reported and treated as empty
/// rather than failing the open.
fn read_server_paths(project: &vale_edit::project::Project) -> vale_mangos::path::Paths {
    let Some(bytes) = project.read(crate::server::creatures::PATHS_VPATH) else {
        return vale_mangos::path::Paths::default();
    };
    match String::from_utf8(bytes) {
        Ok(text) => vale_mangos::path::Paths::from_text(&text),
        Err(e) => {
            warn!("{}: {e}", crate::server::creatures::PATHS_VPATH);
            vale_mangos::path::Paths::default()
        }
    }
}

/// Which tiles a map's WDT claims, the project's copy first.
fn claimed_tiles(
    assets: &GameAssets,
    project: &vale_edit::project::Project,
    map: &str,
) -> HashSet<(u32, u32)> {
    let path = vale_edit::wdt::wdt_path(map);
    let bytes = project.read(&path).or_else(|| {
        assets
            .with_archive(|chain| Ok(chain.read(&path).ok()))
            .ok()
            .flatten()
    });
    let Some(bytes) = bytes else {
        error!("{path}: not found, so no tile is known to exist");
        return HashSet::default();
    };
    match vale_edit::wdt::WdtFile::parse(&bytes) {
        Ok(wdt) => wdt.tiles().into_iter().collect(),
        Err(e) => {
            error!("{path}: {e}");
            HashSet::default()
        }
    }
}

/// `Map.dbc`'s id-to-directory table, in [`order_maps`]'s order.
///
/// Read here rather than hardcoded for the reason
/// `vale_assets::tables::dbc::map_directories` gives: a custom map is a row
/// in that table and nothing else, so reading it means a map somebody added
/// appears in the editor's list without a line of code.
pub(crate) fn map_directories(assets: &GameAssets) -> Vec<(u32, String)> {
    let table = assets.with_archive(|chain| {
        let bytes = chain
            .read(&vale_assets::tables::dbc::dbc_path("Map"))
            .map_err(|e| e.to_string())?;
        vale_assets::tables::dbc::map_directories(&bytes).map_err(|e| e.to_string())
    });
    let mut maps: Vec<(u32, String)> = match table {
        Ok(table) => table.into_iter().collect(),
        Err(e) => {
            error!("Map.dbc: {e}");
            Vec::new()
        }
    };
    order_maps(&mut maps);
    maps
}

/// The order every map list in the editor is shown in: by id, lowest first.
pub(crate) fn order_maps(maps: &mut [(u32, String)]) {
    maps.sort_by_key(|&(id, _)| id);
}

/// Drop the camera onto the ground it is over, once the ground arrives.
///
/// A start position, and every jump, is two numbers: the third has to come from
/// the terrain, which is not parsed until several frames after the camera gets
/// there. Until then the focus keeps whatever height it had, which after a jump
/// across a continent is as likely to be inside a mountain as above it.
///
/// It answers once per request rather than every frame — see
/// [`crate::camera::EditorCamera::wants_the_ground`] — so flying below the ground
/// afterwards is allowed.
fn drop_to_the_ground(
    session: Option<Res<EditSession>>,
    mut camera: ResMut<crate::camera::EditorCamera>,
) {
    if !camera.wants_the_ground {
        return;
    }
    let Some(session) = session else { return };
    let coord = vale_assets::tile_for_position(camera.target.x, camera.target.y);
    let Some(tile) = session.tiles.get(&coord) else {
        return;
    };
    if let Some(ground) =
        vale_edit::adt::heights::height_at(tile, camera.target.x, camera.target.y)
    {
        camera.target.z = ground;
        camera.wants_the_ground = false;
    }
}

/// Keep [`crate::places::Places`] in step with the map being edited.
///
/// The join it holds is over two parsed DBCs and a few hundred rows, which is
/// nothing to do once and not something to do every frame. The map id decides
/// when to rebuild: it changes when the panel's drop-down does and at no other
/// time.
fn follow_the_map(
    session: Option<Res<EditSession>>,
    assets: Res<GameAssets>,
    mut places: ResMut<crate::places::Places>,
) {
    let Some(session) = session else { return };
    if places.map_id == session.map_id && !places.zones.is_empty() {
        return;
    }
    *places = crate::places::Places::of(&assets, session.map_id);
}

/// Point the streaming passes at the editor's camera while there is no session.
///
/// The client's own writer owns the focus whenever a character is logged in, so
/// this is a no-op during a playtest — which is the behaviour that is wanted:
/// what streams while there is a character is what the character can see.
/// It reads [`crate::camera::EditorCamera`] and not `CameraRig`. The rig has
/// three writers — `world::camera::orbit`, `world::camera::collide` and the
/// editor's own `camera::drive` — and nothing orders this against any of them,
/// so reading it would take whichever value the frame happened to leave there.
/// The editor's camera has one writer, which is what makes the ordering
/// unnecessary rather than merely unstated.
fn follow_the_camera(
    session: Res<vale_client::world::session::Session>,
    camera: Res<crate::camera::EditorCamera>,
    editing: Option<Res<EditSession>>,
    state: Res<crate::playtest::Playtest>,
    mut focus: ResMut<WorldFocus>,
) {
    // Not while a playtest is logging in either, which is the case
    // `session.active` alone cannot see. `crate::playtest::stand_the_world_down`
    // takes the editor's nine tiles off the screen for the login screen, and a
    // focus still pointing at the editor's camera would stream all nine of them
    // straight back in behind the backdrop.
    if session.active.is_some() || !state.editing() {
        return;
    }
    let Some(editing) = editing else {
        return;
    };
    let at = camera.target;
    // Guarded on the value having moved, because `ResMut`'s `DerefMut` marks the
    // resource changed whether or not it did, and the light chain keys on it.
    let wanted = WorldFocus::at(editing.map_id, editing.map.clone(), at);
    if !focus.same_place(&wanted, 0.05) || focus.map_name != wanted.map_name {
        *focus = wanted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The map's locked placements read back as written, are saved to the
    /// project as they change, and the file goes when the last one is
    /// unlocked.
    #[test]
    fn locked_placements_are_stored_in_the_project() {
        use crate::tools::place::Kind;
        let text = "# comment
doodad 7
wmo 12

doodad 3
";
        let locks = PlacementLocks::from_text(text).unwrap();
        assert_eq!(locks.doodads.iter().copied().collect::<Vec<_>>(), [3, 7]);
        assert_eq!(locks.wmos.iter().copied().collect::<Vec<_>>(), [12]);
        assert_eq!(PlacementLocks::from_text(&locks.to_text()).unwrap(), locks);
        assert!(PlacementLocks::from_text("tree 4").unwrap_err().starts_with("line 1:"));

        let install =
            std::env::temp_dir().join(format!("vale-session-placement-locks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        assert_eq!(session.set_placements_locked(Kind::Doodad, [5, 6], true), 2);
        assert_eq!(session.set_placements_locked(Kind::Doodad, [6], true), 0, "already locked");
        assert!(session.placement_locked(Kind::Doodad, 5));
        assert!(!session.placement_locked(Kind::Wmo, 5), "a doodad id is not a WMO id");
        let vpath = session.placement_locks_vpath();
        assert!(session.project.has(&vpath));
        assert_eq!(session.set_placements_locked(Kind::Doodad, [5, 6], false), 2);
        assert!(!session.project.has(&vpath), "no locks, no file");
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A table a preview published is taken back out of the overlay, and the
    /// client is told to read its tables again. Discard depends on this: it
    /// reads the table again, and the overlay answers before the project
    /// folder and the archives.
    #[test]
    fn a_published_table_is_taken_back_out_of_the_overlay() {
        let install =
            std::env::temp_dir().join(format!("vale-session-unpublish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        let mut bytes = b"WDBC".to_vec();
        for word in [0u32, 35, 140, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0);
        let table = DbcFile::parse(&bytes).expect("an empty table");
        session.tables.insert("SpellVisualKit".to_string(), table);
        let path = vale_assets::tables::dbc::dbc_path("SpellVisualKit").to_ascii_lowercase();

        session.publish_table("SpellVisualKit");
        assert!(session.edited.read().unwrap().contains_key(&path));
        session.tables_republished = false;
        session.unpublish_table("SpellVisualKit");
        assert!(!session.edited.read().unwrap().contains_key(&path));
        assert!(session.tables_republished);

        // A table that was never published asks for no re-read.
        session.tables_republished = false;
        session.unpublish_table("SpellVisualKit");
        assert!(!session.tables_republished);
        let _ = std::fs::remove_dir_all(&install);
    }
}
