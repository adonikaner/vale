//! The world is drawn into the game's own byte values, and this module puts
//! them on the screen.
//!
//! Every other module in this directory decides what colour a pixel is. This
//! one decides what the number in the framebuffer means, because that decides
//! how two draws combine when they are blended, which no shader can see.
//!
//! ## Why the blend has to be taken in byte space
//!
//! `atmosphere.wgsl` documents that this game adds colours as the bytes its
//! files state, because the 1.12.1 client is fixed-function: no sRGB sampler
//! state, no linear working space, no display transform, and a plain
//! `X8R8G8B8` back buffer with no `D3DRS_SRGBWRITEENABLE`. The sun plus the
//! fill, the ambient plus `MOCV`, the air mixed into a shaded fragment — all of
//! them are taken in that space and decoded once.
//!
//! The blend is a sum too, and this renderer used to take it in the wrong
//! space. Bevy's main texture is `Rgba8UnormSrgb` by default, so the ROP
//! decodes the destination to linear, adds, and re-encodes: each term of the
//! sum is decoded separately, by fixed-function hardware no shader can reach.
//! That is the error `atmosphere.wgsl` avoids, in the one place that file
//! cannot control.
//!
//! The error is largest where the game uses additive blending most.
//! `vale model` counts 1,137 of the game's 1,332 spell-effect batches at
//! blend 4 (`SRCALPHA, ONE`) — an effect is a stack of additive layers, and a
//! stack is where the two spaces diverge fastest.
//! `Spells\ArcaneExplosion_Base.m2` is six batches, every one of them blend 4,
//! whose three dome shells share one texture averaging 26/5/40: over ground at
//! byte 120 the client's byte-space adds reach 198 and the same three layers
//! summed in linear reach 129. That gap is why effects drew faint and hard to
//! see; the art, the alpha and the blend factors were all correct.
//!
//! So the world camera takes `CompositingSpace::Srgb`, which makes its main
//! texture a raw `Rgba8Unorm`; every world shader ends in `gamma::to_frame`,
//! which writes the encoded byte that texture expects; and the hardware adds
//! bytes to bytes. An opaque draw is unchanged apart from rounding: the encode
//! moved out of the target's view and into the fragment, and a before/after
//! at one framing measures 0.87 of a byte mean over 75,360 sampled pixels, all
//! of it the rounding of one extra 8-bit round trip. Only blended draws change.
//!
//! ## Why there is a second camera
//!
//! [`present`] is a full-screen quad on a 2D camera that copies [`WorldFrame`]
//! to the window. On its own that would be pure overhead: Bevy can give the
//! window a byte-space main texture directly, and its own upscaling node
//! already does this blit.
//!
//! It exists because `bevy_egui` cannot draw into a byte-space main texture.
//! Its pipeline's colour format is hard-coded `Rgba8UnormSrgb`
//! (`render/systems.rs`, and again in the extracted UI view), and it draws into
//! whatever `ViewTarget` its camera has, so a byte-space main texture is a wgpu
//! validation failure — `the RenderPass uses textures with formats
//! [Rgba8Unorm] but the RenderPipeline with 'egui_pipeline' label uses
//! [Rgba8UnormSrgb]` — and the app quits on the first frame the HUD draws.
//!
//! Two cameras on one window do not solve it either: their main textures are
//! keyed by format, so a second camera in the other space gets a different
//! texture and its own blit over the top, which erases the world. The split has
//! to be world to an image, UI to the window, which is this module.
//!
//! The cost is one full-screen textured quad per frame. The UI keeps a linear
//! target, which is what egui expects.
//!
//! When `bevy_egui` can read its target's format, this module can be removed
//! and the world camera can take `CompositingSpace::Srgb` against the window
//! directly. Nothing else would change: the shaders already write bytes.

use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin, MeshMaterial2d};
use bevy::window::{PrimaryWindow, WindowResized};

/// The image the world is drawn into, at the window's size.
///
/// The format is `Rgba8UnormSrgb`, which is consistent with the module note.
/// The target that matters for blending is the camera's main texture, and
/// `CompositingSpace::Srgb` makes that a raw `Rgba8Unorm` holding the game's own
/// bytes — that is where every `SRCALPHA, ONE` in the world is summed. This is
/// the camera's out texture, which the main texture is blitted to at the end
/// of the frame, and Bevy's own upscaling node applies `SRGB_TO_LINEAR` on the
/// way (it assumes the destination is a swapchain that will re-encode). So this
/// texture has to re-encode.
///
/// An `Rgba8Unorm` here stores the blit's linear output raw, `present.wgsl`
/// decodes it a second time, and the world is drawn as dark as dusk in the
/// middle of the afternoon, with no error anywhere. It also stores light
/// linearly in eight bits, which bands the darks.
#[derive(Resource)]
pub struct WorldFrame(pub Handle<Image>);

impl WorldFrame {
    /// What [`crate::world::camera`] points its `Camera` at.
    pub fn target(&self) -> RenderTarget {
        RenderTarget::Image(ImageRenderTarget {
            handle: self.0.clone(),
            scale_factor: 1.0,
        })
    }
}

/// The size the frame is created at, before a window has been measured.
///
/// Replaced by [`resize`] on the first frame there is a window to read. It is
/// not 1x1 because a camera whose target has no area spends that frame
/// producing nothing, and a zero-area viewport is the kind of thing that trips
/// a driver rather than producing a small picture.
const INITIAL: UVec2 = UVec2::new(1280, 720);

pub struct PresentPlugin;

impl Plugin for PresentPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/present.wgsl");
        // Built here rather than in a startup system, because
        // `world::camera::spawn` needs the handle to aim at and both are
        // `Startup`: Bevy runs plugin `build` before every schedule, so a
        // resource inserted here exists for every startup system without an
        // `.after()`.
        let frame = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(frame_image(INITIAL));
        app.insert_resource(WorldFrame(frame))
            .add_plugins(Material2dPlugin::<PresentMaterial>::default())
            // Before any camera is added, so `bevy_egui`'s "attach the primary
            // context to the first camera you see" never gets the chance to
            // pick the world camera — whose target is the image and whose
            // format is the one egui cannot draw into. `present` marks the
            // right one by hand.
            .add_systems(PreStartup, hand_egui_its_camera)
            .add_systems(Startup, present)
            .add_systems(Update, resize);
    }
}

/// One frame's worth of world, at `size`.
fn frame_image(size: UVec2) -> Image {
    let mut image = Image::new_uninit(
        Extent3d {
            width: size.x.max(1),
            height: size.y.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        // See [`WorldFrame`]: this is the out texture, and the encode the
        // camera's byte-space main texture lost to Bevy's blit is put back here.
        TextureFormat::Rgba8UnormSrgb,
        // The render world only: nothing on the CPU ever reads this, and the
        // screenshot path captures the window rather than this image.
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage =
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    // Nearest, because this is a copy and not a resample. The image is the
    // window's size and the quad covers the window exactly, so every texel lands
    // on its own pixel; a linear filter here could only soften a 1:1 blit.
    image.sampler = ImageSampler::nearest();
    image
}

/// Stop `bevy_egui` adopting whichever camera it happens to see first.
///
/// Its `setup_primary_egui_context_system` takes the first `Added<Camera>` it
/// finds, and both cameras are spawned in `Startup` — so which one it picked
/// would be query-iteration order, and picking the world camera is a validation
/// failure on the first frame the HUD draws rather than a wrong pixel. See the
/// module note.
fn hand_egui_its_camera(mut settings: ResMut<bevy_egui::EguiGlobalSettings>) {
    settings.auto_create_primary_context = false;
}

/// The one render layer the present pass sees, carried by its camera, by
/// the quad it blits, and by everything else meant to be drawn over the world.
///
/// Anything that is drawn on screen and is not the world has to carry it.
/// This is the only 2D camera in the app, so an entity without this layer is
/// drawn by no camera. `ui::mesh` — the interface painter — is the one thing
/// besides the quad that carries it. Without it, all of `Interface\` is
/// invisible: the frames load, the events fire, the scripts run, the loading
/// screen goes up and comes down on time, and no pixel of any of it is drawn.
/// See [`crate::ui::mesh`], where the layer is put on every batch.
///

/// The present camera is a `Camera2d` with an orthographic projection that maps
/// a 1x1 world to the whole window. Without a layer of its own it saw layer 0,
/// and Bevy draws gizmo lines on 2D cameras as well as on 3D ones, so every
/// gizmo in the world was drawn a second time, orthographically, at one world
/// unit to the window. Anything at Bevy `y = 0` whose extent crossed
/// `x ∈ [-0.5, 0.5]` came out as a two-pixel line across the whole window at
/// the centre row, on top of everything, with no depth test. A light's
/// falloff ring lies at world `z = 0` (Bevy `y = 0`) and runs to 2,648 yards,
/// so a light near the origin drew that line; any other gizmo crossing the
/// origin — a diagnostic overlay, one of the editor's gizmos — would too.
///
/// 31 is above every layer the portraits, the paper doll and the editor's
/// stage hand out.
pub const PRESENT_LAYER: usize = 31;

/// The camera the window sees: one quad of [`WorldFrame`], and the UI over it.
fn present(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PresentMaterial>>,
    frame: Res<WorldFrame>,
) {
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    let material = materials.add(PresentMaterial {
        frame: frame.0.clone(),
        grade: NightGrade::default(),
    });
    commands.spawn((
        Camera2d,
        Camera {
            // After the world camera, which is order 0 and draws into the image
            // this one reads. Two cameras with the same order is the world drawn
            // into a texture nobody has blitted yet.
            order: 1,
            // The quad covers every pixel, so a clear would be a full-screen
            // write thrown away — and `ClearColor` is the fog band, which would
            // be the wrong colour to see if the quad ever failed to draw.
            clear_color: ClearColorConfig::None,
            ..default()
        },
        // A 1x1 world mapped to the whole viewport, so a 1x1 quad fills the
        // screen at any window size and nothing has to be resized when the
        // window is. `Fixed` stretches rather than letterboxing, which is
        // exactly what a full-screen blit wants.
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::Fixed {
                width: 1.0,
                height: 1.0,
            },
            ..OrthographicProjection::default_2d()
        }),
        // Nothing here is edge-sampled and nothing is graded: this pass is a
        // copy. Both are named rather than defaulted because `Camera2d`'s
        // defaults are `Msaa::Sample4` and a filmic tonemapper, and either
        // would be applied to a picture that is already finished.
        Msaa::Off,
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        // The UI rides this camera — see [`hand_egui_its_camera`].
        bevy_egui::PrimaryEguiContext,
        // Only its own quad. See [`PRESENT_LAYER`]: on layer 0 this camera
        // drew every gizmo in the world a second time, flat, at one world unit
        // to the window. egui is unaffected — it is a pass on the camera, not
        // an entity on a layer.
        RenderLayers::layer(PRESENT_LAYER),
        Name::new("present"),
    ));
    commands.spawn((
        Mesh2d(quad),
        MeshMaterial2d(material),
        RenderLayers::layer(PRESENT_LAYER),
        Name::new("world frame"),
    ));
}

/// Keep the frame the size of the window.
///
/// The image is replaced under the same id rather than resized in place,
/// because the handle has to keep pointing at the same asset — the camera's
/// `RenderTarget` and the present material both hold it — and replacing the
/// `Image` under the id is what makes the render world drop the old texture
/// and allocate the new one.
///
/// Runs on the resize message and once on the first frame, so the 1280x720 the
/// plugin created is only ever what frame zero is drawn at.
///
/// ## Why the material has to be updated as well
///
/// The two holders of that handle do not resolve it the same way, and the
/// difference is a frozen picture with nothing in the log. See [`repoint`],
/// which keeps them in step.
fn resize(
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<PresentMaterial>>,
    quad: Query<&MeshMaterial2d<PresentMaterial>>,
    frame: Res<WorldFrame>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut resized: MessageReader<WindowResized>,
    mut settled: Local<bool>,
) {
    let moved = resized.read().count() > 0;
    if *settled && !moved {
        return;
    }
    let Ok(window) = window.single() else {
        return;
    };
    let size = window.physical_size();
    if size.x == 0 || size.y == 0 {
        return;
    }
    let current = images
        .get(&frame.0)
        .map(|i| UVec2::new(i.width(), i.height()));
    if current == Some(size) {
        *settled = true;
        return;
    }
    // The only failure is a handle that names no asset, which cannot happen —
    // the plugin created it and nothing removes it.
    let _ = images.insert(&frame.0, frame_image(size));
    repoint(&mut materials, &quad, &frame);
    *settled = true;
}

/// Tell the quad's material that the texture under its handle is a new one.
///
/// Without this the picture freezes after a resize. The cause is not visible
/// from this file: the camera and the material hold the same `Handle<Image>`
/// and resolve it by two different rules.
///
/// * The camera resolves it every frame.
///   `NormalizedRenderTarget::get_texture_view` looks the id up in
///   `RenderAssets<GpuImage>` at render time, so it always draws into whatever
///   texture is current.
/// * The material resolves it once. `PreparedMaterial2d`'s bind group is
///   built by `AsBindGroup` and captures a `TextureView` — a handle to that
///   frame's texture, not to the asset — and it is rebuilt only when an
///   `AssetEvent` names the material. `Material2dPlugin` does register
///   `RenderAssetPlugin::<PreparedMaterial2d<M>, GpuImage>`, but that second
///   generic is a `RenderAssetDependency`, and all it registers is ordering:
///   images are prepared before materials. It is not invalidation, and nothing
///   in Bevy walks a material's handles to see if one changed.
///
/// So [`resize`] replacing the image gives the camera a new texture and leaves
/// the quad sampling the old one, which nothing renders into any more. The world
/// camera goes on drawing, every count on the HUD goes on moving, the controls
/// go on working, and the picture stays on the last frame drawn before the
/// resize. There is no error, no warning and no dropped frame, so the symptom
/// looks like a renderer hang rather than a resize bug: the one surface that
/// stops updating is the only one that goes through this texture.
///
/// The moment it happens also varies between runs. The trigger is a
/// `WindowResized`, and the one that always arrives is the window settling from
/// the 1280x720 [`INITIAL`] to its real size, which races against the frame the
/// material's bind group is first built on. If the resize comes first the world
/// never appears at all; if it comes later the world freezes mid-session.
///
/// Writing the same handle back is enough: `Assets::get_mut` marks the asset
/// changed on `DerefMut` and queues `AssetEvent::Modified` when the guard drops,
/// which re-runs `as_bind_group` against the image prepared earlier in the same
/// frame. It also drops the last reference to the orphaned texture — which the
/// stale bind group was otherwise keeping alive, a full frame buffer leaked per
/// resize.
fn repoint(
    materials: &mut Assets<PresentMaterial>,
    quad: &Query<&MeshMaterial2d<PresentMaterial>>,
    frame: &WorldFrame,
) {
    for handle in quad {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.frame = frame.0.clone();
        }
    }
}

/// What the deep night does to a finished picture — see
/// [`crate::render::night`], which owns the option and writes this.
///
/// Two terms, both of which are display transforms rather than light: a
/// vignette, and a lift of the darks toward a cold blue. Neither belongs in
/// `atmosphere.wgsl`, because a fragment there is shaded once per overlapping
/// draw and this quad is the only surface in the client that sees the frame
/// exactly once.
#[derive(Clone, Copy, Default, ShaderType)]
pub struct NightGrade {
    /// The night factor, 0 in daylight — and the shader's early-out.
    pub night: f32,
}

/// The full-screen quad's material: the world frame, the transfer function that
/// undoes the encode the swapchain is about to redo, and the night grade.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct PresentMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub frame: Handle<Image>,
    /// Written by [`crate::render::night::grade_present`], and only when it
    /// moves: a uniform touched every frame is a bind group rebuilt every
    /// frame. See [`repoint`] for what that machinery is.
    #[uniform(2)]
    pub grade: NightGrade,
}

impl Material2d for PresentMaterial {
    fn fragment_shader() -> ShaderRef {
        crate::render::shader::PRESENT.into()
    }

    /// Opaque: this quad is the frame, and there is nothing behind it to
    /// blend with — the camera does not clear.
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Opaque
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frame is the camera's out texture, so it carries the encode.
    ///
    /// The blending space is decided by the camera's `CompositingSpace`, which
    /// makes its main texture a raw `Rgba8Unorm`; this image is what that is
    /// blitted to, through a blit that applies `SRGB_TO_LINEAR`. An
    /// `Rgba8Unorm` here stores that linear output raw and the world renders as
    /// dark as dusk in the middle of the afternoon, with no error anywhere.
    ///
    /// The test also checks the usage flags: without `RENDER_ATTACHMENT` no
    /// camera can draw to it and without `TEXTURE_BINDING` no quad can sample
    /// it, and either failure gives a black screen.
    #[test]
    fn the_frame_carries_the_encode_and_can_be_both_drawn_to_and_sampled() {
        let image = frame_image(UVec2::new(800, 600));
        assert_eq!(
            image.texture_descriptor.format,
            TextureFormat::Rgba8UnormSrgb,
            "the out texture re-encodes what Bevy's blit decoded"
        );
        let usage = image.texture_descriptor.usage;
        assert!(usage.contains(TextureUsages::RENDER_ATTACHMENT), "drawn to");
        assert!(usage.contains(TextureUsages::TEXTURE_BINDING), "sampled");
        assert_eq!((image.width(), image.height()), (800, 600));

        // A window can be dragged to nothing, and a zero-extent texture is a
        // device-lost rather than a small picture.
        let degenerate = frame_image(UVec2::ZERO);
        assert_eq!((degenerate.width(), degenerate.height()), (1, 1));
    }

    /// A resized frame updates the quad's material, or the picture freezes.
    ///
    /// The camera resolves its target through `RenderAssets<GpuImage>` every
    /// frame and the quad's bind group captured a `TextureView` once, so
    /// replacing the image left the world being drawn into a texture nothing
    /// sampled. Everything else — the HUD, the counts, the controls, the
    /// session — went on working, so it looked like a renderer hang rather than
    /// a resize bug. See [`repoint`].
    ///
    /// Asserted as the `AssetEvent::Modified` that re-runs `as_bind_group`,
    /// because that event is the whole contract with the render world and the
    /// render world is out of reach of any test in this crate: a bind group
    /// needs a GPU, and this failure needs a window that changes size. The
    /// event is what crosses between them.
    #[test]
    fn a_resized_frame_repoints_the_quad_at_the_new_texture() {
        use bevy::window::WindowResolution;

        let mut app = App::new();
        app.add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<Image>()
            .init_asset::<PresentMaterial>()
            .add_message::<WindowResized>()
            .add_systems(Update, resize);

        let frame = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(frame_image(INITIAL));
        app.insert_resource(WorldFrame(frame.clone()));
        let material = app
            .world_mut()
            .resource_mut::<Assets<PresentMaterial>>()
            .add(PresentMaterial {
                frame: frame.clone(),
                grade: NightGrade::default(),
            });
        app.world_mut().spawn(MeshMaterial2d(material.clone()));
        // Deliberately not 1280x720: `Window`'s own default is exactly
        // `INITIAL`, so a default window would agree with the frame already and
        // this test would pass by never resizing anything.
        app.world_mut().spawn((
            Window {
                resolution: WindowResolution::new(1600, 900),
                ..default()
            },
            bevy::window::PrimaryWindow,
        ));

        app.update();

        let images = app.world().resource::<Assets<Image>>();
        let image = images.get(&frame).expect("the frame outlives the resize");
        assert_eq!(
            (image.width(), image.height()),
            (1600, 900),
            "the frame must follow the window"
        );

        let mut reader = bevy::ecs::system::SystemState::<
            bevy::ecs::message::MessageReader<bevy::asset::AssetEvent<PresentMaterial>>,
        >::new(app.world_mut());
        let told = reader
            .get_mut(app.world_mut())
            .expect("the reader's only state is the message buffer")
            .read()
            .any(|event| matches!(event, bevy::asset::AssetEvent::Modified { id } if *id == material.id()));
        assert!(
            told,
            "the quad was left sampling the texture the resize orphaned — the world \
             renders into the new one and the screen freezes on the last frame before it"
        );
    }

    /// The measurement the module note quotes, as a test so that the number is
    /// checked rather than only stated.
    ///
    /// `Spells\ArcaneExplosion_Base.m2` is six batches, every one of them blend
    /// 4 (`SRCALPHA, ONE`), and its three dome shells share one texture whose
    /// mean is 26/5/40. Three of those layers over ground at byte 120: the
    /// client adds bytes and reaches 198; the same three summed in linear and
    /// encoded once reach 129. The art, the alpha and the blend factors are
    /// identical in both; the only difference is which space the hardware
    /// took the sum in, which is what `CompositingSpace::Srgb` decides.
    ///
    /// It tests arithmetic rather than this crate's code. It is the claim the
    /// byte-space blend rests on, and without the test it would be re-derived by
    /// hand whenever it is questioned.
    #[test]
    fn an_additive_stack_only_saturates_in_the_games_own_space() {
        let byte = |v: f32| (v * 255.0).round() as i32;
        let ground = 120.0 / 255.0;
        let layer = 26.0 / 255.0;

        // What the client does: three adds in the framebuffer's own bytes.
        assert_eq!(byte(ground + 3.0 * layer), 198);

        // What a linear-storage target does: decode the destination, add,
        // re-encode. Bevy's own conversions, so this cannot drift from the
        // shader's — both are the piecewise IEC 61966-2-1 curve.
        let linear = |v: f32| LinearRgba::from(Srgba::new(v, v, v, 1.0)).red;
        let summed = linear(ground) + 3.0 * linear(layer);
        assert_eq!(byte(Srgba::from(LinearRgba::new(summed, 0.0, 0.0, 1.0)).red), 129);

        // And the direction is the one reported: the effect is fainter, never
        // brighter, so nothing about this change can blow a highlight out.
        for dst in [0.0f32, 0.25, 0.5, 0.75] {
            for src in [0.05f32, 0.1, 0.3] {
                let bytes = (dst + src).min(1.0);
                let mixed = Srgba::from(LinearRgba::new(
                    (linear(dst) + linear(src)).min(1.0),
                    0.0,
                    0.0,
                    1.0,
                ))
                .red;
                assert!(
                    mixed <= bytes + 1e-4,
                    "linear add {mixed} over-shot the byte add {bytes} at dst {dst} src {src}"
                );
            }
        }
    }

    /// The present pass does not see the world's layer. If it did, every
    /// gizmo would be drawn twice — once by the world camera and once, flat,
    /// by this one — and a falloff ring at Bevy `y = 0` would be a line across
    /// the window. See [`PRESENT_LAYER`].
    #[test]
    fn the_present_pass_is_off_the_worlds_layer() {
        let present = RenderLayers::layer(PRESENT_LAYER);
        assert!(!present.intersects(&RenderLayers::layer(0)));
        // …and it is not one of the layers the portraits, the paper doll and
        // the editor's stage use, which start at 1 and count up.
        assert!(PRESENT_LAYER > 16);
    }
}
