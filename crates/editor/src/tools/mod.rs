//! What the pointer does when a button is held.
//!
//! ```text
//! terrain.rs     the height brush: raise, lower, flatten, smooth
//! grade.rs       a ramp between two points, which the height brush cannot make
//! shading.rs     the colour painted onto the terrain's vertices: MCCV
//! textures.rs    the texture layers the terrain is painted with, and their
//!                blend maps
//! holes.rs       where the terrain is not drawn: the sixteen bits per chunk
//! areas.rs       which place in the world a chunk is: one AreaTable id per
//!                chunk
//! water.rs       the water standing on the terrain: MCLQ, and the flags that
//!                declare it
//! doodads.rs     the placed models (MDDF): select, move, turn, scale, remove
//! wmos.rs        the placed buildings, which are MODF and a different list
//! rehome.rs      what moving a doodad or a building past a tile border means
//! sweep.rs       a find-and-replace over every tile of the map: the paths in
//!                MTEX, MMDX and MWMO, most of which are not open. Not a
//!                pointer tool, because the subject is the map rather than a
//!                place on it
//! place.rs       putting a new doodad or building in the world
//! tables.rs      the client's own tables: which row is open, and what an edit
//!                to a field is. Not a pointer tool either: a table has no
//!                place in the world, so the workspace replaces the viewport
//! creatures.rs   the server's `creature` spawns on the map: each drawn where
//!                it stands, picked, moved, edited together with the
//!                `creature_template` behind it, created and removed. The
//!                first subject here whose document is a database row, and
//!                the first where an edit can remove a row; game objects,
//!                items and quests remove rows the same way
//! gameobjects.rs the same for `gameobject` and `gameobject_template`: a chest,
//!                a door, a vein, a mailbox. It works as the creature tool
//!                does over the next table, with no path and with a facing
//!                that is three columns
//! items.rs       `item_template`, which has no place in the world: what an
//!                item is, what it does and what it looks like. A workspace
//!                like the one for tables.rs, over the server's rows rather
//!                than a DBC, and the one server subject a `.reload` makes live
//! quests.rs      `quest_template` with the four relation tables beside it:
//!                what a quest asks for, says and gives, and who hands it out.
//!                A workspace of the same kind, and the list the creature
//!                tool's Quests window reads from the other direction
//! loot.rs        the nine loot tables. No workspace and no rail entry: a set
//!                is reached from the creature, the object or the item whose
//!                column names it, in a window that follows the selection.
//!                What a set holds, read one set at a time, and what an edit
//!                to a row of it is
//! displays.rs    the display id picker's state: a creature's or a game
//!                object's model chosen from CreatureDisplayInfo or
//!                GameObjectDisplayInfo by picture, searched by model path.
//!                Not a pointer tool: a dialog over the creature and game
//!                object forms
//! behaviour.rs   what a creature does: its `creature_ai_events` rows, the
//!                `creature_spells` list its template names, and the scripts
//!                either runs. No workspace and no rail entry: two windows
//!                opened from a selected creature and a third from either.
//!                Events and lists are rows of the store; a script is
//!                replaced whole, since its rows have no key
//! waypoints.rs   the path a creature walks: `creature_movement`, drawn over
//!                the ground, its points picked, dragged, added and removed. A
//!                mode of the creature tool rather than an entry on the rail,
//!                because a path has no existence apart from its creature
//! flightpaths.rs the flight paths: TaxiNodes, TaxiPath and TaxiPathNode,
//!                drawn on the map, their nodes and points picked, dragged,
//!                added and removed. Client tables whose rows are places, so
//!                the tool keeps the viewport and works during a playtest, as
//!                lights.rs does
//! lights.rs      Light.dbc's spheres, drawn on the map, picked with the
//!                pointer and flown to. A client table whose row is also a
//!                position, so, like flightpaths.rs, it is a DBC tool that
//!                keeps the viewport
//! tiles.rs       the tile itself: making ground where there was none, and the
//!                shadow bake and minimap picture that no other edit keeps in
//!                step. Not a pointer tool: the map window drives it
//! measure.rs     a pointer tool that changes nothing and reports what is at a
//!                point, and the distance and facing between two points
//! gizmo.rs       the handles a selected placement is moved and turned by
//! group.rs       a selection of more than one placement: the rules the four
//!                placement tools share, the rectangle a drag on empty ground
//!                draws, turning a group about one point, and the clipboard
//! spawn.rs       the handles and keys for a selected creature or game-object
//!                spawn, which has a position and a facing and no other angle:
//!                the reader and writer the handles use, and the keys that
//!                move, turn, remove and duplicate it
//! ```
//!
//! ## The four steps every tool follows
//!
//! [`Tool`] says which tool has the left button. Every tool in this directory
//! follows the same four steps, stated here once rather than in each file:
//!
//! ```text
//! crate::pick   the pointer resolves to a place in the world
//! stroke        the button is held, the model is changed, the change is recorded
//! live          what is on screen is caught up without reading the file
//! publish       the tile's bytes go into the overlay when the button comes up
//! ```
//!
//! The third step differs between height edits and placement edits, because
//! they move different things. A height edit moves vertices in a mesh already
//! on the GPU; a placement edit moves the transform of an entity that is
//! already spawned. Neither reads the tile again. [`terrain::remesh`] is the
//! fallback for a tile the live path could not reach, not the normal path.
//!
//! ## Where tiles are opened
//!
//! [`open_tiles`] parses the square of tiles `TerrainReach` names around the
//! focus (7x7 by default), one tile a frame. It is not part of the terrain
//! tool: the doodad pick reads the same `AdtFile` to find which `MDDF` entry
//! was clicked, so the function sits here, shared by both.

pub mod areas;
pub mod behaviour;
pub mod creatures;
pub mod displays;
pub mod doodads;
pub mod flightpaths;
pub mod gameobjects;
pub mod gizmo;
pub mod grade;
pub mod group;
pub mod holes;
pub mod items;
pub mod lights;
pub mod loot;
pub mod measure;
pub mod place;
pub mod quests;
pub mod rehome;
pub mod shading;
pub mod spawn;
pub mod sweep;
pub mod tables;
pub mod terrain;
pub mod textures;
pub mod tiles;
pub mod water;
pub mod waypoints;
pub mod wmos;

use crate::session::EditSession;
use vale_client::assets::GameAssets;
use bevy::prelude::*;

/// Which tool the pointer is holding.
///
/// The camera, the pick and the panels are the same whichever tool is chosen;
/// the tool decides only what a left-drag in the viewport does. The list is
/// expected to grow; [`crate::ui::rail`] names the tools
/// that do not exist yet.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// The pointer picks and reports and changes nothing.
    #[default]
    Select,
    /// What is at a clicked point, and the distance and facing between two —
    /// see [`measure`]. Changes nothing.
    Measure,
    /// The height brush. What it does to the ground is [`terrain::Terrain`].
    Terrain,
    /// A ramp between two points, which the height brush cannot make — see
    /// [`grade`]. Two clicks and a button rather than a stroke.
    Grade,
    /// The shading brush: the colour painted onto the terrain's vertices —
    /// see [`shading`].
    Shading,
    /// The texture brush — see [`textures`].
    Textures,
    /// Where the terrain is cut away entirely — see [`holes`].
    Holes,
    /// Which place in the world each chunk is — see [`areas`].
    Areas,
    /// The water standing on the terrain — see [`water`].
    Water,
    /// The placed models standing on the terrain — see [`doodads`].
    Doodads,
    /// The placed buildings, which are a different list — see [`wmos`].
    Wmos,
    /// `Spell.dbc` and every table it points at — see [`tables`].
    ///
    /// The first entry of the rail's Data group, and the first subject whose
    /// document is not the world: the workspace replaces the viewport rather
    /// than drawing over it.
    Spells,
    /// `Light.dbc` and its chain: the sun, the fill, the sky dome, the fog and
    /// the water's colour, per map and per zone — see [`tables`].
    ///
    /// A `Light` row is both a table row and a place: it is a sphere at a
    /// position on the open map, so the workspace sits beside the viewport
    /// rather than replacing it. See [`Tool::covers_viewport`].
    ///
    /// For the same reason it is on the rail's World half with the doodads
    /// rather than in the Data group beside Spells. See [`crate::ui::rail`].
    Lights,
    /// The server's creatures: where each one stands, and what it is — see
    /// [`creatures`].
    ///
    /// The first subject whose document is a row in vmangos' database rather
    /// than a file. It keeps the viewport for the same reason as `Lights`: a
    /// `creature` row carries a map and three coordinates, so it stands in the
    /// world, and the world shows whether it stands in the right place.
    ///
    /// The tool needs a database connection. With none, the tool opens, says
    /// so and lists nothing — see [`crate::server::settings`], and
    /// [`crate::ui::rail`], which greys the row rather than hiding it.
    Creatures,
    /// The server's game objects (chests, doors, veins, mailboxes): where each
    /// one stands, and what it is — see [`gameobjects`].
    ///
    /// The same as [`Tool::Creatures`] over the next table: it keeps the
    /// viewport, needs a connection and is disabled during a playtest on the
    /// same terms.
    GameObjects,
    /// The server's items: what each one is, what it does and what it looks
    /// like — see [`items`].
    ///
    /// A row in vmangos' database like [`Tool::Creatures`], and a workspace
    /// like [`Tool::Spells`]: an item has no place in the world, so the
    /// middle of the screen holds the form rather than the ground.
    ///
    /// It works during a playtest, which no other server subject except
    /// `Tool::Quests` does — see [`Tool::survives_playtest`].
    Items,
    /// The server's quests: what each asks for, says and gives, and who hands
    /// it out — see [`quests`].
    ///
    /// A row in vmangos' database and a workspace, as [`Tool::Items`] is. It
    /// survives a playtest for the same reason: `LoadQuests` and the relation
    /// loaders clear their maps before they read, so an applied row is live
    /// after a reload.
    Quests,
    /// A find-and-replace over every tile of the map — see [`sweep`].
    ///
    /// The second subject whose document is not a place: it edits the map's
    /// tiles, most of which are not open and none of which the pointer can
    /// reach. The viewport stays, because the tool changes what is drawn in
    /// it.
    Sweep,
    /// The flight paths: `TaxiNodes.dbc`, `TaxiPath.dbc` and
    /// `TaxiPathNode.dbc` — see [`flightpaths`].
    ///
    /// Client tables whose rows are places, as `Light` is: a node stands on the
    /// ground and a path's points are in the air over it, so they are picked
    /// and dragged in the viewport and the form goes in the inspector. For the
    /// same reason as `Lights`, it is on the rail's World half rather than
    /// with the data editors on the top bar.
    Flightpaths,
}

/// Where a tool's controls are drawn — see [`Tool::surface`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The viewport is the document and the inspector describes what is
    /// selected in it. Every brush and every placement tool.
    World,
    /// The workspace replaces the viewport, because the subject has no place
    /// in the world to look at.
    Middle,
    /// The viewport stays and the subject's form goes in the inspector, beside
    /// where a selected doodad's numbers go. For a subject that is both a row
    /// and a position, and is reached by pointing at it.
    Inspector,
}

/// Every tool, listed once.
///
/// Three places enumerate the tools: `--tool`, the rail's own check, and any
/// later caller. Each kept, or would have kept, its own list. `--tool` had
/// eight entries when a ninth tool was added, and it was not updated, so the
/// flag, which exists for photographing panels, could not select the new tool
/// and reported nothing.
///
/// A fieldless enum cannot enumerate itself without a derive crate, so
/// [`Tool::at`] enforces the list: an exhaustive `match` that does not compile
/// until a new variant has an index, plus a test that this list is exactly
/// those indices in order.
pub const ALL: [Tool; 19] = [
    Tool::Select,
    Tool::Terrain,
    Tool::Grade,
    Tool::Shading,
    Tool::Textures,
    Tool::Holes,
    Tool::Areas,
    Tool::Water,
    Tool::Doodads,
    Tool::Wmos,
    Tool::Sweep,
    Tool::Spells,
    Tool::Lights,
    Tool::Creatures,
    Tool::GameObjects,
    Tool::Items,
    Tool::Quests,
    Tool::Measure,
    Tool::Flightpaths,
];

impl Tool {
    /// Where this tool sits in [`ALL`].
    ///
    /// The `match` is exhaustive so that a new variant does not compile until
    /// it is given an index. That is the half of the check the compiler does;
    /// the other half is [`tests::the_list_holds_every_tool`].
    pub fn at(self) -> usize {
        match self {
            Tool::Select => 0,
            Tool::Terrain => 1,
            // Beside the height brush, because it is the shape that brush
            // cannot make — see `ALL`, whose order this must match.
            Tool::Grade => 2,
            Tool::Shading => 3,
            Tool::Textures => 4,
            Tool::Holes => 5,
            Tool::Areas => 6,
            Tool::Water => 7,
            Tool::Doodads => 8,
            Tool::Wmos => 9,
            Tool::Sweep => 10,
            Tool::Spells => 11,
            Tool::Lights => 12,
            Tool::Creatures => 13,
            Tool::GameObjects => 14,
            Tool::Items => 15,
            Tool::Quests => 16,
            Tool::Measure => 17,
            Tool::Flightpaths => 18,
        }
    }

    /// Which client table this tool edits, or `None` for a tool that edits the
    /// world.
    ///
    /// A tool with a table keeps working during a playtest (see
    /// [`Tool::survives_playtest`]); where its controls are drawn is
    /// [`Tool::surface`]'s answer. A tool that edits a chain of tables answers
    /// with the first: `Tool::Flightpaths` answers `TaxiNodes`, not
    /// `TaxiPath` or `TaxiPathNode`. See [`tables`].
    pub fn table(self) -> Option<&'static str> {
        match self {
            Tool::Spells => Some("Spell"),
            Tool::Lights => Some("Light"),
            Tool::Flightpaths => Some("TaxiNodes"),
            _ => None,
        }
    }

    /// Which table of the server's database this tool edits, or `None`.
    ///
    /// This is a separate question from [`Tool::table`]. [`Tool::table`]
    /// decides whether a tool keeps working through a
    /// playtest: a DBC is the same file whoever reads it, so a spell can be
    /// edited while a character stands in the world. A server table cannot be
    /// edited that way here: its rows are drawn where they stand in the
    /// editor's own world, and during a playtest the ground on screen is the
    /// game's and the creatures on it are the ones the server spawned.
    ///
    /// The name is the one the row writers use, so it is also the name a
    /// `.reload` is sent for.
    pub fn server_table(self) -> Option<&'static str> {
        match self {
            Tool::Creatures => Some(vale_mangos::creature::SPAWN),
            Tool::GameObjects => Some(vale_mangos::gameobject::SPAWN),
            Tool::Items => Some(vale_mangos::item::TEMPLATE),
            Tool::Quests => Some(vale_mangos::quest::TEMPLATE),
            _ => None,
        }
    }

    /// Whether this tool keeps working while a playtest is running.
    ///
    /// A DBC is the same file whoever reads it, so every tool with a
    /// [`Tool::table`] does. A tool whose document is the world does not: the
    /// editor's own tiles were despawned when the playtest started, and the
    /// ground on screen is the client's stream around the character.
    ///
    /// [`Tool::Items`] is a server subject that does, for the reason given in
    /// `vale_mangos::item`: `LoadItemPrototypes` clears the prototype map
    /// before it reads and `Item::GetProto` is a lookup per call, so an
    /// applied row is live for every copy of that item already in the world.
    /// This lets an item be edited and seen in the same session; greying the
    /// row would prevent that. `Tool::Quests` also survives, for the reason
    /// given on the variant.
    ///
    /// [`Tool::Creatures`] and [`Tool::GameObjects`] do not: a change to
    /// either needs the server restarted, so a panel offering one during a
    /// playtest would offer a change the game cannot show.
    pub fn survives_playtest(self) -> bool {
        self.table().is_some() || matches!(self, Tool::Items | Tool::Quests)
    }

    /// Where this tool's controls are drawn.
    ///
    /// Separate from [`Tool::table`] because the two answers differ: `Light`
    /// and the three flight path tables have a table but are picked in the
    /// world rather than found in a list.
    ///
    /// A spell has no place in the world, so its workspace takes the middle of
    /// the screen and the viewport is not the document. A `Light` row and a
    /// `TaxiNodes` or `TaxiPathNode` row are places: each is picked and dragged
    /// with the pointer, so its numbers go in the inspector with every other
    /// selection's numbers, and the world keeps the middle.
    ///
    /// Answering anything but [`Surface::Middle`] does not make a tool a world
    /// subject. It still has a table, so it still works while a playtest is
    /// running and the rail still leaves it lit — see [`crate::ui::rail`].
    pub fn surface(self) -> Surface {
        match self {
            Tool::Spells | Tool::Items | Tool::Quests => Surface::Middle,
            // A light's row, a flight path node or point, a creature's spawn
            // row and a game object's are each a place in the world, so each
            // is picked with the pointer and puts its form where every other
            // selection's numbers go.
            Tool::Lights | Tool::Flightpaths | Tool::Creatures | Tool::GameObjects => {
                Surface::Inspector
            }
            _ => Surface::World,
        }
    }

    /// Whether this tool can change anything. Select and Measure only report,
    /// so the inspector draws no undo row for them; `Ctrl+Z` still works.
    pub fn edits(self) -> bool {
        !matches!(self, Tool::Select | Tool::Measure)
    }

    /// Whether the workspace replaces the viewport. [`Surface::Middle`] alone.
    pub fn covers_viewport(self) -> bool {
        self.surface() == Surface::Middle
    }

    /// The tables this tool's workspace offers as tabs, in the order the chain
    /// is walked.
    ///
    /// Per tool rather than one list, because each data tool walks its own
    /// chain: `Spell`, `Light` and `TaxiNodes` each start one. A table
    /// reached by following a reference, such as `SpellIcon` or `LightSkybox`,
    /// is not a tab and highlights none; the browser still navigates to it.
    pub fn tabs(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Tool::Spells => &tables::SPELL_TABS,
            Tool::Lights => &tables::LIGHT_TABS,
            Tool::Flightpaths => &tables::TAXI_TABS,
            _ => &[],
        }
    }

    /// What to call it in an interface.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Terrain => "Terrain",
            Tool::Grade => "Grade",
            Tool::Shading => "Shading",
            Tool::Textures => "Textures",
            Tool::Holes => "Holes",
            Tool::Areas => "Areas",
            Tool::Water => "Water",
            Tool::Doodads => "Doodads",
            // "WMO" rather than "Buildings": it is what `MWMO` names, what
            // `MODF` places and what `vale wmos` checks, and half of what a
            // `.wmo` holds is not a building — a bridge, a gate, a canal.
            Tool::Wmos => "WMO",
            Tool::Sweep => "Sweep",
            Tool::Spells => "Spells",
            Tool::Lights => "Lights",
            Tool::Creatures => "Creatures",
            // "Objects" is this repository's own word for them: `vale
            // objects` is the CLI check over the same population.
            // The rail's tooltip says game objects, which is vmangos' word.
            Tool::GameObjects => "Objects",
            Tool::Items => "Items",
            Tool::Quests => "Quests",
            Tool::Measure => "Measure",
            // "Taxi" is the prefix of the three tables it edits: TaxiNodes,
            // TaxiPath and TaxiPathNode. One word, so `--tool taxi` needs no
            // quoting.
            Tool::Flightpaths => "Taxi",
        }
    }
}

/// How many tiles are parsed a frame. One: a tile is about two megabytes, and
/// parsing nine on the frame a map opens is a visible stall, while one a frame
/// spreads the same work over nine frames with no visible stall.
const OPEN_BUDGET: usize = 1;

/// Close the open tiles the focus has moved away from.
///
/// [`open_tiles`] parses every tile the block reaches. Without this system
/// nothing closes one, so a session that jumps around a map holds every tile
/// it has been near: two megabytes of parsed file each, forty-nine per stop.
/// Memory climbed with every teleport and was only partly released. This
/// system drops the
/// tiles further than the reach plus one from the focus that nothing needs
/// kept: not unsaved, not named by the history (undo writes into an open tile
/// and skips a closed one), and not waiting on any of the session's live sets.
///
/// It keeps one ring past the reach, so a focus moving along a border does
/// not close and re-parse the same row of tiles. Chained after `open_tiles`,
/// and subject to the same two conditions.
fn close_tiles(
    mut session: Option<ResMut<EditSession>>,
    focus: Res<vale_client::render::focus::WorldFocus>,
    reach: Res<vale_client::render::terrain::TerrainReach>,
    state: Res<crate::playtest::Playtest>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if !focus.present || !state.editing() {
        return;
    }
    // `VALE_KEEP_TILES=1` switches this off, so the memory growth it removes
    // can be measured by comparing two `--tour` runs, one with the variable
    // set and one without. The variable is read once.
    static KEEP_EVERYTHING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *KEEP_EVERYTHING.get_or_init(|| std::env::var_os("VALE_KEEP_TILES").is_some()) {
        return;
    }
    let (cx, cy) = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    let keep = reach.tiles() + 1;
    let far: Vec<(u32, u32)> = session
        .tiles
        .keys()
        .copied()
        .filter(|&(x, y)| {
            (x as i32 - cx as i32).abs() > keep || (y as i32 - cy as i32).abs() > keep
        })
        .filter(|coord| !session.unsaved.contains(coord))
        .filter(|coord| {
            !session.stale.contains(coord)
                && !session.restream.contains(coord)
                && !session.dirty.contains_key(coord)
                && !session.regrow.contains_key(coord)
                && !session.moved.contains_key(coord)
                && !session.moved_buildings.contains_key(coord)
                && !session.repaint.contains_key(coord)
        })
        .collect();
    for coord in far {
        session.close(coord);
    }
}

pub struct ToolPlugin;

impl Plugin for ToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tool>()
            .add_systems(Update, (open_tiles, close_tiles, modes).chain());
        app.add_plugins((
            terrain::TerrainToolPlugin,
            shading::ShadingToolPlugin,
            textures::TextureToolPlugin,
            holes::HoleToolPlugin,
            areas::AreaToolPlugin,
            lights::LightToolPlugin,
            water::WaterToolPlugin,
            doodads::DoodadToolPlugin,
            wmos::WmoToolPlugin,
            place::PlaceToolPlugin,
            sweep::SweepPlugin,
            gizmo::GizmoToolPlugin,
            grade::GradeToolPlugin,
            tables::TableToolPlugin,
            tiles::TilePlugin,
        ));
        // A second call, because `add_plugins` takes at most sixteen. The
        // first tuple is full; further tool plugins go in this one.
        app.add_plugins((
            creatures::CreatureToolPlugin,
            gameobjects::GameObjectToolPlugin,
            waypoints::WaypointToolPlugin,
            items::ItemToolPlugin,
            quests::QuestToolPlugin,
            loot::LootToolPlugin,
            behaviour::BehaviourToolPlugin,
            spawn::SpawnKeysPlugin,
            group::GroupPlugin,
            measure::MeasureToolPlugin,
            flightpaths::FlightpathToolPlugin,
        ));
    }
}

/// Parse the tiles around the focus, one a frame.
///
/// It opens the block that is drawn, not only the tile under the pointer,
/// because the pointer cannot pick against a tile that is not open and a
/// stroke near a tile border reaches into the next one. The block is
/// `render::terrain::TerrainReach`'s own square, the same one the streamer
/// puts on screen, so the editable area and the visible area are the same:
/// otherwise a tool would refuse edits on drawn ground the person can see.
///
/// Tiles are opened nearest first. A 3x3 reach does not need the ordering; a
/// 7x7 reach does. One tile is parsed a frame ([`OPEN_BUDGET`]), so at 49 tiles a row-major
/// walk spends its first half-second on the corners while the ground under
/// the pointer is still unopened, and until it is opened the pointer reports
/// no ground and the brush does nothing.
fn open_tiles(
    mut session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    focus: Res<vale_client::render::focus::WorldFocus>,
    reach: Res<vale_client::render::terrain::TerrainReach>,
    state: Res<crate::playtest::Playtest>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    // Not during a playtest. The focus follows the character then, so this
    // would parse two megabytes a frame along the character's route, for
    // tiles no tool may touch, and hold every one of them for the rest of the
    // session.
    if !focus.present || !state.editing() {
        return;
    }
    let (cx, cy) = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    let radius = reach.tiles();
    let mut wanted: Vec<(i32, (u32, u32))> = Vec::new();
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let (x, y) = (cx as i32 + dx, cy as i32 + dy);
            if !(0..64).contains(&x) || !(0..64).contains(&y) {
                continue;
            }
            let coord = (x as u32, y as u32);
            if session.tiles.contains_key(&coord) {
                continue;
            }
            wanted.push((dx * dx + dy * dy, coord));
        }
    }
    wanted.sort_unstable_by_key(|(distance, _)| *distance);
    let mut opened = 0;
    for (_, coord) in wanted {
        if opened >= OPEN_BUDGET {
            return;
        }
        // A coordinate the map does not have is asked for once a frame for
        // as long as the camera is over it. That is one archive miss, which
        // is a hash lookup in each of nineteen archives. The alternative, a
        // second set that remembers the absences, is not used because the
        // cost of the miss has not been measured as significant.
        if session.open(&assets, coord) {
            opened += 1;
        }
    }
}

/// Undo, redo, save, the playtest, and the shell over one.
///
/// Three of the six work during a playtest, each for its own reason. `Ctrl+P`
/// ends the playtest. `Ctrl+E` opens and closes the editor's panels over it —
/// see [`crate::playtest::ShellOpen`]. `Ctrl+S` writes the project and
/// republishes, which makes an edit made during a playtest reach the running
/// game without leaving it.
///
/// All of them check the game's own keyboard focus as well as egui's:
/// `KeyboardFocus` says whether an edit box or an `enableKeyboard` frame took
/// the keystroke. Without that check, a `Ctrl+P` typed into the chat line
/// would end the session it was typed in.
#[allow(clippy::too_many_arguments)]
pub(crate) fn shortcuts(
    mut session: Option<ResMut<EditSession>>,
    mut state: ResMut<crate::playtest::Playtest>,
    mut shell: ResMut<crate::playtest::ShellOpen>,
    mut client: ResMut<vale_client::world::session::Session>,
    login: Res<crate::playtest::Login>,
    mut auto: ResMut<vale_client::game::session::autologin::AutoLogin>,
    keys: Res<ButtonInput<KeyCode>>,
    typing: Res<vale_client::lua::api::keyboard::KeyboardFocus>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut tool: ResMut<Tool>,
    assets: Res<vale_client::assets::GameAssets>,
    doodads: Res<doodads::Selection>,
    wmos: Res<wmos::Selection>,
    mut placing: ResMut<place::Placing>,
    // What a save and a playtest need to reach the server: the reload queue,
    // where the database is, and whether the next session keeps its query
    // answers. See [`crate::server::Reach`], which bundles them into one
    // parameter because this system is at Bevy's limit of sixteen.
    mut reach: crate::server::Reach,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if wants.wants_keyboard_input() || !keys.pressed(KeyCode::ControlLeft) {
        return;
    }
    if keys.just_pressed(KeyCode::KeyP) && !typing.active {
        match state.editing() {
            true => crate::playtest::start(
                &mut state,
                session,
                &assets,
                &login,
                &mut auto,
                &mut reach.queue,
                &reach.settings,
                &mut reach.caches,
            ),
            false => crate::playtest::stop(&mut state, &mut client, &mut auto, session),
        }
        return;
    }
    // The shell over a playtest. This is the only shortcut here that does
    // nothing while editing, because the panels are already drawn then.
    if keys.just_pressed(KeyCode::KeyE) && !typing.active && state.playing() {
        shell.0 = !shell.0;
        if shell.0 {
            *tool = open_on(*tool);
        }
        return;
    }
    // Save works both while editing and during a playtest. During a playtest
    // it performs the whole reload: the write puts the edit in the project
    // folder and [`crate::playtest::republish`] puts it where the running
    // client reads it, so the next cast, the next entity and the next model
    // are the edited ones.
    if keys.just_pressed(KeyCode::KeyS) && !typing.active {
        session.save_all();
        session.save_all_tables();
        // The server half of the same key press: every subject's SQL, and,
        // when Apply on save is on, the statements run and the tables
        // reloaded. See [`crate::server::save`], which holds the only list of
        // subjects.
        crate::server::save(session, &assets, &reach.settings, &mut reach.queue);
        if state.playing() {
            crate::playtest::republish(session, &assets);
            session.status = "saved and republished to the playtest".to_string();
        }
        return;
    }
    if !state.editing() {
        return;
    }
    // Undo and redo mark what they moved, exactly as a stroke does, so the
    // ground and the placements follow them the same way.
    if keys.just_pressed(KeyCode::KeyZ) {
        session.undo(&assets);
    }
    if keys.just_pressed(KeyCode::KeyY) {
        session.redo(&assets);
    }
    // Duplicate arms the placement tool rather than copying — see [`place`]
    // for the reason. It is here rather than in either tool because it is one
    // key doing one thing to whichever list is open.
    //
    // One placement only. A group cannot ride on the cursor as one model,
    // so over several `Ctrl+D` is `group::clipboard`'s, which copies them in
    // place a step to the north.
    if keys.just_pressed(KeyCode::KeyD) {
        let armed = match *tool {
            Tool::Doodads if !doodads.also.is_empty() => None,
            Tool::Wmos if !wmos.also.is_empty() => None,
            Tool::Doodads => doodads.at.as_ref().map(|at| {
                let turns =
                    vale_assets::world::adt::placement_euler_to_world(at.record.rotation);
                (
                    place::Kind::Doodad,
                    at.path.clone(),
                    turns[2],
                    f32::from(at.record.scale) / 1024.0,
                )
            }),
            Tool::Wmos => wmos.at.as_ref().map(|at| {
                let turns =
                    vale_assets::world::adt::placement_euler_to_world(at.record.rotation);
                (place::Kind::Wmo, at.path.clone(), turns[2], 1.0)
            }),
            _ => None,
        };
        if let Some((kind, path, turn, scale)) = armed {
            placing.arm(kind, path, turn, scale);
            session.status = format!("duplicating {}", place::leaf(&placing_path(&placing)));
        }
    }
}

/// Which subject the shell opens on over a playtest.
///
/// The world tools cannot act during a playtest, because the editor's own
/// tiles were despawned when it started. Opening the shell on the terrain
/// brush would show a greyed rail, an inspector for a brush that does nothing,
/// and the game behind. Every subject whose document is a table works, so a
/// world tool is replaced by the first of them and a table tool is kept.
pub(crate) fn open_on(tool: Tool) -> Tool {
    match tool.table() {
        Some(_) => tool,
        None => Tool::Spells,
    }
}

/// The armed path, for the status line.
fn placing_path(placing: &place::Placing) -> String {
    placing.path.clone().unwrap_or_default()
}

/// Which brush setting the mouse wheel adjusts, chosen by a held modifier.
///
/// A brush has more frequently adjusted numbers than there are gestures to
/// adjust them with, and the hotkeys exist so that nobody has to reach for a
/// panel field mid-stroke. So the wheel carries three, chosen by modifier:
/// `Ctrl` the size, `Shift` the strength, `Alt` the core.
///
/// It is one list because three systems read it and they must agree. The two
/// brushes act on it, and `camera::fly` ignores the wheel whenever this names
/// a setting. Without that, one notch both resized the brush and moved the
/// camera. The tileset list had the same fault and has the same fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wheel {
    /// How far the brush reaches.
    Radius,
    /// How strongly each application of the brush acts.
    Strength,
    /// How much of the radius is at full strength — the inner ring the preview
    /// draws. See `vale_edit::ops::Falloff::over`.
    Core,
}

impl Wheel {
    /// Which of the three a held modifier names, if any.
    ///
    /// Checked in the order a person reaches for them, and on both sides of
    /// the keyboard. When only the left-hand modifiers were checked, the same
    /// gesture worked or did not depending on which hand made it.
    pub fn held(keys: &ButtonInput<KeyCode>) -> Option<Wheel> {
        let either = |a, b| keys.pressed(a) || keys.pressed(b);
        if either(KeyCode::ControlLeft, KeyCode::ControlRight) {
            return Some(Wheel::Radius);
        }
        if either(KeyCode::ShiftLeft, KeyCode::ShiftRight) {
            return Some(Wheel::Strength);
        }
        if either(KeyCode::AltLeft, KeyCode::AltRight) {
            return Some(Wheel::Core);
        }
        None
    }
}

/// How much of the radius may be held at full strength.
///
/// Below 1, because a core of the whole radius is the same as
/// [`Falloff::Flat`] and leaves the falloff no distance to run over.
pub const CORE: std::ops::RangeInclusive<f32> = 0.0..=0.9;

/// The number row selects the chosen tool's modes.
///
/// Every tool here has a small set of states a person switches between often
/// (raise and lower, select and place), and each switch required moving the
/// pointer to a segmented control. The digits are the one block of keys that
/// nothing in this crate or in the game's own bindings used. Their meaning
/// depends on the tool: `1` is Raise on the terrain brush and Select on the
/// placements, because those are the first entry of each panel's first row.
///
/// `Tab` also works for the placements, because Select and Place is a pair
/// people toggle between rather than choose from.
///
/// It is one system rather than a branch inside each tool so that the whole
/// mapping is visible in one place: if a tool's keys did not follow the
/// layout of its panel, they would have to be learnt separately.
#[allow(clippy::too_many_arguments)]
fn modes(
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    typing: Res<vale_client::lua::api::keyboard::KeyboardFocus>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut terrain: ResMut<terrain::Terrain>,
    mut shading: ResMut<shading::Shading>,
    mut textures: ResMut<textures::Textures>,
    mut placing: ResMut<place::Placing>,
) {
    if !state.editing() || wants.wants_keyboard_input() || typing.active {
        return;
    }
    // `Ctrl` and `Alt` belong to other gestures: `Ctrl` is the
    // undo/redo/save/playtest block and `Alt` turns a placement. `Shift` is
    // free and selects the second row: a brush panel has more choices than
    // there are digits, and a shifted digit is the common convention for that.
    if keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::AltLeft)
        || keys.pressed(KeyCode::AltRight)
    {
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    // Ten, and `0` is the tenth: the row as it is laid out on the keyboard
    // rather than as it is numbered.
    let digit = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
        KeyCode::Digit0,
    ]
    .iter()
    .position(|&key| keys.just_pressed(key));

    match *tool {
        // No modes. A grade is two clicks and a button, so the number row has
        // no modes to pick between. The same is true of the creature, object,
        // item, quest and flight path tools: the number row picks a brush
        // mode, and a row of a table has none.
        Tool::Grade
        | Tool::Creatures
        | Tool::GameObjects
        | Tool::Items
        | Tool::Quests
        | Tool::Flightpaths => {}
        Tool::Terrain => {
            // Unshifted digits cover the two long rows; shifted digits cover
            // the short one. 1..5 are the five modes, 6..0 the five falloffs,
            // and `Shift`+1..3 the three shapes: the panel's own rows in the
            // panel's own order, so the panel shows which key does what.
            let Some(at) = digit else { return };
            if shift {
                if let Some(&(_, shape)) = terrain::SHAPES.get(at) {
                    terrain.brush.shape = shape;
                }
            } else if let Some(&(_, mode)) = terrain::MODES.get(at) {
                terrain.brush.mode = mode;
            } else if let Some(&(_, falloff)) = terrain::FALLOFFS.get(at - terrain::MODES.len()) {
                terrain.brush.falloff = falloff;
            }
        }
        Tool::Shading => {
            // Five modes and five falloffs, laid out exactly as the height
            // brush's are and for the same reason: the two are the same three
            // controls over the same vertices, so a key learnt on one is the key
            // on the other.
            let Some(at) = digit else { return };
            if shift {
                if let Some(&(_, shape)) = terrain::SHAPES.get(at) {
                    shading.brush.shape = shape;
                }
            } else if let Some(&(_, mode)) = shading::MODES.get(at) {
                shading.brush.mode = mode;
            } else if let Some(&(_, falloff)) = terrain::FALLOFFS.get(at - shading::MODES.len()) {
                shading.brush.falloff = falloff;
            }
        }
        Tool::Textures => {
            // The texture brush has no modes, so its unshifted digits are the
            // falloffs — the panel's first row of choices — and `Shift` is the
            // shapes, exactly as above.
            let Some(at) = digit else { return };
            match shift {
                true => {
                    if let Some(&(_, shape)) = textures::SHAPES.get(at) {
                        textures.brush.shape = shape;
                    }
                }
                false => {
                    if let Some(&(_, falloff)) = textures::FALLOFFS.get(at) {
                        textures.brush.falloff = falloff;
                    }
                }
            }
        }
        Tool::Doodads | Tool::Wmos => {
            // Two choices and no second row, so a shifted digit means nothing
            // here rather than meaning the unshifted one.
            if shift {
                return;
            }
            let wanted = match digit {
                Some(at) => place::MODES.get(at).map(|&(_, mode)| mode),
                // `Tab` is the toggle rather than a third key, because these two
                // are a pair rather than a list.
                None if keys.just_pressed(KeyCode::Tab) => Some(match placing.mode {
                    place::Mode::Select => place::Mode::Place,
                    place::Mode::Place => place::Mode::Select,
                }),
                None => None,
            };
            if let Some(mode) = wanted {
                placing.mode = mode;
                // Switching to Select puts down whatever was on the cursor,
                // which is what the panel's own switch does — see
                // [`place::aim`], which would do it a frame later anyway.
                if mode == place::Mode::Select {
                    placing.disarm();
                }
            }
        }
        // These tools have no modes. In Spells the digits go to whichever
        // text field has focus: a workspace has text fields, and a number row
        // that meant something here would conflict with every one of them.
        Tool::Areas
        | Tool::Water
        | Tool::Select
        | Tool::Measure
        | Tool::Holes
        | Tool::Spells
        | Tool::Lights
        | Tool::Sweep => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shell over a playtest opens on a subject that works there.
    ///
    /// Every tool whose document is the world is greyed during a playtest,
    /// because the editor's own tiles were despawned when it started. Opening
    /// the panels on one would show a greyed rail and an inspector about a
    /// brush that does nothing. A tool whose document is a table is left where
    /// it is, because a DBC is the same file whoever is looking at it.
    #[test]
    fn the_shell_opens_on_a_table_subject() {
        for tool in ALL {
            let opened = open_on(tool);
            assert!(
                opened.table().is_some(),
                "{} opened the shell on {}, which has no table",
                tool.name(),
                opened.name()
            );
            if tool.table().is_some() {
                assert_eq!(opened, tool, "a table subject is left where it is");
            }
        }
    }

    /// [`ALL`] is every tool, in index order.
    ///
    /// The compiler catches a new variant with no index; this test catches one
    /// with an index that is not in the list. The three hand-kept copies this
    /// list replaced went wrong that way: a tool was added to the enum, to its
    /// `name`, to the rail and to the inspector, and not to `--tool`.
    #[test]
    fn the_list_holds_every_tool() {
        for (i, tool) in ALL.iter().enumerate() {
            assert_eq!(tool.at(), i, "{} is listed out of order", tool.name());
        }
        // The length is the largest index plus one, which shows that nothing
        // is missing from the end.
        let highest = ALL.iter().map(|tool| tool.at()).max().unwrap_or(0);
        assert_eq!(ALL.len(), highest + 1);
    }

    /// Every tool answers to a distinct name, because that name is what
    /// `--tool` matches on and what the rail prints.
    #[test]
    fn every_tool_has_its_own_name() {
        let mut names: Vec<&str> = ALL.iter().map(|tool| tool.name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two tools share a name");
        assert!(names.iter().all(|name| !name.is_empty()));
    }

    /// A data subject's tabs are its own chain, starting at its own table.
    ///
    /// The tab list was once one constant, the spell chain's, read by two call
    /// sites; the second chain made it a per-tool answer. A wrong list fails
    /// silently: the lights workspace would draw the spell chain's four tabs,
    /// and pressing one would leave the subject the rail shows as open.
    #[test]
    fn a_data_subjects_tabs_are_its_own_chain() {
        use vale_assets::tables::schema;
        for tool in ALL {
            let tabs = tool.tabs();
            let Some(table) = tool.table() else {
                assert!(
                    tabs.is_empty(),
                    "{} edits no table but has tabs",
                    tool.name()
                );
                continue;
            };
            assert!(
                !tabs.is_empty(),
                "{} edits {table} and offers no tab",
                tool.name()
            );
            assert_eq!(
                tabs[0].1,
                table,
                "{}'s first tab is not the table the rail opens it on",
                tool.name()
            );
            for &(label, name) in tabs {
                assert!(!label.is_empty(), "{name} has no label");
                assert!(
                    schema::for_table(name).is_some(),
                    "{}'s tab {name} has no schema, so it would draw as numbered fields",
                    tool.name()
                );
            }
        }
    }

    /// `LightIntBand` and `LightFloatBand` are tabs because no reference
    /// column names them, so a tab is the only route to them.
    ///
    /// Every other table in either chain is named by a `Kind::Reference`
    /// somewhere, so a browser can follow a row into it. `LightIntBand` and
    /// `LightFloatBand` are named by nothing: a band is found by arithmetic on
    /// its `LightParams` row. Without a tab there would be no route to either
    /// table at all.
    #[test]
    fn the_band_tables_are_named_by_no_reference_column() {
        use vale_assets::tables::schema::{self, Kind};
        let pointed_at = |table: &str| {
            schema::ALL.iter().any(|schema| {
                schema
                    .columns
                    .iter()
                    .any(|column| matches!(column.kind, Kind::Reference(at) if at == table))
            })
        };
        assert!(!pointed_at("LightIntBand"));
        assert!(!pointed_at("LightFloatBand"));
        // The tables that are referenced are found, which shows the check
        // can detect a reference.
        assert!(pointed_at("LightParams"));
        assert!(pointed_at("LightSkybox"));
        // Both bands are on the lights tool's tabs, which is the only route.
        let tabs = Tool::Lights.tabs();
        for band in ["LightIntBand", "LightFloatBand"] {
            assert!(
                tabs.iter().any(|&(_, name)| name == band),
                "{band} has no tab"
            );
        }
    }

    /// Covering the viewport is a narrower condition than having a table.
    ///
    /// A tool that covers the viewport must have a subject to put there, a
    /// client table or a server one, but the reverse does not hold: a `Light`
    /// row is a sphere at a position on the open map, a flight path node
    /// stands on the ground and so does a creature spawn, so hiding the world
    /// would hide half of what is being edited.
    ///
    /// The test accepts a server table as well as a client one because
    /// `Items` has no client table: `item_template` is a row in vmangos'
    /// database, and an item has no place in the world either way. Before
    /// `Items`, every workspace had a client table.
    #[test]
    fn only_a_placeless_subject_covers_the_viewport() {
        for tool in ALL {
            if tool.covers_viewport() {
                assert!(
                    tool.table().is_some() || tool.server_table().is_some(),
                    "{} covers the viewport with no table to put there",
                    tool.name()
                );
            }
        }
        assert!(Tool::Items.covers_viewport());
        assert!(
            Tool::Items.table().is_none(),
            "there is no Item.dbc in 1.12"
        );
        assert!(Tool::Spells.covers_viewport());
        assert!(!Tool::Lights.covers_viewport());
        // The creature tool also keeps the viewport, for the same reason: a
        // spawn is a place, and a form that hid the place would hide the
        // subject. It has no client table, which is why it is unavailable for
        // the duration of a playtest.
        assert_eq!(Tool::Creatures.surface(), Surface::Inspector);
        assert!(Tool::Creatures.table().is_none());
        assert_eq!(Tool::Creatures.server_table(), Some("creature"));
        assert_eq!(Tool::GameObjects.surface(), Surface::Inspector);
        assert!(Tool::GameObjects.table().is_none());
        assert_eq!(Tool::GameObjects.server_table(), Some("gameobject"));
        assert!(!Tool::GameObjects.survives_playtest());
        assert!(Tool::Lights.table().is_some());
        // A world subject is unaffected either way.
        assert!(!Tool::Terrain.covers_viewport());
    }

    /// A tool that draws anywhere but the world has a table to draw.
    ///
    /// The three surfaces and what may take each: a world subject is the
    /// viewport's, a middle subject covers it, and an inspector subject keeps
    /// it and puts its numbers where a selection's numbers go. Only the last
    /// two are data subjects, and only the middle one hides the world.
    #[test]
    fn every_surface_but_the_world_has_a_table_behind_it() {
        for tool in ALL {
            match tool.surface() {
                Surface::World => assert!(
                    tool.table().is_none() && tool.server_table().is_none(),
                    "{} edits {:?} and draws nowhere",
                    tool.name(),
                    tool.table()
                ),
                // A client table or a server one. The subject has to be a row
                // either way. `server_table` says which side of the wire it
                // lives on, and that decides whether the tool survives a
                // playtest, not where it draws.
                Surface::Middle | Surface::Inspector => assert!(
                    tool.table().is_some() || tool.server_table().is_some(),
                    "{} draws a form with no table in it",
                    tool.name()
                ),
            }
        }
        assert_eq!(Tool::Spells.surface(), Surface::Middle);
        // The lights tool keeps the viewport, because a light is picked and
        // dragged in it; covering it would hide the subject.
        assert_eq!(Tool::Lights.surface(), Surface::Inspector);
        assert!(!Tool::Lights.covers_viewport());
        // The creature tool also keeps the viewport, for the same reason: a
        // spawn is a place, and a form that hid the place would hide the
        // subject. It has no client table, which is why it is unavailable for
        // the duration of a playtest.
        assert_eq!(Tool::Creatures.surface(), Surface::Inspector);
        assert!(Tool::Creatures.table().is_none());
        assert_eq!(Tool::Creatures.server_table(), Some("creature"));
        assert_eq!(Tool::GameObjects.surface(), Surface::Inspector);
        assert!(Tool::GameObjects.table().is_none());
        assert_eq!(Tool::GameObjects.server_table(), Some("gameobject"));
        assert!(!Tool::GameObjects.survives_playtest());
        assert_eq!(Tool::Terrain.surface(), Surface::World);
    }

    /// A data subject survives a playtest whether or not it covers the
    /// viewport, because a DBC is the same file whoever is looking at it.
    ///
    /// [`open_on`] keys on the table rather than on the covering, so a tool
    /// with a table that keeps the viewport, such as `Lights`, still opens the
    /// shell on itself.
    #[test]
    fn a_playtest_keeps_every_data_subject() {
        assert_eq!(open_on(Tool::Lights), Tool::Lights);
        assert_eq!(open_on(Tool::Spells), Tool::Spells);
    }
}
