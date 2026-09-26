// The sway on the ground's own foliage — and **the one thing in this renderer
// that 1.12 does not do at all.**
//
// That is measured rather than assumed, and it was worth measuring because the
// answer decides whether this file is a reconstruction or an invention:
//
//   * `Shaders\Vertex\Model2.bls` — the game's own model vertex program, and
//     the only one a doodad is ever drawn through — carries **1,080 distinct
//     permutations** and its whole opcode set is
//     ADD ARL DP3 DP4 FRC MAD MAX MIN MOV MUL RCP RSQ SLT. No trigonometry, no
//     time uniform, and `result.position` is the skinned position through the
//     view-projection and nothing else, in every one of them.
//   * and the detail models themselves are static: no skeleton, no texture
//     matrices, and the one transparency track any of them carries is the
//     identity (`vale model 'World\NoDXT\Detail\ElwGra01.m2'`).
//
// So 5875's grass stands still. This is an invention, it is switchable with the
// rest of the foliage layer, and it is written here rather than folded into the
// M2 vertex stage so that the deviation is one file with its own name on it.
//
// ## What it costs
//
// One `sin`, two multiply-adds, per foliage vertex — and **only** on foliage:
// the caller is inside `#ifdef WIND`, which is a shader def set from
// `M2MaterialKey::wind`, so every other material in the game compiles the
// vertex stage it compiled before this existed. Nothing is read per frame from
// the CPU; the two per-vertex inputs are baked into `uv_b` once, when the
// chunk's lawn is merged.
//
// ## The two baked inputs
//
// `uv_b.x` is the **sway weight**: 0 at the tuft's base and 1 at its tip, so
// the base stays planted in the ground and the tip travels. It is baked as the
// square of the vertex's height up the model, which is the cheapest curve that
// looks like a stalk bending rather than a card shearing.
//
// `uv_b.y` is the **phase**, and it carries the whole of what makes this read
// as wind rather than as jitter: it is `dot(world_xz, WIND_DIRECTION) * WAVE`
// plus a per-tuft hash. The first term makes the sway a *wave travelling across
// the field* — neighbouring tufts lean together — and the second keeps it from
// looking like one rigid sheet.

#define_import_path vale::wind

// ## The clock, which is bound at **two different slots**
//
// This costs a `#ifdef` and it is not optional. The forward pass binds the view
// group from `bevy_pbr::mesh_view_bindings`, where `globals` is
// `@group(0) @binding(11)`. The **prepass** binds a group of its own —
// `prepass/mod.rs` builds it as `(0, view), (1, globals), (2, previous_view)` —
// so the same name is at binding 1 there, and nothing in
// `bevy_pbr::prepass_bindings` declares it because no upstream prepass shader
// reads it.
//
// Importing the forward one into both is what the first attempt did, and it
// fails at *run time* rather than at compile time, with the whole app quitting
// on `Shader global ResourceBinding { group: 0, binding: 11 } is not available
// in the pipeline layout` — and only under `F6`, which is off by default.
#ifdef PREPASS_PIPELINE
#import bevy_render::globals::Globals
@group(0) @binding(1) var<uniform> prepass_clock: Globals;
fn wind_time() -> f32 {
    return prepass_clock.time;
}
#else
#import bevy_pbr::mesh_view_bindings::globals
fn wind_time() -> f32 {
    return globals.time;
}
#endif

/// How far a tip travels from rest, in yards.
///
/// Small on purpose. A tuft is 0.4 to 1.6 yards tall, so 6 cm at the tip is a
/// visible lean of a few degrees — which is what a breeze looks like. Grass
/// that swings far enough to notice individually reads as underwater.
const WIND_AMPLITUDE: f32 = 0.06;

/// Radians per second. Two full cycles about every five seconds.
const WIND_RATE: f32 = 1.3;

/// The direction the wave travels and the tips lean along, in Bevy's ground
/// plane (`xz`), normalised.
const WIND_DIRECTION: vec2<f32> = vec2<f32>(0.7071, 0.7071);

/// The world-space offset a foliage vertex sways by.
///
/// Returned rather than applied so that the two vertex stages that call it —
/// the forward one and the prepass one — displace **identically**. They have to:
/// the prepass writes the depth the forward pass is then tested against, and a
/// sway in one and not the other punches holes in the grass wherever a swayed
/// fragment lands behind its own unswayed depth.
fn sway(uv_b: vec2<f32>) -> vec3<f32> {
    let lean = sin(wind_time() * WIND_RATE + uv_b.y) * uv_b.x * WIND_AMPLITUDE;
    return vec3<f32>(WIND_DIRECTION.x * lean, 0.0, WIND_DIRECTION.y * lean);
}
