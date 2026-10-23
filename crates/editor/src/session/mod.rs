//! What is open: the project, the tiles, the history, and the overlay the
//! archives are read through.
//!
//! ```text
//! locks.rs      locked vertices and locked placements
//! tiles.rs      the open tiles, map switches, and the sets a tool feeds
//! project.rs    switching and clearing the project the session writes to
//! tables.rs     the open client tables, in the overlay and in the project
//! files.rs      any other file of the project, in the overlay and the folder
//! save.rs       saving tiles, and packing the project into an archive
//! history.rs    undo and redo
//! originals.rs  the archives' own copies, and project files that match them
//! open.rs       startup: the session built and the overlay installed
//! republish.rs  the client's caches told what changed, and the preview's
//!               tables republished
//! ```
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
//!   [`crate::tools::ground::terrain::live_foliage`] drains this once, when the button
//!   comes up.
//! * Moved — the placements on it are behind their `MDDF` records, per
//!   entry. Nothing about a doodad is in the ground's mesh, so this is its own
//!   set and [`crate::tools::placements::doodads::reconcile`] drains it.
//! * Repaint — the blend maps on the GPU are behind `MCAL`, per chunk.
//!   The paint is in a texture rather than in a mesh or a transform, so
//!   neither of the two sets above reaches it.
//!   [`crate::tools::ground::textures::live_paint`] drains it.
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

use vale_edit::adt::AdtFile;
use vale_edit::dbc::DbcFile;
use vale_edit::project::Project;
use vale_edit::undo::History;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::Task;
use std::sync::{Arc, RwLock};

pub(crate) mod locks;
pub(crate) mod tiles;
pub(crate) mod project;
pub(crate) mod tables;
pub(crate) mod files;
pub(crate) mod save;
pub(crate) mod history;
pub(crate) mod originals;
pub(crate) mod open;
pub(crate) mod republish;

use locks::PlacementLocks;
use tiles::refresh_claims;
use project::reread_after_a_switch;
use originals::{Shipped, Unchanged, drop_unchanged_tiles};
use open::open;
use republish::{forget_what_changed, publish_for_the_preview, show_the_edit};

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
pub(crate) struct EditSession {
    pub(crate) project: Project,
    /// The map being edited, by its `World\Maps\` directory name.
    pub(crate) map: String,
    /// …and its id, which is what the light chain and the collision world are
    /// keyed by.
    pub(crate) map_id: u32,
    /// Every map the archives name, so a panel can offer them. Sorted by name.
    pub(crate) maps: Vec<(u32, String)>,
    /// Tiles parsed for editing.
    pub(crate) tiles: HashMap<(u32, u32), AdtFile>,
    /// …and client tables, by their bare name (`"Spell"`).
    ///
    /// The same arrangement as `tiles` and for the same reasons: opened on
    /// demand, edited in memory, written to the project folder and into the
    /// overlay. There is no 3x3 to stream, so a table stays open for the
    /// session once something has asked for it.
    pub(crate) tables: std::collections::HashMap<String, DbcFile>,
    pub(crate) history: History,
    /// The chunks whose ground on screen is behind their heights, per tile.
    ///
    /// Cleared by [`crate::tools::live_ground`], which pushes the new vertices
    /// straight into the meshes already on the GPU.
    pub(crate) dirty: HashMap<(u32, u32), HashSet<usize>>,
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
    /// Cleared by [`crate::tools::ground::terrain::live_foliage`].
    pub(crate) regrow: HashMap<(u32, u32), HashSet<usize>>,
    /// The `MDDF` entries whose drawn placement is behind their record, per
    /// tile.
    ///
    /// A third set on the same argument as the second: a placement is not in the
    /// ground's mesh and is not a chunk, so neither of the other two names it.
    /// Cleared by [`crate::tools::placements::doodads::reconcile`], every frame of a drag —
    /// which it can afford to be, because moving a placement is writing one
    /// transform where moving the ground is writing a buffer.
    pub(crate) moved: HashMap<(u32, u32), HashSet<usize>>,
    /// …and the `MODF` entries, which is a separate set because the two
    /// lists are numbered separately: `MDDF` entry 3 and `MODF` entry 3 are
    /// different objects, and one set of indices shared between them moves the
    /// wrong thing. Cleared by [`crate::tools::placements::wmos::reconcile`].
    pub(crate) moved_buildings: HashMap<(u32, u32), HashSet<usize>>,
    /// The chunks whose blend maps on the GPU are behind their `MCAL`, per
    /// tile.
    ///
    /// A fourth set on the same argument as the third. What is behind here is a
    /// texture rather than a mesh or a transform, and the fix is a write into the
    /// tile's alpha atlas — see [`crate::tools::ground::textures::live_paint`], which
    /// drains it every frame of a stroke because one chunk's cell is 16 KB
    /// against the ground's 81,840 vertices.
    ///
    /// A chunk whose set of textures changed is here as well, for its new
    /// cell, and its tile is in `regroup`: the set is part of the material a
    /// draw group was built with, so the chunk has to move to another group.
    pub(crate) repaint: HashMap<(u32, u32), HashSet<usize>>,
    /// Tiles whose ground has to be cut into draw groups again because a chunk
    /// carries a different set of textures: done in place, replacing only the
    /// groups that changed, by [`crate::tools::ground::textures::live_regroup`]. A tile
    /// that cannot be done that way is moved to `stale`.
    pub(crate) regroup: HashSet<(u32, u32)>,
    /// Tiles that need the whole tile read again rather than a vertex patch:
    /// what the live path could not do, and anything that changes more than
    /// heights.
    pub(crate) stale: HashSet<(u32, u32)>,
    /// Tiles whose bytes are ahead of the project folder.
    pub(crate) unsaved: HashSet<(u32, u32)>,
    /// Tables whose bytes are ahead of the project folder, by name.
    pub(crate) unsaved_tables: HashSet<String>,
    /// How many field edits have landed, for anything that caches a reading
    /// of a table rather than the table itself.
    ///
    /// The storyboard is the one reader: it walks four tables and asks the
    /// archives about every model, which is not a per-frame cost. A counter
    /// rather than a per-table flag because what it answers is "is what I built
    /// still true", and an edit to any table in a chain can make it false.
    pub(crate) table_revision: u64,
    /// Which tiles this map claims — the WDT's `MAIN` grid.
    ///
    /// Held here rather than asked for each time because it is the answer to a
    /// question the map window asks for all 4,096 cells every frame, and the
    /// file it comes from is 32 KB. `crate::map::tiles` changes it at the same
    /// time it writes the file.
    pub(crate) claimed: HashSet<(u32, u32)>,
    /// Tiles whose existence changed, for the streamer to be told about.
    ///
    /// Made and unmade ground both land here. See
    /// `crate::map::tiles::restream`, which is what acts on it, and the note
    /// there on why the streamer has to be told at all rather than noticing.
    pub(crate) restream: HashSet<(u32, u32)>,
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
    pub(crate) republished: Vec<String>,
    /// Whether every path has changed under the overlay, which is what a
    /// project switch does: every path the overlay answers now comes out of a
    /// different folder.
    pub(crate) republished_all: bool,
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
    pub(crate) tables_republished: bool,
    /// How many times a file has been forgotten, for anything holding a
    /// picture of a file rather than the file.
    ///
    /// The file counterpart of `table_revision`. The lab is the one reader: it
    /// compares what it has hanging against what it wants, and a re-bake over
    /// the same path has the same path. Without this counter the preview goes
    /// on showing the copy it hung the first time, even after the model cache
    /// has forgotten the path. Bumped by [`forget_what_changed`], which runs
    /// before the hang.
    pub(crate) republished_revision: u64,
    /// Moved by every change to the project's own files that a picker lists:
    /// a save or removal through [`Self::save_bytes`] or
    /// [`Self::remove_file`], and a switch of project. A list built from
    /// [`Self::listed`] is built again when it moves. See
    /// `crate::import::assets`.
    pub(crate) files_revision: u64,
    /// Everything drawn is from a project that is no longer open.
    ///
    /// Set by [`Self::switch_to`] and drained by [`reread_after_a_switch`],
    /// which is where the reason for the indirection is: the session has no
    /// list of what is on screen, and the tiles that are drawn are the
    /// streamer's business rather than the session's.
    pub(crate) reread_everything: bool,
    /// Which map [`Self::claimed`] is of.
    ///
    /// Without it the grid is the first map's for the whole session: switching
    /// maps clears the open tiles but the claims were read once at startup, so
    /// the map window drew Azeroth's claims over Development's empty grid. A
    /// cell claimed with no picture behind it looks the same as a real tile
    /// that has no minimap, so the fault was not visible as an error.
    /// [`refresh_claims`] puts it back in step for every route that changes
    /// the map, not only the drop-down.
    pub(crate) claims_for: String,
    /// The last thing worth telling the person at the keyboard.
    pub(crate) status: String,
    /// How many times each tile's bytes have been republished.
    ///
    /// A counter and not a flag, and it exists for one reader. A tile's
    /// replacement is a read of the overlay that starts on one frame and lands
    /// several frames later, and anything published in between is a change that
    /// read cannot have seen. The copy that arrives is then built from the older
    /// bytes and nothing corrects it: the live sets above were drained against
    /// the outgoing copy, which is despawned when the incoming one appears.
    ///
    /// So the number is what `crate::tools::ground::terrain::remesh` compares. It
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
    /// The rows this project changes in the server's database: the row
    /// edits, waypoint paths and scripts, and whether each is ahead of the
    /// project folder. See [`crate::server::store::ServerRows`].
    pub(crate) rows: crate::server::store::ServerRows,
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
    pub(crate) applied_signature: Option<u64>,
    /// …and the same for each row subject, indexed by
    /// [`crate::server::stack::Subject::index`]: what the last Apply of that
    /// subject that succeeded in this process put in the database, as
    /// `crate::server::subject::Plan::signature` measures it.
    ///
    /// One slot per subject, apart from [`Self::applied_signature`], because
    /// each is applied by its own gesture at its own moment, so one number
    /// would report another subject's state on this one's block. The panel
    /// uses it to choose between "applied" and "applied, and changed since",
    /// two states that look identical in a revert file.
    ///
    /// Per process, for [`Self::applied_signature`]'s reason. `None` at the
    /// start of a session means the panel falls back to what the revert file
    /// says, which is that the rows in it were applied — because what happened
    /// in an earlier session is not this editor's to know beyond that.
    pub(crate) applied: [Option<u64>; crate::server::stack::Subject::COUNT],
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
    /// Not [`Self::applied`], which cannot tell a
    /// put-back from nothing in a session that did not make the apply.
    pub(crate) database_writes: u64,
    /// The archives' own copy of a file, which [`Self::save`] compares a
    /// tile against. `None` in a test session, which has no archives; every
    /// tile is then written.
    pub(crate) shipped: Option<Shipped>,
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
            regroup: HashSet::default(),
            stale: HashSet::default(),
            unsaved: HashSet::default(),
            unsaved_tables: HashSet::default(),
            table_revision: 0,
            restream: HashSet::default(),
            republished: Vec::new(),
            republished_all: false,
            tables_republished: false,
            republished_revision: 0,
            files_revision: 0,
            claimed: HashSet::default(),
            reread_everything: false,
            claims_for: "Azeroth".to_string(),
            edited: Edited::default(),
            rows: Default::default(),
            applied_signature: None,
            applied: [None; crate::server::stack::Subject::COUNT],
            database_writes: 0,
            shipped: None,
            unchanged: None,
            locks: HashMap::default(),
            placement_locks: None,
        }
    }

    /// Record that the world database has changed under every cache of it:
    /// an Apply or a Put back of any subject finished, whether or not it
    /// succeeded, or the editor was pointed at another database.
    pub(crate) fn wrote_the_database(&mut self) {
        self.database_writes += 1;
    }
}

/// Open the project, install the overlay, and point the world at the map.
pub(crate) struct SessionPlugin;

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
                    // [`crate::camera::drop_to_the_ground`].
                    crate::camera::drop_to_the_ground,
                    // …and the list of places a jump can name, which is a join
                    // over two tables and is rebuilt only when the map changes.
                    crate::places::follow_the_map,
                    crate::camera::follow_the_camera,
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
            //
            // The client's banks forget in `ForgetChanged`, between the drain
            // and the hang, and the stage is shown the edit after they have.
            .add_systems(
                Update,
                (publish_for_the_preview, forget_what_changed)
                    .chain()
                    .before(vale_client::assets::ForgetChanged),
            )
            .configure_sets(
                Update,
                vale_client::assets::ForgetChanged.before(crate::preview::lab::hang_the_effect),
            )
            .add_systems(
                Update,
                show_the_edit
                    .after(vale_client::assets::ForgetChanged)
                    .before(crate::preview::lab::hang_the_effect),
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

