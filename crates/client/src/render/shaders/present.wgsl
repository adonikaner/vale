// The world, onto the screen.
//
// One texture fetch and an opaque alpha, and the *absence* of anything else is
// the whole content of this file — so here is what already happened to the
// pixel by the time it arrives, because every stage of it is a place a transfer
// function could be applied twice.
//
// The world camera's main texture is `Rgba8Unorm` holding the game's own
// gamma-encoded bytes (`CompositingSpace::Srgb`, `atmosphere`'s `to_frame`),
// which is what makes the hardware blend add bytes to bytes the way 1.12's
// fixed-function ROP did — see `render::present`. At the end of that camera's
// graph Bevy's own upscaling node blits it to the camera's out texture and
// applies `SRGB_TO_LINEAR` on the way; the out texture is `Rgba8UnormSrgb`, so
// it re-encodes, and what this samples is the byte the world pass wrote.
// Sampling an sRGB texture decodes it, and the target this writes to encodes it
// again.
//
// So the transfer function is applied and undone twice before this shader and
// once after it, and the honest thing for this quad to do is **nothing**. An
// `srgb_to_linear` here — the obvious-looking thing, and what this file did
// first — is a third decode against two encodes: the world comes out at dusk in
// the middle of the afternoon, with no error anywhere.
//
// The alpha is forced to 1. What is in the frame's alpha channel after a
// world's worth of `SRCALPHA, ONE` is not a coverage of anything, and this quad
// is the picture rather than a layer over one.

// `bevy_sprite`, not `bevy_sprite_render`: the crate moved and the WGSL
// `#define_import_path` did not. An import naga_oil cannot resolve is not an
// error — the pipeline simply stays pending forever, so the quad never draws
// and the screen is black with nothing in the log.
#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var frame_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var frame_sampler: sampler;

// **The one thing this quad does other than nothing**, and it is off unless the
// deep night is on — see `render::night`, which owns the option, and
// `render::present::NightGrade`, which is this struct on the CPU.
struct NightGrade {
    night: f32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> grade: NightGrade;

// **These three run in linear, and that is not a contradiction of the note
// above.** What the file argues is that no *transfer function* belongs here: an
// `srgb_to_linear` would be a third decode against two encodes. These are not
// transfer functions, they are a display transform over a finished picture, and
// the value `textureSample` returns is already linear because the frame is an
// `Rgba8UnormSrgb`. A crush and a vignette taken in linear are a crush and a
// vignette; taken in bytes they would be neither, for the same reason the
// world's sums are the other way round.
//
// The reason they are here at all rather than in `atmosphere.wgsl` is
// arithmetic: a world fragment is shaded once per overlapping draw, and this
// quad is the only surface in the client that sees each pixel exactly once. A
// vignette applied per-surface would darken the corners once per layer of
// transparency standing in them.

// How much the midtones are pushed down at full night. An exponent above 1 in
// linear darkens everything below white and leaves white alone, which is what
// separates "night" from "the brightness slider is down": a lantern, a spell
// and the moon keep their value while the ground under them loses it.
const NIGHT_CRUSH: f32 = 0.35;

// How far the *darks* are cooled. Weighted by how dark the pixel already is, so
// that the bright things in the frame keep their own hue — the same scotopic
// argument `render::night`'s `COLD` is chosen under, applied to the picture
// rather than to the bands.
//
// **A tint and no longer a desaturation**, which is the whole of the fix for
// "everything looks washed out". This used to `mix` toward `vec3(grey) *
// NIGHT_COLD` — a pull toward a *colourless* blue — so at 0.45 the bottom half
// of every frame lost most of its chroma and the picture went grey. Multiplying
// by the tint instead cools the darks and keeps whatever colour was in them,
// which is what the game's own art has and what the report asked for.
const NIGHT_COOL: f32 = 0.35;
const NIGHT_COLD: vec3<f32> = vec3<f32>(0.55, 0.75, 1.00);

// …and the corners. Deliberately wide and shallow rather than a hard tunnel:
// what it is for is making the *middle* of the frame read as the lit part.
const NIGHT_VIGNETTE: f32 = 0.55;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var colour = textureSample(frame_texture, frame_sampler, in.uv).rgb;
    // Uniform, so the branch is coherent over the whole quad and daylight pays
    // one compare for the entire pass.
    if grade.night > 0.0 {
        let n = clamp(grade.night, 0.0, 1.0);
        colour = pow(max(colour, vec3<f32>(0.0)), vec3<f32>(1.0 + NIGHT_CRUSH * n));
        let grey = dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
        let dark = 1.0 - smoothstep(0.0, 0.25, grey);
        colour = mix(colour, colour * NIGHT_COLD, n * NIGHT_COOL * dark);
        let radius = length(in.uv - vec2<f32>(0.5));
        colour *= 1.0 - NIGHT_VIGNETTE * n * smoothstep(0.28, 0.72, radius);
    }
    return vec4<f32>(colour, 1.0);
}
