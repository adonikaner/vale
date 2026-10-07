//! The third-person camera.
//!
//! The rig is kept in WoW's axes (yaw about +Z, target in world coordinates)
//! and converted once, when [`place`] writes the `Transform`. Keeping it in
//! WoW's axes means a camera bug and an axes bug cannot be confused with each
//! other.

use crate::axes;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

/// Marks the one camera, so the placement system can find it.
#[derive(Component)]
pub struct WorldCamera;

/// Where the camera is looking and from how far.
#[derive(Resource)]
pub struct CameraRig {
    /// What the camera orbits, in WoW world coordinates. This is the character's
    /// feet: a unit's position is its ground point, which is also the point the
    /// terrain lookup and the anticheat use.
    pub target: Vec3,
    /// How far above [`Self::target`] the camera looks, in yards.
    ///
    /// Orbiting the feet puts the head out of frame when zoomed in: at a
    /// distance of two yards the pitch has to be aimed at the ground the
    /// character is standing on, so the character is above the screen. This
    /// lifts the whole rig, the focus and the eye, to the character's camera
    /// anchor near the head, which is the point the 1.12.1 client orbits.
    ///
    /// Set from the model, because a gnome, a tauren and a mounted character do
    /// not share an anchor height; see `EntityModel::anchor` in
    /// `world/entities/mod.rs` and `vale_assets::look::anchor`. Zero until the
    /// local player has a model, which is only the first moment of a session.
    pub target_height: f32,
    /// Radians about +Z (up). Zero looks from the north.
    ///
    /// Absolute, and carried along by the character's own turning. The rig is
    /// not parented to the player: `--view` sets this by hand and the eye maths
    /// reads it directly. `session::follow_player` adds every change in the
    /// character's facing to it, so turning with A/D swings the camera with the
    /// character while a left-drag still looks freely around them. Without
    /// that, the camera stays pointed north while the character turns, and
    /// every turn ends with the character side-on to the view.
    ///
    /// While the right button is held the dependency is reversed: the mouse
    /// writes this, and the character is aimed at `yaw + π` from it.
    /// `follow_player` does not write it for the duration, so the turn is not
    /// counted twice. See [`orbit`] for why the direction matters.
    pub yaw: f32,
    /// Radians above the horizon, clamped away from both poles because a
    /// `looking_at` straight down has no defined roll.
    ///
    /// Negative values are allowed: the eye below the character, looking up.
    /// The 1.12.1 client allows this, and a limit level with the ground
    /// ([`PITCH_LIMIT`]) made the sky above the character impossible to see.
    /// [`collide`] keeps the eye above ground by pulling it in when the ground
    /// gets between it and the head.
    pub pitch: f32,
    /// How far the eye is asked to sit from the focus. The position it gets is
    /// [`Self::eye`], which is this shortened by whatever is in the way.
    ///
    /// The zoom moves this value over time rather than in one step; see
    /// [`Self::glide`].
    pub distance: f32,
    /// Seconds of zoom still to run, positive towards the character and
    /// negative away from it.
    ///
    /// The 1.12.1 client does not move the camera by a whole notch in one
    /// frame. A notch of `CameraZoomIn(amount)` starts a movement of `amount`
    /// yards at `cameraDistanceMoveSpeed` yards per second (8.33 by default,
    /// so one notch takes 120 ms), and [`glide`] runs it a frame at a time.
    /// A further notch the same way lengthens the movement; a notch the other
    /// way cancels what is left and starts its own. See [`zoom`].
    pub glide: f32,
    /// The distance actually used this frame, after collision. Held on the rig
    /// rather than recomputed by each reader for the same reason [`Self::eye`]
    /// is shared: the billboard bones and the view matrix must come from one
    /// eye, or they drift apart.
    pub reach: f32,
}

impl Default for CameraRig {
    fn default() -> Self {
        CameraRig {
            target: Vec3::ZERO,
            target_height: 0.0,
            yaw: 0.0,
            pitch: 0.5,
            distance: 25.0,
            glide: 0.0,
            reach: 25.0,
        }
    }
}

impl CameraRig {
    /// The point the camera looks at: the target lifted to head height.
    ///
    /// Everything that frames the view goes through this rather than through
    /// `target`, so the two cannot disagree about where the middle of the
    /// screen is. A rig whose eye orbits the head and whose `looking_at` is the
    /// feet tilts further off the character the closer the zoom.
    pub fn focus(&self) -> Vec3 {
        self.target + Vec3::new(0.0, 0.0, self.target_height)
    }

    /// The eye position in WoW coordinates, from the orbit angles.
    ///
    /// Shared with [`place`] rather than recomputed, because the billboard bones
    /// in stage 6 need the camera's own axes and they must come from the same
    /// eye the view matrix was built from or they drift apart.
    pub fn eye(&self) -> Vec3 {
        self.at(self.reach)
    }

    /// Where the eye would sit at a given distance. The collision pass asks for
    /// the unobstructed one; everything else wants [`Self::eye`].
    pub fn at(&self, distance: f32) -> Vec3 {
        self.focus()
            + Vec3::new(
                distance * self.pitch.cos() * self.yaw.cos(),
                distance * self.pitch.cos() * self.yaw.sin(),
                distance * self.pitch.sin(),
            )
    }
}

/// The current mouse gesture on the world, decided once and read by systems
/// in three directories.
///
/// A press is not always a click and a drag is not always a look. When each of
/// the three readers decided for itself, the results were:
///
/// * [`orbit`] turned the camera whenever a button was down, wherever the press
///   had landed, so dragging a bag icon or the slider in the options panel
///   swung the view with it. In 1.12 the world is a frame (`WorldFrame`) and
///   the pointer lands on exactly one thing. This client keeps its own pick
///   and has to decline explicitly, in the same way
///   [`crate::interface::target::hover`] does for the pick and
///   `select_on_click` for the click.
/// * [`crate::ui::cursor`] hid the pointer for the right button only, so a
///   left-drag round the character left the arrow in the middle of the screen.
/// * `select_on_click` treated any motion as a drag (`motion.delta !=
///   Vec2::ZERO`). A mouse moves a pixel or two during an ordinary click, so a
///   large fraction of clicks on a mob selected nothing. [`LOOK_SLOP`] exists
///   for this reason.
///
/// The three fields answer different questions. `active` says whether a world
/// gesture is happening now; it turns the camera and hides the pointer.
/// `on_world` and `dragged` describe the current or just-ended gesture, because
/// a release is read a frame after `active` has cleared: `just_released` implies
/// the button is no longer `pressed`.
#[derive(Resource, Default)]
pub struct MouseLook {
    /// A button is down right now and the gesture began on the world.
    pub active: bool,
    /// Where the press landed. Held across the release, so the click that
    /// follows is judged by where it started rather than by where the pointer
    /// is when the button comes up.
    pub on_world: bool,
    /// The pointer has travelled past [`LOOK_SLOP`] since the press, so the
    /// release is a look and never a click. Held across the release for the same
    /// reason `on_world` is.
    pub dragged: bool,
    /// Pixels travelled since the press, summed rather than measured
    /// end-to-end: a drag out and back is still a drag.
    travel: f32,
}

/// Whether the world's mouse gestures belong to something other than the game.
///
/// [`arm_look`] already asks whether the press landed on the interface, because
/// a drag that starts on a bag icon must not swing the view. A host with a
/// pointer tool of its own needs the same check for the same reason: 1.12 steers
/// with the left button, which is also the button a tool that paints or drags
/// in the world is held down with.
///
/// A press the game wrongly treats as a look has two effects. The camera turns,
/// because [`orbit`] steers on it. The pointer is locked, because
/// `ui::cursor::hide_while_steering` grabs it while `MouseLook::active`, so
/// `Window::cursor_position` stops moving and every pick stays at the pixel the
/// press landed on; the tool then acts on one spot however far the mouse
/// travels.
///
/// False in this crate, where nothing else has a claim.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct PointerTaken(pub bool);

/// How far the pointer travels before a press becomes a look rather than a
/// click, in window pixels.
///
/// The same value as the interface's `lua::mouse::DRAG_THRESHOLD`, which is 4.
/// That constant separates a click from a drag on a frame; this one makes the
/// same separation in the world. It is a small screen-space slop chosen by
/// this client, not a value of the 1.12.1 client. With it, a release is a
/// click or a look and never both.
pub const LOOK_SLOP: f32 = 4.0;

/// Decide what the current mouse gesture is, once, before anything acts on it.
///
/// Runs before [`orbit`], before [`crate::interface::target::select_on_click`] and
/// before [`crate::ui::cursor`]'s hide. All three are stated with `.after`,
/// because each of them reads a resource this writes and Bevy will not
/// otherwise sequence them.
pub fn arm_look(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    interface: Res<crate::lua::api::mouse::MouseFocus>,
    taken: Res<PointerTaken>,
    mut look: ResMut<MouseLook>,
) {
    // A gesture starts on the first button down, not on each of them. Pressing
    // left, then right without letting go, is one gesture (both-button
    // autorun), and re-arming on the second press would reset a drag that had
    // already been judged a look.
    let held = buttons.pressed(MouseButton::Left) || buttons.pressed(MouseButton::Right);
    let began = !look.active
        && held
        && (buttons.just_pressed(MouseButton::Left) || buttons.just_pressed(MouseButton::Right));
    if began {
        // Where the press landed decides the whole gesture, and it is asked
        // once. Asking per frame would hand the camera back and forth as the
        // pointer crossed the action bar mid-swing.
        look.on_world = !interface.over_interface && !taken.0;
        look.dragged = false;
        look.travel = 0.0;
        look.active = look.on_world;
    }
    if !held {
        // `on_world` and `dragged` survive: this frame's release still has to be
        // judged, and they are only overwritten by the next press.
        look.active = false;
        return;
    }
    if look.active {
        look.travel += motion.delta.length();
        if look.travel > LOOK_SLOP {
            look.dragged = true;
        }
    }
}

/// How close to vertical the pitch may get, either way.
///
/// Not quite π/2 at the top because `looking_at` straight down has no defined
/// roll and the view rolls unpredictably there; not quite -π/2 at the bottom for
/// the same reason. Between them the camera can be put under the character and
/// aimed at the sky, as in the 1.12.1 client.
pub(crate) const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.02;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraRig>()
            .init_resource::<RenderTuning>()
            .init_resource::<MouseLook>()
            // False by default, so no press is excluded here. See
            // [`PointerTaken`]; its only setter is another host.
            .init_resource::<PointerTaken>()
            .add_systems(Startup, (spawn, stop_clustering))
            // The far plane follows the streamed block. `spawn` reads
            // `TerrainReach` once. A host that widens it afterwards would
            // otherwise have the extra ground loaded and then frustum-culled
            // against a plane that stops at the old block's far corner, which
            // shows as a ring of sky where the new tiles are. Runs only when the
            // resource changes, which in a session is never.
            .add_systems(
                Update,
                follow_reach.run_if(bevy::ecs::schedule::common_conditions::resource_changed::<
                    crate::render::terrain::TerrainReach,
                >),
            )
            // Runs on the first frame as well as on every change, which is what
            // puts `Tonemapping::None` and the window's present mode on a camera
            // that `spawn` built without them.
            .add_systems(Update, (tuning_keys, apply_tuning).chain())
            // Before the character is told where to face. A right-drag turns
            // the camera directly and the character follows the camera
            // (`session::send_input`), so a frame that read the yaw before
            // `orbit` wrote it would send the character last frame's heading.
            // This is the same one-frame skew the `place`-after-`follow_player`
            // ordering below removes, on the other half of the rig.
            .add_systems(
                Update,
                (arm_look, orbit)
                    .chain()
                    // After the bindings, because `orbit` consumes
                    // `Binding::CameraZoom`. Running first would move the eye
                    // on the next frame, and on a key-repeat the zoom would lag
                    // the key.
                    .after(crate::input::bindings::BindingSet)
                    // After the interface has hit-tested this frame's pointer.
                    // `arm_look` asks `MouseFocus` once, on the frame the button
                    // goes down, and that answer decides the whole gesture.
                    // Reading last frame's answer would hand the camera a drag
                    // that started on a panel which had just opened under a
                    // stationary pointer.
                    .after(crate::lua::api::mouse::poll)
                    .before(crate::world::session::send_input),
            )
            // Before [`place`], which puts it before the login screen's own
            // framing: `glue::aim_camera` is `.after(place)` and sets the scene
            // file's angle, which must take precedence while a glue screen is
            // up.
            .add_systems(Update, frame_the_view.before(place))
            .add_systems(
                Update,
                (collide, place)
                    .chain()
                    // After the rig has been pointed at this frame's player.
                    // `follow_player` writes `rig.target` from the interpolated
                    // position. Unordered, `place` could build the view matrix
                    // from last frame's target while the world was drawn at
                    // this frame's, depending on system order. That is a frame
                    // of travel between the character and the camera, 0.13
                    // yards at a run and varying with the frame time, and
                    // because everything else in the shot is stationary it
                    // shows as the character jittering.
                    .after(crate::world::session::follow_player),
            );
    }
}

/// The three frame settings that can be changed while running: MSAA, vsync and
/// sun shadows, on F5, F9 and F10.
///
/// They are runtime switches rather than build options so that one login can
/// compare them at one framing; each trades CPU, GPU and image quality in a
/// ratio that depends on the machine and the scene.
///
/// `Clone` and `PartialEq` are for the panels: each edits a copy and writes the
/// resource back only when something moved, because `ResMut`'s `DerefMut` marks
/// the resource changed whether or not the value did, and [`apply_tuning`]
/// re-inserts the camera's components on every change.
#[derive(Resource, Clone, PartialEq, Eq)]
pub struct RenderTuning {
    /// Off by default, where Bevy's default is 4x. `Camera3d::default()`
    /// carries no `Msaa` and `Msaa::default()` is `Sample4`, so without this
    /// field the camera resolves four samples a pixel over a city's overdraw.
    /// MSAA does not smooth alpha-keyed foliage, where aliasing is worst,
    /// because foliage coverage comes from a `discard`, not from the edge of a
    /// triangle.
    pub msaa: bool,
    /// Wait for the display. On by default, for play. It distorts every
    /// measurement: under vsync a frame is 16.7 ms
    /// or 33.3 ms and nothing in between, so a scene that misses the budget by
    /// one millisecond shows as the frame rate halving, and the wait itself is
    /// indistinguishable from CPU cost in the frame-against-gpu-spans
    /// comparison. F9, and `--tune novsync` from a script.
    pub vsync: bool,
    /// Shadow maps from the sun, which 1.12 does not have.
    ///
    /// The game's own shadows are baked (`MCSH` on the ground) and a blob under
    /// each unit, and this client always draws both. The 1.12 client has
    /// `Textures\ShadowBlob.blp` and a `shadowBias` CVar and nothing resembling
    /// a shadow map. This setting is therefore not a fidelity option; it is an
    /// A/B comparison against a world with sun shadows. It is the most expensive
    /// switch on this list, since a cascade re-draws every caster in the view.
    /// Off, on F10.
    pub sun_shadows: bool,
}

/// Every setting on [`RenderTuning`], once, as `(key, label, accessor)`.
///
/// The same kind of list as [`crate::render::tuning::SWITCHES`] and
/// [`crate::render::overlay::OVERLAYS`], for the reason both of those give in
/// their own files: a panel that writes its checkboxes out by hand drifts from
/// the struct when a setting is added, and a setting no panel draws cannot be
/// reached.
///
/// The key is the function key that writes the same field, so a surface drawing
/// these can say which one without knowing anything about the bindings.
pub const SWITCHES: [(&str, &str, fn(&mut RenderTuning) -> &mut bool); 3] = [
    ("F5", "MSAA", |t| &mut t.msaa),
    ("F9", "vsync", |t| &mut t.vsync),
    ("F10", "sun shadows", |t| &mut t.sun_shadows),
];

impl Default for RenderTuning {
    fn default() -> Self {
        RenderTuning {
            msaa: false,
            vsync: true,
            sun_shadows: false,
        }
    }
}

fn spawn(
    mut commands: Commands,
    tuning: Res<RenderTuning>,
    reach: Res<crate::render::terrain::TerrainReach>,
    frame: Res<crate::render::present::WorldFrame>,
) {
    commands.spawn((
        Camera3d::default(),
        Camera {
            // The world is drawn into an image, not into the window; see
            // `render::present` for why. The image is `Rgba8Unorm`, so the
            // blend adds the game's own bytes, and a full-screen quad on a
            // second camera puts it on screen. It is not a post-processing
            // chain: there is one copy and one transfer function in it. It
            // exists because `bevy_egui` cannot draw into a byte-space target.
            // Before the camera that blits it. See `present`.
            order: 0,
            ..default()
        },
        frame.target(),
        // The encoding of the values in that image, which `WorldFrame`'s
        // format does not state on its own.
        //
        // The format already makes the blend a byte-space add (see
        // `render::present`), and every world shader ends in
        // `atmosphere::to_frame` to write the bytes it expects. This component
        // decides the one value Bevy writes into the target without a
        // fragment shader: the clear. With it, the conversion is
        // `Srgba::from(color)` rather than `LinearRgba::from`, so
        // `render::sky`'s decoded fog band lands as the byte the light table
        // states. Without it the linear value is read as a byte, and the sky
        // clear is drawn at a third of its brightness.
        //
        // It also declares correctly what this camera produces, which matters
        // if the world is ever drawn straight to the window again: Bevy's own
        // upscaling blit reads it and undoes the encode (`SRGB_TO_LINEAR`),
        // which is what `present.wgsl` does by hand.
        bevy::camera::CompositingSpace::Srgb,
        // Far enough to hold the ground that is loaded.
        //
        // Bevy's perspective projection is `perspective_infinite_reverse_rh`, so
        // `far` clips nothing, but it is the far plane of the `Frustum` every
        // mesh is culled against. The default is 1,000 yards, against a
        // streamed 3x3 whose far corner stands 1,509 away. With the default a
        // distant mountain was culled and the sky drawn in its place, which is
        // one of the two causes of the skybox appearing in front of far
        // terrain; the other is `render::sky::behind_the_world`.
        //
        // The extra distance costs little: every light in the game fogs out by
        // 888 yards at the latest (`vale light`), so the terrain this admits
        // is drawn entirely in the fog colour. The 1.12.1 client shows a
        // silhouette in the fog colour there, not a hole in the mountain.
        Projection::Perspective(PerspectiveProjection {
            far: reach.yards(),
            // The world camera's opening angle is a diagonal one, 1.925 rad
            // ([`crate::render::lens::WORLD_FIELD_OF_VIEW`], the value the
            // `vanilla-tweaks` patch sets), and Bevy takes a vertical one.
            // [`crate::render::lens`] holds the constant, the conversion and
            // the 16:9 clamp, which is a deviation from the client. This is the
            // 16:9 value, used until the window is measured: `frame_the_view`
            // replaces it with the value for this window.
            // `PerspectiveProjection`'s own default aspect is 1.0, which
            // matches no screen.
            fov: crate::render::lens::framed_vertical_fov(
                crate::render::lens::WORLD_FIELD_OF_VIEW,
                crate::render::lens::WIDEST_FRAMED_ASPECT,
            ),
            ..default()
        }),
        // Overwritten by `place` on the first frame; this only avoids a frame at
        // the origin looking down the default axis.
        Transform::from_translation(axes::to_bevy([25.0, 0.0, 12.0])).looking_at(Vec3::ZERO, Vec3::Y),
        WorldCamera,
        msaa(&tuning),
        // One cluster covering the view, which is the daylight setting.
        // Bevy's clustered forward path bins point and spot lights (and
        // decals, and probes) into a per-view froxel grid every frame. By day
        // this client lights the world with one directional sun and a global
        // fill, neither of which is clusterable, and a camera with the default
        // grid still pays for it: `prepare_clusters_for_gpu_clustering` plus a
        // `cluster_on_gpu` pass, measured at 0.8 + 0.4 ms/frame of render
        // thread in the Trade District trace, binning an empty set. One
        // cluster is the documented low-end setting and skips the grid.
        //
        // `render::lamps` puts up to 24 `PointLight`s in the world at night,
        // and with one cluster every fragment walks all of them. At the default
        // 1200x675 that measured as no cost (6.59 ms either way, opaque pass
        // identical). At 3440x1440 the same loop was ~3 ms of opaque-pass GPU
        // (5.1–5.3 against 1.93–2.04 with the pool suppressed), which was the
        // cause of the 90 fps at fullscreen. A per-fragment cost measured at one
        // window size holds only for that window size.
        //
        // The config is therefore dynamic, and this line sets only the daylight
        // half. `render::lamps::light_the_flames` switches the camera to the
        // default froxel grid when it raises the lamp pool and back to
        // `Single` when it drops it at the end of the night. A noon session
        // pays no CPU binning for an empty set, and a night fragment walks the
        // 0–3 lamps its froxel touches. Both switches are in `render::lamps`,
        // beside the pool.
        bevy::light::cluster::ClusterConfig::Single,
    ));
}

/// Keep the far plane on the far corner of the streamed block.
///
/// See [`spawn`]'s note on `far`: it clips nothing, because the projection is
/// `perspective_infinite_reverse_rh`, but it is the far plane of the
/// `Frustum` every mesh is culled against. A host that widens
/// [`crate::render::terrain::TerrainReach`] gets tiles loaded past it, and a
/// mesh outside the frustum is not drawn, so the extra ground would be loaded
/// and not seen.
fn follow_reach(
    reach: Res<crate::render::terrain::TerrainReach>,
    mut cameras: Query<&mut Projection, With<WorldCamera>>,
) {
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = projection.as_mut() {
            perspective.far = reach.yards();
        }
    }
}

/// Turn Bevy's GPU clustering off, leaving the CPU path to bin the lights.
///
/// `ClusterConfig::Single` in [`spawn`] collapses the grid to one cluster, but
/// it does not remove the machinery. With GPU clustering enabled, every frame
/// still ran `prepare_clusters_for_gpu_clustering` (allocating and uploading
/// this view's cluster buffers), `prepare_clustering_bind_groups`,
/// `upload_view_gpu_clustering_buffers`, and a `cluster_on_gpu` compute pass.
/// When this was added the world's clusterable population was zero: its light
/// was one `DirectionalLight` and a `GlobalAmbientLight`, neither of which is
/// clusterable, and it had no point light, spot light, clustered decal or light
/// probe.
///
/// Measured at an Elwynn framing when this was added:
/// `prepare_clusters_for_gpu_clustering` 0.46 ms/frame,
/// `prepare_clustering_bind_groups` 0.10, the `cluster_on_gpu` encode 0.40,
/// and a 0.14 ms GPU pass beside them. That is about a millisecond of a
/// ten-millisecond frame, spent on an empty set.
///
/// `gpu_clustering: None` is Bevy's own documented switch for this (see
/// `GlobalClusterSettings`). Every GPU-clustering system is
/// `run_if(gpu_clustering_is_enabled)`, so setting it here removes them from
/// the schedule rather than making them return early, and the CPU path takes
/// over.
///
/// Point lights (a lantern, a campfire glow, a spell light) are the case for
/// undoing this: they need the grid, and GPU clustering exists because the
/// CPU path does not scale. `render::lamps` adds such lights at night (see the
/// note on `ClusterConfig::Single` in [`spawn`]), so this setting has to be
/// measured again rather than assumed.
fn stop_clustering(settings: Option<ResMut<bevy::light::cluster::GlobalClusterSettings>>) {
    // Optional because the resource is inserted by `PbrPlugin`, and a headless
    // test app has no such plugin. `DrawCallPlugin` checks for a render world
    // for the same reason.
    if let Some(mut settings) = settings {
        settings.gpu_clustering = None;
    }
}

fn msaa(tuning: &RenderTuning) -> Msaa {
    if tuning.msaa {
        Msaa::Sample4
    } else {
        Msaa::Off
    }
}

/// F5, F9 and F10 flip the three settings (MSAA, vsync, sun shadows).
///
/// Function keys because the other keys are already bound: WASD and QE drive
/// the character, Home re-centres the camera, F12 takes a screenshot, and the
/// chat pane takes every printable key when it has focus. Because these three
/// keys are none of those, they need no `ChatPane::typing` guard.
fn tuning_keys(keys: Res<ButtonInput<KeyCode>>, mut tuning: ResMut<RenderTuning>) {
    if keys.just_pressed(KeyCode::F5) {
        tuning.msaa = !tuning.msaa;
    }
    if keys.just_pressed(KeyCode::F9) {
        tuning.vsync = !tuning.vsync;
    }
    if keys.just_pressed(KeyCode::F10) {
        tuning.sun_shadows = !tuning.sun_shadows;
    }
}

/// Which present mode to ask the swapchain for. The `VALE_PRESENT` override
/// is an experiment rather than a setting.
///
/// The F9 flag chooses between `AutoVsync` and `AutoNoVsync`, which covers
/// normal use. `VALE_PRESENT` overrides both with a named mode. It exists to
/// measure what the acquire and the present cost, which are the two places a
/// frame meets the swapchain: `prepare_windows` (~0.46 ms of self, where the
/// acquire blocks) and `present_frames` (~0.43). `AutoNoVsync` resolves to
/// `Mailbox` on Vulkan/Windows, which keeps a queue of rendered frames;
/// `Immediate` keeps none and tears. Which is cheaper had not been measured.
///
/// It is an environment variable and not a `--tune` name because a player
/// would not choose `Immediate`, which tears; a saving here is a measurement
/// rather than a switch on the render tab. It overrides unconditionally so
/// that the A/B can be taken in the configuration every other render
/// measurement was taken in: the default framing, with vsync on. A number measured in
/// any other configuration cannot be compared with the rest.
fn present_mode(vsync: bool) -> bevy::window::PresentMode {
    use bevy::window::PresentMode;
    match std::env::var("VALE_PRESENT").as_deref() {
        Ok("immediate") => PresentMode::Immediate,
        Ok("mailbox") => PresentMode::Mailbox,
        Ok("fifo") => PresentMode::Fifo,
        _ if vsync => PresentMode::AutoVsync,
        _ => PresentMode::AutoNoVsync,
    }
}

/// Apply [`RenderTuning`] to the camera and the window whenever it changes.
///
/// Sets each window's present mode from `vsync` (see [`present_mode`]), and
/// re-inserts the camera's `Msaa` and `Tonemapping::None`. Bevy reads these
/// settings as components on the view, so a change is an insert rather than a
/// flag the render graph reads.
fn apply_tuning(
    mut commands: Commands,
    tuning: Res<RenderTuning>,
    camera: Query<Entity, With<WorldCamera>>,
    mut windows: Query<&mut bevy::window::Window>,
) {
    if !tuning.is_changed() {
        return;
    }
    for mut window in &mut windows {
        window.present_mode = present_mode(tuning.vsync);
    }
    for entity in &camera {
        let mut camera = commands.entity(entity);
        camera.insert(msaa(&tuning));
        // `Tonemapping` is a required component of `Camera3d` and defaults to
        // `TonyMcMapface`, a filmic curve. 1.12 has no display transform:
        // the client is fixed-function and its only gamma is
        // `SetDeviceGammaRamp`, the display slider, which the driver applies.
        // `None` bypasses the transform.
        camera.insert(Tonemapping::None);
    }
}

/// Left-drag orbits, right-drag looks and steers, the wheel zooms.
///
/// There is no pan. 1.12 has no such control: shift is a walk modifier and
/// every drag orbits the character. A shift+left-drag pan that slid the focus
/// across the ground plane and cleared `CameraRig::follow` left the camera
/// pointed at empty ground for the rest of the session if the player did not
/// know to press `Home`. The pan, the field, the key and the "(free)" read-out
/// were removed together.
///
/// Both buttons turn the camera, and only the right button turns the
/// character, as in the game. A right-drag that rotated only the character
/// left the camera where it was, so steering ended with the view side-on.
///
/// The mouse moves the camera, and the character follows it. In the reverse
/// arrangement a right-drag turned the character and the camera picked the
/// turn up from the character's interpolated facing in `follow_player`: every
/// pixel of drag went out on a channel to the session thread, waited for its
/// next simulation step, came back in the poll, and was then drawn 1.5 steps
/// behind by `Motion`'s play-out delay, so the view answered the mouse about
/// 80 ms late. The accumulator it turned was also `WorldStatus::orientation`,
/// which `poll_world` overwrites from the session every time the simulation
/// advances, so a drag's increment was discarded whenever the round trip had
/// not completed and the turn was slower than its own rate.
///
/// The rig is therefore the accumulator, nothing else writes it while the
/// button is held, and `session::send_input` aims the character at `yaw + π`.
pub fn orbit(
    look: Res<MouseLook>,
    interface: Res<crate::lua::api::mouse::MouseFocus>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    // The drag's four settings and the zoom's three, which are the game's own
    // CVars; see [`look_rate`], [`inversion`], [`rate`] and [`furthest`]. Read
    // per frame, because a CVar can be written by any script between two
    // frames: three hash lookups and parses on every frame, and four more on a
    // frame where the button is held.
    cvars: Res<crate::settings::cvars::CVars>,
    // The zoom moves the eye at a speed, so it needs the frame's length.
    time: Res<Time>,
    mut pressed: MessageReader<crate::input::bindings::BindingPressed>,
    mut rig: ResMut<CameraRig>,
) {
    let speed = rate(&cvars, "cameraDistanceMoveSpeed", DISTANCE_MOVE_SPEED);

    // The zoom already under way runs before this frame's notches are added.
    // In the 1.12.1 client a notch starts its movement at the time of the
    // press, so the frame that reads the press does not move the camera yet.
    glide(&mut rig, time.delta_secs(), speed, furthest(&cvars));

    // `CAMERAZOOMIN` / `CAMERAZOOMOUT`, which can be keys as well as the
    // wheel. The shipped defaults put them on the wheel and a player may bind
    // them to any key. The argument is in yards: `Bindings.xml` passes `1.0`
    // to `CameraZoomIn` and `CameraZoomOut`, so one notch moves the eye one
    // yard.
    //
    // The wheel below is read directly. `MOUSEWHEELUP` is a key name no
    // `KeyCode` can produce, so no key press reaches those two bindings, and
    // without the direct read the wheel would not zoom. As a result the wheel
    // cannot be re-bound. Routing mouse input through the binding table is not
    // implemented, for zoom or for steering.
    for crate::input::bindings::BindingPressed(binding) in pressed.read() {
        if let crate::input::bindings::Binding::CameraZoom(hundredths) = binding {
            zoom(&mut rig, *hundredths as f32 / 100.0, speed);
        }
    }

    let notches = match scroll.unit {
        bevy::input::mouse::MouseScrollUnit::Line => scroll.delta.y,
        // A touchpad reports pixels. Bevy's own factor converts them to lines,
        // so a touchpad swipe zooms about as far as the same travel on a wheel.
        bevy::input::mouse::MouseScrollUnit::Pixel => {
            scroll.delta.y / bevy::input::mouse::MouseScrollUnit::SCROLL_UNIT_CONVERSION_FACTOR
        }
    };
    if notches != 0.0 && !interface.wheel_taken {
        // One wheel notch is one `CameraZoomIn(1.0)`: a yard, whatever the
        // distance, as in the 1.12.1 client.
        //
        // Gated on a frame having handled the wheel, not on the pointer being
        // over the interface, as in the 1.12.1 client. That client hands the
        // wheel to the frame under the pointer and passes it up to the first
        // ancestor with an `<OnMouseWheel>`, so a wheel over the chat or over
        // an open bag reaches no handler and falls through to the zoom, while
        // a wheel over a scroll frame is taken. `lua::mouse` decides it and
        // `MouseFocus::wheel_taken` carries it here.
        zoom(&mut rig, notches, speed);
    }

    // A press that landed on the interface does not steer; see [`MouseLook`].
    // Without this check, dragging a bag icon, a slider or a movable frame
    // across the screen swung the view with it.
    if !look.active {
        return;
    }
    let d = motion.delta;
    if d == Vec2::ZERO {
        return;
    }

    let rate = look_rate(
        windows
            .single()
            .map(|w| w.width() / w.height())
            .unwrap_or(4.0 / 3.0),
        &cvars,
    ) * inversion(&cvars);

    // The drag moves the world, not the eye: a point on the character follows
    // the cursor, which moves the eye the opposite way on both axes. Drag right
    // and the world slides right, so the camera swings left (yaw down, towards
    // the east); drag down and the world tips away from the viewer, so the
    // camera rises (pitch up). Both signs follow from this one convention, so
    // they cannot be wrong independently.
    //
    // No public source states the convention: vmangos says nothing about a
    // camera and the game's files nothing about input. An error here is only
    // visible in the window, so the convention is written down here and pinned
    // by a test that asserts where the eye ends up rather than which way a
    // number moved. A fix to the inverted pitch once also flipped the yaw,
    // which broke the axis that had been correct.
    rig.yaw -= d.x * rate.x;
    // Clear of both poles: at exactly straight down the up vector is degenerate
    // and the view rolls unpredictably. Below the horizon is allowed; see
    // [`PITCH_LIMIT`].
    rig.pitch = (rig.pitch + d.y * rate.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
}

/// Degrees the camera turns per second on a turn key, and the scale on a
/// pixel of mouse-look. `cameraYawMoveSpeed`, default `"180.0"`.
///
/// The 1.12.1 client's mouse-look rate depends on it as well. See
/// [`look_rate`].
///
/// This is the fallback, not the value. The value is the CVar of that name,
/// which `UIOptionsFrame.lua`'s `UIOptionsFrameSliders` table gives a slider
/// (one of the seven `GetCVar` reads in that file), and which `CVars` seeds
/// with this number, so the two agree until the slider is moved.
const YAW_MOVE_SPEED: f32 = 180.0;

/// The same for pitch. `cameraPitchMoveSpeed`, default `"90.0"`.
const PITCH_MOVE_SPEED: f32 = 90.0;

/// Yards per second the zoom moves the eye. `cameraDistanceMoveSpeed`, default
/// `"8.33"`.
///
/// The fallback, as [`YAW_MOVE_SPEED`] is: the value is the CVar, which
/// `CVars` seeds with this number. See [`CameraRig::glide`].
const DISTANCE_MOVE_SPEED: f32 = 8.33;

/// The furthest the 1.12.1 client lets the zoom go, in yards, whatever
/// `cameraDistanceMax` and `cameraDistanceMaxFactor` say.
const DISTANCE_CEILING: f32 = 50.0;

/// How far the wheel may pull the eye back, in yards:
/// `cameraDistanceMax * cameraDistanceMaxFactor`, at most [`DISTANCE_CEILING`].
///
/// Both are the game's own CVars, registered as `"15.0"` and `"1.0"`. The
/// factor is a slider on `UIOptionsFrame.lua`'s own panel (`MAX_FOLLOW_DIST`,
/// 1 to 2 in tenths), which is why this is not a constant. A constant limit
/// (previously 1,500 yards) leaves the game's zoom slider with no effect.
///
/// The product is floored at [`CLOSEST`] so that a `Config.wtf` carrying a zero
/// or a typo cannot produce an empty range, and the eye cannot be sent nearer
/// than the near limit by zooming out.
///
/// It is applied to a zoom outwards and to nothing else, as in the 1.12.1
/// client. [`CameraRig::default`] starts at 25 yards (this project's screenshot
/// framing, which predates the limit), so the first notch out of a session
/// moves the eye in to the limit, while a notch in moves it one yard from 25.
/// Clamping the rig itself would break `--view <distance>,…`, which frames
/// every scripted shot in this repository and may ask for a hundred yards.
fn furthest(cvars: &crate::settings::cvars::CVars) -> f32 {
    let max = cvars.number("cameraDistanceMax") * cvars.number("cameraDistanceMaxFactor");
    max.min(DISTANCE_CEILING).max(CLOSEST)
}

/// Start a zoom of `yards`, positive towards the character.
///
/// The 1.12.1 client turns the amount into a duration at the moment of the
/// press, `yards / cameraDistanceMoveSpeed` seconds truncated to whole
/// milliseconds, and moves the eye at that speed for that long. A zoom the
/// same way as the one under way adds its duration to it; a zoom the other
/// way stops the one under way where it is and starts afresh.
fn zoom(rig: &mut CameraRig, yards: f32, speed: f32) {
    if yards == 0.0 {
        return;
    }
    let seconds = (yards.abs() / speed * 1000.0).trunc() / 1000.0;
    let seconds = seconds.copysign(yards);
    if rig.glide * seconds > 0.0 {
        rig.glide += seconds;
    } else {
        rig.glide = seconds;
    }
}

/// Run `dt` seconds of the zoom under way, at `speed` yards per second.
///
/// Each direction is bounded on its own side only, as in the 1.12.1 client: a
/// zoom in stops at [`CLOSEST`] and a zoom out at `furthest`. The movement is
/// linear rather than eased. It is spent by time, not by distance, so a zoom
/// that reaches a bound early ends at the time it would have ended anyway.
fn glide(rig: &mut CameraRig, dt: f32, speed: f32, furthest: f32) {
    let left = rig.glide.abs();
    if left == 0.0 {
        return;
    }
    let run = left.min(dt);
    let travel = speed * run;
    if rig.glide > 0.0 {
        rig.distance = (rig.distance - travel).max(CLOSEST);
        rig.glide = left - run;
    } else {
        rig.distance = (rig.distance + travel).min(furthest);
        rig.glide = -(left - run);
    }
}

/// How close the wheel may push the eye, in yards. This is the one number here
/// that is this client's rather than the game's: 1.12 zooms all the way into
/// first person, and this client has no first-person mode because the
/// character's own model would be inside the near plane. It is a named
/// constant so that the deviation is stated rather than hidden as a literal in
/// the clamp.
pub(crate) const CLOSEST: f32 = 2.0;

/// Which way a drag turns the view, as a multiplier on [`look_rate`]'s two
/// axes: `mouseInvertYaw` and `mouseInvertPitch`, both registered `"0"`. The
/// second is `UIOptionsFrame.lua`'s `INVERT_MOUSE` box, the first entry under
/// Controls.
///
/// Two CVars because the client registers two, although only the pitch has a
/// box: `mouseInvertYaw` can be set from `/console` and from `Config.wtf`.
/// Reading only one of the pair would leave a setting that is written and
/// ignored, which looks like a bug in the setting.
fn inversion(cvars: &crate::settings::cvars::CVars) -> Vec2 {
    let sign = |on: bool| if on { -1.0 } else { 1.0 };
    Vec2::new(
        sign(cvars.flag("mouseInvertYaw")),
        sign(cvars.flag("mouseInvertPitch")),
    )
}

/// One of the camera's rate settings (the two turn rates in degrees per second,
/// the zoom in yards per second), or the client's registered default if the
/// string is not a number.
///
/// [`crate::settings::cvars::CVars::number`] answers `0.0` for anything it cannot
/// parse, and a zero rate disables mouse-look or the zoom, so a typo in the
/// folder's `Config.wtf` would disable the control without any message. Zero is
/// treated as absent for the same reason. The deviation is small: 1.12's own
/// slider stops well above zero.
fn rate(cvars: &crate::settings::cvars::CVars, name: &str, registered: f32) -> f32 {
    let value = cvars.number(name);
    if value.is_finite() && value != 0.0 {
        value
    } else {
        registered
    }
}

/// The reference viewport the mouse deltas are divided by. The 1.12.1 client's
/// mouse-look scales a delta by `1/800` horizontally and `1/600` vertically.
const REFERENCE_VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);

/// Where a character steered by the mouse should be facing.
///
/// [`CameraRig::yaw`] is the bearing from the character to the eye (the orbit
/// angle), so the direction the camera looks, away from the eye and into the
/// screen, is half a turn from it. `session::send_input` aims the character
/// here for as long as the right button is held, so both-buttons autorun goes
/// where the player is looking rather than along the character's previous
/// heading.
///
/// Here rather than in `session.rs` because the `+ π` is a fact about this rig
/// and nothing else, and because this half of the rule can be pinned by a test
/// without a server: see `a_mouse_looking_character_walks_the_way_the_camera_looks`.
pub fn mouse_look_heading(rig: &CameraRig) -> f32 {
    vale_protocol::state::movement::wrap_angle(rig.yaw + std::f32::consts::PI)
}

/// Radians of camera turn per pixel of drag, yaw and pitch, by the game's own
/// formula.
///
/// The 1.12.1 client turns the camera, for a mouse delta `(dx, dy)`, by:
///
/// ```text
/// yaw   = radians(cameraYawMoveSpeed)   * (dx / sx) / 800
/// pitch = radians(cameraPitchMoveSpeed) * (dy / sy) / 600
/// ```
///
/// where `(sx, sy)` is the half-extent of the client's normalised viewport,
/// built from the aspect ratio `a` as `a/√(a²+1)` and `1/√(a²+1)`
/// (0.8 and 0.6 at 4:3). The rate is therefore independent of resolution and
/// dependent on aspect, which is why the window's aspect is passed in rather
/// than a constant used: 0.281°/px yaw and 0.250°/px pitch at 4:3, 0.258° and
/// 0.306° at 16:9.
///
/// One interpretation is made: `dx` is taken to be raw pixels. The
/// alternative, the client's normalised screen units (which the `/sx` beside
/// it converts out of), would make a full-screen sweep turn the camera by half
/// a degree, which does not match the 1.12.1 client.
///
/// The sensitivity setting 1.12 gives the player does not scale a delta.
/// `mouseSpeed` is clamped to `[0.1, 2.0]`, multiplied by 10 and applied as the
/// Windows pointer speed (`SystemParametersInfo(SPI_SETMOUSESPEED)`). This
/// client therefore has no sensitivity setting either; the operating system's
/// setting is the game's.
fn look_rate(aspect: f32, cvars: &crate::settings::cvars::CVars) -> Vec2 {
    let sy = 1.0 / (aspect * aspect + 1.0).sqrt();
    let sx = aspect * sy;
    let yaw = rate(cvars, "cameraYawMoveSpeed", YAW_MOVE_SPEED);
    let pitch = rate(cvars, "cameraPitchMoveSpeed", PITCH_MOVE_SPEED);
    Vec2::new(
        yaw.to_radians() / (REFERENCE_VIEWPORT.x * sx),
        pitch.to_radians() / (REFERENCE_VIEWPORT.y * sy),
    )
}

/// How far in front of a surface the eye stops, in yards. Under this the near
/// plane clips into the wall and the camera sees the inside of it.
const CAMERA_SKIN: f32 = 0.35;

/// How far the eye may fall back out per second once the obstruction is gone.
///
/// Pulling in is instantaneous and letting out is not, as in most third-person
/// cameras: a frame spent inside a wall is a frame of solid grey, while a frame
/// spent closer than asked is barely noticeable. Easing the return stops a
/// fence post the character runs past from snapping the view in and out.
const CAMERA_RETURN: f32 = 12.0;

/// Pull the eye in until nothing is between it and the character.
///
/// Two sources of obstruction are asked. The buildings, trees and furniture
/// are `CollisionWorld`'s, the same hulls the mover walks into, so the camera
/// is stopped by the same triangle of Stormwind's wall that stops the
/// character. The ground is `MapTerrain`'s, which has no triangles and is
/// sampled along the ray instead. Without the ground check a camera below the
/// horizon is unusable, because the commonest obstruction outdoors is the
/// hillside the character is standing on.
pub fn collide(
    mut rig: ResMut<CameraRig>,
    time: Res<Time>,
    session: Res<crate::world::session::Session>,
    solids: Res<crate::world::session::Solids>,
) {
    let Some(active) = session.active.as_ref() else {
        rig.reach = rig.distance;
        return;
    };
    let focus = rig.focus();
    let wanted = rig.at(rig.distance);
    let map_id = active.map_id;

    // The buildings, as a fraction of the segment.
    let mut hit = solids
        .0
        .ray(map_id, focus.to_array(), wanted.to_array())
        .unwrap_or(1.0);

    // The ground, sampled: the terrain is a height field, so "is the eye under
    // it" is a question asked at points rather than at triangles. Sixteen steps
    // over at most 1500 yards is coarse at full zoom-out and fine at the near
    // end: the near samples are fractions of a yard apart, and the near end
    // decides whether the view is buried.
    //
    // Asked only when the character stands on top of the terrain. A height
    // field has no underside: for anybody already below it every sample reads
    // as the eye being buried. That covers Ironforge, Undercity, the Deeprun
    // Tram, every cave and every cellar. The ground over Ironforge is the
    // mountain the city is cut into, so without this check the first sample of
    // every ray failed and the eye was held at `hit = 1/16` for as long as the
    // player was in the city: the camera stayed at its minimum distance
    // whatever the wheel did.
    //
    // The building ray above stops the eye indoors. It has triangles, so it
    // knows which side of a wall the camera is on, which the ground cannot.
    let on_top_of_the_ground = active
        .terrain_height(map_id, focus.x, focus.y)
        .is_none_or(|ground| focus.z >= ground);
    const SAMPLES: usize = 16;
    if on_top_of_the_ground {
        for step in 1..=SAMPLES {
            let t = step as f32 / SAMPLES as f32;
            if t >= hit {
                break;
            }
            let p = focus.lerp(wanted, t);
            let Some(ground) = active.terrain_height(map_id, p.x, p.y) else {
                continue;
            };
            if p.z < ground + CAMERA_SKIN {
                hit = t;
                break;
            }
        }
    }

    let target = if hit < 1.0 {
        (rig.distance * hit - CAMERA_SKIN).clamp(0.5, rig.distance)
    } else {
        rig.distance
    };
    rig.reach = if target <= rig.reach {
        target
    } else {
        (rig.reach + CAMERA_RETURN * time.delta_secs()).min(target)
    };
}

/// Write the rig onto the camera's `Transform`. The single point where the
/// camera crosses from WoW's axes into Bevy's.
///
/// Public so the sky dome can order itself after it. The dome is centred on
/// the camera, and a dome placed a frame behind slides against the world every
/// time the character moves.
pub fn place(rig: Res<CameraRig>, mut camera: Query<&mut Transform, With<WorldCamera>>) {
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let eye = axes::to_bevy(rig.eye().to_array());
    let target = axes::to_bevy(rig.focus().to_array());
    // Bevy's up is +Y, which is WoW's +Z; `axes` does that conversion.
    *transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
}

/// Keep the opening angle on the 1.12.1 client's rule as the window changes.
///
/// The rule reads the aspect, so the vertical angle is not a constant. The
/// 1.12.1 client's field of view is one diagonal angle, and the vertical angle
/// it draws with depends on the aspect through `sqrt(aspect² + 1)`. The rule
/// and the world camera's diagonal angle are in `crate::render::lens`,
/// including the 16:9 clamp past which this project stops following it. This
/// function decides only when to apply it.
///
/// Reads `PerspectiveProjection::aspect_ratio`, which Bevy writes in
/// `PostUpdate` from the size of the render target: for this camera the
/// `WorldFrame` image, which `render::present::resize` keeps at the window's
/// physical size. A resize is therefore answered one frame late, which is not
/// visible, and the value is the same aspect the projection matrix is about to
/// be built with.
///
/// Writes only when the value changes. A `Mut` dereferenced every frame marks
/// `Projection` changed every frame, which re-derives the frustum and every
/// view uniform that depends on it for the whole of a session; this is the
/// same problem `render::glue::aim_camera` notes on the other side of this
/// component. In the world that means one write per resize and none in
/// between. On a login screen it writes every frame, because `aim_camera` puts
/// the scene's own angle back every frame and this sees a changed value; that
/// adds no cost, since `aim_camera` already marks the component changed every
/// frame.
///
/// Ordered before [`place`], which puts it before `aim_camera`: a login screen
/// sets its own scene's angle, which must take precedence, and `aim_camera` is
/// `.after(place)`.
fn frame_the_view(mut camera: Query<&mut Projection, With<WorldCamera>>) {
    let Ok(mut projection) = camera.single_mut() else {
        return;
    };
    let Projection::Perspective(current) = &*projection else {
        return;
    };
    let want = crate::render::lens::framed_vertical_fov(
        crate::render::lens::WORLD_FIELD_OF_VIEW,
        current.aspect_ratio,
    );
    // 1e-4 radians is six thousandths of a degree: smaller than the step any
    // change of window size moves the value by, and far below anything visible.
    if (current.fov - want).abs() < 1e-4 {
        return;
    }
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.fov = want;
    }
}

#[cfg(test)]
mod tests {
    /// Each entry in `SWITCHES` has its own field.
    ///
    /// A copied line can leave two entries pointing at one flag: the panel looks
    /// correct, and one of the two checkboxes does the other's job. This is the
    /// same assertion `render::tuning` makes about its own list.
    #[test]
    fn every_render_switch_has_its_own_field() {
        use super::{RenderTuning, SWITCHES};

        let mut tuning = RenderTuning::default();
        // Drive every field to a known state first, so "how many are off" is a
        // count of what this test turned off and not of the defaults.
        for (_, _, field) in SWITCHES {
            *field(&mut tuning) = true;
        }
        for (i, (key, label, field)) in SWITCHES.iter().enumerate() {
            *field(&mut tuning) = false;
            let off = SWITCHES.iter().filter(|(_, _, f)| !*f(&mut tuning)).count();
            assert_eq!(off, i + 1, "{key} {label} shares a field with another");
        }
    }

    /// Every entry has a label and a distinct key, since a surface drawing them
    /// prints both.
    #[test]
    fn every_render_switch_is_named() {
        use super::SWITCHES;

        let mut keys: Vec<&str> = SWITCHES.iter().map(|(key, _, _)| *key).collect();
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "two settings share a key");
        assert!(SWITCHES.iter().all(|(_, label, _)| !label.is_empty()));
    }

    use super::*;

    /// The opening angle follows the window, which is what [`frame_the_view`]
    /// does. It is asserted here rather than checked on screen because a wrong
    /// field of view looks plausible at any value.
    ///
    /// The aspect is written by hand: Bevy fills `aspect_ratio` from the render
    /// target in `PostUpdate`, and this app has no target. The live value was
    /// measured separately, when the world camera still used the unmodified
    /// client's `π/2`: a 1000-wide window snapped to 1000x563 reported
    /// `aspect 1.776199` and took the fov to 44.153.
    #[test]
    fn the_opening_angle_follows_the_window() {
        let mut app = App::new();
        app.add_systems(Update, frame_the_view);
        let camera = app
            .world_mut()
            .spawn((
                WorldCamera,
                Projection::Perspective(PerspectiveProjection {
                    aspect_ratio: 4.0 / 3.0,
                    ..default()
                }),
            ))
            .id();
        let fov = |app: &App, e: Entity| match app.world().get::<Projection>(e).unwrap() {
            Projection::Perspective(p) => p.fov.to_degrees(),
            _ => unreachable!(),
        };
        app.update();
        // The vertical angle at 4:3.
        assert!((fov(&app, camera) - 66.18).abs() < 0.05, "{}", fov(&app, camera));

        // At 16:9 the vertical angle is smaller.
        let mut projection = app.world_mut().get_mut::<Projection>(camera).unwrap();
        if let Projection::Perspective(p) = &mut *projection {
            p.aspect_ratio = 16.0 / 9.0;
        }
        app.update();
        assert!((fov(&app, camera) - 54.08).abs() < 0.05, "{}", fov(&app, camera));

        // Past 16:9 it stops moving; see `lens::WIDEST_FRAMED_ASPECT`.
        let mut projection = app.world_mut().get_mut::<Projection>(camera).unwrap();
        if let Projection::Perspective(p) = &mut *projection {
            p.aspect_ratio = 3440.0 / 1440.0;
        }
        app.update();
        assert!((fov(&app, camera) - 54.08).abs() < 0.05, "{}", fov(&app, camera));
    }

    /// A settled camera is not written to, so the frustum and every view
    /// uniform that depends on it are not re-derived every frame.
    #[test]
    fn a_settled_opening_angle_is_left_alone() {
        let mut app = App::new();
        app.add_systems(Update, frame_the_view);
        let camera = app
            .world_mut()
            .spawn((
                WorldCamera,
                Projection::Perspective(PerspectiveProjection {
                    aspect_ratio: 16.0 / 9.0,
                    fov: crate::render::lens::framed_vertical_fov(
                        crate::render::lens::WORLD_FIELD_OF_VIEW,
                        16.0 / 9.0,
                    ),
                    ..default()
                }),
            ))
            .id();
        app.update();
        // Once for the spawn, and not again for the write this must not make.
        app.update();
        let ticks = app
            .world_mut()
            .entity(camera)
            .get_change_ticks::<Projection>()
            .unwrap();
        assert_eq!(ticks.changed, ticks.added);
    }

    /// An app with the gesture and the orbit in it, in that order, and the
    /// inputs the pair reads.
    ///
    /// `arm_look` is included rather than stubbed because it decides whether
    /// `orbit` acts at all. Without it, a test that pressed a button and
    /// asserted on the rig would never reach `orbit`'s steering.
    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<CameraRig>()
            .init_resource::<MouseLook>()
            .init_resource::<crate::lua::api::mouse::MouseFocus>()
            .init_resource::<PointerTaken>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            // No `TimePlugin`, so the frame length is whatever a test advances
            // it by, and zero otherwise.
            .init_resource::<Time>()
            // The four settings the drag reads, at their registered values,
            // which these tests are written against.
            .init_resource::<crate::settings::cvars::CVars>()
            .add_message::<crate::input::bindings::BindingPressed>()
            .add_systems(Update, (arm_look, orbit).chain());
        app
    }

    /// A host that has taken the pointer steers nothing and keeps its cursor.
    ///
    /// The same claim the interface makes, made by something outside this
    /// crate: see [`PointerTaken`]. Both effects are asserted because both
    /// occurred: a host's pointer tool holds the left button and 1.12 steers
    /// with the left button, so using the tool turned the view and locked the
    /// pointer at the pixel the press landed on.
    ///
    /// `MouseLook::active` is the second half: `ui::cursor::hide_while_steering`
    /// grabs the pointer on it, and a grabbed pointer stops moving, which pins
    /// every pick in the client to one place.
    #[test]
    fn a_press_a_host_has_claimed_neither_steers_nor_grabs() {
        let mut app = app();
        app.world_mut().resource_mut::<PointerTaken>().0 = true;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .insert_resource(AccumulatedMouseMotion { delta: Vec2::new(40.0, 20.0) });
        app.update();
        assert_eq!(app.world().resource::<CameraRig>().yaw, 0.0, "the view held still");
        assert!(
            !app.world().resource::<MouseLook>().active,
            "and the pointer was not grabbed"
        );

        // Clearing the flag is enough to give the controls back: the next press
        // steers again.
        app.world_mut().resource_mut::<PointerTaken>().0 = false;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(app.world().resource::<MouseLook>().active);
        assert_ne!(app.world().resource::<CameraRig>().yaw, 0.0);
    }

    /// A press that landed on the interface steers nothing. This is the part
    /// of the gesture the camera reads.
    ///
    /// The check is on the press, not on the current pointer position. A
    /// per-frame check would hand the camera back as soon as a drag left the
    /// button it started on, so the test asserts that a drag which begins over
    /// a frame and ends over the world turns nothing.
    #[test]
    fn a_drag_that_began_on_the_interface_never_steers() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<crate::lua::api::mouse::MouseFocus>()
            .over_interface = true;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .insert_resource(AccumulatedMouseMotion { delta: Vec2::new(40.0, 20.0) });
        app.update();
        assert_eq!(app.world().resource::<CameraRig>().yaw, 0.0);

        // The pointer leaves the frame mid-drag; the gesture is still not a
        // look, because the gesture was decided when the button went down.
        app.world_mut()
            .resource_mut::<crate::lua::api::mouse::MouseFocus>()
            .over_interface = false;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear_just_pressed(MouseButton::Left);
        app.update();
        assert_eq!(app.world().resource::<CameraRig>().yaw, 0.0);
        assert!(!app.world().resource::<MouseLook>().active);
    }

    /// A click is not a drag until it has travelled past the slop. This is the
    /// part of the gesture `interface::target` reads.
    ///
    /// The previous rule was `motion.delta != Vec2::ZERO`, and a mouse moves a
    /// pixel or two during an ordinary click, so a large fraction of clicks on
    /// a mob were classed as camera swings and selected nothing. Tested at both
    /// ends: under the slop is a click, over it is a look, and the flag
    /// survives the release that reads it.
    #[test]
    fn a_press_becomes_a_look_only_past_the_slop() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .insert_resource(AccumulatedMouseMotion { delta: Vec2::new(1.0, 1.0) });
        app.update();
        let look = app.world().resource::<MouseLook>();
        assert!(look.active && look.on_world);
        assert!(!look.dragged, "a two-pixel wobble is a click");

        // Past the slop, then released: the result must still be readable after
        // the button is up.
        app.world_mut()
            .insert_resource(AccumulatedMouseMotion { delta: Vec2::new(20.0, 0.0) });
        app.update();
        assert!(app.world().resource::<MouseLook>().dragged);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.world_mut()
            .insert_resource(AccumulatedMouseMotion { delta: Vec2::ZERO });
        app.update();
        let look = app.world().resource::<MouseLook>();
        assert!(!look.active, "the gesture is over");
        assert!(look.dragged && look.on_world, "…but the release still reads it");
    }

    /// Travel is summed, not measured end to end. A drag out and back finishes
    /// where it started, and treating it as a click would fire an `OnClick` at
    /// the end of a camera swing that returned to its start.
    #[test]
    fn a_drag_out_and_back_is_still_a_drag() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        for delta in [Vec2::new(30.0, 0.0), Vec2::new(-30.0, 0.0)] {
            app.world_mut().insert_resource(AccumulatedMouseMotion { delta });
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .clear_just_pressed(MouseButton::Left);
        }
        assert!(app.world().resource::<MouseLook>().dragged);
    }

    /// There is no pan, and a shift-drag is an ordinary orbit.
    ///
    /// The removed pan cleared `follow` and nothing set it again, so one
    /// accidental shift-drag detached the camera from the character for the
    /// rest of the session (see [`orbit`]). This test pins that shift is only a
    /// walk modifier: the focus must not move, whichever button is held.
    #[test]
    fn a_shift_drag_orbits_and_never_moves_the_focus() {
        let mut app = app();

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        app.world_mut()
            .insert_resource(AccumulatedMouseMotion { delta: Vec2::new(40.0, 20.0) });
        app.update();

        let rig = app.world().resource::<CameraRig>();
        assert_eq!(rig.target, Vec3::ZERO, "shift must not pan the focus");
        assert_ne!(rig.yaw, 0.0, "…and the drag is an ordinary orbit");
    }

    /// The head height lifts the whole rig, eye included.
    ///
    /// Lifting only the point the camera looks at looks almost right: the
    /// character is framed at any normal distance, and the view tilts further
    /// into the ground the closer the zoom. That looks like a wrong pitch clamp
    /// rather than two halves of one rig disagreeing.
    #[test]
    fn the_head_height_lifts_the_eye_and_the_focus_together() {
        let mut rig = CameraRig::default();
        let (focus, eye) = (rig.focus(), rig.eye());
        rig.target_height = 2.0;
        assert_eq!(rig.focus() - focus, Vec3::Z * 2.0);
        assert_eq!(rig.eye() - eye, Vec3::Z * 2.0);
        // And the eye is still `distance` from what it is looking at, which is
        // the property zooming in relies on.
        assert!(((rig.eye() - rig.focus()).length() - rig.distance).abs() < 1e-3);
    }

    /// The camera can be put under the character and aimed at the sky.
    ///
    /// A pitch clamped to `0.05..π/2` never lets the eye below the height it
    /// orbits, so the view can look down at the ground from any angle and
    /// never up, and dragging towards the horizon stops well short of it. The
    /// lower limit is therefore `-PITCH_LIMIT`.
    #[test]
    fn the_pitch_reaches_below_the_horizon_as_well_as_above_it() {
        let drag = |from: f32, dy: f32| {
            let mut app = app();
            app.world_mut().resource_mut::<CameraRig>().pitch = from;
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            app.world_mut()
                .insert_resource(AccumulatedMouseMotion { delta: Vec2::new(0.0, dy) });
            app.update();
            app.world().resource::<CameraRig>().pitch
        };

        // Dragging up from level puts the eye below the focus, looking up.
        let low = drag(0.0, -400.0);
        assert!(low < -1.0, "the camera cannot get under the character: {low}");
        assert!(low >= -PITCH_LIMIT, "past the pole: {low}");
        // …and the far side is still clamped clear of straight down.
        let high = drag(0.0, 400.0);
        assert!(high <= PITCH_LIMIT && high > 1.0, "{high}");

        // With the eye below the focus the camera looks upward, which is the
        // property the angle stands for.
        let mut rig = CameraRig::default();
        rig.pitch = low;
        assert!(rig.eye().z < rig.focus().z, "{:?}", rig.eye());
    }

    /// A right-drag turns the camera itself, at the same rate as a left-drag
    /// and in the same frame the mouse moved.
    ///
    /// When the yaw arrived by way of the character it did neither: a channel
    /// to the session thread, a simulation step, a poll and `Motion`'s
    /// play-out delay added about 80 ms of camera lag. `follow_player`
    /// not writing the yaw while the button is held is the other half of the
    /// same rule; it stops the turn being counted twice when the facing
    /// arrives.
    #[test]
    fn steering_turns_the_camera_in_the_frame_the_mouse_moved() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.world_mut()
            .insert_resource(AccumulatedMouseMotion { delta: Vec2::new(40.0, 20.0) });
        app.update();

        let rig = app.world().resource::<CameraRig>();
        let rate = look_rate(4.0 / 3.0, &Default::default());
        assert!((rig.yaw - -40.0 * rate.x).abs() < 1e-6, "{}", rig.yaw);
        assert_ne!(rig.pitch, CameraRig::default().pitch, "and it pitches");
    }

    /// Shift is a walk modifier and nothing else. A steer with shift held is
    /// still a steer, with the character walking, which is what shift means to
    /// `send_input`. The removed pan slid the camera off the character
    /// mid-turn.
    #[test]
    fn a_steer_with_shift_held_still_steers() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        app.world_mut()
            .insert_resource(AccumulatedMouseMotion { delta: Vec2::new(40.0, 0.0) });
        app.update();

        let rig = app.world().resource::<CameraRig>();
        assert_ne!(rig.yaw, 0.0, "a walking steer must still turn");
        assert_eq!(rig.target, Vec3::ZERO, "and must not pan");
    }

    /// The direction of autorun. `session::send_input` aims the character at
    /// [`mouse_look_heading`] while the right button is held, and both buttons
    /// together run forward, so this asserts that the run goes into the screen
    /// rather than along the heading the character had when the player
    /// finished looking around with the left button.
    ///
    /// Stated as the direction the character walks against the direction the
    /// camera looks, rather than as `yaw + π`, because the `+ π` is what could
    /// be wrong: it is a claim about which end of the rig the yaw measures
    /// from.
    #[test]
    fn a_mouse_looking_character_walks_the_way_the_camera_looks() {
        for yaw in [0.0, 0.7, 2.5, -1.2, 6.0] {
            let mut rig = CameraRig::default();
            rig.yaw = yaw;
            rig.target_height = 2.0;

            // The character's forward, in WoW's axes: facing 0 is north (+X).
            let heading = mouse_look_heading(&rig);
            let walks = Vec2::new(heading.cos(), heading.sin());
            // Where the camera looks, flattened onto the ground.
            let looks = (rig.focus() - rig.eye()).truncate().normalize();
            assert!(
                (walks - looks).length() < 1e-5,
                "at yaw {yaw} the character walks {walks:?} while the camera looks {looks:?}"
            );
        }
    }

    /// The game's own rate, at the two common aspect ratios. Pins the
    /// arithmetic in [`look_rate`] against the 1.12.1 client's degrees per
    /// pixel, so a change to the rate fails against the client's values rather
    /// than against a chosen number.
    #[test]
    fn the_look_rate_is_the_clients() {
        let four_three = look_rate(4.0 / 3.0, &Default::default());
        assert!((four_three.x.to_degrees() - 0.28125).abs() < 1e-5, "{four_three:?}");
        assert!((four_three.y.to_degrees() - 0.25).abs() < 1e-5, "{four_three:?}");

        // Wider screens turn a little slower per pixel and pitch faster, as a
        // result of the normalised viewport.
        let wide = look_rate(16.0 / 9.0, &Default::default());
        assert!(wide.x < four_three.x && wide.y > four_three.y, "{wide:?}");
    }

    /// The two rates in [`look_rate`] come from the CVars, not from this file.
    /// A `Config.wtf` that halves `cameraYawMoveSpeed` halves the drag, and an
    /// unparseable value leaves the rate at its default rather than disabling
    /// the control. See [`rate`], which is the part of this that is a
    /// choice made by this client.
    #[test]
    fn the_look_rate_follows_the_settings() {
        let registered = look_rate(4.0 / 3.0, &Default::default());

        let halved = crate::settings::cvars::CVars::with_saved(&[(
            "cameraYawMoveSpeed".into(),
            "90.0".into(),
        )]);
        let slower = look_rate(4.0 / 3.0, &halved);
        assert!((slower.x - registered.x / 2.0).abs() < 1e-7, "{slower:?}");
        // The other axis is untouched: they are two settings.
        assert!((slower.y - registered.y).abs() < 1e-7, "{slower:?}");

        let broken = crate::settings::cvars::CVars::with_saved(&[(
            "cameraPitchMoveSpeed".into(),
            "fast".into(),
        )]);
        assert!((look_rate(4.0 / 3.0, &broken).y - registered.y).abs() < 1e-7);
    }

    /// The wheel stops where the game's two settings say: 15 yards by default
    /// and 30 with the panel's slider at its top, where it used to allow 1,500.
    /// Floored, because `clamp` panics on an empty range and a hand-edited
    /// `Config.wtf` can produce one.
    #[test]
    fn the_zoom_reaches_as_far_as_the_two_distance_settings_say() {
        assert!((furthest(&Default::default()) - 15.0).abs() < 1e-4);
        let doubled = crate::settings::cvars::CVars::with_saved(&[(
            "cameraDistanceMaxFactor".into(),
            "2".into(),
        )]);
        assert!((furthest(&doubled) - 30.0).abs() < 1e-4);
        let broken =
            crate::settings::cvars::CVars::with_saved(&[("cameraDistanceMax".into(), "".into())]);
        assert!(furthest(&broken) >= CLOSEST);
    }

    /// Inversion is a sign on the rate and nothing else, so the drag's two
    /// direction conventions are not restated in a third place. The shipped
    /// default inverts neither axis.
    #[test]
    fn the_two_invert_settings_flip_one_axis_each() {
        assert_eq!(inversion(&Default::default()), Vec2::ONE);
        let pitch =
            crate::settings::cvars::CVars::with_saved(&[("mouseInvertPitch".into(), "1".into())]);
        assert_eq!(inversion(&pitch), Vec2::new(1.0, -1.0));
        let yaw = crate::settings::cvars::CVars::with_saved(&[("mouseInvertYaw".into(), "1".into())]);
        assert_eq!(inversion(&yaw), Vec2::new(-1.0, 1.0));
    }

    /// The eye is drawn at [`CameraRig::reach`], not at the distance asked for,
    /// and everything that frames the view goes through the one accessor, so a
    /// camera pulled in by a wall keeps its focus, its angles and the property
    /// that the billboards and the view matrix agree about where the eye is.
    #[test]
    fn the_eye_sits_at_the_reach_that_survived_collision() {
        let mut rig = CameraRig::default();
        rig.reach = 4.0;
        assert!(((rig.eye() - rig.focus()).length() - 4.0).abs() < 1e-3);
        assert!(((rig.at(rig.distance) - rig.focus()).length() - 25.0).abs() < 1e-3);
        // Same direction, different length: pulling in must not move the aim.
        let near = (rig.eye() - rig.focus()).normalize();
        let far = (rig.at(rig.distance) - rig.focus()).normalize();
        assert!((near - far).length() < 1e-4);
    }

    /// A plain left-drag orbits the character and never carries the focus with
    /// it. The camera has no way to stop following, so the focus is the
    /// session's one fixed point.
    #[test]
    fn orbiting_turns_the_camera_and_leaves_the_focus_alone() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .insert_resource(AccumulatedMouseMotion { delta: Vec2::new(40.0, 20.0) });
        app.update();

        let rig = app.world().resource::<CameraRig>();
        assert_eq!(rig.target, Vec3::ZERO, "orbiting must not move the focus");
        assert_ne!(rig.yaw, 0.0, "orbiting must turn the camera");
    }

    /// Which way a drag moves the view. No public source states this (vmangos
    /// says nothing about a camera and the game's files nothing about input),
    /// so the convention is stated here, and an error in it is only visible in
    /// the window. A fix to the inverted pitch once also flipped the yaw, which
    /// broke the axis that had been correct. One convention therefore covers
    /// both axes, and this test asserts where the eye ends up rather than
    /// which way a number moved.
    ///
    /// The drag moves the world and the eye goes the other way. A point on the
    /// character follows the cursor, so dragging right swings the eye east (the
    /// camera's own left) and dragging down raises it.
    #[test]
    fn a_drag_moves_the_world_with_the_cursor() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);

        // Right and down, ten pixels each. Bevy's mouse delta is +y downwards.
        app.world_mut()
            .insert_resource(AccumulatedMouseMotion { delta: Vec2::new(10.0, 10.0) });
        app.update();

        let rig = app.world().resource::<CameraRig>();
        // Where the eye went, rather than what the angles are called. At yaw 0
        // the eye is due north of the target and the camera looks south, so its
        // screen-right is west (+Y): a world dragged right means an eye swung
        // the other way, to the east.
        let eye = rig.eye() - rig.focus();
        assert!(
            eye.y < 0.0,
            "the world did not follow the cursor: the eye went west, {eye:?}"
        );
        assert!(
            eye.z > CameraRig::default().eye().z,
            "dragging down must raise the eye: {eye:?}"
        );
    }

    /// No drag of any kind moves the focus, which is what "there is no pan"
    /// means. The test covers every button and modifier combination this
    /// client has.
    #[test]
    fn no_drag_moves_the_focus() {
        for button in [MouseButton::Left, MouseButton::Right] {
            for shift in [false, true] {
                let mut app = app();
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(button);
                if shift {
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .press(KeyCode::ShiftLeft);
                }
                app.world_mut()
                    .insert_resource(AccumulatedMouseMotion { delta: Vec2::new(10.0, 10.0) });
                app.update();
                let rig = app.world().resource::<CameraRig>();
                assert_eq!(rig.target, Vec3::ZERO, "{button:?} shift={shift}");
                assert_ne!(rig.yaw, 0.0, "…and it is still a turn: {button:?}");
            }
        }
    }

    /// One notch of `CameraZoomIn(1.0)` moves the eye one yard at
    /// `cameraDistanceMoveSpeed`, over 120 ms at the default 8.33 yards per
    /// second, and in a straight line rather than an ease.
    #[test]
    fn a_notch_moves_the_eye_a_yard_at_the_zoom_speed() {
        let mut rig = CameraRig {
            distance: 10.0,
            ..default()
        };
        zoom(&mut rig, 1.0, DISTANCE_MOVE_SPEED);
        assert!((rig.glide - 0.120).abs() < 1e-6, "{}", rig.glide);

        glide(&mut rig, 0.05, DISTANCE_MOVE_SPEED, 15.0);
        assert!((rig.distance - (10.0 - 0.05 * 8.33)).abs() < 1e-5, "{}", rig.distance);
        // Past the end the movement stops where its duration says.
        glide(&mut rig, 0.5, DISTANCE_MOVE_SPEED, 15.0);
        assert!((rig.distance - (10.0 - 0.120 * 8.33)).abs() < 1e-5, "{}", rig.distance);
        assert_eq!(rig.glide, 0.0);
        glide(&mut rig, 0.5, DISTANCE_MOVE_SPEED, 15.0);
        assert!((rig.distance - (10.0 - 0.120 * 8.33)).abs() < 1e-5, "it moved after it ended");
    }

    /// Notches the same way add up; a notch the other way drops what was left
    /// and starts its own movement.
    #[test]
    fn notches_add_up_and_a_reversal_starts_afresh() {
        let mut rig = CameraRig::default();
        zoom(&mut rig, 1.0, DISTANCE_MOVE_SPEED);
        zoom(&mut rig, 1.0, DISTANCE_MOVE_SPEED);
        assert!((rig.glide - 0.240).abs() < 1e-6, "{}", rig.glide);
        zoom(&mut rig, -1.0, DISTANCE_MOVE_SPEED);
        assert!((rig.glide + 0.120).abs() < 1e-6, "{}", rig.glide);
    }

    /// A zoom in stops at the near limit and a zoom out at the far one, and
    /// neither looks at the other bound. From the 25-yard start, beyond the
    /// default 15-yard limit, a notch in moves the eye a yard and a notch out
    /// puts it on the limit.
    #[test]
    fn each_direction_is_bounded_on_its_own_side() {
        let run = |yards: f32| {
            let mut rig = CameraRig::default();
            zoom(&mut rig, yards, DISTANCE_MOVE_SPEED);
            glide(&mut rig, 20.0, DISTANCE_MOVE_SPEED, 15.0);
            rig.distance
        };
        assert!((run(1.0) - (25.0 - 0.120 * 8.33)).abs() < 1e-4, "{}", run(1.0));
        assert_eq!(run(-1.0), 15.0);
        assert_eq!(run(100.0), CLOSEST);
    }

    /// The press is read in one frame and the eye starts moving in the next,
    /// at the speed the CVar says, and the far limit never exceeds 50 yards.
    #[test]
    fn the_wheel_zooms_over_the_following_frames() {
        let mut app = app();
        app.world_mut().resource_mut::<CameraRig>().distance = 10.0;
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(40));
        app.world_mut().insert_resource(AccumulatedMouseScroll {
            unit: bevy::input::mouse::MouseScrollUnit::Line,
            delta: Vec2::new(0.0, 1.0),
        });
        app.update();
        assert_eq!(app.world().resource::<CameraRig>().distance, 10.0);

        app.world_mut().insert_resource(AccumulatedMouseScroll::default());
        app.update();
        let distance = app.world().resource::<CameraRig>().distance;
        assert!((distance - (10.0 - 0.040 * 8.33)).abs() < 1e-4, "{distance}");

        // Twice the speed, twice the travel in the same frame.
        let mut fast = self::app();
        fast.insert_resource(crate::settings::cvars::CVars::with_saved(&[(
            "cameraDistanceMoveSpeed".into(),
            "16.66".into(),
        )]));
        fast.world_mut().resource_mut::<CameraRig>().distance = 10.0;
        fast.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(40));
        fast.world_mut().insert_resource(AccumulatedMouseScroll {
            unit: bevy::input::mouse::MouseScrollUnit::Line,
            delta: Vec2::new(0.0, 1.0),
        });
        fast.update();
        fast.world_mut().insert_resource(AccumulatedMouseScroll::default());
        fast.update();
        let distance = fast.world().resource::<CameraRig>().distance;
        assert!((distance - (10.0 - 0.040 * 16.66)).abs() < 1e-4, "{distance}");

        let far = crate::settings::cvars::CVars::with_saved(&[(
            "cameraDistanceMax".into(),
            "100".into(),
        )]);
        assert_eq!(furthest(&far), DISTANCE_CEILING);
    }
}
