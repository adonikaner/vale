// `Shaders\Pixel\FFXGauss4.bls` along one axis, whole:
//
//     PARAM c[1] = { { 0.125, 0.375 } };
//     TEX R0, fragment.texcoord[1], texture[1], 2D;   MUL R1, R0, c[0].y;
//     TEX R0, fragment.texcoord[0], texture[0], 2D;   MAD R1, R0, c[0].x, R1;
//     TEX R0, fragment.texcoord[2], texture[2], 2D;   MAD R1, R0, c[0].y, R1;
//     TEX R0, fragment.texcoord[3], texture[3], 2D;   MAD result.color, R0, c[0].x, R1;
//
// The four coordinates are the pixel's centre offset by -2.5, -0.5, +0.5 and
// +2.5 texels along the axis (one set for each of the two draws). A bilinear fetch half a texel from a centre is the mean of two
// texels, so each tap is a pair and the kernel per axis is
// `1, 1, 3, 6, 3, 1, 1` over 16, across seven texels. The reference draws
// this twice through an intermediate target, horizontal then vertical;
// `render::glow` does the same with the axis below flipped.
//
// Bytes in, bytes out, for the reason `glow_box4.wgsl` gives.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var source: texture_2d<f32>;
// `(1, 0, 0, 0)` or `(0, 1, 0, 0)`.
@group(0) @binding(1) var<uniform> axis: vec4<f32>;

fn byte_at(p: vec2<i32>, limit: vec2<i32>) -> vec3<f32> {
    return textureLoad(source, clamp(p, vec2<i32>(0), limit), 0).rgb;
}

// One of the reference's taps: a bilinear fetch half a texel past `offset`
// texels from the centre, which is the mean of texels `offset` and
// `offset + 1`.
fn pair(p: vec2<i32>, step: vec2<i32>, offset: i32, limit: vec2<i32>) -> vec3<f32> {
    return 0.5 * (byte_at(p + step * offset, limit) + byte_at(p + step * (offset + 1), limit));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let limit = vec2<i32>(textureDimensions(source)) - vec2<i32>(1);
    let p = vec2<i32>(floor(in.position.xy));
    let step = vec2<i32>(axis.xy);
    let sum = 0.125 * pair(p, step, -3, limit)
        + 0.375 * pair(p, step, -1, limit)
        + 0.375 * pair(p, step, 0, limit)
        + 0.125 * pair(p, step, 2, limit);
    return vec4<f32>(sum, 1.0);
}
