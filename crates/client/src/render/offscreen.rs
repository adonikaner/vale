//! Render-to-texture pictures: one model drawn by its own camera into an
//! image, alone on a render layer.
//!
//! [`super::portraits`] and [`super::paperdoll`] both draw a dressed unit into
//! an image the interface painter samples. This module holds what the two
//! share: the render-layer table, the lowest free layer in a range, the target
//! image, the spawn of the model and the camera ([`spawn`]), the joint write,
//! the unit lookup by token, and the take-down ([`Picture::take_down`]).
//!
//! ## The render layers
//!
//! A picture is isolated by its render layer: its model and its camera are on
//! a layer nothing else is on, so the camera draws that model and nothing
//! else. Two models on one layer are drawn into each other's image. [`layers`]
//! assigns every layer this crate uses, and a range for a host app's own
//! offscreen scenes, in one table, so no two passes take the same layer.

use std::ops::Range;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::render::mesh::skinning::SkinnedMesh;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

use vale_assets::look::portrait::Framing;
use vale_assets::world::m2::M2Skeleton;

use crate::render::axes;
use crate::render::models::material::M2Material;
use crate::render::models::ModelAssets;
use crate::world::entities::Joint;

/// Every render layer in the app, by user.
///
/// ```text
/// 0        the world, and every entity that sets no layer
/// 1..=9    the unit-frame portraits, one each
/// 10..=14  the paper dolls, one each
/// 15..=30  a host app's own offscreen scenes
/// 31       the present pass: the window's quad and the interface
/// ```
pub mod layers {
    use std::ops::Range;

    /// The world's layer. A camera with no `RenderLayers` sees only this one.
    pub const WORLD: usize = 0;
    /// The portraits on the unit frames, one layer each, so its length is the
    /// most portraits kept at once. Nine: the player, the target,
    /// target-of-target, four party members, the pet and one spare. The cap
    /// exists because the interface, not this client, chooses the tokens:
    /// without it, an addon asking for a portrait of `"party8"` would create
    /// a new camera each time.
    pub(crate) const PORTRAITS: Range<usize> = 1..10;
    /// The paper dolls, one layer each. Its length is the number of frames
    /// that can hold one, not a performance budget.
    ///
    /// `Interface\FrameXML\` declares five `<PlayerModel>`-family frames, and
    /// four of them are pointed at a unit by `SetUnit`: `CharacterModelFrame`,
    /// `PetModelFrame`, `DressUpModel` and `TabardModel`. The fifth,
    /// `PetStableModel`, is pointed by `SetPetStablePaperdoll(frame)`. A sixth
    /// would come from an addon, and gets no layer.
    pub(crate) const DOLLS: Range<usize> = 10..15;
    /// Layers a host app takes for its own offscreen scenes. No pass in this
    /// crate draws on them.
    pub const HOST: Range<usize> = 15..31;
    /// The present pass, which must not see the world's layer; see
    /// [`crate::render::present::PRESENT_LAYER`].
    pub(crate) const PRESENT: usize = 31;
}

/// The lowest layer in `range` that `in_use` does not claim.
pub(crate) fn free_layer(mut range: Range<usize>, in_use: impl Fn(usize) -> bool) -> Option<usize> {
    range.find(|layer| !in_use(*layer))
}

/// The camera order a picture on `layer` draws at.
///
/// Before the world camera, which is order 0: a camera drawing into an image
/// that is sampled later in the frame must finish first. One order per layer,
/// so no two pictures share one, and a target is never sampled before it is
/// drawn into.
pub(crate) fn order(layer: usize) -> isize {
    -1 - layer as isize
}

/// One picture's entities and assets: the image, the camera that draws into
/// it, and the model on its own layer.
pub(crate) struct Picture {
    /// The image the camera draws into.
    pub(crate) image: Handle<Image>,
    pub(crate) camera: Entity,
    /// The model root; despawning it takes the batches and joints with it.
    pub(crate) root: Entity,
    pub(crate) joints: Vec<Entity>,
    pub(crate) skeleton: Option<Arc<M2Skeleton>>,
    /// The render layer this model and camera have to themselves.
    pub(crate) layer: usize,
}

impl Picture {
    /// Despawn the model and the camera. The image asset is freed when the
    /// picture's handle, the last strong one, is dropped with it.
    pub(crate) fn take_down(&self, commands: &mut Commands) {
        commands.entity(self.root).despawn();
        commands.entity(self.camera).despawn();
    }
}

/// How a picture's camera draws, where the two passes differ.
pub(crate) struct Lens {
    /// The target's width over its height, which the projection is built at.
    pub(crate) aspect: f32,
    /// What the target is cleared to before the model is drawn.
    pub(crate) clear: Color,
    pub(crate) msaa: Msaa,
    pub(crate) name: &'static str,
}

/// Spawn the image, the camera and the model for one picture.
///
/// `root` is the model root's components; its `Transform` turns the parts
/// that are not skinned. `material` chooses each part's material from the
/// model's own. The camera stands along `framing`, the model file's camera.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    model: &ModelAssets,
    framing: Framing,
    size: UVec2,
    layer: usize,
    root: impl Bundle,
    lens: Lens,
    mut material: impl FnMut(&Handle<M2Material>) -> Handle<M2Material>,
) -> Picture {
    let image = images.add(target_image(size));
    let layers = RenderLayers::layer(layer);

    // --- The model, alone on its render layer at the origin ---

    let root = commands.spawn(root).id();
    let mut joints = Vec::new();
    if model.skeleton.is_some() {
        joints.extend((0..model.joint_count).map(|_| {
            commands
                .spawn((GlobalTransform::default(), Joint, ChildOf(root)))
                .id()
        }));
    }
    for draw in &model.draws {
        let mut part = commands.spawn((
            Mesh3d(draw.mesh.clone()),
            MeshMaterial3d(material(&draw.material)),
            Transform::default(),
            ChildOf(root),
            // Set on every drawn entity, because Bevy reads `RenderLayers` per
            // entity and does not propagate it down a hierarchy. A part without
            // the layer is drawn by the world camera, at the origin of the map.
            layers.clone(),
            // The camera can sit inside the model's bounding box, and the
            // declared box is the widest extent over all the model's
            // animations, not the pose drawn. Culling against it can cull a
            // model out of its own picture, which looks the same as a model
            // that failed to load.
            NoFrustumCulling,
        ));
        // Bind the batch's own bones, not the model's whole skeleton.
        // `models::skin_for` is the one place that decides this. Binding the
        // whole skeleton poses every vertex by the wrong bone and stretches the
        // model across the picture.
        if let Some(joints) = crate::render::models::skin_for(draw, &joints) {
            part.insert(SkinnedMesh {
                inverse_bindposes: model.inverse_bindposes.clone(),
                joints,
            });
        }
    }

    // --- The camera, along the model file's camera ---

    let eye = axes::to_bevy(framing.eye);
    // A degenerate framing has its aim point moved rather than aimed at the
    // eye. An eye and an aim at the same place make `looking_at` produce a NaN
    // basis, and a NaN view matrix breaks culling for the whole frame, for
    // every mesh in the world. `render::glue::aim_camera` makes the same check
    // and returns instead; that is not possible here because the camera must
    // exist.
    let aim = match axes::to_bevy(framing.aim) {
        target if eye.distance_squared(target) < 1e-6 => eye - Vec3::Z,
        target => target,
    };
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera {
                order: order(layer),
                clear_color: ClearColorConfig::Custom(lens.clear),
                ..default()
            },
            // The render target is its own component in Bevy 0.19, as in
            // `world::camera::spawn`, which writes `WorldFrame::target()`.
            RenderTarget::Image(ImageRenderTarget {
                handle: image.clone(),
                scale_factor: 1.0,
            }),
            // The world camera sets the same space for the same reason. Every
            // model shader in this client ends in `atmosphere::to_frame` and
            // writes the game's own bytes, so a camera in the default space
            // would encode them a second time and give the interface a
            // washed-out model. See `crate::render::present`.
            bevy::camera::CompositingSpace::Srgb,
            Projection::Perspective(PerspectiveProjection {
                // `M2Camera::fov` is a diagonal angle
                // (`render::lens::vertical_fov` records the evidence), taken
                // at the target's own aspect. Not clamped, because this target
                // is not the window: see `render::lens::framed_vertical_fov`.
                fov: crate::render::lens::vertical_fov(framing.fov, lens.aspect),
                near: framing.near.max(0.01),
                far: framing.far.max(framing.near.max(0.01) * 10.0),
                aspect_ratio: lens.aspect,
                ..default()
            }),
            // Both set explicitly, because `Camera3d` defaults to multisampled
            // and tonemapped. Tonemapping is always off.
            lens.msaa,
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            layers,
            Transform::from_translation(eye).looking_at(aim, Vec3::Y),
            Name::new(lens.name),
        ))
        .id();

    Picture {
        image,
        camera,
        root,
        joints,
        skeleton: model.skeleton.clone(),
        layer,
    }
}

/// One skeleton's joints at `world`, with the identity joint last, which
/// weightless vertices ride; without it the skinning shader collapses them to
/// the origin. `world::entities` and `render::glue` spawn the same extra joint.
pub(crate) fn write_joints(
    joints: &mut Query<&mut GlobalTransform, With<Joint>>,
    entities: &[Entity],
    pose: &[[f32; 12]],
    world: Affine3A,
) {
    for (bone, &joint) in pose.iter().zip(entities.iter()) {
        if let Ok(mut transform) = joints.get_mut(joint) {
            *transform = GlobalTransform::from(world * axes::pose_to_bevy_affine(bone));
        }
    }
    if let Some(&last) = entities.last() {
        if let Ok(mut transform) = joints.get_mut(last) {
            *transform = GlobalTransform::from(world);
        }
    }
}

/// The object type of a token's unit, which selects the display table its
/// display id is resolved through. A token that names no unit reads as a
/// creature.
pub(crate) fn unit_kind(
    units: &crate::interface::api::Units,
    token: &str,
) -> vale_protocol::state::update::ObjectType {
    crate::interface::api::UnitId::parse(token)
        .and_then(|id| units.get(id))
        .map(|unit| unit.kind)
        .unwrap_or(vale_protocol::state::update::ObjectType::Unit)
}

/// The image a picture is drawn into.
///
/// The format is `Rgba8UnormSrgb`, for the same reason
/// `render::present::WorldFrame` uses it. The camera's main texture holds raw
/// bytes (`CompositingSpace::Srgb`), and Bevy's upscaling blit applies
/// `SRGB_TO_LINEAR` on output because it assumes the destination re-encodes;
/// this image is that destination. With a linear format the linear value would
/// be stored as is, and the model would render as dark as dusk in daylight, the
/// same artefact that note describes for the world frame.
pub fn target_image(size: UVec2) -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: size.x.max(1),
            height: size.y.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        // Transparent black, so a picture whose camera has not run yet shows
        // the frame's own art rather than a coloured rectangle.
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        // Render world only: nothing on the CPU reads this image, and the
        // painter samples it on the GPU like any other texture.
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage =
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No two users in the table share a layer. An overlap would draw two
    /// models into each other's image, and both would still be well-formed
    /// images, so nothing else would report it.
    #[test]
    fn the_layer_ranges_never_overlap_and_none_is_the_worlds() {
        let ranges = [layers::PORTRAITS, layers::DOLLS, layers::HOST];
        for (i, a) in ranges.iter().enumerate() {
            assert!(!a.contains(&layers::WORLD), "layer 0 is the world's");
            assert!(!a.contains(&layers::PRESENT), "{a:?} holds the present layer");
            for b in &ranges[i + 1..] {
                assert!(a.end <= b.start || b.end <= a.start, "{a:?} overlaps {b:?}");
            }
        }
        // The dolls start where the portraits end.
        assert_eq!(layers::DOLLS.start, layers::PORTRAITS.end);
        assert_eq!(layers::DOLLS.start, 10);
    }

    /// Layers are handed out lowest first and run out rather than repeating.
    #[test]
    fn a_free_layer_is_the_lowest_unclaimed_one_in_its_range() {
        let mut used = Vec::new();
        for _ in layers::PORTRAITS {
            let layer = free_layer(layers::PORTRAITS, |l| used.contains(&l)).expect("a free layer");
            assert!(!used.contains(&layer), "layer {layer} handed out twice");
            used.push(layer);
        }
        assert_eq!(free_layer(layers::PORTRAITS, |l| used.contains(&l)), None);
        assert_eq!(used.first(), Some(&layers::PORTRAITS.start));
    }

    /// Every picture camera draws before the world camera, which is order 0,
    /// and no two share an order. If two cameras shared an order, a target
    /// could be sampled before it was drawn into: a picture one frame old, or
    /// an empty square on the first frame.
    #[test]
    fn every_picture_camera_draws_before_the_world_and_none_share_an_order() {
        let orders: Vec<isize> = layers::PORTRAITS.chain(layers::DOLLS).map(order).collect();
        assert!(orders.iter().all(|o| *o < 0));
        let mut sorted = orders.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), orders.len(), "no two share an order");
    }
}
