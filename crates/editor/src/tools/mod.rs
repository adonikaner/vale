//! What the pointer does when a button is held.
//!
//! ```text
//! ground/        the tools that edit the terrain itself: heights, grades,
//!                roads, shading, textures, holes, areas, chunks and water
//! placements/    the placed models and buildings, and the handles, groups and
//!                tile moves the placement tools share
//! spawns/        the server's creature and game object spawns, their
//!                waypoint paths, and the handles a spawn is moved by
//! places/        the client-table rows that are places on the map: lights,
//!                flight paths, area triggers and graveyards
//! measure.rs     a pointer tool that changes nothing and reports what is at a
//!                point, and the distance and facing between two points
//! guides.rs      what is drawn on the ground to read it by, under every
//!                tool: the chunk and tile grid, the ground that is too steep,
//!                contour lines and the texture count per chunk. Switches on
//!                the view bar; nothing here edits
//! shortcuts.rs   the keys that work whichever tool is chosen: undo, redo,
//!                save, the playtest, and the shell over one
//! modes.rs       the number row, which selects the chosen tool's modes
//! ```
//!
//! ## The four steps every tool follows
//!
//! [`Tool`] says which tool has the left button. Every tool under this directory
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
//! already spawned. Neither reads the tile again. [`ground::terrain::remesh`] is the
//! fallback for a tile the live path could not reach, not the normal path.
//!
//! ## Where tiles are opened
//!
//! [`open_tiles`] reads and parses the square of tiles `TerrainReach` names
//! around the focus (7x7 by default) on the task pool, and puts each in the
//! session when it is ready. It is not part of the terrain tool: the doodad
//! pick reads the same `AdtFile` to find which `MDDF` entry was clicked, so
//! the function is in `crate::map::open`, shared by both, and [`ToolPlugin`]
//! runs it.

pub mod ground;
pub mod guides;
pub mod measure;
pub(crate) mod modes;
pub mod places;
pub mod placements;
pub(crate) mod shortcuts;
pub mod spawns;

use crate::map::open::{close_tiles, open_tiles, Opening};
use crate::session::EditSession;
use bevy::prelude::*;
use modes::modes;

/// Which tool the pointer is holding.
///
/// The camera, the pick and the panels are the same whichever tool is chosen;
/// the tool decides only what a left-drag in the viewport does. The list is
/// expected to grow; [`crate::ui::shell::rail`] names the tools
/// that do not exist yet.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// The pointer picks and reports and changes nothing.
    #[default]
    Select,
    /// What is at a clicked point, and the distance and facing between two —
    /// see [`measure`]. Changes nothing.
    Measure,
    /// The height brush. What it does to the ground is [`ground::terrain::Terrain`].
    Terrain,
    /// A ramp between two points, which the height brush cannot make — see
    /// [`ground::grade`]. Two clicks and a button rather than a stroke.
    Grade,
    /// A road along a run of points: the ground graded along it and its
    /// textures painted — see [`ground::road`].
    Road,
    /// The shading brush: the colour painted onto the terrain's vertices —
    /// see [`ground::shading`].
    Shading,
    /// The texture brush — see [`ground::textures`].
    Textures,
    /// Where the terrain is cut away entirely — see [`ground::holes`].
    Holes,
    /// Which place in the world each chunk is — see [`ground::areas`].
    Areas,
    /// A selection of chunks, and one value written to all of them — see
    /// [`ground::chunks`].
    Chunks,
    /// The water standing on the terrain — see [`ground::water`].
    Water,
    /// The placed models standing on the terrain — see [`placements::doodads`].
    Doodads,
    /// The placed buildings, which are a different list — see [`placements::wmos`].
    Wmos,
    /// `Spell.dbc` and every table it points at — see [`crate::tables`].
    ///
    /// The first entry of the rail's Data group, and the first subject whose
    /// document is not the world: the workspace replaces the viewport rather
    /// than drawing over it.
    Spells,
    /// `Light.dbc` and its chain: the sun, the fill, the sky dome, the fog and
    /// the water's colour, per map and per zone — see [`crate::tables`].
    ///
    /// A `Light` row is both a table row and a place: it is a sphere at a
    /// position on the open map, so the workspace sits beside the viewport
    /// rather than replacing it. See [`Tool::covers_viewport`].
    ///
    /// For the same reason it is on the rail's World half with the doodads
    /// rather than in the Data group beside Spells. See [`crate::ui::shell::rail`].
    Lights,
    /// The server's creatures: where each one stands, and what it is — see
    /// [`spawns::creatures`].
    ///
    /// The first subject whose document is a row in vmangos' database rather
    /// than a file. It keeps the viewport for the same reason as `Lights`: a
    /// `creature` row carries a map and three coordinates, so it stands in the
    /// world, and the world shows whether it stands in the right place.
    ///
    /// The tool needs a database connection. With none, the tool opens, says
    /// so and lists nothing — see [`crate::server::settings`], and
    /// [`crate::ui::shell::rail`], which greys the row rather than hiding it.
    Creatures,
    /// The server's game objects (chests, doors, veins, mailboxes): where each
    /// one stands, and what it is — see [`spawns::gameobjects`].
    ///
    /// The same as [`Tool::Creatures`] over the next table: it keeps the
    /// viewport, needs a connection and is disabled during a playtest on the
    /// same terms.
    GameObjects,
    /// The server's items: what each one is, what it does and what it looks
    /// like — see [`crate::subjects::items`].
    ///
    /// A row in vmangos' database like [`Tool::Creatures`], and a workspace
    /// like [`Tool::Spells`]: an item has no place in the world, so the
    /// middle of the screen holds the form rather than the ground.
    ///
    /// It works during a playtest, which no other server subject except
    /// `Tool::Quests` does — see [`Tool::survives_playtest`].
    Items,
    /// `ItemSet.dbc`: which items make up a set, and the bonuses it grants at
    /// how many pieces — see [`crate::tables`].
    ///
    /// A DBC table, edited in the table browser, and a part of the Items
    /// workspace: the top bar shows Items for both and a strip at the head of
    /// the list switches between them ([`Tool::parts`]). The set is a file, so
    /// it needs no database; the item names beside its item columns do.
    ItemSets,
    /// The server's quests: what each asks for, says and gives, and who hands
    /// it out — see [`crate::subjects::quests`].
    ///
    /// A row in vmangos' database and a workspace, as [`Tool::Items`] is. It
    /// survives a playtest for the same reason: `LoadQuests` and the relation
    /// loaders clear their maps before they read, so an applied row is live
    /// after a reload.
    Quests,
    /// The Creatures workspace: every `creature_template` row as a list,
    /// searched by name, subname or entry, and edited in the middle of the
    /// screen — see [`crate::subjects::creature_data`].
    ///
    /// The same row the creature tool's template window edits, reached by
    /// name rather than by clicking a spawn. A template has no place in the
    /// world, so the workspace replaces the viewport. It does not survive a
    /// playtest, because the server reads `creature_template` at startup.
    CreatureData,
    /// The Game Objects workspace: every `gameobject_template` row, on
    /// [`Tool::CreatureData`]'s terms — see [`crate::subjects::gameobject_data`].
    GameObjectData,
    /// `CreatureDisplayInfo.dbc` with `CreatureDisplayInfoExtra` and
    /// `CreatureModelData` as two more tabs: what a creature looks like. A
    /// part of the Creatures workspace, edited in the table browser.
    CreatureDisplays,
    /// `GameObjectDisplayInfo.dbc`: a game object's model and sounds. A part
    /// of the Game Objects workspace.
    ObjectDisplays,
    /// `ItemDisplayInfo.dbc`: an item's icon, models and body textures. A
    /// part of the Items workspace.
    ItemDisplays,
    /// The files the project adds to the archives: imported textures, baked
    /// models and the rest — see [`crate::import::assets`]. A workspace, since a file has no
    /// place in the world; it has no table, because its subject is the
    /// project folder.
    Assets,
    /// A find-and-replace over every tile of the map — see [`crate::map::sweep`].
    ///
    /// The second subject whose document is not a place: it edits the map's
    /// tiles, most of which are not open and none of which the pointer can
    /// reach. The viewport stays, because the tool changes what is drawn in
    /// it.
    Sweep,
    /// The flight paths: `TaxiNodes.dbc`, `TaxiPath.dbc` and
    /// `TaxiPathNode.dbc` — see [`places::flightpaths`].
    ///
    /// Client tables whose rows are places, as `Light` is: a node stands on the
    /// ground and a path's points are in the air over it, so they are picked
    /// and dragged in the viewport and the form goes in the inspector. For the
    /// same reason as `Lights`, it is on the rail's World half rather than
    /// with the data editors on the top bar.
    Flightpaths,
    /// `AreaTable.dbc`: the zones and the sub-areas inside them, what each
    /// is called and how the game treats a character standing in it — see
    /// [`crate::tables`].
    ///
    /// A DBC table edited in the table browser, and the row [`Tool::Areas`]
    /// paints onto the ground by id: this tool says what an area is, and that
    /// one says where it is. The server reads its copy from `area_template`
    /// at startup, so it is not available during a playtest.
    Zones,
    /// `AreaTrigger.dbc`: the spheres and boxes the client reports standing
    /// in, and what the server does when it is told — see [`places::triggers`].
    ///
    /// A client table whose rows are places, as `Light` is, so it keeps the
    /// viewport and puts its form in the inspector. The server half is three
    /// tables of vmangos' database, which the form reads when a connection is
    /// set and leaves out when none is.
    Triggers,
    /// `WorldSafeLocs.dbc`: where a released spirit appears, and which zones
    /// each place serves — see [`places::graveyards`].
    ///
    /// On `Tool::Triggers`' terms: the places are a client table picked in the
    /// viewport, and the links to zones are rows of vmangos' database.
    Graveyards,
    /// Any DBC table, by name — see [`crate::tables`].
    ///
    /// The workspace's list is the files of `DBFilesClient\`. Choosing one
    /// opens it in the browser the spell workspace uses: as named, typed
    /// fields where `vale_assets::tables::schema` describes the table, and
    /// as numbered fields with guessed readings where it does not.
    Tables,
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
pub const ALL: [Tool; 32] = [
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
    Tool::Chunks,
    Tool::ItemSets,
    Tool::Tables,
    Tool::Zones,
    Tool::Triggers,
    Tool::Graveyards,
    Tool::Road,
    Tool::CreatureData,
    Tool::GameObjectData,
    Tool::CreatureDisplays,
    Tool::ObjectDisplays,
    Tool::ItemDisplays,
    Tool::Assets,
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
            Tool::Chunks => 19,
            Tool::ItemSets => 20,
            Tool::Tables => 21,
            Tool::Zones => 22,
            Tool::Triggers => 23,
            Tool::Graveyards => 24,
            Tool::Road => 25,
            Tool::CreatureData => 26,
            Tool::GameObjectData => 27,
            Tool::CreatureDisplays => 28,
            Tool::ObjectDisplays => 29,
            Tool::ItemDisplays => 30,
            Tool::Assets => 31,
        }
    }

    /// Which client table this tool edits, or `None` for a tool that edits the
    /// world.
    ///
    /// Where a tool's controls are drawn is [`Tool::surface`]'s answer, and
    /// whether it works during a playtest is [`Tool::survives_playtest`]'s. A
    /// tool that edits a chain of tables answers
    /// with the first: `Tool::Flightpaths` answers `TaxiNodes`, not
    /// `TaxiPath` or `TaxiPathNode`. See [`crate::tables`].
    pub fn table(self) -> Option<&'static str> {
        match self {
            Tool::Spells => Some("Spell"),
            Tool::Lights => Some("Light"),
            Tool::Flightpaths => Some("TaxiNodes"),
            Tool::ItemSets => Some("ItemSet"),
            Tool::Zones => Some(crate::tables::zones::area::TABLE),
            Tool::Triggers => Some(vale_edit::dbc::places::TRIGGERS),
            Tool::Graveyards => Some(vale_edit::dbc::places::SAFE_LOCS),
            Tool::CreatureDisplays => Some("CreatureDisplayInfo"),
            Tool::ObjectDisplays => Some("GameObjectDisplayInfo"),
            Tool::ItemDisplays => Some("ItemDisplayInfo"),
            // No one table: the workspace starts on the list of them.
            Tool::Tables => Some(crate::tables::tabs::ANY),
            _ => None,
        }
    }

    /// Which table of the server's database this tool edits, or `None`.
    ///
    /// This is a separate question from [`Tool::table`], which names a client
    /// table.
    ///
    /// The name is the one the row writers use, so it is also the name a
    /// `.reload` is sent for.
    pub fn server_table(self) -> Option<&'static str> {
        match self {
            Tool::Creatures => Some(vale_mangos::creature::SPAWN),
            Tool::GameObjects => Some(vale_mangos::gameobject::SPAWN),
            Tool::Items => Some(vale_mangos::item::TEMPLATE),
            Tool::Quests => Some(vale_mangos::quest::TEMPLATE),
            Tool::CreatureData => Some(vale_mangos::creature::TEMPLATE),
            Tool::GameObjectData => Some(vale_mangos::gameobject::TEMPLATE),
            _ => None,
        }
    }

    /// Whether this tool keeps working while a playtest is running.
    ///
    /// Four workspaces do: Spells, Items with its Sets part, Quests and
    /// Tables. Each replaces the viewport, so none needs the editor's own
    /// world, which was despawned when the playtest started. [`Tool::Zones`]
    /// is a workspace that does not: the server reads `area_template` at
    /// startup only, and an area is put on the ground with a tool that keeps
    /// the viewport.
    ///
    /// A DBC is the same file whoever reads it, so a spell or any other table
    /// can be edited while a character stands in the world. [`Tool::Items`] is
    /// a server subject that survives, for the reason given in
    /// `vale_mangos::item`: `LoadItemPrototypes` clears the prototype map
    /// before it reads and `Item::GetProto` is a lookup per call, so an
    /// applied row is live for every copy of that item already in the world.
    /// `Tool::Quests` survives for the reason given on the variant.
    ///
    /// Every tool that keeps the viewport does not. The ground on screen
    /// during a playtest is the client's stream around the character, and the
    /// tool's picks, drags and markers act on the editor's own camera and
    /// tiles. That includes [`Tool::Lights`] and [`Tool::Flightpaths`], which
    /// edit client tables but pick and drag their rows in the viewport, and
    /// the server reads the taxi tables at startup only.
    /// [`Tool::Creatures`] and [`Tool::GameObjects`] also need the server
    /// restarted for a change to show. The Creatures and Game Objects
    /// workspaces replace the viewport but do not survive either: the server
    /// reads both template tables at startup, so an edit made during a
    /// playtest is not seen in it.
    ///
    /// The three display tables are client files like `Spell.dbc`, and the
    /// Assets workspace edits files, so the four survive for the spells'
    /// reason.
    pub fn survives_playtest(self) -> bool {
        matches!(
            self,
            Tool::Spells
                | Tool::Items
                | Tool::ItemSets
                | Tool::Quests
                | Tool::Tables
                | Tool::CreatureDisplays
                | Tool::ObjectDisplays
                | Tool::ItemDisplays
                | Tool::Assets
        )
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
    /// Only a [`Surface::Middle`] tool works while a playtest is running; see
    /// [`Tool::survives_playtest`].
    pub fn surface(self) -> Surface {
        match self {
            Tool::Spells
            | Tool::Items
            | Tool::ItemSets
            | Tool::Quests
            | Tool::Tables
            | Tool::Zones
            | Tool::CreatureData
            | Tool::GameObjectData
            | Tool::CreatureDisplays
            | Tool::ObjectDisplays
            | Tool::ItemDisplays
            | Tool::Assets => Surface::Middle,
            // A light's row, a flight path node or point, a creature's spawn
            // row and a game object's are each a place in the world, so each
            // is picked with the pointer and puts its form where every other
            // selection's numbers go.
            Tool::Lights
            | Tool::Flightpaths
            | Tool::Triggers
            | Tool::Graveyards
            | Tool::Creatures
            | Tool::GameObjects => Surface::Inspector,
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
            Tool::Spells => &crate::tables::tabs::SPELL_TABS,
            Tool::Lights => &crate::tables::tabs::LIGHT_TABS,
            Tool::Flightpaths => &crate::tables::tabs::TAXI_TABS,
            Tool::ItemSets => &crate::tables::tabs::SET_TABS,
            Tool::Zones => &crate::tables::tabs::ZONE_TABS,
            Tool::Triggers => &crate::tables::tabs::TRIGGER_TABS,
            Tool::Graveyards => &crate::tables::tabs::SAFE_LOC_TABS,
            Tool::CreatureDisplays => &crate::tables::tabs::CREATURE_DISPLAY_TABS,
            Tool::ObjectDisplays => &crate::tables::tabs::OBJECT_DISPLAY_TABS,
            Tool::ItemDisplays => &crate::tables::tabs::ITEM_DISPLAY_TABS,
            _ => &[],
        }
    }

    /// The tools that make up this tool's workspace, in the order the strip
    /// at the head of its list draws them, or an empty list for a workspace
    /// of one tool.
    ///
    /// Items and Sets are one workspace of two tools because they are two
    /// documents: an item is a row of the server's database and a set is a
    /// DBC table, so each has its own list, form and save. The spell
    /// workspace's skill tables are tabs of one tool instead ([`Tool::tabs`]),
    /// since they are tables in the same browser.
    pub fn parts(self) -> &'static [(&'static str, Tool)] {
        match self {
            Tool::Items | Tool::ItemSets | Tool::ItemDisplays => &[
                ("Items", Tool::Items),
                ("Sets", Tool::ItemSets),
                ("Displays", Tool::ItemDisplays),
            ],
            Tool::CreatureData | Tool::CreatureDisplays => &[
                ("Templates", Tool::CreatureData),
                ("Displays", Tool::CreatureDisplays),
            ],
            Tool::GameObjectData | Tool::ObjectDisplays => &[
                ("Templates", Tool::GameObjectData),
                ("Displays", Tool::ObjectDisplays),
            ],
            _ => &[],
        }
    }

    /// The tool the top bar's workspace control shows for this one: the
    /// first of its [`Tool::parts`], or itself.
    pub fn workspace(self) -> Tool {
        self.parts().first().map(|&(_, tool)| tool).unwrap_or(self)
    }

    /// The workspace a DBC table is browsed in, for a reference followed
    /// from another form: Sets for `ItemSet`, Zones for `AreaTable`, Spells
    /// for the spell chain and the skill tables, and Tables for everything
    /// else.
    pub fn for_table(table: &str) -> Tool {
        let in_spells = table == "Spell"
            || crate::tables::tabs::SPELL_TABS.iter().any(|&(_, name)| name == table)
            || vale_assets::tables::schema::chain_for("Spell").contains(&table);
        match table {
            "ItemSet" => Tool::ItemSets,
            "AreaTable" => Tool::Zones,
            "CreatureDisplayInfo" | "CreatureDisplayInfoExtra" | "CreatureModelData" => {
                Tool::CreatureDisplays
            }
            "GameObjectDisplayInfo" => Tool::ObjectDisplays,
            "ItemDisplayInfo" => Tool::ItemDisplays,
            _ if in_spells => Tool::Spells,
            _ => Tool::Tables,
        }
    }

    /// What to call it in an interface.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Terrain => "Terrain",
            Tool::Grade => "Grade",
            Tool::Road => "Road",
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
            Tool::Chunks => "Chunks",
            Tool::ItemSets => "Sets",
            Tool::Tables => "Tables",
            // "Zones" and not "Areas": the rail's Areas tool paints an area
            // onto chunks, and this workspace is what a zone and its
            // sub-areas are. Two subjects with one name would be two answers
            // to `--tool areas`.
            Tool::Zones => "Zones",
            Tool::Triggers => "Triggers",
            Tool::Graveyards => "Graveyards",
            // The top bar calls these two workspaces Creatures and Game
            // Objects. The tool names differ because `--tool` matches a name,
            // and the rail's spawn tools are already Creatures and Objects.
            Tool::CreatureData => "Creature Data",
            Tool::GameObjectData => "Game Object Data",
            Tool::CreatureDisplays => "Creature Display Info",
            Tool::ObjectDisplays => "Game Object Display Info",
            Tool::ItemDisplays => "Item Display Info",
            Tool::Assets => "Assets",
        }
    }
}

pub struct ToolPlugin;

impl Plugin for ToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tool>()
            .init_resource::<Opening>()
            .add_systems(Update, (open_tiles, close_tiles, modes).chain())
            .add_systems(Update, (crate::map::maps::follow_map_table, crate::map::maps::read_edited_map))
            .add_systems(Update, release_held);
        app.add_plugins((
            ground::terrain::TerrainToolPlugin,
            ground::shading::ShadingToolPlugin,
            ground::textures::TextureToolPlugin,
            ground::holes::HoleToolPlugin,
            ground::areas::AreaToolPlugin,
            places::lights::LightToolPlugin,
            ground::water::WaterToolPlugin,
            placements::doodads::DoodadToolPlugin,
            placements::wmos::WmoToolPlugin,
            placements::place::PlaceToolPlugin,
            crate::map::sweep::SweepPlugin,
            placements::gizmo::GizmoToolPlugin,
            // The two point-and-apply terrain tools as one entry, which keeps
            // the tuple within what `add_plugins` accepts.
            (ground::grade::GradeToolPlugin, ground::road::RoadToolPlugin),
            crate::tables::TableToolPlugin,
            crate::map::tiles::TilePlugin,
        ));
        // A second call, because `add_plugins` takes at most fifteen. The
        // first tuple is full; further tool plugins go in this one.
        app.add_plugins((
            spawns::creatures::CreatureToolPlugin,
            spawns::gameobjects::GameObjectToolPlugin,
            spawns::waypoints::WaypointToolPlugin,
            // The item tool and the two workspaces' lists as one entry, which
            // keeps the tuple within what `add_plugins` accepts.
            (
                crate::subjects::items::ItemToolPlugin,
                crate::subjects::creature_data::CreatureDataPlugin,
                crate::subjects::gameobject_data::GameObjectDataPlugin,
                crate::import::assets::AssetsPlugin,
            ),
            crate::subjects::quests::QuestToolPlugin,
            crate::subjects::loot::LootToolPlugin,
            // Behaviour, conditions and gossip as one entry, which keeps the
            // tuple within what `add_plugins` accepts: three windows over the
            // server's rows that open from any form.
            (
                crate::subjects::behaviour::BehaviourToolPlugin,
                crate::subjects::conditions::ConditionToolPlugin,
                crate::subjects::gossip::GossipToolPlugin,
            ),
            crate::subjects::services::ServicesToolPlugin,
            spawns::spawn::SpawnKeysPlugin,
            placements::group::GroupPlugin,
            measure::MeasureToolPlugin,
            places::flightpaths::FlightpathToolPlugin,
            // The two place tools as one entry, which keeps the tuple within
            // what `add_plugins` accepts.
            (places::triggers::TriggerToolPlugin, places::graveyards::GraveyardToolPlugin),
            ground::chunks::ChunkToolPlugin,
            guides::GuidesPlugin,
        ));
    }
}

/// End the history's hold on a gesture once the left button is up.
///
/// A pointer drag that holds a gesture releases it on its own release frame.
/// This covers the drags that end another way: the tool switched, a playtest
/// begun, the selection dropped. A hold left on would fold the next edit of the
/// same subject into the drag's entry. See `vale_edit::undo::History::hold`.
fn release_held(mut session: Option<ResMut<EditSession>>, buttons: Res<ButtonInput<MouseButton>>) {
    if buttons.pressed(MouseButton::Left) {
        return;
    }
    if let Some(session) = session.as_mut() {
        session.bypass_change_detection().history.release();
    }
}

/// Which subject the shell opens on over a playtest.
///
/// The tools that keep the viewport cannot act during a playtest, because the
/// editor's own tiles were despawned when it started. Opening the shell on
/// the terrain brush would show an inspector for a brush that does nothing,
/// and the game behind. A workspace works ([`Tool::survives_playtest`]), so a
/// viewport tool is replaced by the first of them and a workspace is kept.
pub(crate) fn open_on(tool: Tool) -> Tool {
    match tool.survives_playtest() {
        true => tool,
        false => Tool::Spells,
    }
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
    /// draws. See `vale_edit::ops::brush::Falloff::over`.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The shell over a playtest opens on a subject that works there.
    ///
    /// Every tool that keeps the viewport is unavailable during a playtest,
    /// because the editor's own tiles were despawned when it started. Opening
    /// the panels on one would show an inspector about a brush that does
    /// nothing. A workspace is left where it is.
    #[test]
    fn the_shell_opens_on_a_workspace() {
        for tool in ALL {
            let opened = open_on(tool);
            assert!(
                opened.survives_playtest() && opened.covers_viewport(),
                "{} opened the shell on {}, which keeps the viewport",
                tool.name(),
                opened.name()
            );
            if tool.survives_playtest() {
                assert_eq!(opened, tool, "a workspace is left where it is");
            }
        }
    }

    /// The tools that work during a playtest are four workspaces, with the
    /// Sets part of Items, and nothing that keeps the viewport. Zones,
    /// Creatures and Game Objects are the workspaces that do not.
    #[test]
    fn only_the_workspaces_survive_a_playtest() {
        for tool in ALL {
            let read_at_startup = matches!(
                tool,
                Tool::Zones | Tool::CreatureData | Tool::GameObjectData
            );
            assert_eq!(
                tool.survives_playtest(),
                tool.covers_viewport() && !read_at_startup,
                "{}",
                tool.name()
            );
        }
        assert_eq!(open_on(Tool::Zones), Tool::Spells);
        assert!(!Tool::Lights.survives_playtest());
        assert!(!Tool::Flightpaths.survives_playtest());
        assert_eq!(open_on(Tool::Lights), Tool::Spells);
        assert_eq!(open_on(Tool::Items), Tool::Items);
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
            // The Tables workspace starts on no table and has no tab: its
            // list is the tables themselves.
            if table == crate::tables::tabs::ANY {
                assert!(tabs.is_empty(), "{} has tabs", tool.name());
                continue;
            }
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
            // The Assets workspace's subject is the project folder, not a
            // table.
            if tool.covers_viewport() && tool != Tool::Assets {
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
                // The Assets workspace's subject is the project folder.
                Surface::Middle if tool == Tool::Assets => {}
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

    /// The Items workspace is two tools and every other workspace is one:
    /// the top bar shows Items for both of its parts, and a DBC table
    /// followed from another form opens in the workspace that holds it.
    #[test]
    fn a_workspace_is_its_parts_and_a_table_has_a_workspace() {
        assert_eq!(Tool::ItemSets.workspace(), Tool::Items);
        assert_eq!(Tool::Items.workspace(), Tool::Items);
        assert_eq!(Tool::Items.parts(), Tool::ItemSets.parts());
        for tool in ALL {
            match tool.parts() {
                [] => assert_eq!(tool.workspace(), tool),
                parts => {
                    assert!(parts.iter().any(|&(_, part)| part == tool), "{}", tool.name());
                    assert!(parts.iter().all(|&(_, part)| part.covers_viewport()));
                }
            }
        }
        // Sets is a file and needs no database; Items is rows and does.
        assert_eq!(Tool::ItemSets.table(), Some("ItemSet"));
        assert!(Tool::ItemSets.server_table().is_none());
        assert!(Tool::ItemSets.survives_playtest() && Tool::Tables.survives_playtest());

        assert_eq!(Tool::for_table("ItemSet"), Tool::ItemSets);
        for table in ["Spell", "SpellVisualKit", "SpellIcon", "SkillLine", "SkillLineAbility"] {
            assert_eq!(Tool::for_table(table), Tool::Spells, "{table}");
        }
        for table in ["Faction", "Map", "Lock"] {
            assert_eq!(Tool::for_table(table), Tool::Tables, "{table}");
        }
        assert_eq!(Tool::for_table("AreaTable"), Tool::Zones);
        assert_eq!(Tool::Zones.table(), Some("AreaTable"));
        assert!(Tool::Zones.covers_viewport());
        // The skill tables are the spell workspace's second row of tabs.
        let tabs = Tool::Spells.tabs();
        assert_eq!(tabs.len(), crate::tables::tabs::TAB_ROW + 3);
        assert_eq!(tabs[crate::tables::tabs::TAB_ROW].1, "SkillLine");
    }

}
