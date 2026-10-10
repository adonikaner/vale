//! The sun and the two moons, the objects in the sky that move.
//!
//! `sky.rs` draws the gradient and `stars.rs` the star field, and both are
//! fixed: the dome is a function of the hour, and the 1.12.1 client does not
//! rotate the star model, which has no skeleton (see `stars.rs`). What moves
//! in 1.12's sky is three sprites, drawn by this module. The cloud layer is
//! [`crate::render::clouds`] and the skybox models are
//! [`crate::render::skybox`]; an opaque skybox hides the sun and the moons
//! (see [`crate::render::sky::SkyCover`]).
//!
//! ## Position tracks
//!
//! Each body's position at an hour is given by two tracks, a polar angle and
//! an azimuth, each interpolated over four keys in the same way as the star
//! fade. The tracks, key by key, are in
//! [`vale_assets::tables::light::celestial`].
//!
//! The sun rises at 05:30, climbs to within five degrees of the zenith at noon
//! and sets at 21:30; the moon runs the opposite arc. Both keep a constant
//! bearing of 45°, which agrees to five degrees with the azimuth `vale sun`
//! measured from the ground's own baked `MCSH` shadows. The blue moon is the
//! only body whose bearing moves, and it keeps its own 1.7-day calendar.
//!
//! ## Distance and size
//!
//! Each body is drawn twelve yards from the camera
//! ([`celestial::CELESTIAL_RADIUS`]) on a one-yard square (the sprite quad's
//! four corners are at ±0.5), scaled by the body's own track. That makes the
//! sun's disc 4.8° across at noon and 9.5° at the horizon, and the moon 8.3°,
//! which is why this game's moon looks the size it does.
//!
//! Twelve yards is nearer than most trees, and it works for the same reason
//! the 950-yard dome works: every sky surface writes its depth at the far
//! plane, so what is in front of it is decided by the depth buffer rather than
//! by distance. See [`crate::render::sky::behind_the_world`], which defines
//! that rule.
//!
//! ## What is not drawn
//!
//! The two glares. `Textures\sunGlare.blp` and `moonGlare.blp` are in the
//! archive and their visibility curves are measured
//! ([`celestial::SUN_GLARE`], [`celestial::MOON_GLARE`]), and `vale sky`
//! reports the sun glare as the one additive sprite in the set: its
//! transparent texels are 0/0/0, where the three discs carry full-strength art
//! under their masks. The glare's geometry is not established. The 1.12.1
//! client draws it as a screen-space flare rather than a billboard, and
//! drawing it without that geometry would mean guessing at the brightest thing
//! in the frame.
//!
//! The discs are not tinted by bands 8 and 9. Both are read and carried
//! ([`vale_assets::tables::light::Atmosphere::sun_disc`] and `sun_halo`), and
//! `vale sky` prints them across the day, but how the 1.12.1 client uses them
//! is not established. Band 8 at map 0's noon is 77/77/77, and a disc
//! multiplied by that is a third as bright, which would look like a blend bug
//! rather than a colour. The sprites are therefore drawn as the art states
//! them, and the two bands are printed but not applied.

use vale_assets::world::blp;
use vale_assets::tables::light::celestial;
use bevy::asset::embedded_asset;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

/// The side of the sprite quad, in yards, before a body's own scale.
///
/// One yard, which is the 1.12.1 client's size: every sky sprite is drawn on a
/// quad with four corners at ±0.5 in the billboard plane. How big the sun looks
/// is therefore set by [`celestial::Body::size`] and
/// [`celestial::CELESTIAL_RADIUS`], both of which are also the client's.
const SPRITE_QUAD: f32 = 1.0;

/// The sun and the moons: three sprites, one material and two systems.
pub struct CelestialPlugin;

impl Plugin for CelestialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/celestial.wgsl");
        app.add_plugins(MaterialPlugin::<CelestialMaterial>::default())
            .init_resource::<CelestialState>()
            // Behind a run condition, because it is a startup job on an
            // `Update` schedule. It reads one enum and returns for the rest of
            // every session once the archives answer, but a return in the
            // system body still fetched four `ResMut<Assets<_>>` and a
            // `Commands` sixty times a second to do nothing. Tracy self-time
            // at the floor framing: 0.073 ms a frame. The same finding applied
            // to `lua::host`'s two loaders.
            .add_systems(Update, load.run_if(nothing_loaded_yet))
            // Placed relative to the camera, so after the camera has been
            // placed: the same anchor the two domes use, for the same reason.
            // One frame late, the sun slides against the world as the
            // character runs.
            .add_systems(Update, follow_camera.after(crate::world::camera::place))
            // The weather fades the sun and the moons through the tint the
            // material has carried since it was built; see [`overcast`].
            .add_systems(Update, overcast)
            // The switch that takes all three off, which also hides them behind
            // an opaque skybox; see [`crate::render::sky::SkyCover`]. Nothing
            // else writes a body's visibility: `follow_camera` only moves them,
            // and a body under the horizon is drawn and occluded by the ground
            // rather than hidden.
            .add_systems(
                Update,
                crate::render::sky::under_skybox::<CelestialBody>(|tuning| tuning.celestial)
                    .after(crate::render::skybox::follow_the_light),
            );
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report.run_if(crate::ui::report::watched));
    }
}

/// [`crate::ui::report`] slot. See that module for why this line is written
/// here rather than in `hud.rs`.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(20);

/// Where the sun and the moons are, and whether the stars are drawn, both on
/// the HUD for the same reason.
///
/// The star byte is the 1.12.1 client's (see [`crate::render::stars`]), so
/// `off` here means the 1.12.1 client would not draw them either: missing
/// stars and stars correctly almost out at 22:36 look the same on screen. The
/// elevations answer the same question for the bodies: a body below the
/// horizon and a body that was never drawn also look the same, and for more
/// than half the hours of the day one of the three is below the horizon. A
/// negative elevation here is correct and does not mean a pass is missing.
#[cfg(feature = "diagnostics")]
fn report(
    clock: Res<crate::render::sky::WorldClock>,
    mut hud: ResMut<crate::ui::report::HudReport>,
) {
    if !clock.is_changed() {
        return;
    }
    let at = celestial::day_fraction(clock.half_minutes);
    hud.set(
            crate::ui::report::Section::World,
        SLOT,
        "celestial",
        format!(
            "{}   sun {:.0}°  moon {:.0}°  blue {:.0}°",
            match celestial::star_byte(clock.half_minutes) {
                Some(byte) => format!("stars {byte}/255"),
                None => "stars off".to_string(),
            },
            celestial::SUN.elevation(at),
            celestial::MOON.elevation(at),
            celestial::BLUE_MOON.elevation(celestial::blue_moon_fraction(0, clock.half_minutes)),
        ),
    );
}

/// Which body an entity is, so [`follow_camera`] can ask the right tracks where
/// to put it.
///
/// A component rather than three marker types, because every one of them is
/// moved by the same five lines.
#[derive(Component)]
pub struct CelestialBody {
    body: &'static celestial::Body,
    /// Whether this body reads the day's clock or its own calendar. The blue
    /// moon uses its own; sampling it on the day's clock would put it exactly
    /// on top of the white moon, which does not look obviously wrong.
    own_calendar: bool,
}

/// Whether the sprites have been asked for yet.
///
/// Read on the main thread, for the same reason as the star dome: three
/// textures, once per session, while the loader queue exists to keep a tile's
/// hundred-odd models off the frame.
#[derive(Resource, Default)]
enum CelestialState {
    #[default]
    Unasked,
    Built,
    /// A sky with no sun is a degradation and not an error, in the same way as
    /// a missing `LightParams`. Reported once.
    Failed,
}

/// One body's material: its sprite, and a colour to scale it by.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct CelestialMaterial {
    /// White, and opaque at every hour in clear weather; the weather sets its
    /// alpha to `1 - storm` (see [`overcast`]). See the module note on why
    /// bands 8 and 9 are not applied here. It is a uniform rather than a
    /// constant in the shader so that it can be written at run time.
    #[uniform(0)]
    pub tint: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
}

impl Material for CelestialMaterial {
    fn fragment_shader() -> ShaderRef {
        crate::render::shader::CELESTIAL.into()
    }

    /// Alpha blended, as measured: `vale sky` decodes each sprite and reports
    /// the mean colour of the texels its alpha channel marks empty. All three discs read bright there (the sun 255/255/167, the moon
    /// 175/178/181, the blue moon 103/162/220), so they are cut-outs; adding
    /// them would paint their whole rectangle over the sky. The one additive
    /// sprite in the set is `sunGlare.blp` at 0/0/0, and it is not drawn.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    /// See [`crate::render::sky::CELESTIAL_SORT`], which defines the sky's
    /// transparent ordering and keeps the moon from being drawn on top of a
    /// lake.
    fn depth_bias(&self) -> f32 {
        crate::render::sky::CELESTIAL_SORT
    }

    /// Not in the depth prepass, for the reason every sky surface is not: depth
    /// written here would stand in front of the occlusion culler's own pyramid.
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The quad is turned to face the camera every frame, so which way round
        // it ends up is not worth depending on.
        descriptor.primitive.cull_mode = None;
        crate::render::sky::behind_the_world(descriptor);
        Ok(())
    }
}

/// Whether the sky's art has not been requested yet.
///
/// The same test as [`load`]'s first line, used as a run condition. The body
/// keeps its own copy of the test, because a condition that differs from the
/// body it guards gives either a system that runs and returns (the cost this
/// removes) or one that never runs (a sky with no sun).
fn nothing_loaded_yet(state: Res<CelestialState>) -> bool {
    matches!(*state, CelestialState::Unasked)
}

/// Read the three sprites once and spawn a quad for each.
fn load(
    mut state: ResMut<CelestialState>,
    assets: Res<crate::assets::GameAssets>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<CelestialMaterial>>,
) {
    if !matches!(*state, CelestialState::Unasked) {
        return;
    }
    let bodies: [(&'static celestial::Body, bool); 3] = [
        (&celestial::SUN, false),
        (&celestial::MOON, false),
        (&celestial::BLUE_MOON, true),
    ];
    let read = assets.with_archive(|archive| {
        let mut sprites = Vec::new();
        for (body, _) in bodies {
            let raw = archive.read(body.texture).map_err(|e| e.to_string())?;
            let decoded = blp::decode_mipped(&raw).map_err(|e| e.to_string())?;
            sprites.push(crate::render::models::loader::RawTexture::from_blp(decoded).into_image());
        }
        Ok(sprites)
    });
    let sprites: Vec<Image> = match read {
        Ok(sprites) => sprites,
        Err(why) => {
            warn!("[sky] no sun or moons: {why}");
            *state = CelestialState::Failed;
            return;
        }
    };

    // One mesh for all three: they differ only in what they are scaled and
    // textured by.
    let quad = meshes.add(Rectangle::new(SPRITE_QUAD, SPRITE_QUAD).mesh());
    for ((body, own_calendar), sprite) in bodies.into_iter().zip(sprites) {
        commands.spawn((
            CelestialBody { body, own_calendar },
            Mesh3d(quad.clone()),
            MeshMaterial3d(materials.add(CelestialMaterial {
                tint: Vec4::ONE,
                texture: images.add(sprite),
            })),
            // Placed by `follow_camera` on the first frame; this keeps it out of
            // the world's middle until then.
            Transform::from_scale(Vec3::ZERO),
            NotShadowCaster,
            NotShadowReceiver,
        ));
    }
    *state = CelestialState::Built;
}

/// Fade the sun and the moons by the weather: their alpha is `1 - storm`, the
/// 1.12.1 client's rule, which it applies to the star field too. Quantised to a
/// byte as the client stores it, and written only when the byte moves.
fn overcast(
    weather: Res<crate::render::weather::WeatherState>,
    bodies: Query<&MeshMaterial3d<CelestialMaterial>, With<CelestialBody>>,
    mut materials: ResMut<Assets<CelestialMaterial>>,
    mut last: Local<Option<u8>>,
) {
    let cover = (weather.storm().clamp(0.0, 1.0) * 255.0) as u8;
    if *last == Some(cover) {
        return;
    }
    *last = Some(cover);
    let clear_sky = 1.0 - f32::from(cover) / 255.0;
    for handle in &bodies {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.tint.w = clear_sky;
        }
    }
}

/// Put each body where the hour says, facing the camera.
///
/// Evaluate the body's two tracks, turn them into a unit vector, push it out
/// to [`celestial::CELESTIAL_RADIUS`] and add the camera's own position, which
/// is how the 1.12.1 client places the bodies. The billboard and the scale are
/// applied separately.
///
/// This reads the camera's `Transform`, not its `GlobalTransform`; `sky.rs`'s
/// own `follow_camera` explains why, and this is the pass where the error was
/// visible. A sprite twelve yards from the eye, placed at last frame's eye
/// position, is at the wrong angle, and under a mouse-look drag the eye moves
/// yards in a frame: the sun and the moons jumped about the sky with every
/// movement of the camera.
fn follow_camera(
    clock: Res<crate::render::sky::WorldClock>,
    camera: Query<&Transform, With<crate::world::camera::WorldCamera>>,
    mut bodies: Query<
        (&CelestialBody, &mut Transform),
        Without<crate::world::camera::WorldCamera>,
    >,
) {
    let Some(eye) = camera.iter().next() else {
        return;
    };
    let eye = eye.translation;
    for (body, mut transform) in &mut bodies {
        // The blue moon's tracks are sampled on its own 1.7-day cycle; day 0 is
        // the epoch, because nothing in the protocol gives this client the
        // 1.12.1 client's day counter. It still crosses the sky correctly
        // within a day, which is the visible part.
        let at = if body.own_calendar {
            celestial::blue_moon_fraction(0, clock.half_minutes)
        } else {
            celestial::day_fraction(clock.half_minutes)
        };
        // The tracks are in the world's own axes (+X north, +Y west, +Z up),
        // which is the one place this becomes Bevy's.
        let direction = Vec3::from(crate::render::axes::to_bevy(body.body.direction(at)));
        transform.translation = eye + direction * celestial::CELESTIAL_RADIUS;
        transform.scale = Vec3::splat(body.body.size(at));
        // The quad's own normal is +Z and `looking_to` points -Z along its
        // argument, so pointing it away from the camera turns its face toward
        // the camera.
        transform.rotation = Transform::default()
            .looking_to(direction, Vec3::Y)
            .rotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::sky::WorldClock;
    use vale_assets::tables::light::DAY;

    /// A world holding one camera and the three bodies, at a given hour.
    fn app(half_minutes: u32) -> App {
        let mut app = App::new();
        app.insert_resource(WorldClock {
            half_minutes,
            from_server: true,
            ..default()
        })
        .add_systems(Update, follow_camera);
        app.world_mut().spawn((
            crate::world::camera::WorldCamera,
            Transform::from_xyz(100.0, 50.0, -20.0),
            GlobalTransform::from_xyz(100.0, 50.0, -20.0),
        ));
        for (body, own_calendar) in [
            (&celestial::SUN, false),
            (&celestial::MOON, false),
            (&celestial::BLUE_MOON, true),
        ] {
            app.world_mut()
                .spawn((CelestialBody { body, own_calendar }, Transform::default()));
        }
        app.update();
        app
    }

    /// The height of each body above the camera, in the order they were spawned.
    fn heights(app: &mut App) -> Vec<f32> {
        let eye = Vec3::new(100.0, 50.0, -20.0);
        let world = app.world_mut();
        let mut query = world.query::<(&CelestialBody, &Transform)>();
        query
            .iter(world)
            .map(|(_, t)| t.translation.y - eye.y)
            .collect()
    }

    /// The sprites are placed relative to the camera, not near the world's
    /// origin. A static sky would fail this way and no count would show it: a
    /// sun left at the origin is a sun a long way off in one direction.
    #[test]
    fn every_body_is_hung_off_the_camera_at_the_clients_own_radius() {
        let mut app = app(DAY / 2);
        let eye = Vec3::new(100.0, 50.0, -20.0);
        let world = app.world_mut();
        let mut query = world.query::<(&CelestialBody, &Transform)>();
        let mut seen = 0;
        for (_, transform) in query.iter(world) {
            let out = (transform.translation - eye).length();
            assert!(
                (out - celestial::CELESTIAL_RADIUS).abs() < 1e-3,
                "{out} yards from the eye"
            );
            seen += 1;
        }
        assert_eq!(seen, 3, "the sun and both moons");
    }

    /// Noon puts the sun overhead and the moon below the horizon, and midnight
    /// swaps them. Bevy's +Y is up, so this is the one assertion that covers
    /// the axis conversion as well as the tracks: a transposed `to_bevy` would
    /// leave every elevation in this project's own tests intact and put the sun
    /// on the horizon here.
    #[test]
    fn the_sun_is_overhead_at_noon_and_the_moon_is_under_the_world() {
        let noon = heights(&mut app(DAY / 2));
        assert!(noon[0] > 11.0, "the sun at noon is nearly straight up: {noon:?}");
        assert!(noon[1] < 0.0, "and the moon is below the horizon: {noon:?}");

        let midnight = heights(&mut app(0));
        assert!(midnight[0] < 0.0, "the sun at midnight: {midnight:?}");
        assert!(midnight[1] > 0.0, "and the moon: {midnight:?}");
    }

    /// The sprites follow the camera in the frame it moves, not the frame
    /// after.
    ///
    /// `camera::place` writes the camera's `Transform` in `Update`; Bevy
    /// propagates it into the `GlobalTransform` in `PostUpdate`. A pass ordered
    /// after `place` that reads the global transform therefore reads last
    /// frame's eye regardless of ordering, and at twelve yards the error is an
    /// angular one: under a mouse-look drag the eye moves yards in a frame and
    /// the sun jumps about the sky with it.
    ///
    /// So this moves the `Transform` alone, leaves the stale `GlobalTransform`
    /// where it was, and asserts the sun went with the new one. Reading the
    /// global transform passes every other test in this file and fails only
    /// this one.
    #[test]
    fn a_body_is_hung_off_this_frames_eye_rather_than_last_frames() {
        let mut app = app(DAY / 2);
        let moved = Vec3::new(400.0, 50.0, -20.0);
        {
            let world = app.world_mut();
            let mut cameras =
                world.query_filtered::<&mut Transform, With<crate::world::camera::WorldCamera>>();
            for mut transform in cameras.iter_mut(world) {
                transform.translation = moved;
            }
        }
        app.update();

        let world = app.world_mut();
        let mut query = world.query::<(&CelestialBody, &Transform)>();
        for (_, transform) in query.iter(world) {
            let out = (transform.translation - moved).length();
            assert!(
                (out - celestial::CELESTIAL_RADIUS).abs() < 1e-3,
                "{out} yards from the eye the camera is actually at"
            );
        }
    }

    /// The blue moon keeps its own calendar, so at the hour the white one is
    /// highest the two are not in the same place, as they would be if the blue
    /// moon were sampled on the plain day fraction.
    #[test]
    fn the_blue_moon_does_not_shadow_the_white_one() {
        let mut app = app(0);
        let world = app.world_mut();
        let mut query = world.query::<(&CelestialBody, &Transform)>();
        let places: Vec<Vec3> = query.iter(world).map(|(_, t)| t.translation).collect();
        assert!(
            (places[1] - places[2]).length() > 1.0,
            "the two moons are on top of each other: {places:?}"
        );
    }
}
