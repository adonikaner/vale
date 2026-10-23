//! The stage's view: the target kept in frame, the world camera parked on the
//! stage, the stage camera drawn into the pane, and the pane's turn and zoom.

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

/// Park the editor's own camera on the stage while the preview is open.
///
/// Not for the picture — that is the stage camera's — but because the client's
/// particle, LOD and billboard passes all ask for the world camera and use it
/// as the one point of view there is. See the module comment. Restored to where
/// the person left it when the preview closes.
pub(crate) fn park_the_world_camera(
    stage: Res<Stage>,
    time: Res<Time>,
    playing: Res<crate::playtest::Playtest>,
    mut camera: ResMut<crate::camera::EditorCamera>,
    mut parked: Local<Option<(Vec3, f32, f32, f32)>>,
) {
    // Nothing to park during a playtest. `camera::drive` returns while the
    // editor is not driving, so every write below would reach a resource the
    // rig never reads — and the saved value would then be restored over the
    // editor's own camera on the way out of the playtest, moving it to wherever
    // a preview had last been opened. [`follow_the_pane`] aims the pane itself
    // in that state; see the note there on what it costs.
    if playing.playing() {
        return;
    }
    match stage.is_open() {
        true => {
            if parked.is_none() {
                *parked = Some((camera.target, camera.distance, camera.pitch, camera.yaw));
            }
            // The stage's orbit is the rig's orbit, in the rig's own terms.
            // The stage camera used to compute an eye of its own from these
            // three numbers in Bevy's axes, and this converted them into the
            // rig's — with the yaw mirrored, so the world camera looked from
            // the other side of the actors. Everything the client billboards
            // and culls against the world camera was then built for a mirrored
            // view, and the hover pick lit the other character. The rig is
            // the one authority now: this writes it, and `follow_the_pane`
            // copies what the client placed.
            camera.target = match stage.look_at {
                Some(at) => Vec3::from(vale_client::render::axes::to_wow(at)),
                None => {
                    let middle = vale_client::render::axes::to_wow(stage.middle);
                    Vec3::new(middle[0], middle[1], middle[2] + 1.2)
                }
            };
            camera.distance = stage.distance;
            camera.pitch = stage.pitch;
            camera.yaw = match stage.spinning {
                true => stage.yaw + time.elapsed_secs() * 0.4,
                false => stage.yaw,
            };
            camera.wants_the_ground = false;
        }
        false => {
            if let Some((target, distance, pitch, yaw)) = parked.take() {
                camera.target = target;
                camera.distance = distance;
                camera.pitch = pitch;
                camera.yaw = yaw;
            }
        }
    }
}

/// The stage camera stands exactly where the world camera stands, and
/// draws into the pane's image.
///
/// Copied from the client's own camera rather than computed here, because
/// every pass that billboards, culls or LODs asks the world camera and the
/// picture is only right when the two coincide — see
/// [`park_the_world_camera`], which is what puts the world camera on the
/// stage's orbit. The field of view is copied too, so the picture frames what
/// the client would, and so the hover pick's arithmetic in [`aim_the_pick`]
/// is a ratio of aspects and nothing else.
pub(crate) fn follow_the_pane(
    mut commands: Commands,
    stage: Res<Stage>,
    playing: Res<crate::playtest::Playtest>,
    time: Res<Time>,
    world: Query<
        (&Transform, &Projection),
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

    let Ok((world_placement, world_projection)) = world.single() else {
        return;
    };
    // During a playtest the pane aims itself, because the world camera is
    // the character's and [`park_the_world_camera`] cannot move it. Copying it
    // then would point the pane at whatever the player is looking at, with the
    // actors standing off to one side of it: the pane would draw the world and
    // none of the preview, and the storyboard and the lab would show nothing
    // at all.
    //
    // The cost is the fault the copying exists to prevent: the client's
    // particle, LOD and billboard passes ask for the world camera, so an
    // emitter in the pane is billboarded against the player's point of view
    // rather than the pane's. A picture taken during a playtest has that
    // error in it. Models, attachments, skins and animation are unaffected,
    // and those are all that the attachment lab and the model view show.
    let wanted = match playing.playing() {
        true => own_view(&stage, time.elapsed_secs()),
        false => *world_placement,
    };
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

/// Where the pane's own camera stands, for the state in which it cannot
/// copy the world's.
///
/// The stage's three orbit numbers are the rig's — a target, a distance, a
/// pitch and a yaw in the world's own axes — so this is the rig's arithmetic
/// done once rather than a second convention. See [`follow_the_pane`], which is
/// the only caller and says when.
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
/// The client's pick casts the window's cursor through the world camera and
/// the world camera renders the whole window; the picture is the stage camera's
/// and covers one rectangle of it at another aspect. So a pointer over the pane
/// is re-expressed as the window position that casts the same ray: the two
/// cameras coincide and share a vertical field of view, so a normalised
/// position in the pane maps to the window by the ratio of the two aspects in
/// x and unchanged in y. It goes in through `HoverProbe`, the client's own
/// planted-pointer seam, and comes out again the moment the pointer leaves the
/// pane — so the pick lights the character the pointer is over, and nothing
/// else in the editor sees a moved pointer.
pub(crate) fn aim_the_pick(
    stage: Res<Stage>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut probe: ResMut<vale_client::HoverProbe>,
) {
    let Some((ndc, pane_aspect)) = stage.pointer.filter(|_| stage.is_open()) else {
        if probe.0.is_some() {
            probe.0 = None;
        }
        return;
    };
    let Ok(window) = windows.single() else { return };
    let size = window.size();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let window_aspect = size.x / size.y;
    let x = ndc.x * pane_aspect / window_aspect;
    let y = ndc.y;
    let planted = Vec2::new((x + 1.0) * 0.5 * size.x, (1.0 - y) * 0.5 * size.y);
    if probe.0 != Some(planted) {
        probe.0 = Some(planted);
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
