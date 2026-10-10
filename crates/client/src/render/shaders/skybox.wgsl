// A batch of a `LightSkybox` model: its texture times its colour track, faded
// by its transparency track and by the weight of the light that names the
// model. See `render::skybox`.
//
// Not lit and not fogged, like every sky surface: the skybox models' batches
// are all flagged unlit, and the sky is what the fog fades toward. Written at
// the far plane; see `render::sky::behind_the_world`.
//
// The blend mode decides how the alpha is used. Modes 0 and 1 (opaque and
// alpha key) take the light's weight as their alpha and ignore the texel's,
// apart from the alpha key's cut. Modes 2 and 4 multiply the two. Mode 3 adds
// with factors one and one, so its colour is scaled instead. Modes 5 and 6
// multiply the frame, so a part weight moves their colour toward the value
// that leaves the frame unchanged: 1 for mode 5, 0.5 for mode 6. These are
// applied after `to_frame`, in the bytes the blend operates on.

#import bevy_pbr::forward_io::VertexOutput
#import vale::gamma::to_frame

struct SkyboxParams {
    tint: vec4<f32>,
    mode: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: SkyboxParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var skybox_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var skybox_sampler: sampler;

struct SkyboxOutput {
    @location(0) colour: vec4<f32>,
    // Reversed depth: 0.0 is infinitely far.
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fragment(in: VertexOutput) -> SkyboxOutput {
    let texel = textureSample(skybox_texture, skybox_sampler, in.uv);
    let blend = u32(params.mode.x + 0.5);
    if blend == 1u && texel.a < params.mode.y {
        discard;
    }
    var alpha = params.tint.a;
    if blend >= 2u {
        alpha = alpha * texel.a;
    }
    var frame = to_frame(vec4<f32>(texel.rgb * params.tint.rgb, alpha));
    if blend == 3u {
        frame = vec4<f32>(frame.rgb * alpha, alpha);
    } else if blend == 5u {
        frame = vec4<f32>(mix(vec3<f32>(1.0), frame.rgb, alpha), alpha);
    } else if blend == 6u {
        frame = vec4<f32>(mix(vec3<f32>(0.5), frame.rgb, alpha), alpha);
    }
    var out: SkyboxOutput;
    out.colour = frame;
    out.depth = 0.0;
    return out;
}
