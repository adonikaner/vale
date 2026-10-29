//! The editor's panels: the docked shell around the viewport and the windows
//! drawn over it.
//!
//! ```text
//! shell/        the docked regions around the viewport and what they open:
//!               the top bar and its forms, the rail, the view bar, the status
//!               line, the pop-out window, the right-click menu, the hover
//!               card, the server marks and the sync view
//! widgets/      the theme and the pieces the forms and lists are built from
//! tools/        the inspector, and the panels and windows of the pointer
//!               tools, at the same paths as their state in `crate::tools`
//! subjects/     the windows and workspaces of the server-row subjects, at the
//!               same paths as their state in `crate::subjects`
//! tables/       the workspace a DBC table is edited in, the spell
//!               storyboard and what a display row looks like
//! preview/      the attachment lab's card and pane
//! map/          the map from above
//! import/       the Assets workspace and the Import Texture, Import Model and
//!               Import Sound dialogs
//! workspace.rs  the workspace a tool puts in the middle of a window, drawn
//!               the same in the main window and a pop-out window
//! ```
//!
//! ## Layout: four docked regions
//!
//! The controls are docked rather than held in one floating `egui::Window`. A
//! single floating window that lists a map drop-down, a coordinate readout,
//! three "go to" rows, the brush, the history, the project and the playtest
//! has no fixed order and no grouping by purpose.
//!
//! The shell has four docked regions with one job each:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │ top     the session: project, map, where, playtest   │
//! ├──────┬───────────────────────────────┬──────────────┤
//! │ rail │        the viewport           │  inspector   │
//! │      │                               │  the tool    │
//! ├──────┴───────────────────────────────┴──────────────┤
//! │ view    what the viewport shows: layers, overlays    │
//! ├─────────────────────────────────────────────────────┤
//! │ status  where you are, what is open, what happened  │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! A new control goes in the top bar or the inspector by this rule: the bar
//! holds session state and the inspector holds what is being edited. Which
//! project, which map, whether the work is saved, whether the game is running
//! go in the bar. A brush radius, a placement's rotation and the undo stack go
//! in the inspector.
//!
//! The view bar is a third category: it changes what the viewport shows and
//! never what is being edited. Turning the doodads off touches no file and
//! adds nothing to the undo stack. It is a strip of its own rather than a
//! section of the inspector because the inspector is about the selection, and
//! these switches apply whatever is selected. See [`shell::viewbar`].
//!
//! ## Why the panels are built inside a root `Ui`
//!
//! In egui 0.35 `egui::Panel::left(..).show(ui, ..)` takes a `Ui`, and there is
//! no `Context` form of it. bevy_egui hands out a `Context`, so the shell opens
//! a root `Ui` over `ctx.viewport_rect()` on the background layer and docks
//! into that, as bevy_egui's own `side_panel` example does. Without the root
//! `Ui` no panel can be docked from a `Context`, and the shell was one
//! floating window before it was added.
//!
//! ## How the viewport rectangle is decided
//!
//! The camera renders the whole window and the panels are painted over it, so
//! the viewport is whatever the four regions leave. The pick reads
//! `Window::cursor_position` against that camera, which covers the whole
//! window.
//!
//! Whether the pointer is over the chrome cannot be taken from egui here.
//! `EguiWantsInput::wants_pointer_input` is egui's own
//! `is_using_pointer() || (over_egui && !any_down())`, so on the frame the
//! button goes down it answers false, and that is the frame a tool acts on. A
//! click on the inspector therefore fell through to the world and moved the
//! selection.
//!
//! `is_pointer_over_area` has no `any_down()` term, so it would answer
//! correctly on the press frame, but it also fails here: for a widget on the
//! background layer egui
//! compares the pointer against `root_ui_available_rect`, which is written by
//! `Context::run_ui`. bevy_egui does not call `run_ui`, because it drives
//! `begin_pass`/`end_pass` itself. With the rectangle unset egui takes its
//! "we shouldn't get here" branch and reports the pointer over the interface
//! everywhere, so no tool would ever act.
//!
//! The shell therefore keeps its own answer. [`Viewport`] is the rectangle the
//! four panels leave, written here where the panels are drawn and read by
//! every tool. [`Viewport::holds`] is the one function that decides whether a
//! pointer position is over the world.

pub(crate) mod import;
pub(crate) mod map;
pub(crate) mod preview;
pub(crate) mod shell;
pub(crate) mod subjects;
pub(crate) mod tables;
pub(crate) mod tools;
pub(crate) mod widgets;
pub(crate) mod workspace;

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use egui::{CornerRadius, Frame, Margin, Stroke, UiBuilder};

use crate::camera::EditorCamera;
use crate::pick::Cursor;
use crate::playtest::{Login, Playtest};
use crate::session::EditSession;
use crate::tools::placements::doodads::Selection;
use crate::tools::placements::gizmo::Gizmo;
use crate::tools::ground::terrain::Terrain;
use crate::tools::ground::textures::Textures;
use crate::tools::Tool;
use vale_client::assets::GameAssets;
use vale_client::glue::autologin::AutoLogin;
use vale_client::render::focus::WorldFocus;
use subjects::windows::subject_windows;
use workspace::{workspace, Bench};

/// The part of the window the chrome leaves for the world, in window points.
///
/// It has two parts because the chrome has two types of element. The docked
/// panels shrink the root `Ui`, so what they leave is one rectangle. A popover,
/// a window or a modal floats over that rectangle and shrinks nothing, so its
/// rectangle is listed separately; without that, a press on a popover's
/// password box is a press on the ground behind it, the same fault the docked
/// panels had. [`shell::popover`] and the windows return their rectangles for this
/// purpose, and [`areas_over_the_world`] adds every other egui area at the end
/// of the pass.
///
/// This is the one place that decides whether a press belongs to the viewport
/// or to a panel. The module comment explains why the two egui answers do not
/// work.
///
/// The rectangles are in egui's points and `Window::cursor_position` is in
/// the window's logical pixels. The two are equal only while egui's zoom
/// factor is 1: bevy_egui sets `pixels_per_point` to the window's scale factor
/// times the zoom, which Ctrl and + or - change. [`Viewport::holds`] divides
/// the pointer by [`Viewport::zoom`] before comparing. The pick multiplies by
/// the scale factor instead, because its ray goes into a render target
/// measured in physical pixels, which the zoom does not change.
#[derive(Resource, Debug, Clone)]
pub(crate) struct Viewport {
    /// `None` before the panels are first drawn; on that frame the whole
    /// window is the world.
    rect: Option<egui::Rect>,
    /// The rectangles drawn floating over it this frame: popovers, windows,
    /// modals and egui areas.
    floating: Vec<egui::Rect>,
    /// egui's zoom factor this frame: how many of the window's logical pixels
    /// one point is. Set by the shell; 1 before it has run.
    pub(crate) zoom: f32,
}

impl Default for Viewport {
    fn default() -> Viewport {
        Viewport {
            rect: None,
            floating: Vec::new(),
            zoom: 1.0,
        }
    }
}

impl Viewport {
    /// Whether a pointer position is over the world.
    ///
    /// `false` over any of the four panels and over anything floating. A
    /// widget held by the pointer and an open combo box list are answered by
    /// `is_using_pointer` and `is_popup_open`, so this is asked together with
    /// egui's own answer rather than instead of it. See [`over_the_world`].
    pub(crate) fn holds(&self, at: Vec2) -> bool {
        // The pointer is in logical pixels and the rectangles in points.
        let at = at / self.zoom.max(0.01);
        let at = egui::pos2(at.x, at.y);
        let inside = match self.rect {
            Some(rect) => rect.contains(at),
            // Nothing drawn yet: the whole window is viewport, as it would be
            // for a host with no panels.
            None => true,
        };
        inside && !self.floating.iter().any(|rect| rect.contains(at))
    }
}

/// Whether a press belongs to the world. Every tool asks this before acting.
///
/// Both halves are needed. [`Viewport::holds`] answers for the four docked
/// panels and for every floating window, which egui cannot answer for here.
/// `is_using_pointer` answers for a widget being dragged past the edge of its
/// window, and `is_popup_open` for what egui opens itself, such as a combo
/// box's list. The popup check is coarse on purpose: while any list is open no
/// press reaches the world, so the click that closes a list does not also act
/// in the world.
pub(crate) fn over_the_world(
    viewport: &Viewport,
    wants: &bevy_egui::input::EguiWantsInput,
    windows: &Query<&Window, With<bevy::window::PrimaryWindow>>,
) -> bool {
    let Ok(window) = windows.single() else {
        return false;
    };
    let Some(at) = window.cursor_position() else {
        return false;
    };
    viewport.holds(at) && !wants.is_using_pointer() && !wants.is_popup_open()
}

/// The four resources a playtest is started and stopped through, as one
/// parameter.
///
/// Bevy's system parameter limit is sixteen and the shell had seventeen.
/// Bundling is the same fix the client's own `interface/mod.rs` uses when
/// `add_plugins` hits the same limit. These four also belong together: they
/// are everything `crate::playtest::start` and `stop` take.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Playing<'w> {
    client: ResMut<'w, vale_client::world::session::Session>,
    state: ResMut<'w, Playtest>,
    login: ResMut<'w, Login>,
    auto: ResMut<'w, AutoLogin>,
    /// Whether the panels are drawn over the playtest rather than the bar.
    /// See [`crate::playtest::ShellOpen`], and [`draw`], which branches on it.
    open: ResMut<'w, crate::playtest::ShellOpen>,
    /// The queue a save's `.reload` lines go on. It is sent over the
    /// playtest's own session, so it is bundled with the other session
    /// resources.
    reloads: ResMut<'w, crate::server::reload::Reloads>,
    /// Where this machine's server is. The bar's Save and its Server… button
    /// both need it.
    server: ResMut<'w, crate::server::settings::ServerSettings>,
    /// Whether the session the Playtest button starts keeps the query answers
    /// the server gives it. See
    /// [`vale_client::world::session::QueryCaches`].
    caches: ResMut<'w, vale_client::world::session::QueryCaches>,
    /// The last known state of each server subject, which the Server panel
    /// draws and does not recompute every frame. See [`crate::server::sync::Standings`].
    standings: ResMut<'w, crate::server::sync::Standings>,
    /// The database writes waiting and running, which the Server panel queues
    /// and the toast in the corner reports. See [`crate::server::queue`].
    queue: ResMut<'w, crate::server::queue::ServerQueue>,
    /// Which step of a tile regeneration is running, for the same toast. See
    /// [`crate::server::datadir::Step`].
    step: Res<'w, crate::server::datadir::Step>,
}

/// The resources the inspector edits, as one parameter, for the same reason
/// as [`Playing`]: the shell reached the parameter limit again when a second
/// brush and a gizmo were added. They are exactly what [`tools::draw`]
/// takes, and nothing else in this file touches any of them.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Editing<'w> {
    pub(crate) terrain: ResMut<'w, Terrain>,
    pub(crate) grading: ResMut<'w, crate::tools::ground::grade::Grading>,
    pub(crate) road: ResMut<'w, crate::tools::ground::road::RoadTool>,
    pub(crate) measuring: ResMut<'w, crate::tools::measure::Measuring>,
    pub(crate) shading: ResMut<'w, crate::tools::ground::shading::Shading>,
    pub(crate) textures: ResMut<'w, Textures>,
    pub(crate) selection: ResMut<'w, Selection>,
    /// The doodad and WMO tools' drag state. Each panel uses it for one
    /// message: that a group change made in the panel's fields needs its
    /// members settled and, for buildings, their boxes re-fitted.
    pub(crate) doodad_held: ResMut<'w, crate::tools::placements::doodads::Held>,
    pub(crate) wmos: ResMut<'w, crate::tools::placements::wmos::Selection>,
    pub(crate) wmo_held: ResMut<'w, crate::tools::placements::wmos::Held>,
    pub(crate) gizmo: ResMut<'w, Gizmo>,
    pub(crate) holes: Res<'w, crate::tools::ground::holes::Holes>,
    pub(crate) areas: ResMut<'w, crate::tools::ground::areas::Areas>,
    /// The chunk tool's selection. See [`crate::tools::ground::chunks`].
    pub(crate) chunks: ResMut<'w, crate::tools::ground::chunks::Chunks>,
    pub(crate) water: ResMut<'w, crate::tools::ground::water::Water>,
    pub(crate) sweep: ResMut<'w, crate::map::sweep::Sweep>,
    pub(crate) thumbnails: ResMut<'w, widgets::thumbnails::Thumbnails>,
    /// The pictures of models, which are rendered rather than decoded (see
    /// [`crate::preview::portraits`]), and the two lists a picker keeps beside its
    /// folders.
    pub(crate) portraits: ResMut<'w, crate::preview::portraits::Portraits>,
    pub(crate) favourites: ResMut<'w, crate::prefs::favourites::Favourites>,
    /// The camera's named places, for the go-to popover.
    pub(crate) bookmarks: ResMut<'w, crate::prefs::bookmarks::Bookmarks>,
    /// The terrain and texture brushes' presets.
    pub(crate) brushes: ResMut<'w, crate::prefs::brushes::Brushes>,
    pub(crate) placing: ResMut<'w, crate::tools::placements::place::Placing>,
    pub(crate) tiles: ResMut<'w, crate::map::tiles::Tiles>,
    pub(crate) mapview: ResMut<'w, map::mapview::MapView>,
    pub(crate) browser: ResMut<'w, crate::tables::browser::Browser>,
    /// The world half of the lights subject: which light is chosen, where they
    /// all are. See [`crate::tools::places::lights`].
    pub(crate) lights: ResMut<'w, crate::tools::places::lights::Lights>,
    /// The flight path subject: the nodes and paths on the map and what is
    /// selected. See [`crate::tools::places::flightpaths`].
    pub(crate) flightpaths: ResMut<'w, crate::tools::places::flightpaths::Flightpaths>,
    /// The area trigger subject: the triggers on the map, what is selected,
    /// and the server rows read for them. See [`crate::tools::places::triggers`].
    pub(crate) triggers: ResMut<'w, crate::tools::places::triggers::Triggers>,
    /// The safe place (graveyard) subject's state, which has the same shape
    /// as the area trigger subject's. See
    /// [`crate::tools::places::graveyards`].
    pub(crate) graveyards: ResMut<'w, crate::tools::places::graveyards::Graveyards>,
    /// The world half of the creature subject: the map's spawns, which is
    /// selected, and the two rows behind it. See [`crate::tools::spawns::creatures`].
    pub(crate) creatures: ResMut<'w, crate::tools::spawns::creatures::Creatures>,
    /// Where each near creature's service icons go this frame. Read-only
    /// here; see [`shell::servicemarks`].
    pub(crate) service_marks: Res<'w, crate::tools::spawns::creatures::ServiceMarks>,
    /// The game-object subject's state, which has the same shape. See
    /// [`crate::tools::spawns::gameobjects`].
    pub(crate) objects: ResMut<'w, crate::tools::spawns::gameobjects::GameObjects>,
    /// The waypoint mode over the creature subject: whose path is open, which
    /// point is chosen. See [`crate::tools::spawns::waypoints`].
    pub(crate) waypoints: ResMut<'w, crate::tools::spawns::waypoints::Waypoints>,
    /// The item workspace's state: the table, which row is open, and what the
    /// appearance it names looks like. See [`crate::subjects::items`].
    pub(crate) items: ResMut<'w, crate::subjects::items::Items>,
    /// The quest workspace's state: the table, the relations, which quest is
    /// open. See [`crate::subjects::quests`].
    pub(crate) quests: ResMut<'w, crate::subjects::quests::Quests>,
    /// The loot window's state: which sets the selection names, and what each
    /// holds. See [`crate::subjects::loot`].
    pub(crate) loot: ResMut<'w, crate::subjects::loot::Loot>,
    /// The behaviour windows' state: the creature they are about, and what
    /// its events, its spell list and one script hold. See
    /// [`crate::subjects::behaviour`].
    pub(crate) behaviour: ResMut<'w, crate::subjects::behaviour::Behaviour>,
    /// The Vendor and Trainer windows' state: the creature they are about,
    /// and what its lists hold. See [`crate::subjects::services`].
    pub(crate) services: ResMut<'w, crate::subjects::services::Services>,
    /// The Equipment window's and the Addon section's state: the creature
    /// they are about, the spawn's addon row and the set read. See
    /// [`crate::subjects::addons`].
    pub(crate) addons: ResMut<'w, crate::subjects::addons::Addons>,
    /// The Formation and Links sections' state: the creature group and
    /// linking tables, read whole. See [`crate::subjects::formations`].
    pub(crate) formations: ResMut<'w, crate::subjects::formations::Formations>,
    /// The Pool section's and window's state: the pool tables, read whole.
    /// See [`crate::subjects::pools`].
    pub(crate) pools: ResMut<'w, crate::subjects::pools::Pools>,
    /// The Game Events section's and window's state: the game event tables,
    /// read whole. See [`crate::subjects::game_events`].
    pub(crate) game_events: ResMut<'w, crate::subjects::game_events::GameEvents>,
    /// Where spawns named by guid stand, for the lists that name spawns on
    /// any map. See [`crate::subjects::located`].
    pub(crate) located: ResMut<'w, crate::subjects::located::Locations>,
    /// Whether each spawn tool draws the other's spawns. See
    /// [`crate::tools::spawns::spawn::OtherSpawns`].
    pub(crate) others: ResMut<'w, crate::tools::spawns::spawn::OtherSpawns>,
    /// The view bar's event filter. See
    /// [`crate::subjects::game_events::EventView`].
    pub(crate) event_view: ResMut<'w, crate::subjects::game_events::EventView>,
    /// The gossip window's state: the creature it is about and the menus it
    /// has read. See [`crate::subjects::gossip`].
    pub(crate) gossip: ResMut<'w, crate::subjects::gossip::Gossip>,
    /// The condition window's state. See [`crate::subjects::conditions`].
    pub(crate) conditions: ResMut<'w, crate::subjects::conditions::Conditions>,
    /// The Assets workspace's state and the Import Texture dialog's. See
    /// [`crate::import::assets`].
    pub(crate) assets: ResMut<'w, crate::import::assets::Assets>,
    /// The Assets workspace's Game Files part. See [`crate::import::files`].
    pub(crate) files: ResMut<'w, crate::import::files::GameFiles>,
    /// The pop-out window: the workspace in it, and what a panel asked of it.
    /// See [`crate::popout`].
    pub(crate) popout: ResMut<'w, crate::popout::Popout>,
    /// The time of day the world is showing. Read-only here: the view bar
    /// sets the hour, and the panels read it so a band's day strip can mark
    /// the viewport's current hour on itself.
    pub(crate) clock: Res<'w, vale_client::render::sky::WorldClock>,
    pub(crate) storyboard: ResMut<'w, tables::storyboard::Storyboard>,
    pub(crate) stage: ResMut<'w, crate::preview::stage::Stage>,
    pub(crate) lab: ResMut<'w, crate::preview::lab::Lab>,
    /// What is typed into the project popover; see [`shell::topbar::Projects`]. It
    /// is in the bundle rather than a parameter of its own because this system
    /// is at Bevy's limit of sixteen parameters.
    pub(crate) projects: ResMut<'w, shell::topbar::Projects>,
    /// The command line, for the two flags that open something a press would
    /// otherwise have to open: `--projects` here, `--row` in the browser.
    pub(crate) args: Res<'w, crate::args::Args>,
    /// The right-click menu over the viewport. See [`shell::context`].
    pub(crate) menu: ResMut<'w, crate::context::ContextMenu>,
    /// What the doodad and WMO tools have copied, for the menu's Paste here.
    pub(crate) clipboard: Res<'w, crate::tools::placements::group::Clipboard>,
    /// The drawing tablet settings and the pen's state, for the section the
    /// shading panel shows and the terrain and texture toolbars' Tablet menus.
    /// See [`crate::tablet`].
    pub(crate) tablet: ResMut<'w, crate::tablet::Tablet>,
    pub(crate) pen: Res<'w, crate::tablet::Pen>,
    /// What the panels asked of the shell this frame, carried out after the
    /// egui pass. See [`shell::asks`].
    pub(crate) asks: ResMut<'w, shell::asks::ShellAsks>,
    /// The camera requests a panel's Fly to button sends. See
    /// [`crate::camera::FlyTo`].
    pub(crate) fly: ResMut<'w, Messages<crate::camera::FlyTo>>,
}

/// What the view bar reads and writes, as one parameter.
///
/// A bundle of its own for the same reason as [`Playing`] and [`Editing`]:
/// this shell is at Bevy's sixteen-parameter limit. These six are what
/// [`shell::viewbar::draw`] takes, and nothing else in this file touches them.
///
/// Two of the six exist only in a `diagnostics` build. `DebugOverlay` is
/// gated in the client together with the debug window that was its only
/// surface (see `render::overlay`), so a build without the feature has no
/// overlay group. The eighteen layer switches and the three frame settings
/// (`RenderTuning`'s MSAA, vsync and sun shadows) are not gated and are always
/// present.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Viewing<'w> {
    world: ResMut<'w, vale_client::render::tuning::WorldTuning>,
    frame: ResMut<'w, vale_client::world::camera::RenderTuning>,
    #[cfg(feature = "diagnostics")]
    overlay: ResMut<'w, vale_client::render::overlay::DebugOverlay>,
    #[cfg(feature = "diagnostics")]
    wireframe: Res<'w, vale_client::render::overlay::WireframeSupported>,
    #[cfg(feature = "diagnostics")]
    drawn: Res<'w, vale_client::render::overlay::CollisionDrawn>,
    navmesh: ResMut<'w, crate::navmesh::Navmesh>,
    /// The ground guides, which the view bar's Guides menu switches. See
    /// [`crate::tools::guides`].
    guides: ResMut<'w, crate::tools::guides::Guides>,
    /// The top-down map view, which the view bar's MAP button switches. See
    /// [`crate::camera::TopDown`].
    top_down: ResMut<'w, crate::camera::TopDown>,
    icons: Res<'w, shell::icons::Icons>,
    /// Which world tool the World part of the top bar returns to. See
    /// [`shell::rail::Rail`].
    rail: ResMut<'w, shell::rail::Rail>,
}

pub(crate) struct PanelPlugin;

impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<map::mapview::MapView>()
            .init_resource::<shell::topbar::GoTo>()
            .init_resource::<shell::topbar::Projects>()
            .init_resource::<shell::popover::Popovers>()
            .init_resource::<crate::server::sync::Standings>()
            .init_resource::<Viewport>()
            .init_resource::<shell::rail::Rail>()
            .init_resource::<shell::asks::ShellAsks>()
            .add_plugins((widgets::thumbnails::ThumbnailPlugin, shell::icons::IconPlugin))
            // The keys that work whichever tool is chosen. After the pick,
            // as every tool's input is; a tool whose order against them
            // matters states it in its own plugin (the height brush, the
            // doodad and WMO picks, the group clipboard).
            // Its three playtest keys are taken before the game's key
            // bindings, which must not see a key the editor acts on; see
            // `crate::tools::shortcuts::take`.
            .init_resource::<crate::tools::shortcuts::Taken>()
            .add_systems(
                Update,
                crate::tools::shortcuts::take.before(vale_client::input::bindings::BindingSet),
            )
            .add_systems(
                Update,
                crate::tools::shortcuts::shortcuts
                    .after(crate::pick::aim)
                    .after(crate::tools::shortcuts::take),
            )
            .add_systems(EguiPrimaryContextPass, draw)
            // One pass per pop-out window slot; see `crate::popout`.
            .add_systems(crate::popout::PopoutPass(0), shell::popout::draw::<0>)
            .add_systems(crate::popout::PopoutPass(1), shell::popout::draw::<1>)
            .add_systems(crate::popout::PopoutPass(2), shell::popout::draw::<2>)
            .add_systems(crate::popout::PopoutPass(3), shell::popout::draw::<3>)
            .add_systems(crate::popout::PopoutPass(4), shell::popout::draw::<4>)
            .add_systems(crate::popout::PopoutPass(5), shell::popout::draw::<5>);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut contexts: EguiContexts,
    mut session: Option<ResMut<EditSession>>,
    mut tool: ResMut<Tool>,
    mut editing: Editing,
    mut camera: ResMut<EditorCamera>,
    cursor: Res<Cursor>,
    // The clock that groups edits into one gesture. See
    // `vale_edit::undo::History::begin_gesture`, which makes a dragged
    // number field one undo step rather than one per frame.
    time: Res<Time>,
    focus: Res<WorldFocus>,
    assets: Res<GameAssets>,
    mut playing: Playing,
    places: Res<crate::places::Places>,
    mut go_to: ResMut<shell::topbar::GoTo>,
    mut popovers: ResMut<shell::popover::Popovers>,
    mut viewport: ResMut<Viewport>,
    mut viewing: Viewing,
    background: crate::map::tiles::Background,
) -> Result {
    let Some(session) = session.as_mut() else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?.clone();
    widgets::theme::install(&ctx);
    viewport.zoom = ctx.zoom_factor();
    // A dialog several windows draw opens here when this is the window last
    // used. See [`dialog_home`].
    mark_dialog_owner(&ctx, editing.popout.last_focused.is_none());
    // The three files kept beside the project folders, read once per install.
    // See `crate::prefs::favourites` for where they are stored and why.
    if let Some(dir) = session.project.root.parent() {
        editing.favourites.load_from(dir);
        editing.bookmarks.load_from(dir);
        editing.brushes.load_from(dir);
    }

    // A playtest has two layouts, chosen by [`crate::playtest::ShellOpen`].
    // Shut, the whole window is the game's and the bar is one row of controls.
    // Open, the panels are drawn as they are while editing, so there is no
    // second layout to learn for the same subjects. Three parts differ: the
    // rail is not drawn, the top bar's World part is disabled with the reason
    // on its hover text, and the right-hand end of the top bar holds the
    // playtest's controls.
    let in_world = playing.state.playing();
    if in_world && !playing.open.0 {
        // The pointer belongs to the game under test. egui's own
        // `is_pointer_over_area` declines a press that lands on the bar; it
        // answers correctly for a floating window.
        viewport.rect = None;
        viewport.floating.clear();
        playtest_bar(
            &ctx,
            &mut playing.state,
            &mut playing.client,
            &mut playing.auto,
            &mut playing.open,
            &mut tool,
            session,
        );
        return Ok(());
    }

    // The root the four regions dock into. See the module comment.
    let mut root = egui::Ui::new(
        ctx.clone(),
        "editor".into(),
        UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    // `--projects`, once. The popover is opened here rather than at startup
    // because its anchor is the button's rectangle, which does not exist until
    // the bar has been drawn once. See `shell::popover::Popover::show`.
    if editing.args.view.projects && !editing.projects.shown_once {
        editing.projects.shown_once = true;
        popovers.project.show();
        // `--doom <name>`: the confirmation, open on that project.
        if let Some(name) = editing.args.view.doom.as_deref() {
            let clear = name == session.project.name;
            let held = match clear {
                true => crate::server::held::held_by(&session.project),
                false => vale_edit::project::Project::open(&assets.root, name)
                    .map(|project| crate::server::held::held_by(&project))
                    .unwrap_or_default(),
            };
            editing.projects.confirming =
                Some(shell::topbar::Doomed::new(name.to_string(), clear, &held));
        }
    }
    // `--server`, once, for the same reason.
    if editing.args.server.panel && !popovers.server_shown_once {
        popovers.server_shown_once = true;
        popovers.server.show();
    }

    egui::Panel::top("editor-top")
        .frame(bar(widgets::theme::PANEL))
        .show(&mut root, |ui| {
            shell::topbar::draw(
                ui,
                session,
                &mut camera,
                &mut popovers,
                &assets,
                &mut shell::topbar::Session {
                    state: &mut playing.state,
                    client: &mut playing.client,
                    login: &playing.login,
                    auto: &mut playing.auto,
                    open: &mut playing.open,
                    queue: &mut playing.queue,
                    server: &mut playing.server,
                    caches: &mut playing.caches,
                    step: &playing.step,
                },
                &mut editing.mapview.open,
                &mut editing.tiles.new_map,
                &mut shell::topbar::Subjects {
                    tool: &mut tool,
                    rail: &mut viewing.rail,
                    popped: editing.popout.tools().collect(),
                },
                &editing.bookmarks,
            );
        });

    // The background work the status line shows: shadow rebakes, minimap
    // runs, and the server's tile tools while a publish or a regeneration
    // runs them. Each entry carries a name so the progress bars can be told
    // apart when several run; the names are the map window's own.
    let mut working: Vec<(&str, String, f32)> = background.summaries();
    if playing.queue.busy() {
        working.extend(
            playing
                .step
                .summary()
                .map(|(label, fraction)| ("Server files", label, fraction)),
        );
    }

    // The switch sets below are edited as copies and written back only when
    // they change. `ResMut`'s `DerefMut` marks a resource changed whether or
    // not the value changed, and `render::tuning::switch` runs a sweep over
    // every doodad batch in the world when that flag is set. A bar that wrote
    // its buttons straight through would run the sweep on every frame it is
    // drawn, which is every frame. The client's own debug window copies for
    // the same reason.
    let mut edited_world = viewing.world.clone();
    let mut edited_frame = viewing.frame.clone();
    #[cfg(feature = "diagnostics")]
    let mut edited_overlay = viewing.overlay.clone();
    // What the view bar measures "moved" against: the editor's own starting
    // state, in which the game's interface is off while editing.
    let baseline = crate::playtest::world_baseline(playing.state.editing());

    // The status line is shown first, so it is the outer of the two bottom
    // regions and the view bar docks above it. Panels take their space in the
    // order they are shown.
    let scripted = {
        #[cfg(feature = "diagnostics")]
        {
            shell::viewbar::scripted(
                &edited_world,
                &baseline,
                &edited_overlay,
                viewing.navmesh.on,
                viewing.top_down.on,
                &viewing.guides,
            )
        }
        #[cfg(not(feature = "diagnostics"))]
        {
            shell::viewbar::scripted(
                &edited_world,
                &baseline,
                viewing.navmesh.on,
                viewing.top_down.on,
                &viewing.guides,
            )
        }
    };
    egui::Panel::bottom("editor-status")
        .frame(bar(widgets::theme::SHELL))
        .show(&mut root, |ui| {
            shell::status::draw(
                ui, session, &cursor, &focus, &camera, &scripted, &working, in_world,
            );
        });

    let mut edited_guides = viewing.guides.clone();
    let mut phase = editing.event_view.phase;
    let event_list: Vec<(u32, String)> = editing
        .game_events
        .all(&session.rows.edits)
        .into_iter()
        .filter(|shown| shown.kept())
        .map(|shown| (shown.row.entry, shown.row.description))
        .collect();
    let mut map_view = viewing.top_down.on;
    let mut flight = camera.movement == crate::camera::Movement::Flight;
    egui::Panel::bottom("editor-view")
        .frame(bar(widgets::theme::PANEL))
        .show(&mut root, |ui| {
            shell::viewbar::draw(
                ui,
                &mut shell::viewbar::Bar {
                    world: &mut edited_world,
                    frame: &mut edited_frame,
                    #[cfg(feature = "diagnostics")]
                    overlay: &mut edited_overlay,
                    #[cfg(feature = "diagnostics")]
                    wireframe: &viewing.wireframe,
                    #[cfg(feature = "diagnostics")]
                    drawn: &viewing.drawn,
                    navmesh: &mut viewing.navmesh,
                    guides: &mut edited_guides,
                    phase: &mut phase,
                    events: &event_list,
                    map_view: &mut map_view,
                    flight: &mut flight,
                    editing: playing.state.editing(),
                    areas_shown: *tool == Tool::Areas,
                    open_guides: editing.args.view.guides_menu
                        && time.elapsed_secs() > 3.0
                        && !std::mem::replace(&mut editing.projects.guides_shown_once, true),
                    baseline: &baseline,
                    icons: &viewing.icons,
                },
            );
        });
    // Written back only when a switch moved, as the other view settings are.
    if edited_guides != *viewing.guides {
        *viewing.guides = edited_guides;
    }
    if phase != editing.event_view.phase {
        editing.event_view.phase = phase;
    }
    if map_view != viewing.top_down.on {
        viewing.top_down.on = map_view;
    }
    let movement = match flight {
        true => crate::camera::Movement::Flight,
        false => crate::camera::Movement::Level,
    };
    if movement != camera.movement {
        camera.movement = movement;
    }

    // The rail is not drawn over a playtest. Every tile on it keeps the
    // viewport, and no such tool can be used then; see
    // [`Tool::survives_playtest`](crate::tools::Tool::survives_playtest).
    // During a playtest the workspace control on the top bar is the only way
    // to choose a tool.
    //
    // Nor is it drawn while a workspace covers the viewport. Every tile would
    // leave the workspace for the world, which the top bar's World does in one
    // click, and the width goes to the workspace's list and form instead.
    if !in_world && !tool.covers_viewport() {
        egui::Panel::left("editor-rail")
            .exact_size(widgets::theme::RAIL_WIDTH)
            .frame(side(widgets::theme::SHELL))
            .show(&mut root, |ui| {
                // The last argument is whether a world database is reachable.
                // A tool whose rows are in vmangos' database is greyed without
                // one; see [`shell::rail::draw`].
                shell::rail::draw(
                    ui,
                    &mut tool,
                    &mut viewing.rail,
                    &viewing.icons,
                    in_world,
                    playing.server.resolve().is_some(),
                );
                // Claim the rest of the rail's height. Without this the panel
                // is only as tall as its rows, so its fill stops part way down
                // the window and leaves a gap in the shell.
                ui.allocate_rect(ui.available_rect_before_wrap(), egui::Sense::hover());
            });
    }

    // The DBCs, loaded only for the three panels that name an area: the zone
    // tool, the chunk tool and the measuring tool. `GameAssets::display_tables` loads every
    // table on its first call, so asking unconditionally would make an editor
    // that opens on the terrain brush load `AreaTable`, `Map` and the rest at
    // startup. Asked here, the cost is paid the first time one of them is
    // chosen.
    let tables = matches!(*tool, Tool::Areas | Tool::Chunks | Tool::Measure)
        .then(|| assets.display_tables().ok())
        .flatten();
    // The session's own copy of AreaTable, for the two panels that list and
    // name areas, so one made in the Zones workspace is there before a save.
    if matches!(*tool, Tool::Areas | Tool::Chunks) {
        editing.areas.follow_table(session);
    }
    // The inspector is resizable and the rail is not. The rail is a fixed
    // list of words; the inspector shows the chosen tool's controls, and two
    // tools show content no single width fits: a tileset's folder and file
    // name, and a row of five named choices in one control. The default width
    // suits most panels and the drag handle is for those two.
    egui::Panel::right("editor-inspector")
        .default_size(widgets::theme::INSPECTOR_WIDTH)
        .min_size(widgets::theme::INSPECTOR_MIN)
        .max_size(widgets::theme::INSPECTOR_MAX)
        .resizable(true)
        .frame(side(widgets::theme::PANEL))
        .show(&mut root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                tools::draw(
                    ui,
                    tools::Subject {
                        tool: *tool,
                        session,
                        tables: tables.as_deref(),
                        assets: &assets,
                        cursor: &cursor,
                        now: time.elapsed_secs_f64(),
                        // Borrowed from the bundle that already holds it
                        // rather than taken again. Two accesses to one
                        // resource in one system is `B0002`, which Bevy
                        // refuses at startup, and a second `Res` beside
                        // `Playing`'s `ResMut` would be that. The creature
                        // panel is the one user of it here.
                        server: &playing.server,
                        server_panel: &mut popovers.server,
                        standings: &mut playing.standings,
                    },
                    &mut editing,
                );
                ui.allocate_rect(ui.available_rect_before_wrap(), egui::Sense::hover());
            });
        });

    // The top edge of the two bottom bars, where the sync toast sits. Read
    // here, after every docked panel has taken its space and before the
    // middle takes the rest.
    let bars_top = root.available_rect_before_wrap().bottom();

    // Where a Data subject's form goes. See
    // [`Tool::surface`](crate::tools::Tool::surface).
    //
    // A spell takes the middle, because it has no place in the world and
    // nothing behind the form needs to be seen. The world is still drawn
    // behind it and is covered; the streamer and the camera are unchanged, so
    // switching back to a World tool shows the same ground as before.
    //
    // A light and a flight path do not take the middle: each is picked and
    // dragged in the viewport, so its form is in the inspector with every
    // other selection's numbers and the world keeps the middle.
    // `tools::draw` handles that, so nothing is needed for either here.
    let covers = tool.covers_viewport();
    // A tool a panel asks to switch to, applied after everything is drawn.
    // The quest workspace's "back to Creatures" and the creature tool's quest
    // window both request one, and the tool is borrowed while they are drawn.
    let mut switch_to: Option<Tool> = None;
    if covers {
        // The central panel has no fill because the storyboard's preview is a
        // hole that shows the world, and a panel background would paint over
        // it. Each docked panel inside carries its own fill, and the fields
        // view paints the area it covers itself. See `tables::draw`.
        egui::CentralPanel::default_margins()
            .frame(egui::Frame::NONE)
            .show(&mut root, |ui| {
                workspace(
                    ui,
                    *tool,
                    Bench {
                        session: &mut **session,
                        editing: &mut editing,
                        playing: &mut playing,
                        popovers: &mut popovers,
                        assets: &assets,
                        now: time.elapsed_secs_f64(),
                        #[cfg(feature = "diagnostics")]
                        wireframe: Some(&mut edited_overlay.wireframe),
                        #[cfg(not(feature = "diagnostics"))]
                        wireframe: None,
                        particles: Some(&mut edited_world.particles),
                        switch_to: &mut switch_to,
                    },
                );
            });
    }

    // Write the copies back after everything that edits them (the view bar
    // and the workspace), and only when they changed. The note above the
    // copies explains why a write every frame is a sweep every frame.
    if edited_world != *viewing.world {
        *viewing.world = edited_world;
    }
    if edited_frame != *viewing.frame {
        *viewing.frame = edited_frame;
    }
    #[cfg(feature = "diagnostics")]
    if edited_overlay != *viewing.overlay {
        *viewing.overlay = edited_overlay;
    }

    // The area the four regions left, which is the docked part of
    // [`Viewport`]. It is read from the root `Ui` rather than computed by
    // subtracting four rectangles: each `Panel::show` shrinks the root, so
    // this is the value egui would have written into `root_ui_available_rect`
    // if bevy_egui called `run_ui`.
    //
    // It is empty while a workspace covers the middle: a press there belongs
    // to the form, and a tool acting on the ground behind it would change
    // ground that cannot be seen.
    // It is also empty while a playtest runs. The world tools act on the
    // editor's own nine tiles, which `playtest::stand_the_world_down`
    // despawned on entry, and the ground on screen is the client's own stream
    // around the character. The rail greys those subjects for the same
    // reason; this check covers a tool reached any other way, which would
    // otherwise act on ground nobody is editing.
    viewport.rect = Some(match covers || in_world {
        true => egui::Rect::NOTHING,
        false => root.available_rect_before_wrap(),
    });
    // The icons over the creatures' heads, painted into the viewport before
    // anything floats over it. The list is empty under every tool but the
    // creature tool. See [`shell::servicemarks`].
    shell::servicemarks::draw(
        &ctx,
        viewport.rect.unwrap_or(egui::Rect::NOTHING),
        &editing.service_marks.0,
        &mut editing.thumbnails,
    );
    // The floating windows follow, which the panels cannot shrink around; see
    // [`Viewport`]. They are drawn after the panels, against the context
    // rather than into the root, so a popover is above the bar it hangs from.
    // A playtest starting closes the tile window, not only its button. The
    // button is disabled for the same reason as the map drop-down (making and
    // deleting tiles changes the map the character is standing on), but a
    // window already open when the playtest began would otherwise stay over
    // the game with all its actions enabled.
    if in_world {
        editing.mapview.open = false;
    }
    // The map window, drawn over everything. Its rectangle joins the
    // popovers' in [`Viewport::floating`] for the same reason: it shrinks no
    // panel, so without it a click inside it is a click on the ground behind.
    let camera_tile = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    let minimaps = assets.minimap_tiles();
    let (map_rect, asked) = map::mapview::draw(
        &ctx,
        &mut editing.mapview,
        session,
        &mut editing.thumbnails,
        &minimaps,
        &mut editing.tiles,
        camera_tile,
        [focus.position.x, focus.position.y],
        &assets.loading_screens(),
    );
    if asked.is_some() {
        editing.tiles.asked = asked;
    }

    viewport.floating = shell::popover::draw(
        &ctx,
        &mut popovers,
        &mut camera,
        &mut go_to,
        &places,
        &mut playing.login,
        &mut editing.projects,
        session,
        &assets,
        &mut editing.bookmarks,
        &mut playing.server,
        &mut playing.queue,
        &mut playing.standings,
        time.elapsed_secs_f64(),
        playing.state.playing(),
        &playing.step,
    );
    viewport.floating.extend(map_rect);
    // Select and Measure over the viewport's corner. Drawn only while the
    // viewport is the world's, which excludes a workspace and a playtest.
    let world_rect = viewport.rect.unwrap_or(egui::Rect::NOTHING);
    let corner = shell::rail::pointer(&ctx, world_rect, &mut tool, &mut viewing.rail, &viewing.icons);
    viewport.floating.extend(corner);
    // A tool's toolbar, beside Select and Measure over the top of the
    // viewport, where the clicks and strokes it sets up land. See
    // `widgets::theme::toolbar`.
    match *tool {
        Tool::Flightpaths => {
            let strip = crate::ui::tools::places::flightpaths::strip(&ctx, world_rect, corner, &mut editing.flightpaths);
            viewport.floating.extend(strip);
        }
        Tool::Terrain => {
            let bar = crate::ui::tools::ground::terrain::toolbar(
                &ctx,
                world_rect,
                corner,
                &mut editing.terrain,
                &mut editing.tablet,
                &editing.pen,
                &mut editing.brushes,
            );
            viewport.floating.extend(bar);
        }
        Tool::Textures => {
            // The guide is written back only when it changed, as the view
            // bar writes its switches, so the resource is not marked changed
            // on every frame.
            let mut full_chunks = viewing.guides.layers;
            let bar = crate::ui::tools::ground::textures::toolbar(
                &ctx,
                world_rect,
                corner,
                &mut editing.textures,
                &mut full_chunks,
                &mut editing.tablet,
                &editing.pen,
                &mut editing.brushes,
            );
            if full_chunks != viewing.guides.layers {
                viewing.guides.layers = full_chunks;
            }
            viewport.floating.extend(bar);
        }
        Tool::Creatures => {
            let bar = tools::spawns::spawn::toolbar(
                &ctx,
                world_rect,
                corner,
                &mut tools::spawns::creatures::Subject::new(
                    session,
                    &mut editing,
                    tools::spawns::Server {
                        settings: &playing.server,
                        panel: &mut popovers.server,
                        standings: &mut playing.standings,
                    },
                    &assets,
                    time.elapsed_secs_f64(),
                    true,
                ),
            );
            viewport.floating.extend(bar);
        }
        Tool::GameObjects => {
            let bar = tools::spawns::spawn::toolbar(
                &ctx,
                world_rect,
                corner,
                &mut tools::spawns::gameobjects::Subject::new(
                    session,
                    &mut editing,
                    tools::spawns::Server {
                        settings: &playing.server,
                        panel: &mut popovers.server,
                        standings: &mut playing.standings,
                    },
                    &assets,
                    time.elapsed_secs_f64(),
                    true,
                ),
            );
            viewport.floating.extend(bar);
        }
        _ => {}
    }
    // The right-click menu, on the root context so it is not clipped to a
    // panel. Its area is added to the floating list with the other areas at
    // the end of this function.
    shell::context::draw(&ctx, session, &mut editing, &assets, viewport.zoom, time.elapsed_secs_f64());
    // The Import Texture, Import Model and Import Sound dialogs, under any
    // tool: the Assets workspace opens them, a form's skin field opens the
    // first and a form's sound field the last. Each covers the window while
    // it is open, so a press on it is not a press on the ground. It is a
    // pop-out window's while that window's workspace asked for it.
    let asked_by_field = editing.assets.asked_by_field();
    if editing.popout.import_window(*tool, asked_by_field).is_none() {
        if editing.assets.dialog_open() {
            viewport.floating.push(ctx.content_rect());
        }
        import::assets::import_dialog(&ctx, &mut editing.assets, &mut editing.browser, session, &assets);
    }

    // The creature template's own window, drawn on the root context like the
    // map window: a window inside a panel is clipped to the panel, and this
    // one is larger than the panel and placed elsewhere on the screen. Its
    // rectangle joins the floating list so a click inside it is not also a
    // click on the ground behind it.
    //
    // It is drawn only under its own tool, as the quest and loot windows are.
    // Its open flag is kept, so the window reappears when the tool is chosen
    // again.
    if *tool == Tool::Creatures {
        let template = tools::spawns::spawn::template_window(
            &ctx,
            &mut tools::spawns::creatures::Subject::new(
                session,
                &mut editing,
                tools::spawns::Server {
                    settings: &playing.server,
                    panel: &mut popovers.server,
                    standings: &mut playing.standings,
                },
                &assets,
                time.elapsed_secs_f64(),
                false,
            ),
        );
        viewport.floating.extend(template);
    }
    // The game object template's window, the same window for
    // gameobject_template.
    if *tool == Tool::GameObjects {
        let template = tools::spawns::spawn::template_window(
            &ctx,
            &mut tools::spawns::gameobjects::Subject::new(
                session,
                &mut editing,
                tools::spawns::Server {
                    settings: &playing.server,
                    panel: &mut popovers.server,
                    standings: &mut playing.standings,
                },
                &assets,
                time.elapsed_secs_f64(),
                false,
            ),
        );
        viewport.floating.extend(template);
    }

    // The windows that follow a creature, a game object or an item: here, or
    // in the pop-out window last used whose workspace has them. See
    // [`subject_windows`].
    if editing.popout.subject_window(*tool).is_none() {
        let rects = subject_windows(
            &ctx,
            *tool,
            Bench {
                session: &mut **session,
                editing: &mut editing,
                playing: &mut playing,
                popovers: &mut popovers,
                assets: &assets,
                now: time.elapsed_secs_f64(),
                wireframe: None,
                particles: None,
                switch_to: &mut switch_to,
            },
        );
        viewport.floating.extend(rects);
    }
    // A request for a tool that keeps the viewport is dropped during a
    // playtest: the quest workspace's link back to the creature tool names
    // one, and that tool cannot be used then.
    if let Some(wanted) = switch_to.filter(|wanted| !in_world || wanted.survives_playtest()) {
        *tool = wanted;
    }

    // What the panels asked of the shell while they were drawn: a row to open
    // in another workspace, a tool to switch to, a texture to import. See
    // [`shell::asks`].
    shell::asks::carry_out(
        &mut editing,
        session,
        &assets,
        &mut tool,
        &mut viewing.rail,
        in_world,
        time.elapsed_secs_f64(),
    );
    // A set's item list changed in the set's form: the items' `set_id`
    // follows once their rows are read. See `Quests::follow_sets`.
    if !editing.quests.set_follows.is_empty() {
        let said = editing.quests.follow_sets(session, time.elapsed_secs_f64());
        if let Some(last) = said.last() {
            session.status = last.clone();
        }
    }

    // The waypoint window, on the root context for the same reason. Its
    // rectangle joins the floating list, so a click inside it is not also a
    // click on the ground behind it. This matters most here, because the
    // waypoint tool acts on clicks on the ground.
    let path_window = tools::spawns::waypoints::window(
        &ctx,
        &mut tools::spawns::waypoints::Subject {
            session,
            waypoints: &mut editing.waypoints,
            now: time.elapsed_secs_f64(),
            fly: &mut editing.fly,
        },
    );
    viewport.floating.extend(path_window);

    // The hover card beside the pointer, naming the creature or game object
    // under it. See [`shell::hovercard`].
    shell::hovercard::draw(
        &ctx,
        *tool,
        &editing.creatures,
        &mut editing.objects,
        &session.rows.edits,
        &assets,
    );

    // A workspace any panel or link chose while the pop-out window is open
    // goes there, and this window stays on the world. See `crate::popout`.
    editing.popout.redirect(&mut tool, viewing.rail.last_world);

    // The toast that says the world database is being read or written, drawn
    // last so it is over everything. See [`shell::status::sync_toast`].
    shell::status::sync_toast(&ctx, &playing.queue, bars_top, time.elapsed_secs_f64());

    // Every window, modal and area drawn this pass, including the ones a
    // window opens from inside itself, such as the display picker and the
    // reference picker. See [`areas_over_the_world`].
    viewport.floating.extend(areas_over_the_world(&ctx));

    Ok(())
}

/// The key in a window's egui memory that says whether it was the window last
/// used. See [`dialog_home`].
const OWNS_DIALOGS: &str = "vale-owns-dialogs";

/// Note in `ctx`'s memory whether its window was the one last used, before
/// anything in it is drawn. Every window's shell does this each frame.
pub(crate) fn mark_dialog_owner(ctx: &egui::Context, owns: bool) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OWNS_DIALOGS), owns));
}

/// Whether a dialog that several windows can draw is drawn in `ctx`'s
/// window, settling which one on its first draw.
///
/// The display picker and the reference picker are each one dialog that
/// several panels draw, and with pop-out windows those panels can be in
/// different windows, each of which would draw it. A dialog belongs to the
/// window it was opened in, which is the window last used, since opening it
/// took a press there; `home` keeps that window's context once it is known.
pub(crate) fn dialog_home(home: &mut Option<egui::Context>, ctx: &egui::Context) -> bool {
    let owns = ctx.data(|data| data.get_temp::<bool>(egui::Id::new(OWNS_DIALOGS))).unwrap_or(true);
    if home.is_none() && owns {
        *home = Some(ctx.clone());
    }
    home.as_ref() == Some(ctx)
}

/// The rectangle of every egui area drawn above the background layer this
/// pass or the one before: each `egui::Window`, each `egui::Modal`, and each
/// `egui::Area`. While a modal is open, the whole screen.
///
/// The windows above register their own rectangles, but a window that opens a
/// modal from inside itself returns only its own rectangle. A modal is drawn
/// at the centre of the screen, usually over the viewport, so a press on its
/// Next button reached the world, and under Place it spawned a creature. This
/// list covers every area whoever drew it.
///
/// A modal's area is only its frame. Its backdrop is a child `Ui` that does
/// not grow the area, so the area's rectangle leaves a press on the dimmed
/// backdrop to the world. `Memory::top_modal_layer` names a modal open on the
/// pass before, and while there is one the whole screen is listed.
///
/// `Order::Tooltip` is left out. The hover card is a tooltip-order area drawn
/// beside the pointer, and near the window's edge `constrain` can move it
/// under the pointer; it is not interactable and must not stop a press on the
/// creature it names.
fn areas_over_the_world(ctx: &egui::Context) -> Vec<egui::Rect> {
    let screen = ctx.content_rect();
    ctx.memory(|mem| {
        if mem.top_modal_layer().is_some() {
            return vec![screen];
        }
        let visible = mem.areas().visible_layer_ids();
        mem.layer_ids()
            .filter(|layer| matches!(layer.order, egui::Order::Middle | egui::Order::Foreground))
            .filter(|layer| visible.contains(layer))
            .filter_map(|layer| mem.area_rect(layer.id))
            .collect()
    })
}

/// The frame a top or bottom bar is drawn in: a fill, a hairline on the inside
/// edge, and a margin that keeps the text off the edge.
fn bar(fill: egui::Color32) -> Frame {
    Frame::default()
        .fill(fill)
        .stroke(Stroke::new(1.0, widgets::theme::LINE))
        .inner_margin(Margin::symmetric(8, 5))
}

/// The frame a side panel is drawn in.
fn side(fill: egui::Color32) -> Frame {
    Frame::default()
        .fill(fill)
        .stroke(Stroke::new(1.0, widgets::theme::LINE))
        .inner_margin(Margin::symmetric(8, 4))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A press over a panel does not belong to the world, and a press in the
    /// middle does.
    ///
    /// Each of the two egui answers is wrong here, in opposite directions; the
    /// module comment explains why. The test is on the rectangle rather than on the
    /// systems because the rectangle is the part that fails silently: a tool
    /// that never fires is noticed at once, and a tool that fires through a
    /// panel shows up only as the selection changing while the inspector is
    /// used.
    #[test]
    fn a_press_over_a_panel_is_not_the_worlds() {
        let viewport = Viewport {
            rect: Some(egui::Rect::from_min_max(
                egui::pos2(132.0, 32.0),
                egui::pos2(1312.0, 868.0),
            )),
            floating: Vec::new(),
            zoom: 1.0,
        };
        assert!(viewport.holds(Vec2::new(700.0, 400.0)), "the middle");
        assert!(!viewport.holds(Vec2::new(60.0, 400.0)), "the rail");
        assert!(!viewport.holds(Vec2::new(1400.0, 400.0)), "the inspector");
        assert!(!viewport.holds(Vec2::new(700.0, 10.0)), "the top bar");
        assert!(!viewport.holds(Vec2::new(700.0, 890.0)), "the status line");
        // The view bar between the two: the region added last, and the one a
        // tool would otherwise act through while a layer is being turned off.
        assert!(!viewport.holds(Vec2::new(700.0, 875.0)), "the view bar");
    }

    /// A press on something floating over the viewport does not belong to the
    /// world either.
    ///
    /// A popover hangs from the top bar over the viewport and shrinks nothing,
    /// so without this check a click into its password box is a click on the
    /// terrain behind it: the same fault the docked panels had.
    #[test]
    fn a_press_on_a_popover_is_not_the_worlds_either() {
        let viewport = Viewport {
            rect: Some(egui::Rect::from_min_max(
                egui::pos2(132.0, 32.0),
                egui::pos2(1312.0, 868.0),
            )),
            floating: vec![egui::Rect::from_min_max(
                egui::pos2(600.0, 36.0),
                egui::pos2(860.0, 200.0),
            )],
            zoom: 1.0,
        };
        assert!(!viewport.holds(Vec2::new(700.0, 100.0)), "in the popover");
        assert!(viewport.holds(Vec2::new(700.0, 400.0)), "under it");
        assert!(viewport.holds(Vec2::new(500.0, 100.0)), "beside it");
    }

    /// Two egui passes over a 1280x720 screen, then the viewport they leave
    /// when the four panels leave the whole screen. Two, because egui draws a
    /// new area invisible on its first pass to measure it.
    fn after_two_passes(mut draw: impl FnMut(&mut egui::Ui)) -> Viewport {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0));
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, &mut draw);
        }
        Viewport {
            rect: Some(screen),
            floating: areas_over_the_world(&ctx),
            zoom: 1.0,
        }
    }

    /// A modal that a window opens from inside itself returns no rectangle to
    /// the shell, and a press on its buttons reached the tool behind it.
    /// [`areas_over_the_world`] lists the whole screen while a modal is open,
    /// so neither a press on the modal nor one on its backdrop reaches the
    /// world.
    #[test]
    fn a_press_while_a_modal_is_open_is_not_the_worlds() {
        let viewport = after_two_passes(|ui| {
            egui::Modal::new(egui::Id::new("a-modal")).show(ui.ctx(), |ui| {
                ui.label("a modal");
            });
        });
        assert!(!viewport.holds(Vec2::new(640.0, 360.0)), "on the modal");
        assert!(!viewport.holds(Vec2::new(20.0, 20.0)), "on its backdrop");
    }

    /// A middle-order area, which is what an `egui::Window` is, refuses a
    /// press. A tooltip-order area such as the hover card does not: it is
    /// drawn beside the pointer to name what is under it.
    #[test]
    fn a_window_area_floats_and_a_tooltip_area_does_not() {
        let viewport = after_two_passes(|ui| {
            egui::Area::new(egui::Id::new("a-window"))
                .order(egui::Order::Middle)
                .fixed_pos(egui::pos2(200.0, 300.0))
                .show(ui.ctx(), |ui| {
                    ui.label("a window");
                });
            egui::Area::new(egui::Id::new("a-card"))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .fixed_pos(egui::pos2(600.0, 300.0))
                .show(ui.ctx(), |ui| {
                    ui.label("a card");
                });
        });
        assert!(!viewport.holds(Vec2::new(210.0, 305.0)), "on the window");
        assert!(viewport.holds(Vec2::new(610.0, 305.0)), "on the card");
    }

    /// With egui zoomed in, a point is more than one logical pixel, and the
    /// pointer is divided by the zoom before it is compared with the panels.
    /// Without that, at 150% only the top-left two thirds of the viewport took
    /// a press.
    #[test]
    fn a_zoomed_shell_measures_the_pointer_in_points() {
        let viewport = Viewport {
            rect: Some(egui::Rect::from_min_max(egui::pos2(132.0, 40.0), egui::pos2(633.0, 560.0))),
            zoom: 1.5,
            ..Viewport::default()
        };
        assert!(viewport.holds(Vec2::new(900.0, 800.0)), "600, 533 in points: in the world");
        assert!(!viewport.holds(Vec2::new(1000.0, 400.0)), "666 in points: on the inspector");
        assert!(!viewport.holds(Vec2::new(150.0, 400.0)), "100 in points: on the rail");
    }

    /// Before anything is drawn the whole window is viewport, as it would be
    /// for a host with no panels. Refusing every press until the first frame
    /// has been laid out would make a tool ignore a click on that frame.
    #[test]
    fn an_undrawn_shell_holds_the_whole_window() {
        assert!(Viewport::default().holds(Vec2::new(0.0, 0.0)));
        assert!(Viewport::default().holds(Vec2::new(9999.0, 9999.0)));
    }
}

/// The whole of the chrome while a playtest is running.
///
/// Anchored to the top rather than docked: during a playtest the window belongs
/// to the game, and a docked bar would take a strip of the viewport from it.
/// It sits in the middle because 1.12 puts the player's own frame in the top
/// left corner.
fn playtest_bar(
    ctx: &egui::Context,
    state: &mut Playtest,
    client: &mut vale_client::world::session::Session,
    auto: &mut vale_client::glue::autologin::AutoLogin,
    open: &mut crate::playtest::ShellOpen,
    tool: &mut Tool,
    session: &mut EditSession,
) {
    // Nothing in this bar may keep the keyboard focus. Tab is 1.12's "target
    // the nearest enemy" and egui's "move to the next widget", and egui takes
    // a key while it has focus: pressing Tab during a playtest put a focus
    // ring on the button below, so the next Enter left the world. The bar has
    // no text field, so it never keeps the focus.
    //
    // Focus is surrendered both before and after the window, and both are
    // needed. egui grants focus while the widgets are built, so surrendering
    // only beforehand leaves the ring from this frame's `show` until the next
    // frame's: one frame in which `EguiWantsInput::wants_any_keyboard_input`
    // is true. One frame is enough to cause a fault: `ui::debug::claim_input`
    // turns that into `ExternalKeyboard`, `keyboard::poll` ORs it into
    // `KeyboardFocus`, and `bindings::dispatch` skips the frame. That was one
    // of two causes of a bug where pressing Tab while running left the
    // character walking indefinitely; the other cause, which produced the
    // stuck movement, is fixed in `dispatch` itself.
    surrender(ctx);
    egui::Window::new("playtest")
        .title_bar(false)
        .resizable(false)
        .frame(
            Frame::default()
                .fill(widgets::theme::SHELL)
                .stroke(Stroke::new(1.0, widgets::theme::LINE))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(10, 5)),
        )
        .anchor(egui::Align2::CENTER_TOP, [0.0, 8.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("PLAYTEST")
                        .color(widgets::theme::GOOD)
                        .size(widgets::theme::SMALL),
                );
                ui.label(
                    egui::RichText::new(match *state {
                        Playtest::Playing => "in the world",
                        _ => "logging in",
                    })
                    .size(widgets::theme::SMALL)
                    .color(widgets::theme::INK_FAINT),
                );
                // The panels are reachable from a button as well as a key,
                // because a function reachable only from a shortcut is not
                // discoverable. The rail's own test makes the same argument
                // about a tool with no row.
                if ui
                    .button("Live Edit")
                    .on_hover_text(
                        "Ctrl+E. Open the editor panels over the running playtest. \
                         Edit a spell, item, quest or any table and save; the client \
                         uses the edited version the next time it reads it. The world \
                         tools are not available.",
                    )
                    .clicked()
                {
                    open.0 = true;
                    *tool = crate::tools::open_on(*tool);
                }
                if ui
                    .button("End Playtest")
                    .on_hover_text("Ctrl+P. Disconnects from the server and returns to the editor.")
                    .clicked()
                {
                    crate::playtest::stop(state, client, auto, session);
                }
            });
        });
    // Surrender again now that the widgets have been built; see above.
    surrender(ctx);
}

/// Clears egui's keyboard focus, whatever widget holds it.
///
/// `playtest_bar` calls it twice; the comment there gives the reason. The
/// playtest bar has no widget that takes typing, and a focus ring on one of
/// its buttons takes a key away from the game.
fn surrender(ctx: &egui::Context) {
    if let Some(focused) = ctx.memory(|memory| memory.focused()) {
        ctx.memory_mut(|memory| memory.surrender_focus(focused));
    }
}
