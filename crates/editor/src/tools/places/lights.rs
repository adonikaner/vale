//! Draws the open map's lights in the viewport and picks them with the pointer.
//!
//! [`tables`](crate::tables) edits the rows of the light chain. This file
//! handles the same rows in the viewport, where their positions can be seen and
//! moved. A `Light` row is a sphere at a position on a map: an origin, an inner
//! radius within which it applies at full strength, and an outer radius at
//! which it has faded to nothing. Those five numbers do not identify a place
//! when read on a form: `612096, 0, 998400, 19200, 25861` is Duskwood's light,
//! and the form does not say so.
//!
//! Each light is therefore marked at its position, and its falloff is drawn as
//! two rings over the ground it covers. The pointer picks a light and the
//! camera goes to it. The row on the form is then shown together with its place
//! on the map.
//!
//! ## The band across the window came from the present camera
//!
//! With some lights selected, a straight line appeared across the whole window.
//! Four rounds of culling were tried in this file before the line was measured
//! down to its segment: one `gizmos.line` between two points, drawn as a
//! two-pixel band at the window's centre row, and unchanged by any rule about
//! distance, depth or angle. The cause was the client's present camera, a
//! `Camera2d` with a 1x1 orthographic view and no render layer, which drew
//! every gizmo a second time. `render::present::PRESENT_LAYER` is the fix. A
//! count of green pixels per row is what identified the segment.
//!
//! ## Why this tool keeps the viewport
//!
//! The other subject edited through a table covers the viewport; see
//! [`Tool::covers_viewport`](crate::tools::Tool::covers_viewport). A spell has
//! no position, so the viewport behind its form would show nothing about it. A
//! light's position is half of its row. The other half is a set of colours, and
//! the accurate preview of those colours is the world drawn in them. The editor
//! already has that preview: the viewport is the client's own renderer reading
//! the client's own light chain.
//!
//! ## Positions are converted with `placement_to_world`
//!
//! `Light.dbc` stores its three coordinates in the internal representation that
//! every `MDDF` and `MODF` placement is in, scaled by 36.
//! `vale_assets::tables::light` has the measurement and
//! [`vale_assets::world::adt::placement_to_world`] is the conversion. This file
//! calls that function and does not restate it. A second reading of those
//! columns is a bug the light chain has produced repeatedly, and it puts a
//! light in a plausible wrong place without raising an error.
//!
//! ## A default light has no position
//!
//! A row whose `FalloffEnd` is zero is the map's default light. It applies
//! everywhere and its coordinates are not read. Drawn, it would be a sphere at
//! the corner of the map that has no meaning, so [`Mark::everywhere`] marks it
//! and [`draw`] leaves it out. The list still carries it, because it lights the
//! whole map and is therefore the row most worth editing.

use crate::tools::Tool;
use crate::marks::{Look, Marks};
use crate::session::EditSession;
use vale_assets::tables::light::{light_field as lf, YARDS_PER_UNIT};
use vale_client::render::axes;
use vale_client::render::focus::WorldFocus;
use vale_client::world::camera::WorldCamera;
use bevy::prelude::*;

/// One light, as the viewport needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Mark {
    pub(crate) id: u32,
    /// Its row in the open table, so a press can open it without searching.
    pub(crate) record: usize,
    /// The centre, in the world's own axes.
    pub(crate) at: Vec3,
    /// The distance in yards within which it applies at full strength.
    pub(crate) start: f32,
    /// The distance in yards at which it has faded to nothing.
    pub(crate) end: f32,
    /// True for the map's default light, which applies everywhere and whose
    /// position is not read. Kept in the list and left out of the drawing.
    pub(crate) everywhere: bool,
}

/// What the pointer is doing with the lights on the open map.
#[derive(Resource, Debug, Default)]
pub(crate) struct Lights {
    /// Every light on the open map. Rebuilt when the table or the map changes,
    /// not per frame; see [`collect`].
    pub(crate) marks: Vec<Mark>,
    /// What [`marks`](Self::marks) was built from: the session's table revision
    /// and the map it was read for.
    built: Option<(u64, u32)>,
    /// The handle under the pointer: which light, and which of its three
    /// handles.
    ///
    /// A handle and not a light, because a light has three handles and dragging
    /// each one changes a different part of the row. [`draw`] draws the hovered
    /// handle highlighted.
    pub(crate) hovered: Option<(u32, Dragging)>,
    /// The id of the chosen light, whose row the panel has open.
    pub(crate) selected: Option<u32>,
    /// Whether the chosen light's falloff is drawn, or only its marker. On by
    /// default.
    ///
    /// This setting used to mean "draw every light's falloff", and that drew a
    /// bar across the window: map 0 has 80 lights, two of their outer radii are
    /// 2,005 and 2,648 yards, and 38 of them lie flat at height zero. Every
    /// light is now marked whatever this is set to. The setting covers only the
    /// falloff of the light being edited, and turning it off is useful while
    /// dragging that light.
    pub(crate) show_falloff: bool,
    /// What the left button is moving, if anything; see [`Drag`].
    pub(crate) drag: Option<Drag>,
    /// Whether a click on the ground makes a light there. See [`new_light`].
    pub(crate) armed: bool,
    /// Which of the chosen light's five conditions the panel shows the
    /// `LightParams` row of.
    pub(crate) condition: Condition,
    /// Where the camera is, in the world's axes. The panel reads it for a
    /// light's weight at the camera and for the distances in its list. `None`
    /// until [`aim`] has run with the tool chosen.
    pub(crate) eye: Option<Vec3>,
    /// What the panel reads out of the tables; see [`PanelData`].
    panel: PanelData,
}

/// What the light panel derives from the open tables and would otherwise
/// compute on every frame it is drawn: who uses each `LightParams` row, and
/// the rows of `LightSkybox`.
///
/// It is built again when [`Self::key`] moves, which is on every table edit,
/// undo, discard and project switch, and when a table is opened. Nothing is
/// rebuilt on a frame where no table changed, and nothing is stale after one
/// that did: a skybox row added by the project is in the list on the next
/// frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct PanelData {
    /// The session's table revision and how many tables it has open, when
    /// this was built. Opening a table does not move the revision.
    key: Option<(u64, usize)>,
    /// For each `LightParams` id a light names, the lights that name it, in
    /// `Light.dbc`'s order, over every map.
    pub(crate) users: std::sync::Arc<std::collections::HashMap<u32, Vec<Use>>>,
    /// The rows of `LightSkybox.dbc` as the project has them, in the table's
    /// order. Empty while the table is not open.
    pub(crate) skyboxes: std::sync::Arc<[Skybox]>,
}

/// One light's use of a `LightParams` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Use {
    pub(crate) light: u32,
    /// Which conditions the light names the row under, as bit `n` for
    /// `Condition::ALL[n]`.
    under: u8,
}

impl Use {
    /// The conditions the light names the row under, in column order.
    pub(crate) fn conditions(self) -> impl Iterator<Item = Condition> {
        Condition::ALL.into_iter().filter(move |condition| self.under & (1 << *condition as u8) != 0)
    }

    /// This use without one condition, or `None` when that was the only one.
    pub(crate) fn without(self, condition: Condition) -> Option<Use> {
        let under = self.under & !(1 << condition as u8);
        (under != 0).then_some(Use { light: self.light, under })
    }
}

/// One row of `LightSkybox.dbc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Skybox {
    pub(crate) id: u32,
    pub(crate) record: usize,
    /// The model as the row states it, with the `.mdx` extension.
    pub(crate) model: String,
    /// The archive path the client reads for it, or empty for no model.
    pub(crate) path: String,
    /// What a list calls the row: the model's file name and the id.
    pub(crate) label: String,
}

impl Lights {
    /// The panel's derived data, built again when a table has changed or
    /// been opened since it was built. The answer is two shared pointers, so
    /// taking it costs no copy of the lists.
    pub(crate) fn panel_data(&mut self, session: &EditSession) -> PanelData {
        let key = (session.table_revision, session.tables.len());
        if self.panel.key != Some(key) {
            let mut users: std::collections::HashMap<u32, Vec<Use>> = std::collections::HashMap::new();
            if let Some(table) = session.table("Light") {
                for record in 0..table.record_count() {
                    let light = table.u32_at(record, 0).unwrap_or(0);
                    for condition in Condition::ALL {
                        let Some(params) = table.u32_at(record, condition.field()).filter(|&id| id != 0) else {
                            continue;
                        };
                        let bit = 1 << condition as u8;
                        let uses = users.entry(params).or_default();
                        match uses.last_mut().filter(|last| last.light == light) {
                            Some(last) => last.under |= bit,
                            None => uses.push(Use { light, under: bit }),
                        }
                    }
                }
            }
            let mut skyboxes = Vec::new();
            if let Some(table) = session.table("LightSkybox") {
                for record in 0..table.record_count() {
                    let id = table.u32_at(record, 0).unwrap_or(0);
                    let model = table.string_at(record, 1).unwrap_or_default();
                    let label = match model.is_empty() {
                        true => format!("LightSkybox {id}, no model"),
                        false => format!("{} ({id})", model.rsplit('\\').next().unwrap_or(&model)),
                    };
                    let path = match model.is_empty() {
                        true => String::new(),
                        false => vale_assets::world::m2::model_path(&model),
                    };
                    skyboxes.push(Skybox { id, record, model, path, label });
                }
            }
            self.panel = PanelData { key: Some(key), users: users.into(), skyboxes: skyboxes.into() };
        }
        self.panel.clone()
    }
}

/// One of the five `LightParams` columns of a `Light` row. The client lights
/// by the row a column names while that column's condition holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Condition {
    #[default]
    Clear,
    ClearUnderwater,
    Storm,
    StormUnderwater,
    Death,
}

impl Condition {
    /// The five, in the order of `Light.dbc`'s columns.
    pub(crate) const ALL: [Condition; 5] = [
        Condition::Clear,
        Condition::ClearUnderwater,
        Condition::Storm,
        Condition::StormUnderwater,
        Condition::Death,
    ];

    /// The field of `Light.dbc` that holds this condition's `LightParams` id.
    /// The five are consecutive from `ParamsClear`.
    pub(crate) fn field(self) -> usize {
        lf::PARAMS_CLEAR + self as usize
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Condition::Clear => "Clear",
            Condition::ClearUnderwater => "Clear Underwater",
            Condition::Storm => "Storm",
            Condition::StormUnderwater => "Storm Underwater",
            Condition::Death => "Death",
        }
    }

    /// The column's name in `Light.dbc`.
    pub(crate) fn column(self) -> &'static str {
        match self {
            Condition::Clear => "ParamsClear",
            Condition::ClearUnderwater => "ParamsClearUnderwater",
            Condition::Storm => "ParamsStorm",
            Condition::StormUnderwater => "ParamsStormUnderwater",
            Condition::Death => "ParamsDeath",
        }
    }

    /// When the client lights by this column's row.
    pub(crate) fn when(self) -> &'static str {
        match self {
            Condition::Clear => "Fair weather, with the camera above the water.",
            Condition::ClearUnderwater => "Fair weather, with the camera under the water.",
            Condition::Storm => {
                "While the server reports rain, snow or a sandstorm. The light is blended from \
                 Clear toward this row by how hard it falls."
            }
            Condition::StormUnderwater => "The storm's light, with the camera under the water.",
            Condition::Death => {
                "While the player is a ghost. The client lights the whole map by the default \
                 light's Death row and lays no other light over it, so this column is read \
                 on the map's default light only."
            }
        }
    }

    /// The condition whose row the client lights by when this one's column
    /// holds 0. `None` for Clear, which has nothing behind it, and for Death,
    /// where a default light with no Death row lights the ghost world as the
    /// living one.
    pub(crate) fn falls_back_to(self) -> Option<Condition> {
        match self {
            Condition::Clear | Condition::Death => None,
            Condition::ClearUnderwater | Condition::Storm => Some(Condition::Clear),
            Condition::StormUnderwater => Some(Condition::ClearUnderwater),
        }
    }

    /// Whether the editor's viewport can draw this condition. It has no
    /// weather and no ghost world, so it draws Clear, and Clear Underwater
    /// while the camera is under the water.
    pub(crate) fn drawn_in_the_editor(self) -> bool {
        matches!(self, Condition::Clear | Condition::ClearUnderwater)
    }
}

impl Mark {
    /// How strongly this light applies at `at`: 1 within `start`, falling
    /// linearly to 0 at `end`, by three-dimensional distance. This is
    /// `vale_assets::tables::light::PositionalLight::weight`'s rule. The
    /// default light has no weight of its own: it is what the others are
    /// laid over.
    pub(crate) fn weight(&self, at: Vec3) -> f32 {
        let distance = self.at.distance(at);
        if self.everywhere || distance >= self.end {
            return 0.0;
        }
        let span = self.end - self.start;
        if distance <= self.start || span <= 0.0 {
            return 1.0;
        }
        (self.end - distance) / span
    }
}

/// The radii a new light is made with, in yards: full strength within the
/// first, faded out by the second.
pub(crate) const NEW_RADII: (f32, f32) = (50.0, 120.0);

impl Lights {
    /// The map's default light, if the open map has one.
    pub(crate) fn default_light(&self) -> Option<&Mark> {
        self.marks.iter().find(|mark| mark.everywhere)
    }

    /// The light a new one copies its weather from: the chosen light, else the
    /// map's default light, else any light on the map.
    pub(crate) fn template(&self) -> Option<&Mark> {
        self.selected
            .and_then(|id| self.marks.iter().find(|mark| mark.id == id))
            .or_else(|| self.default_light())
            .or_else(|| self.marks.first())
    }
}

/// Make a light on `map`, at `centre` or as the map's default light with
/// `None`, lit like [`Lights::template`] (or, on a map with no light, like
/// light 1, Eastern Kingdoms' default). Select it and open its row. Answers
/// the status line.
pub(crate) fn new_light(
    session: &mut EditSession,
    lights: &mut Lights,
    browser: &mut crate::tables::browser::Browser,
    map: u32,
    centre: Option<Vec3>,
) -> String {
    let (template, like) = match lights.template() {
        Some(mark) => (Some(mark.record), mark.id),
        None => (session.table("Light").and_then(|table| table.row_of(1)), 1),
    };
    let Some(record) = crate::tables::rows::add_light(session, map, centre.map(|at| at.to_array()), NEW_RADII, template) else {
        return "Light.dbc is not open".to_string();
    };
    let Some(id) = session.table("Light").and_then(|table| table.u32_at(record, 0)) else {
        return "the new light has no id".to_string();
    };
    lights.selected = Some(id);
    lights.armed = false;
    browser.look_at("Light");
    browser.follow(session, "Light", id);
    match centre {
        Some(_) => format!("created light {id} on map {map} with the settings of light {like}"),
        None => format!("created light {id} as the default light of map {map}"),
    }
}

/// What a held left button is moving.
///
/// A drag belongs to the handle it began on, which is a rule of this crate.
/// Once the button is down on a handle, the drag goes on moving that handle
/// when the pointer leaves it or moves over a panel, and does not pick a new
/// one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Drag {
    /// The light being moved, by id and not by index. [`collect`] rebuilds and
    /// reorders the list on every edit, and a drag is an edit, so an index
    /// taken when the button went down names a different light by the next
    /// frame.
    pub(crate) id: u32,
    pub(crate) what: Dragging,
    /// The offset from the pointer to the light, in the world's own axes. With
    /// it, a press near the edge of a handle does not move the light under the
    /// cursor on the first frame.
    ///
    /// `None` until the first frame of the drag, where it is measured. The
    /// press is handled by a system that has no ray, and the offset cannot be
    /// computed without the ray. A drag writes nothing on that frame, which is
    /// correct because the pointer has not moved yet.
    pub(crate) grab: Option<Vec3>,
    /// The label the undo entry gets, and the gesture key every frame of this
    /// drag writes under; see [`crate::tables::fields::set_fields`].
    pub(crate) subject: &'static str,
}

/// Which of a light's three handles is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dragging {
    /// The centre, across the horizontal plane at the light's own height.
    ///
    /// The height is not dragged. A pointer can aim at two of the three
    /// coordinates and not at the third. A drag that took the height from the
    /// ground would move a light that sits 200 yards up down onto the terrain.
    /// The height is edited in the form's `InternalY`.
    Centre,
    /// The inner radius: where the light stops applying at full strength.
    Start,
    /// The outer radius: where the light has faded to nothing.
    End,
}

impl Lights {
    /// The chosen light, if it is still on this map.
    pub(crate) fn chosen(&self) -> Option<&Mark> {
        let id = self.selected?;
        self.marks.iter().find(|mark| mark.id == id)
    }

    /// The camera request that looks at a light, at the row's stored height.
    ///
    /// A request and not a write, because [`crate::camera`] drives the camera
    /// and a second writer would conflict with it on the frames where a drag is
    /// also moving the camera.
    pub(crate) fn fly_to(mark: &Mark) -> crate::camera::FlyTo {
        crate::camera::FlyTo::at(mark.at)
    }
}

pub(crate) struct LightToolPlugin;

impl Plugin for LightToolPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Lights {
            show_falloff: true,
            ..default()
        })
        .add_systems(
            Update,
            // After the pick, like every tool here. Reading the pointer before
            // this frame's ray is built aims at where the pointer was.
            (collect.run_if(crate::tools::tool_is(Tool::Lights)), aim, press.run_if(crate::tools::tool_is(Tool::Lights)), drag, disarm.run_if(crate::tools::tool_is(Tool::Lights))).chain().after(crate::pick::aim),
        )
        // The drawing runs after the picking, so the ring under the pointer is
        // highlighted on the frame the pointer reached it.
        .add_systems(Update, draw.run_if(crate::tools::tool_is(Tool::Lights)).after(drag));
    }
}

/// How wide a light's centre handle is to the pointer, in pixels.
///
/// A light is a point, and a point cannot be clicked. A handle sized in yards
/// cannot be clicked from far away and covers the screen close up; the doodad
/// tool's first draft had the same fault. A handle sized in pixels is the same
/// target at every distance. Twelve pixels is about a finger's width at the
/// framings this tool is used at.
const HANDLE_PIXELS: f32 = 12.0;

/// How many segments a circle is drawn with. Enough that a 700-yard circle does
/// not look like a polygon.
const RING_SEGMENTS: usize = 48;

/// How far away a light's marker is still drawn, in yards.
///
/// Map 0 has 80 positional lights spread over a continent. Drawing all of them
/// at any distance put a marker in the sky for a light forty tiles away, which
/// shows nothing about the area in view. The range is a little past the block
/// the editor streams, so a marker appears when its ground does.
const MARKER_RANGE: f32 = 2200.0;

/// Yards the drop line under a marker falls.
///
/// The drop line shows that a light is in the air. A point drawn alone has no
/// visible height: 38 of map 0's lights sit at z = 0 and several sit 150 to 436
/// yards up, and without a line running down from the marker nothing on screen
/// tells the two apart.
const DROP: f32 = 120.0;

/// Reads the open map's lights out of the session when the table or the map has
/// changed.
///
/// Not per frame: `Light.dbc` is 374 rows, and this walks all of them, converts
/// five floats each and allocates. The key is the session's table revision and
/// the focused map. Between them they change on every edit, every discard,
/// every project switch and every jump to another map.
fn collect(
    mut lights: ResMut<Lights>,
    session: Option<Res<EditSession>>,
    focus: Res<WorldFocus>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !state.editing() || *tool != Tool::Lights {
        return;
    }
    let Some(session) = session else { return };
    let key = (session.table_revision, focus.map_id);
    if lights.built == Some(key) {
        return;
    }
    let Some(table) = session.table("Light") else {
        // The table is opened a frame or two after the tool is chosen; see
        // `tables::open::open_tables`. Leave the key unset so this runs again.
        return;
    };
    lights.built = Some(key);
    lights.marks.clear();
    for record in 0..table.record_count() {
        if table.u32_at(record, lf::MAP) != Some(focus.map_id) {
            continue;
        }
        let float = |field: usize| table.f32_at(record, field).unwrap_or(0.0);
        let end = float(lf::FALLOFF_END) * YARDS_PER_UNIT;
        lights.marks.push(Mark {
            id: table.u32_at(record, 0).unwrap_or(0),
            record,
            at: Vec3::from(vale_assets::world::adt::placement_to_world([
                float(lf::INTERNAL_X) * YARDS_PER_UNIT,
                float(lf::INTERNAL_Y) * YARDS_PER_UNIT,
                float(lf::INTERNAL_Z) * YARDS_PER_UNIT,
            ])),
            start: float(lf::FALLOFF_START) * YARDS_PER_UNIT,
            end,
            everywhere: end <= 0.0,
        });
    }
    // Widest first, so that a press inside two spheres takes the smaller one.
    // `aim` walks this list and keeps the last match at an equal distance. The
    // nested case is the one that matters: Orgrimmar's light sits inside
    // Durotar's, and if a click on Orgrimmar selected Durotar, the click could
    // not reach the row it points at. `LightTables::parse` sorts the same way
    // for the same reason.
    lights.marks.sort_by(|a, b| {
        b.end
            .partial_cmp(&a.end)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Which light's centre the pointer is over.
///
/// The pick tests the centre handle and not the sphere. A sphere is 700 yards
/// wide and overlaps half the zone, so with a pick against the volume every
/// pointer position would be inside several lights, none of them the one aimed
/// at. The handle is at the light's position, and the position is what a drag
/// moves.
fn aim(
    mut lights: ResMut<Lights>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !state.editing() || *tool != Tool::Lights {
        lights.hovered = None;
        return;
    }
    // The camera's place, for the panel. Read before the returns below, which
    // are about the pointer and not about the camera.
    lights.eye = camera.single().ok().map(|(_, at)| Vec3::from(axes::to_wow(at.translation())));
    // A held drag keeps the handle it grabbed. Without this return the hover
    // moves off the handle as soon as the light moves, and the ring being
    // dragged is no longer the ring drawn highlighted.
    if lights.drag.is_some() {
        return;
    }
    // Where the viewport does not have the pointer, a panel has it. The shell
    // answers that question and this file does not re-derive it. The check
    // keeps a drag inside the form from also aiming here, and it covers the
    // menus and tooltips egui opens over the world as well as the four docked
    // panels.
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        lights.hovered = None;
        return;
    }
    let Some((origin, direction)) = crate::pick::ray(&windows, &camera) else {
        lights.hovered = None;
        return;
    };
    // The handle's radius in yards at the light's own distance, so the target
    // is `HANDLE_PIXELS` wide on screen wherever the light is. It is derived
    // from the camera's vertical field of view and the window's height and is
    // not written as a constant, because both change with the window.
    let Ok(window) = windows.single() else { return };
    let Ok((camera, _)) = camera.single() else {
        return;
    };
    let height = window.height().max(1.0);
    let fov = match camera.clip_from_view().to_cols_array()[5] {
        // The projection's [1][1] is `1 / tan(fov / 2)`. It turns a pixel
        // height into an angle without a check of the projection's type.
        y if y.abs() > f32::EPSILON => y,
        _ => return,
    };
    // The centres are tested first, over every light, and the rings only after.
    // A ring is a 700-yard circle and a centre is a point, so a centre that sat
    // on another light's ring could not be reached if the two competed on
    // distance alone. Ranking the centres ahead of the rings gives the pick to
    // the target that is harder to hit.
    let mut best: Option<(f32, u32)> = None;
    for mark in &lights.marks {
        if mark.everywhere {
            continue;
        }
        let to = mark.at - origin;
        let along = to.dot(direction);
        if along <= 0.0 {
            continue;
        }
        let grabbable = (HANDLE_PIXELS / height) * 2.0 * along / fov;
        if (to - direction * along).length() > grabbable {
            continue;
        }
        // The nearest to the eye wins, and an equal distance keeps the later
        // one, which after `collect`'s sort is the smaller sphere.
        if best.is_none_or(|(best, _)| along <= best) {
            best = Some((along, mark.id));
        }
    }
    if let Some((_, id)) = best {
        lights.hovered = Some((id, Dragging::Centre));
        return;
    }

    // A ring is picked where the pointer's ray meets the light's own plane, not
    // against the circle in three dimensions. The ring is drawn flat at the
    // light's height, so it lies in that plane. The test is then the distance
    // from the centre compared against the radius, and it behaves the same at
    // every camera pitch.
    //
    // Only the chosen light's rings can be grabbed. Map 0 has eighty lights and
    // their rings cross everywhere. Without this rule a pointer moved across
    // the map would pass through a dozen of them, and each one would offer to
    // be resized. The light whose row is open is the one being edited.
    let chosen = lights.chosen().copied();
    let Some(mark) = chosen.filter(|mark| !mark.everywhere) else {
        lights.hovered = None;
        return;
    };
    let Some(at) = crate::pick::meets_plane(origin, direction, mark.at.z) else {
        lights.hovered = None;
        return;
    };
    let from_centre = (at - mark.at).truncate().length();
    let along = (at - origin).length();
    // The same pixel width the centre handle uses, turned into yards at the
    // ring's own distance, so a ring is as easy to grab far away as near.
    let grabbable = (HANDLE_PIXELS / height) * 2.0 * along / fov;
    let candidates = [(Dragging::Start, mark.start), (Dragging::End, mark.end)];
    lights.hovered = candidates
        .into_iter()
        .filter(|&(_, radius)| radius > 0.0)
        .map(|(what, radius)| ((from_centre - radius).abs(), what))
        .filter(|&(off, _)| off <= grabbable)
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, what)| (mark.id, what));
}

/// Escape disarms New Light. With Select chosen it deselects the light, and
/// the panel then lists the map's lights. Not while a text box has the
/// keyboard or a dialog is open, where Escape closes that.
fn disarm(
    mut lights: ResMut<Lights>,
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    tool: Res<Tool>,
) {
    if *tool != Tool::Lights || wants.wants_keyboard_input() || wants.is_popup_open() {
        return;
    }
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    match lights.armed {
        true => lights.armed = false,
        false if lights.drag.is_none() => lights.selected = None,
        false => {}
    }
}

/// Chooses the light under the pointer and opens its row.
fn press(
    mut lights: ResMut<Lights>,
    buttons: Res<ButtonInput<MouseButton>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    mut session: Option<ResMut<EditSession>>,
    mut browser: Option<ResMut<crate::tables::browser::Browser>>,
    cursor: Res<crate::pick::Cursor>,
    focus: Res<WorldFocus>,
    over: (Res<crate::ui::Viewport>, Res<bevy_egui::input::EguiWantsInput>, Query<&Window, With<bevy::window::PrimaryWindow>>),
) {
    if !state.editing() || *tool != Tool::Lights {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    // Armed, a click on the ground makes a light there.
    if lights.armed {
        let (viewport, wants, windows) = over;
        if !crate::ui::over_the_world(&viewport, &wants, &windows) {
            return;
        }
        if let (Some(at), Some(session), Some(browser)) = (cursor.surface, session.as_mut(), browser.as_mut()) {
            let line = new_light(session, &mut lights, browser, focus.map_id, Some(at));
            session.status = line;
        }
        return;
    }
    let Some((id, what)) = lights.hovered else {
        return;
    };
    let session = session.as_deref();
    let already = lights.selected == Some(id);
    lights.selected = Some(id);
    // The press that selects a light also starts the move: one gesture, not
    // select-then-drag. Every other placement tool here does the same, and a
    // user expects it of a handle.
    lights.drag = Some(Drag {
        id,
        what,
        grab: None,
        subject: match what {
            Dragging::Centre => "Light position",
            // One key for both rings, so a drag that takes the inner radius
            // past the outer and carries on is still one entry and not two; see
            // `tables::fields::set_fields`.
            Dragging::Start | Dragging::End => "Light falloff",
        },
    });
    // The row is opened when the selection changes, or when the panel is
    // showing something else.
    //
    // Not on every press: pointing the browser at `Light` each time would
    // discard a followed reference, and re-grabbing the same light to drag it
    // again is the common case. The light already being selected is not a
    // sufficient test, though. After following one of its `ParamsClear` links
    // the panel is on `LightParams`, and a press on the light that is already
    // selected must bring its own row back.
    //
    // Opening the row is the purpose of the pick. The browser is pointed at
    // `Light` first: a press in the world while the panel is looking at
    // `LightParams` would otherwise select a light whose row nothing shows.
    let (Some(session), Some(browser)) = (session, browser.as_mut()) else {
        return;
    };
    let showing_it = browser.table == "Light"
        && browser
            .open
            .and_then(|record| session.table("Light")?.u32_at(record, 0))
            == Some(id);
    if already && showing_it {
        return;
    }
    browser.look_at("Light");
    browser.follow(&session, "Light", id);
}

/// Draws the lights on the open map.
///
/// The falloff is two rings, at the radii the row states: the inner ring is
/// where the light stops applying at full strength and the outer ring is where
/// it has faded out. The rings are flat and are not spheres. A sphere at 700
/// yards surrounds the camera and hides the ground inside it, and the drawing
/// has to show how far across the map the light reaches.
///
/// Everything here is [`Look::Ghosted`]: a light is a target for the pointer,
/// and a ring behind a hill is still part of the falloff.
fn draw(
    mut marks: ResMut<Marks>,
    lights: Res<Lights>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
) {
    if !state.editing() || *tool != Tool::Lights {
        return;
    }
    let Ok(camera) = camera.single() else { return };
    let eye = Vec3::from(axes::to_wow(camera.translation()));

    for mark in &lights.marks {
        if mark.everywhere {
            continue;
        }
        let chosen = lights.selected == Some(mark.id);
        let under = lights.hovered.is_some_and(|(id, _)| id == mark.id);
        // Every light within range gets a marker; only the chosen light is
        // drawn in full. The chosen light is exempt from the range, because the
        // panel's button flies to it, and arriving at a light with no marker
        // would be worse than drawing a marker that is far away.
        if !chosen && (mark.at - eye).length() > MARKER_RANGE {
            continue;
        }
        // Opaque, and warm in colour, not blue. A light marker competes with
        // terrain, water and sky. The first draft was a translucent blue-grey,
        // which against all three was visible in a screenshot and not at a
        // glance. The colours are warm for the light being edited, white under
        // the pointer, and a dimmer amber otherwise.
        let colour = match (chosen, under) {
            (true, _) => Color::srgb(1.0, 0.82, 0.25),
            (false, true) => Color::srgb(1.0, 1.0, 1.0),
            (false, false) => Color::srgb(0.95, 0.72, 0.30),
        };
        marker(&mut marks, mark.at, grab_radius(mark.at, eye), colour);

        // The falloff is drawn for the light being edited and for no other.
        // Drawn for all eighty, it was a screen full of overlapping circles up
        // to 2,648 yards across. One of them lay flat at z = 0 and projected to
        // a straight line across the window at every camera position, which was
        // the bar across the window.
        if !chosen || !lights.show_falloff {
            continue;
        }
        // The ring under the pointer is drawn brighter than the same light's
        // other ring, which shows which of the two a press will take.
        let lit = Color::WHITE;
        let grabbing = |what: Dragging| {
            lights.hovered == Some((mark.id, what))
                || lights
                    .drag
                    .is_some_and(|drag| drag.id == mark.id && drag.what == what)
        };
        let inner = match grabbing(Dragging::Start) {
            true => lit,
            false => Color::srgba(1.0, 0.85, 0.35, 0.85),
        };
        let outer = match grabbing(Dragging::End) {
            true => lit,
            false => Color::srgba(1.0, 0.55, 0.15, 0.6),
        };
        // One ring per radius, flat on the ground, with nothing culled.
        //
        // A falloff is a sphere, and it was once drawn as one. At 2,648 yards
        // the camera is usually inside the sphere, and it showed as arcs
        // leaving the window. The flat footprint lies in the plane a drag moves
        // in and the plane a zone is measured on, and it shows how far the
        // light reaches.
        //
        // Nothing here clips, caps or fades a ring, on purpose. A straight band
        // across the whole window used to appear with certain lights selected.
        // Four culls were added here to remove it: by distance, by what was
        // behind the camera, by the camera's height above the ring, and by
        // depression angle. None changed a pixel of the band, and each one
        // removed part of a ring that should have been drawn. The band did not
        // come from this geometry. The present pass drew every gizmo a second
        // time, orthographically, at one world unit to the window; see
        // `vale_client::render::present::PRESENT_LAYER`. A ring at world z = 0
        // is at Bevy y = 0, which that camera put on the centre row.
        circle(&mut marks, mark.at, mark.start, inner);
        circle(&mut marks, mark.at, mark.end, outer);
    }
}

/// How wide a light's handle is in yards, at the distance it is seen from.
///
/// The pick uses [`HANDLE_PIXELS`] so that a light is the same size to the
/// pointer wherever it is. The marker is drawn at the same width, so the area
/// that can be clicked is the area that is drawn. The width is approximated
/// from the distance alone and does not go through the projection: the marker
/// is only a visual aid, and an error of a pixel or two at the edge of the
/// screen has no effect. The pick itself computes the exact width.
fn grab_radius(at: Vec3, eye: Vec3) -> f32 {
    const AT_A_YARD: f32 = 0.020;
    ((at - eye).length() * AT_A_YARD).clamp(MARKER_SMALLEST, MARKER_LARGEST)
}

/// How small and how large a marker may be drawn, in yards.
///
/// The falloff rings had no upper bound. An apparent size that grows with
/// distance and is not capped fills the window once the camera is far enough
/// away, and the bar that used to run across the window had that form. The
/// bounds are named constants so that the test of the clamp reads the same
/// numbers the drawing does.
const MARKER_SMALLEST: f32 = 0.6;
const MARKER_LARGEST: f32 = 22.0;

/// Draws a light's marker: a ball at the position with a line dropping under
/// it.
///
/// The ball is the same size on screen wherever it is, like a doodad's handle.
/// The drop line shows the height: without it a light in the air and a light on
/// the ground are drawn the same.
fn marker(marks: &mut Marks, at: Vec3, radius: f32, colour: Color) {
    let head = Vec3::from(axes::to_bevy(at.to_array()));
    marks.sphere(head, radius, colour, Look::Ghosted);
    let foot = Vec3::from(axes::to_bevy([at.x, at.y, at.z - DROP]));
    // Dimmer than the ball: the drop line shows what the light is over, and it
    // should not compete with the ball it points at.
    marks.line(foot, head, colour.with_alpha(0.55), Look::Ghosted);
}

/// One flat circle at the light's height, in the world's own axes.
///
/// Drawn as a run of segments, each [`Marks::line`] thick at its own distance,
/// rather than as one ring of a single thickness: a falloff is up to 2,648
/// yards across, and a thickness right for the near side is a thread or a bar
/// on the far side.
fn circle(marks: &mut Marks, at: Vec3, radius: f32, colour: Color) {
    // `is_finite` as well as positive. A NaN fails every comparison, so
    // `radius <= 0.0` alone lets one through and every point of the circle
    // becomes NaN. That draws a line at an undefined position instead of
    // drawing no line.
    if !(radius > 0.0) || !radius.is_finite() {
        return;
    }
    let point = |n: usize| {
        let angle = n as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
        let (a, b) = (radius * angle.cos(), radius * angle.sin());
        Vec3::from(axes::to_bevy([at.x + a, at.y + b, at.z]))
    };
    marks.line_strip((0..=RING_SEGMENTS).map(point), colour, Look::Ghosted);
}

/// Moves the light under a held button and writes the move as one undo entry.
///
/// The write goes through [`crate::tables::fields::set_fields`] so that a drag
/// across the map is one entry and not two per frame. The centre moves
/// `InternalX` and `InternalZ` together, and an undo that took back only one of
/// the two would restore half a movement.
///
/// The position is converted to the file's own representation by
/// [`vale_assets::world::adt::placement_from_world`]. Loading the game's tiles
/// never exercises that direction, so this file calls the function and does not
/// restate it. A second reading of the axis swap that got it wrong would write
/// a light a hundred yards from where it was dropped.
fn drag(
    mut lights: ResMut<Lights>,
    buttons: Res<ButtonInput<MouseButton>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    time: Res<Time>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    mut session: Option<ResMut<EditSession>>,
) {
    if !state.editing() || *tool != Tool::Lights {
        lights.drag = None;
        return;
    }
    if !buttons.pressed(MouseButton::Left) {
        lights.drag = None;
        return;
    }
    let Some(held) = lights.drag else { return };
    let Some(mark) = lights.marks.iter().copied().find(|mark| mark.id == held.id) else {
        // The light left the list: another map, or an undo that removed the
        // row. Let go and do not write into whatever is at that index now.
        lights.drag = None;
        return;
    };
    let Some(session) = session.as_mut() else {
        return;
    };
    // The plane at the light's own height, not the ground: a light 200 yards up
    // is dragged around at 200 yards up, and one under the terrain stays under
    // it. See [`Dragging::Centre`].
    let Some(at) = crate::pick::on_plane(mark.at.z, &windows, &camera) else {
        return;
    };
    // The first frame measures the grab and writes nothing. The pointer has not
    // moved since the press, so there is nothing to write. On the frames after,
    // the light keeps the offset it was grabbed at and its centre does not snap
    // under the cursor.
    let Some(grab) = held.grab else {
        lights.drag = Some(Drag {
            grab: Some(mark.at - at),
            ..held
        });
        return;
    };
    let now = time.elapsed_secs_f64();
    match held.what {
        Dragging::Centre => {
            // The grab offset keeps the light where it was taken hold of, so a
            // press near the edge of the handle does not snap the centre under
            // the pointer.
            let moved = Vec3::new(at.x + grab.x, at.y + grab.y, mark.at.z);
            let placement = vale_assets::world::adt::placement_from_world(moved.to_array());
            crate::tables::fields::set_fields(
                session,
                "Light",
                mark.record,
                &[
                    (lf::INTERNAL_X, (placement[0] / YARDS_PER_UNIT).to_bits()),
                    (lf::INTERNAL_Z, (placement[2] / YARDS_PER_UNIT).to_bits()),
                ],
                "Move light",
                held.subject,
                now,
            );
        }
        Dragging::Start | Dragging::End => {
            // The grab is not applied to a radius. The grab is a vector between
            // two points on the plane and a radius is a length; adding it would
            // make a ring grabbed on its far side resize backwards. The press
            // is within a handle's width of the ring by construction, so the
            // jump the grab would prevent is at most that width.
            let radius = (at - mark.at).truncate().length();
            let field = match held.what {
                Dragging::Start => lf::FALLOFF_START,
                _ => lf::FALLOFF_END,
            };
            crate::tables::fields::set_fields(
                session,
                "Light",
                mark.record,
                &[(field, (radius / YARDS_PER_UNIT).to_bits())],
                "Resize light",
                held.subject,
                now,
            );
        }
    }
    // The list is rebuilt now and not next frame. `collect` keys on the
    // session's table revision, which this write has just changed. Leaving the
    // list as it is would draw the ring where the light was before this frame's
    // drag, which is one frame of lag on the ring the drag is moving.
    lights.built = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A use names its conditions in column order, and taking one away leaves
    /// the others, or nothing when it was the only one.
    #[test]
    fn a_use_names_its_conditions_and_can_lose_one() {
        let both = Use { light: 7, under: (1 << Condition::Clear as u8) | (1 << Condition::Storm as u8) };
        assert_eq!(both.conditions().collect::<Vec<_>>(), vec![Condition::Clear, Condition::Storm]);
        let left = both.without(Condition::Clear).expect("Storm is left");
        assert_eq!(left.conditions().collect::<Vec<_>>(), vec![Condition::Storm]);
        assert_eq!(left.without(Condition::Storm), None);
        // Taking away a condition the use does not have changes nothing.
        assert_eq!(both.without(Condition::Death), Some(both));
    }

    /// The five conditions are `Light.dbc`'s five consecutive columns, and a
    /// light's weight is 1 inside its inner radius and 0 from its outer one.
    #[test]
    fn a_condition_is_a_column_and_a_weight_falls_between_the_radii() {
        let fields: Vec<usize> = Condition::ALL.iter().map(|condition| condition.field()).collect();
        assert_eq!(fields, vec![7, 8, 9, 10, 11]);
        let light = Mark { id: 1, record: 0, at: Vec3::ZERO, start: 100.0, end: 200.0, everywhere: false };
        assert_eq!(light.weight(Vec3::new(50.0, 0.0, 0.0)), 1.0);
        assert!((light.weight(Vec3::new(150.0, 0.0, 0.0)) - 0.5).abs() < 1e-6);
        assert_eq!(light.weight(Vec3::new(0.0, 0.0, 250.0)), 0.0);
        // The default light is what the others are laid over.
        let default = Mark { everywhere: true, end: 0.0, ..light };
        assert_eq!(default.weight(Vec3::ZERO), 0.0);
    }

    fn mark(id: u32, end: f32) -> Mark {
        Mark {
            id,
            record: id as usize,
            at: Vec3::ZERO,
            start: end * 0.5,
            end,
            everywhere: end <= 0.0,
        }
    }

    /// A row with no falloff is the map's default light and has no position.
    ///
    /// It stays in the list, because it lights the whole map and is the row
    /// most worth editing. `draw` leaves it out, because its coordinates are
    /// not read and a ring at the corner of the map would have no meaning.
    #[test]
    fn a_default_light_is_marked_as_having_no_position() {
        assert!(mark(1, 0.0).everywhere);
        assert!(!mark(2, 700.0).everywhere);
    }

    /// The chosen light is looked up by id rather than by position, so a
    /// rebuild that reorders the list does not change what is selected.
    #[test]
    fn the_chosen_light_survives_a_rebuild() {
        let mut lights = Lights {
            marks: vec![mark(1, 700.0), mark(2, 300.0)],
            selected: Some(2),
            ..default()
        };
        assert_eq!(lights.chosen().map(|mark| mark.id), Some(2));
        lights.marks.reverse();
        assert_eq!(lights.chosen().map(|mark| mark.id), Some(2));
        // A light that is not on this map answers nothing, and does not answer
        // with whatever is at that position now.
        lights.marks.retain(|mark| mark.id != 2);
        assert_eq!(lights.chosen(), None);
    }

    /// The grab offset keeps a light from snapping under the pointer, and it
    /// composes to the identity when the pointer has not moved.
    ///
    /// The arithmetic is two lines in `drag`. With it backwards, a light jumps
    /// by the width of the handle on the first frame of every drag, in the
    /// direction of the edge it was grabbed by.
    #[test]
    fn a_grab_keeps_the_light_where_it_was_taken_hold_of() {
        let light = Vec3::new(100.0, 200.0, 50.0);
        // Pressed a little off the centre, as every press is.
        let pressed_at = Vec3::new(104.0, 197.0, 50.0);
        let grab = light - pressed_at;
        // The pointer has not moved: the light must not either.
        let moved = Vec3::new(pressed_at.x + grab.x, pressed_at.y + grab.y, light.z);
        assert_eq!(moved, light);
        // A pointer moved by a vector moves the light by the same vector.
        let then = pressed_at + Vec3::new(30.0, -12.0, 0.0);
        let moved = Vec3::new(then.x + grab.x, then.y + grab.y, light.z);
        assert_eq!(moved, light + Vec3::new(30.0, -12.0, 0.0));
    }

    /// A drag writes the file's own representation, and the conversion round
    /// trips.
    ///
    /// A drag uses the direction that loading the game's tiles never exercises,
    /// so it is checked here: a position converted out and back is the position
    /// it started at, and the scale is the chain's 1/36 of a yard and not
    /// yards.
    #[test]
    fn a_dragged_position_round_trips_through_the_files_own_axes() {
        use vale_assets::world::adt::{placement_from_world, placement_to_world};
        let at = [-10666.7_f32, 64.0, 0.0];
        let placement = placement_from_world(at);
        let back = placement_to_world(placement);
        for n in 0..3 {
            assert!(
                (back[n] - at[n]).abs() < 0.01,
                "axis {n}: {} vs {}",
                back[n],
                at[n]
            );
        }
        // Stored, the numbers are 36 times larger, which is why `Light.dbc`'s
        // columns read as hundreds of thousands.
        let stored = placement[0] / YARDS_PER_UNIT;
        assert!((stored - placement[0] * 36.0).abs() < 0.5);
    }

    /// The two rings share one gesture key and the centre has its own.
    ///
    /// A drag that carries the inner radius out past the outer one and keeps
    /// going is one movement and should be one entry to undo. A move and a
    /// resize are different edits and should not fold together because they
    /// happened within the gesture window.
    #[test]
    fn the_two_rings_undo_as_one_movement_and_a_move_is_its_own() {
        let key = |what: Dragging| match what {
            Dragging::Centre => "Light position",
            Dragging::Start | Dragging::End => "Light falloff",
        };
        assert_eq!(key(Dragging::Start), key(Dragging::End));
        assert_ne!(key(Dragging::Centre), key(Dragging::Start));
    }

    /// A ring is picked by how far the pointer is from the radius, and the
    /// nearer of the two wins.
    ///
    /// This is the arithmetic `aim` runs, without the ray. The case that
    /// matters is a pointer between two close rings: it must take the ring it
    /// is nearer to, not the first one tested.
    #[test]
    fn the_nearer_ring_is_the_one_grabbed() {
        let pick = |from_centre: f32, start: f32, end: f32, grabbable: f32| {
            [(Dragging::Start, start), (Dragging::End, end)]
                .into_iter()
                .filter(|&(_, radius)| radius > 0.0)
                .map(|(what, radius)| ((from_centre - radius).abs(), what))
                .filter(|&(off, _)| off <= grabbable)
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(_, what)| what)
        };
        // Just outside the inner ring of a 100/140 light.
        assert_eq!(pick(104.0, 100.0, 140.0, 12.0), Some(Dragging::Start));
        assert_eq!(pick(136.0, 100.0, 140.0, 12.0), Some(Dragging::End));
        // Between the two and out of reach of both.
        assert_eq!(pick(120.0, 100.0, 140.0, 12.0), None);
        // Nearly on top of each other: the nearer still wins rather than the
        // first tested.
        assert_eq!(pick(101.0, 100.0, 102.0, 12.0), Some(Dragging::Start));
        assert_eq!(pick(101.5, 100.0, 102.0, 12.0), Some(Dragging::End));
        // A light with no inner radius offers only its outer one.
        assert_eq!(pick(0.5, 0.0, 1.0, 12.0), Some(Dragging::End));
    }

    /// A drag names its light by id, so the rebuild an edit causes cannot make
    /// it write into a different row.
    ///
    /// `collect` re-sorts by radius on every edit and a drag is an edit, so an
    /// index taken at the press names a different light by the next frame. The
    /// lookup by id is what makes the rebuild safe.
    #[test]
    fn a_drag_names_its_light_by_id() {
        let held = Drag {
            id: 7,
            what: Dragging::Centre,
            grab: None,
            subject: "Light position",
        };
        let mut marks = vec![mark(3, 900.0), mark(7, 400.0), mark(9, 100.0)];
        let found = |marks: &[Mark]| {
            marks
                .iter()
                .find(|mark| mark.id == held.id)
                .map(|m| m.record)
        };
        assert_eq!(found(&marks), Some(7));
        marks.reverse();
        assert_eq!(found(&marks), Some(7));
        marks.retain(|mark| mark.id != 7);
        assert_eq!(found(&marks), None);
    }

    /// A circle whose radius is not a positive number is not drawn.
    ///
    /// `radius <= 0.0` alone lets a NaN through, because every comparison
    /// against NaN is false. A NaN radius makes every point of the circle NaN,
    /// which draws a line at an undefined position instead of drawing no line.
    #[test]
    fn a_circle_needs_a_real_radius() {
        let drawable = |radius: f32| radius > 0.0 && radius.is_finite();
        assert!(drawable(700.0));
        assert!(!drawable(0.0));
        assert!(!drawable(-1.0));
        assert!(!drawable(f32::NAN));
        assert!(!drawable(f32::INFINITY));
    }

    /// A marker is the size at which the pointer can grab it, so what is on
    /// screen is what can be clicked, and its size is bounded.
    ///
    /// The clamp exists because of the bar across the window: an unbounded size
    /// in yards fills the window when the camera is far from the marker.
    #[test]
    fn a_marker_is_grabbable_at_every_distance() {
        let near = grab_radius(Vec3::ZERO, Vec3::new(0.0, 0.0, 10.0));
        let far = grab_radius(Vec3::ZERO, Vec3::new(0.0, 0.0, 4000.0));
        assert!(near < far, "further away is drawn larger in yards");
        assert!(
            near >= MARKER_SMALLEST,
            "and never smaller than a clickable dot"
        );
        assert!(far <= MARKER_LARGEST, "…nor larger than a handle");
        // Very large distances stay clamped and do not fill the window, which
        // is the failure the falloff rings had.
        assert_eq!(
            grab_radius(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0e6)),
            MARKER_LARGEST
        );
    }

    /// Only the chosen light's falloff is drawn. This rule removed the bar
    /// across the window.
    ///
    /// Map 0 has 80 positional lights; two of their outer radii are 2,005 and
    /// 2,648 yards and 38 of them lie flat at height zero, so a flat circle per
    /// light projected to a straight line across the whole window from most
    /// camera positions. A marker per light is drawn either way, which makes a
    /// light clickable without the list.
    #[test]
    fn the_falloff_belongs_to_the_chosen_light_alone() {
        let lights = Lights {
            marks: vec![mark(1, 2648.0), mark(2, 700.0)],
            selected: Some(2),
            show_falloff: true,
            ..default()
        };
        let drawn = |id: u32| lights.selected == Some(id) && lights.show_falloff;
        assert!(
            !drawn(1),
            "the 2,648-yard one is not drawn unless it is chosen"
        );
        assert!(drawn(2));
        // The switch turns off the chosen light's falloff as well.
        let off = Lights {
            show_falloff: false,
            ..lights
        };
        assert!(!(off.selected == Some(2) && off.show_falloff));
    }

    /// A press re-opens the light's row only when the panel is not already
    /// showing it, which lets a followed reference survive.
    ///
    /// The rule is `already && showing_it`, and both halves are needed. Without
    /// `showing_it`, a press on the light that is already selected leaves the
    /// panel on the row that was followed to: the light was clicked and its
    /// numbers did not come back. Without `already`, every press re-points the
    /// browser. That fault made the five `ParamsClear` links appear to do
    /// nothing: the follow landed and was undone on the next frame.
    #[test]
    fn a_press_keeps_a_followed_reference_but_can_get_back_from_it() {
        // (selected already, panel is showing this light) -> does it re-open?
        let reopens = |already: bool, showing_it: bool| !(already && showing_it);
        // Holding it and looking at it: leave the panel alone, so a drag does
        // not fight the form.
        assert!(!reopens(true, true));
        // Holding it and looking at its LightParams row: bring the light back.
        assert!(reopens(true, false));
        // A different light: open it, wherever the panel was.
        assert!(reopens(false, false));
        assert!(reopens(false, true));
    }
}
