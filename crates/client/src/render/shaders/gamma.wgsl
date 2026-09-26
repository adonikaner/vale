#define_import_path vale::gamma

// **The transfer function, and nothing else.**
//
// This is the smallest module in the renderer and it exists for a mechanical
// reason rather than a tidy one: naga_oil does **not** strip an imported
// module's unused functions. `#import vale::atmosphere::srgb_to_linear`
// pulls in `fogged` and `daylight` with it, and those reference
// `bevy_pbr::mesh_view_bindings::{lights, fog, view}` — bindings a 2D pipeline
// does not have. The failure is `no definition in scope for identifier:
// bevy_pbr::mesh_view_bindings::fog` from a file that never asked for it.
//
// So the two conversions live where anything can import them: `atmosphere.wgsl`
// for the world's own lighting sums, `present.wgsl` for the blit that puts the
// finished frame on the screen. One copy of the curve, no bindings.
//
// **The piecewise IEC 61966-2-1 function, not a 2.2 power** — `Color::srgb` is
// what the CPU side calls and the two have to agree.

// One sRGB colour decoded to what a shader multiplies by.
//
// The model and terrain textures are uploaded `Rgba8UnormSrgb`, so
// `textureSample` already returns linear; what needs this is every colour that
// arrives as the *bytes a file states* — a `Light.dbc` band, a `MOCV` vertex,
// a particle's ramp — after it has been summed with the others in that space.
// See `atmosphere.wgsl`, which is the long form of why the sum comes first.
fn srgb_to_linear(colour: vec3<f32>) -> vec3<f32> {
    let low = colour / 12.92;
    let high = pow((colour + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, colour <= vec3<f32>(0.04045));
}

// And back, which two things need: a mix taken in the file's own space (the
// fog), and the frame itself (see [`to_frame`]).
fn linear_to_srgb(colour: vec3<f32>) -> vec3<f32> {
    let low = colour * 12.92;
    let high = 1.055 * pow(max(colour, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, colour <= vec3<f32>(0.0031308));
}

// **What a fragment hands to the framebuffer — the one sum in this renderer
// that is not taken inside a shader.**
//
// `atmosphere.wgsl` is about sums the *fragment* takes. The blend is a sum the
// *hardware* takes, between this fragment and what is already in the target,
// and it obeys the same rule for the same reason: 1.12 is fixed-function, its
// back buffer is a plain `X8R8G8B8` with no `D3DRS_SRGBWRITEENABLE` anywhere,
// so every `SRCALPHA, ONE` in the game adds bytes to bytes. A linear-storage
// target makes the hardware decode the destination, add, and re-encode — the
// same "each term of a sum decoded separately" error, taken by the ROP where no
// shader can see it.
//
// So the world is drawn into an `Rgba8Unorm` image holding the game's own bytes
// (see `render::present`), and every fragment that writes to it ends here. An
// opaque draw is unchanged to the bit: it used to be encoded by the target's
// sRGB view and is now encoded one instruction earlier. What changes is every
// *blended* draw, and the population that changes most is the one that stacks —
// `vale model` counts **1,137 of the game's 1,332 spell-effect batches at
// blend 4** (`SRCALPHA, ONE`).
//
// Measured on `Spells\ArcaneExplosion_Base.m2`, whose six batches are all blend
// 4 and whose three dome shells share one texture averaging 26/5/40: over ground
// at byte 120 the client reaches **198**, and the same three layers summed in
// linear reach **129**. That gap is the whole of "the effect is faint and hard
// to see" — nothing about the art, the alpha or the blend factors was wrong.
//
// The alpha is passed through untouched. It is a coverage fraction, not a
// colour, and the blend factors read it in whatever space it is written in.
fn to_frame(colour: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(linear_to_srgb(colour.rgb), colour.a);
}
