// The cloud layer: the generated cloud texture, with each ring of the dome's
// vertex alpha multiplied into the texel's, so the clouds fade out toward the
// horizon. See `render::clouds` and `vale_assets::tables::light::clouds`.
//
// Not lit and not fogged: the texture already holds the light's cloud colours.
// Written at the far plane; see `render::sky::behind_the_world`.

#import bevy_pbr::forward_io::VertexOutput
#import vale::gamma::to_frame

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var cloud_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var cloud_sampler: sampler;

struct CloudOutput {
    @location(0) colour: vec4<f32>,
    // Reversed depth: 0.0 is infinitely far.
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fragment(in: VertexOutput) -> CloudOutput {
    let texel = textureSample(cloud_texture, cloud_sampler, in.uv);
    var out: CloudOutput;
#ifdef VERTEX_COLORS
    let ring = in.color.a;
#else
    let ring = 1.0;
#endif
    out.colour = to_frame(vec4<f32>(texel.rgb, texel.a * ring));
    out.depth = 0.0;
    return out;
}
