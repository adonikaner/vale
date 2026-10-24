//! The stage's view: the target kept the pane's size, the stage camera on its
//! own orbit and drawn into the pane, the hover pick through it, and the pane's
//! turn and zoom.

use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy_egui::egui;
use super::{Stage, StageCamera};

/// Keep the render target the size of the pane, and register it with egui.
///
/// Resized rather than made once, because the pane is resizable and the window
/// is: an image stretched from 300 points to 900 is a preview drawn at a third
/// of the resolution it is shown at. A rebuild costs one texture allocation and
/// happens only when the size actually changes.
pub(crate) fn keep_the_target(
    mut stage: ResMut<Stage>,
    mut images: ResMut<Assets<Image>>,
    mut contexts: bevy_egui::EguiContexts,
) {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

    let Some(want) = stage.pane else { return };
    let want = want.max(UVec2::new(16, 16));
    // Compared against the image itself rather than against a remembered
    // size: a `Local` outlives a teardown, and after one the remembered size
    // matches an image nothing is rendering into any more.
    let have = stage
        .target
        .as_ref()
        .and_then(|handle| images.get(handle))
        .map(|image| image.size());
    if have == Some(want) {
        return;
    }

    let mut image = Image::new_uninit(
        Extent3d {
            width: want.x,
            height: want.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::all(),
    );
    // Both usages. A render target has to be written by a pass and read by
    // egui's own sampler, and an image that declares only one of the two draws
    // as a black rectangle and raises a validation error.
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;

    let handle = images.add(image);
    stage.texture = Some(contexts.add_image(bevy_egui::EguiTextureHandle::Strong(handle.clone())));
    stage.target = Some(handle);
}

/// The stage camera stands on the stage's orbit and draws into the pane's
/// image.
///
/// The world camera is not moved. The client's passes that cull, pose and
/// billboard an entity do it against the camera that draws the entity's layer
/// (`vale_client::render::views`), and the actors are on [`super::STAGE_LAYER`]
/// alone, so they are seen from this camera, and the viewport keeps showing
/// the world while the preview is open in another window. The field of view is
/// the world camera's, so the picture frames what the client would.
pub(crate) fn follow_the_pane(
    mut commands: Commands,
    stage: Res<Stage>,
    time: Res<Time>,
    world: Query<
        &Projection,
        (
            With<vale_client::world::camera::WorldCamera>,
            Without<StageCamera>,
        ),
    >,
    mut camera: Query<(Entity, &mut Camera, &mut Transform, &mut Projection), With<StageCamera>>,
    mut aimed_at: Local<Option<Handle<Image>>>,
) {
    let Ok((entity, mut camera, mut placement, mut projection)) = camera.single_mut() else {
        return;
    };
    let Some(target) = stage.target.clone() else {
        camera.is_active = false;
        return;
    };
    camera.is_active = stage.is_open();
    // `RenderTarget` is a component in this Bevy and not a field of `Camera`.
    // It is inserted when the handle changes rather than every frame:
    // inserting a component marks the entity changed, and the render world
    // rebuilds a view for a camera whose target it thinks moved.
    if aimed_at.as_ref() != Some(&target) {
        *aimed_at = Some(target.clone());
        commands
            .entity(entity)
            .insert(RenderTarget::Image(ImageRenderTarget {
                handle: target,
                scale_factor: 1.0,
            }));
    }

    let Ok(world_projection) = world.single() else {
        return;
    };
    let wanted = own_view(&stage, time.elapsed_secs());
    // Written only when it moves, for the reason `world::camera::frame_the_view`
    // gives: a `Mut` dereferenced marks the component changed.
    if *placement != wanted {
        *placement = wanted;
    }
    if let (Projection::Perspective(world), Projection::Perspective(stage_projection)) =
        (world_projection, &*projection)
    {
        if (stage_projection.fov - world.fov).abs() > 1e-4 {
            if let Projection::Perspective(own) = &mut *projection {
                own.fov = world.fov;
            }
        }
    }
}

/// Where the pane's camera stands.
///
/// The stage's orbit numbers are the rig's (a target, a distance, a pitch and
/// a yaw in the world's own axes), so this is the rig's arithmetic done once
/// rather than a second convention. [`follow_the_pane`] is the only caller.
fn own_view(stage: &Stage, spin: f32) -> Transform {
    let at = stage.look_at.unwrap_or_else(|| {
        let middle = vale_client::render::axes::to_wow(stage.middle);
        vale_client::render::axes::to_bevy([middle[0], middle[1], middle[2] + 1.2])
    });
    let yaw = match stage.spinning {
        true => stage.yaw + spin * 0.4,
        false => stage.yaw,
    };
    // The rig's own frame: the eye sits behind the focus along the yaw, lifted
    // by the pitch. `render::axes` is the one place the two conventions meet.
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = stage.pitch.sin_cos();
    let back = [
        -cos_yaw * cos_pitch * stage.distance,
        -sin_yaw * cos_pitch * stage.distance,
        sin_pitch * stage.distance,
    ];
    let focus = vale_client::render::axes::to_wow(at);
    let eye = vale_client::render::axes::to_bevy([
        focus[0] + back[0],
        focus[1] + back[1],
        focus[2] + back[2],
    ]);
    Transform::from_translation(eye).looking_at(at, Vec3::Y)
}

/// Route the hover pick through the pane.
///
/// The client's pick casts the window's pointer through the world camera, and
/// the picture is the stage camera's, drawn into one rectangle of a window at
/// another aspect. So a pointer over the pane is given to the pick as the ray
/// the stage camera casts through that point of its picture, through the
/// client's `HoverRay` seam, and taken away the moment the pointer leaves the
/// pane. The pick then lights the character the pointer is over, and nothing
/// else in the editor sees a moved pointer.
pub(crate) fn aim_the_pick(
    stage: Res<Stage>,
    camera: Query<(&Camera, &GlobalTransform), With<StageCamera>>,
    mut ray: ResMut<vale_client::interface::target::HoverRay>,
) {
    let wanted = stage
        .pointer
        .filter(|_| stage.is_open())
        .and_then(|(ndc, _)| {
            let (camera, placement) = camera.single().ok()?;
            let size = camera.logical_viewport_size()?;
            let at = Vec2::new((ndc.x + 1.0) * 0.5 * size.x, (1.0 - ndc.y) * 0.5 * size.y);
            camera.viewport_to_world(placement, at).ok()
        });
    if ray.0 != wanted {
        ray.0 = wanted;
    }
}

/// Turn and zoom the stage.
///
/// Driven by the pane's own widget response, not by a hit test: the image
/// is an ordinary egui widget, so egui has already decided whether a drag
/// belongs to it. A drag that began on the form and moved over the preview is
/// not the preview's. The shell's own chrome rule prevents the same fault one
/// layer up.
impl Stage {
    pub(crate) fn turn(&mut self, by: egui::Vec2) {
        self.yaw -= by.x * 0.01;
        self.pitch = (self.pitch + by.y * 0.008).clamp(-1.2, 1.3);
    }

    /// `by` is egui's scroll delta in points, which is fifty or so per
    /// notch and decays over several frames rather than arriving in one.
    ///
    /// The zoom is proportional to the delta, at a rate that makes one notch
    /// about a tenth of the distance, and clamped per frame so a trackpad's
    /// fling cannot move the camera through the floor. Taking only the sign
    /// of the delta and multiplying by 0.12 a frame does not work: the delta
    /// lasts several frames, so one notch runs the whole range and any scroll
    /// goes all the way in or all the way out.
    pub(crate) fn zoom(&mut self, by: f32) {
        let step = 1.0 - (by * 0.002).clamp(-0.2, 0.2);
        self.distance = (self.distance * step).clamp(2.5, 40.0);
    }
}
