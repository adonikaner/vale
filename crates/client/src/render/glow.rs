//! **The full-screen glow** — `ffxGlow`, the one post-process 1.12 applies to
//! every frame, as the client and the four `Shaders\Pixel\FFX*.bls` it
//! names define it.
//!
//! This is the largest single difference between a screenshot of this client
//! and one of the reference standing in the same place: the reference is
//! brighter and warmer *everywhere*, on the ground and the trees as much as on
//! the lantern, and none of the lighting sums in `atmosphere.wgsl` can produce
//! that because all of them stop at the surface. The glow is applied to the
//! finished picture. It is on by default (`ffxGlow` registers `"1"`)
//! and the options panel's *Full Screen Glow Effect* is the only
//! thing that turns it off, so a player of 5875 has been looking through it
//! since 2004.
//!
//! ## What the reference computes
//!
//! Three passes over the frame, in the client's own order, one pass object
//! each:
//!
//! 1. **`FFXBox4.bls`**: the scene, downsampled by four. Four bilinear taps at
//!    `(-1.5, -1.5)`, `(+0.5, -1.5)`, `(-1.5, +0.5)`, `(+0.5, +0.5)` source
//!    texels, averaged (`MUL result, R0, 0.25`) — with
//!    the quad's half-texel offset each tap lands on a texel *corner*, so the
//!    four together are the mean of the 4x4 block.
//! 2. **`FFXGauss4.bls`**, twice, horizontal then vertical (two
//!    draws through an intermediate target): four taps at `-2.5`, `-0.5`,
//!    `+0.5`, `+2.5` texels of the *low-resolution* image, weighted
//!    `0.125, 0.375, 0.375, 0.125`. Each tap is a bilinear fetch half a texel
//!    from a centre, so it is the mean of two texels; per axis that is the
//!    seven-texel kernel `1, 1, 3, 6, 3, 1, 1` over 16.
//! 3. **`FFXGlow.bls`**, the combine, whole:
//!
//!    ```text
//!    TEX R1.xyz, fragment.texcoord[0], texture[0], 2D;   // the scene
//!    TEX R0.xyz, fragment.texcoord[1], texture[1], 2D;   // the blur
//!    ADD R2.xyz, R0, -R1;
//!    MAD R1.xyz, fragment.color.primary.z, R2, R1;       // mix(scene, blur, z)
//!    MUL R0.xyz, R0, R0;                                 // blur²
//!    MAD result.color.xyz, R0, fragment.color.primary.w, R1;   // + w · blur²
//!    ```
//!
//!    `z` and `w` are the quad's vertex colour, packed
//!    as `A = w, R = G = B = z`. **`w` is a config float**,
//!    `0.3` at start-up and then, when
//!    the world is entered, **`0.4`** for the plain
//!    glow or `0.15` for the `FFXGlowWave` variant. **`z` is the drunk
//!    level**: `PLAYER_BYTES_3` byte 1 (byte 1 of field 195), clamped to 100 and
//!    scaled by 2.55 — and at least `0x54/255` while the camera
//!    is under a liquid (any liquid type other than 15).
//!    Sober and on land it is 0, so the frame is `scene + 0.4 · blur²`.
//!
//! Every term is in the frame's own bytes: the back buffer is `X8R8G8B8` and
//! nothing in the chain decodes it, so the square is a square of the encoded
//! value and the add is a byte add. That is why the effect brightens a whole
//! sunlit road and not only the lantern: at byte 160 the road's own blur
//! squared is 100, and 0.4 of that is a lift of forty bytes. It is also why it
//! reads *warm* — the square of a warm colour is warmer than the colour.
//!
//! ## What this does, and where
//!
//! **A pass in the world camera's own post-process set**, on its main
//! texture. That texture is a raw `Rgba8Unorm` holding the game's bytes
//! (`CompositingSpace::Srgb` — see `render::present`), so every `textureLoad`
//! here *is* a byte and every write is one: no encode, no decode, and no
//! sampler anywhere — the reference's bilinear taps are written out as the
//! byte means they are. The two intermediate images are pooled quarter-size
//! `Rgba8Unorm` textures from the render world's `TextureCache`,
//! ping-ponged: box into A, horizontal into B, vertical back into A, and the
//! combine reads the main texture and A and writes the other main texture
//! through `post_process_write`.
//!
//! **It used to be three 2D cameras and three quads**, and that cost about a
//! millisecond of CPU a frame for nothing the picture needed: each camera is
//! a view, and a view is an extraction, a visibility sweep over every 2D mesh
//! the interface painter owns, a queue, and a render-graph run of its own. A
//! pass is four render-pass encodes and four bind groups, and it runs only
//! for the one view that carries [`Glow`].
//!
//! **Two things are readings rather than copies, and are stated as such.**
//! The reference's quad builder offsets its texture coordinates
//! by half a source texel on top of D3D9's half-pixel position rule, which
//! shifts every pass's kernel by half a texel down and to the right; this
//! keeps the kernels centred. And the drunk and underwater blur mix `z` is
//! not applied — this client has neither the drunk state nor the camera's
//! liquid type wired to a post-process, so `z` is 0 and only the additive
//! term is drawn.
//!
//! ## The two switches
//!
//! `ffxGlow` off in `Config.wtf` turns it off, as the reference's options
//! panel does, and `glow` on the world tab (`--without glow`) is the
//! subtraction for pricing it. Either one zeroes [`Glow::amount`], and the
//! pass returns before it encodes anything, so a frame with the glow off costs
//! exactly what it did before this file existed.

use bevy::asset::embedded_asset;
use bevy::core_pipeline::schedule::Core3d;
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{texture_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, Extent3d, FragmentState, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, ShaderStages,
    ShaderType, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType,
    TextureUsages, UniformBuffer,
};
use bevy::render::renderer::{CurrentView, RenderContext, RenderDevice, RenderQueue};
use bevy::render::texture::TextureCache;
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use super::tuning::WorldTuning;

/// The CVar the reference's options panel toggles — *Full Screen Glow
/// Effect* — registered `"1"`.
const FFX_GLOW: &str = "ffxGlow";

/// What the reference multiplies the squared blur by while the world is up.
/// The `0.15` written beside it belongs to the wave variant this client
/// does not draw.
pub const AMOUNT: f32 = 0.4;

/// …and before it is: the `0.3` the config starts with, which is what the login and character screens are
/// drawn through until the world is entered.
pub const GLUE_AMOUNT: f32 = 0.3;

/// The downsample the box pass makes, per axis.
pub const DOWNSAMPLE: u32 = 4;

/// **The format the pass is written for**: the world camera's main texture
/// under `CompositingSpace::Srgb`, whose bytes are the game's own. The three
/// pipelines target it, and a view whose main texture is anything else is
/// skipped rather than drawn wrong — see [`glow_pass`].
const FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

/// **What the combine multiplies the squared blur by** — `w` above. On the
/// world camera, extracted to the render world every frame, and the one
/// number both switches write. Zero is the pass's early-out.
#[derive(Component, ExtractComponent, Clone, Copy, Default, ShaderType)]
pub struct Glow {
    pub amount: f32,
    /// 1 while the player is a ghost and `ffxDeath` is on: the combine is
    /// `FFXDeath.bls` instead of `FFXGlow.bls`. See `glow_combine.wgsl`.
    pub death: f32,
}

/// The CVar for the full-screen death effect, registered `"1"`.
const FFX_DEATH: &str = "ffxDeath";

pub struct GlowPlugin;

impl Plugin for GlowPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/glow_box4.wgsl");
        embedded_asset!(app, "shaders/glow_gauss4.wgsl");
        embedded_asset!(app, "shaders/glow_combine.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<Glow>::default(),
            UniformComponentPlugin::<Glow>::default(),
        ))
        // After the world camera exists, which `world::camera::spawn` does in
        // `Startup`.
        .add_systems(PostStartup, attach)
        .add_systems(Update, switch);

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, write_axes.in_set(RenderSystems::PrepareResources))
            .add_systems(
                Core3d,
                glow_pass.in_set(Core3dSystems::PostProcess).before(tonemapping),
            );
    }
}

/// Put [`Glow`] on the world camera, at zero until [`switch`] decides.
fn attach(mut commands: Commands, camera: Query<Entity, With<crate::world::camera::WorldCamera>>) {
    for entity in &camera {
        commands.entity(entity).insert(Glow::default());
    }
}

/// The two switches and the screen, folded into one number: [`AMOUNT`] in the
/// world and [`GLUE_AMOUNT`] before it with both switches on, zero otherwise.
/// Written only when it moves, since a changed component is re-extracted and
/// its uniform re-uploaded.
///
/// While the player is a ghost the 1.12.1 client swaps the glow for the death
/// effect, which is on with `ffxDeath` whatever `ffxGlow` says. Its amount is
/// the `LightParams` glow of the light in force, which for a ghost is the
/// death column's (0.8 on map 0).
fn switch(
    tuning: Res<WorldTuning>,
    cvars: Res<crate::settings::cvars::CVars>,
    session: Res<crate::world::session::Session>,
    dying: Option<Res<crate::interface::death::Dying>>,
    sky: Option<Res<crate::render::sky::Sky>>,
    mut glow: Query<&mut Glow>,
) {
    let in_world = matches!(session.screen(), crate::world::session::Screen::InWorld);
    let ghost = in_world && dying.is_some_and(|d| d.ghost);
    let wanted = if ghost {
        let on = tuning.glow && cvars.flag(FFX_DEATH);
        Glow {
            amount: if on { sky.map_or(0.0, |s| s.current.glow) } else { 0.0 },
            death: if on { 1.0 } else { 0.0 },
        }
    } else {
        Glow {
            amount: amount_for(tuning.glow && cvars.flag(FFX_GLOW), in_world),
            death: 0.0,
        }
    };
    for mut glow in &mut glow {
        if glow.amount != wanted.amount || glow.death != wanted.death {
            info!(
                "glow: amount {} death {} (switch {}, {FFX_GLOW} {:?}, {FFX_DEATH} {:?})",
                wanted.amount,
                wanted.death,
                tuning.glow,
                cvars.get(FFX_GLOW),
                cvars.get(FFX_DEATH)
            );
            *glow = wanted;
        }
    }
}

/// What the combine multiplies the squared blur by: nothing with either
/// switch off, the world's amount in the world, the start-up amount before it.
fn amount_for(on: bool, in_world: bool) -> f32 {
    match (on, in_world) {
        (false, _) => 0.0,
        (true, true) => AMOUNT,
        (true, false) => GLUE_AMOUNT,
    }
}

/// The three pipelines, their layouts, and the two axis uniforms the Gaussian
/// takes — one horizontal, one vertical — written once.
#[derive(Resource)]
struct GlowPipelines {
    box_layout: BindGroupLayoutDescriptor,
    gauss_layout: BindGroupLayoutDescriptor,
    combine_layout: BindGroupLayoutDescriptor,
    box_pipeline: CachedRenderPipelineId,
    gauss_pipeline: CachedRenderPipelineId,
    combine_pipeline: CachedRenderPipelineId,
    horizontal: UniformBuffer<Vec4>,
    vertical: UniformBuffer<Vec4>,
    axes_written: bool,
}

fn init_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    asset_server: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
) {
    let source = texture_2d(TextureSampleType::Float { filterable: false });
    let box_layout = BindGroupLayoutDescriptor::new(
        "glow box layout",
        &BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, (source,)),
    );
    let gauss_layout = BindGroupLayoutDescriptor::new(
        "glow gauss layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (source, uniform_buffer::<Vec4>(false)),
        ),
    );
    let combine_layout = BindGroupLayoutDescriptor::new(
        "glow combine layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (source, source, uniform_buffer::<Glow>(true)),
        ),
    );
    let pipeline = |label: &'static str, layout: &BindGroupLayoutDescriptor, shader: &'static str| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: asset_server.load(shader),
                targets: vec![Some(ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        })
    };
    let box_pipeline = pipeline("glow box", &box_layout, super::shader::GLOW_BOX4);
    let gauss_pipeline = pipeline("glow gauss", &gauss_layout, super::shader::GLOW_GAUSS4);
    let combine_pipeline = pipeline("glow combine", &combine_layout, super::shader::GLOW_COMBINE);
    commands.insert_resource(GlowPipelines {
        box_layout,
        gauss_layout,
        combine_layout,
        box_pipeline,
        gauss_pipeline,
        combine_pipeline,
        horizontal: UniformBuffer::from(Vec4::new(1.0, 0.0, 0.0, 0.0)),
        vertical: UniformBuffer::from(Vec4::new(0.0, 1.0, 0.0, 0.0)),
        axes_written: false,
    });
}

/// Upload the two axis uniforms, once.
fn write_axes(
    mut pipelines: ResMut<GlowPipelines>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    if pipelines.axes_written {
        return;
    }
    pipelines.horizontal.write_buffer(&device, &queue);
    pipelines.vertical.write_buffer(&device, &queue);
    pipelines.axes_written = true;
}

/// A quarter of `size` on each axis, rounded up so the last partial block is
/// still covered, and never zero.
fn quarter_of(size: UVec2) -> UVec2 {
    ((size + DOWNSAMPLE - 1) / DOWNSAMPLE).max(UVec2::ONE)
}

/// The three passes and the combine, for the view being rendered — when it
/// carries a non-zero [`Glow`] and its main texture is the byte-space format
/// the shaders are written for.
#[allow(clippy::too_many_arguments)]
fn glow_pass(
    current: Res<CurrentView>,
    views: Query<(&ViewTarget, &Glow, &DynamicUniformIndex<Glow>)>,
    pipelines: Res<GlowPipelines>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<Glow>>,
    mut textures: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let Ok((target, glow, index)) = views.get(current.0) else { return };
    if glow.amount <= 0.0 && glow.death <= 0.0 {
        return;
    }
    if target.main_texture_format() != FORMAT {
        // A view that is not in the game's own bytes: the arithmetic would be
        // taken in the wrong space, and the pipelines' target would not match.
        // The world camera declares the format; nothing else carries `Glow`.
        return;
    }
    let (Some(box_pipeline), Some(gauss_pipeline), Some(combine_pipeline)) = (
        pipeline_cache.get_render_pipeline(pipelines.box_pipeline),
        pipeline_cache.get_render_pipeline(pipelines.gauss_pipeline),
        pipeline_cache.get_render_pipeline(pipelines.combine_pipeline),
    ) else {
        // Still compiling: the frame ships without the glow rather than
        // waiting, which is a frame or two at start-up.
        return;
    };
    let Some(amount) = uniforms.uniforms().binding() else { return };
    let (Some(horizontal), Some(vertical)) =
        (pipelines.horizontal.binding(), pipelines.vertical.binding())
    else {
        return;
    };

    // The two pooled quarter-size textures, ping-ponged.
    let full = target.main_texture().size();
    let quarter = quarter_of(UVec2::new(full.width, full.height));
    let descriptor = TextureDescriptor {
        label: Some("glow quarter"),
        size: Extent3d {
            width: quarter.x,
            height: quarter.y,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };
    let a = textures.get(&device, descriptor.clone());
    let b = textures.get(&device, descriptor);

    let post = target.post_process_write();
    let box_group = device.create_bind_group(
        "glow box",
        &pipeline_cache.get_bind_group_layout(&pipelines.box_layout),
        &BindGroupEntries::sequential((post.source,)),
    );
    let gauss_h = device.create_bind_group(
        "glow gauss horizontal",
        &pipeline_cache.get_bind_group_layout(&pipelines.gauss_layout),
        &BindGroupEntries::sequential((&a.default_view, horizontal)),
    );
    let gauss_v = device.create_bind_group(
        "glow gauss vertical",
        &pipeline_cache.get_bind_group_layout(&pipelines.gauss_layout),
        &BindGroupEntries::sequential((&b.default_view, vertical)),
    );
    let combine_group = device.create_bind_group(
        "glow combine",
        &pipeline_cache.get_bind_group_layout(&pipelines.combine_layout),
        &BindGroupEntries::sequential((post.source, &a.default_view, amount)),
    );

    let offsets = [index.index()];
    let passes: [(&str, _, _, _, &[u32]); 4] = [
        ("glow box", box_pipeline, &box_group, &a.default_view, &[]),
        ("glow gauss horizontal", gauss_pipeline, &gauss_h, &b.default_view, &[]),
        ("glow gauss vertical", gauss_pipeline, &gauss_v, &a.default_view, &[]),
        ("glow combine", combine_pipeline, &combine_group, post.destination, &offsets),
    ];
    for (label, pipeline, group, view, offsets) in passes {
        let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, offsets);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quarter covers the last partial block and is never empty.
    #[test]
    fn a_quarter_rounds_up_and_never_reaches_zero() {
        assert_eq!(quarter_of(UVec2::new(1600, 900)), UVec2::new(400, 225));
        assert_eq!(quarter_of(UVec2::new(1601, 901)), UVec2::new(401, 226));
        assert_eq!(quarter_of(UVec2::new(1, 1)), UVec2::new(1, 1));
        assert_eq!(quarter_of(UVec2::ZERO), UVec2::new(1, 1));
    }

    /// **The combine's arithmetic, in bytes**, so the number in the module doc
    /// is checkable: a road at byte 160 whose blur is its own value is lifted
    /// by forty bytes, and a black pixel beside nothing bright is not lifted
    /// at all.
    #[test]
    fn the_glow_lifts_a_lit_road_by_forty_bytes_and_black_by_nothing() {
        let lift = |scene: f32, blur: f32| ((scene + AMOUNT * blur * blur).min(1.0) * 255.0).round();
        assert_eq!(lift(160.0 / 255.0, 160.0 / 255.0), 200.0);
        assert_eq!(lift(0.0, 0.0), 0.0);
        // …and a warm colour gets warmer: the square favours the larger channel.
        let warm = [200.0 / 255.0, 120.0 / 255.0, 60.0 / 255.0];
        let lifted: Vec<f32> = warm.iter().map(|&c| lift(c, c) - (c * 255.0).round()).collect();
        assert!(lifted[0] > lifted[1] && lifted[1] > lifted[2], "{lifted:?}");
    }

    /// Either switch off is zero, and the two numbers are the reference's:
    /// the config's start-up value at the login screen and the value written
    /// on entering the world.
    #[test]
    fn either_switch_off_is_zero_and_the_world_is_brighter_than_the_login() {
        assert_eq!(amount_for(true, true), 0.4);
        assert_eq!(amount_for(true, false), 0.3);
        assert_eq!(amount_for(false, true), 0.0);
        assert_eq!(amount_for(false, false), 0.0);
    }

    /// The pass is written for the world camera's byte-space main texture and
    /// nothing else — the same format `render::present` argues for.
    #[test]
    fn the_pass_targets_the_games_own_bytes() {
        assert_eq!(FORMAT, TextureFormat::Rgba8Unorm);
    }
}
