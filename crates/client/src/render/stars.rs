//! The star field, drawn from `Stars.m2`.
//!
//! `sky.rs` draws the gradient: six colours out of `Light.dbc` on a dome. The
//! table's bands 8..12 are for a sun disc, its halo and three cloud sheets; the
//! stars are not in the table at all.
//!
//! The sun and the moons are [`crate::render::celestial`], the cloud layer is
//! [`crate::render::clouds`] and the skybox models are
//! [`crate::render::skybox`]. This module is only the field behind them. An
//! opaque skybox hides the field; see [`crate::render::sky::SkyCover`].
//!
//! ## Cost when the stars are not up
//!
//! Nothing, and this follows the 1.12.1 client's rule rather than being an
//! optimisation. The client stores the fade as `value * 254 + 1` in a byte and
//! does not draw the dome at all below 2, so from 04:30 to 22:30 the pass is
//! seven entities at `Visibility::Hidden`. [`fade`] applies the same rule: it
//! writes only when the byte changes, which is a few times an hour.
//!
//! ## The dome does not rotate
//!
//! The 1.12.1 client does not rotate the star dome, and `Stars.m2` carries no
//! skeleton (`vale anim 'Environments\Stars\Stars.m2'` reports this in one
//! line), so there is no track for a rotation. The 1.12 sky moves by moving
//! the sun and the moons across it, which is [`crate::render::celestial`]; the
//! field behind them is fixed.

use vale_assets::tables::light::celestial;
use vale_assets::world::m2::M2;
use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

/// How far out the star dome is drawn, in yards.
///
/// Inside the gradient dome; otherwise the value is arbitrary. It was first
/// chosen to sit beyond the loaded terrain, which it never did, so the field
/// could be drawn over a far mountain. Occlusion is now handled by the
/// far-plane depth rule ([`crate::render::sky::behind_the_world`]), not by the
/// radius. The only remaining requirement is that the dome, centred on the
/// camera, is inside `sky::DOME_RADIUS`, so the gradient is behind the stars
/// and not in front of them.
const STAR_RADIUS: f32 = 900.0;

/// The star pass: a model, a material and one system.
pub struct StarPlugin;

impl Plugin for StarPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/stars.wgsl");
        app.add_plugins(MaterialPlugin::<StarMaterial>::default())
            .init_resource::<StarDomeState>()
            .add_systems(
                Update,
                (load, fade.after(crate::render::skybox::follow_the_light)),
            )
            // Centred on the camera, so it moves after the camera has been
            // placed; one frame late, the whole sky slides against the world
            // as the character runs. The same anchor `sky::follow_camera`
            // uses, for the same reason: nothing fails when it is missing.
            .add_systems(
                Update,
                follow_camera.after(crate::world::camera::place),
            );
    }
}

/// The dome's root: the one entity that follows the camera, with a child per
/// batch of the model.
#[derive(Component)]
pub struct StarDome;

/// Whether the model has been asked for yet, and what came back.
///
/// Read on the main thread rather than through the model loader, which is the
/// one place in this renderer where that is allowed: it is 200 vertices and two
/// 256x256 textures, read once per session, and the loader queue exists to keep
/// a tile's 130 models off the frame. Using the loader would need a cache key,
/// a dressing and a `Lookup` state machine for a model that is never dressed
/// and never has a second instance.
#[derive(Resource, Default)]
enum StarDomeState {
    #[default]
    Unasked,
    /// Built, with the byte, the overcast and the skybox cover the materials
    /// were last written for; see [`fade`].
    Built {
        byte: Option<u32>,
        overcast: u8,
        behind_skybox: bool,
    },
    /// The model could not be read. Reported once; a sky with no stars is a
    /// degradation and not an error, in the same way as a missing
    /// `LightParams`.
    Failed,
}

/// The opacity the model states for this batch, kept beside the material
/// rather than only inside it.
///
/// The material's `tint.w` is this times the hour's fade and is rewritten every
/// time the hour moves; multiplying the fade into it in place would compound,
/// so the constant has to survive somewhere. A component rather than a second
/// uniform field, because the GPU has no use for it.
#[derive(Component)]
struct BatchOpacity(f32);

/// One batch's material: the model's own texture, and one colour to scale it by.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct StarMaterial {
    /// `rgb` is the star colour (white) and `a` is the batch's own constant
    /// opacity times the hour's fade and the weather's `1 - storm`; see
    /// [`fade`].
    #[uniform(0)]
    pub tint: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
}

impl Material for StarMaterial {
    fn fragment_shader() -> ShaderRef {
        crate::render::shader::STARS.into()
    }

    /// Alpha blended, which is the model's own `blend 2` on all seven batches.
    /// This also puts the dome in the transparent phase, after the opaque one,
    /// and the code depends on that order in one direction: the gradient dome
    /// is opaque, so it is already in the framebuffer when the stars are drawn
    /// over it.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    /// Not in the depth prepass, for the same reason the gradient dome is not:
    /// depth written 900 yards out would stand in front of the occlusion
    /// culler's own pyramid and reject the far half of the world.
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    /// Behind every blended surface in the world, which the far-plane depth
    /// rule cannot arrange on its own; see
    /// [`crate::render::sky::STARS_SORT`]. The field's mesh centre is the
    /// camera's own position, so without a bias it sorted as the nearest
    /// transparent surface in the frame and was drawn over the water, the
    /// particles and every spell effect.
    fn depth_bias(&self) -> f32 {
        crate::render::sky::STARS_SORT
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Seen from the inside, and the model's own batches declare it: every
        // one of the seven is two-sided.
        descriptor.primitive.cull_mode = None;
        // No depth write, which the model also states (`no_depth_write` on all
        // seven) and which stops the seven layers of the field from occluding
        // each other; and the far-plane depth rule the whole sky shares, which
        // stops the field standing in front of the far half of the terrain.
        // See [`crate::render::sky::behind_the_world`].
        crate::render::sky::behind_the_world(descriptor);
        Ok(())
    }
}

/// Read the star model once, and spawn one entity per batch.
///
/// Runs every frame until it succeeds or fails, which costs one enum compare;
/// it cannot run at `Startup` because the archive chain opens lazily and the
/// window is meant to paint before it does.
fn load(
    mut state: ResMut<StarDomeState>,
    assets: Res<crate::assets::GameAssets>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StarMaterial>>,
) {
    if !matches!(*state, StarDomeState::Unasked) {
        return;
    }
    let read = assets.with_archive(|archive| {
        let bytes = archive
            .read(celestial::STAR_MODEL)
            .map_err(|e| e.to_string())?;
        let model = M2::parse(&bytes).map_err(|e| e.to_string())?;
        // The textures come out with the model, because the archive lock is
        // held here and nothing else in this pass ever opens it again.
        let textures: Vec<Option<Image>> = model
            .textures
            .iter()
            .map(|texture| {
                if texture.kind != 0 || texture.file_name.is_empty() {
                    return None;
                }
                let raw = archive.read(&texture.file_name).ok()?;
                let blp = vale_assets::world::blp::decode_mipped(&raw).ok()?;
                Some(crate::render::models::loader::RawTexture::from_blp(blp).into_image())
            })
            .collect();
        Ok((model, textures))
    });
    let (model, textures) = match read {
        Ok(pair) => pair,
        Err(why) => {
            warn!("[sky] no star field: {why}");
            *state = StarDomeState::Failed;
            return;
        }
    };

    let images: Vec<Option<Handle<Image>>> = textures
        .into_iter()
        .map(|image| image.map(|i| images.add(i)))
        .collect();

    // The model is a hemisphere about the origin; scaling by its own declared
    // radius is what puts it at `STAR_RADIUS` whatever the file says, rather
    // than baking a number measured off one build of one archive.
    let scale = if model.bounding_radius > 0.0 {
        STAR_RADIUS / model.bounding_radius
    } else {
        1.0
    };

    let dome = commands
        .spawn((
            StarDome,
            Transform::from_scale(Vec3::splat(scale)),
            Visibility::Hidden,
        ))
        .id();

    for batch in &model.batches {
        let Some(texture) = batch
            .texture
            .and_then(|slot| images.get(slot as usize).cloned().flatten())
        else {
            continue;
        };
        let Some(mesh) = batch_mesh(&model, batch) else {
            continue;
        };
        // The batch's own constant opacity, baked. All five of the model's
        // transparency tracks hold a single key (0.25, 0.35, 0.50, 0.65, 0.75),
        // so there is nothing to animate. The field appears to have depth
        // because its seven layers are drawn at different strengths, and this
        // is where those strengths come from. `sample` is asked for the value
        // at t=0 rather than reading the track directly, so an animated track
        // would still be used at whatever it holds when the night starts.
        let alpha = batch
            .tint
            .map(|tint| model.tints.sample(tint, 0, 0, 0, 0)[3])
            .unwrap_or(1.0);
        commands.spawn((
            ChildOf(dome),
            BatchOpacity(alpha),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(StarMaterial {
                // Zero until the first [`fade`], which is the correct starting
                // state: the dome spawns `Hidden` and the hour decides.
                tint: Vec4::new(1.0, 1.0, 1.0, 0.0),
                texture,
            })),
            // The dome is centred on the camera and is 900 yards across, so a
            // frustum test on it can only ever answer "yes" — and answering it
            // costs a bounding-sphere transform per batch per frame.
            NoFrustumCulling,
            NotShadowCaster,
            NotShadowReceiver,
        ));
    }

    *state = StarDomeState::Built {
        byte: None,
        overcast: 0,
        behind_skybox: false,
    };
}

/// One batch of the model as a mesh, in Bevy's axes.
///
/// Returns `None` for a batch whose index range is not inside the model, which
/// is the one way a damaged file could reach the GPU as a panic rather than as
/// a missing star layer.
fn batch_mesh(model: &M2, batch: &vale_assets::world::m2::M2Batch) -> Option<Mesh> {
    let start = batch.index_start as usize;
    let end = start.checked_add(batch.index_count as usize)?;
    let indices = model.indices.get(start..end)?;

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        // Built once and never written again. The per-frame fade is in the
        // material rather than in a vertex colour so that nothing here has to
        // come back to the CPU.
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        model
            .positions
            .iter()
            .map(|p| crate::render::axes::to_bevy(*p).to_array())
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        model
            .normals
            .iter()
            .map(|n| crate::render::axes::to_bevy(*n).to_array())
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, model.uvs.clone());
    mesh.insert_indices(Indices::U32(indices.iter().map(|i| *i as u32).collect()));
    Some(mesh)
}

/// Keep the dome centred on the camera, so the sky never gets nearer.
///
/// The camera's `Transform` and not its `GlobalTransform`; see `sky.rs`'s own
/// `follow_camera`, which explains why.
fn follow_camera(
    camera: Query<&Transform, With<crate::world::camera::WorldCamera>>,
    mut dome: Query<&mut Transform, (With<StarDome>, Without<crate::world::camera::WorldCamera>)>,
) {
    let Some(eye) = camera.iter().next() else {
        return;
    };
    for mut transform in &mut dome {
        transform.translation = eye.translation;
    }
}

/// Fade the field with the hour, and hide it entirely when the 1.12.1 client's
/// byte floor says the stars are not up.
///
/// Written only when the byte changes, which is the difference between a few
/// writes an hour and rewriting seven materials, with seven GPU uniform
/// uploads, sixty times a second for a value that changes on a 24-minute ramp.
fn fade(
    clock: Res<crate::render::sky::WorldClock>,
    // The overcast; see the body. A resource the glue screens also have, so
    // this system runs unchanged before any world exists.
    weather: Res<crate::render::weather::WeatherState>,
    // The switch is applied here rather than by a separate system, which is
    // the rule [`crate::render::tuning`] states: this system already owns the
    // dome's visibility, and a second writer would conflict with it and take
    // effect only on the frames the hour happened to change.
    tuning: Res<crate::render::tuning::WorldTuning>,
    // An opaque skybox hides the field; see `render::sky::SkyCover`.
    skybox: Res<crate::render::sky::SkyCover>,
    mut state: ResMut<StarDomeState>,
    mut dome: Query<(&mut Visibility, &Children), With<StarDome>>,
    parts: Query<(&MeshMaterial3d<StarMaterial>, &BatchOpacity)>,
    mut materials: ResMut<Assets<StarMaterial>>,
) {
    let StarDomeState::Built {
        byte: last,
        overcast: last_overcast,
        behind_skybox,
    } = &mut *state
    else {
        return;
    };
    let byte = celestial::star_byte(clock.half_minutes);
    // The weather fades the field by `1 - storm`, the 1.12.1 client's rule,
    // quantised to a byte as the hour's own fade is, so a ten-second ramp is a
    // fade and a still sky rewrites nothing.
    let cover = (weather.storm().clamp(0.0, 1.0) * 255.0) as u8;
    if *last == byte
        && *last_overcast == cover
        && *behind_skybox == skybox.opaque
        && !tuning.is_changed()
    {
        return;
    }
    *last = byte;
    *last_overcast = cover;
    *behind_skybox = skybox.opaque;
    let byte = if tuning.stars && !skybox.opaque { byte } else { None };
    let clear_sky = 1.0 - f32::from(cover) / 255.0;

    for (mut visibility, children) in &mut dome {
        *visibility = match byte {
            Some(_) => Visibility::Inherited,
            None => Visibility::Hidden,
        };
        let Some(byte) = byte else { continue };
        // Back out of the byte rather than re-evaluating the curve, so what is
        // on screen is exactly what the HUD reports and exactly what the client
        // itself would have quantised it to.
        let fade = byte as f32 / 255.0;
        for child in children {
            let Ok((handle, opacity)) = parts.get(*child) else {
                continue;
            };
            if let Some(mut material) = materials.get_mut(&handle.0) {
                material.tint.w = opacity.0 * fade * clear_sky;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::sky::WorldClock;

    /// A dome with one batch at a known opacity, and the clock it is fading on.
    fn app(opacity: f32) -> (App, Handle<StarMaterial>) {
        let mut app = App::new();
        app.init_resource::<Assets<StarMaterial>>()
            .insert_resource(WorldClock {
                half_minutes: 0,
                from_server: true,
                ..default()
            })
            .insert_resource(StarDomeState::Built {
                byte: None,
                overcast: 0,
                behind_skybox: false,
            })
            .init_resource::<crate::render::tuning::WorldTuning>()
            .init_resource::<crate::render::sky::SkyCover>()
            // A clear sky, so the hour's own fade is what these tests measure.
            .init_resource::<crate::render::weather::WeatherState>()
            .add_systems(Update, fade);
        let handle = app
            .world_mut()
            .resource_mut::<Assets<StarMaterial>>()
            .add(StarMaterial {
                tint: Vec4::new(1.0, 1.0, 1.0, 0.0),
                texture: Handle::default(),
            });
        let dome = app.world_mut().spawn((StarDome, Visibility::Hidden)).id();
        app.world_mut()
            .spawn((ChildOf(dome), BatchOpacity(opacity), MeshMaterial3d(handle.clone())));
        (app, handle)
    }

    fn alpha(app: &App, handle: &Handle<StarMaterial>) -> f32 {
        app.world()
            .resource::<Assets<StarMaterial>>()
            .get(handle)
            .expect("the material")
            .tint
            .w
    }

    fn set(app: &mut App, hour: u32, minute: u32) {
        app.world_mut().resource_mut::<WorldClock>().half_minutes = (hour * 60 + minute) * 2;
        app.update();
    }

    /// The hour scales the batch's own opacity, and applying it twice gives the
    /// same result as applying it once.
    ///
    /// Keeping one alpha in the material and multiplying the fade into it as
    /// the hour moves would compound: an evening ramp would multiply the batch
    /// constant by every value the curve passed through, and the field would
    /// be gone by midnight even though the curve itself is correct. So the
    /// constant lives in [`BatchOpacity`] and the material is written, never
    /// scaled.
    #[test]
    fn the_hours_fade_scales_the_batch_constant_and_never_compounds() {
        let (mut app, handle) = app(0.25);

        set(&mut app, 0, 0);
        assert_eq!(alpha(&app, &handle), 0.25, "full stars, full batch opacity");

        set(&mut app, 23, 15);
        let half_way = alpha(&app, &handle);
        assert!(
            (half_way - 0.125).abs() < 0.01,
            "halfway up the evening ramp: {half_way}"
        );

        // Round the clock and back to the same instant. A compounding write
        // reads a fraction of this the second time.
        set(&mut app, 0, 0);
        set(&mut app, 12, 0);
        set(&mut app, 23, 15);
        assert_eq!(alpha(&app, &handle), half_way, "the hour is not cumulative");
    }

    /// The whole dome is hidden when the 1.12.1 client's byte floor says the
    /// stars are not up, which is three quarters of the day; this is why the
    /// pass costs nothing at noon.
    #[test]
    fn the_dome_is_hidden_outright_when_the_stars_are_not_up() {
        let (mut app, _) = app(1.0);
        let visible = |app: &App| {
            app.world()
                .iter_entities()
                .filter_map(|e| e.get::<Visibility>().copied())
                .next()
                .expect("the dome")
        };

        set(&mut app, 12, 0);
        assert_eq!(visible(&app), Visibility::Hidden);
        set(&mut app, 2, 0);
        assert_eq!(visible(&app), Visibility::Inherited);

        // The switch hides it at an hour the curve says it is up, without
        // changing the curve: turned back on, the dome is visible at 2am again.
        app.world_mut().resource_mut::<crate::render::tuning::WorldTuning>().stars = false;
        app.update();
        assert_eq!(visible(&app), Visibility::Hidden);
        app.world_mut().resource_mut::<crate::render::tuning::WorldTuning>().stars = true;
        app.update();
        assert_eq!(visible(&app), Visibility::Inherited);
    }
}
