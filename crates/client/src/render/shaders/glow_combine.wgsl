// `Shaders\Pixel\FFXGlow.bls`, the combine, whole:
//
//     TEX R1.xyz, fragment.texcoord[0], texture[0], 2D;   // the scene
//     TEX R0.xyz, fragment.texcoord[1], texture[1], 2D;   // the blur
//     ADD R2.xyz, R0, -R1;
//     MAD R1.xyz, fragment.color.primary.z, R2, R1;       // mix(scene, blur, z)
//     MUL R0.xyz, R0, R0;                                 // blur²
//     MAD result.color.xyz, R0, fragment.color.primary.w, R1;   // + w * blur²
//
// `z` is the drunk and underwater blur mix, 0 here — see `render::glow` for
// what it is and why it is not wired; `w` is the amount below. The scene is
// the world camera's byte-space main texture and the blur the quarter-size
// image the two passes before this one left, so every term is a byte, the
// square is of the encoded value and the add is a byte add — which is what
// makes a sunlit road lift and not only a lantern.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var blur: texture_2d<f32>;
struct Glow {
    amount: f32,
    // 1 for the death effect, `FFXDeath.bls`; see below.
    death: f32,
}

// The colour the death effect puts back into the mid-tones of the grey
// frame, as bytes: 83, 147, 168, the 1.12.1 client's own.
const DEATH_TINT: vec3<f32> = vec3<f32>(83.0, 147.0, 168.0) / 255.0;

// `Shaders\Pixel\FFXDeath.bls`, the combine while the player is a ghost:
//
//     MUL R0.xyz, R0, R0;                          // blur²
//     MAD R0.xyz, R0, fragment.color.primary.w, R1; // scene + w * blur²
//     DP3_SAT R0.x, R0, c[0].yzww;                 // grey: 0.299, 0.587, 0.144
//     ADD R0.y, -R0.x, c[0].x;
//     MUL R0.y, R0.x, R0;
//     MUL_SAT R0.y, R0, c[1].x;                    // 4 * grey * (1 - grey)
//     MAD result.color.xyz, fragment.color.primary, R0.y, R0.x;
//
// The tint is the quad's vertex colour and `w` its alpha. The blue weight is
// 0.144 in the shipped file, not the usual 0.114.
fn death(lifted: vec3<f32>) -> vec3<f32> {
    let grey = clamp(dot(lifted, vec3<f32>(0.299, 0.587, 0.144)), 0.0, 1.0);
    let midtone = clamp(4.0 * grey * (1.0 - grey), 0.0, 1.0);
    return DEATH_TINT * midtone + vec3<f32>(grey);
}
@group(0) @binding(2) var<uniform> glow: Glow;

fn blur_at(p: vec2<i32>, limit: vec2<i32>) -> vec3<f32> {
    return textureLoad(blur, clamp(p, vec2<i32>(0), limit), 0).rgb;
}

// The blur under this pixel, upsampled bilinearly in bytes — the reference's
// fixed-function fetch interpolated the bytes of its quarter-size target.
fn blur_bytes(uv: vec2<f32>) -> vec3<f32> {
    let dims = vec2<f32>(textureDimensions(blur));
    let limit = vec2<i32>(dims) - vec2<i32>(1);
    let c = uv * dims - vec2<f32>(0.5);
    let i = vec2<i32>(floor(c));
    let f = c - floor(c);
    let a = blur_at(i, limit);
    let b = blur_at(i + vec2<i32>(1, 0), limit);
    let d = blur_at(i + vec2<i32>(0, 1), limit);
    let e = blur_at(i + vec2<i32>(1, 1), limit);
    return mix(mix(a, b, f.x), mix(d, e, f.x), f.y);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(floor(in.position.xy));
    let source = textureLoad(scene, pixel, 0);
    let b = blur_bytes(in.uv);
    if glow.death > 0.5 {
        // The death program does not clamp before its grey, and its result is
        // written through the frame's own clamp.
        let lifted = source.rgb + glow.amount * b * b;
        return vec4<f32>(clamp(death(lifted), vec3<f32>(0.0), vec3<f32>(1.0)), source.a);
    }
    let lifted = clamp(source.rgb + glow.amount * b * b, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(lifted, source.a);
}
