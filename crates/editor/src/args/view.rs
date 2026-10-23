//! The flags for what the editor opens on: the map, the place and the camera,
//! the tool, the overlays and guides, and the windows and modes open at the
//! start.

use super::Arity::*;
use super::{first, named, number, parse_pair, parse_tool, parse_view, text, Flag};
use crate::tools;

/// What the editor opens on.
#[derive(Debug, Clone)]
pub(crate) struct View {
    /// The map to open, by its `World\Maps\` directory name.
    pub(crate) map: String,
    /// The position to open at, in the world's own axes. Without it the editor
    /// opens at the middle of the map, which for an outdoor map is in the
    /// ocean, so a run with no `--at` expects the person to choose a place from
    /// the panel.
    pub(crate) at: Option<(f32, f32)>,
    /// `--project <name>`: the project folder under `Edit\`, which is what an
    /// edit is written to. `None` opens the project opened last, or the
    /// default when there is none — see `session::open`.
    pub(crate) project: Option<String>,
    /// `--without <layer>[,<layer>…]`: leave those layers out of the world.
    ///
    /// The client's own flag, parsed by the client's own
    /// `render::tuning::WorldTuning::without`, so the names are its names and a
    /// misspelling warns. The editor accepts it because the view bar prints the
    /// `--without` list that reproduces whatever has been switched off, and
    /// that printed list has to be usable on this command line.
    ///
    /// The layer that matters for a wide `--reach` is `fog`. Light.dbc closes
    /// the fog at 500 yards at the latest, so a scripted picture taken from
    /// further back than that shows only the fog colour, whatever is loaded
    /// behind it.
    pub(crate) without: Option<String>,
    /// `--overlay <name>[,<name>…]`: draw those overlays over the world, which
    /// are otherwise the view bar's buttons. The client's own flag, parsed by
    /// `render::overlay::DebugOverlay::with`, so the names are its names and a
    /// misspelling warns. `collision` is the one this editor most needs from a
    /// script: it is drawn round the world's focus, which a scripted run puts
    /// under the camera. Not the navmesh, which is the editor's own overlay
    /// and has its own flag, `--navmesh`.
    pub(crate) overlay: Option<String>,
    /// `--guides <name>[,<name>…]`: draw those ground guides, which are
    /// otherwise the switches in the view bar's Guides menu: `chunks`,
    /// `tiles`, `slope`, `contours`, `layers`. A scripted run cannot open the
    /// menu, and a misspelling warns. See `tools::guides`.
    pub(crate) guides: Option<String>,
    /// `--guides-menu`: open the view bar's Guides menu, for a picture of it.
    pub(crate) guides_menu: bool,
    /// `--view <distance>[,<pitch°>[,<yaw°>]]`: where the camera stands.
    ///
    /// `--at` sets where the editor looks and this sets the distance and angle,
    /// the same split as the client's own `--view`. It exists for the same
    /// reason as `--row` and `--browse`: the camera opens 60 yards out and a
    /// scripted run has no mouse wheel, so without this flag a scripted picture
    /// cannot show distant ground. That includes the
    /// streamed block itself, which is 3,733 yards across and cannot be seen
    /// from 60.
    pub(crate) camera: Option<(f32, Option<f32>, Option<f32>)>,
    /// `--reach <tiles>`: how many tiles out to stream, as a radius.
    ///
    /// The editor's default is [`crate::REACH`] rather than the client's 1, and
    /// this sets a larger or smaller value: a smaller one for a machine that
    /// cannot hold the 7x7, a larger one such as the 11x11 for shaping a whole
    /// zone. It is a client resource (`render::terrain::TerrainReach`) and the
    /// client clamps it; see that type for what a tile costs.
    pub(crate) reach: Option<i32>,
    /// The window size, or bevy's default.
    pub(crate) size: Option<(u32, u32)>,
    /// `--tool <name>`: which subject to open on.
    ///
    /// A convenience for a person and the only route for a scripted run: the
    /// rail is a click and the inspector draws whatever the rail chose, so
    /// without this `--shot` can photograph only the panel the editor starts
    /// on. See [`crate::ui::tools`].
    ///
    /// Many of the scripted flags in the other groups set it too, because what
    /// they name belongs to one tool.
    pub(crate) tool: Option<tools::Tool>,
    /// `--pop-out [workspace,...]`: open the workspace `--tool` names in a
    /// window of its own, with the main window on the world, and each
    /// workspace listed (by the names `--tool` takes) in another. With
    /// `--shot`, each window is photographed too, beside the main window's
    /// picture. See `crate::popout`.
    pub(crate) pop_out: bool,
    pub(crate) pop_out_also: Vec<tools::Tool>,
    /// `--projects`: open the project popover, which is otherwise a press on
    /// the bar and so unreachable from a scripted run. Same reason as
    /// `--place`.
    pub(crate) projects: bool,
    /// `--doom <name>`: open the project dialog with its confirmation on that
    /// project, as though Clear files (the open project) or Delete (another)
    /// had been pressed. The confirmation is what asks before the record of an
    /// apply is lost, and nothing else can open it from a script.
    pub(crate) doom: Option<String>,
    /// `--tiles`: open the map window, and the tile tool with it.
    ///
    /// Same reason as `--place`: the window is two clicks in, so without this
    /// it is the one part of the chrome `--shot` cannot photograph, and its
    /// layout goes unchecked. See [`crate::instruments::shot`].
    pub(crate) show_map: bool,
    /// `--new-map`: the map window with its New map form open on the loading
    /// screen pictures, for the same reason as `--tiles`.
    pub(crate) new_map: bool,
    /// `--navmesh`: start with the server's navmesh drawn over the ground,
    /// which is otherwise the view bar's NAV button. See [`crate::navmesh`].
    pub(crate) navmesh: bool,
    /// `--map-view`: start in the top-down map view, which is otherwise the
    /// view bar's MAP button. See [`crate::camera::TopDown`].
    pub(crate) map_view: bool,
}

impl Default for View {
    fn default() -> View {
        View {
            map: "Azeroth".into(),
            at: None,
            project: None,
            without: None,
            overlay: None,
            guides: None,
            guides_menu: false,
            camera: None,
            reach: None,
            size: None,
            tool: None,
            pop_out: false,
            pop_out_also: Vec::new(),
            projects: false,
            doom: None,
            show_map: false,
            new_map: false,
            navmesh: false,
            map_view: false,
        }
    }
}

pub(super) const FLAGS: &[Flag] = &[
    Flag::new("--map", One, |a, v| {
        if let Some(map) = text(v) {
            a.view.map = map;
        }
    }),
    Flag::new("--at", One, |a, v| a.view.at = first(v).and_then(parse_pair)),
    Flag::new("--project", One, |a, v| a.view.project = text(v)),
    Flag::new("--without", One, |a, v| a.view.without = text(v)),
    Flag::new("--overlay", One, |a, v| a.view.overlay = text(v)),
    Flag::new("--guides", One, |a, v| a.view.guides = text(v)),
    Flag::new("--guides-menu", Switch, |a, _| a.view.guides_menu = true),
    Flag::new("--view", One, |a, v| a.view.camera = first(v).and_then(parse_view)),
    Flag::new("--reach", One, |a, v| a.view.reach = number(v)),
    Flag::new("--size", One, |a, v| {
        a.view.size = first(v).and_then(parse_pair).map(|(w, h)| (w as u32, h as u32));
    }),
    Flag::new("--tool", One, |a, v| a.view.tool = first(v).and_then(parse_tool)),
    // The list is optional, so a following flag is not eaten.
    Flag::new("--pop-out", Optional, |a, v| {
        a.view.pop_out = true;
        if let Some(listed) = first(v) {
            a.view.pop_out_also = listed.split(',').filter_map(parse_tool).collect();
        }
    }),
    Flag::new("--projects", Switch, |a, _| a.view.projects = true),
    Flag::new("--doom", One, |a, v| {
        a.view.doom = named(v);
        a.view.projects = true;
    }),
    Flag::new("--tiles", Switch, |a, _| a.view.show_map = true),
    Flag::new("--new-map", Switch, |a, _| {
        a.view.show_map = true;
        a.view.new_map = true;
    }),
    Flag::new("--navmesh", Switch, |a, _| a.view.navmesh = true),
    Flag::new("--map-view", Switch, |a, _| a.view.map_view = true),
];
