// The sun's disc and the two moons: one textured quad each, facing the camera.
//
// **The blend is alpha, not additive, and that is a measurement rather than a
// taste.** `vale sky` decodes each sprite and reports the mean colour of the
// texels its own alpha channel says are empty. `sunCenter.blp` reads
// 255/255/167 there, `moon.blp` 175/178/181 and `moon02.blp` 103/162/220 — all
// three carry full-strength art under a mask, so adding them would paint their
// whole 128x128 rectangle across the sky. The one sprite in the set that *is*
// additive is `sunGlare.blp`, whose clear texels mean 0/0/0, and it is not
// drawn (see `render::celestial`).
//
// Nothing here is lit or fogged, for the same reason nothing in `sky.wgsl` is:
// this is sky, and a sun multiplied by the daylight would be lighting the light.
// `tint` exists so the hour can fade a body in and out; it is white and opaque
// at every hour the client draws one.
//
// The depth is the far plane, like every other sky surface — see
// `render::sky::behind_the_world`. It is what lets the sprites sit twelve yards
// from the camera, which is where the 1.12.1 client puts them, without being
// inside the first tree.

#import bevy_pbr::forward_io::VertexOutput
#import vale::gamma::to_frame

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> tint: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var body_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var body_sampler: sampler;

struct CelestialOutput {
    @location(0) colour: vec4<f32>,
    // Reversed depth: 1.0 is the near plane and 0.0 is infinitely far.
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fragment(in: VertexOutput) -> CelestialOutput {
    let texel = textureSample(body_texture, body_sampler, in.uv);
    var out: CelestialOutput;
    // Encoded on the way out — see `to_frame`. These three are alpha-blended
    // over the dome behind them, so the space is the one the mask is resolved
    // in: the soft edge of a moon is a gradient of partial coverage, and the
    // client resolved it against bytes.
    out.colour = to_frame(vec4<f32>(texel.rgb * tint.rgb, texel.a * tint.a));
    out.depth = 0.0;
    return out;
}
