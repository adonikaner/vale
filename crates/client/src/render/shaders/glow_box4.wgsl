// `Shaders\Pixel\FFXBox4.bls`: the scene, downsampled by four.
//
// The reference's program is four bilinear taps at (-1.5, -1.5), (+0.5, -1.5),
// (-1.5, +0.5) and (+0.5, +0.5) source texels from the destination pixel's
// centre, averaged — and with its quad's half-texel offset each tap lands on a
// texel corner, so the four are the mean of the 4x4 block. See
// `render::glow`, which quotes the offsets and where they were read.
//
// The source is the world camera's main texture, a raw `Rgba8Unorm` holding
// the game's own bytes, so every `textureLoad` here is a byte and the mean is
// a byte mean — which is what the reference's fixed-function taps averaged.
// No sampler: the sixteen loads are the four taps written out.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var source: texture_2d<f32>;

fn byte_at(p: vec2<i32>, limit: vec2<i32>) -> vec3<f32> {
    return textureLoad(source, clamp(p, vec2<i32>(0), limit), 0).rgb;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let limit = vec2<i32>(textureDimensions(source)) - vec2<i32>(1);
    // `position` is the target pixel, centred at +0.5; its block is the four
    // source texels from four times its index.
    let origin = vec2<i32>(floor(in.position.xy)) * 4;
    var sum = vec3<f32>(0.0);
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            sum += byte_at(origin + vec2<i32>(x, y), limit);
        }
    }
    return vec4<f32>(sum * 0.0625, 1.0);
}
