//! Flight paths: the taxi nodes on the open map and the paths between them,
//! drawn in the world, picked, dragged, added and removed.
//!
//! The document is three client tables:
//!
//! ```text
//! TaxiNodes     a node: where a flight master's flights start and end
//! TaxiPath      one flight from one node to another, in one direction
//! TaxiPathNode  the points that flight follows, numbered from 0
//! ```
//!
//! What an edit to them is, and the two rules every edit keeps (a path's
//! points numbered without a gap, and no point naming a path that does not
//! exist), is `vale_edit::dbc::taxi`. This file is the pointer and the
//! drawing. The panel is `crate::ui::flightpaths`.
//!
//! ## What the pointer does
//!
//! ```text
//! click a node              select it
//! click a path's line       select the path
//! click a point             select it (only the selected path's points are drawn)
//! drag a node or a point    move it across the horizontal plane at its height
//! Ctrl + drag               move it up and down instead
//! Add points armed + click  insert a point into the nearest leg of the path
//! New node armed + click    make a node on the ground there
//! Connect armed + click     make a path from the selected node to the clicked
//!                           one, and the path back when that is ticked
//! Delete                    remove the selected point
//! Escape                    disarm, then drop the point, the path, the node
//! ```
//!
//! ## Drawing a path point by point
//!
//! With [`Flightpaths::draw_by_hand`] on, Connect does not seed the path.
//! It starts a [`Draft`] at the selected node instead, and the points are
//! placed one at a time:
//!
//! ```text
//! click the ground          place the next point above the click, at the
//!                           draft height
//! Ctrl + drag               place the next point above the press, at the
//!                           height the drag sets (the upright plane a selected
//!                           point's Ctrl + drag moves in)
//! Backspace                 remove the last point placed
//! click another node        make the path, and the path back when ticked
//! Escape                    discard the draft
//! ```
//!
//! The draft height starts at the clearance above the start node and is kept
//! from one point to the next, so a path drawn at one altitude needs no Ctrl.
//! Nothing is written to the tables until the far node is clicked, and the
//! finished path is one undo entry.
//!
//! A press does not move anything until the pointer has travelled
//! [`DRAG_PIXELS`], so a click that selects does not also nudge. A dragged
//! node carries the first point of every path leaving it and the last point of
//! every path arriving at it, when [`Flightpaths::carry_ends`] is on: the
//! shipped paths start and end within a few yards of their nodes, and a node
//! moved without its path ends leaves the flight starting somewhere else.
//!
//! ## Where an edit reaches the game
//!
//! The client reads all three tables from the project's copies on the next
//! playtest. The server reads `TaxiPath` and `TaxiPathNode` as files from
//! `DataDir\5875\dbc\`, which a publish writes, and the nodes from its own
//! `taxi_nodes` table, which a save's server half writes (`crate::server::rows`).
//! vmangos reads all three at startup only.

use super::Tool;
use crate::marks::{Look, Marks};
use crate::session::EditSession;
use vale_client::render::axes;
use vale_client::render::focus::WorldFocus;
use vale_client::world::camera::WorldCamera;
use vale_edit::dbc::taxi::{self, Edits, Finding, Node, Path, Point, PointSpec};
use bevy::prelude::*;
use std::collections::HashMap;

/// One path, with its points in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub path: Path,
    pub points: Vec<Point>,
    /// Yards, point to point, not counting a crossing between maps.
    pub length: f32,
    /// Whether either end is a node with no mount on either side. Those are
    /// the ends of the boat and zeppelin routes, which share `TaxiPath.dbc`
    /// with the flights; `vale taxi` separates them by the same rule.
    pub transport: bool,
}

/// What the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Node(u32),
    Point(u32),
    /// A path's line, between two of its points.
    Path(u32),
}

/// What a click on empty ground does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Armed {
    /// Nothing.
    #[default]
    Nothing,
    /// Insert a point into the selected path.
    AddPoints,
    /// Make a node on the ground.
    NewNode,
    /// Make a path from this node to the next node clicked.
    Connect { from: u32 },
    /// Draw a path from this node point by point; see [`Draft`].
    Draw { from: u32 },
}

/// A path being drawn point by point from a node, before it is a row.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub from: u32,
    /// The points placed so far, in order, after the start node, in the
    /// world's axes.
    pub points: Vec<Vec3>,
    /// The height, in world yards, the next point is placed at.
    pub height: f32,
    /// A Ctrl + drag in progress.
    pub lift: Option<Lift>,
}

/// A Ctrl + drag that sets the next point's height. The point stands above
/// the place the press landed, and is placed when the button comes up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lift {
    /// Where the point stands across the ground, in the world's axes.
    pub at: Vec2,
    /// The offset from the pointer's hit on the upright plane to the point's
    /// height, measured on the first frame of the drag so the point does not
    /// jump.
    pub grab: Option<f32>,
}

impl Draft {
    /// Where the next point would go for a pointer over `surface`: above the
    /// lift's place while one is held, else above the pointer.
    pub fn next(&self, surface: Option<Vec3>) -> Option<Vec3> {
        let across = match self.lift {
            Some(lift) => lift.at,
            None => surface?.truncate(),
        };
        Some(across.extend(self.height))
    }
}

/// A held left button on a node or a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    pub handle: Handle,
    /// Where the pointer was when the button went down, in logical pixels.
    pub pressed_at: Vec2,
    /// Whether the pointer has travelled far enough for this to be a drag.
    pub moving: bool,
    /// The offset from the pointer's hit to the handle, measured on the first
    /// moving frame, and whether it was measured for the vertical plane. A
    /// change of plane measures it again, so pressing or releasing Ctrl in the
    /// middle of a drag does not make the handle jump.
    pub grab: Option<(Vec3, bool)>,
}

/// The flight path tool's state.
#[derive(Resource, Debug)]
pub struct Flightpaths {
    /// The nodes on the open map.
    pub nodes: Vec<Node>,
    /// Every node's name by id, on any map, for a path whose far end is not
    /// on this one.
    pub names: HashMap<u32, String>,
    /// The paths with a point or an end on the open map.
    pub routes: Vec<Route>,
    /// What `vale_edit::dbc::taxi::check` finds wrong with the three
    /// tables. Empty for the shipped ones.
    pub findings: Vec<Finding>,
    /// The session's table revision and the map the lists were built for.
    built: Option<(u64, u32)>,
    pub map: u32,
    pub node: Option<u32>,
    pub path: Option<u32>,
    pub point: Option<u32>,
    pub hovered: Option<Handle>,
    pub drag: Option<Drag>,
    pub armed: Armed,
    /// Make the path back as well when connecting two nodes. On by default:
    /// 270 of the 275 shipped flights have one.
    pub with_return: bool,
    /// Connect draws the path point by point instead of seeding it. Off by
    /// default.
    pub draw_by_hand: bool,
    /// The path being drawn, while [`Armed::Draw`] is armed.
    pub draft: Option<Draft>,
    /// Move the ends of a node's paths with the node. On by default.
    pub carry_ends: bool,
    /// Yards above the ground a new point is put at.
    pub clearance: f32,
    /// Draw the boat and zeppelin routes as well as the flights.
    pub transports: bool,
    /// A fly-to asked for by the panel, consumed by [`fly`].
    pub fly_to: Option<Vec3>,
    /// The camera distance that `fly_to` asks for, when it asks for one: a
    /// path is framed whole, a node or a point is not.
    pub fly_distance: Option<f32>,
    /// Whether the command line's flight path flags have run, or none were
    /// given. `crate::server::release::migration_on_the_command_line` waits on
    /// it.
    pub scripted_done: bool,
    /// Whether the archive chain holds the flight map picture of a map, by
    /// map, read once per map and again after [`draw_flight_map`].
    pub picture_known: Option<(u32, bool)>,
}

impl Default for Flightpaths {
    fn default() -> Flightpaths {
        Flightpaths {
            nodes: Vec::new(),
            names: HashMap::new(),
            routes: Vec::new(),
            findings: Vec::new(),
            built: None,
            map: 0,
            node: None,
            path: None,
            point: None,
            hovered: None,
            drag: None,
            armed: Armed::Nothing,
            with_return: true,
            draw_by_hand: false,
            draft: None,
            carry_ends: true,
            clearance: DEFAULT_CLEARANCE,
            transports: false,
            fly_to: None,
            fly_distance: None,
            scripted_done: false,
            picture_known: None,
        }
    }
}

impl Flightpaths {
    pub fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn route(&self, id: u32) -> Option<&Route> {
        self.routes.iter().find(|route| route.path.id == id)
    }

    /// The point with this id, and the route it is on.
    pub fn point(&self, id: u32) -> Option<(&Route, &Point)> {
        self.routes.iter().find_map(|route| {
            route
                .points
                .iter()
                .find(|point| point.id == id)
                .map(|point| (route, point))
        })
    }

    /// The paths leaving a node, then the paths arriving at it.
    pub fn routes_of(&self, node: u32) -> (Vec<&Route>, Vec<&Route>) {
        let leaving = self.routes.iter().filter(|r| r.path.from == node).collect();
        let arriving = self.routes.iter().filter(|r| r.path.to == node).collect();
        (leaving, arriving)
    }

    /// A node's name, or its id when the table has no node by that id.
    pub fn name(&self, id: u32) -> String {
        match self.names.get(&id) {
            Some(name) if !name.is_empty() => name.clone(),
            _ => format!("node {id}"),
        }
    }

    pub fn select_node(&mut self, id: u32) {
        self.node = Some(id);
        // Keep the path when it starts or ends here, so clicking along a
        // path's ends does not drop it.
        if let Some(path) = self.path.and_then(|path| self.route(path)) {
            if path.path.from != id && path.path.to != id {
                self.path = None;
                self.point = None;
            }
        }
    }

    pub fn select_path(&mut self, id: u32) {
        if self.path != Some(id) {
            self.point = None;
        }
        self.path = Some(id);
    }

    pub fn select_point(&mut self, id: u32) {
        if let Some((route, _)) = self.point(id) {
            self.path = Some(route.path.id);
        }
        self.point = Some(id);
    }

    /// Ask the camera to look at a place, in the world's axes.
    pub fn fly_to(&mut self, at: [f32; 3]) {
        self.fly_to = Some(Vec3::from(at));
    }

    /// Ask the camera to frame a whole path: the middle of the box its points
    /// on this map span, from a distance that fits the box's width.
    pub fn fly_to_path(&mut self, id: u32) {
        let Some(route) = self.route(id) else { return };
        let here: Vec<Vec3> = route
            .points
            .iter()
            .filter(|point| point.map == self.map)
            .map(|point| Vec3::from(point.at))
            .collect();
        let Some(first) = here.first().copied() else {
            return;
        };
        let (low, high) = here
            .iter()
            .fold((first, first), |(low, high), p| (low.min(*p), high.max(*p)));
        self.fly_to = Some((low + high) * 0.5);
        self.fly_distance = Some(framing_distance((high - low).truncate().length()));
    }

    /// Rebuild the lists on the next frame, after an edit this frame.
    pub fn stale(&mut self) {
        self.built = None;
    }

    /// Arm Connect from a node: [`Armed::Draw`] with an empty draft when
    /// [`Self::draw_by_hand`] is on, else [`Armed::Connect`]. The path and
    /// point selections are dropped while drawing, so their handles do not
    /// take the clicks that place points.
    pub fn arm_connect(&mut self, from: u32) {
        if !self.draw_by_hand {
            self.armed = Armed::Connect { from };
            return;
        }
        let Some(node) = self.node(from) else { return };
        self.draft = Some(Draft {
            from,
            points: Vec::new(),
            height: node.at[2] + self.clearance,
            lift: None,
        });
        self.armed = Armed::Draw { from };
        self.path = None;
        self.point = None;
    }

    /// Disarm, discarding a draft.
    pub fn disarm(&mut self) {
        self.armed = Armed::Nothing;
        self.draft = None;
    }
}

/// Yards above the ground a new point is put at, by default. The shipped
/// flights cruise well above this; it is a floor for a point placed by hand.
pub const DEFAULT_CLEARANCE: f32 = 40.0;

/// How wide a handle is to the pointer, in logical pixels. The light tool
/// uses the same width for the same reason: a handle is a point, so the
/// pointer picks it within a disc of this width around it.
pub(super) const HANDLE_PIXELS: f32 = 12.0;

/// How far the pointer travels before a press on a handle becomes a drag.
pub(super) const DRAG_PIXELS: f32 = 4.0;

/// How far from the camera a node marker is drawn and picked, in yards. A
/// little past the block the editor streams.
const MARKER_RANGE: f32 = 2600.0;

/// How far from the camera a path's leg is drawn and picked, in yards,
/// measured to the leg's middle. The selected path and the selected node's
/// paths are drawn whole.
const ROUTE_RANGE: f32 = 3500.0;

/// Yards between the points a new path is seeded with.
const SEED_SPACING: f32 = 120.0;

/// Mount creature ids a new node is given: the Horde wind rider in field 14
/// and the Alliance gryphon in field 15, which is how vmangos reads the two
/// columns. Node 79, Marshal's Refuge, carries the same pair.
pub const NEW_NODE_MOUNTS: [u32; 2] = [2224, 541];

pub struct FlightpathToolPlugin;

impl Plugin for FlightpathToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Flightpaths>()
            .add_systems(
                Update,
                (collect, scripted, aim, press, drag, keys)
                    .chain()
                    .after(crate::pick::aim),
            )
            .add_systems(Update, (draw, draw_draft, fly).after(keys));
    }
}

fn active(tool: &Tool, state: &crate::playtest::Playtest) -> bool {
    *tool == Tool::Flightpaths && state.editing()
}

/// Read the three tables into the lists when the tables or the map changed.
/// The three are opened by `tables::open_tables`, because the tool's table is
/// `TaxiNodes` and the other two are its chain.
fn collect(
    mut flights: ResMut<Flightpaths>,
    session: Option<Res<EditSession>>,
    focus: Res<WorldFocus>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !active(&tool, &state) {
        return;
    }
    let Some(session) = session else { return };
    let key = (session.table_revision, focus.map_id);
    if flights.built == Some(key) {
        return;
    }
    let (Some(nodes), Some(paths), Some(points)) = (
        session.table(taxi::NODES),
        session.table(taxi::PATHS),
        session.table(taxi::POINTS),
    ) else {
        return;
    };
    flights.built = Some(key);
    rebuild(&mut flights, nodes, paths, points, focus.map_id);
}

/// The lists, from the three tables, for one map.
fn rebuild(
    flights: &mut Flightpaths,
    nodes: &vale_edit::dbc::DbcFile,
    paths: &vale_edit::dbc::DbcFile,
    points: &vale_edit::dbc::DbcFile,
    map: u32,
) {
    let every = taxi::nodes(nodes);
    let mounts: HashMap<u32, [u32; 2]> = every.iter().map(|n| (n.id, n.mounts)).collect();
    let node_map: HashMap<u32, u32> = every.iter().map(|n| (n.id, n.map)).collect();
    flights.names = every.iter().map(|n| (n.id, n.name.clone())).collect();
    flights.nodes = every.into_iter().filter(|node| node.map == map).collect();
    let mut by_path = taxi::points_by_path(points);
    flights.routes = taxi::paths(paths)
        .into_iter()
        .filter_map(|path| {
            let points = by_path.remove(&path.id).unwrap_or_default();
            let here = points.iter().any(|point| point.map == map)
                || [path.from, path.to]
                    .iter()
                    .any(|end| node_map.get(end) == Some(&map));
            if !here {
                return None;
            }
            let transport = [path.from, path.to]
                .iter()
                .any(|end| mounts.get(end).is_some_and(|m| *m == [0, 0]));
            Some(Route {
                length: taxi::length(&points),
                path,
                points,
                transport,
            })
        })
        .collect();
    flights.findings = taxi::check(nodes, paths, points);
    flights.map = map;
    // A selection whose row went away with an undo or a removal is dropped
    // rather than left naming nothing.
    if flights.node.is_some_and(|id| flights.names.get(&id).is_none()) {
        flights.node = None;
    }
    if flights.path.is_some_and(|id| flights.route(id).is_none()) {
        flights.path = None;
    }
    if flights.point.is_some_and(|id| flights.point(id).is_none()) {
        flights.point = None;
    }
}

/// The pointer's ray and the numbers that turn a distance off it into pixels.
pub(super) struct Aim {
    pub(super) origin: Vec3,
    pub(super) direction: Vec3,
    /// The projection's `[1][1]`, `1 / tan(fov / 2)`.
    focal: f32,
    /// The window's height in logical pixels.
    height: f32,
}

impl Aim {
    /// How many pixels a distance of `off` yards from the ray is, at `along`
    /// yards down it.
    pub(super) fn pixels(&self, off: f32, along: f32) -> f32 {
        off / along * self.focal * self.height * 0.5
    }

    /// How far a point is from the ray in pixels, or `None` behind the eye.
    pub(super) fn to_point(&self, at: Vec3) -> Option<f32> {
        let to = at - self.origin;
        let along = to.dot(self.direction);
        if along <= 0.0 {
            return None;
        }
        Some(self.pixels((to - self.direction * along).length(), along))
    }

    /// How far a segment is from the ray in pixels, at its nearest.
    fn to_segment(&self, a: Vec3, b: Vec3) -> Option<f32> {
        let (along, off) = ray_to_segment(self.origin, self.direction, a, b)?;
        Some(self.pixels(off, along))
    }
}

/// The closest approach between a ray and a segment: how far down the ray it
/// is, and how far apart the two are there. `None` when it is behind the eye.
fn ray_to_segment(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3) -> Option<(f32, f32)> {
    let u = b - a;
    let w = origin - a;
    let (uu, ud, uw, dw) = (u.dot(u), u.dot(direction), u.dot(w), direction.dot(w));
    // With a unit direction, the two lines' parameters at their closest.
    let denominator = uu - ud * ud;
    let s = match denominator.abs() > 1e-6 {
        true => ((uw - ud * dw) / denominator).clamp(0.0, 1.0),
        false => 0.0,
    };
    let on_segment = a + u * s;
    let t = (on_segment - origin).dot(direction);
    if t <= 0.0 {
        return None;
    }
    Some((t, (origin + direction * t - on_segment).length()))
}

pub(super) fn aim_of(
    windows: &Query<&Window>,
    camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<Aim> {
    let (origin, direction) = crate::pick::ray(windows, camera)?;
    let window = windows.single().ok()?;
    let (camera, _) = camera.single().ok()?;
    let focal = camera.clip_from_view().to_cols_array()[5];
    (focal.abs() > f32::EPSILON).then_some(Aim {
        origin,
        direction,
        focal,
        height: window.height().max(1.0),
    })
}

/// The eye, in the world's axes.
pub(super) fn eye(camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>) -> Option<Vec3> {
    let (_, at) = camera.single().ok()?;
    Some(Vec3::from(axes::to_wow(at.translation())))
}

/// Whether a leg is drawn and picked: near the eye, or on a path that is
/// selected or touches the selected node.
fn leg_shown(flights: &Flightpaths, route: &Route, a: Vec3, b: Vec3, eye: Vec3) -> bool {
    let chosen = flights.path == Some(route.path.id)
        || flights
            .node
            .is_some_and(|node| route.path.from == node || route.path.to == node);
    chosen || ((a + b) * 0.5 - eye).length() <= ROUTE_RANGE
}

/// Whether a route is drawn at all.
fn route_shown(flights: &Flightpaths, route: &Route) -> bool {
    flights.transports || !route.transport || flights.path == Some(route.path.id)
}

/// Which handle the pointer is over: the selected path's points first, then
/// the nodes, then any drawn path's line.
#[allow(clippy::too_many_arguments)]
fn aim(
    mut flights: ResMut<Flightpaths>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !active(&tool, &state) {
        flights.hovered = None;
        return;
    }
    if flights.drag.is_some_and(|drag| drag.moving) {
        return;
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        flights.hovered = None;
        return;
    }
    let (Some(aim), Some(eye)) = (aim_of(&windows, &camera), eye(&camera)) else {
        flights.hovered = None;
        return;
    };
    let nearest = |candidates: &mut dyn Iterator<Item = (f32, Handle)>| {
        candidates
            .filter(|(pixels, _)| *pixels <= HANDLE_PIXELS)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, handle)| handle)
    };
    let map = flights.map;
    let points = flights.path.and_then(|path| flights.route(path)).map(|route| {
        route
            .points
            .iter()
            .filter(|point| point.map == map)
            .filter_map(|point| Some((aim.to_point(Vec3::from(point.at))?, Handle::Point(point.id))))
            .collect::<Vec<_>>()
    });
    if let Some(handle) = points.and_then(|found| nearest(&mut found.into_iter())) {
        flights.hovered = Some(handle);
        return;
    }
    let nodes: Vec<(f32, Handle)> = flights
        .nodes
        .iter()
        .filter(|node| {
            flights.node == Some(node.id) || (Vec3::from(node.at) - eye).length() <= MARKER_RANGE
        })
        .filter_map(|node| Some((aim.to_point(Vec3::from(node.at))?, Handle::Node(node.id))))
        .collect();
    if let Some(handle) = nearest(&mut nodes.into_iter()) {
        flights.hovered = Some(handle);
        return;
    }
    let mut legs: Vec<(f32, Handle)> = Vec::new();
    for route in flights.routes.iter().filter(|route| route_shown(&flights, route)) {
        for pair in route.points.windows(2) {
            if pair[0].map != map || pair[1].map != map {
                continue;
            }
            let (a, b) = (Vec3::from(pair[0].at), Vec3::from(pair[1].at));
            if !leg_shown(&flights, route, a, b, eye) {
                continue;
            }
            if let Some(pixels) = aim.to_segment(a, b) {
                legs.push((pixels, Handle::Path(route.path.id)));
            }
        }
    }
    flights.hovered = nearest(&mut legs.into_iter());
}

/// A press: select, start a drag, or do what is armed.
#[allow(clippy::too_many_arguments)]
fn press(
    mut flights: ResMut<Flightpaths>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
    cursor: Res<crate::pick::Cursor>,
    mut session: Option<ResMut<EditSession>>,
) {
    if !active(&tool, &state) || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    let Some(session) = session.as_mut() else { return };
    if let Armed::Draw { from } = flights.armed {
        let lifting = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
        let line = draw_press(session, &mut flights, from, cursor.surface, lifting);
        if let Some(line) = line {
            session.status = line;
        }
        return;
    }
    let pointer = windows
        .single()
        .ok()
        .and_then(Window::cursor_position)
        .unwrap_or_default();
    let begin = |handle: Handle| Drag {
        handle,
        pressed_at: pointer,
        moving: false,
        grab: None,
    };
    match (flights.armed, flights.hovered) {
        (Armed::Connect { from }, Some(Handle::Node(to))) => {
            flights.armed = Armed::Nothing;
            let with_return = flights.with_return;
            let clearance = flights.clearance;
            let line = connect(session, &mut flights, from, to, with_return, clearance);
            session.status = line;
        }
        (Armed::AddPoints, None | Some(Handle::Path(_))) => {
            if let Some(at) = cursor.surface {
                let line = add_point(session, &mut flights, at);
                session.status = line;
            }
        }
        (Armed::NewNode, None | Some(Handle::Path(_))) => {
            if let Some(at) = cursor.surface {
                flights.armed = Armed::Nothing;
                let map = flights.map;
                let line = new_node(session, &mut flights, map, at);
                session.status = line;
            }
        }
        (_, Some(Handle::Node(id))) => {
            flights.select_node(id);
            flights.drag = Some(begin(Handle::Node(id)));
        }
        (_, Some(Handle::Point(id))) => {
            flights.select_point(id);
            flights.drag = Some(begin(Handle::Point(id)));
        }
        (_, Some(Handle::Path(id))) => flights.select_path(id),
        _ => {}
    }
}

/// Move the held node or point.
#[allow(clippy::too_many_arguments)]
fn drag(
    mut flights: ResMut<Flightpaths>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    time: Res<Time>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    mut session: Option<ResMut<EditSession>>,
) {
    if !active(&tool, &state) {
        flights.drag = None;
        return;
    }
    if flights.draft.as_ref().is_some_and(|draft| draft.lift.is_some()) {
        let session = session.as_deref_mut();
        lift(&mut flights, &buttons, &windows, &camera, session);
        return;
    }
    if !buttons.pressed(MouseButton::Left) {
        flights.drag = None;
        return;
    }
    let Some(mut held) = flights.drag else { return };
    let Some(pointer) = windows.single().ok().and_then(Window::cursor_position) else {
        return;
    };
    if !held.moving {
        if pointer.distance(held.pressed_at) < DRAG_PIXELS {
            return;
        }
        held.moving = true;
    }
    let Some(session) = session.as_mut() else { return };
    let Some(from) = handle_position(&flights, held.handle) else {
        flights.drag = None;
        return;
    };
    let vertical = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let Some((origin, direction)) = crate::pick::ray(&windows, &camera) else {
        return;
    };
    let hit = match vertical {
        false => crate::pick::meets_plane(origin, direction, from.z),
        true => meets_upright_plane(origin, direction, from),
    };
    let Some(hit) = hit else { return };
    let grab = match held.grab {
        Some((grab, was_vertical)) if was_vertical == vertical => grab,
        _ => {
            // First moving frame, or the plane changed: measure the offset and
            // write nothing, so the handle does not jump.
            held.grab = Some((from - hit, vertical));
            flights.drag = Some(held);
            return;
        }
    };
    let to = match vertical {
        false => Vec3::new(hit.x + grab.x, hit.y + grab.y, from.z),
        true => Vec3::new(from.x, from.y, hit.z + grab.z),
    };
    flights.drag = Some(held);
    let now = time.elapsed_secs_f64();
    match held.handle {
        Handle::Node(id) => move_node(session, &flights, id, to, now),
        Handle::Point(id) => move_point(session, &flights, id, to, now),
        Handle::Path(_) => {}
    }
    flights.stale();
}

/// Where a handle is now, in the world's axes.
fn handle_position(flights: &Flightpaths, handle: Handle) -> Option<Vec3> {
    match handle {
        Handle::Node(id) => flights.node(id).map(|node| Vec3::from(node.at)),
        Handle::Point(id) => flights.point(id).map(|(_, point)| Vec3::from(point.at)),
        Handle::Path(_) => None,
    }
}

/// A press while a path is being drawn. A press on another node finishes the
/// path; any other press places the next point, or with `lifting` (Ctrl held)
/// starts a [`Lift`] that places it on release. Answers the status line, or
/// `None` when nothing happened.
fn draw_press(
    session: &mut EditSession,
    flights: &mut Flightpaths,
    from: u32,
    surface: Option<Vec3>,
    lifting: bool,
) -> Option<String> {
    if let Some(Handle::Node(to)) = flights.hovered {
        if to == from {
            return Some("click another node to end the path".to_string());
        }
        return Some(finish_draft(session, flights, to));
    }
    let surface = surface?;
    let draft = flights.draft.as_mut()?;
    if lifting {
        draft.lift = Some(Lift {
            at: surface.truncate(),
            grab: None,
        });
        return None;
    }
    let at = surface.truncate().extend(draft.height);
    draft.points.push(at);
    Some(placed_line(session, draft, at))
}

/// The status line for a point just placed: its number and its height above
/// the ground.
fn placed_line(session: &EditSession, draft: &Draft, at: Vec3) -> String {
    let n = draft.points.len();
    match super::doodads::ground_height(session, at.x, at.y) {
        Some(ground) => format!("point {n} placed at {:.0} yd, {:.0} yd above the ground", at.z, at.z - ground),
        None => format!("point {n} placed at {:.0} yd", at.z),
    }
}

/// Hold a [`Lift`]: while the button is down the draft height follows the
/// pointer on the upright plane through the point; when it comes up the point
/// is placed.
fn lift(
    flights: &mut Flightpaths,
    buttons: &ButtonInput<MouseButton>,
    windows: &Query<&Window>,
    camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    session: Option<&mut EditSession>,
) {
    let Some(draft) = flights.draft.as_mut() else { return };
    let Some(mut held) = draft.lift else { return };
    if !buttons.pressed(MouseButton::Left) {
        let at = held.at.extend(draft.height);
        draft.lift = None;
        draft.points.push(at);
        if let Some(session) = session {
            let line = placed_line(session, draft, at);
            session.status = line;
        }
        return;
    }
    let Some((origin, direction)) = crate::pick::ray(windows, camera) else {
        return;
    };
    let point = held.at.extend(draft.height);
    let Some(hit) = meets_upright_plane(origin, direction, point) else {
        return;
    };
    match held.grab {
        Some(grab) => draft.height = hit.z + grab,
        None => {
            held.grab = Some(point.z - hit.z);
            draft.lift = Some(held);
        }
    }
}

/// The points of a drawn path: the start node, the points placed, the far
/// node.
pub fn drawn_points(from: Vec3, placed: &[Vec3], to: Vec3) -> Vec<[f32; 3]> {
    std::iter::once(from)
        .chain(placed.iter().copied())
        .chain(std::iter::once(to))
        .map(|at| at.to_array())
        .collect()
}

/// Make the drafted path to node `to`, and the path back when asked, as one
/// undo entry, and disarm. Answers the status line.
fn finish_draft(session: &mut EditSession, flights: &mut Flightpaths, to: u32) -> String {
    let Some(draft) = flights.draft.clone() else {
        flights.disarm();
        return "nothing is being drawn".to_string();
    };
    let (Some(a), Some(b)) = (flights.node(draft.from).cloned(), flights.node(to).cloned()) else {
        return "both nodes must be on this map".to_string();
    };
    let points = drawn_points(Vec3::from(a.at), &draft.points, Vec3::from(b.at));
    let with_return = flights.with_return;
    flights.disarm();
    make_path(session, flights, &a, &b, &points, with_return, "Draw flight path")
}

/// Where the ray meets the upright plane through `at` that faces the camera
/// across the ground: the plane a vertical drag moves in.
pub(super) fn meets_upright_plane(origin: Vec3, direction: Vec3, at: Vec3) -> Option<Vec3> {
    let normal = Vec3::new(direction.x, direction.y, 0.0).normalize_or_zero();
    let facing = direction.dot(normal);
    if normal == Vec3::ZERO || facing.abs() < 1e-4 {
        return None;
    }
    let t = (at - origin).dot(normal) / facing;
    (t > 0.0).then(|| origin + direction * t)
}

/// Write a node's position, and the ends of its paths when they are carried,
/// as one gesture.
pub fn move_node(session: &mut EditSession, flights: &Flightpaths, id: u32, to: Vec3, now: f64) {
    use vale_assets::tables::taxi::{node_fields as nf, path_node_fields as wf};
    let Some(node) = flights.node(id) else { return };
    let subject = format!("TaxiNodes {id} position");
    super::tables::set_fields(
        session,
        taxi::NODES,
        node.record,
        &[
            (nf::X, to.x.to_bits()),
            (nf::Y, to.y.to_bits()),
            (nf::Z, to.z.to_bits()),
        ],
        "Move flight node",
        &subject,
        now,
    );
    if !flights.carry_ends {
        return;
    }
    let (leaving, arriving) = flights.routes_of(id);
    let ends = leaving
        .iter()
        .filter_map(|route| route.points.first())
        .chain(arriving.iter().filter_map(|route| route.points.last()));
    for point in ends {
        super::tables::set_fields(
            session,
            taxi::POINTS,
            point.record,
            &[
                (wf::X, to.x.to_bits()),
                (wf::Y, to.y.to_bits()),
                (wf::Z, to.z.to_bits()),
            ],
            "Move flight node",
            &subject,
            now,
        );
    }
}

/// Write one point's position as a gesture.
pub fn move_point(session: &mut EditSession, flights: &Flightpaths, id: u32, to: Vec3, now: f64) {
    use vale_assets::tables::taxi::path_node_fields as wf;
    let Some((_, point)) = flights.point(id) else { return };
    super::tables::set_fields(
        session,
        taxi::POINTS,
        point.record,
        &[
            (wf::X, to.x.to_bits()),
            (wf::Y, to.y.to_bits()),
            (wf::Z, to.z.to_bits()),
        ],
        "Move flight point",
        &format!("TaxiPathNode {id} position"),
        now,
    );
}

/// Put an operation's edits on the undo stack as one entry and mark the
/// tables it wrote as edited.
pub fn record(session: &mut EditSession, label: &str, done: Edits) {
    let mut touched: Vec<String> = Vec::new();
    session.history.begin(label);
    for (table, row) in done.rows {
        if !touched.contains(&table) {
            touched.push(table.clone());
        }
        session.history.record_row(&table, row);
    }
    for (table, cell) in done.cells {
        if !touched.contains(&table) {
            touched.push(table.clone());
        }
        session.history.record_cell(&table, cell);
    }
    session.history.end();
    for table in touched {
        session.table_edited(&table);
    }
}

/// Insert a point into the selected path at the leg nearest to `at`, at the
/// clearance above `at` or at the leg's own height, whichever is higher.
fn add_point(session: &mut EditSession, flights: &mut Flightpaths, at: Vec3) -> String {
    let Some(route) = flights.path.and_then(|id| flights.route(id)).cloned() else {
        return "select a path to add points to it".to_string();
    };
    let (index, height) = insertion(&route.points, at);
    let z = (at.z + flights.clearance).max(height);
    let spec = PointSpec::flying(flights.map, [at.x, at.y, z]);
    match taxi::insert_point(&mut session.tables, route.path.id, index, spec) {
        Ok(done) => {
            let made = done.made;
            record(session, "Add flight point", done);
            flights.stale();
            flights.point = made;
            format!("point {} added to path {} at index {index}", made.unwrap_or(0), route.path.id)
        }
        Err(e) => e.to_string(),
    }
}

/// Where a point clicked at `at` goes in a path: the index it takes, and the
/// height of the leg at that place. It goes into the leg whose line, across
/// the ground, passes nearest the click, and never before the first point or
/// after the last, which stand at the nodes.
pub fn insertion(points: &[Point], at: Vec3) -> (u32, f32) {
    let mut best: Option<(f32, u32, f32)> = None;
    for pair in points.windows(2) {
        let (a, b) = (Vec3::from(pair[0].at), Vec3::from(pair[1].at));
        let u = (b - a).truncate();
        let s = match u.length_squared() > 1e-6 {
            true => ((at.truncate() - a.truncate()).dot(u) / u.length_squared()).clamp(0.0, 1.0),
            false => 0.0,
        };
        let off = (a.truncate() + u * s - at.truncate()).length();
        let height = a.z + (b.z - a.z) * s;
        if best.is_none_or(|(had, _, _)| off < had) {
            best = Some((off, pair[1].index, height));
        }
    }
    match best {
        Some((_, index, height)) => (index, height),
        None => (points.len() as u32, at.z),
    }
}

/// Make a node on the ground at `at`.
fn new_node(session: &mut EditSession, flights: &mut Flightpaths, map: u32, at: Vec3) -> String {
    match taxi::new_node(&mut session.tables, map, at.to_array(), "New node", NEW_NODE_MOUNTS) {
        Ok(done) => {
            let made = done.made;
            record(session, "Add flight node", done);
            flights.stale();
            flights.node = made;
            flights.path = None;
            flights.point = None;
            format!("node {} made", made.unwrap_or(0))
        }
        Err(e) => e.to_string(),
    }
}

/// The points a new path between two places is seeded with: the two ends,
/// and a point every [`SEED_SPACING`] yards between them at `clearance`
/// yards above the ground where the ground is known, and never below the
/// straight line between the ends.
pub fn seed(
    from: Vec3,
    to: Vec3,
    clearance: f32,
    ground: impl Fn(f32, f32) -> Option<f32>,
) -> Vec<[f32; 3]> {
    let across = (to - from).truncate().length();
    let legs = ((across / SEED_SPACING).ceil() as usize).max(1);
    (0..=legs)
        .map(|n| {
            let t = n as f32 / legs as f32;
            let p = from.lerp(to, t);
            if n == 0 || n == legs {
                return p.to_array();
            }
            let floor = ground(p.x, p.y).map_or(p.z + clearance, |z| z + clearance);
            [p.x, p.y, floor.max(p.z)]
        })
        .collect()
}

/// A cost for a new path: the median copper per yard of the paths on this map
/// that cost anything, times the new path's length, rounded to ten copper.
/// Zero when no path on the map has a cost.
pub fn suggested_cost(routes: &[Route], length: f32) -> u32 {
    let mut rates: Vec<f32> = routes
        .iter()
        .filter(|route| route.path.cost > 0 && route.length > 0.0 && !route.transport)
        .map(|route| route.path.cost as f32 / route.length)
        .collect();
    if rates.is_empty() {
        return 0;
    }
    rates.sort_by(|a, b| a.total_cmp(b));
    let rate = rates[rates.len() / 2];
    ((rate * length / 10.0).round() * 10.0) as u32
}

/// Make a path from one node to another, and the path back when asked, as one
/// undo entry. Answers the status line.
pub fn connect(
    session: &mut EditSession,
    flights: &mut Flightpaths,
    from: u32,
    to: u32,
    with_return: bool,
    clearance: f32,
) -> String {
    let (Some(a), Some(b)) = (flights.node(from).cloned(), flights.node(to).cloned()) else {
        return "both nodes must be on this map".to_string();
    };
    let points = seed(Vec3::from(a.at), Vec3::from(b.at), clearance, |x, y| {
        super::doodads::ground_height(session, x, y)
    });
    make_path(session, flights, &a, &b, &points, with_return, "Connect flight nodes")
}

/// Make a path from node `a` to node `b` through `points`, and the path back
/// when asked, as one undo entry under `label`. Answers the status line.
fn make_path(
    session: &mut EditSession,
    flights: &mut Flightpaths,
    a: &Node,
    b: &Node,
    points: &[[f32; 3]],
    with_return: bool,
    label: &str,
) -> String {
    let (from, to) = (a.id, b.id);
    let specs: Vec<PointSpec> = points
        .iter()
        .map(|&at| PointSpec::flying(a.map, at))
        .collect();
    let length = points
        .windows(2)
        .map(|pair| taxi::distance(pair[0], pair[1]))
        .sum::<f32>();
    let cost = suggested_cost(&flights.routes, length);
    let mut done = match taxi::new_path(&mut session.tables, from, to, cost, &specs) {
        Ok(done) => done,
        Err(e) => return e.to_string(),
    };
    let there = done.made;
    let mut back = None;
    if let Some(path) = there.filter(|_| with_return) {
        match taxi::reverse_path(&mut session.tables, path) {
            Ok(more) => {
                back = more.made;
                done.rows.extend(more.rows);
                done.cells.extend(more.cells);
            }
            Err(e) => {
                record(session, label, done);
                flights.stale();
                flights.path = there;
                return format!("path {} made; creating the return path failed: {e}", there.unwrap_or(0));
            }
        }
    }
    record(session, label, done);
    flights.stale();
    flights.path = there;
    flights.point = None;
    match back {
        Some(back) => format!(
            "paths {} and {back} made between {} and {}, {} points, {cost} copper",
            there.unwrap_or(0),
            a.name,
            b.name,
            points.len()
        ),
        None => format!(
            "path {} made from {} to {}, {} points, {cost} copper",
            there.unwrap_or(0),
            a.name,
            b.name,
            points.len()
        ),
    }
}

/// Remove the selected point. Answers the status line.
pub fn remove_selected_point(session: &mut EditSession, flights: &mut Flightpaths) -> String {
    let Some(id) = flights.point else {
        return "no point is selected".to_string();
    };
    match taxi::remove_point(&mut session.tables, id) {
        Ok(done) => {
            record(session, "Remove flight point", done);
            flights.point = None;
            flights.stale();
            format!("point {id} removed")
        }
        Err(e) => e.to_string(),
    }
}

/// Escape and Delete.
fn keys(
    mut flights: ResMut<Flightpaths>,
    keys: Res<ButtonInput<KeyCode>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut session: Option<ResMut<EditSession>>,
) {
    if !active(&tool, &state) || wants.wants_keyboard_input() {
        return;
    }
    if let Some(draft) = flights.draft.as_mut() {
        if keys.just_pressed(KeyCode::Backspace) && draft.lift.is_none() {
            let line = match draft.points.pop() {
                Some(_) => format!("{} point(s) placed", draft.points.len()),
                None => "no point to remove".to_string(),
            };
            if let Some(session) = session.as_mut() {
                session.status = line;
            }
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        if flights.armed != Armed::Nothing {
            flights.disarm();
        } else if flights.point.is_some() {
            flights.point = None;
        } else if flights.path.is_some() {
            flights.path = None;
        } else {
            flights.node = None;
        }
    }
    if keys.just_pressed(KeyCode::Delete) && flights.point.is_some() {
        if let Some(session) = session.as_mut() {
            let line = remove_selected_point(session, &mut flights);
            session.status = line;
        }
    }
}

/// How wide a marker is drawn, in yards, at a distance: about the handle's
/// width on screen, clamped.
pub(super) fn marker_radius(at: Vec3, eye: Vec3) -> f32 {
    ((at - eye).length() * 0.02).clamp(0.6, 22.0)
}

/// A node's colour: by which sides vmangos offers it to, from its two mount
/// columns (field 14 Horde, field 15 Alliance).
pub fn side_colour(mounts: [u32; 2]) -> Color {
    match (mounts[0] != 0, mounts[1] != 0) {
        (true, true) => Color::srgb(0.95, 0.8, 0.3),
        (true, false) => Color::srgb(0.9, 0.3, 0.25),
        (false, true) => Color::srgb(0.3, 0.55, 0.95),
        (false, false) => Color::srgb(0.6, 0.6, 0.6),
    }
}

/// Draw the nodes, the paths and the selected path's points.
///
/// A route that is selected, hovered or leaves the selected node, and every
/// node and point, is [`Look::Ghosted`]: a mountain between it and the camera
/// hides it to a faint line rather than not at all. The other routes are
/// [`Look::Solid`].
#[allow(clippy::too_many_arguments)]
fn draw(
    mut marks: ResMut<Marks>,
    flights: Res<Flightpaths>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    cursor: Res<crate::pick::Cursor>,
    session: Option<Res<EditSession>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !active(&tool, &state) {
        return;
    }
    let Some(eye) = eye(&camera) else { return };
    let bevy = |p: Vec3| axes::to_bevy(p.to_array());
    let map = flights.map;
    let lit = Color::WHITE;
    let hovered = flights.hovered;

    for route in flights.routes.iter().filter(|route| route_shown(&flights, route)) {
        let id = route.path.id;
        let selected = flights.path == Some(id);
        let of_node = flights
            .node
            .is_some_and(|node| route.path.from == node || route.path.to == node);
        let under = hovered == Some(Handle::Path(id));
        let colour = match (selected, under, of_node) {
            (true, _, _) => Color::srgb(1.0, 0.6, 0.15),
            (false, true, _) => lit,
            (false, false, true) => Color::srgba(0.35, 0.85, 0.95, 0.95),
            _ if route.transport => Color::srgba(0.6, 0.6, 0.7, 0.5),
            _ => Color::srgba(0.35, 0.75, 0.85, 0.55),
        };
        for (n, pair) in route.points.windows(2).enumerate() {
            if pair[0].map != map || pair[1].map != map {
                continue;
            }
            let (a, b) = (Vec3::from(pair[0].at), Vec3::from(pair[1].at));
            if !leg_shown(&flights, route, a, b, eye) {
                continue;
            }
            let look = match selected || under || of_node {
                true => Look::Ghosted,
                false => Look::Solid,
            };
            marks.line(bevy(a), bevy(b), colour, look);
            // Which way the selected path flies: a chevron on every third leg.
            if selected && n % 3 == 1 {
                chevron(&mut marks, a, b, colour);
            }
        }
        if !selected {
            continue;
        }
        for point in route.points.iter().filter(|point| point.map == map) {
            let at = Vec3::from(point.at);
            let colour = match (flights.point == Some(point.id), hovered == Some(Handle::Point(point.id))) {
                (true, _) => lit,
                (false, true) => Color::srgb(1.0, 0.9, 0.6),
                _ => Color::srgb(1.0, 0.6, 0.15),
            };
            marks.sphere(bevy(at), marker_radius(at, eye) * 0.6, colour, Look::Ghosted);
        }
        // A line from the selected point down to the ground, which shows its
        // height.
        if let (Some((_, point)), Some(session)) =
            (flights.point.and_then(|id| flights.point(id)), session.as_ref())
        {
            let at = Vec3::from(point.at);
            if let Some(ground) = super::doodads::ground_height(session, at.x, at.y) {
                marks.line(bevy(at), bevy(Vec3::new(at.x, at.y, ground)), lit.with_alpha(0.6), Look::Ghosted);
            }
        }
    }

    for node in &flights.nodes {
        let at = Vec3::from(node.at);
        let selected = flights.node == Some(node.id);
        if !selected && (at - eye).length() > MARKER_RANGE {
            continue;
        }
        let source = flights.armed == (Armed::Connect { from: node.id });
        let colour = match (selected || source, hovered == Some(Handle::Node(node.id))) {
            (true, _) => lit,
            (false, true) => Color::srgb(1.0, 1.0, 0.8),
            _ => side_colour(node.mounts),
        };
        let radius = marker_radius(at, eye);
        marks.sphere(bevy(at), radius, colour, Look::Ghosted);
        marks.line(bevy(at), bevy(at + Vec3::Z * radius * 4.0), colour, Look::Ghosted);
        if selected {
            // A shell around the node, in its side's colour, which the white
            // of the selection would otherwise hide.
            marks.sphere(bevy(at), radius * 1.6, side_colour(node.mounts).with_alpha(0.35), Look::Ghosted);
        }
    }

    // What a click would do, drawn to the pointer.
    let Some(pointer) = cursor.surface else { return };
    match flights.armed {
        Armed::Connect { from } => {
            if let Some(node) = flights.node(from) {
                marks.line(bevy(Vec3::from(node.at)), bevy(pointer), Color::srgb(0.5, 1.0, 0.5), Look::Ghosted);
            }
        }
        Armed::AddPoints => {
            if let Some(route) = flights.path.and_then(|id| flights.route(id)) {
                let (index, height) = insertion(&route.points, pointer);
                let new = Vec3::new(pointer.x, pointer.y, (pointer.z + flights.clearance).max(height));
                let before = route.points.iter().find(|p| p.index + 1 == index);
                let after = route.points.iter().find(|p| p.index == index);
                for end in [before, after].into_iter().flatten() {
                    marks.line(bevy(Vec3::from(end.at)), bevy(new), Color::srgb(1.0, 0.85, 0.5), Look::Ghosted);
                }
                marks.line(bevy(new), bevy(pointer), lit.with_alpha(0.5), Look::Ghosted);
            }
        }
        Armed::NewNode => {
            marks.sphere(bevy(pointer), marker_radius(pointer, eye), side_colour(NEW_NODE_MOUNTS), Look::Ghosted);
        }
        Armed::Draw { .. } | Armed::Nothing => {}
    }
}

/// Draw the draft: its legs so far, the next point with its drop to the
/// ground, and the leg the next click makes. The next point is red when it is
/// below the ground under it.
fn draw_draft(
    mut marks: ResMut<Marks>,
    flights: Res<Flightpaths>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    cursor: Res<crate::pick::Cursor>,
    session: Option<Res<EditSession>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !active(&tool, &state) {
        return;
    }
    let (Some(draft), Some(eye)) = (flights.draft.as_ref(), eye(&camera)) else {
        return;
    };
    let Some(start) = flights.node(draft.from).map(|node| Vec3::from(node.at)) else {
        return;
    };
    let bevy = |p: Vec3| axes::to_bevy(p.to_array());
    let drawn = Color::srgb(0.5, 1.0, 0.5);
    let mut last = start;
    for &at in &draft.points {
        marks.line(bevy(last), bevy(at), drawn, Look::Ghosted);
        marks.sphere(bevy(at), marker_radius(at, eye) * 0.6, drawn, Look::Ghosted);
        last = at;
    }
    // A hovered far node is where the next click ends the path.
    if let Some(end) = match flights.hovered {
        Some(Handle::Node(id)) if id != draft.from => flights.node(id).map(|node| Vec3::from(node.at)),
        _ => None,
    } {
        marks.line(bevy(last), bevy(end), Color::WHITE, Look::Ghosted);
        return;
    }
    let Some(next) = draft.next(cursor.surface) else { return };
    let ground = session
        .as_ref()
        .and_then(|session| super::doodads::ground_height(session, next.x, next.y));
    let colour = match ground {
        Some(ground) if next.z < ground => Color::srgb(1.0, 0.3, 0.25),
        _ => Color::srgb(1.0, 0.85, 0.5),
    };
    marks.line(bevy(last), bevy(next), colour.with_alpha(0.7), Look::Ghosted);
    marks.sphere(bevy(next), marker_radius(next, eye) * 0.6, colour, Look::Ghosted);
    if let Some(ground) = ground {
        marks.line(bevy(next), bevy(next.truncate().extend(ground)), Color::WHITE.with_alpha(0.6), Look::Ghosted);
    }
}

/// A cone at the middle of a leg, pointing from `a` to `b`.
fn chevron(marks: &mut Marks, a: Vec3, b: Vec3, colour: Color) {
    let along = b - a;
    let length = along.length();
    if length < 1.0 {
        return;
    }
    let size = (length * 0.15).clamp(2.0, 12.0);
    let tip = (a + b) * 0.5;
    let base = tip - along / length * size;
    marks.cone(
        axes::to_bevy(base.to_array()),
        axes::to_bevy(tip.to_array()),
        size * 0.35,
        colour.with_alpha(1.0),
        Look::Ghosted,
    );
}

/// Take the camera to a place the panel asked for.
fn fly(mut flights: ResMut<Flightpaths>, mut camera: ResMut<crate::camera::EditorCamera>) {
    let Some(at) = flights.fly_to.take() else {
        return;
    };
    camera.target = at;
    camera.wants_the_ground = false;
    if let Some(distance) = flights.fly_distance.take() {
        camera.distance = distance;
    }
}

/// How far back the camera stands to see a box `across` yards wide: about its
/// width, within the range the editor streams.
pub fn framing_distance(across: f32) -> f32 {
    (across * 0.9).clamp(120.0, 1500.0)
}

/// The command line's flight path flags, once the tables are read and, for a
/// new node, the ground under it is open: `--taxi-node`, `--taxi-path`,
/// `--taxi-connect`, `--taxi-node-add` and `--flight-map`.
#[allow(clippy::too_many_arguments)]
fn scripted(
    mut flights: ResMut<Flightpaths>,
    args: Res<crate::Args>,
    assets: Res<vale_client::assets::GameAssets>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    mut done: Local<bool>,
) {
    let asked = args.taxi_node.is_some()
        || args.taxi_path.is_some()
        || args.taxi_connect.is_some()
        || args.taxi_node_add.is_some()
        || args.flight_map;
    if *done {
        return;
    }
    if !asked {
        *done = true;
        flights.scripted_done = true;
        return;
    }
    if !active(&tool, &state) || flights.built.is_none() {
        return;
    }
    let Some(session) = session.as_mut() else { return };
    if let Some((x, y)) = args.taxi_node_add {
        let Some(z) = super::doodads::ground_height(session, x, y) else {
            return;
        };
        let map = flights.map;
        let line = new_node(session, &mut flights, map, Vec3::new(x, y, z));
        info!("--taxi-node-add: {line}");
        session.status = line;
        rebuilt(session, &mut flights);
    }
    if args.flight_map {
        use vale_edit::flightmap;
        if !session.open_table(&assets, flightmap::TABLE) || !session.open_table(&assets, WORLD_MAP_AREA) {
            return;
        }
        let line = match flight_map(session, &flights) {
            Some(FlightMap::Shipped) => "this map is a continent of the world map; its flight map is shipped".to_string(),
            _ => make_flight_map(session, &assets, &mut flights),
        };
        info!("--flight-map: {line}");
        session.status = line;
    }
    if let Some((from, to)) = args.taxi_connect {
        let (with_return, clearance) = (flights.with_return, flights.clearance);
        let line = connect(session, &mut flights, from, to, with_return, clearance);
        info!("--taxi-connect: {line}");
        session.status = line;
        rebuilt(session, &mut flights);
        if let Some(path) = flights.path {
            flights.fly_to_path(path);
        }
    }
    if let Some(id) = args.taxi_node {
        match flights.node(id).cloned() {
            Some(node) => {
                flights.select_node(id);
                flights.fly_to(node.at);
                let (leaving, arriving) = flights.routes_of(id);
                info!(
                    "--taxi-node: {id} {}, {} path(s) leaving and {} arriving",
                    node.name,
                    leaving.len(),
                    arriving.len()
                );
            }
            None => warn!("--taxi-node {id}: no node by that id on this map"),
        }
    }
    if let Some(id) = args.taxi_path {
        match flights.route(id).cloned() {
            Some(route) => {
                flights.select_path(id);
                flights.fly_to_path(id);
                info!(
                    "--taxi-path: {id} {} -> {}, {} points, {:.0} yd, {} copper",
                    flights.name(route.path.from),
                    flights.name(route.path.to),
                    route.points.len(),
                    route.length,
                    route.path.cost
                );
            }
            None => warn!("--taxi-path {id}: no path by that id on this map"),
        }
    }
    for finding in &flights.findings {
        warn!("flight paths: {finding}");
    }
    *done = true;
    flights.scripted_done = true;
}

/// Rebuild the lists now from the session's tables, so a scripted step can
/// read what the one before it made.
fn rebuilt(session: &EditSession, flights: &mut Flightpaths) -> Option<()> {
    let map = flights.map;
    rebuild(
        flights,
        session.table(taxi::NODES)?,
        session.table(taxi::PATHS)?,
        session.table(taxi::POINTS)?,
        map,
    );
    Some(())
}

/// What the open map's flight map is.
#[derive(Debug, Clone, PartialEq)]
pub enum FlightMap {
    /// The map is a continent of the world map, whose flight map the game
    /// ships.
    Shipped,
    /// The map has no `WorldMapContinent` row, so the client opens no flight
    /// map here.
    Missing,
    /// The map has a row: its box, and the nodes outside it, which the client
    /// does not draw.
    Made { flight: vale_edit::flightmap::FlightMap, outside: Vec<u32> },
}

/// The open map's flight map. `None` while the tables are not open.
pub fn flight_map(session: &EditSession, flights: &Flightpaths) -> Option<FlightMap> {
    use vale_edit::flightmap;
    let continents = session.table(flightmap::TABLE)?;
    let areas = session.table(WORLD_MAP_AREA)?;
    let continent = (0..areas.record_count()).any(|record| {
        areas.u32_at(record, 1) == Some(flights.map) && areas.u32_at(record, 2) == Some(0)
    });
    if continent {
        return Some(FlightMap::Shipped);
    }
    let Some(flight) = flightmap::row_of(continents, flights.map).and_then(|record| flightmap::read(continents, record)) else {
        return Some(FlightMap::Missing);
    };
    let outside = flights.nodes.iter().filter(|node| !flight.holds(node.at[0], node.at[1])).map(|node| node.id).collect();
    Some(FlightMap::Made { flight, outside })
}

/// The table whose continent rows (area 0) mark a map as one of the world
/// map's continents: field 1 is the map, field 2 the area.
pub const WORLD_MAP_AREA: &str = "WorldMapArea";

/// Give the open map a flight map, or fit the one it has to its tiles and
/// nodes: write its `WorldMapContinent` row as one undo entry, and draw the
/// picture. Answers what happened, for the status line.
pub fn make_flight_map(session: &mut EditSession, assets: &vale_client::assets::GameAssets, flights: &mut Flightpaths) -> String {
    use vale_edit::flightmap;
    if session.map_id != flights.map {
        return "the flight path tool is still reading this map".to_string();
    }
    let index = minimap_index(assets);
    let mut tiles: Vec<(u32, u32)> = session.claimed.iter().copied().collect();
    tiles.extend(index.tiles_of(&session.map));
    tiles.sort_unstable();
    tiles.dedup();
    let nodes: Vec<[f32; 2]> = flights.nodes.iter().map(|node| [node.at[0], node.at[1]]).collect();
    let Some(flight) = flightmap::plan(flights.map, &tiles, &nodes) else {
        return "this map has no tiles and no nodes to make a flight map of".to_string();
    };
    let Some(table) = session.table_mut(flightmap::TABLE) else {
        return format!("{} is not open", flightmap::TABLE);
    };
    let written = match flightmap::write(table, &flight) {
        Ok(written) => written,
        Err(why) => return why.to_string(),
    };
    session.history.begin("Make flight map");
    match written {
        flightmap::Written::Added(row) => session.history.record_row(flightmap::TABLE, row),
        flightmap::Written::Changed(cells) => {
            for cell in cells {
                session.history.record_cell(flightmap::TABLE, cell);
            }
        }
    }
    session.history.end();
    session.table_edited(flightmap::TABLE);
    let side = flight.taxi_box[2] - flight.taxi_box[0];
    let drawn = draw_flight_map(session, assets, flights, &flight);
    format!("flight map for map {}: {side:.0} yards square; {drawn}", flight.map)
}

/// Draw the flight map picture of `flight` from the map's minimap tiles and
/// write it into the project. Answers what happened.
pub fn draw_flight_map(
    session: &mut EditSession,
    assets: &vale_client::assets::GameAssets,
    flights: &mut Flightpaths,
    flight: &vale_edit::flightmap::FlightMap,
) -> String {
    use vale_assets::world::blp;
    use vale_edit::flightmap;
    let index = minimap_index(assets);
    let directory = session.map.clone();
    let mut drawn = 0usize;
    let rgba = flightmap::picture(flight.taxi_box, |col, row| {
        let path = index.texture(&directory, col, row)?;
        let bytes = assets.with_archive(|chain| Ok(chain.read(&path).ok())).ok().flatten()?;
        let picture = blp::decode(&bytes).ok()?;
        let side = flightmap::TILE_SIDE;
        (picture.width == side && picture.height == side).then(|| {
            drawn += 1;
            picture.rgba
        })
    });
    let bytes = match blp::encode_dxt1(&rgba, flightmap::SIDE, flightmap::SIDE) {
        Ok(bytes) => bytes,
        Err(why) => return format!("the picture could not be encoded: {why}"),
    };
    let path = vale_assets::tables::taxi::map_art(flight.map);
    if !session.save_bytes(&path, bytes) {
        return format!("{path} could not be written");
    }
    flights.picture_known = Some((flight.map, true));
    match drawn {
        0 => "the picture is blank: no minimap tile of this map is drawn".to_string(),
        n => format!("picture drawn from {n} minimap tile(s)"),
    }
}

/// Whether the archive chain holds the open map's flight map picture, read
/// once per map.
pub fn has_picture(assets: &vale_client::assets::GameAssets, flights: &mut Flightpaths) -> bool {
    if let Some((map, known)) = flights.picture_known {
        if map == flights.map {
            return known;
        }
    }
    let path = vale_assets::tables::taxi::map_art(flights.map);
    let known = assets.with_archive(|chain| Ok(chain.read(&path).is_ok())).unwrap_or(false);
    flights.picture_known = Some((flights.map, known));
    known
}

/// The minimap index as the archive chain holds it now, the project's own
/// tiles included. Read each time rather than through the cached copy,
/// which predates any tile drawn this session.
fn minimap_index(assets: &vale_client::assets::GameAssets) -> vale_assets::tables::minimap::MinimapTiles {
    use vale_assets::tables::minimap::{MinimapTiles, MD5_TRANSLATE};
    let raw = assets.with_archive(|chain| Ok(chain.read(MD5_TRANSLATE).ok())).ok().flatten().unwrap_or_default();
    MinimapTiles::parse(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(index: u32, x: f32, z: f32) -> Point {
        Point {
            id: 100 + index,
            record: index as usize,
            path: 1,
            index,
            map: 0,
            at: [x, 0.0, z],
            action_flag: 0,
            delay: 0,
        }
    }

    #[test]
    fn a_click_goes_into_the_nearest_leg() {
        let points = [point(0, 0.0, 10.0), point(1, 100.0, 50.0), point(2, 200.0, 10.0)];
        // Beside the first leg, a quarter of the way along it.
        let (index, height) = insertion(&points, Vec3::new(25.0, 30.0, 0.0));
        assert_eq!(index, 1);
        assert!((height - 20.0).abs() < 1e-3, "{height}");
        // Beside the second leg.
        assert_eq!(insertion(&points, Vec3::new(150.0, -5.0, 0.0)).0, 2);
        // Past the end: still between the last two points, never after the
        // last, which stands at the node.
        assert_eq!(insertion(&points, Vec3::new(400.0, 0.0, 0.0)).0, 2);
        assert_eq!(insertion(&points, Vec3::new(-400.0, 0.0, 0.0)).0, 1);
    }

    #[test]
    fn a_seeded_path_starts_and_ends_at_its_nodes_and_clears_the_ground() {
        let from = Vec3::new(0.0, 0.0, 10.0);
        let to = Vec3::new(500.0, 0.0, 30.0);
        let points = seed(from, to, 40.0, |x, _| Some(if x > 200.0 { 100.0 } else { 0.0 }));
        assert_eq!(points.first(), Some(&from.to_array()));
        assert_eq!(points.last(), Some(&to.to_array()));
        assert_eq!(points.len(), 6, "500 yards at 120 is five legs");
        for point in &points[1..points.len() - 1] {
            let ground = if point[0] > 200.0 { 100.0 } else { 0.0 };
            assert!(point[2] >= ground + 40.0 - 1e-3, "{point:?}");
        }
        // Two nodes closer than the spacing are one leg.
        assert_eq!(seed(from, from + Vec3::X * 10.0, 40.0, |_, _| None).len(), 2);
    }

    #[test]
    fn the_suggested_cost_is_the_maps_median_rate() {
        let route = |cost: u32, length: f32, transport: bool| Route {
            path: Path {
                id: 1,
                record: 0,
                from: 1,
                to: 2,
                cost,
            },
            points: Vec::new(),
            length,
            transport,
        };
        let routes = [
            route(100, 1000.0, false),
            route(300, 1000.0, false),
            route(200, 1000.0, false),
            route(0, 1000.0, false),
            route(9000, 10.0, true),
        ];
        assert_eq!(suggested_cost(&routes, 2000.0), 400);
        assert_eq!(suggested_cost(&[], 2000.0), 0);
    }

    #[test]
    fn the_ray_meets_a_segment_where_they_pass_closest() {
        let origin = Vec3::new(0.0, 0.0, 100.0);
        let down = Vec3::new(0.0, 0.0, -1.0);
        let (along, off) =
            ray_to_segment(origin, down, Vec3::new(-10.0, 3.0, 0.0), Vec3::new(10.0, 3.0, 0.0))
                .unwrap();
        assert!((along - 100.0).abs() < 1e-3);
        assert!((off - 3.0).abs() < 1e-3);
        // A segment behind the eye is not picked.
        assert!(ray_to_segment(origin, -down, Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0))
            .is_none());
    }

    #[test]
    fn an_upright_drag_plane_faces_the_camera() {
        let origin = Vec3::new(-100.0, 0.0, 50.0);
        let direction = Vec3::new(1.0, 0.0, -0.2).normalize();
        let at = Vec3::new(0.0, 0.0, 20.0);
        let hit = meets_upright_plane(origin, direction, at).unwrap();
        assert!(hit.x.abs() < 1e-3, "the plane passes through the handle");
        assert!((hit.z - 30.0).abs() < 1e-2, "{hit:?}");
        // Looking straight down there is no upright plane to meet.
        assert!(meets_upright_plane(origin, Vec3::new(0.0, 0.0, -1.0), at).is_none());
    }

    /// A drawn path runs from the start node through the placed points, in
    /// the order they were placed, to the far node.
    #[test]
    fn a_drawn_path_is_its_two_nodes_and_the_points_between() {
        let from = Vec3::new(0.0, 0.0, 10.0);
        let to = Vec3::new(300.0, 0.0, 20.0);
        let placed = [Vec3::new(100.0, 5.0, 80.0), Vec3::new(200.0, -5.0, 120.0)];
        let points = drawn_points(from, &placed, to);
        assert_eq!(points.len(), 4);
        assert_eq!(points[0], from.to_array());
        assert_eq!(points[1], placed[0].to_array());
        assert_eq!(points[2], placed[1].to_array());
        assert_eq!(points[3], to.to_array());
        // No points placed is a straight path between the nodes.
        assert_eq!(drawn_points(from, &[], to).len(), 2);
    }

    /// Connect with drawing on starts a draft at the clearance above the start
    /// node and drops the path and point selections; with drawing off it arms
    /// the seeded Connect. Disarming discards the draft.
    #[test]
    fn connect_arms_a_draft_when_drawing_by_hand() {
        let mut flights = Flightpaths::default();
        flights.nodes.push(Node {
            id: 5,
            record: 0,
            map: 0,
            at: [10.0, 20.0, 30.0],
            name: "Here".to_string(),
            mounts: NEW_NODE_MOUNTS,
        });
        flights.path = Some(7);
        flights.point = Some(101);

        flights.arm_connect(5);
        assert_eq!(flights.armed, Armed::Connect { from: 5 });
        assert!(flights.draft.is_none());

        flights.draw_by_hand = true;
        flights.clearance = 40.0;
        flights.arm_connect(5);
        assert_eq!(flights.armed, Armed::Draw { from: 5 });
        let draft = flights.draft.as_ref().unwrap();
        assert_eq!(draft.height, 70.0);
        assert!(draft.points.is_empty());
        assert_eq!((flights.path, flights.point), (None, None));

        flights.disarm();
        assert_eq!(flights.armed, Armed::Nothing);
        assert!(flights.draft.is_none());
    }

    /// The next point stands above the pointer at the draft height, and above
    /// the press while a lift is held, wherever the pointer has gone.
    #[test]
    fn the_next_point_follows_the_pointer_or_the_lift() {
        let mut draft = Draft {
            from: 1,
            points: Vec::new(),
            height: 50.0,
            lift: None,
        };
        let pointer = Vec3::new(3.0, 4.0, 1.0);
        assert_eq!(draft.next(Some(pointer)), Some(Vec3::new(3.0, 4.0, 50.0)));
        assert_eq!(draft.next(None), None);
        draft.lift = Some(Lift {
            at: Vec2::new(9.0, 9.0),
            grab: None,
        });
        assert_eq!(draft.next(Some(pointer)), Some(Vec3::new(9.0, 9.0, 50.0)));
        assert_eq!(draft.next(None), Some(Vec3::new(9.0, 9.0, 50.0)));
    }

    #[test]
    fn a_path_is_framed_from_about_its_width() {
        assert_eq!(framing_distance(10.0), 120.0);
        assert_eq!(framing_distance(1000.0), 900.0);
        assert_eq!(framing_distance(50_000.0), 1500.0);
    }

    #[test]
    fn a_nodes_colour_is_the_sides_vmangos_offers_it_to() {
        assert_eq!(side_colour([2224, 541]), side_colour(NEW_NODE_MOUNTS));
        assert_ne!(side_colour([2224, 0]), side_colour([0, 541]));
        assert_ne!(side_colour([0, 0]), side_colour([2224, 541]));
    }

    #[test]
    fn a_selection_follows_what_is_clicked() {
        let mut flights = Flightpaths::default();
        flights.routes.push(Route {
            path: Path {
                id: 7,
                record: 0,
                from: 1,
                to: 2,
                cost: 0,
            },
            points: vec![point(0, 0.0, 0.0), point(1, 1.0, 0.0)],
            length: 1.0,
            transport: false,
        });
        flights.select_point(101);
        assert_eq!((flights.path, flights.point), (Some(7), Some(101)));
        // A node at either end keeps the path.
        flights.select_node(2);
        assert_eq!(flights.path, Some(7));
        // A node the path does not touch drops it and its point.
        flights.select_node(3);
        assert_eq!((flights.path, flights.point), (None, None));
    }
}
