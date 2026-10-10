//! The `LightSkybox` models: a model drawn around the camera wherever a light
//! whose `LightParams` names one is in force.
//!
//! The light chain decides which models are in force and how strongly; see
//! [`vale_assets::tables::light::SkyboxWeight`]. This pass draws them. At most
//! two are in force at a point, each at its light's weight, so a skybox fades in
//! over its light's falloff and is opaque inside its inner radius. While one is
//! drawn at [`SKYBOX_OPAQUE`] or more, the gradient dome, the stars, the sun,
//! the moons and the clouds are not drawn; [`SkyCover`] carries that to their
//! passes.
//!
//! ## How a model is drawn
//!
//! The model is the skinned build from [`ModelCache::as_scene`], posed on its
//! first sequence, looped, from when it was spawned. `DeathClouds.m2` turns as
//! one bone on a 200-second loop; `CavernsOfTimeSky.m2` moves four. Its origin
//! is at the camera and it does not turn with it, as the 1.12.1 client draws
//! it. It is scaled to [`SKYBOX_RADIUS`] by its header sphere; a model centred
//! on the camera looks the same at any scale, and the scale keeps the smallest
//! of them (`DireMaulSkyBox.m2`, 1 yard) clear of the near plane.
//!
//! Each batch is drawn through [`SkyboxMaterial`] rather than the world's M2
//! material, because a sky surface writes the far plane's depth (see
//! [`crate::render::sky::behind_the_world`]), takes no light and no fog, and is
//! faded by its light's weight. Opaque and alpha-keyed batches are drawn first
//! and blended ones after, in the model's batch order, which is the order the
//! client's model renderer draws them in; every batch is in the transparent
//! phase so that this order holds.
//!
//! No skybox model in 1.12 states a texture matrix, so none is applied.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::embedded_asset;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::math::Affine3A;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::mesh::skinning::SkinnedMesh;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState,
    RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use vale_assets::tables::light::{skybox_models, SKYBOX_OPAQUE};
use vale_assets::world::m2::{BatchTint, M2Skeleton, M2Tints};

use crate::render::axes;
use crate::render::models::material::{M2Material, Materials};
use crate::render::models::{Lookup, ModelAssets, ModelCache, SceneLighting};
use crate::render::sky::SkyCover;

/// How far out a skybox model is drawn, in yards, measured by its header
/// sphere. Inside the gradient dome; the far-plane depth keeps it behind the
/// world whatever the radius.
const SKYBOX_RADIUS: f32 = 800.0;

/// How many sort steps each of the two models' batches may take; see
/// [`crate::render::sky::SKYBOX_SORT`].
const SORT_STEPS_PER_MODEL: f32 = 64.0;

pub struct SkyboxPlugin;

impl Plugin for SkyboxPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/skybox.wgsl");
        app.add_plugins(MaterialPlugin::<SkyboxMaterial>::default())
            .init_resource::<Skyboxes>()
            .add_systems(
                Update,
                (
                    follow_the_light.after(crate::render::sky::apply),
                    spawn_loaded.after(follow_the_light),
                    pose.after(spawn_loaded).after(crate::world::camera::place),
                ),
            );
    }
}

/// The skybox models wanted and drawn.
#[derive(Resource, Default)]
pub struct Skyboxes {
    /// `LightSkybox` rows by id, as model paths, and which parse of the tables
    /// they were read from.
    models: Option<(u64, HashMap<u32, String>)>,
    shown: Vec<Shown>,
}

impl Skyboxes {
    /// Every model in force: its `LightSkybox` id, its path, its weight and
    /// whether it is drawn yet. For the debug window and the tests.
    pub fn in_force(&self) -> impl Iterator<Item = (u32, &str, f32, bool)> {
        self.shown
            .iter()
            .map(|shown| (shown.id, shown.path.as_str(), shown.weight, matches!(shown.state, State::Drawn(_))))
    }
}

/// One model in force.
struct Shown {
    id: u32,
    path: String,
    /// Its light's weight, 0..1: the opacity it is drawn at.
    weight: f32,
    /// Which of the light chain's two slots it is in, for the sort order.
    slot: usize,
    state: State,
}

enum State {
    /// Asked of the model cache, not back yet.
    Loading,
    /// Back, not yet spawned.
    Loaded(Arc<ModelAssets>),
    Drawn(Drawn),
    /// The archives do not hold it. Nothing is drawn and the sky is not hidden.
    Failed,
}

/// The entities of one drawn model.
struct Drawn {
    root: Entity,
    joints: Vec<Entity>,
    /// Each batch, its material, and the colour and transparency tracks that
    /// fade it, if any.
    parts: Vec<(Handle<SkyboxMaterial>, Option<BatchTint>)>,
    skeleton: Option<Arc<M2Skeleton>>,
    tints: Option<Arc<M2Tints>>,
    /// The header sphere's radius, which [`SKYBOX_RADIUS`] scales against.
    radius: f32,
    /// Seconds of the app's clock when it was spawned; its animation starts
    /// there.
    since: f32,
}

/// Read which skyboxes the light names, load them, despawn the ones no longer
/// named, and decide [`SkyCover`].
pub fn follow_the_light(
    sky: Res<crate::render::sky::Sky>,
    tuning: Res<crate::render::tuning::WorldTuning>,
    assets: Res<crate::assets::GameAssets>,
    mut skyboxes: ResMut<Skyboxes>,
    mut cache: ResMut<ModelCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: Materials,
    mut cover: ResMut<SkyCover>,
    mut commands: Commands,
) {
    let generation = assets.tables_generation();
    if skyboxes.models.as_ref().is_none_or(|(read, _)| *read != generation) {
        let bytes = assets
            .with_archive(|archive| {
                archive
                    .read(r"DBFilesClient\LightSkybox.dbc")
                    .map_err(|e| e.to_string())
            })
            .unwrap_or_default();
        skyboxes.models = Some((generation, skybox_models(&bytes)));
    }
    let Some((_, models)) = &skyboxes.models else {
        return;
    };

    // What the light wants: each named model with a path, at its weight.
    let wanted: Vec<(usize, u32, String, f32)> = if tuning.skyboxes {
        sky.current
            .skyboxes
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.id != 0 && slot.weight > 0.0)
            .filter_map(|(at, slot)| Some((at, slot.id, models.get(&slot.id)?.clone(), slot.weight)))
            .collect()
    } else {
        Vec::new()
    };

    // Despawn what is no longer wanted, or whose row now names another model.
    skyboxes.shown.retain(|shown| {
        let keep = wanted.iter().any(|(_, id, path, _)| *id == shown.id && *path == shown.path);
        if !keep {
            if let State::Drawn(drawn) = &shown.state {
                commands.entity(drawn.root).despawn();
            }
        }
        keep
    });
    for (slot, id, path, weight) in wanted {
        match skyboxes.shown.iter_mut().find(|shown| shown.id == id) {
            Some(shown) => {
                shown.weight = weight;
                shown.slot = slot;
            }
            None => skyboxes.shown.push(Shown {
                id,
                path,
                weight,
                slot,
                state: State::Loading,
            }),
        }
    }
    for shown in &mut skyboxes.shown {
        if !matches!(shown.state, State::Loading) {
            continue;
        }
        shown.state = match cache.as_scene(&shown.path, SceneLighting::NONE, &mut meshes, &mut materials) {
            Lookup::Ready(model) => State::Loaded(model),
            Lookup::Loading => State::Loading,
            Lookup::Failed => {
                warn!("[sky] skybox {} names {}, which does not load", shown.id, shown.path);
                State::Failed
            }
        };
    }

    let opaque = skyboxes
        .shown
        .iter()
        .any(|shown| shown.weight >= SKYBOX_OPAQUE && matches!(shown.state, State::Drawn(_)));
    if cover.opaque != opaque {
        cover.opaque = opaque;
    }
}

/// Spawn the models that have loaded: a root, the joints, and one entity per
/// batch with a [`SkyboxMaterial`] built from the batch's own M2 material.
fn spawn_loaded(
    mut skyboxes: ResMut<Skyboxes>,
    m2_materials: Res<Assets<M2Material>>,
    mut materials: ResMut<Assets<SkyboxMaterial>>,
    time: Res<Time>,
    mut commands: Commands,
) {
    for shown in &mut skyboxes.shown {
        let State::Loaded(model) = &shown.state else {
            continue;
        };
        let model = Arc::clone(model);
        let root = commands
            .spawn((SkyboxRoot, Transform::default(), Visibility::default()))
            .id();
        let joints: Vec<Entity> = (0..model.joint_count)
            .map(|_| commands.spawn((SkyboxJoint, GlobalTransform::default(), ChildOf(root))).id())
            .collect();

        // Opaque and alpha-keyed batches first, then the blended ones, each
        // group in the model's own order.
        let mut order: Vec<usize> = (0..model.draws.len()).collect();
        order.sort_by_key(|&i| {
            m2_materials
                .get(&model.draws[i].material)
                .is_none_or(|m| m.blend > 1)
        });
        let mut parts = Vec::new();
        for (step, &index) in order.iter().enumerate() {
            let draw = &model.draws[index];
            let Some(m2) = m2_materials.get(&draw.material) else {
                continue;
            };
            let material = materials.add(SkyboxMaterial {
                params: SkyboxParams {
                    tint: Vec4::new(1.0, 1.0, 1.0, shown.weight),
                    mode: Vec4::new(f32::from(m2.blend), m2.params.alpha_cutoff, 0.0, 0.0),
                },
                texture: m2.texture.clone(),
                blend: m2.blend,
                sort: shown.slot as f32 * SORT_STEPS_PER_MODEL + step as f32,
            });
            let mut part = commands.spawn((
                Mesh3d(draw.mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                ChildOf(root),
                NoFrustumCulling,
                NotShadowCaster,
                NotShadowReceiver,
            ));
            if let Some(joints) = crate::render::models::skin_for(draw, &joints) {
                part.insert(SkinnedMesh {
                    inverse_bindposes: model.inverse_bindposes.clone(),
                    joints,
                });
            }
            parts.push((material, draw.tint));
        }
        shown.state = State::Drawn(Drawn {
            root,
            joints,
            parts,
            skeleton: model.skeleton.clone(),
            tints: model.tints.clone(),
            radius: model.model_sphere.radius,
            since: time.elapsed_secs(),
        });
    }
}

/// Marks a skybox model's root.
#[derive(Component)]
pub struct SkyboxRoot;

/// Marks a skybox model's joints, which [`pose`] writes.
#[derive(Component)]
pub struct SkyboxJoint;

/// Put each model on the camera, pose it, and write each batch's opacity: its
/// colour and transparency tracks times its light's weight.
fn pose(
    skyboxes: Res<Skyboxes>,
    time: Res<Time>,
    camera: Query<&Transform, (With<crate::world::camera::WorldCamera>, Without<SkyboxRoot>)>,
    mut roots: Query<&mut Transform, With<SkyboxRoot>>,
    mut joints: Query<&mut GlobalTransform, With<SkyboxJoint>>,
    mut materials: ResMut<Assets<SkyboxMaterial>>,
) {
    let Some(eye) = camera.iter().next().map(|t| t.translation) else {
        return;
    };
    for shown in &skyboxes.shown {
        let State::Drawn(drawn) = &shown.state else {
            continue;
        };
        let scale = if drawn.radius > 0.0 {
            SKYBOX_RADIUS / drawn.radius
        } else {
            SKYBOX_RADIUS
        };
        let placement = Transform {
            translation: eye,
            scale: Vec3::splat(scale),
            ..default()
        };
        if let Ok(mut transform) = roots.get_mut(drawn.root) {
            *transform = placement;
        }
        let raw = ((time.elapsed_secs() - drawn.since).max(0.0) * 1000.0) as u32;
        let window = drawn.skeleton.as_ref().and_then(|s| s.sequences.first());
        // The first sequence loops when bit 0 of its flags is clear, which it
        // is in every skybox model.
        let elapsed = match &drawn.skeleton {
            Some(skeleton) if window.is_none_or(|s| s.flags & 1 == 0) => skeleton.phase(0, raw),
            _ => raw,
        };
        let world_from_model = placement.compute_affine();
        if let (Some(skeleton), false) = (&drawn.skeleton, drawn.joints.is_empty()) {
            let posed = skeleton.pose(0, elapsed, raw, None, Default::default());
            for (bone, &joint) in posed.iter().zip(drawn.joints.iter()) {
                if let Ok(mut transform) = joints.get_mut(joint) {
                    *transform = GlobalTransform::from(
                        world_from_model * Affine3A::from_mat4(axes::pose_to_bevy(bone)),
                    );
                }
            }
            // The identity joint at the end, which weightless vertices follow.
            if let Some(&last) = drawn.joints.last() {
                if let Ok(mut transform) = joints.get_mut(last) {
                    *transform = GlobalTransform::from(world_from_model);
                }
            }
        }
        for (handle, tint) in &drawn.parts {
            let colour = match (tint, &drawn.tints) {
                (Some(tint), Some(tints)) => tints.sample_in(*tint, window, elapsed, raw),
                _ => [1.0; 4],
            };
            let wanted = Vec4::new(colour[0], colour[1], colour[2], colour[3] * shown.weight);
            // Written only when it moves: a write re-uploads the material.
            let moved = materials
                .get(handle)
                .is_some_and(|m| (m.params.tint - wanted).abs().max_element() > 1.0 / 512.0);
            if moved {
                if let Some(mut material) = materials.get_mut(handle) {
                    material.params.tint = wanted;
                }
            }
        }
    }
}

/// What a skybox batch's fragment shader reads.
#[derive(Clone, Copy, ShaderType)]
pub struct SkyboxParams {
    /// rgb: the batch's colour track. a: its transparency track times its
    /// light's weight.
    pub tint: Vec4,
    /// x: the batch's M2 blend mode. y: the alpha below which an alpha-keyed
    /// batch (mode 1) discards a texel.
    pub mode: Vec4,
}

/// One skybox batch: its texture, how it blends, and its place in the sky's
/// draw order.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(SkyboxKey)]
pub struct SkyboxMaterial {
    #[uniform(0)]
    pub params: SkyboxParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
    /// The M2 blend mode: 0 opaque, 1 alpha key, 2 alpha, 3 additive, 4
    /// additive by alpha, 5 modulate, 6 modulate by two.
    pub blend: u16,
    /// Added to [`crate::render::sky::SKYBOX_SORT`].
    pub sort: f32,
}

/// The pipeline variant: the blend mode alone.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SkyboxKey {
    blend: u16,
}

impl From<&SkyboxMaterial> for SkyboxKey {
    fn from(material: &SkyboxMaterial) -> Self {
        SkyboxKey {
            blend: material.blend,
        }
    }
}

impl Material for SkyboxMaterial {
    fn fragment_shader() -> ShaderRef {
        crate::render::shader::SKYBOX.into()
    }

    /// Every batch is in the transparent phase, whatever its blend mode, so
    /// that [`Self::depth_bias`] orders all of them; an opaque batch's
    /// fragment writes alpha equal to its light's weight, which is 1 where
    /// the skybox is fully in force.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn depth_bias(&self) -> f32 {
        crate::render::sky::SKYBOX_SORT + self.sort
    }

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
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The model is seen from inside, and two of the five skyboxes'
        // batches are not flagged two-sided.
        descriptor.primitive.cull_mode = None;
        crate::render::sky::behind_the_world(descriptor);
        // The M2 blend modes, with opaque and alpha-keyed batches blended by
        // their alpha so that a skybox at part weight fades. The shader writes
        // alpha 1 for them at full weight, which is the unblended result.
        let (src, dst) = match key.bind_group_data.blend {
            3 => (BlendFactor::One, BlendFactor::One),
            4 => (BlendFactor::SrcAlpha, BlendFactor::One),
            5 => (BlendFactor::Dst, BlendFactor::Zero),
            6 => (BlendFactor::Dst, BlendFactor::Src),
            _ => (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                let component = BlendComponent {
                    src_factor: src,
                    dst_factor: dst,
                    operation: BlendOperation::Add,
                };
                target.blend = Some(BlendState {
                    color: component,
                    alpha: component,
                });
            }
        }
        Ok(())
    }
}
