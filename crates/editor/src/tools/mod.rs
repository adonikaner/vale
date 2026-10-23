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
//! points.rs      points on the map picked and dragged by the pointer: the
//!                aim, the drag and the list the place tools share
//! measure.rs     a pointer tool that changes nothing and reports what is at a
//!                point, and the distance and facing between two points
//! guides.rs      what is drawn on the ground to read it by, under every
//!                tool: the chunk and tile grid, the ground that is too steep,
//!                contour lines and the texture count per chunk. Switches on
//!                the view bar; nothing here edits
//! shortcuts.rs   the keys that work whichever tool is chosen: undo, redo,
//!                save, the playtest, and the shell over one
//! modes.rs       the number row: when a digit reaches the chosen tool, read
//!                by each tool's own system that maps it onto its modes
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

pub(crate) mod ground;
pub(crate) mod guides;
pub(crate) mod measure;
pub(crate) mod modes;
pub(crate) mod places;
pub(crate) mod points;
pub(crate) mod placements;
pub(crate) mod shortcuts;
pub(crate) mod spawns;

use crate::map::open::{close_tiles, open_tiles, Opening};
use crate::session::EditSession;
use bevy::prelude::*;
use bevy_egui::egui;

/// Which tool the pointer is holding.
///
/// The camera, the pick and the panels are the same whichever tool is chosen;
/// the tool decides only what a left-drag in the viewport does. The list is
/// expected to grow; [`crate::ui::shell::rail`] names the tools
/// that do not exist yet.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tool {
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
    /// screen — see [`crate::subjects::templates`].
    ///
    /// The same row the creature tool's template window edits, reached by
    /// name rather than by clicking a spawn. A template has no place in the
    /// world, so the workspace replaces the viewport. It does not survive a
    /// playtest, because the server reads `creature_template` at startup.
    CreatureData,
    /// The Game Objects workspace: every `gameobject_template` row, on
    /// [`Tool::CreatureData`]'s terms — see [`crate::subjects::templates`].
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
///
/// The surface is also the line between the two sorts of entry [`Tool`]
/// holds: a pointer tool on the world ([`Surface::World`] and
/// [`Surface::Inspector`]) and a workspace that replaces the viewport
/// ([`Surface::Middle`]). They are one type because the chosen entry is one
/// value: the `Tool` resource, the pop-out windows' list, `--tool`, the
/// rail's last world tool and every panel's switch request each hold or set
/// one of either sort. Two types would make each of those an enum over the
/// two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
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

/// Where the shell offers a tool. See [`crate::ui::shell::rail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    /// A button over the viewport's top-left corner, for the tools that
    /// change nothing.
    Corner,
    /// A tile on the rail, under the heading named.
    Rail(&'static str),
    /// A workspace on the top bar, under the name given. A workspace of
    /// several parts is a menu of them.
    Bar(&'static str),
    /// A part of a workspace of several parts, reached through that
    /// workspace's menu ([`Spec::menu`]) and the strip at the head of its
    /// list.
    Part,
}

/// A part's entry in its workspace's menu.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Entry {
    /// The name the menu gives it, and the key an icon is looked up by.
    pub(crate) name: &'static str,
    /// What it does, for the tooltip.
    pub(crate) about: &'static str,
}

/// A tool's inspector panel. [`crate::ui::tools::draw`] draws the header and
/// the undo row, then this.
pub(crate) type InspectorPanel =
    fn(&mut egui::Ui, crate::ui::tools::Subject<'_>, &mut crate::ui::Editing<'_>);

/// What a [`Surface::Middle`] tool puts in the middle of a window. See
/// [`crate::ui::workspace::workspace`].
pub(crate) type WorkspacePanel = fn(&mut egui::Ui, Tool, crate::ui::workspace::Bench<'_, '_, '_>);

/// What the editor knows about one tool: a row of [`SPECS`].
///
/// The methods on [`Tool`] read it, and so do the rail and the top bar
/// ([`Spec::place`]), the inspector ([`Spec::inspector`]), the middle of the
/// window ([`Spec::workspace`]) and the pop-out windows ([`Spec::window`]).
/// A new tool is a variant, its index in [`Tool::at`], its row here, its
/// plugin and its panel functions.
#[derive(Clone, Copy)]
pub(crate) struct Spec {
    pub(crate) tool: Tool,
    /// See [`Tool::name`].
    pub(crate) name: &'static str,
    /// See [`Tool::surface`].
    pub(crate) surface: Surface,
    /// See [`Tool::edits`].
    pub(crate) edits: bool,
    /// See [`Tool::table`].
    pub(crate) table: Option<&'static str>,
    /// See [`Tool::server_table`].
    pub(crate) server_table: Option<&'static str>,
    /// See [`Tool::survives_playtest`].
    pub(crate) survives_playtest: bool,
    /// See [`Tool::tabs`].
    pub(crate) tabs: &'static [(&'static str, &'static str)],
    /// Whether a table on the chain of [`Spec::table`]
    /// (`vale_assets::tables::schema::chain_for`) is browsed here when a
    /// reference is followed to it, as the tables of its tabs are. See
    /// [`Tool::for_table`].
    pub(crate) chain: bool,
    /// See [`Tool::parts`].
    pub(crate) parts: &'static [(&'static str, Tool)],
    /// Where the shell offers it.
    pub(crate) place: Place,
    /// What it does, for the tooltip on its tile, corner button or top-bar
    /// button. Empty for a [`Place::Part`], whose text is its menu entry's.
    pub(crate) about: &'static str,
    /// Its entry in its workspace's menu, for a part of a workspace of
    /// several parts.
    pub(crate) menu: Option<Entry>,
    /// The pop-out window it shares with the other tools over the same
    /// state, or `None` for a tool with no state a workspace holds. See
    /// [`crate::popout::Group`].
    pub(crate) window: Option<crate::popout::Group>,
    /// What the inspector shows while it is chosen.
    pub(crate) inspector: InspectorPanel,
    /// What the middle of the window shows while it is chosen. `Some` for a
    /// [`Surface::Middle`] tool alone.
    pub(crate) workspace: Option<WorkspacePanel>,
}

/// A row for a pointer tool on the world: drawn over the viewport, editing,
/// with no table, no window and unavailable during a playtest. A row built
/// from it states what differs.
const fn world(
    tool: Tool,
    name: &'static str,
    place: Place,
    about: &'static str,
    inspector: InspectorPanel,
) -> Spec {
    Spec {
        tool,
        name,
        surface: Surface::World,
        edits: true,
        table: None,
        server_table: None,
        survives_playtest: false,
        tabs: &[],
        chain: false,
        parts: &[],
        place,
        about,
        menu: None,
        window: None,
        inspector,
        workspace: None,
    }
}

/// A row for a workspace that replaces the viewport. A row built from it
/// states its table, its window and whether it survives a playtest.
const fn middle(
    tool: Tool,
    name: &'static str,
    place: Place,
    about: &'static str,
    inspector: InspectorPanel,
    workspace: WorkspacePanel,
) -> Spec {
    Spec {
        surface: Surface::Middle,
        workspace: Some(workspace),
        ..world(tool, name, place, about, inspector)
    }
}

/// A row for a workspace drawn by the table browser: its first table, its
/// tabs, and the browser's window.
const fn browsed(
    tool: Tool,
    name: &'static str,
    place: Place,
    about: &'static str,
    table: &'static str,
    tabs: &'static [(&'static str, &'static str)],
) -> Spec {
    Spec {
        table: Some(table),
        tabs,
        window: Some(crate::popout::Group::Tables),
        ..middle(tool, name, place, about, crate::ui::tables::inspect, crate::ui::tables::workspace)
    }
}

/// The Items workspace's parts, in the order its strip and menu list them.
const ITEM_PARTS: [(&str, Tool); 3] = [
    ("Items", Tool::Items),
    ("Sets", Tool::ItemSets),
    ("Displays", Tool::ItemDisplays),
];

/// The Creatures workspace's parts.
const CREATURE_PARTS: [(&str, Tool); 2] = [
    ("Templates", Tool::CreatureData),
    ("Displays", Tool::CreatureDisplays),
];

/// The Game Objects workspace's parts.
const OBJECT_PARTS: [(&str, Tool); 2] = [
    ("Templates", Tool::GameObjectData),
    ("Displays", Tool::ObjectDisplays),
];

/// Every tool's row, in [`Tool::at`]'s order, which is also the order the
/// shell draws them in: the corner, the rail's three headings, then the top
/// bar's workspaces each followed by its other parts.
pub(crate) const SPECS: &[Spec] = &[
    Spec {
        edits: false,
        ..world(
            Tool::Select,
            "Select",
            Place::Corner,
            "Reports the position under the pointer. Changes nothing.",
            crate::ui::tools::pointer,
        )
    },
    Spec {
        edits: false,
        ..world(
            Tool::Measure,
            "Measure",
            Place::Corner,
            "Reports the position, ground height, tile, chunk, area, slope and water \
             at a point. Click two points for the distance, rise and facing between them; \
             Escape clears. Changes nothing.",
            crate::ui::tools::measure::inspect,
        )
    },
    world(
        Tool::Terrain,
        "Terrain",
        Place::Rail("Terrain"),
        "Raises, lowers, flattens and smooths the ground. Writes MCVT, the \
         145 heights of a chunk.",
        crate::ui::tools::ground::terrain::inspect,
    ),
    world(
        Tool::Grade,
        "Grade",
        Place::Rail("Terrain"),
        "Builds a straight ramp between two points. Click a start, click an \
         end, set the width, then apply.",
        crate::ui::tools::ground::grade::inspect,
    ),
    world(
        Tool::Road,
        "Road",
        Place::Rail("Terrain"),
        "Builds a road along a run of points: grades the ground along it \
         and paints a surface texture and a verge texture. Click to add \
         points, then apply.",
        crate::ui::tools::ground::road::inspect,
    ),
    world(
        Tool::Shading,
        "Shading",
        Place::Rail("Terrain"),
        "Paints MCCV, the per-vertex colour of the ground. Each value is a \
         multiplier: 1.00 leaves a vertex unchanged. The reference client \
         does not read MCCV.",
        crate::ui::tools::ground::shading::inspect,
    ),
    world(
        Tool::Textures,
        "Textures",
        Place::Rail("Terrain"),
        "Paints tilesets onto the ground. A painted tileset becomes a \
         texture layer of every chunk the brush covers.",
        crate::ui::tools::ground::textures::inspect,
    ),
    world(
        Tool::Holes,
        "Holes",
        Place::Rail("Terrain"),
        "Cuts holes in the ground, sixteen squares per chunk. Click cuts a \
         square; shift-click fills it.",
        crate::ui::tools::ground::holes::inspect,
    ),
    world(
        Tool::Water,
        "Water",
        Place::Rail("Terrain"),
        "Paints MCLQ water at a set level. Shift clears it. The level can be \
         taken from the water already under the pointer.",
        crate::ui::tools::ground::water::inspect,
    ),
    world(
        Tool::Areas,
        "Areas",
        Place::Rail("Terrain"),
        "Assigns an AreaTable row to each chunk. The client takes the zone \
         and sub-area a character is in from this field.",
        crate::ui::tools::ground::areas::inspect,
    ),
    world(
        Tool::Chunks,
        "Chunks",
        Place::Rail("Terrain"),
        "Selects chunks: click one, drag a block on the ground, shift adds. \
         Then writes one area, base texture, hole mask or impassable flag \
         to all of them as one undo entry.",
        crate::ui::tools::ground::chunks::inspect,
    ),
    world(
        Tool::Doodads,
        "Doodads",
        Place::Rail("World"),
        "MDDF: one record per placed M2 model. Select, move, rotate, \
         scale.",
        crate::ui::tools::placements::doodads::inspect,
    ),
    world(
        Tool::Wmos,
        // "WMO" rather than "Buildings": it is what `MWMO` names, what
        // `MODF` places and what `vale wmos` checks, and half of what a
        // `.wmo` holds is not a building — a bridge, a gate, a canal.
        "WMO",
        Place::Rail("World"),
        "MODF: one record per placed WMO building. Select, move, rotate, \
         remove. No scale: the 1.12 record has no scale field.",
        crate::ui::tools::placements::wmos::inspect,
    ),
    // A light's row, a flight path node or point, a trigger and a safe place
    // are each a place in the world, so each is picked with the pointer and
    // puts its form where every other selection's numbers go.
    Spec {
        surface: Surface::Inspector,
        table: Some("Light"),
        tabs: &crate::tables::tabs::LIGHT_TABS,
        window: Some(crate::popout::Group::Tables),
        ..world(
            Tool::Lights,
            "Lights",
            Place::Rail("World"),
            "Light.dbc and the tables it references (sun, fill light, sky dome, \
             fog and water colour) per map and per zone. Each light is a \
             sphere in the world, selected and dragged like a doodad.",
            crate::ui::tools::places::lights::inspect,
        )
    },
    Spec {
        surface: Surface::Inspector,
        table: Some("TaxiNodes"),
        tabs: &crate::tables::tabs::TAXI_TABS,
        window: Some(crate::popout::Group::Tables),
        ..world(
            Tool::Flightpaths,
            // "Taxi" is the prefix of the three tables it edits: TaxiNodes,
            // TaxiPath and TaxiPathNode. One word, so `--tool taxi` needs no
            // quoting.
            "Taxi",
            Place::Rail("World"),
            "The taxi network: TaxiNodes, TaxiPath and TaxiPathNode. Nodes and \
             the paths between them are drawn on the map; drag a node or a \
             point, add points, connect two nodes, create a node.",
            crate::ui::tools::places::flightpaths::inspect,
        )
    },
    Spec {
        surface: Surface::Inspector,
        table: Some(vale_edit::dbc::places::TRIGGERS),
        tabs: &crate::tables::tabs::TRIGGER_TABS,
        window: Some(crate::popout::Group::Tables),
        ..world(
            Tool::Triggers,
            "Triggers",
            Place::Rail("World"),
            "AreaTrigger.dbc: the spheres and boxes the client reports entering, \
             and the server's action for each: a teleport, an inn, a quest \
             objective. Drag a trigger, create one, change its shape.",
            crate::ui::tools::places::triggers::inspect,
        )
    },
    Spec {
        surface: Surface::Inspector,
        table: Some(vale_edit::dbc::places::SAFE_LOCS),
        tabs: &crate::tables::tabs::SAFE_LOC_TABS,
        window: Some(crate::popout::Group::Tables),
        ..world(
            Tool::Graveyards,
            "Graveyards",
            Place::Rail("World"),
            "WorldSafeLocs.dbc: the graveyards where a released spirit appears. \
             Drag a graveyard, create one, and link it to the zones it serves \
             (game_graveyard_zone rows in vmangos' database).",
            crate::ui::tools::places::graveyards::inspect,
        )
    },
    world(
        Tool::Sweep,
        "Sweep",
        Place::Rail("World"),
        "Finds and replaces a texture, model or building path across every \
         tile of the map: for example, a tileset retired from a zone, or one \
         tree model replaced across a forest.",
        crate::ui::map::sweep::inspect,
    ),
    // A creature's spawn row and a game object's are places too, for the
    // reason above.
    Spec {
        surface: Surface::Inspector,
        server_table: Some(vale_mangos::creature::SPAWN),
        ..world(
            Tool::Creatures,
            "Creatures",
            Place::Rail("Spawns"),
            "Every creature spawn on the map, drawn at its position. Click one \
             to open the spawn and its creature_template. Rows in vmangos' \
             database, so this needs a database connection.",
            crate::ui::tools::spawns::spawn::inspect::<spawns::creatures::CreatureSubject>,
        )
    },
    Spec {
        surface: Surface::Inspector,
        server_table: Some(vale_mangos::gameobject::SPAWN),
        ..world(
            Tool::GameObjects,
            // "Objects" is this repository's own word for them: `vale
            // objects` is the CLI check over the same population.
            // The rail's tooltip says game objects, which is vmangos' word.
            "Objects",
            Place::Rail("Spawns"),
            "Every game object spawn on the map (chests, doors, mining veins and \
             herbs, mailboxes, signs), drawn at its position. Click one to open \
             the spawn and its gameobject_template. Rows in vmangos' \
             database, so this needs a database connection.",
            crate::ui::tools::spawns::spawn::inspect::<spawns::gameobjects::GameObjectSubject>,
        )
    },
    Spec {
        survives_playtest: true,
        chain: true,
        ..browsed(
            Tool::Spells,
            "Spells",
            Place::Bar("Spells"),
            "Spell.dbc: 22,360 rows of 173 fields, named, typed and cross-referenced. \
             The workspace replaces the viewport, because a spell has no place in the \
             world.",
            "Spell",
            &crate::tables::tabs::SPELL_TABS,
        )
    },
    Spec {
        server_table: Some(vale_mangos::item::TEMPLATE),
        survives_playtest: true,
        parts: &ITEM_PARTS,
        menu: Some(Entry {
            name: "Items",
            about: "item_template: an item's stats, effects and price, with a preview of its \
                    display id. Rows in vmangos' database, so this needs a database connection.",
        }),
        window: Some(crate::popout::Group::Items),
        ..middle(
            Tool::Items,
            "Items",
            Place::Bar("Items"),
            "Items, item sets and item display info: item_template rows in vmangos' \
             database, and ItemSet.dbc and ItemDisplayInfo.dbc, which are client files \
             and need no database.",
            crate::ui::subjects::items::inspect,
            crate::ui::subjects::items::workspace,
        )
    },
    Spec {
        survives_playtest: true,
        parts: &ITEM_PARTS,
        menu: Some(Entry {
            name: "Item Sets",
            about: "ItemSet.dbc: the items in each set and the set bonuses. A client file; it \
                    needs no database.",
        }),
        ..browsed(Tool::ItemSets, "Sets", Place::Part, "", "ItemSet", &crate::tables::tabs::SET_TABS)
    },
    Spec {
        survives_playtest: true,
        parts: &ITEM_PARTS,
        menu: Some(Entry {
            name: "Item Display Info",
            about: "ItemDisplayInfo.dbc: an item's bag icon, the models it hangs on the wearer \
                    and the textures it paints on the body. A client file; it needs no database.",
        }),
        ..browsed(
            Tool::ItemDisplays,
            "Item Display Info",
            Place::Part,
            "",
            "ItemDisplayInfo",
            &crate::tables::tabs::ITEM_DISPLAY_TABS,
        )
    },
    Spec {
        server_table: Some(vale_mangos::quest::TEMPLATE),
        survives_playtest: true,
        window: Some(crate::popout::Group::Quests),
        ..middle(
            Tool::Quests,
            "Quests",
            Place::Bar("Quests"),
            "quest_template (objectives, text and rewards) and the four relation tables \
             that set which creatures and game objects start and end each quest. Rows \
             in vmangos' database, so this needs a database connection.",
            crate::ui::subjects::quests::inspect,
            crate::ui::subjects::quests::workspace,
        )
    },
    Spec {
        server_table: Some(vale_mangos::creature::TEMPLATE),
        parts: &CREATURE_PARTS,
        menu: Some(Entry {
            name: "Creature Templates",
            about: "creature_template: every creature the server loads, searched by name, \
                    subname or entry, with every column of the open row, its model, and its \
                    quests, loot, events, spells, gossip, vendor and trainer windows. Rows in \
                    vmangos' database, so this needs a database connection.",
        }),
        window: Some(crate::popout::Group::Creatures),
        ..middle(
            Tool::CreatureData,
            // The top bar calls these two workspaces Creatures and Game
            // Objects. The tool names differ because `--tool` matches a name,
            // and the rail's spawn tools are already Creatures and Objects.
            "Creature Data",
            Place::Bar("Creatures"),
            "Creature templates and creature display info: creature_template rows in \
             vmangos' database, and the client's CreatureDisplayInfo, \
             CreatureDisplayInfoExtra and CreatureModelData, which need no database.",
            crate::ui::subjects::templates::inspect::<spawns::creatures::CreatureSubject>,
            crate::ui::subjects::templates::workspace::<spawns::creatures::CreatureSubject>,
        )
    },
    Spec {
        survives_playtest: true,
        parts: &CREATURE_PARTS,
        menu: Some(Entry {
            name: "Creature Display Info",
            about: "CreatureDisplayInfo.dbc: a creature's model, scale and skins, with \
                    CreatureDisplayInfoExtra and CreatureModelData as two more tabs. A new skin \
                    for an existing model is a copied row with another texture. Client files; \
                    they need no database.",
        }),
        ..browsed(
            Tool::CreatureDisplays,
            "Creature Display Info",
            Place::Part,
            "",
            "CreatureDisplayInfo",
            &crate::tables::tabs::CREATURE_DISPLAY_TABS,
        )
    },
    Spec {
        server_table: Some(vale_mangos::gameobject::TEMPLATE),
        parts: &OBJECT_PARTS,
        menu: Some(Entry {
            name: "Game Object Templates",
            about: "gameobject_template: every game object the server loads, searched by name, \
                    type or entry, with every column of the open row, its model, and its quest \
                    and loot windows. Rows in vmangos' database, so this needs a database \
                    connection.",
        }),
        window: Some(crate::popout::Group::Objects),
        ..middle(
            Tool::GameObjectData,
            "Game Object Data",
            Place::Bar("Game Objects"),
            "Game object templates and game object display info: gameobject_template \
             rows in vmangos' database, and the client's GameObjectDisplayInfo, which \
             needs no database.",
            crate::ui::subjects::templates::inspect::<spawns::gameobjects::GameObjectSubject>,
            crate::ui::subjects::templates::workspace::<spawns::gameobjects::GameObjectSubject>,
        )
    },
    Spec {
        survives_playtest: true,
        parts: &OBJECT_PARTS,
        menu: Some(Entry {
            name: "Game Object Display Info",
            about: "GameObjectDisplayInfo.dbc: a game object's model and its ten sounds. A \
                    client file; it needs no database.",
        }),
        ..browsed(
            Tool::ObjectDisplays,
            "Game Object Display Info",
            Place::Part,
            "",
            "GameObjectDisplayInfo",
            &crate::tables::tabs::OBJECT_DISPLAY_TABS,
        )
    },
    browsed(
        Tool::Zones,
        // "Zones" and not "Areas": the rail's Areas tool paints an area
        // onto chunks, and this workspace is what a zone and its
        // sub-areas are. Two subjects with one name would be two answers
        // to `--tool areas`.
        "Zones",
        Place::Bar("Zones"),
        "AreaTable.dbc: zones and their sub-areas. Each row's name, parent zone, \
         music and ambience, duel and rest flags, area level and exploration bit. \
         Create a zone or sub-area here, then paint it onto the ground with the \
         Areas tool. The server loads its copy from area_template at startup.",
        crate::tables::zones::area::TABLE,
        &crate::tables::tabs::ZONE_TABS,
    ),
    // No one table: the workspace starts on the list of them.
    Spec {
        survives_playtest: true,
        ..browsed(
            Tool::Tables,
            "Tables",
            Place::Bar("Tables"),
            "Any DBC table: the 158 files of DBFilesClient\\, chosen from a list. A \
             table with a schema opens as named, typed fields and any other as numbered \
             fields. The workspace replaces the viewport.",
            crate::tables::tabs::ANY,
            &[],
        )
    },
    Spec {
        survives_playtest: true,
        window: Some(crate::popout::Group::Assets),
        ..middle(
            Tool::Assets,
            "Assets",
            Place::Bar("Assets"),
            "The files this project adds to the archives: imported textures, baked \
             models and the rest. Import Texture turns a PNG, TGA or BLP picture into a \
             BLP the 1.12.1 client reads. Every file here is listed in the editor's \
             pickers and published with the project.",
            crate::ui::import::assets::inspect,
            crate::ui::import::assets::workspace,
        )
    },
];

/// Every tool, listed once, in the order of [`SPECS`].
///
/// Three places enumerate the tools: `--tool`, the rail's own check, and any
/// later caller. Each kept, or would have kept, its own list. `--tool` had
/// eight entries when a ninth tool was added, and it was not updated, so the
/// flag, which exists for photographing panels, could not select the new tool
/// and reported nothing.
///
/// A fieldless enum cannot enumerate itself without a derive crate, so
/// [`Tool::at`] enforces the list: an exhaustive `match` that does not compile
/// until a new variant has an index, plus a test that [`SPECS`] is exactly
/// those indices in order.
pub(crate) const ALL: [Tool; SPECS.len()] = {
    let mut all = [Tool::Select; SPECS.len()];
    let mut i = 0;
    while i < SPECS.len() {
        all[i] = SPECS[i].tool;
        i += 1;
    }
    all
};

impl Tool {
    /// Where this tool sits in [`SPECS`] and [`ALL`].
    ///
    /// The `match` is exhaustive so that a new variant does not compile until
    /// it is given an index. That is the half of the check the compiler does;
    /// the other half is [`tests::the_list_holds_every_tool`].
    pub(crate) fn at(self) -> usize {
        match self {
            Tool::Select => 0,
            Tool::Measure => 1,
            Tool::Terrain => 2,
            // Beside the height brush, because it is the shape that brush
            // cannot make — see `SPECS`, whose order this must match.
            Tool::Grade => 3,
            Tool::Road => 4,
            Tool::Shading => 5,
            Tool::Textures => 6,
            Tool::Holes => 7,
            Tool::Water => 8,
            Tool::Areas => 9,
            Tool::Chunks => 10,
            Tool::Doodads => 11,
            Tool::Wmos => 12,
            Tool::Lights => 13,
            Tool::Flightpaths => 14,
            Tool::Triggers => 15,
            Tool::Graveyards => 16,
            Tool::Sweep => 17,
            Tool::Creatures => 18,
            Tool::GameObjects => 19,
            Tool::Spells => 20,
            Tool::Items => 21,
            Tool::ItemSets => 22,
            Tool::ItemDisplays => 23,
            Tool::Quests => 24,
            Tool::CreatureData => 25,
            Tool::CreatureDisplays => 26,
            Tool::GameObjectData => 27,
            Tool::ObjectDisplays => 28,
            Tool::Zones => 29,
            Tool::Tables => 30,
            Tool::Assets => 31,
        }
    }

    /// This tool's row of [`SPECS`].
    pub(crate) fn spec(self) -> &'static Spec {
        &SPECS[self.at()]
    }

    /// Which client table this tool edits, or `None` for a tool that edits the
    /// world.
    ///
    /// Where a tool's controls are drawn is [`Tool::surface`]'s answer, and
    /// whether it works during a playtest is [`Tool::survives_playtest`]'s. A
    /// tool that edits a chain of tables answers
    /// with the first: `Tool::Flightpaths` answers `TaxiNodes`, not
    /// `TaxiPath` or `TaxiPathNode`. See [`crate::tables`].
    pub(crate) fn table(self) -> Option<&'static str> {
        self.spec().table
    }

    /// Which table of the server's database this tool edits, or `None`.
    ///
    /// This is a separate question from [`Tool::table`], which names a client
    /// table.
    ///
    /// The name is the one the row writers use, so it is also the name a
    /// `.reload` is sent for.
    pub(crate) fn server_table(self) -> Option<&'static str> {
        self.spec().server_table
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
    pub(crate) fn survives_playtest(self) -> bool {
        self.spec().survives_playtest
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
    pub(crate) fn surface(self) -> Surface {
        self.spec().surface
    }

    /// Whether this tool can change anything. Select and Measure only report,
    /// so the inspector draws no undo row for them; `Ctrl+Z` still works.
    pub(crate) fn edits(self) -> bool {
        self.spec().edits
    }

    /// Whether the workspace replaces the viewport. [`Surface::Middle`] alone.
    pub(crate) fn covers_viewport(self) -> bool {
        self.surface() == Surface::Middle
    }

    /// The tables this tool's workspace offers as tabs, in the order the chain
    /// is walked.
    ///
    /// Per tool rather than one list, because each data tool walks its own
    /// chain: `Spell`, `Light` and `TaxiNodes` each start one. A table
    /// reached by following a reference, such as `SpellIcon` or `LightSkybox`,
    /// is not a tab and highlights none; the browser still navigates to it.
    pub(crate) fn tabs(self) -> &'static [(&'static str, &'static str)] {
        self.spec().tabs
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
    pub(crate) fn parts(self) -> &'static [(&'static str, Tool)] {
        self.spec().parts
    }

    /// The tool the top bar's workspace control shows for this one: the
    /// first of its [`Tool::parts`], or itself.
    pub(crate) fn workspace(self) -> Tool {
        self.parts().first().map(|&(_, tool)| tool).unwrap_or(self)
    }

    /// The workspace a DBC table is browsed in, for a reference followed
    /// from another form: Sets for `ItemSet`, Zones for `AreaTable`, Spells
    /// for the spell chain and the skill tables, and Tables for everything
    /// else.
    ///
    /// A workspace whose tabs name the table comes first, then one whose
    /// chain holds it ([`Spec::chain`]).
    pub(crate) fn for_table(table: &str) -> Tool {
        let workspaces = || SPECS.iter().filter(|spec| spec.surface == Surface::Middle);
        let on_tabs = |spec: &&Spec| {
            spec.table == Some(table) || spec.tabs.iter().any(|&(_, name)| name == table)
        };
        let on_chain = |spec: &&Spec| {
            spec.chain
                && spec
                    .table
                    .is_some_and(|first| vale_assets::tables::schema::chain_for(first).contains(&table))
        };
        workspaces()
            .find(on_tabs)
            .or_else(|| workspaces().find(on_chain))
            .map_or(Tool::Tables, |spec| spec.tool)
    }

    /// What to call it in an interface.
    pub(crate) fn name(self) -> &'static str {
        self.spec().name
    }
}

pub(crate) struct ToolPlugin;

impl Plugin for ToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tool>()
            .init_resource::<Opening>()
            .add_systems(Update, (open_tiles, close_tiles).chain())
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
                crate::subjects::templates::TemplatesPlugin::<spawns::creatures::CreatureSubject>::default(),
                crate::subjects::templates::TemplatesPlugin::<spawns::gameobjects::GameObjectSubject>::default(),
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
pub(crate) enum Wheel {
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
    pub(crate) fn held(keys: &ButtonInput<KeyCode>) -> Option<Wheel> {
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
pub(crate) const CORE: std::ops::RangeInclusive<f32> = 0.0..=0.9;

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

    /// [`ALL`], and the [`SPECS`] it is read from, is every tool, in index
    /// order.
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

    /// [`SPECS`] has one row per tool, each at its tool's index.
    ///
    /// The rows are read by index ([`Tool::spec`]), so a row out of place
    /// gives one tool another's name, panel and table.
    #[test]
    fn the_specs_have_one_row_per_tool_in_order() {
        for (i, spec) in SPECS.iter().enumerate() {
            assert_eq!(spec.tool.at(), i, "{} is in row {i}", spec.name);
        }
        let highest = SPECS.iter().map(|spec| spec.tool.at()).max().unwrap_or(0);
        assert_eq!(SPECS.len(), highest + 1);
    }

    /// A tool has a workspace exactly when it replaces the viewport, and a
    /// tool with a client table shares the table browser's window.
    #[test]
    fn a_spec_agrees_with_itself() {
        use crate::popout::Group;
        for spec in SPECS {
            assert_eq!(
                spec.workspace.is_some(),
                spec.surface == Surface::Middle,
                "{}",
                spec.name
            );
            if spec.table.is_some() {
                assert_eq!(spec.window, Some(Group::Tables), "{}", spec.name);
            }
            if spec.window.is_some() && spec.window != Some(Group::Tables) {
                assert!(spec.table.is_none() && spec.surface == Surface::Middle, "{}", spec.name);
            }
            assert_eq!(spec.place == Place::Part, spec.about.is_empty(), "{}", spec.name);
            assert_eq!(
                spec.menu.is_some(),
                !spec.parts.is_empty(),
                "{} is a part of a workspace without a menu entry, or the reverse",
                spec.name
            );
        }
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
