//! The editor's free camera, a control the client does not have.
//!
//! ## The camera writes the client's `CameraRig`, not a `Transform`
//!
//! `vale_client::world::camera::CameraRig` holds the camera position for the
//! whole client: `place` builds the view matrix from it, `render::sky` asks it
//! whether the eye is under water, and `render::glue` aims through it. Writing a
//! `Transform` directly would move the picture while every one of those systems
//! read the old position. So this module writes the rig, and the client's
//! `place` puts the camera where the rig says, as it does in a session.
//!
//! The rig is an orbit, not a flight: a focus point on the ground, a yaw, a
//! pitch and a distance. The 1.12.1 client's camera has the same shape. It also
//! suits an editor, because the point being orbited is the point being edited.
//!
//! ## System order against `orbit` and `collide`
//!
//! `world::camera::orbit` writes the rig from the mouse and from the interface's
//! bindings, and `world::camera::collide` writes `CameraRig::reach` from
//! `CameraRig::distance` afterwards. This module runs between them and is
//! ordered against both. Without either ordering the camera jerked back and
//! forth while painting.
//!
//! * After `orbit`, so the frame draws the editor's yaw and pitch rather than
//!   those of whichever system ran last.
//! * Before `collide`, and writing `distance` rather than `reach`. `reach` is
//!   not an input: `collide` derives it from `distance` every frame. A host that
//!   set `reach` kept its value on frames where `collide` ran first and lost it
//!   on frames where `collide` ran second, so the zoom alternated between the
//!   editor's value and the client's default from one frame to the next.
//!
//! ## The world's pointer gestures are claimed while the editor drives
//!
//! In the 1.12.1 client a left-drag steers the camera. Without a claim, the
//! press that starts a brush stroke armed a mouse-look: the view turned, and
//! `ui::cursor::hide_while_steering` locked the pointer, which froze
//! `Window::cursor_position` and pinned every pick to the pixel where the press
//! landed. While the editor drives, it claims the world's gestures through
//! `world::camera::PointerTaken`. The interface makes the same claim for a press
//! that lands on a frame. [`crate::playtest`] writes the claim, not this module,
//! because it is one of three switches that follow the same state, and the three
//! are kept together.
//!
//! ## Gestures over a panel are tested with `crate::ui::over_the_world`
//!
//! The panels' lists are scroll areas. A wheel notch over one moved the list and
//! also zoomed the camera, so the world moved back while the list scrolled down.
//! The guard was `EguiWantsInput::wants_pointer_input`, which is egui's
//! `is_using_pointer() || (over_egui && !any_down())`. Its `over_egui` term is
//! `is_pointer_over_area`, which, for a widget on the background layer, answers
//! from a rectangle bevy_egui never writes. [`crate::ui`] sets out both of
//! egui's answers and why neither works.
//!
//! So this module asks the shell, through [`crate::ui::over_the_world`], as
//! every tool does. It asks about every pointer gesture, not only the wheel,
//! because a right-drag that started on a panel has the same problem. An
//! accepted drag is remembered, because a drag belongs to where it began: a look
//! that moves over the inspector keeps turning.

use crate::playtest::Playtest;
use vale_client::world::camera::CameraRig;
use crate::tablet::Pointer;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

/// Where the editor is looking, and how fast it moves.
#[derive(Resource, Debug, Clone)]
pub struct EditorCamera {
    /// The point being orbited, in the world's own axes.
    pub target: Vec3,
    /// Radians about +Z. Zero looks from the north, matching the rig's own.
    pub yaw: f32,
    /// Radians above the horizon.
    pub pitch: f32,
    /// Yards the eye is asked to sit from the focus, before collision.
    ///
    /// The rig's `distance` and not its `reach`: `reach` is what
    /// `world::camera::collide` leaves after shortening this by whatever is in
    /// the way, and it is an output.
    pub distance: f32,
    /// Yards a second at a walk. Held shift is [`SPRINT`] times this.
    pub speed: f32,
    /// Whether the editor is driving at all. False during a playtest, in both
    /// of its stages — see [`Playtest::editing`].
    pub active: bool,
    /// Whether the focus should be put on the ground as soon as the ground under
    /// it is known.
    ///
    /// Set by every jump and by the start position, because both of those name a
    /// place on the map and not a height, and the tile they land on is not parsed
    /// for several frames afterwards. Cleared by
    /// `crate::session::drop_to_the_ground` the moment it can answer, so flying
    /// below the ground afterwards is allowed: a camera pinned to the terrain
    /// could not get under a bridge to look at its underside.
    pub wants_the_ground: bool,
}

impl Default for EditorCamera {
    fn default() -> EditorCamera {
        EditorCamera {
            target: Vec3::ZERO,
            yaw: 0.0,
            // Looking down at the ground from above, which is the framing a
            // terrain tool is used in.
            pitch: 0.9,
            distance: 60.0,
            speed: 60.0,
            active: true,
            wants_the_ground: true,
        }
    }
}

impl EditorCamera {
    /// Put the focus somewhere on the map, and ask for the ground under it.
    ///
    /// The one way anything outside this file moves the camera. It keeps the
    /// yaw, the pitch and the distance, so a jump changes where the camera is
    /// looking and not how.
    pub fn go_to(&mut self, at: Vec2) {
        self.target.x = at.x;
        self.target.y = at.y;
        self.wants_the_ground = true;
    }
}

/// The top-down map view: the world drawn straight down through an
/// orthographic projection with north at the top, as the game's minimap is.
/// The view bar's MAP button switches it; `--map-view` starts with it on.
///
/// While it is on, the editor's camera looks straight down from just above the
/// focus, the wheel sets [`Self::span`], a right-drag moves the map under the
/// pointer, and W, A, S and D move north, west, south and east. Q and E do
/// nothing: the focus is put on the ground under it every frame, so the slab
/// of world that is drawn follows the terrain. The tools work as they do in
/// the perspective view, because the pick casts its ray through the same
/// camera.
///
/// The eye stays one yard above the focus and the projection draws from
/// [`ABOVE`] yards above it to [`BELOW`] yards below. Bevy culls a doodad by
/// its distance from the eye (`VisibilityRange`), so an eye far above the
/// ground would remove every small doodad from the picture; an orthographic
/// view does not need the eye to be far away to see a wide area.
#[derive(Resource, Debug)]
pub struct TopDown {
    pub on: bool,
    /// Yards from the top of the picture to the bottom.
    pub span: f32,
    /// What the view replaced while it is on, restored when it is turned off.
    held: Option<Held>,
}

impl TopDown {
    /// The map view switched on, for `--map-view`.
    pub fn shown() -> TopDown {
        TopDown {
            on: true,
            ..TopDown::default()
        }
    }
}

impl Default for TopDown {
    fn default() -> TopDown {
        TopDown {
            on: false,
            span: vale_assets::world::adt::TILE_SIZE,
            held: None,
        }
    }
}

/// The perspective projection and the world switches the map view turns off.
#[derive(Debug)]
struct Held {
    projection: Projection,
    fog: bool,
    sky_dome: bool,
    stars: bool,
    celestial: bool,
    weather: bool,
}

/// Yards above the focus the map view still draws. Peaks higher than this
/// above the ground under the focus are cut off.
const ABOVE: f32 = 600.0;

/// Yards below the focus the map view still draws.
const BELOW: f32 = 1500.0;

/// The depth of the slab the map view draws. The pick follows its ray this far
/// in an orthographic view, because the ray starts at the top of the slab; see
/// `pick::range`.
pub const MAP_DEPTH: f32 = ABOVE + BELOW;

/// The narrowest and widest [`TopDown::span`]: from about five chunks to the
/// whole open block.
const SPAN: std::ops::RangeInclusive<f32> = 20.0..=crate::OPEN_BLOCK;

/// Switch the world camera between its perspective projection and the map
/// view's orthographic one, and the sky, fog and weather off and on with it.
///
/// Runs in [`crate::playtest`]'s chain after the playtest state is decided and
/// before `keep_view_settings` stores the editor's view settings: a playtest
/// started with the map view on turns it off here first, so the settings
/// stored for editing are the ones the person had before the map view, and
/// the playtest starts in perspective.
pub fn look_down(
    state: Res<Playtest>,
    mut top: ResMut<TopDown>,
    mut world: ResMut<vale_client::render::tuning::WorldTuning>,
    mut cameras: Query<&mut Projection, With<vale_client::world::camera::WorldCamera>>,
) {
    if !state.editing() && top.on {
        top.on = false;
    }
    let Ok(mut projection) = cameras.single_mut() else {
        return;
    };
    match (top.on, top.held.is_some()) {
        (true, false) => {
            top.held = Some(Held {
                projection: projection.clone(),
                fog: world.fog,
                sky_dome: world.sky_dome,
                stars: world.stars,
                celestial: world.celestial,
                weather: world.weather,
            });
            switch_off_for_map_view(&mut world);
            *projection = orthographic(top.span);
        }
        (true, true) => {
            // Written only when the span changed: a `Projection` marked changed
            // re-derives the frustum and the view uniforms.
            if let Projection::Orthographic(current) = &*projection {
                let changed = !matches!(
                    current.scaling_mode,
                    bevy::camera::ScalingMode::FixedVertical { viewport_height }
                        if (viewport_height - top.span).abs() < 1e-3
                );
                if changed {
                    *projection = orthographic(top.span);
                }
            }
        }
        (false, true) => {
            let held = top.held.take().expect("checked above");
            *projection = held.projection;
            world.fog = held.fog;
            world.sky_dome = held.sky_dome;
            world.stars = held.stars;
            world.celestial = held.celestial;
            world.weather = held.weather;
        }
        (false, false) => {}
    }
}

/// Turn off the world switches the map view has no use for.
///
/// The sky, stars, sun and moons are drawn on a dome around the eye and show
/// nothing useful looking straight down. Fog is measured from the eye and would
/// grey the picture by distance from the focus. Weather is drawn in a volume
/// around the eye. The view bar's echo uses this too, so that `--map-view`
/// does not also print these five as `--without`.
pub fn switch_off_for_map_view(world: &mut vale_client::render::tuning::WorldTuning) {
    world.fog = false;
    world.sky_dome = false;
    world.stars = false;
    world.celestial = false;
    world.weather = false;
}

/// The map view's projection for a picture `span` yards tall.
fn orthographic(span: f32) -> Projection {
    Projection::Orthographic(OrthographicProjection {
        near: -ABOVE,
        far: BELOW,
        scaling_mode: bevy::camera::ScalingMode::FixedVertical { viewport_height: span },
        ..OrthographicProjection::default_3d()
    })
}

/// How much faster a held shift is.
pub const SPRINT: f32 = 6.0;

/// Radians of yaw or pitch per physical pixel of drag.
const LOOK_RATE: f32 = 0.005;

/// How far from level the pitch may go. Short of the poles, where a camera
/// looking straight down has no defined roll.
const PITCH_LIMIT: f32 = 1.5;

/// Yards the closest and furthest the eye may be asked to sit from the focus.
///
/// The far end is half of [`crate::OPEN_BLOCK`], not a separate number. It was
/// 400 yards, which suited a 3x3 block of tiles and is a fifth of the ground the
/// session now streams. At 400 yards the camera cannot pull back far enough to
/// see the block being edited.
const DISTANCE: std::ops::RangeInclusive<f32> = 5.0..=(crate::OPEN_BLOCK / 2.0);

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EditorCamera>().init_resource::<TopDown>().add_systems(
            Update,
            (fly, drive)
                .chain()
                // After the client's `orbit` and before its `collide`. The
                // module comment says which fault each of the two orderings
                // removes.
                .after(vale_client::world::camera::orbit)
                .before(vale_client::world::camera::collide)
                // Before `place`, which puts the camera where the rig says, so
                // the frame that reads a key is the frame that moves. After
                // `place`, every frame would be drawn from the rig as it was
                // before this frame's input: two yards of lag at a sprint.
                .before(vale_client::world::camera::place),
        );
    }
}

/// Read the keyboard, the wheel and the [`Pointer`], and move the camera.
///
/// `pub` so [`crate::playtest`] can order itself in front of it. The state this
/// reads is written in the same schedule.
#[allow(clippy::too_many_arguments)]
pub fn fly(
    mut camera: ResMut<EditorCamera>,
    state: Res<Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    // The pointer's motion in physical pixels and its right button: raw mouse
    // motion and the right mouse button, or, while a tablet pen is in range,
    // the cursor's change of position and the pen's barrel button. See
    // [`crate::tablet`].
    motion: Res<Pointer>,
    scroll: Res<AccumulatedMouseScroll>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    // Whether the right-drag now in progress was started over the world.
    mut steering: Local<bool>,
    time: Res<Time>,
    mut top: ResMut<TopDown>,
    // How far the pointer has moved since the right button went down, in
    // physical pixels. Under `crate::context::CLICK_SLOP` the press is a click
    // that asks for the right-click menu, and turns nothing.
    mut travel: Local<f32>,
    mut menu: ResMut<crate::context::ContextMenu>,
) {
    // The editor drives only while it is editing. The test was
    // `session.active.is_none()`, which is also true on the login screen, so
    // the `w`, `a`, `s` and `d` in a password typed there moved the camera.
    // `Playtest` is the same condition with the login screen excluded. See
    // [`crate::playtest`], which also writes `PointerTaken` from the same
    // state.
    camera.active = state.editing();
    if !camera.active {
        return;
    }
    // A key typed into one of the panels' fields belongs to the panel. Only
    // the keys: bevy_egui reports keyboard input as wanted whenever any egui
    // widget has focus, and a button that was clicked keeps the focus, so
    // returning here also stopped the wheel and the right-drag after a press
    // of the view bar's MAP button, until a click in the world took the focus
    // away.
    let typing = wants.wants_keyboard_input();
    // A wheel notch over a panel, and a drag that began on one, also belong to
    // the panel. `in_world` is the shell's answer, not egui's; see the module
    // comment.
    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);

    // A drag belongs to where it began, so the look is armed on the press and
    // followed wherever the pointer goes. Asking every frame instead would start
    // the camera turning when the pointer crossed back into the viewport with
    // the button still down.
    if motion.right_pressed {
        *steering = in_world;
        *travel = 0.0;
    }
    if *steering {
        *travel += motion.delta.length();
    }
    // A press and release that barely moved is a click, which opens the
    // right-click menu at the pointer. See `crate::context`.
    if motion.right_released && *steering && *travel <= crate::context::CLICK_SLOP {
        if let Some(at) = windows.single().ok().and_then(Window::cursor_position) {
            menu.pending = Some(at);
        }
    }
    if !motion.right {
        *steering = false;
    }
    // The view turns, or the map pans, only once the press has become a drag.
    let looking = *steering && *travel > crate::context::CLICK_SLOP;

    // The map view: north is at the top of the picture, so the right-drag
    // moves the map with the pointer and the keys move along the compass.
    if top.on {
        top_down(&mut camera, &mut top, &keys, &motion, &scroll, &windows, &time, in_world, looking, typing);
        return;
    }

    if looking {
        let delta = motion.delta;
        camera.yaw -= delta.x * LOOK_RATE;
        camera.pitch = (camera.pitch + delta.y * LOOK_RATE).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    // The wheel zooms, and under any modifier it is the tool's rather than the
    // camera's — see [`crate::tools::Wheel`], which is the one list of what each
    // modifier puts on it and is read here so the two cannot disagree.
    if in_world && scroll.delta.y != 0.0 && crate::tools::Wheel::held(&keys).is_none() {
        let step = 1.0 - scroll.delta.y * 0.1;
        camera.distance = (camera.distance * step).clamp(*DISTANCE.start(), *DISTANCE.end());
    }

    // Nothing below this line reads the pointer, so a pointer over a panel does
    // not stop the keys: `W` still flies while the model list is under the
    // cursor, and a person can scroll a list and fly at the same time.

    if typing {
        return;
    }
    // The horizontal frame the keys move in. The eye sits at `yaw` from the
    // focus, so the direction the view runs in is the opposite of it.
    let (sin, cos) = camera.yaw.sin_cos();
    let forward = Vec3::new(-cos, -sin, 0.0);
    let right = Vec3::new(-sin, cos, 0.0);

    let mut step = Vec3::ZERO;
    for (key, direction) in [
        (KeyCode::KeyW, forward),
        (KeyCode::KeyS, -forward),
        (KeyCode::KeyD, right),
        (KeyCode::KeyA, -right),
        (KeyCode::KeyE, Vec3::Z),
        (KeyCode::KeyQ, -Vec3::Z),
    ] {
        if keys.pressed(key) {
            step += direction;
        }
    }
    if step != Vec3::ZERO {
        let speed = if keys.pressed(KeyCode::ShiftLeft) {
            camera.speed * SPRINT
        } else {
            camera.speed
        };
        camera.target += step.normalize() * speed * time.delta_secs();
    }
}

/// The map view's half of [`fly`].
///
/// The right-drag moves the focus by the pointer's movement in physical pixels,
/// at the picture's scale, so the ground under the pointer stays under it. The keys move along
/// the compass at a speed proportional to the span, so crossing the picture
/// takes the same time at every zoom. The wheel sets the span. The focus is
/// asked onto the ground every frame, which `session::drop_to_the_ground`
/// answers wherever the ground under it is open; over a hole, or past the open
/// tiles, it keeps its last height.
#[allow(clippy::too_many_arguments)]
fn top_down(
    camera: &mut EditorCamera,
    top: &mut TopDown,
    keys: &ButtonInput<KeyCode>,
    motion: &Pointer,
    scroll: &AccumulatedMouseScroll,
    windows: &Query<&Window>,
    time: &Time,
    in_world: bool,
    steering: bool,
    // Whether a panel has the keyboard; the keys are its then.
    typing: bool,
) {
    // Yards per physical pixel. The world camera renders the whole window, and
    // the span is the picture's full height.
    let height = windows
        .single()
        .map(|window| window.physical_height() as f32)
        .unwrap_or(1.0)
        .max(1.0);
    let yards_per_pixel = top.span / height;
    if steering {
        // North (+X) is up and west (+Y) is left, so a drag to the right moves
        // the focus west and a drag down moves it north.
        camera.target.x += motion.delta.y * yards_per_pixel;
        camera.target.y += motion.delta.x * yards_per_pixel;
    }

    if in_world && scroll.delta.y != 0.0 && crate::tools::Wheel::held(keys).is_none() {
        let step = 1.0 - scroll.delta.y * 0.1;
        top.span = (top.span * step).clamp(*SPAN.start(), *SPAN.end());
    }

    camera.wants_the_ground = true;
    if typing {
        return;
    }
    let mut step = Vec3::ZERO;
    for (key, direction) in [
        (KeyCode::KeyW, Vec3::X),
        (KeyCode::KeyS, -Vec3::X),
        (KeyCode::KeyA, Vec3::Y),
        (KeyCode::KeyD, -Vec3::Y),
    ] {
        if keys.pressed(key) {
            step += direction;
        }
    }
    if step != Vec3::ZERO {
        // Half the picture's height a second, and a held shift as in the
        // perspective view.
        let mut speed = top.span * 0.5;
        if keys.pressed(KeyCode::ShiftLeft) {
            speed *= SPRINT;
        }
        camera.target += step.normalize() * speed * time.delta_secs();
    }
}

/// Write the [`EditorCamera`] onto the `CameraRig` that the whole client reads.
fn drive(camera: Res<EditorCamera>, top: Res<TopDown>, mut rig: ResMut<CameraRig>) {
    if !camera.active {
        return;
    }
    rig.target = camera.target;
    // The rig lifts its focus to the head height of whatever it is following,
    // and it is following nothing here.
    rig.target_height = 0.0;
    if top.on {
        // Straight down from one yard above the focus, with a yaw of pi, which
        // `world::camera::place` turns into north at the top of the picture.
        // See [`TopDown`] for why the eye is kept close to the ground.
        rig.yaw = std::f32::consts::PI;
        rig.pitch = std::f32::consts::FRAC_PI_2;
        rig.distance = 1.0;
        return;
    }
    rig.yaw = camera.yaw;
    rig.pitch = camera.pitch;
    // `distance`, not `reach`: `reach` is the output of `collide`, which
    // recomputes it from `distance` later in the frame. See the module comment.
    rig.distance = camera.distance;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frame the keys move in is the one the view runs along: forward at a
    /// yaw of zero is south, because the eye sits north of the focus.
    #[test]
    fn forward_is_the_way_the_view_points() {
        let yaw = 0.0f32;
        let (sin, cos) = yaw.sin_cos();
        let forward = Vec3::new(-cos, -sin, 0.0);
        let right = Vec3::new(-sin, cos, 0.0);
        assert_eq!(forward, Vec3::new(-1.0, 0.0, 0.0), "+X is north");
        assert_eq!(right, Vec3::new(0.0, 1.0, 0.0), "+Y is west");
        // …and right is ninety degrees clockwise from forward, seen from above.
        assert!(forward.dot(right).abs() < 1e-6);
    }
}
