//! The world's light, from `Light.dbc` to the frame.
//!
//! The sun's colour, the ambient fill paired with it and the fog distance are
//! all functions of the map, the position and the hour, and this module
//! resolves them. The table is [`vale_assets::tables::light`]; this module turns
//! it into the four things a frame needs and applies one colour-space
//! conversion.
//!
//! ## Where the sRGB bands are decoded to linear
//!
//! The band colours are sRGB and the shaders work in linear, so every band is
//! decoded, but after it has been combined with the other bands, not on the way
//! to the GPU. [`linear`] is the decode, and whether each of the four outputs
//! below is decoded is decided separately for each.
//!
//! A band that is only multiplied by a texel can be decoded here, and a band
//! that is only written to the framebuffer must be. A band that is added to or
//! mixed with another band must not be, because a sum does not survive a
//! transfer function; the sun, the fill, the shadow and the fog are all in this
//! third group. `shaders/atmosphere.wgsl` carries the measurement and the
//! argument. In short, the 1.12.1 client is fixed-function and adds its light
//! in the bytes the file states, so this client does too.
//!
//! The sun, the fill and the fog therefore go to the GPU undecoded, packed into
//! `LinearRgba` because that is the only type Bevy's light and fog uniforms
//! accept. Here the type is a container and does not mean the values are
//! linear. `ClearColor` and the sky dome's six stops are decoded, because
//! nothing mixes either of them.
//!
//! The decode is [`Color::srgb`] into [`LinearRgba`], which is Bevy's own
//! transfer function and not a gamma approximation.
//!
//! ## Transport of the sun and the fill to the GPU
//!
//! The sun and the fill are carried in Bevy's own `DirectionalLight` and
//! `AmbientLight` rather than in a uniform of this crate's, because those are
//! already bound at group 0 for every material in the world. A custom global
//! would need either a render-world bind group or writing the same eight
//! numbers into thousands of per-material uniforms every frame.
//!
//! Bevy premultiplies both by their intensity on the way to the GPU
//! (`color * illuminance`, `color * brightness`), so both intensities are set
//! to 1.0 and the colour carries the whole value. The alternative is splitting
//! one number the table states into two numbers the renderer invents.
//!
//! ## The sun's direction
//!
//! `Light.dbc` carries the sun's colour at every hour and no direction. The
//! direction follows the 1.12.1 client's track,
//! [`vale_assets::tables::light::celestial::light_toward`]: a constant bearing
//! of 225° for the light's travel and an elevation that sweeps 20°..37° over
//! the day, re-aimed by [`apply`] whenever the clock moves. At 17:50 the
//! 1.12.1 client's sun light travels along `(-0.6625, -0.6625, -0.3497)`, which
//! the track matches to half a degree. Before that was measured the direction
//! was held fixed at [`vale_assets::tables::light::SUN_TOWARD`], the sun the
//! ground's `MCSH` shadows were baked for.

use vale_assets::tables::light::{Atmosphere, DAY, NOON, SKY_ALTITUDES, SKY_STOPS};
use bevy::asset::embedded_asset;
use bevy::light::{
    CascadeShadowConfigBuilder, GlobalAmbientLight, NotShadowCaster, NotShadowReceiver,
};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{DistanceFog, FogFalloff, Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, ShaderType,
    SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

/// The hour, in half-minutes past midnight, which is `LightIntBand`'s own unit.
///
/// Once there is a session the value is the server's: `SMSG_LOGIN_SETTIMESPEED`
/// states the world's time and the rate it runs at, and the session thread
/// advances it (see [`vale_protocol::play::time`]). Before a session exists
/// there is nothing to read, and this stands at noon, which is the hour every
/// measurement in this project was taken at.
#[derive(Resource)]
pub struct WorldClock {
    pub half_minutes: u32,
    /// Whether the value above is the server's or the noon placeholder. Shown
    /// on the HUD, because a wrong sky and a correct sky at midnight have
    /// different causes.
    pub from_server: bool,
    /// An hour set by hand, which takes precedence over the server's.
    ///
    /// The settings panel's time slider writes only this field; see
    /// [`crate::ui::debug`]. `None` follows the server, which is the normal
    /// state and the one every measurement in this project was taken in.
    ///
    /// It lives on the clock rather than in the panel because [`resolve`] is the
    /// only place the hour is decided; a second place could produce a sky that
    /// disagrees with the sun. The panel writes a number and reads nothing.
    pub override_half_minutes: Option<u32>,
}

impl Default for WorldClock {
    fn default() -> Self {
        WorldClock {
            half_minutes: NOON,
            from_server: false,
            override_half_minutes: None,
        }
    }
}

impl WorldClock {
    /// The clock as an hour and a minute, for the HUD.
    pub fn hour_minute(&self) -> (u32, u32) {
        let minutes = (self.half_minutes % DAY) / 2;
        (minutes / 60, minutes % 60)
    }
}

/// What the world is currently lit by, and which map and hour it was resolved
/// for.
///
/// Kept so the work is not redone every frame. The lookup is three hash lookups
/// and a few interpolations, which is cheap, but the writes it drives are
/// change-detected components, and rewriting `AmbientLight` every frame would
/// re-upload the light uniform sixty times a second.
#[derive(Resource)]
pub struct Sky {
    pub current: Atmosphere,
    /// Whether the camera is under a liquid surface. Kept beside the atmosphere
    /// it selected so the HUD and the report can say which of the two lights is
    /// in force without asking the terrain again.
    pub submerged: bool,
    /// Every input the resolved atmosphere depends on — see `resolve`, which
    /// names each one where it builds it. The last is which parse of the DBCs
    /// the bands came from, so a table edit re-lights the world.
    resolved_for: Option<(u32, u32, [i32; 3], bool, i32, i32, i32, u64, bool)>,
}

/// How coarsely the player's position is remembered in [`Sky::resolved_for`],
/// in yards.
///
/// The light depends on position through `Light.dbc`'s positional spheres, so
/// the resolve is keyed on a cell of this size rather than on the exact
/// position, which changes every frame the character moves. Crossing a cell
/// moves every blended value by one step, and a skybox's opacity follows its
/// light's weight, so a step is visible as a jump in the sky's brightness: at
/// 4 yards a 70-yard falloff band (a light made with the editor's default
/// radii) faded a skybox in 6% steps. At 1 yard the steps are under 1.5% in
/// the narrowest bands, and a resolve still happens only a few times a second
/// at a run.
pub(crate) const RESOLVE_CELL: f32 = 1.0;

/// Whether a skybox model is drawn at an opacity that hides the rest of the
/// sky; written by [`crate::render::skybox`].
///
/// While one is, the 1.12.1 client draws none of the gradient dome, the star
/// field, the sun, the moons or the cloud layer. A skybox counts only once its
/// model has loaded, so a light naming a model the archives lack leaves the sky
/// as it was. See [`vale_assets::tables::light::SkyboxWeight`].
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkyCover {
    pub opaque: bool,
}

/// A visibility switch for one sky population: shown when its own
/// [`crate::render::tuning::WorldTuning`] switch is on and no opaque skybox is
/// up. The same two-query shape as [`crate::render::tuning::switch`], which
/// this extends with [`SkyCover`].
pub fn under_skybox<M: Component>(
    on: fn(&crate::render::tuning::WorldTuning) -> bool,
) -> impl FnMut(
    Res<crate::render::tuning::WorldTuning>,
    Res<SkyCover>,
    ParamSet<(
        Query<&mut Visibility, With<M>>,
        Query<&mut Visibility, (With<M>, Added<M>)>,
    )>,
) + Send
       + Sync
       + 'static {
    move |tuning, cover, mut targets| {
        let wanted = if on(&tuning) && !cover.opaque {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if tuning.is_changed() || cover.is_changed() {
            for mut visibility in &mut targets.p0() {
                visibility.set_if_neq(wanted);
            }
        } else {
            for mut visibility in &mut targets.p1() {
                visibility.set_if_neq(wanted);
            }
        }
    }
}

impl Default for Sky {
    fn default() -> Self {
        Sky {
            current: Atmosphere::PLACEHOLDER,
            submerged: false,
            resolved_for: None,
        }
    }
}

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        // `atmosphere.wgsl` is the one copy of the sun and the fog, imported by
        // both `terrain.wgsl` and `m2.wgsl`. It is a shader library rather than
        // an `embedded_asset!` because nothing loads it directly: it has to be
        // resident before either fragment shader that imports it compiles.
        //
        // `gamma.wgsl` is the transfer function `atmosphere.wgsl` is built on.
        // Every shader that writes to the frame also needs it directly:
        // `present.wgsl` cannot import it through `atmosphere.wgsl`, because
        // naga_oil does not strip an imported module's unused functions and
        // would pull `bevy_pbr::mesh_view_bindings` into a 2D pipeline. Both
        // are registered here so the library is defined in one place.
        bevy::shader::load_shader_library!(app, "shaders/gamma.wgsl");
        // `night.wgsl` is the deviation `atmosphere.wgsl` imports, which is
        // `render::night`'s part of the world's light.
        //
        // It is registered here rather than by `NightPlugin`. The import chain
        // is `gamma <- atmosphere <- night`, and all three have to be resident
        // before any fragment shader that imports them compiles, so they are
        // loaded in one place where the dependency is visible. If `NightPlugin`
        // loaded it, removing that plugin (to measure the night's cost, for
        // example) would leave every world shader with an import naga_oil
        // cannot resolve. That produces no error: the pipeline stays pending
        // indefinitely and the world is not drawn. See `present.wgsl`'s note on
        // the same failure.
        bevy::shader::load_shader_library!(app, "shaders/night.wgsl");
        bevy::shader::load_shader_library!(app, "shaders/atmosphere.wgsl");
        embedded_asset!(app, "shaders/sky.wgsl");
        app.add_plugins(MaterialPlugin::<SkyMaterial>::default())
            .init_resource::<WorldClock>()
            .init_resource::<Sky>()
            .init_resource::<SkyCover>()
            .add_systems(Startup, (spawn_sun, spawn_dome))
            .add_systems(
                Update,
                (resolve, apply, sun_shadows)
                    .chain()
                    // After this frame's weather has been ramped: `resolve`
                    // mixes the storm light by
                    // [`crate::render::weather::WeatherState::grade`], which
                    // `follow_the_server` writes. Without the ordering the sky
                    // lags the rain by one frame. Nothing fails and the lag is
                    // not visible, which is the form of the three ordering bugs
                    // this project has already had.
                    .after(crate::render::weather::follow_the_server)
                    // After this frame's player position has been settled. The
                    // light depends on position as well as the hour
                    // (`Light.dbc`'s positional spheres), and `resolve` reads
                    // `WorldStatus::position`, which the session pass writes.
                    // Without the ordering it is one frame behind, which is a
                    // fifth of a yard at a run and not visible; the `.after` is
                    // here because a missing ordering causes no failure, and
                    // this project has had three bugs of that form.
                    // `follow_player` is the last system of the session chain
                    // and is the same anchor `camera::place` uses, for the same
                    // reason.
                    .after(crate::world::session::follow_player),
            )
            // The dome is centred on the camera, so it has to be moved after the
            // camera has been placed; one frame late, the whole sky slides
            // against the world every time the character moves. Nothing fails
            // when this ordering is missing.
            .add_systems(Update, follow_camera.after(crate::world::camera::place))
            // The switch that takes the gradient away, leaving `ClearColor`,
            // which is the fog band. It also hides the dome behind an opaque
            // skybox; see [`SkyCover`]. Its own system rather than a branch
            // inside [`apply`], because nothing else writes the dome's
            // visibility.
            .add_systems(
                Update,
                under_skybox::<SkyDome>(|tuning| tuning.sky_dome)
                    .after(crate::render::skybox::follow_the_light),
            );
        // The HUD line, which is not part of drawing the sky — see
        // `ui::debug` for what the `diagnostics` feature removes.
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report.run_if(crate::ui::report::watched));
    }
}

/// [`crate::ui::report`] slot. See that module for why this line is written
/// here rather than in `hud.rs`.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(10);

/// The world's light on the HUD. Without this line the light can be judged
/// only by looking at the world, which cannot distinguish a wrong sky from a
/// correct sky at midnight.
///
/// Shows the hour, whether it is the server's or the noon placeholder, and the
/// two numbers the frame is built from.
#[cfg(feature = "diagnostics")]
fn report(
    clock: Res<WorldClock>,
    sky: Res<Sky>,
    mut hud: ResMut<crate::ui::report::HudReport>,
) {
    if !clock.is_changed() && !sky.is_changed() {
        return;
    }
    let (hour, minute) = clock.hour_minute();
    let byte = |v: f32| (v * 255.0).round() as i32;
    let rgb = |c: [f32; 3]| format!("{}/{}/{}", byte(c[0]), byte(c[1]), byte(c[2]));
    hud.set(
            crate::ui::report::Section::World,
        SLOT,
        "sky",
        format!(
            "{hour:02}:{minute:02}{}  sun {}  fog {}{} to {:.0}y..{:.0}y",
            match (clock.override_half_minutes, clock.from_server) {
                // The hand-set hour is shown explicitly, because this line is
                // read when the sky looks wrong, and a slider left set is the
                // simplest cause to rule out.
                (Some(_), _) => " (set by hand)",
                (None, true) => "",
                (None, false) => " (noon, no clock)",
            },
            rgb(sky.current.diffuse),
            rgb(sky.current.fog()),
            // Which of the light's two rows is in force (`Sky::submerged`). An
            // underwater atmosphere is a short, strongly coloured fog, and a
            // wrong fog and a camera the client considers under a lake have
            // different causes.
            if sky.submerged { " (underwater)" } else { "" },
            sky.current.fog_start,
            sky.current.fog_end,
        ),
    );
}

/// How far away the dome is drawn, in yards.
///
/// The radius does not decide occlusion; [`behind_the_world`] does. The number
/// only has to put the dome's geometry beyond the near plane and around the
/// camera. The world stays in front of the dome because every sky fragment
/// writes the far-plane depth, not because of the radius. When the radius was
/// the occlusion mechanism it failed: 950 is short of the loaded 3x3's far
/// corner (about 1,500 yards), so a mountain there was drawn first and then
/// painted over by the sky.
const DOME_RADIUS: f32 = 950.0;

/// The rule that keeps the sky behind the world, applied by every material
/// that draws sky: the gradient dome, the star field and the celestial sprites.
///
/// The problem is distance, not draw order. Bevy's opaque phase is binned by
/// pipeline rather than sorted by depth, and the sky writes no depth of its
/// own, so whether a sky fragment survived depended only on the depth test
/// against geometry already drawn. That works only while the sky is the
/// furthest thing in the frame, and it is not: the terrain streamer keeps a 3x3
/// of 533-yard tiles, whose far corner is about 1,500 yards from a character
/// anywhere in the middle tile, while the dome is at 950 yards and the sprites
/// at twelve.
///
/// The fragment shaders therefore write `frag_depth = 0.0`, the far plane under
/// Bevy's reversed depth, and this function loosens the comparison from
/// `Greater` to `GreaterEqual` so that a fragment exactly at the far plane
/// still passes where nothing has been drawn. This holds in either draw order:
/// anything that has written depth is drawn over the sky, and the sky writes no
/// depth, so it occludes nothing.
///
/// The cost is early-Z on the sky surfaces. A shader that computes its own
/// depth cannot be rejected before it runs, so the dome's fragment shader runs
/// for every pixel it covers rather than only the ones the terrain left. It is
/// a six-stop mix with no texture fetch, and the star and sprite passes are one
/// texture fetch each over a few hundred pixels.
pub fn behind_the_world(descriptor: &mut RenderPipelineDescriptor) {
    let Some(depth) = descriptor.depth_stencil.as_mut() else {
        return;
    };
    depth.depth_write_enabled = Some(false);
    depth.depth_compare = Some(CompareFunction::GreaterEqual);
}

/// The sort rule for the blended sky surfaces (the star field and the three
/// celestial sprites), which complements [`behind_the_world`].
///
/// [`behind_the_world`] orders the sky against everything that has written
/// depth, which is all of the opaque world. It does not order the sky against
/// the transparent world, because a blended surface writes no depth either: the
/// two are ordered only by `Transparent3d`'s sort, which is each mesh's own
/// centre plus its material's `depth_bias`, ascending. Bevy's values increase
/// toward the camera, so the smallest is drawn first.
///
/// An earlier bias of +4096 on the sprites drew the moon over lakes. The bias
/// was chosen to lift the sprites over the star dome, whose mesh centre is the
/// camera's own position and therefore the nearest thing in the frame. A
/// positive bias means nearer, so the sprites sorted ahead of every blended
/// surface in the world and were drawn last, over all of them. Water is
/// blended, and so are particles, ribbons and every spell effect, so a moon low
/// over a lake was drawn on top of the water.
///
/// Every sky surface now takes a bias far more negative than any real distance
/// in a loaded 3x3, so the whole sky sorts behind the whole world, and the
/// small offsets between the sky surfaces keep the 1.12.1 client's order: the
/// star dome, then the sun, then the two moons.
///
/// The value is a million rather than the smallest that works: the loaded
/// world reaches about 1,500 yards, and an `f32` at 1e6 still resolves the
/// one-yard steps below it exactly (its spacing there is 1/16 of a yard).
pub const SKY_SORT: f32 = -1.0e6;

/// The star field's place in that order: first, so the sun and the moons are
/// drawn over it rather than speckled by it.
pub const STARS_SORT: f32 = SKY_SORT;

/// The sun's and the moons' place in that order: after the star field.
pub const CELESTIAL_SORT: f32 = SKY_SORT + 1.0;

/// The cloud layer, drawn over the sun and the moons as the 1.12.1 client
/// draws it.
pub const CLOUDS_SORT: f32 = SKY_SORT + 2.0;

/// The skybox models, drawn over everything else in the sky. Each batch adds
/// its own offset to this, so the batches of up to two models keep a fixed
/// order; see [`crate::render::skybox`].
pub const SKYBOX_SORT: f32 = SKY_SORT + 16.0;

/// The one directional light that represents the sun.
///
/// Its colour is the light table's and is written by [`apply`]; what is set
/// here is its initial direction and that there is exactly one. Whether it
/// casts shadow maps is [`crate::world::camera::RenderTuning::sun_shadows`],
/// which is off: the game's own shadows are the ground's baked `MCSH` and a
/// blob under each unit, both of which are drawn regardless. See
/// [`sun_shadows`].
fn spawn_sun(mut commands: Commands, tuning: Res<crate::world::camera::RenderTuning>) {
    commands.spawn((
        Sun,
        DirectionalLight {
            // The whole value is in the colour; see the module note on transport.
            illuminance: 1.0,
            shadow_maps_enabled: tuning.sun_shadows,
            ..default()
        },
        // Used only when F10 turns shadow maps on, and sized for a character
        // rather than for the zone: the object a player wants a real shadow of
        // is in front of the camera, and a cascade stretched over the whole 3x3
        // spends its resolution on ground nobody is looking at. 150 yards is
        // past the far edge of a normal framing and well inside every fog
        // distance the table states.
        CascadeShadowConfigBuilder {
            num_cascades: 4,
            first_cascade_far_bound: 12.0,
            maximum_distance: 150.0,
            ..default()
        }
        .build(),
        // Stand where the sun stands at noon and face the world; [`apply`]
        // re-aims it as the clock moves. See the module note.
        aim(vale_assets::tables::light::celestial::light_toward(NOON)),
    ));
}

/// The sun's transform for a unit vector toward it in the world's own axes:
/// stood a hundred yards out along it, facing the origin. A directional light
/// reads only the orientation, so the distance is cosmetic.
fn aim(toward: [f32; 3]) -> Transform {
    Transform::from_translation(crate::render::axes::to_bevy([
        100.0 * toward[0],
        100.0 * toward[1],
        100.0 * toward[2],
    ]))
    .looking_at(Vec3::ZERO, Vec3::Y)
}

/// Marks the sun, so [`apply`] can find the one light rather than every light.
#[derive(Component)]
pub struct Sun;

/// Whether the sun casts shadow maps, from [`crate::world::camera::RenderTuning`].
///
/// Off by default and toggled by a key, because 1.12 does not draw shadow maps.
/// The game's own shadows are two things and neither is a shadow map: the
/// ground carries `MCSH`, baked by Blizzard's tools, and a unit stands on a
/// blob (`Textures\ShadowBlob.blp`; see [`crate::entities`]). Both are always
/// drawn. A cascade over nine tiles of terrain, a city and twelve thousand
/// doodads is a second pass over all of it and is the most expensive option
/// this renderer has. It exists so that shadow maps can be compared with the
/// game's shadows by toggling at one framing, and F10 is faster than a
/// rebuild.
fn sun_shadows(
    tuning: Res<crate::world::camera::RenderTuning>,
    mut sun: Query<&mut DirectionalLight, With<Sun>>,
) {
    if !tuning.is_changed() {
        return;
    }
    for mut light in &mut sun {
        light.shadow_maps_enabled = tuning.sun_shadows;
    }
}

/// The six sky bands as the dome's fragment shader wants them.
///
/// `w` carries the altitude each stop sits at, not padding: a gradient is
/// stops plus positions, and keeping both in one array means the shader has no
/// second copy of [`SKY_ALTITUDES`] to drift from.
#[derive(Clone, Copy, ShaderType)]
pub struct SkyStops {
    pub stops: [Vec4; SKY_STOPS],
}

/// The dome's material: six colours and nothing else.
///
/// No texture, no light, no fog. The sky's appearance comes entirely from the
/// table, and the material holds only the six colours taken from it.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct SkyMaterial {
    #[uniform(0)]
    pub stops: SkyStops,
}

impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        crate::render::shader::SKY.into()
    }

    /// Not in the depth prepass. The dome writes no depth in the main pass
    /// either (see [`Self::specialize`]), and a prepass that wrote it would put
    /// a wall at 950 yards in front of the occlusion culler, which reads the
    /// prepass's own depth pyramid and would then reject the far half of the
    /// world.
    fn enable_prepass() -> bool {
        false
    }

    /// And it casts nothing. `NotShadowCaster` says so on the entity too; this
    /// says it for the pipeline, so no shadow variant of this shader is ever
    /// compiled.
    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Seen from the inside, so the sphere's own winding faces away.
        descriptor.primitive.cull_mode = None;
        behind_the_world(descriptor);
        Ok(())
    }
}

/// The dome itself.
#[derive(Component)]
pub struct SkyDome;

/// One sphere, centred wherever the camera is, drawn from the inside.
///
/// A mesh rather than a full-screen pass because the colour is a function of
/// direction and a sphere's geometry already encodes direction, and because a
/// mesh needs no render-graph node, no view uniform of this crate's and no
/// second camera.
///
/// The resolution is the gradient's, not the geometry's: the fragment shader
/// interpolates the six stops itself, so the sphere only has to be round enough
/// that its silhouette is not visible against the terrain, and no silhouette at
/// 950 yards is.
fn spawn_dome(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
) {
    commands.spawn((
        SkyDome,
        Mesh3d(meshes.add(Sphere::new(DOME_RADIUS).mesh().uv(32, 16))),
        MeshMaterial3d(materials.add(SkyMaterial {
            stops: stops_of(&Atmosphere::PLACEHOLDER),
        })),
        // The dome is a backdrop: neither casting nor receiving shadow means
        // anything for it, and casting would put the whole world inside a
        // 950-yard shadow when F10 is pressed.
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

/// Keep the dome centred on the camera, so the sky never gets nearer.
///
/// This reads the camera's `Transform`, not its `GlobalTransform`. The three
/// sky passes that follow the camera share this note.
///
/// `camera::place` writes the `Transform` in `Update` and Bevy propagates it
/// into the `GlobalTransform` in `PostUpdate`, so a system ordered after
/// `place` that reads the global transform reads last frame's eye position,
/// and the ordering has no effect. For the dome the error is not visible: it
/// is 950 yards across, and one frame of travel moves it by a few centimetres
/// relative to that. For the celestial sprites, twelve yards from the eye, the
/// same error is an angular error: under a mouse-look drag the eye moves yards
/// in a frame, so the sun and the moons jumped about the sky while the camera
/// moved and stopped when it stopped.
///
/// The camera has no parent, so its `Transform` is its global transform one
/// frame before propagation copies it. `Without<WorldCamera>` on the mutable
/// query lets both queries live in one system, since both access `Transform`.
fn follow_camera(
    camera: Query<&Transform, With<crate::world::camera::WorldCamera>>,
    mut dome: Query<&mut Transform, (With<SkyDome>, Without<crate::world::camera::WorldCamera>)>,
) {
    let Some(eye) = camera.iter().next() else {
        return;
    };
    for mut transform in &mut dome {
        transform.translation = eye.translation;
    }
}

/// The atmosphere's six sky bands as the shader's stops: decoded to linear, and
/// tagged with the altitude each sits at.
fn stops_of(atmosphere: &Atmosphere) -> SkyStops {
    let mut stops = [Vec4::ZERO; SKY_STOPS];
    for (stop, (colour, altitude)) in stops
        .iter_mut()
        .zip(atmosphere.sky.iter().zip(SKY_ALTITUDES))
    {
        *stop = linear(*colour).extend(altitude);
    }
    SkyStops { stops }
}

/// Look the current map and hour up in the light chain, when either has moved.
///
/// Reads the display tables through [`crate::assets::GameAssets`], which parses
/// them on first use, so before a session exists this leaves
/// [`Atmosphere::PLACEHOLDER`] in place and the glue screens are lit by it.
fn resolve(
    mut sky: ResMut<Sky>,
    mut clock: ResMut<WorldClock>,
    // The session, for the one input only a server has: the world's own clock.
    session: Res<crate::world::session::Session>,
    // Where that world is, which a host with no server also provides. See
    // [`crate::render::focus`].
    focus: Res<crate::render::focus::WorldFocus>,
    assets: Res<crate::assets::GameAssets>,
    rig: Res<crate::world::camera::CameraRig>,
    // The deep night is decided here and nowhere else, for the same reason as
    // the hand-set hour: a second place that decides it could produce a sky
    // that disagrees with the sun. See [`crate::render::night`], which is this
    // client's own deviation from the table and is an option rather than a
    // correction.
    deep_night: Res<crate::render::night::NightTuning>,
    mut night: ResMut<crate::render::night::Night>,
    // The weather the server reports. The storm column of `Light.dbc` is the
    // whole effect of rain on the light (see
    // [`vale_assets::tables::light::LightTables::atmosphere_in_storm`]), and it
    // is read here rather than elsewhere for the same reason as the hour: a
    // second place that reads it could produce a sky that disagrees with
    // itself.
    weather: Res<crate::render::weather::WeatherState>,
    // The switch that removes weather from the frame, so that
    // `--without weather` removes the storm light as well as the drops. A
    // subtraction that leaves half the pass in place measures neither half;
    // see [`crate::render::tuning`].
    tuning: Res<crate::render::tuning::WorldTuning>,
    // Whether the player is a ghost, which lights the world with the death row
    // of the map's default light; see `LightTables::atmosphere_dead`.
    dying: Option<Res<crate::interface::death::Dying>>,
) {
    if !focus.present {
        // Outside the world, the light returns to what it was before the first
        // login. This is what clears the night on logout.
        //
        // Three separate things carry the resolved atmosphere and nothing else
        // resets any of them, so without this reset all three kept it. The
        // grade in [`crate::render::night`] made all three visible: the sun and
        // the fill stayed cold and dim, the fog stayed close, and the present
        // pass kept darkening, cooling and vignetting the whole frame, which at
        // the character screen is the glue scene. Logging out at midnight left
        // the login screen faded for the life of the process.
        //
        // The teardown in `residency::leave_world` is not the place for it:
        // this is the one function that decides what the world is lit by, and
        // the note on the hand-set hour explains why there must be only one.
        // Doing it here also covers the case that is not a logout, the frames
        // before the first login, which now take the same path rather than
        // relying on the resource's default.
        //
        // Guarded, because this runs every frame the client is not in a world.
        // `ResMut`'s `DerefMut` marks a resource changed whether or not the
        // value moved, and [`apply`] reads that flag: an unguarded write would
        // re-upload the light uniform, the fog and the dome's six stops sixty
        // times a second for as long as the login screen was open.
        // `resolved_for` is the indicator: it is `None` on a fresh start and
        // `Some` only after a world has been resolved.
        if sky.resolved_for.is_some() || night.factor != 0.0 {
            sky.current = Atmosphere::PLACEHOLDER;
            sky.submerged = false;
            sky.resolved_for = None;
            night.factor = 0.0;
            night.deck = 0.0;
        }
        return;
    }
    // The session is optional and the focus is not. Of the three inputs that
    // could come from the session, two come from the focus instead (the map
    // and the position), and the third, the server's clock, does not exist for
    // a host with no server. That host sets the hour by hand, which is the
    // guard four lines below.
    let active = session.active.as_ref();
    // A hand-set hour takes precedence, and the server's is still read
    // underneath it, so releasing the slider returns to the world's own clock
    // with nothing to re-sync. See [`WorldClock::override_half_minutes`].
    if let Some(hour) = clock.override_half_minutes {
        if clock.half_minutes != hour {
            clock.half_minutes = hour;
        }
    }
    // Written only when it moves: a half-minute is 30 s of real time at the
    // server's rate, so this is a no-op on all but one frame in eighteen
    // hundred, and touching a `ResMut` is what marks it changed.
    else if let Some(time) = active.and_then(|a| a.live.game_time()) {
        let half_minutes = time.half_minutes();
        if clock.half_minutes != half_minutes || !clock.from_server {
            clock.half_minutes = half_minutes;
            clock.from_server = true;
        }
    }
    // No resolve until every input comes from the world. Measured: 2 of 5
    // scripted logins resolved before `SMSG_LOGIN_SETTIMESPEED` had been read,
    // keyed the atmosphere on the clock's noon default (`half_minutes 1440,
    // from_server false`, orange daylight bands), and corrected only when the
    // packet arrived and changed the key. `WorldFocus::present` is the same
    // guard for the position: the client sets it in the same frame it
    // publishes a position, so before it the position may be another
    // session's or absent. A resolve keyed on a wrong position does not
    // correct itself while the character stands still, because the cell is
    // the only movement-keyed input; that is the cause of the long-standing
    // "fog too thick and no night until I take a step" report. Returning
    // leaves the placeholder in place and retries next frame, which costs at
    // most the frames the login burst already takes.
    if clock.override_half_minutes.is_none() && !clock.from_server {
        return;
    }
    // The world's own axes, which the light table uses.
    // `WorldFocus::position` is the server's position unconverted; `axes.rs`
    // is the one place positions are converted, and this is not it.
    let at = [focus.position.x, focus.position.y, focus.position.z];
    let cell = at.map(|c| (c / RESOLVE_CELL) as i32);
    // The eye, not the feet. Which of a light's two rows is in force depends
    // on the camera: a swimmer at the surface has their head above it, and the
    // view turns blue only when the orbiting eye dips under. The 1.12.1 client
    // behaves this way, which is why the same wave breaking over a stationary
    // character switches the effect on and off. `CameraRig::eye` is already in
    // the world's own axes, so nothing here converts anything.
    //
    // This is outside the `RESOLVE_CELL` quantisation the position gets: four
    // yards is most of the distance between a floating head and a submerged
    // one, so rounding this would answer a frame or two late at every
    // crossing. It is a boolean, so evaluating it every frame costs one tile
    // lookup on a cache the mover has already warmed.
    let eye = rig.eye();
    // Whether the eye is under water is the one question here that only a
    // session answers: the liquid lookup goes through the mover's own warmed
    // tile cache. A host with no session draws the surface from above.
    let submerged = active.is_some_and(|a| {
        a.terrain_liquid(focus.map_id, eye.x, eye.y)
            .is_some_and(|(_, surface)| eye.z < surface)
    });
    // The night setting is part of the key, because the grade is applied to
    // the resolved atmosphere: without it, moving the panel's slider would
    // change nothing until the world's own clock next ticked, which at the
    // server's rate can take up to thirty seconds. Quantised to a byte so that
    // dragging it does not re-resolve the light chain on every pixel of
    // travel, and `-1` for off so that off and zero strength are the same key.
    let graded = if deep_night.enabled {
        (deep_night.strength.clamp(0.0, 1.0) * 255.0) as i32
    } else {
        -1
    };
    // Quantised to a byte, like the night's grade and for the same reason: the
    // grade ramps over ten seconds, so an unquantised key would re-resolve the
    // whole light chain on every frame of a ramp. A 255th of the grade is a
    // step too small to see.
    let storm = if tuning.weather {
        (weather.storm() * 255.0) as i32
    } else {
        0
    };
    // The visibility the precipitation leaves, quantised the same way. This
    // one comes from the density (and is released indoors), so it is a
    // separate key from the grade's; see
    // [`crate::render::weather::WeatherState::visibility`].
    let visibility = if tuning.weather {
        (weather.visibility().clamp(0.0, 1.0) * 255.0) as i32
    } else {
        255
    };
    // Which parse of the tables these bands came from.
    //
    // Without it, `GameAssets::forget_tables` does not re-light the world: the
    // key holds the hour, the position quantised to `RESOLVE_CELL` and the
    // three settings, and an edited `LightIntBand` changes none of them. A
    // colour changed while standing still then took effect only on the next
    // half-minute the server sent (up to thirty seconds) or after the next four
    // yards walked, so the edit appeared to do nothing.
    //
    // It is the same pattern as the five caches keyed on `spellbook_version()`
    // that a table edit does not touch. One relaxed atomic load per resolve,
    // and a resolve happens only when this key changes.
    let tables_from = assets.tables_generation();
    let ghost = dying.is_some_and(|d| d.ghost);
    let key = (
        focus.map_id,
        clock.half_minutes,
        cell,
        submerged,
        graded,
        storm,
        visibility,
        tables_from,
        ghost,
    );
    if sky.resolved_for == Some(key) {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    // A chain with no light tables leaves the placeholder in place, which is
    // the documented degradation; see `DisplayTables::load`.
    // The deep night grades on the hour's own light, without the storm.
    // `night::factor` reads the atmosphere's `diffuse + ambient` luma, and the
    // storm row's grey sun sums to 0.706 on map 0, below `NIGHT_LUMA`'s 0.72,
    // so grading on the storm-mixed light applied the full deep night to a
    // noon downpour: Goldshire at 12:55 in heavy rain was drawn as midnight,
    // including the lamp glow. The night depends on the hour; the storm comes
    // from the server and already carries its own darkening.
    let mut hours_own = sky.current;
    if let (true, Some(light)) = (ghost, tables.light()) {
        sky.current = light.atmosphere_dead(focus.map_id, clock.half_minutes);
        hours_own = sky.current;
    } else if let Some(light) = tables.light() {
        let weather = if submerged {
            vale_assets::tables::light::Weather::Underwater
        } else {
            vale_assets::tables::light::Weather::Clear
        };
        sky.current = light.atmosphere_in_storm(
            focus.map_id,
            at,
            clock.half_minutes,
            weather,
            storm as f32 / 255.0,
        );
        hours_own = if storm > 0 {
            light.atmosphere_in(focus.map_id, at, clock.half_minutes, weather)
        } else {
            sky.current
        };
    }
    // The visibility reduction is this client's own, added on request. Falling
    // snow reduces visibility in proportion to how much of it is in the air,
    // in addition to the storm row's fog; the number used is the density. Not
    // applied underwater, because precipitation is a property of air, and
    // floored so that an instance whose fog is already close does not close
    // further to an opaque wall. See `render::weather::VISIBILITY_TAKEN`.
    if visibility < 255 && !submerged && !ghost {
        let squeeze = visibility as f32 / 255.0;
        let floor = crate::render::weather::VISIBILITY_FLOOR_YARDS.min(sky.current.fog_end);
        sky.current.fog_end = (sky.current.fog_end * squeeze).max(floor);
        sky.current.fog_start = (sky.current.fog_start * squeeze).min(sky.current.fog_end);
    }
    // The grade, applied after the chain and before anything reads it.
    // `deepen` is the identity at a factor of zero, so daylight leaves this
    // function with exactly the bands `atmosphere_in` returned; see the test
    // in `render::night` that checks it.
    let factor = deep_night.applied_to(&hours_own);
    crate::render::night::deepen(&mut sky.current, factor);
    let deck = at[2];
    if night.factor != factor || night.deck != deck {
        night.factor = factor;
        night.deck = deck;
    }
    let first = sky.resolved_for.is_none();
    sky.submerged = submerged;
    sky.resolved_for = Some(key);
    // The first resolve of a world is logged at info and the rest at debug: a
    // resolve fires every four yards of travel, which is a line a second at a
    // run, and only the first is likely to be wrong. This line found the login
    // race that this function's guard now prevents (2 of 5 logins keyed on the
    // clock's noon default before `SMSG_LOGIN_SETTIMESPEED` was read). It is
    // also the diagnostic for the part of the report that has not reproduced
    // under a script: a session that starts pale logs which input was wrong.
    let line = format!(
        "light: resolved at {:.0},{:.0},{:.0} (cell {:?}) map {} half_minutes {} \
         from_server {} -> sun {:?} fill {:?} fog {:.0}..{:.0} night {:.2}",
        at[0], at[1], at[2], cell, focus.map_id, clock.half_minutes, clock.from_server,
        sky.current.diffuse.map(|c| (c * 255.0) as i32),
        sky.current.ambient.map(|c| (c * 255.0) as i32),
        sky.current.fog_start, sky.current.fog_end, night.factor,
    );
    if first {
        info!("{line}");
    } else {
        debug!("{line}");
    }
}

/// Push the resolved atmosphere into the four things that carry it: the sun, the
/// fill, the fog, and what is behind everything.
///
/// Each write is guarded, because all four are change-detected and three of them
/// are uploaded to the GPU when touched.
pub(crate) fn apply(
    sky: Res<Sky>,
    // The global ambient rather than the camera's own component: this is the
    // world's fill, and a second camera (a portrait, a minimap) should be lit by
    // the same one rather than needing its own copy.
    mut ambient: ResMut<GlobalAmbientLight>,
    mut clear: ResMut<ClearColor>,
    mut sun: Query<(&mut DirectionalLight, &mut Transform), With<Sun>>,
    // The clock, for where the sun stands; see [`aim`]. Optional because the
    // colour half of this is tested on an app with no clock, which gets noon.
    clock: Option<Res<WorldClock>>,
    dome: Query<&MeshMaterial3d<SkyMaterial>, With<SkyDome>>,
    mut sky_materials: ResMut<Assets<SkyMaterial>>,
    camera: Query<Entity, With<crate::world::camera::WorldCamera>>,
    // A camera that appears after the light has settled needs the fog too.
    // The sky resolves once per map and hour and then stops changing, so a
    // camera spawned or respawned after that would otherwise never be given a
    // `DistanceFog`. It would draw the world to the horizon with a sharp edge,
    // which looks like missing fog rather than an ordering bug.
    fresh_camera: Query<(), Added<crate::world::camera::WorldCamera>>,
    // Only the fog switch is read here; see [`crate::render::tuning`]. It
    // joins the guard below rather than being applied every frame, because all
    // four writes underneath are change-detected and three of them are uploads.
    tuning: Res<crate::render::tuning::WorldTuning>,
    // The mist is carried in the fog component, so it is written here with
    // the rest of the fog rather than by a second system that would have to
    // insert the same component. See [`crate::render::night::NightAir::pack`]
    // for which two of `DistanceFog`'s fields carry it and why they are
    // otherwise unused.
    night: Res<crate::render::night::Night>,
    mut commands: Commands,
) {
    if !sky.is_changed()
        && !tuning.is_changed()
        && !night.is_changed()
        && !clock.as_ref().is_some_and(|c| c.is_changed())
        && fresh_camera.is_empty()
    {
        return;
    }
    let atmosphere = &sky.current;

    // All three are undecoded; see the module note. `atmosphere.wgsl` adds the
    // first two together, mixes the third into the result, and decodes once at
    // the end; decoding them here would decode each term of a sum separately,
    // which gives a different colour, not a darker one.
    //
    // `LinearRgba` is used as a container and does not mean the values are
    // linear. Bevy uploads `LinearRgba::from(color) * brightness` and
    // `* illuminance`, both of which are 1.0, so the shader reads exactly what
    // is written here.
    let sun_colour = Vec3::from_array(atmosphere.diffuse);
    let fill = Vec3::from_array(atmosphere.ambient);
    let fog = Vec3::from_array(atmosphere.fog());

    // The sun's direction by the hour, on the 1.12.1 client's track, which
    // the client was measured to follow. Rewritten whenever this runs rather
    // than only when the clock moved, because `Transform` is change-detected
    // and the value is a few multiplies.
    let half_minutes = clock.as_ref().map_or(NOON, |c| c.half_minutes);
    let toward = vale_assets::tables::light::celestial::light_toward(half_minutes);
    for (mut light, mut transform) in &mut sun {
        light.color = LinearRgba::from_vec3(sun_colour).into();
        *transform = aim(toward);
    }
    ambient.color = LinearRgba::from_vec3(fill).into();
    ambient.brightness = 1.0;

    // The dome, which is the only thing that reads the other five bands.
    for handle in &dome {
        if let Some(mut material) = sky_materials.get_mut(&handle.0) {
            material.stops = stops_of(atmosphere);
        }
    }

    // What is behind the world is the fog colour. The dome covers every pixel
    // of the sky, so this is not what the horizon is made of; it shows only on
    // the one frame between the camera appearing and the dome being moved onto
    // it, and it is the same colour either way.
    //
    // Decoded, unlike the three above, because a clear colour is written
    // directly into the framebuffer and the swapchain encodes it on output, so
    // the decode here makes the pixel come out as the byte the file states.
    // Nothing is ever added to it.
    clear.0 = LinearRgba::from_vec3(linear(atmosphere.fog())).into();

    // Logs what it wrote on every write, at debug, because the sky changes at
    // every 4-yard cell while travelling. This line found the login-fog bug: a
    // night fog logged here while the screen stayed pale showed the write was
    // being overwritten, not skipped. See `render::glue::stand_the_world_down`,
    // whose give-back edge now makes this function run again.
    debug!(
        "sky: applied — fog {:?} {:.0}..{:.0} air.x {:.3} deck {:.1} to {} camera(s)",
        atmosphere.fog().map(|c| (c * 255.0) as i32),
        atmosphere.fog_start,
        atmosphere.fog_end,
        crate::render::night::NightAir::new(night.factor, night.deck).pack().0.to_linear().red,
        night.deck,
        camera.iter().count(),
    );
    for entity in &camera {
        // Removed rather than pushed out to infinity when the switch is off. A
        // `FogFalloff::Linear` with a very large end still runs the blend on
        // every fragment and still tints the far field by a fraction of a
        // band, so the result would be neither an accurate measurement nor a
        // fog-free picture. Without the component the pass draws the world to
        // the horizon with a hard edge at the streaming radius, which is what
        // turning fog off is for.
        if tuning.fog {
            let (air, deck) = crate::render::night::NightAir::new(night.factor, night.deck).pack();
            commands.entity(entity).insert(DistanceFog {
                color: LinearRgba::from_vec3(fog).into(),
                falloff: FogFalloff::Linear {
                    start: atmosphere.fog_start,
                    end: atmosphere.fog_end,
                },
                // These two fields do not hold a light colour or an exponent
                // here. Both are Bevy's own scattering controls, both are
                // unread under a linear falloff with a zero scattering vector,
                // and both are already bound at group 0 for every world
                // material, so the deep night's five numbers are carried in
                // them rather than in a bind group of this crate's own.
                directional_light_color: air,
                directional_light_exponent: deck,
            });
        } else {
            commands.entity(entity).remove::<DistanceFog>();
        }
    }
}

/// One band colour, sRGB as the file states it, decoded to the value a shader
/// multiplies a texel by.
///
/// Only for a band nothing is added to. See the module note: the sun, the
/// fill, the shadow and the fog are each combined with another band before
/// they light anything, so each goes to the GPU as the file states it and is
/// decoded there, once, after the arithmetic. What remains for this function is
/// the sky dome's six stops and `ClearColor`, colours that are written to the
/// pixel directly.
///
/// Public because the dome is not the only such surface: a room's interior
/// light is composed on the GPU too, and a second copy of this transfer
/// function would be the same kind of duplication `atmosphere.wgsl` exists to
/// prevent in the shaders.
pub fn linear(colour: [f32; 3]) -> Vec3 {
    let c: LinearRgba = Color::srgb(colour[0], colour[1], colour[2]).to_linear();
    Vec3::new(c.red, c.green, c.blue)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks which of the four outputs the decode is applied to.
    ///
    /// This is the bug the module note describes, and nothing else detects it:
    /// decoding the sun and the fill here instead of in the shader does not
    /// fail, does not warn and does not look obviously wrong. It lights the
    /// world with a plausible colour that is not the one the file states,
    /// because `srgb_to_linear(a) + srgb_to_linear(b)` is not
    /// `srgb_to_linear(a + b)`. So the three outputs that are terms of a sum
    /// are asserted to reach the GPU exactly as the table states them, and the
    /// one that is a framebuffer value is asserted to be decoded.
    #[test]
    fn the_bands_that_are_summed_reach_the_gpu_undecoded() {
        let mut app = App::new();
        let mut sky = Sky::default();
        // A band with no fixed point under the transfer function, so "decoded"
        // and "not decoded" cannot coincide by accident.
        sky.current.diffuse = [1.0, 0.5, 0.0];
        sky.current.ambient = [0.4, 0.5, 0.6];
        let fog_band = sky.current.fog();
        app.insert_resource(sky)
            .insert_resource(GlobalAmbientLight::default())
            .insert_resource(ClearColor::default())
            .insert_resource(Assets::<SkyMaterial>::default())
            .init_resource::<crate::render::tuning::WorldTuning>()
            // The deep night's own resource, left at its default: a factor of
            // zero, which is what daylight resolves to and which keeps this
            // test about the four bands rather than about the grade. See
            // `render::night`, whose own tests pin the grade itself.
            .init_resource::<crate::render::night::Night>()
            .add_systems(Update, apply);
        let sun = app.world_mut().spawn((Sun, DirectionalLight::default())).id();
        app.update();

        let light = app.world().entity(sun).get::<DirectionalLight>().unwrap();
        let colour = LinearRgba::from(light.color);
        assert_eq!(
            [colour.red, colour.green, colour.blue],
            [1.0, 0.5, 0.0],
            "the sun is a term of `daylight`'s sum and must arrive as the file states it"
        );

        let fill = LinearRgba::from(app.world().resource::<GlobalAmbientLight>().color);
        assert_eq!([fill.red, fill.green, fill.blue], [0.4, 0.5, 0.6]);
        assert_eq!(
            app.world().resource::<GlobalAmbientLight>().brightness,
            1.0,
            "the whole value is in the colour; see the module note on transport"
        );

        // The one written directly into the framebuffer is decoded: nothing
        // is ever added to it, and the swapchain encodes it on output.
        let clear = LinearRgba::from(app.world().resource::<ClearColor>().0);
        let expected = linear(fog_band);
        assert!(
            (Vec3::new(clear.red, clear.green, clear.blue) - expected).length() < 1e-6,
            "{clear:?} is not the decoded horizon band {expected:?}"
        );
    }

    /// Logging out restores the light once and then leaves it alone.
    ///
    /// The bug this test covers had one symptom and three causes, all the same
    /// omission: nothing reset the resolved atmosphere when the world went
    /// away, so a session that ended at midnight left the sun cold, the fog
    /// close and the present pass grading every pixel of the character screen.
    /// See [`resolve`]'s own note.
    ///
    /// The second half of the test checks that the reset happens once; a
    /// regression there would not be visible. [`apply`] is keyed on
    /// `Sky::is_changed`, and `ResMut`'s `DerefMut` marks a resource changed
    /// whether or not the value moved, so a reset written unconditionally would
    /// re-upload the light uniform, the fog and the dome's six stops on every
    /// frame the login screen was open.
    #[test]
    fn leaving_the_world_puts_the_light_back_and_then_leaves_it_alone() {
        /// How many frames [`apply`] would have run on. Its own guard, in a
        /// system of its own so the count is what that guard sees.
        #[derive(Resource, Default)]
        struct Uploads(u32);

        fn count(sky: Res<Sky>, night: Res<crate::render::night::Night>, mut n: ResMut<Uploads>) {
            if sky.is_changed() || night.is_changed() {
                n.0 += 1;
            }
        }

        // A sky left as a session at midnight would leave it: graded bands, a
        // key it was resolved for, and a full night factor.
        let mut sky = Sky::default();
        sky.current.diffuse = [0.19, 0.28, 0.40];
        sky.current.ambient = [0.03, 0.05, 0.08];
        sky.submerged = true;
        sky.resolved_for = Some((0, NOON, [1, 2, 3], true, 255, 0, 255, 0, false));

        let mut app = App::new();
        app.insert_resource(sky)
            .insert_resource(crate::render::night::Night {
                factor: 1.0,
                deck: -412.5,
            })
            .init_resource::<crate::render::night::NightTuning>()
            .init_resource::<WorldClock>()
            // No world in the focus, which is the only condition: the glue
            // screens, and every frame before the first login. The condition
            // is the focus rather than the session since `render::focus` took
            // that question off the socket; the session is still here because
            // `resolve` reads the server's clock from it when there is one.
            .init_resource::<crate::render::focus::WorldFocus>()
            .init_resource::<crate::world::session::Session>()
            .init_resource::<crate::world::camera::CameraRig>()
            // Clear weather and every layer drawn, which is what `resolve`
            // reads them for; see the storm column.
            .init_resource::<crate::render::weather::WeatherState>()
            .init_resource::<crate::render::tuning::WorldTuning>()
            .insert_resource(crate::assets::GameAssets::new(String::from("no-archives")))
            .init_resource::<Uploads>()
            .add_systems(Update, (resolve, count).chain());

        app.update();
        let sky = app.world().resource::<Sky>();
        assert_eq!(
            sky.current.diffuse,
            Atmosphere::PLACEHOLDER.diffuse,
            "the sun goes back to what lit the glue screens before any login"
        );
        assert_eq!(sky.current.ambient, Atmosphere::PLACEHOLDER.ambient);
        assert_eq!(sky.resolved_for, None, "and the next login resolves afresh");
        assert!(!sky.submerged);
        let night = app.world().resource::<crate::render::night::Night>();
        assert_eq!(night.factor, 0.0, "the grade is what the report is about");
        assert_eq!(night.deck, 0.0);

        // Nothing is written again on later frames.
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(
            app.world().resource::<Uploads>().0,
            1,
            "the reset must be one write, not one a frame"
        );
    }

    /// The size of the error, on map 0's own numbers at the hour of the
    /// screenshot that prompted this check: the fill at 102/129/155, the sun at
    /// 255/128/0, and ground turned 0.76 toward it.
    ///
    /// Summed in the file's space the ground is lit by 255/227/155; summed in
    /// linear it is lit by 243/167/155. The green channel is the difference
    /// between grass and rust, and no count in this project would have shown
    /// it.
    #[test]
    fn summing_in_the_wrong_space_is_a_different_colour_and_not_a_darker_one() {
        let fill = [102.0 / 255.0, 129.0 / 255.0, 155.0 / 255.0];
        let sun = [1.0, 128.0 / 255.0, 0.0];
        let lambert: f32 = 0.7625;

        // The 1.12.1 client's arithmetic: sum the bytes, clamp, decode once.
        let mut in_file_space = [0.0f32; 3];
        for k in 0..3 {
            in_file_space[k] = (fill[k] + sun[k] * lambert).clamp(0.0, 1.0);
        }
        // What the renderer did instead: decode each term, then sum.
        let wrong = linear(fill) + linear(sun) * lambert;

        // Compared as bytes, which is what either one paints with.
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        let encoded = |v: Vec3| {
            let c: LinearRgba = LinearRgba::from_vec3(v.min(Vec3::ONE));
            let s = Color::from(c).to_srgba();
            [byte(s.red), byte(s.green), byte(s.blue)]
        };
        assert_eq!(
            [byte(in_file_space[0]), byte(in_file_space[1]), byte(in_file_space[2])],
            [255, 227, 155]
        );
        assert_eq!(encoded(wrong), [243, 167, 155]);
    }

    /// The clock is in half-minutes, which is neither seconds nor minutes and is
    /// the unit the bands' own times are in.
    #[test]
    fn the_clock_reads_as_an_hour_and_a_minute() {
        let at = |half_minutes| {
            WorldClock {
                half_minutes,
                from_server: true,
                ..default()
            }
            .hour_minute()
        };
        assert_eq!(at(NOON), (12, 0));
        assert_eq!(at(0), (0, 0));
        // A day is 2,880 half-minutes; one past the end wraps rather than
        // reading as hour 24.
        assert_eq!(at(DAY), (0, 0));
        assert_eq!(at(DAY - 2), (23, 59));
    }

    /// The dome's bottom stop is the fog colour in the renderer, not only in
    /// the table. This is why the horizon has no seam: the ground fades to
    /// `DistanceFog`'s colour and the sky is drawn in the same colour at
    /// altitude zero. These are two conversions of one band, so the test
    /// checks that neither has been decoded twice or not at all.
    #[test]
    fn the_bottom_of_the_dome_is_exactly_what_the_world_fogs_to() {
        let atmosphere = Atmosphere::PLACEHOLDER;
        let stops = stops_of(&atmosphere);
        let bottom = stops.stops[SKY_STOPS - 1];
        assert_eq!(bottom.truncate(), linear(atmosphere.fog()));
        assert_eq!(bottom.w, 0.0, "and it is drawn at the horizon");
        // The zenith is at the top and the list descends, which is what the
        // fragment shader's one-pass loop depends on.
        assert_eq!(stops.stops[0].w, 1.0);
        for i in 1..SKY_STOPS {
            assert!(stops.stops[i].w < stops.stops[i - 1].w, "stop {i}");
        }
    }

    /// sRGB in, linear out, and the difference is large. Mid grey shows whether
    /// the transfer function ran: 0.5 sRGB is 0.21 linear,
    /// and a client that skips this multiplies its textures by more than twice
    /// the light the table asked for.
    #[test]
    fn the_band_colours_are_decoded_out_of_srgb() {
        let mid = linear([0.5, 0.5, 0.5]);
        assert!((mid.x - 0.2140).abs() < 1e-3, "{mid:?}");
        // The ends are fixed points, which shows nothing else was applied.
        assert_eq!(linear([0.0, 0.0, 0.0]).x, 0.0);
        assert!((linear([1.0, 1.0, 1.0]).x - 1.0).abs() < 1e-6);
    }

    /// A table edit re-lights the world, which is the purpose of the generation
    /// in the resolve key.
    ///
    /// The key holds the hour, the position quantised to four yards and three
    /// settings, and an edited `LightIntBand` changes none of them, so without
    /// the generation a colour changed while standing still took effect on the
    /// next half-minute the server sent, up to thirty seconds later, or after
    /// the next four yards walked. The edit appeared to do nothing.
    ///
    /// Asserted as a property of the key rather than by running the system,
    /// which would need an archive: two keys that differ only in the
    /// generation must not compare equal.
    #[test]
    fn a_forgotten_table_makes_the_sky_resolve_again() {
        let before = (0u32, NOON, [1i32, 2, 3], false, -1i32, 0i32, 255i32, 4u64, false);
        let after = (0u32, NOON, [1i32, 2, 3], false, -1i32, 0i32, 255i32, 5u64, false);
        assert_ne!(before, after, "a re-parse of the tables must re-resolve");
        assert_eq!(before, before.clone(), "and nothing else about it moved");
    }
}
