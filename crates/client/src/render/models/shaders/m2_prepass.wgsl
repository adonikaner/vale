// The depth prepass's alpha test, and it is not optional.
//
// **Without this file, a depth prepass draws every leaf as a solid rectangle.**
// Bevy only runs a fragment shader in a depth-only prepass when the material
// declares one (`prepass/mod.rs`: `fragment_required` is `MAY_DISCARD &&
// PrepassFragmentShader.is_some()`), and the *default* prepass shader cannot do
// the test for a custom material — it has no idea where this one's texture is
// bound. So an `AlphaMode::Mask` batch with no prepass shader of its own writes
// depth for the whole quad it was cut out of.
//
// That is blend mode 1, which is most of the greenery in the game plus every
// window lattice and railing in every building. The symptom would not read as a
// depth bug: a tree would occlude whatever stood behind it in a rectangle around
// its canopy, and with occlusion culling on — which reads the same buffer —
// the things behind it would not merely be shaded wrong, they would be *culled*.
// Whole rooms disappearing behind a bush.
//
// The test is the same one `m2.wgsl` does, deliberately duplicated rather than
// shared: the two shaders take different `VertexOutput` structs (`prepass_io`
// against `forward_io`) and there is nothing left to factor out but the
// comparison itself. If one changes the other must, which is what this note is
// for — **and that now includes the `BINDLESS` binding layout**, which is the
// same two-world split as `m2.wgsl`'s, over the same index-table contract.
// `alpha_cutoff` is 0 for every mode but 1, so an opaque batch that somehow
// reached here discards nothing.
//
// **Depth only.** The client adds `DepthPrepass` and nothing else — no normal
// prepass, no motion vectors, no deferred — so this returns no targets at all.
// Adding either of the others means giving this function a `FragmentOutput`;
// Bevy would otherwise expect an output this does not write.

#import bevy_pbr::prepass_io::VertexOutput

#ifdef BINDLESS
#import bevy_render::bindless::{bindless_textures_2d, bindless_samplers_filtering}
#import bevy_pbr::mesh_bindings::mesh
#endif

// **Every field, although this shader reads one.** The struct is the element
// type of `material_array` in the bindless path, so its *size* is the stride the
// lookup indexes by: a copy that stops early does not read a wrong field, it
// reads a wrong material — silently, for every alpha-masked batch in the world.
// The three liquid fields below are dead weight here and are not optional.
struct M2Params {
    ambient: vec4<f32>,
    alpha_cutoff: f32,
    unlit: f32,
    vertex_lit: f32,
    liquid: f32,
    liquid_close: vec4<f32>,
    liquid_far: vec4<f32>,
    // The texture matrix, rows `(a, b, tx, active)` and `(c, d, ty, _)`. Read
    // here as well as in `m2.wgsl`, and not only for the stride: an alpha-keyed
    // batch whose texture scrolls cuts its silhouette out of a *moving* image,
    // so a prepass sampling the unmoved UV writes depth for the wrong holes.
    uv_row0: vec4<f32>,
    uv_row1: vec4<f32>,
    particle: vec4<f32>,
    // `.x`: how solid this batch is, multiplied into the alpha **after** the
    // alpha test — 1.0 on everything in the world, and less on a body the
    // creep or ghost bit has made see-through. After the test because the
    // cutoff belongs to the blend mode, so scaling first eats holes in a
    // cut-out. `.y`: what the blend mode was before it was forced to 2 to be
    // blendable at all, **plus one**, so that zero can mean "nothing was
    // forced" — mode 0 is `Opaque`, which is most of a body. Read by nobody
    // here; it is CPU-side bookkeeping that happens to live in the uniform.
    // `.zw` unused. See `models::material::M2Params::body`.
    body: vec4<f32>,
    // **The lighting the *model* states for itself** — `M2Light`, which nothing
    // in the world carries and which is the whole shading of the two screens
    // before it. `scene_ambient.rgb` is the fill and `.w` is how many of the
    // lamps below are real; each lamp is two vectors,
    // `(position.xyz, attenuation_start)` and `(colour.rgb, attenuation_end)`.
    // Zero everywhere else, which costs the fragment one compare. See
    // `models::material::SceneLighting`.
    scene_ambient: vec4<f32>,
    scene_lamps: array<vec4<f32>, 8>,
};

/// This batch's UV, with its texture matrix applied — or unchanged, which is
/// what `active == 0` means and what all but two dozen batches get.
fn transformed_uv(params: M2Params, uv: vec2<f32>) -> vec2<f32> {
    let moved = vec2<f32>(
        dot(params.uv_row0.xy, uv) + params.uv_row0.z,
        dot(params.uv_row1.xy, uv) + params.uv_row1.z,
    );
    return select(uv, moved, params.uv_row0.w > 0.5);
}

#ifdef BINDLESS
// See m2.wgsl: field order is binding order, and that is the contract.
struct M2MaterialBindings {
    material: u32,
    texture: u32,
    texture_sampler: u32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<storage> material_indices: array<M2MaterialBindings>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var<storage> material_array: array<M2Params>;
#else   // BINDLESS
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: M2Params;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;
#endif  // BINDLESS

@fragment
fn fragment(in: VertexOutput) {
#ifdef BINDLESS
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    let params = material_array[material_indices[slot].material];
    let cutoff = params.alpha_cutoff;
    let texel = textureSample(
        bindless_textures_2d[material_indices[slot].texture],
        bindless_samplers_filtering[material_indices[slot].texture_sampler],
        transformed_uv(params, in.uv));
#else   // BINDLESS
    let cutoff = material.alpha_cutoff;
    let texel = textureSample(base_texture, base_sampler, transformed_uv(material, in.uv));
#endif  // BINDLESS
    if texel.a < cutoff {
        discard;
    }
}
