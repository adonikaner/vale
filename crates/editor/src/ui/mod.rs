//! The editor's panels: the docked shell around the viewport and the windows
//! drawn over it.
//!
//! ```text
//! theme.rs      the palette, the style, and the widgets built on them
//! topbar.rs     the top bar: the open project and map, the camera position,
//!               and the playtest button
//! popover.rs    the four forms the top bar opens: where to go, which account
//!               to log in as, which project, and where the server is
//! rail.rs       the subject list: what the editor edits now and what it will
//!               edit
//! inspector.rs  the panel beside the viewport: the chosen tool's controls
//! thumbnails.rs the pictures the inspector draws in a list, decoded a few per
//!               frame
//! data.rs       the workspace a table is edited in, which replaces the
//!               viewport rather than sitting beside it: the browser, the
//!               schema-driven form, and the rows a row references
//! storyboard.rs the one view of a row that is not a form: a spell's visual
//!               chain as its sequence of phases
//! creatures.rs  the form for the server's creature rows: one spawn, the
//!               creature_template behind it, and which of the two an edit
//!               changes. It is separate from data.rs's browser because a DBC
//!               row is an index into a fixed-width record and a server row is
//!               a keyed map of text
//! gameobjects.rs the same form for a game object: one spawn, the
//!               gameobject_template behind it, the 24 data columns under the
//!               names the row's type gives them, and a lock resolved through
//!               the client's Lock.dbc
//! items.rs      the workspace item_template is edited in: that form, a
//!               browser, and a third part in the inspector: a picture of what
//!               the display id a row names looks like, and the grid of every
//!               appearance it is chosen from
//! quests.rs     the workspace quest_template is edited in: the list, a form
//!               whose objectives and rewards are lines rather than numbered
//!               columns, the quest as the log would show it, who gives and
//!               takes it, and the window the creature tool opens on one
//!               creature's quests
//! behaviour.rs  the three windows a creature's behaviour is edited in: its
//!               events, the spell list its template names, and one script,
//!               with every parameter named by the event's type, the row's
//!               command or the target
//! loot.rs       the window a loot set is edited in, opened from a selected
//!               creature, a selected game object or the open item: its rows
//!               by group, each drawn as the server reads it, and the
//!               reference sets they name, followed and edited in place
//! services.rs   the Vendor and Trainer windows of a selected creature: its
//!               own list and its template's, each row drawn as the server
//!               reads it, with the reasons it would skip one
//! rowform.rs    the widgets all of those forms are built from: how a column
//!               of each kind is drawn, and what typing into one produces. The
//!               form itself stays with its subject
//! reference.rs  a column that names a row of another table, as the creature
//!               and game object forms draw it: the picker button, the name
//!               the number resolves to, and the link that opens the row
//! displays.rs   the dialog a creature's or a game object's display id is
//!               chosen in: a paged grid of rendered models, searched by
//!               model path, with a large preview
//! waypoints.rs  the window one creature's path is edited in: its points as a
//!               list, beside the same points drawn on the ground. The list
//!               and the world share one selection, so they always agree on
//!               which point is chosen
//! flightpaths.rs the flight path tool's panel: the selected node and its
//!               paths, the selected path and its points, and what a click on
//!               the ground is armed to make
//! bands.rs      the one table no reference reaches: a light's eighteen
//!               colours and six numbers over the day, each drawn as the day
//!               it produces rather than as sixteen pairs of numbers
//! lab.rs        the attachment lab's card and pane: an effect's model on a
//!               body, with the handles that move it
//! hovercard.rs  the card beside the pointer that names the creature or game
//!               object under it: a title and four or five attributes
//! mapview.rs    the map from above: which tiles exist, and which are selected
//! viewbar.rs    every toggle that changes what the viewport shows
//! icons.rs      the toolbar icons compiled into the editor (none yet)
//! status.rs     the status line at the bottom
//! sync.rs       what this project has changed on the server: one block per
//!               subject, the same four verbs for each. Drawn inside the top
//!               bar's Server… popover, which holds every server operation
//! manifest.rs   the open project's files and what each one changes, drawn in
//!               the project dialog
//! ```
//!
//! ## Layout: four docked regions
//!
//! The controls are docked rather than held in one floating `egui::Window`. A
//! floating window listing a map drop-down, a coordinate readout, three
//! "go to" rows, the brush, the history, the project and the playtest in no
//! particular order reads as a debug window.
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
//! holds session state and the inspector holds the thing being edited. Which
//! project, which map, whether the work is saved, whether the game is running
//! go in the bar. A brush radius, a placement's rotation and the undo stack go
//! in the inspector.
//!
//! The view bar is a third category: it changes what the viewport shows and
//! never what is being edited. Turning the doodads off touches no file and
//! adds nothing to the undo stack. It is a strip of its own rather than a
//! section of the inspector because the inspector is about the selection, and
//! these switches apply whatever is selected. See [`viewbar`].
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
//! `is_pointer_over_area` has no `any_down()` term and asks the right
//! question, but it also fails here: for a widget on the background layer egui
//! compares the pointer against `root_ui_available_rect`, which is written by
//! `Context::run_ui`. bevy_egui does not call `run_ui`, because it drives
//! `begin_pass`/`end_pass` itself. With the rectangle unset egui takes its
//! "we shouldn't get here" branch and reports the pointer over the interface
//! everywhere, so no tool would ever act.
//!
//! The shell therefore keeps its own answer. [`Viewport`] is the rectangle the
//! four panels leave, written here where the panels are drawn and read by
//! every tool. [`Viewport::holds`] is the one place the question is asked.

pub mod bands;
pub mod behaviour;
pub mod creatures;
pub mod data;
pub mod displays;
pub mod flightpaths;
pub mod gameobjects;
pub mod hovercard;
pub mod icons;
pub mod inspector;
pub mod items;
pub mod lab;
pub mod loot;
pub mod manifest;
pub mod mapview;
pub mod popover;
pub mod quests;
pub mod rail;
pub mod reference;
pub mod rowform;
pub mod services;
pub mod status;
pub mod storyboard;
pub mod sync;
pub mod theme;
pub mod thumbnails;
pub mod topbar;
pub mod viewbar;
pub mod waypoints;

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use egui::{CornerRadius, Frame, Margin, Stroke, UiBuilder};

use crate::camera::EditorCamera;
use crate::pick::Cursor;
use crate::playtest::{Login, Playtest};
use crate::session::EditSession;
use crate::tools::doodads::Selection;
use crate::tools::gizmo::Gizmo;
use crate::tools::terrain::Terrain;
use crate::tools::textures::Textures;
use crate::tools::Tool;
use vale_client::assets::GameAssets;
use vale_client::glue::autologin::AutoLogin;
use vale_client::render::focus::WorldFocus;

/// The part of the window the chrome leaves for the world, in window points.
///
/// It has two parts because the chrome has two kinds of element. The docked
/// panels shrink the root `Ui`, so what they leave is one rectangle. A popover,
/// a window or a modal floats over that rectangle and shrinks nothing, so its
/// rectangle is listed separately; without that, a press on a popover's
/// password box is a press on the ground behind it, the same fault the docked
/// panels had. [`popover`] and the windows return their rectangles for this
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
pub struct Viewport {
    /// `None` before the panels are first drawn; on that frame the whole
    /// window is the world.
    rect: Option<egui::Rect>,
    /// The rectangles drawn floating over it this frame: popovers, windows,
    /// modals and egui areas.
    floating: Vec<egui::Rect>,
    /// egui's zoom factor this frame: how many of the window's logical pixels
    /// one point is. Set by the shell; 1 before it has run.
    pub zoom: f32,
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
    pub fn holds(&self, at: Vec2) -> bool {
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
pub fn over_the_world(
    viewport: &Viewport,
    wants: &bevy_egui::input::EguiWantsInput,
    windows: &Query<&Window>,
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
pub struct Playing<'w> {
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
    /// draws and does not recompute every frame. See [`sync::Standings`].
    standings: ResMut<'w, sync::Standings>,
    /// The database writes waiting and running, which the Server panel queues
    /// and the toast in the corner reports. See [`crate::server::queue`].
    queue: ResMut<'w, crate::server::queue::ServerQueue>,
    /// Which step of a tile regeneration is running, for the same toast. See
    /// [`crate::server::datadir::Step`].
    step: Res<'w, crate::server::datadir::Step>,
}

/// The resources the inspector edits, as one parameter, for the same reason
/// as [`Playing`]: the shell reached the parameter limit again when a second
/// brush and a gizmo were added. They are exactly what [`inspector::draw`]
/// takes, and nothing else in this file touches any of them.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Editing<'w> {
    pub(crate) terrain: ResMut<'w, Terrain>,
    pub(crate) grading: ResMut<'w, crate::tools::grade::Grading>,
    pub(crate) measuring: ResMut<'w, crate::tools::measure::Measuring>,
    pub(crate) shading: ResMut<'w, crate::tools::shading::Shading>,
    pub(crate) textures: ResMut<'w, Textures>,
    pub(crate) selection: ResMut<'w, Selection>,
    pub(crate) wmos: ResMut<'w, crate::tools::wmos::Selection>,
    pub(crate) gizmo: ResMut<'w, Gizmo>,
    pub(crate) holes: Res<'w, crate::tools::holes::Holes>,
    pub(crate) areas: ResMut<'w, crate::tools::areas::Areas>,
    pub(crate) water: ResMut<'w, crate::tools::water::Water>,
    pub(crate) sweep: ResMut<'w, crate::tools::sweep::Sweep>,
    pub(crate) thumbnails: ResMut<'w, thumbnails::Thumbnails>,
    /// The pictures of models, which are rendered rather than decoded (see
    /// [`crate::portraits`]), and the two lists a picker keeps beside its
    /// folders.
    pub(crate) portraits: ResMut<'w, crate::portraits::Portraits>,
    pub(crate) favourites: ResMut<'w, crate::favourites::Favourites>,
    /// The camera's named places, for the go-to popover.
    pub(crate) bookmarks: ResMut<'w, crate::bookmarks::Bookmarks>,
    pub(crate) placing: ResMut<'w, crate::tools::place::Placing>,
    pub(crate) tiles: ResMut<'w, crate::tools::tiles::Tiles>,
    pub(crate) mapview: ResMut<'w, mapview::MapView>,
    pub(crate) browser: ResMut<'w, crate::tools::tables::Browser>,
    /// The world half of the lights subject: which light is chosen, where they
    /// all are, and a pending fly-to. See [`crate::tools::lights`].
    pub(crate) lights: ResMut<'w, crate::tools::lights::Lights>,
    /// The flight path subject: the nodes and paths on the map and what is
    /// selected. See [`crate::tools::flightpaths`].
    pub(crate) flightpaths: ResMut<'w, crate::tools::flightpaths::Flightpaths>,
    /// The world half of the creature subject: the map's spawns, which is
    /// selected, and the two rows behind it. See [`crate::tools::creatures`].
    pub(crate) creatures: ResMut<'w, crate::tools::creatures::Creatures>,
    /// The game-object subject's state, which has the same shape. See
    /// [`crate::tools::gameobjects`].
    pub(crate) objects: ResMut<'w, crate::tools::gameobjects::GameObjects>,
    /// The waypoint mode over the creature subject: whose path is open, which
    /// point is chosen. See [`crate::tools::waypoints`].
    pub(crate) waypoints: ResMut<'w, crate::tools::waypoints::Waypoints>,
    /// The item workspace's state: the table, which row is open, and what the
    /// appearance it names looks like. See [`crate::tools::items`].
    pub(crate) items: ResMut<'w, crate::tools::items::Items>,
    /// The quest workspace's state: the table, the relations, which quest is
    /// open. See [`crate::tools::quests`].
    pub(crate) quests: ResMut<'w, crate::tools::quests::Quests>,
    /// The loot window's state: which sets the selection names, and what each
    /// holds. See [`crate::tools::loot`].
    pub(crate) loot: ResMut<'w, crate::tools::loot::Loot>,
    /// The behaviour windows' state: the creature they are about, and what
    /// its events, its spell list and one script hold. See
    /// [`crate::tools::behaviour`].
    pub(crate) behaviour: ResMut<'w, crate::tools::behaviour::Behaviour>,
    /// The Vendor and Trainer windows' state: the creature they are about,
    /// and what its lists hold. See [`crate::tools::services`].
    pub(crate) services: ResMut<'w, crate::tools::services::Services>,
    /// The time of day the world is showing. Read-only here: the view bar
    /// sets the hour, and the panels read it so a band's day strip can mark
    /// the viewport's current hour on itself.
    pub(crate) clock: Res<'w, vale_client::render::sky::WorldClock>,
    pub(crate) storyboard: ResMut<'w, storyboard::Storyboard>,
    pub(crate) stage: ResMut<'w, crate::stage::Stage>,
    pub(crate) lab: ResMut<'w, crate::lab::Lab>,
    /// What is typed into the project popover; see [`topbar::Projects`]. It
    /// is in the bundle rather than a parameter of its own because this system
    /// is at Bevy's limit of sixteen parameters.
    pub(crate) projects: ResMut<'w, topbar::Projects>,
    /// The command line, for the two flags that open something a press would
    /// otherwise have to open: `--projects` here, `--row` in the browser.
    pub(crate) args: Res<'w, crate::Args>,
}

/// What the view bar reads and writes, as one parameter.
///
/// A bundle of its own for the same reason as [`Playing`] and [`Editing`]:
/// this shell is at Bevy's sixteen-parameter limit. These six are what
/// [`viewbar::draw`] takes, and nothing else in this file touches them.
///
/// Two of the six exist only in a `diagnostics` build. `DebugOverlay` is
/// gated in the client together with the debug window that was its only
/// surface (see `render::overlay`), so a build without the feature has no
/// overlay group. The eighteen layer switches and the three frame settings
/// (`RenderTuning`'s MSAA, vsync and sun shadows) are not gated and are always
/// present.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Viewing<'w> {
    world: ResMut<'w, vale_client::render::tuning::WorldTuning>,
    frame: ResMut<'w, vale_client::world::camera::RenderTuning>,
    #[cfg(feature = "diagnostics")]
    overlay: ResMut<'w, vale_client::render::overlay::DebugOverlay>,
    #[cfg(feature = "diagnostics")]
    wireframe: Res<'w, vale_client::render::overlay::WireframeSupported>,
    #[cfg(feature = "diagnostics")]
    drawn: Res<'w, vale_client::render::overlay::CollisionDrawn>,
    navmesh: ResMut<'w, crate::navmesh::Navmesh>,
    icons: Res<'w, icons::Icons>,
    /// Which world tool the World part of the top bar returns to. See
    /// [`rail::Rail`].
    rail: ResMut<'w, rail::Rail>,
}

pub struct PanelPlugin;

impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<mapview::MapView>()
            .init_resource::<topbar::GoTo>()
            .init_resource::<topbar::Projects>()
            .init_resource::<popover::Popovers>()
            .init_resource::<sync::Standings>()
            .init_resource::<Viewport>()
            .init_resource::<rail::Rail>()
            .add_plugins((thumbnails::ThumbnailPlugin, icons::IconPlugin))
            .add_systems(EguiPrimaryContextPass, draw);
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
    mut go_to: ResMut<topbar::GoTo>,
    mut popovers: ResMut<popover::Popovers>,
    mut viewport: ResMut<Viewport>,
    mut viewing: Viewing,
    baking: Res<crate::jobs::Running<crate::tools::tiles::Baked>>,
) -> Result {
    let Some(session) = session.as_mut() else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?.clone();
    theme::install(&ctx);
    viewport.zoom = ctx.zoom_factor();
    // The two lists kept beside the project folders, read once per install.
    // See `crate::favourites` for where they are stored and why.
    if let Some(dir) = session.project.root.parent() {
        editing.favourites.load_from(dir);
        editing.bookmarks.load_from(dir);
    }

    // A playtest has two layouts, chosen by [`crate::playtest::ShellOpen`].
    // Shut, the whole window is the game's and the bar is one row of controls.
    // Open, the panels are drawn exactly as they are while editing, so there
    // is no second layout to learn for the same subjects. Only the rail's
    // availability and the right-hand end of the top bar change, and both
    // state the reason.
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
    // the bar has been drawn once. See `popover::Popover::show`.
    if editing.args.projects && !editing.projects.shown_once {
        editing.projects.shown_once = true;
        popovers.project.show();
        // `--doom <name>`: the confirmation, open on that project.
        if let Some(name) = editing.args.doom.as_deref() {
            let clear = name == session.project.name;
            let held = match clear {
                true => crate::server::held::held_by(&session.project),
                false => vale_edit::project::Project::open(&assets.root, name)
                    .map(|project| crate::server::held::held_by(&project))
                    .unwrap_or_default(),
            };
            editing.projects.confirming =
                Some(topbar::Doomed::new(name.to_string(), clear, &held));
        }
    }
    // `--server`, once, for the same reason.
    if editing.args.server && !popovers.server_shown_once {
        popovers.server_shown_once = true;
        popovers.server.show();
    }

    egui::Panel::top("editor-top")
        .frame(bar(theme::PANEL))
        .show(&mut root, |ui| {
            topbar::draw(
                ui,
                session,
                &mut camera,
                &mut popovers,
                &assets,
                &mut topbar::Session {
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
                &mut topbar::Subjects {
                    tool: &mut tool,
                    rail: &mut viewing.rail,
                },
            );
        });

    // The background work the status line shows: the shadow rebakes, and the
    // server's tile tools while a publish or a regeneration runs them.
    // Each entry carries a name so the two progress bars can be told apart
    // when both run; the names are the map window's own button names.
    let mut working: Vec<(&str, String, f32)> = baking
        .summary()
        .into_iter()
        .map(|(label, fraction)| ("Shadows", label, fraction))
        .collect();
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
            viewbar::scripted(&edited_world, &baseline, &edited_overlay, viewing.navmesh.on)
        }
        #[cfg(not(feature = "diagnostics"))]
        {
            viewbar::scripted(&edited_world, &baseline, viewing.navmesh.on)
        }
    };
    egui::Panel::bottom("editor-status")
        .frame(bar(theme::SHELL))
        .show(&mut root, |ui| {
            status::draw(
                ui, session, &cursor, &focus, &camera, &scripted, &working, in_world,
            );
        });

    egui::Panel::bottom("editor-view")
        .frame(bar(theme::PANEL))
        .show(&mut root, |ui| {
            viewbar::draw(
                ui,
                &mut viewbar::Bar {
                    world: &mut edited_world,
                    frame: &mut edited_frame,
                    #[cfg(feature = "diagnostics")]
                    overlay: &mut edited_overlay,
                    #[cfg(feature = "diagnostics")]
                    wireframe: &viewing.wireframe,
                    #[cfg(feature = "diagnostics")]
                    drawn: &viewing.drawn,
                    navmesh: &mut viewing.navmesh,
                    baseline: &baseline,
                    icons: &viewing.icons,
                },
            );
        });

    egui::Panel::left("editor-rail")
        .exact_size(theme::RAIL_WIDTH)
        .frame(side(theme::SHELL))
        .show(&mut root, |ui| {
            // The last argument is whether a world database is reachable. A
            // tool whose rows are in vmangos' database is greyed without one;
            // see [`rail::draw`].
            rail::draw(
                ui,
                &mut tool,
                &mut viewing.rail,
                &viewing.icons,
                in_world,
                playing.server.resolve().is_some(),
            );
            // Claim the rest of the rail's height. Without this the panel is
            // only as tall as its rows, so its fill stops part way down the
            // window and leaves a gap in the shell.
            ui.allocate_rect(ui.available_rect_before_wrap(), egui::Sense::hover());
        });

    // The DBCs, loaded only for the two panels that name an area: the zone
    // tool and the measuring tool. `GameAssets::display_tables` loads every
    // table on its first call, so asking unconditionally would make an editor
    // that opens on the terrain brush load `AreaTable`, `Map` and the rest at
    // startup. Asked here, the cost is paid the first time either tool is
    // chosen.
    let tables = matches!(*tool, Tool::Areas | Tool::Measure)
        .then(|| assets.display_tables().ok())
        .flatten();
    // The inspector is resizable and the rail is not. The rail is a fixed
    // list of words; the inspector shows the chosen tool's controls, and two
    // tools show content no single width fits: a tileset's folder and file
    // name, and a row of five named choices in one control. The default width
    // suits most panels and the drag handle is for those two.
    egui::Panel::right("editor-inspector")
        .default_size(theme::INSPECTOR_WIDTH)
        .min_size(theme::INSPECTOR_MIN)
        .max_size(theme::INSPECTOR_MAX)
        .resizable(true)
        .frame(side(theme::PANEL))
        .show(&mut root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                inspector::draw(
                    ui,
                    inspector::Subject {
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
    // `inspector::draw` handles that, so nothing is needed for either here.
    // Taken out of the bundle before the workspace borrows the rest of it.
    let showing_hour = editing.clock.half_minutes;
    let covers = tool.covers_viewport();
    // A tool a panel asks to switch to, applied after everything is drawn.
    // The quest workspace's "back to Creatures" and the creature tool's quest
    // window both request one, and the tool is borrowed while they are drawn.
    let mut switch_to: Option<Tool> = None;
    if covers {
        // The central panel has no fill because the storyboard's preview is a
        // hole that shows the world, and a panel background would paint over
        // it. Each docked panel inside carries its own fill, and the fields
        // view paints the area it covers itself. See `data::draw`.
        egui::CentralPanel::default_margins()
            .frame(egui::Frame::NONE)
            .show(&mut root, |ui| {
                // Two workspaces take the middle, and they are separate
                // panels: a spell's row references a chain of other rows, and
                // an item's row references one display id that cannot be read
                // as text. See `ui::items`, whose third part, the appearance
                // in the inspector, exists for that difference.
                if *tool == Tool::Items {
                    items::draw(
                        ui,
                        items::Workspace {
                            session,
                            items: &mut editing.items,
                            assets: &assets,
                            thumbnails: &mut editing.thumbnails,
                            portraits: &mut editing.portraits,
                            quests: &mut editing.quests,
                            loot: &mut editing.loot,
                            server: &playing.server,
                            server_panel: &mut popovers.server,
                            reloads: &mut playing.reloads,
                            patch: crate::tools::creatures::server_patch(&playing.server),
                            now: time.elapsed_secs_f64(),
                        },
                    );
                    return;
                }
                // The quest workspace has the item workspace's shape with a
                // different third part. See `ui::quests`.
                if *tool == Tool::Quests {
                    quests::draw(
                        ui,
                        quests::Workspace {
                            session,
                            quests: &mut editing.quests,
                            items: &mut editing.items,
                            assets: &assets,
                            thumbnails: &mut editing.thumbnails,
                            now: time.elapsed_secs_f64(),
                        },
                        quests::Shell {
                            reloads: &mut playing.reloads,
                            server_panel: &mut popovers.server,
                            patch: crate::tools::creatures::server_patch(&playing.server),
                            switch_to: &mut switch_to,
                        },
                    );
                    return;
                }
                data::draw(
                    ui,
                    data::Workspace {
                        tool: *tool,
                        session,
                        browser: &mut editing.browser,
                        assets: &assets,
                        thumbnails: &mut editing.thumbnails,
                        portraits: &mut editing.portraits,
                        favourites: &mut editing.favourites,
                        now: time.elapsed_secs_f64(),
                        // The model view's foot toggles the same two the view
                        // bar does, on the same edited copies.
                        #[cfg(feature = "diagnostics")]
                        wireframe: Some(&mut edited_overlay.wireframe),
                        #[cfg(not(feature = "diagnostics"))]
                        wireframe: None,
                        particles: Some(&mut edited_world.particles),
                        lights: Some(&mut editing.lights),
                        hour: showing_hour,
                    },
                    &mut editing.storyboard,
                    &mut editing.stage,
                    &mut editing.lab,
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
    let (map_rect, asked) = mapview::draw(
        &ctx,
        &mut editing.mapview,
        session,
        &mut editing.thumbnails,
        &minimaps,
        &mut editing.tiles,
        camera_tile,
    );
    if asked.is_some() {
        editing.tiles.asked = asked;
    }

    viewport.floating = popover::draw(
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
    let corner = rail::pointer(&ctx, world_rect, &mut tool, &mut viewing.rail, &viewing.icons);
    viewport.floating.extend(corner);

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
        let template = creatures::template_window(
            &ctx,
            &mut creatures::Subject {
                session,
                creatures: &mut editing.creatures,
                waypoints: &mut editing.waypoints,
                quests: &mut editing.quests,
                loot: &mut editing.loot,
                behaviour: &mut editing.behaviour,
                services: &mut editing.services,
                server: &playing.server,
                server_panel: &mut popovers.server,
                assets: &assets,
                portraits: &mut editing.portraits,
                now: time.elapsed_secs_f64(),
                gizmo: None,
            },
        );
        viewport.floating.extend(template);
    }
    // The game object template's window, the same window for
    // gameobject_template.
    if *tool == Tool::GameObjects {
        let template = gameobjects::template_window(
            &ctx,
            &mut gameobjects::Subject {
                session,
                objects: &mut editing.objects,
                quests: &mut editing.quests,
                loot: &mut editing.loot,
                server: &playing.server,
                server_panel: &mut popovers.server,
                assets: &assets,
                portraits: &mut editing.portraits,
                now: time.elapsed_secs_f64(),
                gizmo: None,
            },
        );
        viewport.floating.extend(template);
    }

    // The quest window of a creature or a game object, on the root context
    // for the same reason. It follows the selection as the template window
    // does: it shows the chosen holder's quests, so selecting another holder
    // shows that one's. It is drawn only under its two tools; elsewhere the
    // same rows are edited in the quest workspace.
    let holder = match *tool {
        Tool::Creatures => editing
            .creatures
            .chosen_edited(Some(&session.server_edits))
            .map(|spawn| {
                (
                    crate::tools::quests::Holder::Creature,
                    spawn.entry,
                    spawn.label(),
                    quests::HolderIs::Creature {
                        npc_flags: spawn.npc_flags,
                        template_key: spawn.template_key(),
                    },
                )
            }),
        Tool::GameObjects => editing
            .objects
            .chosen_edited(Some(&session.server_edits))
            .map(|spawn| {
                (
                    crate::tools::quests::Holder::Object,
                    spawn.entry,
                    spawn.label(),
                    quests::HolderIs::Object {
                        kind: spawn.known.as_ref().map(|known| known.kind).unwrap_or(0),
                        template_key: spawn.template_key(),
                    },
                )
            }),
        _ => None,
    };
    if let Some((kind, entry, label, is)) = holder {
        if editing.quests.window_for.is_some() {
            editing.quests.window_for = Some((kind, entry, label));
        }
        let quest_window = quests::holder_window(
            &ctx,
            quests::HolderQuests {
                session,
                quests: &mut editing.quests,
                assets: &assets,
                is,
                switch_to: &mut switch_to,
                now: time.elapsed_secs_f64(),
            },
        );
        viewport.floating.extend(quest_window);
    }
    if let Some(wanted) = switch_to {
        *tool = wanted;
    }

    // The loot window of a creature, a game object or an item, on the root
    // context for the same reason, following the selection as the quest
    // window does. Its subject is rebuilt here every frame it is open, from
    // the selection with the project's edits applied, so a `loot_id` typed
    // into the template form changes the tab on the next frame. See
    // [`crate::tools::loot::Window`].
    let mut fixes: Vec<Option<loot::Fix>> = Vec::new();
    let described = match *tool {
        Tool::Creatures => editing
            .creatures
            .chosen_edited(Some(&session.server_edits))
            .map(|spawn| {
                let key = spawn.template_key();
                let template = editing
                    .creatures
                    .template
                    .as_ref()
                    .filter(|held| held.entry == spawn.entry);
                let column = |name: &str| -> Option<u32> {
                    use vale_mangos::creature::RowValue;
                    session
                        .server_edits
                        .get(vale_mangos::creature::TEMPLATE, &key, name)
                        .and_then(|value| value.trim().parse::<i64>().ok())
                        .or_else(|| template?.row.integer(name))
                        .map(|value| value.max(0) as u32)
                };
                let sets = match template {
                    None => Vec::new(),
                    Some(_) => {
                        let mut sets = Vec::new();
                        for (table, name) in [
                            (vale_mangos::loot::CREATURE, "loot_id"),
                            (vale_mangos::loot::PICKPOCKETING, "pickpocket_loot_id"),
                            (vale_mangos::loot::SKINNING, "skinning_loot_id"),
                        ] {
                            let entry = column(name).unwrap_or(0);
                            sets.push(crate::tools::loot::Set::new(table, entry));
                            fixes.push((entry == 0).then(|| loot::Fix {
                                table: vale_mangos::creature::TEMPLATE,
                                key: key.clone(),
                                column: name,
                                value: spawn.entry.to_string(),
                                label: format!("Set {name} to {}", spawn.entry),
                                about: format!(
                                    "Write creature_template.{name} = {}, which is the \
                                     convention the shipped rows follow. It is a creature \
                                     edit: applied from the Creatures block of the Server \
                                     panel, and it needs the server restarted.",
                                    spawn.entry
                                ),
                                gesture: "Edit creature",
                            }));
                        }
                        sets
                    }
                };
                crate::tools::loot::Window {
                    title: spawn.label(),
                    about: match template {
                        Some(_) => format!(
                            "creature_template entry {}: every spawn of it drops these.",
                            spawn.entry
                        ),
                        None => "reading the template row\u{2026}".to_string(),
                    },
                    sets,
                }
            }),
        Tool::GameObjects => editing
            .objects
            .chosen_edited(Some(&session.server_edits))
            .map(|spawn| {
                let key = spawn.template_key();
                let known = spawn.known.as_ref();
                let looted = known.is_some_and(|known| known.is_looted());
                let entry = known.and_then(|known| known.loot()).unwrap_or(0);
                if looted {
                    fixes.push((entry == 0).then(|| loot::Fix {
                        table: vale_mangos::gameobject::TEMPLATE,
                        key: key.clone(),
                        column: "data1",
                        value: spawn.entry.to_string(),
                        label: format!("Set lootId to {}", spawn.entry),
                        about: format!(
                            "Write gameobject_template.data1 = {}, which is the convention \
                             the shipped rows follow. It is a game object edit: applied from \
                             the Objects block of the Server panel, and it needs the server \
                             restarted.",
                            spawn.entry
                        ),
                        gesture: "Edit game object",
                    }));
                }
                crate::tools::loot::Window {
                    title: spawn.label(),
                    about: match (known, looted) {
                        (None, _) => "No gameobject_template row at this content patch.".to_string(),
                        (Some(known), false) => format!(
                            "type is {}, which the server takes no loot from: only a chest and \
                             a fishing hole have a lootId it reads.",
                            known.type_word()
                        ),
                        (Some(_), true) => format!(
                            "gameobject_template entry {}: every spawn of it holds these.",
                            spawn.entry
                        ),
                    },
                    sets: match looted {
                        true => vec![crate::tools::loot::Set::new(vale_mangos::loot::GAMEOBJECT, entry)],
                        false => Vec::new(),
                    },
                }
            }),
        Tool::Items => editing.items.open_item(&session.server_edits).map(|known| {
            use vale_mangos::item::RowValue;
            let key = known.key();
            let disenchant = session
                .server_edits
                .get(vale_mangos::item::TEMPLATE, &key, "disenchant_id")
                .and_then(|value| value.trim().parse::<i64>().ok())
                .or_else(|| {
                    editing
                        .items
                        .row
                        .as_ref()
                        .filter(|held| held.entry == known.entry)
                        .and_then(|held| held.row.integer("disenchant_id"))
                })
                .map(|value| value.max(0) as u32);
            // `ITEM_FLAG_LOOTABLE`, the bit `LoadLootTemplates_Item` reads.
            const LOOTABLE: u32 = 0x4;
            fixes.push((known.flags & LOOTABLE == 0).then(|| loot::Fix {
                table: vale_mangos::item::TEMPLATE,
                key: key.clone(),
                column: "flags",
                value: (known.flags | LOOTABLE).to_string(),
                label: "Set LOOTABLE".to_string(),
                about: "Write item_template.flags with LOOTABLE (0x4) added, which is what \
                        makes the client offer to open the item and the server read this \
                        set. It is an item edit: applied from the Items block of the \
                        Server panel, live on the reload."
                    .to_string(),
                gesture: "Edit item",
            }));
            fixes.push(None);
            let mut sets = vec![crate::tools::loot::Set::new(vale_mangos::loot::ITEM, known.entry)];
            match disenchant {
                Some(disenchant) => {
                    sets.push(crate::tools::loot::Set::new(vale_mangos::loot::DISENCHANT, disenchant));
                }
                None => {
                    fixes.pop();
                }
            }
            crate::tools::loot::Window {
                title: known.label(),
                about: match known.flags & LOOTABLE == 0 {
                    true => format!(
                        "item_template entry {}: flags has no LOOTABLE bit, so the server \
                         reads no item_loot_template for it and the client will not open it.",
                        known.entry
                    ),
                    false => format!("item_template entry {}: opening it gives these.", known.entry),
                },
                sets,
            }
        }),
        _ => None,
    };
    editing.loot.window = described;
    let loot_window = loot::window(
        &ctx,
        loot::Subject {
            session,
            loot: &mut editing.loot,
            quests: &mut editing.quests,
            items: &mut editing.items,
            assets: &assets,
            thumbnails: &mut editing.thumbnails,
            fixes,
            now: time.elapsed_secs_f64(),
        },
    );
    viewport.floating.extend(loot_window);

    // The behaviour windows of a creature, on the root context for the same
    // reason. Their subject is rebuilt each frame from the selection with the
    // project's edits over it, so `ai_name` or `spell_list_id` typed into the
    // template form changes the window on the next frame. Drawn under the
    // creature tool only; the windows keep their open flags across tools.
    editing.behaviour.about = match *tool {
        Tool::Creatures => editing
            .creatures
            .chosen_edited(Some(&session.server_edits))
            .map(|spawn| {
                let key = spawn.template_key();
                let template = editing
                    .creatures
                    .template
                    .as_ref()
                    .filter(|held| held.entry == spawn.entry);
                let column = |name: &str| -> String {
                    use vale_mangos::creature::RowValue;
                    session
                        .server_edits
                        .get(vale_mangos::creature::TEMPLATE, &key, name)
                        .map(|value| super::ui::rowform::unquote(value))
                        .or_else(|| template.and_then(|held| held.row.text(name).map(str::to_string)))
                        .unwrap_or_default()
                };
                crate::tools::behaviour::About {
                    entry: spawn.entry,
                    label: spawn.label(),
                    template_key: key.clone(),
                    spell_list_id: column("spell_list_id").trim().parse().unwrap_or(0),
                    ai_name: column("ai_name"),
                }
            }),
        _ => None,
    };
    // The Vendor and Trainer windows of a creature, on the same terms: the
    // subject is rebuilt each frame from the selection with the project's
    // edits over the template row, so a `vendor_id`, a `trainer_id` or
    // `npc_flags` typed into the template form changes the window on the next
    // frame.
    editing.services.about = match *tool {
        Tool::Creatures => editing
            .creatures
            .chosen_edited(Some(&session.server_edits))
            .and_then(|spawn| {
                let key = spawn.template_key();
                let template = editing
                    .creatures
                    .template
                    .as_ref()
                    .filter(|held| held.entry == spawn.entry)?;
                let column = |name: &str| -> u32 {
                    use vale_mangos::creature::RowValue;
                    session
                        .server_edits
                        .get(vale_mangos::creature::TEMPLATE, &key, name)
                        .and_then(|value| value.trim().parse::<i64>().ok())
                        .or_else(|| template.row.integer(name))
                        .map_or(0, |value| value.max(0) as u32)
                };
                Some(crate::tools::services::About {
                    entry: spawn.entry,
                    label: spawn.label(),
                    template_key: key.clone(),
                    npc_flags: column("npc_flags"),
                    vendor_id: column("vendor_id"),
                    trainer_id: column("trainer_id"),
                    trainer_type: column("trainer_type"),
                    trainer_class: column("trainer_class"),
                    trainer_race: column("trainer_race"),
                    trainer_spell: column("trainer_spell"),
                })
            }),
        _ => None,
    };
    if *tool == Tool::Creatures {
        let service_windows = services::windows(
            &ctx,
            services::Subject {
                session,
                services: &mut editing.services,
                quests: &mut editing.quests,
                items: &mut editing.items,
                assets: &assets,
                thumbnails: &mut editing.thumbnails,
                now: time.elapsed_secs_f64(),
            },
        );
        viewport.floating.extend(service_windows);
    }
    if *tool == Tool::Creatures {
        let behaviour_windows = behaviour::windows(
            &ctx,
            behaviour::Subject {
                session,
                behaviour: &mut editing.behaviour,
                quests: &mut editing.quests,
                assets: &assets,
                now: time.elapsed_secs_f64(),
            },
        );
        viewport.floating.extend(behaviour_windows);
    }

    // An item a quest names, clicked in the quest form or its inspector,
    // opens the item workspace on that item.
    if let Some(entry) = editing.quests.show_item.take() {
        editing.items.open = Some(entry);
        *tool = Tool::Items;
    }
    // A reference clicked on the item, creature or game object form: a client
    // table's row opens the table browser on it under the spell subject, which
    // browses any table a reference reaches; a quest opens the quest
    // workspace. The item form writes these requests to `show_row` and
    // `show_quest` on the item tool's state, and the other two forms write
    // them to the same fields on the quest tool's state.
    let row = editing.items.show_row.take().or_else(|| editing.quests.show_row.take());
    if let Some((table, id)) = row {
        if session.open_table(&assets, table) && editing.browser.follow(session, table, id) {
            editing.browser.followed_in = true;
            *tool = Tool::Spells;
        }
    }
    let quest = editing.items.show_quest.take().or_else(|| editing.quests.show_quest.take());
    if let Some(entry) = quest {
        editing.quests.open = Some(entry);
        editing.quests.of_holder = None;
        editing.quests.forget_matches();
        *tool = Tool::Quests;
    }

    // The waypoint window, on the root context for the same reason. Its
    // rectangle joins the floating list, so a click inside it is not also a
    // click on the ground behind it. This matters most here, because the
    // waypoint tool acts on clicks on the ground.
    let path_window = waypoints::window(
        &ctx,
        &mut waypoints::Subject {
            session,
            waypoints: &mut editing.waypoints,
            now: time.elapsed_secs_f64(),
        },
    );
    viewport.floating.extend(path_window);

    // The hover card beside the pointer, naming the creature or game object
    // under it. See [`hovercard`].
    hovercard::draw(
        &ctx,
        *tool,
        &editing.creatures,
        &mut editing.objects,
        &session.server_edits,
        &assets,
    );

    // The toast that says the world database is being read or written, drawn
    // last so it is over everything. See [`status::sync_toast`].
    status::sync_toast(&ctx, &playing.queue, bars_top, time.elapsed_secs_f64());

    // Every window, modal and area drawn this pass, including the ones a
    // window opens from inside itself, such as the display picker and the
    // reference picker. See [`areas_over_the_world`].
    viewport.floating.extend(areas_over_the_world(&ctx));

    Ok(())
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
        .stroke(Stroke::new(1.0, theme::LINE))
        .inner_margin(Margin::symmetric(8, 5))
}

/// The frame a side panel is drawn in.
fn side(fill: egui::Color32) -> Frame {
    Frame::default()
        .fill(fill)
        .stroke(Stroke::new(1.0, theme::LINE))
        .inner_margin(Margin::symmetric(8, 4))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A press over a panel does not belong to the world, and a press in the
    /// middle does.
    ///
    /// Both egui answers got this wrong, one in each direction; the module
    /// comment sets out why. The test is on the rectangle rather than on the
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
                .fill(theme::SHELL)
                .stroke(Stroke::new(1.0, theme::LINE))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(10, 5)),
        )
        .anchor(egui::Align2::CENTER_TOP, [0.0, 8.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("PLAYTEST")
                        .color(theme::GOOD)
                        .size(theme::SMALL),
                );
                ui.label(
                    egui::RichText::new(match *state {
                        Playtest::Playing => "in the world",
                        _ => "logging in",
                    })
                    .size(theme::SMALL)
                    .color(theme::INK_FAINT),
                );
                // The panels are reachable from a button as well as a key,
                // because a function reachable only from a shortcut is not
                // discoverable. The rail's own test makes the same argument
                // about a tool with no row.
                if ui
                    .button("Live Edit")
                    .on_hover_text(
                        "Ctrl+E. Open the editor's panels over the running game: \
                         change a table, save, and the next thing to read it is \
                         the edited one. Not available for world tools.",
                    )
                    .clicked()
                {
                    open.0 = true;
                    *tool = crate::tools::open_on(*tool);
                }
                if ui
                    .button("End Playtest")
                    .on_hover_text("Ctrl+P. Drops the connection and returns to the tools.")
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
