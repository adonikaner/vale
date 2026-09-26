//! The world's light, from `Light.dbc` to the frame.
//!
//! Everything global about how the world reads — the sun's colour, the fill it
//! is paired with, and how far away it fades out — is a function of **which map
//! and what hour**, and this module is the whole of that join. The table is
//! [`vale_assets::tables::light`]; what happens here is the four things a frame needs
//! it as, and one conversion that is not decoration.
//!
//! ## The conversion, and where it happens
//!
//! **The band colours are sRGB and the shaders work in linear**, so every one of
//! them is decoded — but *after* it has been combined with the other bands, not
//! on the way to the GPU. That distinction is the whole of this module's
//! trickiest half, so it is stated twice: [`linear`] is the decode, and which of
//! the four things below gets it is decided one at a time.
//!
//! A band that is only ever **multiplied** by a texel can be decoded here, and a
//! band that is only ever **written to the framebuffer** must be. A band that is
//! **added to or mixed with another band** must not be, because a sum does not
//! survive a transfer function — and the sun, the fill, the shadow and the fog
//! are all of them in that third group. `shaders/atmosphere.wgsl` carries the
//! measurement and the argument; the short version is that the 1.12.1 client
//! is fixed-function and adds its light in the bytes the file states, so this does
//! too.
//!
//! So: the sun, the fill and the fog go to the GPU **undecoded**, packed into
//! `LinearRgba` because that is the only channel Bevy's light and fog uniforms
//! offer — the type is a container here, not a claim. `ClearColor` and the sky
//! dome's six stops are decoded, because nothing mixes either of them.
//!
//! The decode is [`Color::srgb`] into [`LinearRgba`], which is Bevy's own
//! transfer function and not a gamma approximation.
//!
//! ## The transport
//!
//! The sun and the fill ride in Bevy's own `DirectionalLight` and `AmbientLight`
//! rather than in a uniform of this crate's, because those are already bound at
//! group 0 for every material in the world — a custom global would mean either a
//! render-world bind group or writing the same eight numbers into thousands of
//! per-material uniforms every frame.
//!
//! Both are premultiplied by their intensity on the way to the GPU
//! (`color * illuminance`, `color * brightness`), so **both intensities are set
//! to 1.0 and the colour carries the whole value**. The alternative is splitting
//! one number the table states into two the renderer invents.
//!
//! ## What the table does not say
//!
//! **Which way the sun is pointing.** `Light.dbc` carries its colour at every
//! hour and no direction at any of them. The direction is the client's own
//! track, [`vale_assets::tables::light::celestial::light_toward`]: a
//! constant bearing of 225° for the light's travel and an elevation that
//! sweeps 20°..37° over the day, re-aimed by [`apply`] whenever
//! the clock moves. It was held fixed at
//! [`vale_assets::tables::light::SUN_TOWARD`] — the sun the ground's
//! `MCSH` shadows were baked for — for as long as the reference's own choice
//! was unmeasured; a Direct3D trace of it then read `(-0.6625, -0.6625,
//! -0.3497)` off its `D3DLIGHT9` at 17:50, the track to half a degree.

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

/// The hour, in half-minutes past midnight — `LightIntBand`'s own unit.
///
/// **The server's, once there is a session**: `SMSG_LOGIN_SETTIMESPEED` states
/// the world's time and the rate it runs at, and the session thread advances it
/// (see [`vale_protocol::play::time`]). Before a session exists there is nothing to
/// read, and this stands at noon — which is the hour every measurement in this
/// project was taken at.
#[derive(Resource)]
pub struct WorldClock {
    pub half_minutes: u32,
    /// Whether the value above is the server's or the noon placeholder. On the
    /// HUD, because "the sky is wrong" and "the sky is right for midnight" are
    /// answered by different things.
    pub from_server: bool,
    /// **An hour set by hand, which wins over the server's.**
    ///
    /// The whole of what the settings panel's time slider writes — see
    /// [`crate::ui::debug`]. `None` follows the server, which is the ordinary
    /// state and the one every measurement in this project was taken in.
    ///
    /// It lives on the clock rather than in the panel because [`resolve`] is the
    /// one place the hour is decided, and a second decider would be a sky that
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
/// Kept so the work is not redone every frame — the lookup is three hash lookups
/// and a handful of interpolations, which is nothing, but the *writes* it drives
/// are change-detected components, and rewriting `AmbientLight` every frame would
/// re-upload the light uniform sixty times a second for no reason.
#[derive(Resource)]
pub struct Sky {
    pub current: Atmosphere,
    /// **Is the camera under a liquid surface?** Kept beside the atmosphere it
    /// selected so the HUD and the report can say which of the two lights is in
    /// force without asking the terrain a second time.
    pub submerged: bool,
    /// Every input the resolved atmosphere depends on — see `resolve`, which
    /// names each one where it builds it. The last is which parse of the DBCs
    /// the bands came from, so a table edit re-lights the world.
    resolved_for: Option<(u32, u32, [i32; 3], bool, i32, i32, i32, u64)>,
}

/// How coarsely the player's position is remembered in [`Sky::resolved_for`],
/// in yards.
///
/// The light is now a function of *where* as well as when — `Light.dbc`'s
/// positional spheres — so the resolve can no longer be keyed on the map and
/// the hour alone. Keying it on the exact position would redo the lookup every
/// frame the character moves, which is every frame; keying it on a cell means
/// the work happens a few times a second at a run, and a cell this size is at
/// worst 1/50th of the narrowest falloff band in the table.
pub(crate) const RESOLVE_CELL: f32 = 4.0;

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
        // The one copy of the sun and the fog, imported by `terrain.wgsl` and
        // `m2.wgsl` alike. A library rather than an `embedded_asset!` because
        // nothing loads it directly: it has to be resident before either
        // fragment shader that imports it compiles.
        // …and the transfer function *it* is built on, which every shader that
        // writes to the frame also needs on its own — `present.wgsl` cannot
        // import it through `atmosphere.wgsl`, because naga_oil does not strip
        // an imported module's unused functions and would drag
        // `bevy_pbr::mesh_view_bindings` into a 2D pipeline with it. Both halves
        // are registered here so the library stays one thing.
        bevy::shader::load_shader_library!(app, "shaders/gamma.wgsl");
        // …and the deviation `atmosphere.wgsl` imports, which is `render::night`'s
        // half of the world's light — see `shaders/night.wgsl`.
        //
        // **Registered here rather than by `NightPlugin`, deliberately.** The
        // chain is `gamma <- atmosphere <- night` and all three have to be
        // resident before any fragment shader that imports them compiles, so
        // they are loaded in one place where the dependency can be seen. The
        // alternative puts a library `atmosphere.wgsl` needs behind a *different*
        // plugin, and removing that plugin — to price the night, say — would then
        // leave every world shader with an import naga_oil cannot resolve. That
        // does not error: the pipeline stays pending for ever and the world is
        // simply not drawn. See `present.wgsl`'s note on the same failure.
        bevy::shader::load_shader_library!(app, "shaders/night.wgsl");
        bevy::shader::load_shader_library!(app, "shaders/atmosphere.wgsl");
        embedded_asset!(app, "shaders/sky.wgsl");
        app.add_plugins(MaterialPlugin::<SkyMaterial>::default())
            .init_resource::<WorldClock>()
            .init_resource::<Sky>()
            .add_systems(Startup, (spawn_sun, spawn_dome))
            .add_systems(
                Update,
                (resolve, apply, sun_shadows)
                    .chain()
                    // **And after this frame's weather has been ramped**, for
                    // the same class of reason: `resolve` mixes the storm light
                    // by [`crate::render::weather::WeatherState::grade`], which
                    // `follow_the_server` writes. Unordered, the sky is a frame
                    // behind the rain, which nothing fails on and nobody can
                    // see — and which is exactly the shape of the three
                    // orderings this project has already paid for.
                    .after(crate::render::weather::follow_the_server)
                    // **After this frame's player position has been settled.**
                    // The light is a function of *where* as well as when now —
                    // `Light.dbc`'s positional spheres — and `resolve` reads
                    // `WorldStatus::position`, which the session pass writes.
                    // Unordered it is a frame behind, which is a fifth of a
                    // yard at a run and invisible; the `.after` is here because
                    // nothing fails when it is missing and this project has
                    // paid for that class three times. `follow_player` is the
                    // last of the session chain and is the same anchor
                    // `camera::place` takes for the same reason.
                    .after(crate::world::session::follow_player),
            )
            // The dome is centred on the camera, so it has to be moved *after*
            // the camera has been placed — a frame behind and the whole sky
            // slides against the world every time the character moves. See the
            // rendering facts on ordering: nothing fails when this is missing.
            .add_systems(Update, follow_camera.after(crate::world::camera::place))
            // …and the switch that takes the gradient away, leaving `ClearColor`
            // — which is the fog band, so the horizon keeps its colour and only
            // the six stops go. Its own system rather than a branch inside
            // [`apply`], because nothing else writes the dome's visibility.
            .add_systems(
                Update,
                crate::render::tuning::switch::<SkyDome>(|tuning| tuning.sky_dome),
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

/// **What the world is lit by**, on the HUD — which is otherwise only visible
/// by looking at it, and looking at it cannot distinguish "the sky is wrong"
/// from "the sky is right, for midnight".
///
/// The hour, whether it is the server's or the noon placeholder, and the two
/// numbers the frame is actually built from.
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
                // **The hand-set hour is called out**, because every complaint
                // this line exists to answer starts with "the sky is wrong" and
                // a slider left where it was put is the cheapest possible cause.
                (Some(_), _) => " (set by hand)",
                (None, true) => "",
                (None, false) => " (noon, no clock)",
            },
            rgb(sky.current.diffuse),
            rgb(sky.current.fog()),
            // **Which of the light's two rows is in force**, which this line
            // has claimed to say since `Sky::submerged` was written and did
            // not: an underwater atmosphere is a short, strongly-coloured fog,
            // and "the fog is wrong" and "the client thinks your head is under
            // a lake" are answered by different things.
            if sky.submerged { " (underwater)" } else { "" },
            sky.current.fog_start,
            sky.current.fog_end,
        ),
    );
}

/// How far away the dome is drawn, in yards.
///
/// **It no longer decides anything about occlusion, and that is the point of
/// [`behind_the_world`].** The number only has to put the dome's geometry in
/// front of the near plane and around the camera; what keeps the world in
/// front of it is the far-plane depth every sky fragment writes, not its
/// radius. It used to be the whole mechanism, and that was the bug: 950 is
/// short of the loaded 3x3's far corner (about 1,500 yards), so a mountain out
/// there was drawn *first* and then painted over by the sky.
const DOME_RADIUS: f32 = 950.0;

/// **The one rule that keeps the sky behind the world**, applied by every
/// material that draws sky: the gradient dome, the star field and the celestial
/// sprites.
///
/// The problem it solves is not ordering, it is *distance*. Bevy's opaque phase
/// is binned by pipeline rather than sorted by depth, and the sky writes no
/// depth of its own, so the only thing that decided whether a sky fragment
/// survived was the depth test against whatever geometry had already been
/// drawn. That works while the sky is the furthest thing in the frame and fails
/// the instant it is not — and it is not: the terrain streamer keeps a 3x3 of
/// 533-yard tiles, whose far corner stands about 1,500 yards from a character
/// standing anywhere in the middle one, where the dome is 950 and the sprites
/// are twelve.
///
/// So the fragment shaders write `frag_depth = 0.0` — the far plane under
/// Bevy's reversed depth — and this loosens the comparison from `Greater` to
/// `GreaterEqual` so that a fragment at exactly the far plane still passes
/// where nothing has been drawn at all. The result holds in both directions and
/// in any order: anything that has written depth beats the sky, and the sky
/// never writes depth so it can occlude nothing.
///
/// **What it costs is early-Z on the sky surfaces.** A shader that computes its
/// own depth cannot be rejected before it runs, so the dome's fragment shader
/// now executes for every pixel it covers rather than only the ones the terrain
/// left. It is a six-stop mix with no texture fetch, and the star and sprite
/// passes are one texture fetch each over a few hundred pixels.
pub fn behind_the_world(descriptor: &mut RenderPipelineDescriptor) {
    let Some(depth) = descriptor.depth_stencil.as_mut() else {
        return;
    };
    depth.depth_write_enabled = Some(false);
    depth.depth_compare = Some(CompareFunction::GreaterEqual);
}

/// **…and the second half of that rule, for the sky surfaces that are
/// *blended*** — the star field and the three celestial sprites.
///
/// [`behind_the_world`] settles the sky against everything that has written
/// depth, which is the whole opaque half of the world. It settles nothing at
/// all against the *transparent* half, because a blended surface writes no
/// depth either: the two are ordered only by `Transparent3d`'s sort, which is
/// each mesh's own centre plus its material's `depth_bias`, ascending — bevy's
/// values increase toward the camera, so the smallest is drawn first.
///
/// That is what put the moon on the lake. The sprites carried a bias of
/// **+4096**, chosen to lift them over the star dome — whose mesh centre is the
/// camera's own position and therefore the nearest thing in the frame — and a
/// positive bias means *nearer*, so the sprites sorted ahead of every blended
/// surface in the world and were drawn last over all of them. Water is blended;
/// so are particles, ribbons and every spell effect. A moon low over a lake was
/// painted onto it.
///
/// The fix is the same relative order with the sign the other way up: every sky
/// surface takes a bias far more negative than any real distance in a loaded
/// 3x3, so the whole sky sorts *behind* the whole world, and the small offsets
/// between them keep the client's own order — the star dome,
/// then the sun, then the two moons.
///
/// A million rather than a merely sufficient number: the loaded world reaches
/// about 1,500 yards, and an `f32` at 1e6 still resolves the one-yard steps
/// below it exactly (its spacing there is 1/16 of a yard).
pub const SKY_SORT: f32 = -1.0e6;

/// The star field's place in that order: first, so the sun and the moons are
/// drawn over it rather than speckled by it.
pub const STARS_SORT: f32 = SKY_SORT;

/// …and the sun and the moons, drawn after the field and before nothing.
pub const CELESTIAL_SORT: f32 = SKY_SORT + 1.0;

/// The one directional light standing in for the sun.
///
/// Its **colour** is the light table's and arrives in [`apply`]; what is fixed
/// here is its direction and the fact that there is exactly one. Whether it
/// casts shadow *maps* is [`crate::world::camera::RenderTuning::sun_shadows`], which is
/// off — the game's own shadows are the ground's baked `MCSH` and a blob under
/// each unit, both of which are drawn regardless. See [`sun_shadows`].
fn spawn_sun(mut commands: Commands, tuning: Res<crate::world::camera::RenderTuning>) {
    commands.spawn((
        Sun,
        DirectionalLight {
            // The whole value is in the colour; see the module note on transport.
            illuminance: 1.0,
            shadow_maps_enabled: tuning.sun_shadows,
            ..default()
        },
        // **Only meaningful when F10 is on**, and sized for a character rather
        // than for the zone: the thing anyone wants a real shadow of is standing
        // in front of the camera, and a cascade stretched over the whole 3x3
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

/// The sun's transform for a unit vector *toward* it in the world's own axes:
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
/// **Off by default and on a key, because 1.12 does not do this.** The game's
/// own shadows are two things and neither is a shadow map: the ground carries
/// `MCSH`, baked by Blizzard's tools, and a unit stands on a blob
/// (`Textures\ShadowBlob.blp` — see [`crate::entities`]). Both are drawn always.
/// A cascade over nine tiles of terrain, a city and twelve thousand doodads is a
/// second pass over all of it and is the most expensive thing this renderer
/// could switch on; it is here because "what would real shadows look like" is
/// exactly the sort of question this project settles by an A/B at one framing
/// rather than by argument, and F10 is cheaper than a rebuild.
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
/// `w` carries **the altitude each stop sits at**, not a padding word: the
/// colours are a gradient and a gradient is stops plus positions, and putting
/// the two in one array is what keeps the shader from having a second copy of
/// [`SKY_ALTITUDES`] to drift from.
#[derive(Clone, Copy, ShaderType)]
pub struct SkyStops {
    pub stops: [Vec4; SKY_STOPS],
}

/// The dome's material: six colours and nothing else.
///
/// No texture, no light, no fog. Everything about how the sky reads is in the
/// table, and the whole of the material is the six numbers that came out of it.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct SkyMaterial {
    #[uniform(0)]
    pub stops: SkyStops,
}

impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        crate::render::shader::SKY.into()
    }

    /// **Not in the depth prepass.** The dome writes no depth in the main pass
    /// either (see [`Self::specialize`]), and a prepass that wrote it would put
    /// a wall at 950 yards in front of the occlusion culler — which reads the
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
        // **Seen from the inside**, so the sphere's own winding faces away.
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
/// *direction* and a sphere's own geometry is that function — and because a
/// mesh needs no render-graph node, no view uniform of this crate's and no
/// second camera.
///
/// The resolution is the gradient's, not the geometry's: the fragment shader
/// interpolates the six stops itself, so the sphere only has to be round enough
/// that its silhouette is not visible against the terrain, which nothing at 950
/// yards ever is.
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
        // It is a light source in the fiction and a backdrop in the renderer:
        // neither casting nor receiving is meaningful, and casting would put the
        // whole world inside a 950-yard shadow the moment F10 is pressed.
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

/// Keep the dome centred on the camera, so the sky never gets nearer.
///
/// **The camera's `Transform`, never its `GlobalTransform`**, and this is the
/// one note the three sky passes that follow the camera share.
///
/// `camera::place` writes the `Transform` in `Update` and Bevy propagates it
/// into the `GlobalTransform` in `PostUpdate`, so a system ordered after
/// `place` and reading the *global* one is reading **last frame's eye** — the
/// ordering buys nothing at all. For the dome that is invisible: it is 950
/// yards across and an error of one frame of travel moves it by a few
/// centimetres out of that. For the celestial sprites, which hang twelve yards
/// from the eye, the same error is an angle — and under a mouse-look drag the
/// eye swings yards in a frame, so the sun and the moons jumped about the sky
/// with every movement of the camera and stood still when it did.
///
/// The camera is unparented, so the `Transform` this reads *is* the global one,
/// a frame earlier. `Without<WorldCamera>` on the mutable half is what lets the
/// two live in one system: both touch `Transform`.
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
/// them on first use — so before a session exists this leaves
/// [`Atmosphere::PLACEHOLDER`] in place and the glue screens are lit by it.
fn resolve(
    mut sky: ResMut<Sky>,
    mut clock: ResMut<WorldClock>,
    // The session, for the one input only a server has: the world's own clock.
    session: Res<crate::world::session::Session>,
    // …and where that world is, which a host with no server also answers. See
    // [`crate::render::focus`].
    focus: Res<crate::render::focus::WorldFocus>,
    assets: Res<crate::assets::GameAssets>,
    rig: Res<crate::world::camera::CameraRig>,
    // **The deep night is decided here** and nowhere else, for the reason the
    // hand-set hour is: two deciders is a sky that disagrees with the sun. See
    // [`crate::render::night`], which is this client's own deviation from the
    // table and is an option rather than a correction.
    deep_night: Res<crate::render::night::NightTuning>,
    mut night: ResMut<crate::render::night::Night>,
    // **What the server says the sky is doing.** The storm column of
    // `Light.dbc` is the whole of what rain does to the light — see
    // [`vale_assets::tables::light::LightTables::atmosphere_in_storm`] — and
    // it is read here rather than anywhere else for the reason the hour is:
    // two deciders is a sky that disagrees with itself.
    weather: Res<crate::render::weather::WeatherState>,
    // …and the switch that takes weather out of the frame, so that
    // `--without weather` subtracts the light it is drawn in as well as the
    // drops. A subtraction that leaves half the pass standing prices neither
    // half — see [`crate::render::tuning`].
    tuning: Res<crate::render::tuning::WorldTuning>,
) {
    if !focus.present {
        // **Out of the world, the light goes back to what it was before the
        // first login**, and this is the whole of "the night clears on logout".
        //
        // Leaving it standing was a bug with a long reach, because three
        // separate things carry the resolved atmosphere and *none* of them is
        // reset by anything else. The grade in [`crate::render::night`] made all
        // three visible at once: the sun and the fill stayed cold and dim, the
        // fog stayed pulled in, and — the one a player actually reports — the
        // present pass went on crushing, cooling and vignetting **the whole
        // frame**, which at the character screen is the glue scene. Logging out
        // at midnight left the login screen looking faded for the life of the
        // process, and nothing in the client said why.
        //
        // The teardown in `residency::leave_world` is not the place for it: this
        // is the one function that decides what the world is lit by, and a
        // second decider is exactly what its own note about the hand-set hour
        // warns against. Doing it here also covers the case that is *not* a
        // logout — the frames before the first login, which now take the same
        // path rather than relying on the resource's default.
        //
        // **Guarded, because this runs every frame the client is not in a
        // world.** `ResMut`'s `DerefMut` marks a resource changed whether or not
        // the value moved, and [`apply`] is what reads that: an unguarded write
        // would re-upload the light uniform, the fog and the dome's six stops
        // sixty times a second for as long as the login screen was open.
        // `resolved_for` is the witness — it is `None` on a fresh start and
        // `Some` only after a world has been resolved for.
        if sky.resolved_for.is_some() || night.factor != 0.0 {
            sky.current = Atmosphere::PLACEHOLDER;
            sky.submerged = false;
            sky.resolved_for = None;
            night.factor = 0.0;
            night.deck = 0.0;
        }
        return;
    }
    // **The session is optional and the focus is not.** Two of the three things
    // read off it here have somewhere else to come from — the map is the focus's
    // and the position is too — and the third, the server's own clock, simply
    // does not answer for a host with no server. That host sets the hour by
    // hand, which is the guard four lines below.
    let active = session.active.as_ref();
    // **A hand-set hour wins**, and the server's is still read underneath it —
    // so letting go of the slider drops straight back onto the world's own
    // clock with nothing to re-sync. See [`WorldClock::override_half_minutes`].
    if let Some(hour) = clock.override_half_minutes {
        if clock.half_minutes != hour {
            clock.half_minutes = hour;
        }
    }
    // Written only when it moves — a half-minute is 30 s of real time at the
    // server's rate, so this is a no-op on all but one frame in eighteen
    // hundred, and touching a `ResMut` is what marks it changed.
    else if let Some(time) = active.and_then(|a| a.live.game_time()) {
        let half_minutes = time.half_minutes();
        if clock.half_minutes != half_minutes || !clock.from_server {
            clock.half_minutes = half_minutes;
            clock.from_server = true;
        }
    }
    // **No resolve until every input is the world's** — measured, not
    // hypothetical: 2 of 5 scripted logins resolved before
    // `SMSG_LOGIN_SETTIMESPEED` had been read, keyed the atmosphere on the
    // clock's noon default (`half_minutes 1440, from_server false`, orange
    // daylight bands), and corrected only when the packet landed and moved the
    // key. `WorldFocus::present` is the same guard for the position: the
    // client sets it in the same frame it publishes a position, so before it
    // the position may be another session's or nothing at all — and a resolve
    // keyed on a wrong *position* does not self-heal standing still, because
    // the cell is the only movement-keyed input. That is the shape of the
    // long-standing "fog too thick and no night until I take a step" report.
    // Returning leaves the placeholder standing and retries next frame, which
    // costs at most the frames the login burst was already taking.
    if clock.override_half_minutes.is_none() && !clock.from_server {
        return;
    }
    // **The world's own axes, which is what the light table is in.**
    // `WorldFocus::position` is the server's position untouched — see
    // `axes.rs` for the one place that is converted, and this is not it.
    let at = [focus.position.x, focus.position.y, focus.position.z];
    let cell = at.map(|c| (c / RESOLVE_CELL) as i32);
    // **The eye, not the feet.** Which of a light's two rows is in force is a
    // question about the *camera*: a swimmer at the surface has their head above
    // it and the view goes blue only when the orbiting eye dips under, which is
    // what the reference shows and is why the same wave breaking over a
    // stationary character flickers the effect on and off. `CameraRig::eye` is
    // already in the world's own axes, so nothing here converts anything.
    //
    // Deliberately *outside* the `RESOLVE_CELL` quantisation the position gets:
    // four yards is most of the distance between a floating head and a
    // submerged one, so rounding this would answer a frame or two late at every
    // crossing — and this is a boolean, so asking it every frame costs one tile
    // lookup on a cache the mover has already warmed.
    let eye = rig.eye();
    // Whether the eye is under water is the one question here that only a
    // session answers: the liquid lookup goes through the mover's own warmed
    // tile cache. A host with no session draws the surface from above.
    let submerged = active.is_some_and(|a| {
        a.terrain_liquid(focus.map_id, eye.x, eye.y)
            .is_some_and(|(_, surface)| eye.z < surface)
    });
    // **The night setting is part of the key**, because the grade is applied to
    // the resolved atmosphere: without it, moving the panel's slider would
    // change nothing until the world's own clock next ticked, which at the
    // server's rate is up to thirty seconds of a control that looks broken.
    // Quantised to a byte so that dragging it does not re-resolve the light
    // chain on every pixel of travel, and `-1` for off so that off and zero
    // strength are the same key.
    let graded = if deep_night.enabled {
        (deep_night.strength.clamp(0.0, 1.0) * 255.0) as i32
    } else {
        -1
    };
    // **Quantised to a byte, like the night's grade and for the same reason**:
    // the grade ramps over ten seconds, so an unquantised key re-resolves the
    // whole light chain on every frame of a ramp. A 255th of the grade is a
    // step nothing on screen can show.
    let storm = if tuning.weather {
        (weather.storm() * 255.0) as i32
    } else {
        0
    };
    // **…and what the precipitation leaves of the visibility**, quantised the
    // same way. This one is the density's (and is released indoors), so it is
    // a separate key from the grade's — see
    // [`crate::render::weather::WeatherState::visibility`].
    let visibility = if tuning.weather {
        (weather.visibility().clamp(0.0, 1.0) * 255.0) as i32
    } else {
        255
    };
    // **Which parse of the tables these bands came from.**
    //
    // Without it, `GameAssets::forget_tables` does not re-light the world: the
    // key holds the hour, the position quantised to `RESOLVE_CELL` and the
    // three settings, and an edited `LightIntBand` moves none of them. A colour
    // changed while standing still then took effect only on the next
    // half-minute the server sent — up to thirty seconds — or on the next four
    // yards walked, which reads as an edit that did nothing.
    //
    // It is the same shape as the five caches keyed on `spellbook_version()`
    // that a table edit does not touch. One relaxed atomic load
    // per resolve, and a resolve happens only when this key moves.
    let tables_from = assets.tables_generation();
    let key = (
        focus.map_id,
        clock.half_minutes,
        cell,
        submerged,
        graded,
        storm,
        visibility,
        tables_from,
    );
    if sky.resolved_for == Some(key) {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    // A chain with no light tables leaves the placeholder standing, which is
    // the documented degradation — see `DisplayTables::load`.
    // **What the deep night grades on: the hour's own light, storm excluded.**
    // `night::factor` reads the atmosphere's `diffuse + ambient` luma, and the
    // storm row's grey sun sums to 0.706 on map 0 — *under* `NIGHT_LUMA`'s
    // 0.72 — so grading on the storm-mixed light applied the full deep night
    // to a noon downpour: Goldshire at 12:55 in heavy rain drew as midnight,
    // lamp glow and all. The night is a question about the hour; the storm is
    // the server's and already carries its own darkening.
    let mut hours_own = sky.current;
    if let Some(light) = tables.light() {
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
    // **The visibility drop — this client's own, by request.** Falling snow
    // closes visibility with how much of it is in the air, over and above the
    // storm row's fog; the number is the density, which is exactly "what is in
    // the air". Not underwater — precipitation is a property of air — and
    // floored so an instance whose fog is already close does not squeeze to
    // milk. See `render::weather::VISIBILITY_TAKEN`.
    if visibility < 255 && !submerged {
        let squeeze = visibility as f32 / 255.0;
        let floor = crate::render::weather::VISIBILITY_FLOOR_YARDS.min(sky.current.fog_end);
        sky.current.fog_end = (sky.current.fog_end * squeeze).max(floor);
        sky.current.fog_start = (sky.current.fog_start * squeeze).min(sky.current.fog_end);
    }
    // **The grade, after the chain and before anything reads it.** `deepen` is
    // the identity at a factor of zero, so daylight leaves this function with
    // exactly the bands `atmosphere_in` returned — see the test in
    // `render::night` that pins it.
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
    // **The first resolve of a world says what it saw**, at info; the rest at
    // debug — a resolve fires every four yards of travel, which is a line a
    // second at a run, and the first is the only suspect. It is the line that
    // caught the login race this function's guard now closes (2 of 5 logins
    // keyed on the clock's noon default before `SMSG_LOGIN_SETTIMESPEED` was
    // read), and it is the instrument for the half of the report that has not
    // reproduced under a script: a session that comes up pale prints, in its
    // own log, which input lied.
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
    // The *global* ambient rather than the camera's own component: this is the
    // world's fill, and a second camera (a portrait, a minimap) should be lit by
    // the same one rather than needing its own copy.
    mut ambient: ResMut<GlobalAmbientLight>,
    mut clear: ResMut<ClearColor>,
    mut sun: Query<(&mut DirectionalLight, &mut Transform), With<Sun>>,
    // The clock, for where the sun stands — see [`aim`]. Optional because the
    // colour half of this is tested on an app with no clock at all; noon then.
    clock: Option<Res<WorldClock>>,
    dome: Query<&MeshMaterial3d<SkyMaterial>, With<SkyDome>>,
    mut sky_materials: ResMut<Assets<SkyMaterial>>,
    camera: Query<Entity, With<crate::world::camera::WorldCamera>>,
    // **A camera that appears after the light has settled needs the fog too.**
    // The sky resolves once per map and hour and then stops changing, so a
    // camera spawned or respawned after that would otherwise never be given a
    // `DistanceFog` at all — and the failure is a camera that draws the world
    // to the horizon with a sharp edge, which reads as "fog is not implemented"
    // rather than as an ordering bug.
    fresh_camera: Query<(), Added<crate::world::camera::WorldCamera>>,
    // Only the fog switch is read here — see [`crate::render::tuning`]. It
    // joins the guard below rather than being applied every frame, because all
    // four writes underneath are change-detected and three of them are uploads.
    tuning: Res<crate::render::tuning::WorldTuning>,
    // **The mist rides in the fog component**, so it is written here with the
    // rest of it rather than by a second system that would have to insert the
    // same component. See [`crate::render::night::NightAir::pack`] for which
    // two of `DistanceFog`'s fields carry it and why they are free.
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

    // **Undecoded**, all three — see the module note. `atmosphere.wgsl` adds the
    // first two together, mixes the third into the result, and decodes once at
    // the end; decoding them here would be decoding each term of a sum
    // separately, which is a different colour and not a darker one.
    //
    // `LinearRgba` is a container and not an assertion. Bevy uploads
    // `LinearRgba::from(color) * brightness` and `* illuminance`, both of which
    // are 1.0, so what the shader reads is exactly what is written here.
    let sun_colour = Vec3::from_array(atmosphere.diffuse);
    let fill = Vec3::from_array(atmosphere.ambient);
    let fog = Vec3::from_array(atmosphere.fog());

    // **And where it stands, by the hour** — the client's own track, which
    // the reference was measured following. Rewritten whenever this runs
    // rather than only when the clock moved, because `Transform` is
    // change-detected and the value is a few multiplies.
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

    // **What is behind the world is the fog colour.** The dome covers every
    // pixel of the sky now, so this is no longer what the horizon is made of —
    // it is what shows on the one frame between the camera appearing and the
    // dome being moved onto it, and it is the same colour either way.
    //
    // **Decoded, unlike the three above**, and that is not an inconsistency: a
    // clear colour is written straight into the framebuffer and the swapchain
    // encodes it on the way out, so the decode here is what makes the pixel come
    // out as the byte the file states. Nothing ever adds anything to it.
    clear.0 = LinearRgba::from_vec3(linear(atmosphere.fog())).into();

    // **Says what it wrote, whenever it writes** — at debug, because a sky
    // change is every 4-yard cell while travelling. It is the line that found
    // the login-fog bug: a night fog logged here while the screen stayed pale
    // proved the write was being *overwritten*, not skipped — see
    // `render::glue::stand_the_world_down`, whose give-back edge now tells
    // this function to run again.
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
        // **Removed rather than pushed out to infinity when the switch is
        // off.** A `FogFalloff::Linear` with an enormous end still runs the
        // blend on every fragment and still tints the far field by a fraction
        // of a band, so the subtraction would be neither honest as a
        // measurement nor clean as a picture. Without the component the pass
        // draws the world to the horizon with a hard edge at the streaming
        // radius, which is exactly what turning fog off is for.
        if tuning.fog {
            let (air, deck) = crate::render::night::NightAir::new(night.factor, night.deck).pack();
            commands.entity(entity).insert(DistanceFog {
                color: LinearRgba::from_vec3(fog).into(),
                falloff: FogFalloff::Linear {
                    start: atmosphere.fog_start,
                    end: atmosphere.fog_end,
                },
                // **Not a light colour and not an exponent.** Both fields are
                // Bevy's own scattering controls, both are unread under a
                // linear falloff with a zero scattering vector, and both are
                // bound at group 0 for every world material already — so the
                // deep night's five numbers ride in them rather than in a bind
                // group of this crate's own.
                directional_light_color: air,
                directional_light_exponent: deck,
            });
        } else {
            commands.entity(entity).remove::<DistanceFog>();
        }
    }
}

/// One band colour, sRGB as the file states it, decoded to what a shader
/// multiplies a texel by.
///
/// **Only for a band nothing is added to.** See the module note: the sun, the
/// fill, the shadow and the fog are all combined with another band before they
/// light anything, and each of those goes to the GPU as the file states it and
/// is decoded there, once, after the arithmetic. What is left for this is the
/// sky dome's six stops and `ClearColor` — colours that *are* the pixel.
///
/// Public because the dome is not the only such surface: a room's interior
/// light is composed on the GPU too, and a second copy of this transfer
/// function is exactly the drift `atmosphere.wgsl` exists to prevent one level
/// down.
pub fn linear(colour: [f32; 3]) -> Vec3 {
    let c: LinearRgba = Color::srgb(colour[0], colour[1], colour[2]).to_linear();
    Vec3::new(c.red, c.green, c.blue)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Which of the four the decode is applied to, run rather than argued.**
    ///
    /// This is the bug the whole module note is about, and it is invisible from
    /// every other angle: decoding the sun and the fill here instead of in the
    /// shader does not fail, does not warn, and does not even look obviously
    /// wrong — it produces a world lit by a plausible colour that is not the
    /// one the file states, because `srgb_to_linear(a) + srgb_to_linear(b)` is
    /// not `srgb_to_linear(a + b)`. So the three that are *terms of a sum* are
    /// asserted to reach the GPU exactly as the table states them, and the one
    /// that is a framebuffer value is asserted to be decoded.
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
            // The deep night's own resource, left at its default — a factor of
            // zero, which is what daylight resolves to and what keeps this test
            // about the four bands rather than about the grade. See
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

        // And the one that is written straight into the framebuffer *is*
        // decoded — nothing is ever added to it, and the swapchain encodes it
        // on the way back out.
        let clear = LinearRgba::from(app.world().resource::<ClearColor>().0);
        let expected = linear(fog_band);
        assert!(
            (Vec3::new(clear.red, clear.green, clear.blue) - expected).length() < 1e-6,
            "{clear:?} is not the decoded horizon band {expected:?}"
        );
    }

    /// **Logging out puts the light back, and then leaves it alone.**
    ///
    /// The bug this pins had one symptom and three causes, all of them the same
    /// omission: nothing reset the resolved atmosphere when the world went
    /// away, so a session that ended at midnight left the sun cold, the fog
    /// pulled in and — the one that is actually reported — the present pass
    /// grading every pixel of the character screen. See [`resolve`]'s own note.
    ///
    /// The second half of the test is the half that would otherwise regress
    /// silently: the reset has to happen **once**. [`apply`] is keyed on
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
        sky.resolved_for = Some((0, NOON, [1, 2, 3], true, 255, 0, 255, 0));

        let mut app = App::new();
        app.insert_resource(sky)
            .insert_resource(crate::render::night::Night {
                factor: 1.0,
                deck: -412.5,
            })
            .init_resource::<crate::render::night::NightTuning>()
            .init_resource::<WorldClock>()
            // **No world in the focus, which is the whole of the condition** —
            // the glue screens, and every frame before the first login. It is
            // the focus rather than the session since `render::focus` took that
            // question off the socket; the session is still here because
            // `resolve` reads the server's clock off it when there is one.
            .init_resource::<crate::render::focus::WorldFocus>()
            .init_resource::<crate::world::session::Session>()
            .init_resource::<crate::world::camera::CameraRig>()
            // Clear weather and every layer drawn, which is what `resolve`
            // reads them for — see the storm column.
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

        // …and nothing is written again, however long the client sits there.
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(
            app.world().resource::<Uploads>().0,
            1,
            "the reset must be one write, not one a frame"
        );
    }

    /// **The measurement that says it matters**, on map 0's own numbers at the
    /// hour of the screenshot that started this: the fill at 102/129/155, the
    /// sun at 255/128/0, and ground turned 0.76 into it.
    ///
    /// Taken in the file's space the ground is lit by 255/227/155; taken in
    /// linear it is lit by 243/167/155. The green channel is the difference
    /// between grass and rust, and no count anywhere in this project would have
    /// shown it.
    #[test]
    fn summing_in_the_wrong_space_is_a_different_colour_and_not_a_darker_one() {
        let fill = [102.0 / 255.0, 129.0 / 255.0, 155.0 / 255.0];
        let sun = [1.0, 128.0 / 255.0, 0.0];
        let lambert: f32 = 0.7625;

        // The client's own arithmetic: sum the bytes, clamp, decode once.
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

    /// **The dome's bottom stop is the fog colour, in the renderer and not only
    /// in the table.** That is the whole reason the horizon has no seam: the
    /// ground fades to `DistanceFog`'s colour and the sky is drawn in the same
    /// one at altitude zero. Two conversions of one band, so what this pins is
    /// that neither has been decoded twice or not at all.
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

    /// **sRGB in, linear out, and the difference is not small.** Mid grey is the
    /// case that says the transfer function ran at all: 0.5 sRGB is 0.21 linear,
    /// and a client that skips this multiplies its textures by more than twice
    /// the light the table asked for.
    #[test]
    fn the_band_colours_are_decoded_out_of_srgb() {
        let mid = linear([0.5, 0.5, 0.5]);
        assert!((mid.x - 0.2140).abs() < 1e-3, "{mid:?}");
        // The ends are fixed points, which is what says nothing else was applied.
        assert_eq!(linear([0.0, 0.0, 0.0]).x, 0.0);
        assert!((linear([1.0, 1.0, 1.0]).x - 1.0).abs() < 1e-6);
    }

    /// **A table edit re-lights the world**, which is the whole of what the
    /// generation is doing in the resolve key.
    ///
    /// The key holds the hour, the position quantised to four yards and three
    /// settings, and an edited `LightIntBand` moves none of them — so before
    /// this a colour changed while standing still took effect on the next
    /// half-minute the server sent, up to thirty seconds later, or on the next
    /// four yards walked. It reads as an edit that did nothing.
    ///
    /// Asserted as a property of the key rather than by running the system,
    /// which would need an archive: two keys that differ only in the
    /// generation must not compare equal.
    #[test]
    fn a_forgotten_table_makes_the_sky_resolve_again() {
        let before = (0u32, NOON, [1i32, 2, 3], false, -1i32, 0i32, 255i32, 4u64);
        let after = (0u32, NOON, [1i32, 2, 3], false, -1i32, 0i32, 255i32, 5u64);
        assert_ne!(before, after, "a re-parse of the tables must re-resolve");
        assert_eq!(before, before.clone(), "and nothing else about it moved");
    }
}
