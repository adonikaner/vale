//! **Where a light is, drawn on the map and picked with the pointer.**
//!
//! [`tables`](super::tables) edits the light chain's *rows*; this file is the
//! other half of the same subject, and it is the half that makes it editable.
//! A `Light` row is a sphere at a position on a map — an origin, an inner
//! radius it applies at full strength inside, and an outer one it fades to
//! nothing at — and none of those five numbers means anything read as a number.
//! `612096, 0, 998400, 19200, 25861` is Duskwood's light; nothing on a form says
//! so.
//!
//! So each light is marked where it stands, its falloff is drawn as two rings
//! on the ground it covers, the pointer picks one and the camera goes to it.
//! What is on the form then has a place attached to it.
//!
//! ## The band that was not this file's
//!
//! A straight line across the whole window appeared with some lights selected,
//! and it cost four rounds of culling here before it was measured to the
//! segment: one ordinary `gizmos.line` between two ordinary points, drawn as a
//! two-pixel band at the window's centre row, unmoved by any rule about
//! distance, depth or angle. It was the client's present camera — a `Camera2d`
//! with a 1x1 orthographic view and no render layer — drawing every gizmo a
//! second time. `render::present::PRESENT_LAYER` is the fix, and the lesson is
//! the project's own: a picture settles what a count cannot, and a count of
//! green pixels per row is what finally settled this one.
//!
//! ## Why this tool keeps the viewport
//!
//! The other subject edited through a table covers it — see
//! [`Tool::covers_viewport`](super::Tool::covers_viewport). A spell has no
//! position, so there is nothing behind the form worth seeing. A light's
//! position *is* half the row, and the other half is a set of colours whose
//! only honest preview is the world drawn in them, which this editor already
//! has: the viewport is the client's own renderer reading the client's own
//! light chain.
//!
//! ## The positions are converted, not read
//!
//! `Light.dbc` stores its three coordinates in the internal representation
//! every `MDDF` and `MODF` placement is in, scaled by 36 —
//! `vale_assets::tables::light` has the measurement and
//! [`vale_assets::world::adt::placement_to_world`] is the conversion. This
//! file calls that rather than restating it: a second reading of those columns
//! is the bug this chain keeps producing, and it produces a light in a
//! plausible wrong place rather than an error.
//!
//! ## A default light has no position
//!
//! A row whose `FalloffEnd` is zero is the map's default — it applies
//! everywhere and its coordinates are not read. Drawing one would be drawing a
//! sphere at the corner of the map that means nothing, so [`Mark::everywhere`]
//! marks it and [`draw`] leaves it out. The list still carries it, because it
//! is the row most worth editing: it is what the whole map is lit by.

use super::Tool;
use crate::marks::{Look, Marks};
use crate::session::EditSession;
use vale_assets::tables::light::{light_field as lf, YARDS_PER_UNIT};
use vale_client::render::axes;
use vale_client::render::focus::WorldFocus;
use vale_client::world::camera::WorldCamera;
use bevy::prelude::*;

/// One light, as the viewport needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    pub id: u32,
    /// Its row in the open table, so a press can open it without searching.
    pub record: usize,
    /// The centre, in the world's own axes.
    pub at: Vec3,
    /// Yards it applies at full strength within.
    pub start: f32,
    /// …and yards it has faded to nothing by.
    pub end: f32,
    /// **The map's default light**, which applies everywhere and whose position
    /// is not read. Kept in the list and left out of the drawing.
    pub everywhere: bool,
}

/// What the pointer is doing with the lights on the open map.
#[derive(Resource, Debug, Default)]
pub struct Lights {
    /// Every light on the open map. Rebuilt when the table or the map moves,
    /// not per frame — see [`collect`].
    pub marks: Vec<Mark>,
    /// What [`marks`](Self::marks) was built from: the session's table revision
    /// and the map it was read for.
    built: Option<(u64, u32)>,
    /// The handle under the pointer: which light, and which of its three.
    ///
    /// A handle rather than a light, because a light has three things that can
    /// be taken hold of and they are dragged to different ends. Drawn as the
    /// lit one by [`draw`].
    pub hovered: Option<(u32, Dragging)>,
    /// …and the one that is chosen, which is the row the panel has open.
    pub selected: Option<u32>,
    /// **Draw the chosen light's falloff, or only its marker.** On by default.
    ///
    /// It was "draw every light's falloff" and that is what put a bar across
    /// the window: 80 lights on map 0, two of their outer radii 2,005 and
    /// 2,648 yards, 38 of them lying flat at height zero. Every light is
    /// marked now whatever this says; this is only the two spheres of the one
    /// being edited, which is worth turning off while dragging it somewhere.
    pub show_falloff: bool,
    /// Whether a fly-to was asked for, and where. Consumed by [`fly`].
    pub fly_to: Option<Vec3>,
    /// What the left button is moving, if anything — see [`Drag`].
    pub drag: Option<Drag>,
    /// Whether a click on the ground makes a light there. See [`new_light`].
    pub armed: bool,
}

/// The radii a new light is made with, in yards: full strength within the
/// first, faded out by the second.
pub const NEW_RADII: (f32, f32) = (50.0, 120.0);

impl Lights {
    /// The map's default light, if the open map has one.
    pub fn default_light(&self) -> Option<&Mark> {
        self.marks.iter().find(|mark| mark.everywhere)
    }

    /// The light a new one copies its weather from: the chosen light, else the
    /// map's default light, else any light on the map.
    pub fn template(&self) -> Option<&Mark> {
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
pub fn new_light(
    session: &mut EditSession,
    lights: &mut Lights,
    browser: &mut super::tables::Browser,
    map: u32,
    centre: Option<Vec3>,
) -> String {
    let (template, like) = match lights.template() {
        Some(mark) => (Some(mark.record), mark.id),
        None => (session.table("Light").and_then(|table| table.row_of(1)), 1),
    };
    let Some(record) = super::tables::add_light(session, map, centre.map(|at| at.to_array()), NEW_RADII, template) else {
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

/// **What a held left button is moving.**
///
/// A drag belongs to where it began, which is this crate's own rule: once the
/// button is down on a handle, the pointer wandering off it — or over a panel —
/// goes on moving that handle rather than picking a new one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    /// The light being moved, by id. Not by index: [`collect`] rebuilds and
    /// reorders the list on every edit, and a drag *is* an edit, so an index
    /// taken when the button went down names a different light by the next
    /// frame.
    pub id: u32,
    pub what: Dragging,
    /// **How far the light is from the pointer**, in the world's own axes, so
    /// a press near the edge of a handle does not snap the light under the
    /// cursor on the first frame.
    ///
    /// `None` until the first frame of the drag, which is where it is measured:
    /// the press is handled by a system with no ray to hand, and the offset is
    /// the difference between two things only the ray knows. A drag writes
    /// nothing on that frame, which is right — the pointer has not moved yet.
    pub grab: Option<Vec3>,
    /// The label the undo entry gets, and the gesture key every frame of this
    /// drag writes under — see [`super::tables::set_fields`].
    pub subject: &'static str,
}

/// Which of a light's three handles is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dragging {
    /// The centre, across the horizontal plane at the light's own height.
    ///
    /// **The height is not dragged.** Two of the three coordinates can be
    /// aimed at with a pointer and the third cannot, and a drag that guessed
    /// at it from the ground would move a light that sits 200 yards up down
    /// onto the terrain. The form's `InternalY` is where the height is edited.
    Centre,
    /// The inner radius: where the light stops applying at full strength.
    Start,
    /// …and the outer: where it has faded to nothing.
    End,
}

impl Lights {
    /// The chosen light, if it is still on this map.
    pub fn chosen(&self) -> Option<&Mark> {
        let id = self.selected?;
        self.marks.iter().find(|mark| mark.id == id)
    }

    /// Ask the camera to go and look at a light.
    ///
    /// A request rather than a write, because the camera is driven by
    /// [`crate::camera`] and a second writer would fight it on the frames a
    /// drag is also moving it.
    pub fn fly_to(&mut self, mark: &Mark) {
        self.fly_to = Some(mark.at);
    }
}

pub struct LightToolPlugin;

impl Plugin for LightToolPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Lights {
            show_falloff: true,
            ..default()
        })
        .add_systems(
            Update,
            // **After the pick, like every tool here**: reading the pointer
            // before this frame's ray is built aims at where the pointer was.
            (collect, aim, press, drag, disarm).chain().after(crate::pick::aim),
        )
        // …and the drawing after the picking, so the ring that lights up under
        // the pointer does so on the frame the pointer reached it.
        .add_systems(Update, (draw, fly).after(drag));
    }
}

/// How wide a light's centre handle is to the pointer, in **pixels**.
///
/// A light is a point, and a point cannot be clicked. In yards a handle is
/// unclickable from far away and swallows the screen close up, which is the
/// same error the doodad tool's first draft made; in pixels it is the same
/// target at every distance. Twelve is about a finger's width at the framings
/// this tool is used at.
const HANDLE_PIXELS: f32 = 12.0;

/// How many segments a circle is drawn with. Enough that a 700-yard one does
/// not read as a polygon.
const RING_SEGMENTS: usize = 48;

/// **How far a light's marker is drawn from, in yards.**
///
/// Map 0 has 80 positional lights spread over a continent. Drawing all of them
/// whatever their distance put a marker in the sky for a light forty tiles
/// away, which says nothing about anywhere you can see. This is a little past
/// the block the editor streams, so a marker appears as its ground does.
const MARKER_RANGE: f32 = 2200.0;

/// Yards the drop line under a marker falls.
///
/// **This is what makes a light read as being in the air rather than painted
/// on the picture.** A point drawn by itself has no height: 38 of map 0's
/// lights sit at z = 0 and several sit 150 to 436 yards up, and without a line
/// running down from it there is nothing on screen that tells the two apart.
const DROP: f32 = 120.0;

/// **Read the open map's lights out of the session, when they have moved.**
///
/// Not per frame: `Light.dbc` is 374 rows and this walks all of them, converts
/// five floats each and allocates. The key is the session's table revision and
/// the focused map, which between them move on every edit, every discard, every
/// project switch and every jump to another map.
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
        // The table is opened a frame or two after the tool is chosen — see
        // `tables::open_tables`. Leave the key unset so this runs again.
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
    // **Widest first, so a press inside two spheres takes the smaller one.**
    // `aim` walks this list and keeps the last match at an equal distance, and
    // the nested case is the one that matters: Orgrimmar's light sits inside
    // Durotar's, and a click on Orgrimmar that selected Durotar would be a
    // click that cannot reach the row it is pointing at. `LightTables::parse`
    // sorts the same way for the same reason.
    lights.marks.sort_by(|a, b| {
        b.end
            .partial_cmp(&a.end)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// **Which light's centre the pointer is over.**
///
/// Against the centre handle rather than against the sphere: a sphere is 700
/// yards wide and overlaps half the zone, so picking against the volume would
/// mean every pointer position is inside several lights and none of them is
/// what was aimed at. The handle is where the light *is*, which is the thing
/// being moved.
fn aim(
    mut lights: ResMut<Lights>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !state.editing() || *tool != Tool::Lights {
        lights.hovered = None;
        return;
    }
    // **A held drag keeps what it grabbed.** Without this the hover moves off
    // the handle the moment the light does, and the ring the pointer is
    // dragging stops being the ring that is lit.
    if lights.drag.is_some() {
        return;
    }
    // **The panel has the pointer wherever the viewport does not**, which is
    // the shell's own question and not one to re-derive — it is what keeps a
    // drag inside the form from also aiming here, and it covers the menus and
    // tooltips egui opens over the world as well as the four docked panels.
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        lights.hovered = None;
        return;
    }
    let Some((origin, direction)) = crate::pick::ray(&windows, &camera) else {
        lights.hovered = None;
        return;
    };
    // **The handle's radius in yards at the light's own distance**, so the
    // target is `HANDLE_PIXELS` wide on screen wherever it is. Derived from the
    // camera's vertical field of view and the window's height rather than
    // written down, since both change with the window.
    let Ok(window) = windows.single() else { return };
    let Ok((camera, _)) = camera.single() else {
        return;
    };
    let height = window.height().max(1.0);
    let fov = match camera.clip_from_view().to_cols_array()[5] {
        // The projection's [1][1] is `1 / tan(fov / 2)`, which is what turns a
        // pixel height into an angle without asking the projection what kind it
        // is.
        y if y.abs() > f32::EPSILON => y,
        _ => return,
    };
    // **The centre first, over every light, and only then the rings.** A ring
    // is a 700-yard circle and a centre is a point, so a centre that happened
    // to sit on another light's ring would be unreachable if the two competed
    // on distance alone. Ranking the centres ahead of the rings makes the
    // thing that is harder to hit the one that wins.
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
        // one — which after `collect`'s sort is the smaller sphere.
        if best.is_none_or(|(best, _)| along <= best) {
            best = Some((along, mark.id));
        }
    }
    if let Some((_, id)) = best {
        lights.hovered = Some((id, Dragging::Centre));
        return;
    }

    // **A ring is picked where the pointer's ray meets the light's own
    // plane**, not against the circle in three dimensions. The ring is drawn
    // flat at the light's height, so the plane is where it is; comparing the
    // distance from the centre against the radius is then the whole test, and
    // it behaves the same at every camera pitch.
    //
    // Only the chosen light's rings are grabbable. Map 0 has eighty lights and
    // their rings cross everywhere; a pointer moved across the map would
    // otherwise pass through a dozen of them and each one would offer to be
    // resized. The one whose row is open is the one being edited.
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

/// Escape disarms New light.
fn disarm(
    mut lights: ResMut<Lights>,
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    tool: Res<Tool>,
) {
    if *tool != Tool::Lights || wants.wants_keyboard_input() {
        return;
    }
    if lights.armed && keys.just_pressed(KeyCode::Escape) {
        lights.armed = false;
    }
}

/// **Choose the light under the pointer, and open its row.**
fn press(
    mut lights: ResMut<Lights>,
    buttons: Res<ButtonInput<MouseButton>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    mut session: Option<ResMut<EditSession>>,
    mut browser: Option<ResMut<super::tables::Browser>>,
    cursor: Res<crate::pick::Cursor>,
    focus: Res<WorldFocus>,
    over: (Res<crate::ui::Viewport>, Res<bevy_egui::input::EguiWantsInput>, Query<&Window>),
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
    // **The press that selects is also the press that starts a move.** One
    // gesture rather than select-then-drag, which is what every other
    // placement tool here does and what a person expects of a handle.
    lights.drag = Some(Drag {
        id,
        what,
        grab: None,
        subject: match what {
            Dragging::Centre => "Light position",
            // **One key for both rings**, so a drag that takes the inner
            // radius past the outer and carries on is still one entry rather
            // than two — see `tables::set_fields`.
            Dragging::Start | Dragging::End => "Light falloff",
        },
    });
    // **The row is opened when the selection changes, or when the panel is
    // somewhere else.**
    //
    // Not on every press: pointing the browser at `Light` each time would
    // throw away a followed reference, and re-grabbing the same light to drag
    // it again is the common case. But "the same light" is not enough on its
    // own — after following one of its `ParamsClear` links the panel is on
    // `LightParams`, and a press on the light you are already holding should
    // bring its own row back rather than leave you looking at something else.
    // **The press opens the row, which is the whole point of the pick.** The
    // browser is pointed at `Light` first: a press in the world while the
    // panel is looking at `LightParams` would otherwise select a light whose
    // row nothing shows.
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

/// **Draw the lights on the open map.**
///
/// Two rings per light, at the radii the row states: the inner one is where it
/// stops applying at full strength and the outer is where it has faded out, and
/// the pair is what a falloff *is*. Flat rings rather than spheres — a sphere
/// at 700 yards surrounds the camera with the ground hidden inside it, and the
/// question being asked is how far across the map the light reaches.
///
/// Everything here is [`Look::Ghosted`]: a light is a thing to aim at, and a
/// ring behind a hill is still part of the falloff.
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
        // **Everything near is marked; only what is chosen is drawn in full.**
        // The chosen light is exempt from the range, because flying to it is
        // what the panel's button does and arriving to nothing would be worse
        // than the marker being far away.
        if !chosen && (mark.at - eye).length() > MARKER_RANGE {
            continue;
        }
        // **Opaque, and warm rather than blue.** A light marker competes with
        // terrain, water and sky, and the first draft was a translucent
        // blue-grey against all three — visible in a screenshot and not at a
        // glance. These are the colours a light is: warm when it is the one
        // being edited, white under the pointer, and a dimmer amber otherwise.
        let colour = match (chosen, under) {
            (true, _) => Color::srgb(1.0, 0.82, 0.25),
            (false, true) => Color::srgb(1.0, 1.0, 1.0),
            (false, false) => Color::srgb(0.95, 0.72, 0.30),
        };
        marker(&mut marks, mark.at, grab_radius(mark.at, eye), colour);

        // **The falloff belongs to the light that is being edited, and to
        // nothing else.** Drawn for all eighty it was a screen full of
        // overlapping circles up to 2,648 yards across, one of which lay flat
        // at z = 0 and projected to a straight line across the window at every
        // camera position. That was the bar.
        if !chosen || !lights.show_falloff {
            continue;
        }
        // **The ring under the pointer is drawn brighter than its own light's
        // other one**, which is what says which of the two a press will take.
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
        // **One ring per radius, flat on the ground, and nothing culled.**
        //
        // A falloff is a sphere, and it was drawn as one for a while; at 2,648
        // yards the camera is usually inside the ball and it reads as arcs
        // leaving the window. The footprint is the plane a drag moves in and
        // the plane a zone is measured on, which is the question being asked.
        //
        // **Nothing here clips, caps or fades a ring, and that is deliberate.**
        // A straight band across the whole window used to appear with certain
        // lights selected, and four different culls were added here to remove
        // it — by distance, by what was behind the camera, by the camera's
        // height above the ring, by depression angle. None changed a pixel of
        // it, and each one cut something real. The band was never this
        // geometry: it was the present pass drawing every gizmo a second time,
        // orthographically, at one world unit to the window — see
        // `vale_client::render::present::PRESENT_LAYER`. A ring at world
        // z = 0 is Bevy y = 0, which that camera put on the centre row.
        circle(&mut marks, mark.at, mark.start, inner);
        circle(&mut marks, mark.at, mark.end, outer);
    }
}

/// **How wide a light's handle is in yards, at the distance it is being seen
/// from.**
///
/// The pick uses [`HANDLE_PIXELS`] so that a light is the same size to the
/// pointer wherever it is; the marker is drawn at the same width so that what
/// you can click is what you can see. Approximated off the distance alone
/// rather than through the projection, because a marker is a decoration and
/// being a pixel or two out at the edge of the screen costs nothing — the pick
/// itself does the exact version.
fn grab_radius(at: Vec3, eye: Vec3) -> f32 {
    const AT_A_YARD: f32 = 0.020;
    ((at - eye).length() * AT_A_YARD).clamp(MARKER_SMALLEST, MARKER_LARGEST)
}

/// How small and how large a marker may be drawn, in yards.
///
/// **The upper bound is what the falloff rings did not have.** An apparent size
/// that grows with distance and is not capped is a thing that fills the window
/// once the camera is far enough away, which is the shape of the bar that used
/// to run across it. Named so the test checking the clamp reads the same
/// numbers the drawing does.
const MARKER_SMALLEST: f32 = 0.6;
const MARKER_LARGEST: f32 = 22.0;

/// **A light, as a thing standing in the world.**
///
/// A ball at the position with a line dropping under it. The ball is the
/// same size on screen wherever it is, like a doodad's handle; the drop is what
/// gives it a height, and without it a light in the air and a light on the
/// ground are the same two pixels.
fn marker(marks: &mut Marks, at: Vec3, radius: f32, colour: Color) {
    let head = Vec3::from(axes::to_bevy(at.to_array()));
    marks.sphere(head, radius, colour, Look::Ghosted);
    let foot = Vec3::from(axes::to_bevy([at.x, at.y, at.z - DROP]));
    // Dimmer than the ball: the drop says where the light is *over*, and it
    // should not compete with the thing it is pointing at.
    marks.line(foot, head, colour.with_alpha(0.55), Look::Ghosted);
}

/// One flat circle at the light's height, in the world's own axes.
///
/// Drawn as a run of segments, each [`Marks::line`] thick at its own distance,
/// rather than as one ring of a single thickness: a falloff is up to 2,648
/// yards across, and a thickness right for the near side is a thread or a bar
/// on the far side.
fn circle(marks: &mut Marks, at: Vec3, radius: f32, colour: Color) {
    // **`is_finite` as well as positive.** A NaN fails every comparison, so
    // `radius <= 0.0` alone lets one through and every point of the circle
    // becomes NaN — which is a line drawn somewhere undefined rather than no
    // line at all.
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

/// **Move the light under a held button, and write it as one undo entry.**
///
/// The write goes through [`super::tables::set_fields`] so that a drag across
/// the map is one entry rather than two per frame: the centre moves
/// `InternalX` and `InternalZ` together, and taking back half a movement is not
/// something anyone wants.
///
/// The position is written into the file's own representation on the way, by
/// [`vale_assets::world::adt::placement_from_world`]. That direction is the
/// one loading the game's tiles never exercises, so it is called rather than
/// restated — a second reading of the swap is a light written a hundred yards
/// from where it was dropped.
fn drag(
    mut lights: ResMut<Lights>,
    buttons: Res<ButtonInput<MouseButton>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    time: Res<Time>,
    windows: Query<&Window>,
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
        // The light left the list — another map, or an undo that removed the
        // row. Let go rather than writing into whatever is at that index now.
        lights.drag = None;
        return;
    };
    let Some(session) = session.as_mut() else {
        return;
    };
    // **The plane at the light's own height**, not the ground: a light 200
    // yards up is dragged around at 200 yards up, and one under the terrain
    // stays under it. See [`Dragging::Centre`].
    let Some(at) = crate::pick::on_plane(mark.at.z, &windows, &camera) else {
        return;
    };
    // **The first frame measures the grab and writes nothing.** The pointer
    // has not moved since the press, so there is nothing to write; what it is
    // for is the frames after, where the light keeps the offset it was taken
    // hold of at instead of snapping its centre under the cursor.
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
            super::tables::set_fields(
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
            // **The grab is not applied to a radius.** It is a vector between
            // two points on the plane and a radius is a length; adding it
            // would make a ring grabbed on its far side resize backwards. The
            // press is within a handle's width of the ring by construction, so
            // the jump it prevents is at most that.
            let radius = (at - mark.at).truncate().length();
            let field = match held.what {
                Dragging::Start => lf::FALLOFF_START,
                _ => lf::FALLOFF_END,
            };
            super::tables::set_fields(
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
    // **The list is rebuilt now rather than next frame.** `collect` keys on the
    // session's table revision, which this write has just moved, so leaving it
    // would draw the ring where the light was before this frame's drag — one
    // frame of lag on the only thing the drag shows.
    lights.built = None;
}

/// **Take the camera to a light that was asked for.**
///
/// The height is set as well as the two horizontal coordinates, and
/// `wants_the_ground` is cleared with it: a light's centre runs from 234 yards
/// under the ground to 763 above it, and dropping the focus to the terrain
/// would put the camera somewhere the row does not name. That is the one case
/// in this editor where the ground is not what the camera wants.
fn fly(mut lights: ResMut<Lights>, mut camera: ResMut<crate::camera::EditorCamera>) {
    let Some(at) = lights.fly_to.take() else {
        return;
    };
    camera.target = at;
    camera.wants_the_ground = false;
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// **A row with no falloff is the map's default and has no place.**
    ///
    /// It stays in the list — it is the row the whole map is lit by, and the
    /// most worth editing — and `draw` leaves it out, because its coordinates
    /// are not read and a ring at the corner of the map would be a ring that
    /// means nothing.
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
        // …and a light that is not on this map answers nothing rather than
        // answering with whatever is at that position now.
        lights.marks.retain(|mark| mark.id != 2);
        assert_eq!(lights.chosen(), None);
    }

    /// A fly-to is a request that is consumed once.
    ///
    /// Two writers on the camera is the fault this avoids: `camera::drive` runs
    /// every frame, so a standing "look here" would fight every drag.
    #[test]
    fn a_fly_to_is_asked_for_once() {
        let mut lights = Lights::default();
        assert_eq!(lights.fly_to, None);
        lights.fly_to(&mark(1, 700.0));
        assert!(lights.fly_to.is_some());
        assert!(lights.fly_to.take().is_some());
        assert_eq!(lights.fly_to, None);
    }

    /// **The grab offset is what keeps a light from snapping under the
    /// pointer**, and it composes to the identity when the pointer has not
    /// moved.
    ///
    /// The arithmetic is two lines in `drag` and getting it backwards is a
    /// light that jumps by the width of the handle on the first frame of every
    /// drag, in the direction of whichever edge it was grabbed by.
    #[test]
    fn a_grab_keeps_the_light_where_it_was_taken_hold_of() {
        let light = Vec3::new(100.0, 200.0, 50.0);
        // Pressed a little off the centre, as every press is.
        let pressed_at = Vec3::new(104.0, 197.0, 50.0);
        let grab = light - pressed_at;
        // The pointer has not moved: the light must not either.
        let moved = Vec3::new(pressed_at.x + grab.x, pressed_at.y + grab.y, light.z);
        assert_eq!(moved, light);
        // …and a pointer moved by a vector moves the light by the same vector.
        let then = pressed_at + Vec3::new(30.0, -12.0, 0.0);
        let moved = Vec3::new(then.x + grab.x, then.y + grab.y, light.z);
        assert_eq!(moved, light + Vec3::new(30.0, -12.0, 0.0));
    }

    /// **A drag writes the file's own representation, and it round trips.**
    ///
    /// The direction that loading the game's tiles never exercises is the one a
    /// drag uses, so it is checked here: a position converted out and back is
    /// the position it started at, and the scale is the chain's 1/36 of a yard
    /// rather than yards.
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
        // …and stored, the numbers are 36 times larger, which is what makes
        // `Light.dbc`'s columns read as hundreds of thousands.
        let stored = placement[0] / YARDS_PER_UNIT;
        assert!((stored - placement[0] * 36.0).abs() < 0.5);
    }

    /// The two rings share one gesture key and the centre has its own.
    ///
    /// A drag that carries the inner radius out past the outer one and keeps
    /// going is one movement and should be one entry to undo; a move and a
    /// resize are two different things and should not fold together just
    /// because they happened within the gesture window.
    #[test]
    fn the_two_rings_undo_as_one_movement_and_a_move_is_its_own() {
        let key = |what: Dragging| match what {
            Dragging::Centre => "Light position",
            Dragging::Start | Dragging::End => "Light falloff",
        };
        assert_eq!(key(Dragging::Start), key(Dragging::End));
        assert_ne!(key(Dragging::Centre), key(Dragging::Start));
    }

    /// **A ring is picked by how far the pointer is from the radius**, and the
    /// nearer of the two wins.
    ///
    /// The arithmetic `aim` runs, with the ray taken out of it. The case that
    /// matters is a pointer between two close rings: it must take the one it is
    /// nearer to, not the first one tested.
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
    /// `collect` re-sorts by radius on every edit and a drag *is* an edit, so an
    /// index taken at the press names something else by the next frame. This is
    /// the property that makes that safe.
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

    /// **A circle with a radius that is not a positive number is not drawn.**
    ///
    /// `radius <= 0.0` alone lets a NaN through, because every comparison
    /// against NaN is false — and a NaN radius makes every point of the circle
    /// NaN, which is a line drawn somewhere undefined rather than no line.
    #[test]
    fn a_circle_needs_a_real_radius() {
        let drawable = |radius: f32| radius > 0.0 && radius.is_finite();
        assert!(drawable(700.0));
        assert!(!drawable(0.0));
        assert!(!drawable(-1.0));
        assert!(!drawable(f32::NAN));
        assert!(!drawable(f32::INFINITY));
    }

    /// **A marker is the size the pointer can grab it at**, so what is on
    /// screen is what can be clicked, and it never grows without bound.
    ///
    /// The clamp is what the bar taught: an unbounded size in yards is a thing
    /// that fills the window when the camera is far from it.
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
        // Absurd distances stay clamped rather than filling the window, which
        // is the failure the falloff rings had.
        assert_eq!(
            grab_radius(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0e6)),
            MARKER_LARGEST
        );
    }

    /// **Only the chosen light's falloff is drawn**, which is the rule that
    /// removed the bar.
    ///
    /// Map 0 has 80 positional lights; two of their outer radii are 2,005 and
    /// 2,648 yards and 38 of them lie flat at height zero, so a flat circle per
    /// light projected to a straight line across the whole window from most
    /// camera positions. A marker per light is drawn either way — that is what
    /// makes one clickable without the list.
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
        // …and the switch turns off even the chosen one's.
        let off = Lights {
            show_falloff: false,
            ..lights
        };
        assert!(!(off.selected == Some(2) && off.show_falloff));
    }

    /// **A press re-opens the light's row only when the panel is not already
    /// showing it**, which is what lets a followed reference survive.
    ///
    /// The rule is `already && showing_it`, and both halves earn their place.
    /// Without `showing_it`, a press on a light you are already holding leaves
    /// the panel on whatever you followed to — you clicked the light and its
    /// numbers did not come back. Without `already`, every press re-points the
    /// browser, which is the fault that made the five `ParamsClear` links
    /// appear to do nothing: the follow landed and was undone on the next
    /// frame.
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
