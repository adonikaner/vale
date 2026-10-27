//! The world editor.
//!
//! ```text
//! args/        the command line: every flag, its default, and the parse
//! camera.rs    the free camera, which the client does not have
//! context.rs   the right-click menu over the viewport: when a click opens
//!              one and what it is about
//! pick.rs      what the camera points at: the edited ground, and the
//!              buildings and scenery standing on it
//! places.rs    the maps the archives name, and the zones of a map and the
//!              middle of each one, for the panel's "go to"
//! playtest.rs  switching from editing to playing and back; every other file
//!              here asks this state whether the editor is in control
//! session/     what is open: the project, the tiles, the history, and the
//!              overlay the archives are read through
//! jobs.rs      slow work, run off the main thread, with a progress bar
//! instruments/ the shot, the tour and the heap census: runs that measure the
//!              editor rather than edit anything
//! preview/     the three rigs that drive the client's passes with no server:
//!              the spell stage, the attachment lab and the portraits
//! popout.rs    a workspace in an OS window of its own, beside the main
//!              window's world
//! prefs/       this machine's own choices, kept in files under Edit\:
//!              favourite models and camera bookmarks
//! navmesh.rs   the server's navmesh drawn over the ground: the mmaps tiles
//!              around the camera, coloured by what a path query makes of
//!              each polygon
//! marks.rs     what the tools draw in the world: handles, domes, dashed
//!              rings, volumes, as meshes the ground can hide
//! tablet.rs    drawing tablet input: pen taps and pressure from the pen's
//!              pointer messages, and pointer motion read from the cursor's
//!              position while a pen is in range
//! tools/       what the pointer does when a button is held, one file per tool
//! subjects/    the server-row subjects that have no place in the world:
//!              items, quests, loot, services, addons, behaviour, gossip, conditions,
//!              display ids, and the two template workspaces
//! tables/      the client's own tables: which row is open, and what an edit
//!              to a field is
//! map/         the map as a whole: a new map, a new tile, a
//!              find-and-replace over every tile, and the tiles opened for
//!              editing around the focus
//! import/      files brought into the project: a tile's heights and blends
//!              as PNG, and the Assets workspace's textures and models
//! ui/          the panels: the shell, the theme, and one file per region of it
//! server/      what is sent to the server a playtest runs against; the one
//!              destination of an edit that is not a file
//! ```
//!
//! ## The editor is the client's app with editor plugins added
//!
//! [`run`] builds the same app `vale_client::run` does, through the same
//! three functions ([`vale_client::app`]), and then adds [`EditorPlugins`].
//! Every pass that draws the world is the client's own. A second copy of a
//! terrain mesher, an M2 material or a light chain would be a second copy of
//! its rule, and that copy becomes wrong when the original is corrected.
//!
//! The editor adds four things the client does not need: a camera that is not
//! attached to a character, a pointer that resolves to a place in the world,
//! tools that change the files behind that place, and panels.
//!
//! Nothing in `vale-client` names this crate. The seams it uses were added
//! to the client for it and are useful without it:
//!
//! * [`vale_client::render::focus::WorldFocus`] — the streaming passes read
//!   the place they draw around from a resource rather than from a socket, so a
//!   host with no server can open a map. The client's own writer of that
//!   resource does nothing when there is no session, so the two hosts share it.
//! * [`vale_client::app`] — the app is built in three steps, so a second host
//!   composes it instead of copying the measured settings in it.
//! * `vale_assets::archive::Overlay` — a function consulted before the
//!   archives, so an unsaved tile is read in its edited form.
//! * `vale_client::render::terrain::LoadedTiles::reload` — one tile is
//!   dropped and read again, which puts an edit on the screen.
//!
//! ## Playtesting uses the client's own login
//!
//! The app contains the whole client, including `Interface\FrameXML\`, so
//! logging in is the client logging in: the session takes the focus back, the
//! camera moves to the character, and the editor's panel becomes a bar with the
//! exit button on it. The Playtest button on the panel, or `Ctrl+P`, enters and
//! leaves a playtest. [`playtest`] documents the three states and the switches
//! each of them sets.
//!
//! The local simulation walks on the edited ground: the session's own
//! `MapTerrain` is opened with the editor's overlay, so an unsaved tile is the
//! tile the character stands on. An edit does not reach the server's
//! collision, which vmangos reads from `vmaps` and `mmaps` built from these
//! same ADTs. Ground raised here is still at the old height on the server, and
//! a character who walks far enough onto it is corrected back down.

pub(crate) mod args;
pub(crate) mod camera;
pub(crate) mod context;
pub(crate) mod import;
pub(crate) mod instruments;
pub(crate) mod jobs;
pub(crate) mod map;
pub(crate) mod marks;
pub(crate) mod navmesh;
pub(crate) mod pick;
pub(crate) mod places;
pub(crate) mod playtest;
pub(crate) mod popout;
pub(crate) mod prefs;
pub(crate) mod preview;
pub(crate) mod server;
pub(crate) mod session;
pub(crate) mod subjects;
pub(crate) mod tables;
pub(crate) mod tablet;
pub(crate) mod tools;
/// The heap census, built only with the `heap-census` feature. See [`instruments::heap`],
/// and the manifest, which states why the feature is off by default.
#[cfg(feature = "heap-census")]
#[global_allocator]
static HEAP: instruments::heap::Counted = instruments::heap::Counted;

pub(crate) mod ui;

use args::Args;
use bevy::prelude::*;

/// How many tiles out the editor streams, as a radius: a 7x7 block rather than
/// the client's 3x3.
///
/// The two uses need different numbers. In a session a character stands in the
/// middle of the block with the fog closing at 500 yards, so nothing past the
/// 3x3 is visible and every tile past it is loaded and hidden by fog. A person
/// grading a valley is above the ground with the fog switched off, looking
/// across a region, and the edge of a 3x3 is an empty drop about 700 yards
/// away, which is less than the largest height brush.
///
/// The cost grows with the square of the radius. 7x7 is 49 tiles against the
/// 3x3's 9: about five times the resident cost, five times the fill, and 2,133
/// yards of ground on a side instead of 800. `--reach` sets another value, and
/// `render::terrain::TerrainReach::HIGHEST` is the client's upper limit.
pub(crate) const REACH: i32 = 3;

/// The width of the block the editor opens, in yards: 3,733 at [`REACH`].
///
/// Three limits are derived from it rather than written as numbers, because
/// each one depends on how far away the far edge of the open ground is, and a
/// number typed into any of them would be wrong as soon as the block changed:
///
/// * how far the camera may be pulled back (`camera::DISTANCE`);
/// * how large the height brush and the texture brush may be
///   (`tools::ground::terrain::RADIUS`, `tools::ground::textures::RADIUS`), since a stroke only
///   reaches tiles the session has open.
///
/// It follows [`REACH`] and not `--reach`, so it is a constant. A run that
/// narrows the block keeps the wider limits, and that causes no harm: a brush
/// that extends past the open ground paints only the open part of its circle,
/// and a camera pulled back further than the ground extends shows sky.
pub(crate) const OPEN_BLOCK: f32 = (2.0 * REACH as f32 + 1.0) * vale_assets::world::adt::TILE_SIZE;

/// What the streamer is told, which is `--reach` or [`REACH`].
pub(crate) fn reach(args: &Args) -> vale_client::render::terrain::TerrainReach {
    vale_client::render::terrain::TerrainReach(args.view.reach.unwrap_or(REACH))
}

/// Build and run the editor.
pub fn run() {
    let config = vale_config::Config::load();
    let gamedata_dir = config.gamedata.clone();
    let args = Args::parse(std::env::args().skip(1));

    let mut app = App::new();
    app.add_plugins(vale_client::app::plugins(&vale_client::app::Host {
        title: "Vale IDE".into(),
        size: args.view.size,
    }));
    #[cfg(feature = "heap-census")]
    instruments::heap::watch_from_env();
    vale_client::app::core(&mut app, gamedata_dir, config);
    // `--place` implies the doodad tool, because placing is a mode of a
    // placement tool and there is nothing to open the picker on otherwise. It
    // is decided here rather than inside the tool so that `--place --tool wmo`
    // still means the buildings. `--tiles` implies no tool: the map window is
    // not a pointer tool and takes no row on the rail, so it opens over
    // whatever subject was already chosen.
    match (args.view.tool, args.tools.place.is_some()) {
        (Some(tool), _) => app.insert_resource(tool),
        (None, true) => app.insert_resource(tools::Tool::Doodads),
        (None, false) => &mut app,
    };
    if args.view.show_map {
        app.insert_resource(ui::map::mapview::MapView::opened());
    }
    if args.view.new_map {
        app.insert_resource(crate::map::tiles::Tiles {
            new_map: Some(crate::map::maps::Form {
                choosing_screen: true,
                ..crate::map::maps::Form::default()
            }),
            ..crate::map::tiles::Tiles::default()
        });
    }
    if args.view.navmesh {
        app.insert_resource(navmesh::Navmesh::shown());
    }
    if args.view.map_view {
        app.insert_resource(camera::TopDown::shown());
    }
    // Inserted before `EditorPlugins` and before the client's own
    // `init_resource`, which keeps a value that is already present. See
    // [`REACH`].
    app.insert_resource(reach(&args));
    // The layers a run asked to leave out, through the client's own flag and
    // the client's own parser. See [`crate::args::view::View::without`].
    if let Some(list) = args.view.without.as_deref() {
        app.insert_resource(vale_client::render::tuning::WorldTuning::without(list));
    }
    // The overlays a run asked to draw, on the same terms, over the editor's
    // own collision radius and cap. See [`crate::args::view::View::overlay`] and
    // `ui::shell::viewbar::editor_overlay`. Inserted whether or not the flag was
    // given, so the radius is the editor's from the first frame.
    #[cfg(feature = "diagnostics")]
    app.insert_resource(ui::shell::viewbar::editor_overlay(args.view.overlay.as_deref()));
    // The ground guides a run asked for. See [`crate::args::view::View::guides`].
    app.insert_resource(match args.view.guides.as_deref() {
        Some(list) => tools::guides::Guides::with(list),
        None => tools::guides::Guides::default(),
    });
    app.insert_resource(instruments::shot::Shot::new(args.instruments.shot.clone(), args.instruments.after));
    if let Some(stops) = args.instruments.tour.clone() {
        app.insert_resource(instruments::tour::Tour::new(stops, args.instruments.dwell));
    }
    app.insert_resource(args);
    app.add_plugins(EditorPlugins);
    vale_client::app::executors(&mut app);
    app.run();
}

/// Everything this crate adds to the client's app.
pub(crate) struct EditorPlugins;

impl Plugin for EditorPlugins {
    fn build(&self, app: &mut App) {
        app.init_resource::<places::Places>().add_plugins((
            // First, because every plugin after it asks whether a playtest is
            // in control. The plugin declares the system ordering it needs
            // itself; this list is only the order the plugins are built in.
            playtest::PlaytestPlugin,
            session::SessionPlugin,
            // After the playtest, whose session it uses: everything it sends
            // goes out on the socket the playtest opened, and every reply comes
            // back on the same socket.
            server::ServerPlugin,
            instruments::shot::ShotPlugin,
            instruments::tour::TourPlugin,
            camera::CameraPlugin,
            context::ContextMenuPlugin,
            pick::PickPlugin,
            tools::ToolPlugin,
            preview::stage::StagePlugin,
            preview::lab::LabPlugin,
            preview::portraits::PortraitPlugin,
            navmesh::NavmeshPlugin,
            marks::MarksPlugin,
            ui::PanelPlugin,
        ));
        // Outside the tuple, which Bevy implements for at most fifteen plugins.
        app.add_plugins((tablet::TabletPlugin, popout::PopoutPlugin));
        app.init_resource::<prefs::favourites::Favourites>()
            .init_resource::<prefs::bookmarks::Bookmarks>();
    }
}
