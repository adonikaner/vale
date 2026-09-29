// The alpha test for the depth-only passes.
//
// Without this file, a depth prepass draws every leaf as a solid rectangle.
// Bevy only runs a fragment shader in a depth-only prepass when the material
// declares one (`prepass/mod.rs`: `fragment_required` is `MAY_DISCARD &&
// PrepassFragmentShader.is_some()`), and the default prepass shader cannot do
// the test for a custom material, because it does not know where this one's
// texture is bound. So an `AlphaMode::Mask` batch with no prepass shader of
// its own writes depth for the whole quad it was cut out of.
//
// That is blend mode 1, which is most of the greenery in the game plus every
// window lattice and railing in every building. A tree would occlude whatever
// stood behind it in a rectangle around its canopy, and with occlusion culling
// on, which reads the same buffer, the things behind it would be culled
// rather than only shaded wrong: whole rooms would disappear behind a bush.
//
// The test is the same one `m2.wgsl` does, duplicated rather than shared: the
// two shaders take different `VertexOutput` structs (`prepass_io` against
// `forward_io`) and there is nothing left to factor out but the comparison
// itself. A change to one must be made to the other. That includes the
// `BINDLESS` binding layout, which is the same two-layout split as
// `m2.wgsl`'s over the same index-table contract, and the texture-matrix
// table read. `alpha_cutoff` is 0 for every mode but 1, so an opaque batch
// that reaches here discards nothing.
//
// Depth only. The client adds `DepthPrepass` and nothing else (no normal
// prepass, no motion vectors, no deferred), so this returns no targets at all.
// Adding either of the others means giving this function a `FragmentOutput`;
// Bevy would otherwise expect an output this does not write.

#import bevy_pbr::prepass_io::VertexOutput

#ifdef BINDLESS
#import bevy_render::bindless::{bindless_textures_2d, bindless_samplers_filtering}
#import bevy_pbr::mesh_bindings::mesh
#endif

// This struct must list every field of `M2Params` in `m2.wgsl`, although this
// shader reads only three of them. The struct is the element type of `material_array` in the
// bindless path, so its size is the stride the lookup indexes by: a copy that
// stops early does not read a wrong field, it reads a wrong material, with no
// error, for every alpha-masked batch in the world. The liquid fields below
// are unused here and still required.
struct M2Params {
    ambient: vec4<f32>,
    alpha_cutoff: f32,
    unlit: f32,
    vertex_lit: f32,
    liquid: f32,
    liquid_close: vec4<f32>,
    liquid_far: vec4<f32>,
    // The texture matrix, rows `(a, b, tx, source)` and `(c, d, ty, _)`, with
    // `source` 0 for no matrix, 1 for a matrix in these rows and 2 + n for row
    // n of the texture-matrix table; see `m2.wgsl`. Read here as well as in
    // `m2.wgsl`: an alpha-keyed batch whose texture scrolls cuts its silhouette
    // out of a moving image, so a prepass sampling the unmoved UV writes depth
    // for the wrong holes.
    uv_row0: vec4<f32>,
    uv_row1: vec4<f32>,
    particle: vec4<f32>,
    // `.x`: how solid this batch is, multiplied into the alpha after the
    // alpha test: 1.0 on everything in the world, and less on a body the
    // creep or ghost bit has made see-through. After the test because the
    // cutoff belongs to the blend mode, so scaling first cuts holes in a
    // cut-out. `.y`: what the blend mode was before it was forced to 2 to be
    // blendable at all, plus one, so that zero can mean "nothing was
    // forced"; mode 0 is `Opaque`, which is most of a body. Not read by this
    // shader; it is CPU-side bookkeeping stored in the material.
    // `.zw` unused. See `models::material::M2Params::body`.
    body: vec4<f32>,
    // The two folded layers' blend modes; not read by this shader. See
    // `models::material::M2Material::overlay_a`.
    overlay: vec4<f32>,
    // The lighting the model states for itself: `M2Light`, which nothing in
    // the world carries and which is the whole shading of the two screens
    // before it. `scene_ambient.rgb` is the fill and `.w` is how many of the
    // lamps below are real; each lamp is two vectors, `(place.xyz, is_point)`
    // and `(colour.rgb, 0)`. Zero everywhere else. See
    // `models::material::SceneLighting`.
    scene_ambient: vec4<f32>,
    scene_lamps: array<vec4<f32>, 8>,
};

// This batch's UV with its texture matrix applied, or unchanged when
// `uv_row0.w` is 0, which is all but two dozen batches. Called after any table
// row has been copied into `uv_row0`/`uv_row1`, so `w` is then 0 or 1.
fn transformed_uv(params: M2Params, uv: vec2<f32>) -> vec2<f32> {
    let moved = vec2<f32>(
        dot(params.uv_row0.xy, uv) + params.uv_row0.z,
        dot(params.uv_row1.xy, uv) + params.uv_row1.z,
    );
    return select(uv, moved, params.uv_row0.w > 0.5);
}

#ifdef BINDLESS
// The same struct as m2.wgsl's, every field included. Field order is binding
// order, and the struct's size is the stride `material_indices` is indexed by:
// bevy writes one entry per bindless index for each material, so a struct that
// stops early reads the wrong material's indices for every slot past the first.
struct M2MaterialBindings {
    material: u32,
    texture: u32,
    texture_sampler: u32,
    overlay_a: u32,
    overlay_a_sampler: u32,
    overlay_b: u32,
    overlay_b_sampler: u32,
    uv_table: u32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<storage> material_indices: array<M2MaterialBindings>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var<storage> material_array: array<M2Params>;
#else   // BINDLESS
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: M2Params;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var uv_table_texture: texture_2d<f32>;
#endif  // BINDLESS

// The texture-matrix table read, the same as m2.wgsl's: one `f32` from the
// four bytes of one `Rgba8Unorm` texel, least significant byte in red.
fn uv_table_float(texel: vec4<f32>) -> f32 {
    let b = vec4<u32>(round(texel * 255.0));
    return bitcast<f32>(b.x | (b.y << 8u) | (b.z << 16u) | (b.w << 24u));
}

// `params` with its texture matrix taken from the six texels of its table row.
fn with_table_matrix(
    params: M2Params,
    e0: vec4<f32>, e1: vec4<f32>, e2: vec4<f32>,
    e3: vec4<f32>, e4: vec4<f32>, e5: vec4<f32>,
) -> M2Params {
    var out = params;
    out.uv_row0 = vec4<f32>(uv_table_float(e0), uv_table_float(e1), uv_table_float(e2), 1.0);
    out.uv_row1 = vec4<f32>(uv_table_float(e3), uv_table_float(e4), uv_table_float(e5), 0.0);
    return out;
}

@fragment
fn fragment(in: VertexOutput) {
#ifdef BINDLESS
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    var params = material_array[material_indices[slot].material];
    if params.uv_row0.w > 1.5 {
        let table = material_indices[slot].uv_table;
        let row = i32(params.uv_row0.w) - 2;
        params = with_table_matrix(params,
            textureLoad(bindless_textures_2d[table], vec2<i32>(0, row), 0),
            textureLoad(bindless_textures_2d[table], vec2<i32>(1, row), 0),
            textureLoad(bindless_textures_2d[table], vec2<i32>(2, row), 0),
            textureLoad(bindless_textures_2d[table], vec2<i32>(3, row), 0),
            textureLoad(bindless_textures_2d[table], vec2<i32>(4, row), 0),
            textureLoad(bindless_textures_2d[table], vec2<i32>(5, row), 0));
    }
    let cutoff = params.alpha_cutoff;
    let texel = textureSample(
        bindless_textures_2d[material_indices[slot].texture],
        bindless_samplers_filtering[material_indices[slot].texture_sampler],
        transformed_uv(params, in.uv));
#else   // BINDLESS
    var params = material;
    if params.uv_row0.w > 1.5 {
        let row = i32(params.uv_row0.w) - 2;
        params = with_table_matrix(params,
            textureLoad(uv_table_texture, vec2<i32>(0, row), 0),
            textureLoad(uv_table_texture, vec2<i32>(1, row), 0),
            textureLoad(uv_table_texture, vec2<i32>(2, row), 0),
            textureLoad(uv_table_texture, vec2<i32>(3, row), 0),
            textureLoad(uv_table_texture, vec2<i32>(4, row), 0),
            textureLoad(uv_table_texture, vec2<i32>(5, row), 0));
    }
    let cutoff = params.alpha_cutoff;
    let texel = textureSample(base_texture, base_sampler, transformed_uv(params, in.uv));
#endif  // BINDLESS
    if texel.a < cutoff {
        discard;
    }
}
