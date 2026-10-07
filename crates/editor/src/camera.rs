//! The free camera, which is the one control a client has no use for.
//!
//! ## It drives the client's own rig rather than the transform
//!
//! `vale_client::world::camera::CameraRig` is what the whole client means by
//! "where the camera is": `place` builds the view matrix from it, `render::sky`
//! asks it whether the eye is under water, and `render::glue` aims through it.
//! Writing a `Transform` directly would move the picture and leave every one of
//! those reading the old position. So this writes the rig, and the client's
//! `place` puts the camera where the rig says, exactly as it does in a session.
//!
//! The rig is an orbit, not a flight: a focus point on the ground, a yaw, a
//! pitch and a distance. That is what the reference client's camera is and it is
//! also the right shape for an editor, because the point being orbited is the
//! point being edited.
//!
//! ## Three writers, and the order between them
//!
//! `world::camera::orbit` writes the rig from the mouse and from the interface's
//! bindings, and `world::camera::collide` writes `CameraRig::reach` from
//! `CameraRig::distance` afterwards. This runs between them and is ordered
//! against both, because both faults that ordering removes were reported as one:
//! "the camera jerks back and forth while painting".
//!
//! * **After `orbit`**, so the editor's yaw and pitch are what the frame draws
//!   rather than whichever of the two ran last.
//! * **Before `collide`**, and writing `distance` rather than `reach`. `reach`
//!   is not an input: `collide` derives it from `distance` every frame, so a
//!   host that set `reach` had its value kept on the frames `collide` ran first
//!   and thrown away on the frames it ran second. That is one frame of the
//!   editor's zoom and the next of the client's default, alternating.
//!
//! ## …and the pointer
//!
//! A left-drag is 1.12's own steer, so the client armed a mouse-look on the
//! press that starts a brush stroke: the view turned, and
//! `ui::cursor::hide_while_steering` locked the pointer, which froze
//! `Window::cursor_position` and pinned every pick to the pixel the press landed
//! on. The world's gestures are claimed through
//! `world::camera::PointerTaken` while the editor is driving, which is the same
//! claim the interface already makes for a press that lands on a frame. That
//! claim is written by [`crate::playtest`] rather than here, because it is one
//! of three switches that all follow the same state and they belong together.
//!
//! ## …and the wheel, which egui could not be asked about
//!
//! The panels' lists are scroll areas, and a wheel notch over one was moving the
//! list **and** zooming the camera: the world slid back while the list slid
//! down, which reads as the whole view lurching every time you look for a
//! texture. The guard was `EguiWantsInput::wants_pointer_input`, which is
//! egui's own `is_using_pointer() || (over_egui && !any_down())` — and its
//! `over_egui` term is `is_pointer_over_area`, which for a widget on the
//! background layer answers from a rectangle bevy_egui never writes. See
//! [`crate::ui`], where both of egui's answers and why neither works are set
//! out.
//!
//! So this asks the shell, through [`crate::ui::over_the_world`], exactly as
//! every tool does — and it asks it about **every** pointer gesture rather than
//! only the wheel, because the same argument covers a right-drag that started on
//! a panel. The drag itself is remembered once it is accepted, on the rule this
//! crate keeps arriving at: *a drag belongs to where it began*, so a look that
//! wanders over the inspector keeps turning.

use crate::playtest::Playtest;
use vale_client::world::camera::CameraRig;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
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
    /// Yards the eye is *asked* to sit from the focus.
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

/// Radians of pitch per pixel of drag.
const LOOK_RATE: f32 = 0.005;

/// How far from level the pitch may go. Short of the poles, where a camera
/// looking straight down has no defined roll.
const PITCH_LIMIT: f32 = 1.5;

/// Yards the closest and furthest the eye may be asked to sit from the focus.
///
/// **The far end is half the open block** and not a number of its own — see
/// [`crate::OPEN_BLOCK`]. It was 400, which was the right answer for the 3x3
/// and is a fifth of the ground this session now streams: a wheel that stops at
/// 400 yards cannot be pulled back far enough to see the block it is editing,
/// which is the whole of what a wider reach is for.
const DISTANCE: std::ops::RangeInclusive<f32> = 5.0..=(crate::OPEN_BLOCK / 2.0);

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EditorCamera>().init_resource::<TopDown>().add_systems(
            Update,
            (fly, drive)
                .chain()
                // **After the client's own steer and before its collision** —
                // see the module comment, where what each of the two orderings
                // removes is written down. Both were the same report.
                .after(vale_client::world::camera::orbit)
                .before(vale_client::world::camera::collide)
                // **Before the client puts the camera where the rig says**, so
                // the frame that reads a key is the frame that moves. Running
                // after `place` would draw every frame from the rig as it was
                // before this frame's input, which at a sprint is two yards of
                // lag on a camera that is being flown by hand.
                .before(vale_client::world::camera::place),
        );
    }
}

/// Read the keyboard and the mouse.
///
/// **`pub` so [`crate::playtest`] can order itself in front of it.** The state
/// this reads is written in the same schedule.
#[allow(clippy::too_many_arguments)]
pub fn fly(
    mut camera: ResMut<EditorCamera>,
    state: Res<Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    // Whether the right-drag now in progress was started over the world.
    mut steering: Local<bool>,
    time: Res<Time>,
    mut top: ResMut<TopDown>,
) {
    // **The editor drives while it is editing, and not one frame longer.** It
    // used to be `session.active.is_none()`, which is also true on the login
    // screen: a password typed there flew the camera, because `w`, `a`, `s` and
    // `d` are half the letters in most of them. `Playtest` is the same condition
    // with that case in it — see [`crate::playtest`], which also writes
    // `PointerTaken` from the same state.
    camera.active = state.editing();
    if !camera.active {
        return;
    }
    // A key typed into one of the panels' fields belongs to the panel.
    if wants.wants_keyboard_input() {
        return;
    }
    // …and so does a wheel notch over one, and a drag that began on one. The
    // shell's own answer and not egui's — see the module comment.
    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);

    // **A drag belongs to where it began**, so the look is armed on the press
    // and answered wherever the pointer has got to. Asking per frame instead is
    // a camera that starts turning the moment the pointer crosses back into the
    // viewport with the button still down.
    if buttons.just_pressed(MouseButton::Right) {
        *steering = in_world;
    }
    if !buttons.pressed(MouseButton::Right) {
        *steering = false;
    }

    // The map view: north is at the top of the picture, so the right-drag
    // moves the map with the pointer and the keys move along the compass.
    if top.on {
        top_down(&mut camera, &mut top, &keys, &motion, &scroll, &windows, &time, in_world, *steering);
        return;
    }

    if *steering {
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

    // …and nothing below this line is the pointer's, so a pointer over a panel
    // does not stop the keys: `W` still flies while the model list is under the
    // cursor, which is what a person scrolling a list and flying at the same
    // time expects.

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
/// The right-drag moves the focus by the pointer's movement at the picture's
/// scale, so the ground under the pointer stays under it. The keys move along
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
    motion: &AccumulatedMouseMotion,
    scroll: &AccumulatedMouseScroll,
    windows: &Query<&Window>,
    time: &Time,
    in_world: bool,
    steering: bool,
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
    camera.wants_the_ground = true;
}

/// …and write it onto the rig the whole client reads.
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
    // **`distance` and not `reach`** — see the module comment. `reach` is
    // `collide`'s output and writing it is writing to something that is about to
    // be recomputed.
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
