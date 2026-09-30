// One texture, an alpha test, and three lighting models: the sun for anything
// outdoors, baked `MOCV` vertex colours for the inside of a building, and, on
// the batches the group's `MOGP` header marks as transition batches, the
// second faded into the first per vertex, which is how a doorway is lit.
//
// Ported from MODEL_FRAG in web/models.js, with different lighting: the
// 0.55/0.45 split there was a colourless stand-in for a sun and a fill the
// game states per zone and per hour. Both now come from `atmosphere.wgsl`, the
// same code the ground is lit by, so a tree and the ground under it are lit
// for the same time of day.
//
// ## Interior lighting
//
// Interiors are not lit by the sun. A room lit by the same directional light
// as the field outside is lit through its own roof: the ceiling faces down,
// the lambert term goes to zero, and every interior draws as a black box. So a
// vertex-lit batch takes `clamp(ambient + colour, 0, 1)` scaled by a 0.9..1.1
// ground-to-sky lean off the normal. The ratio is Noggit's, and it keeps a
// wall from looking like flat paint. The ambient is added back here because
// `WmoGroup::shaded_colours` subtracted it out; the two halves are one round
// trip and dropping either leaves every room too dark by exactly the ambient.
//
// A WMO batch is the only thing that supplies vertex colours, and Bevy compiles
// `VERTEX_COLORS` from the mesh's own attributes, so a doodad does not pay for
// the branch; it gets a variant without it.
//
// ## Blend mode and alpha test
//
// The blend mode is not applied here. Blend modes 0..6 are pipeline state
// (blend factors, depth write, cull mode) and are set in
// `M2Material::specialize` from the key, so a batch's mode costs a pipeline
// variant rather than a branch in every fragment.
//
// The shader does the alpha-key test (mode 1) itself: `alpha_cutoff` is a
// `discard`, not a blend. A leaf is either there or not, and blending it
// instead of discarding it turns every tree into a grey haze. Zero means no
// test at all.
//
// The test is taken at the end, against the fragment's own alpha. The 1.12
// alpha test is a ROP stage: it runs after every texture stage and after the
// vertex diffuse has been modulated in, and before the blend. Testing the
// texel instead, as this shader previously did, makes the test blind to the
// two things that carry a fade: a particle's over-life ramp and a batch's
// `M2Color` transparency track. Effects textured with a file that has no alpha
// channel (`SPELLS\CLOUDS.BLP` decodes 0% transparent) then passed a test their
// own opacity ramp should have failed and drew as hard opaque tiles; Evocation
// drew as a wall of blue squares.
//
// ## Binding layouts
//
// `M2Material` is `#[bindless]`, and on a machine that supports it the
// `BINDLESS` def is set: textures and samplers live in global binding arrays
// (`bevy_render::bindless`), the params in one storage array, and this
// material's indices into them come from the index table at binding 0, looked
// up by the mesh's own material slot. On a machine that does not, the def is
// absent and the material uses ordinary per-material bindings. The fragment
// body reads `params` and `texel` and is the same in both layouts.

#import bevy_pbr::forward_io::VertexOutput
#import vale::atmosphere::{daylight_scaled_lamplit, fogged, fogged_to_black, sun_lambert}
// The lamp term, which is this client's addition rather than the game's, from
// the module that owns it rather than through `atmosphere`; see
// `render/shaders/night.wgsl`.
#import vale::night::lamplight
// The transfer function is its own module — see the note at the top of
// `gamma.wgsl`. naga_oil does not re-export, so it is imported here rather than
// reached through `atmosphere`.
#import vale::gamma::{linear_to_srgb, srgb_to_linear, to_frame}
// The instance array, in both binding worlds: the bindless path reads its
// material slot from it, and the room-light tag below is read from it either
// way. `VERTEX_OUTPUT_INSTANCE_INDEX` is pushed unconditionally by the mesh
// pipeline, so `in.instance_index` is always there to index it with.
#import bevy_pbr::mesh_bindings::mesh

#ifdef BINDLESS
#import bevy_render::bindless::{bindless_textures_2d, bindless_samplers_filtering}
#endif

struct M2Params {
    // The building's `MOHD` ambient, added back to the vertex colours the parser
    // subtracted it from. Zero for an M2.
    //
    // `w` is "do not fog". It is here rather than in a field of its own because
    // a new field would change the size of this struct, which in the bindless
    // path is the stride `material_array` is indexed by; `m2_prepass.wgsl` would
    // have to change with it or every alpha-masked batch in the world would
    // read some other material. One surface sets it: the shadow blob under a
    // unit, which is a `modulate` draw. See the fog at the end of the fragment.
    ambient: vec4<f32>,
    // The alpha-key cutoff, or 0 for no test. M2 cuts at 0.5; a WMO cuts at
    // 224/255, which is why this is a number and not a flag.
    alpha_cutoff: f32,
    // Material flag 0x01 — ignore lighting. 1.0 or 0.0, because a uniform
    // cannot be a bool without a shader def and this costs nothing.
    unlit: f32,
    // Which of three lighting laws this batch takes: 0 the sun, 1 its own
    // baked `MOCV`, 2 the bake faded per vertex toward the sun by the vertex's
    // alpha. The third is the seam between a building's inside and its outside;
    // see `vale_assets::wmo::BatchLight`, and the room sum below for why the
    // alpha channel means two different things under two of them.
    vertex_lit: f32,
    // This batch is an `MLIQ` surface: take both its colour and its alpha from
    // the two below rather than from the texel. 1.0 or 0.0, and nothing but
    // water, lava and slime sets it.
    liquid: f32,
    // The liquid's near colour, with the shallow opacity in `w`; and its far
    // colour, with the deep opacity in `w`. Both from `Light.dbc`; see
    // `vale_assets::light`, and see the fragment below for why the texture
    // cannot supply either.
    liquid_close: vec4<f32>,
    liquid_far: vec4<f32>,
    // The batch's texture matrix, rows `(a, b, tx, source)` and
    // `(c, d, ty, _)`: `u' = a*u + b*v + tx`. `source` says where the matrix is:
    // 0, no matrix, which is all but two dozen batches in the game; 1, the
    // matrix is in these two rows; 2 + n, it is in row n of the texture-matrix
    // table, and the fragment replaces both rows with the table's values
    // (`with_table_matrix`) before applying them. A batch with no matrix skips
    // the transform rather than storing the identity, because its rows are zero
    // and applying them would collapse its texture onto one texel.
    uv_row0: vec4<f32>,
    uv_row1: vec4<f32>,
    // `.x`: 0 for everything that is not a particle quad; 1 for one; 2 for one
    // whose blend is additive, which fogs toward black, as the 1.12.1 client
    // fogs additive blends. The air cannot be mixed into a glow that is added
    // to the frame, or every distant flame paints a fog-coloured square around
    // itself.
    //
    // `.y`: this batch's `MeshTag` is an animated colour (`0xAARRGGBB`)
    // rather than a room light and a sun scale. `M2Color` and the transparency
    // block are what make a spell effect end (an explosion's opacity runs
    // 0 -> 0.91 -> 0 over 767 ms), and a value that changes every frame cannot
    // be a material, so it is carried in the one per-instance word there is.
    // See `models::tint_tag`.
    //
    // `.z`: the mouseover/target highlight, as a flat lift added to this
    // batch's lighting factor before the texel is modulated by it. The 1.12.1
    // client adds the highlight colour to the model's lighting as an emissive
    // term, and its default is 0x40/255 per channel. Zero on every batch in the
    // world but the two `render::selection` is lifting this frame.
    //
    // `.w`: the colour an aura paints this unit's own model with, as
    // `0xRRGGBB + 1` held in a float: Stoneform's `#44465e`, a ghost's
    // `#8cb9fd`, Shadowform's `#270042`. Zero on everything in the world that
    // no aura is painting. The encoding is offset by one because black is a
    // colour a row states (`Glowy (Black)`) and would otherwise be
    // indistinguishable from "nothing here". An f32 holds every 24-bit integer
    // exactly, so the pack is lossless; the `SpellVisualKit` row carries the
    // colour the same way.
    //
    // `.y`, `.z` and `.w` share this vec4 rather than taking one of their own
    // because the struct's size is the bindless stride and every material in
    // the slab carries every field. See `world::entities::tint`.
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
    // The two folded environment-map layers' blend modes, `.x` then `.y`,
    // in the file's own numbering (2 alpha, 3 additive, 4 add-alpha, 5
    // modulate, 6 modulate2x), and 0 for "no layer", which is every batch in
    // the world that is not a piece of armour. `.zw` unused. See
    // `models::material::M2Material::overlay_a`.
    overlay: vec4<f32>,
    // The lighting the model states for itself: `M2Light`, which nothing in
    // the world carries and which is the whole shading of the two screens
    // before it. `scene_ambient.rgb` is the fill and `.w` is how many of the
    // lamps below are real; each lamp is two vectors, `(place.xyz, is_point)`
    // and `(colour.rgb, 0)`, where `place` is a position for a point light and
    // a unit vector toward the light for a directional one. Zero everywhere
    // else, which costs the fragment one compare. See
    // `models::material::SceneLighting`.
    scene_ambient: vec4<f32>,
    scene_lamps: array<vec4<f32>, 8>,
};

#ifdef BINDLESS
// The bindless index table: one entry per material in the slab, whose fields
// are, in binding order, where each of this material's resources landed: the
// `M2Params` element in `material_array` (binding 0), the texture in
// `bindless_textures_2d` (binding 1), the sampler in
// `bindless_samplers_filtering` (binding 2), and so on. The field names are
// this file's; the order is the contract with the `AsBindGroup` derive.
struct M2MaterialBindings {
    material: u32,
    texture: u32,
    texture_sampler: u32,
    // The two folded layers, bindless indices 3..6. A material with no layer
    // names its own base texture here, so these are always valid.
    overlay_a: u32,
    overlay_a_sampler: u32,
    overlay_b: u32,
    overlay_b_sampler: u32,
    // The texture-matrix table, bindless index 7. See `with_table_matrix`.
    uv_table: u32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<storage> material_indices: array<M2MaterialBindings>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var<storage> material_array: array<M2Params>;
#else   // BINDLESS
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: M2Params;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var overlay_a_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var overlay_a_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var overlay_b_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var overlay_b_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var uv_table_texture: texture_2d<f32>;
#endif  // BINDLESS

// One `f32` of the texture-matrix table, reassembled from the four bytes of one
// `Rgba8Unorm` texel, least significant byte in red. See
// `models::material::uv_table_image` for why the table is stored this way.
fn uv_table_float(texel: vec4<f32>) -> f32 {
    let b = vec4<u32>(round(texel * 255.0));
    return bitcast<f32>(b.x | (b.y << 8u) | (b.z << 16u) | (b.w << 24u));
}

// `params` with its texture matrix taken from the table: the six texels of the
// batch's row, `a, b, tx, c, d, ty`. Called only when `uv_row0.w` is 2 or more,
// which is how `Materials::moving` marks a batch whose matrix is in the table.
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

// This batch's UV with its texture matrix applied, or unchanged when
// `uv_row0.w` is 0, which is every batch but two dozen. Called after any table
// row has been copied into `uv_row0`/`uv_row1`, so `w` is then 0 or 1.
//
// Applied in the fragment rather than the vertex program. In the 1.12 client
// `Model2.bls` passes `vertex.texcoord[0]` through untouched and the transform
// is fixed-function texture-matrix state. Doing it per pixel is the same
// arithmetic on an affine map, and it keeps the vertex program Bevy's own.
fn transformed_uv(params: M2Params, uv: vec2<f32>) -> vec2<f32> {
    let moved = vec2<f32>(
        dot(params.uv_row0.xy, uv) + params.uv_row0.z,
        dot(params.uv_row1.xy, uv) + params.uv_row1.z,
    );
    return select(uv, moved, params.uv_row0.w > 0.5);
}

// The lighting for a model that carries its own lights, in place of the sun.
//
// Only the screens before the world use it (see `vale_assets::m2::M2Light`
// and `models::material::SceneLighting`), and there it determines the whole
// picture: `UI_MainMenu` states a near-black warm fill and three short-range
// lamps, its far scenery is flagged `unlit` and draws at full texture, and its
// near stone is authored to be dark except where the brazier reaches it. Lit
// by `Light.dbc`'s daylight instead, that stone draws at nearly its own
// texture value, which gave a flat, pale login screen.
//
// Summed in the file's own space and decoded once, as `daylight` is and for
// the same reason: see `atmosphere.wgsl`, whose opening note is that this game
// adds colours as the bytes its files state. The clamp is the game's own
// saturation: a surface inside the brazier's radius reaches past 1.0 in the
// warm channel first, which makes lit stone orange where its shadowed side
// shows the ambient.
//
// Point and directional lights are computed differently; see
// `vale_assets::m2::M2Light::kind` for the arithmetic that distinguishes them.
// The 1.12 client lights per vertex with fixed-function state, where this
// lights per fragment, which makes the result smoother than the original
// rather than differently coloured.
#ifdef SCENE_LIT
fn model_light(params: M2Params, world_position: vec3<f32>, world_normal: vec3<f32>) -> vec3<f32> {
    var sum = params.scene_ambient.rgb;
    let count = u32(params.scene_ambient.w);
    let normal = normalize(world_normal);
    for (var i = 0u; i < count; i = i + 1u) {
        let place = params.scene_lamps[i * 2u];
        let colour = params.scene_lamps[i * 2u + 1u];
        if place.w > 0.5 {
            // A point light, with the client's fixed falloff
            // `1 / (0.7d + 0.03d²)`, which has no radius in it. The file's two
            // attenuation distances are not used, because the client does not
            // use them: nearly every light in these scenes carries the same
            // 2.222..5.556, a directional light carries the same values where
            // they can mean nothing (as would a lamp 127 yards from what it
            // lights), and the curve is the client's. Clamped at 1 because the
            // expression diverges at the light itself.
            let toward = place.xyz - world_position;
            let distance = max(length(toward), 1e-4);
            let fade = min(1.0 / (0.7 * distance + 0.03 * distance * distance), 1.0);
            sum += colour.rgb * (max(dot(normal, toward / distance), 0.0) * fade);
        } else {
            // A directional light: `place.xyz` is already a unit vector
            // pointing toward it, and there is no distance term.
            sum += colour.rgb * max(dot(normal, place.xyz), 0.0);
        }
    }
    return srgb_to_linear(clamp(sum, vec3<f32>(0.0), vec3<f32>(1.0)));
}
#endif  // SCENE_LIT

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
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
    let texel = textureSample(base_texture, base_sampler, transformed_uv(params, in.uv));
#endif  // BINDLESS
    // An early copy of the alpha test at the end of this function, for the
    // fragments it would discard in any case. Off a liquid, the final alpha is
    // `texel.a` times factors in 0..1 (the particle ramp, the vertex alpha, the
    // tint), so a texel under the cutoff fails the late test too. Discarding
    // here skips the lighting, the overlays and the tint for those fragments,
    // which in a forest are most of the leaf cards' area. The late test stays,
    // because only it sees the fades.
    if params.liquid < 0.5 && texel.a < params.alpha_cutoff {
        discard;
    }
    // The instance's own light, from the mesh tag: a room colour and a sun
    // scale. For a `MODD` doodad (a chair, a barrel, a mug) and for an entity
    // standing in a room, the room's colour is per spawn, so it is carried in
    // `MeshTag` (`0x00RRGGBB`, sRGB bytes) rather than in the material: a
    // material is a batch set, and one tile of Stormwind's furniture holds
    // 2,249 distinct lights. The top byte is the instance's sun scale in
    // 1/32nds, the per-instance multiplier the game's `Model2.bls` ends in:
    // 2.5 for a terrain doodad on lit ground and 0.5 for one in the ground's
    // baked `MCSH` shadow (`models::sun_scale`). Zero (every entity, every WMO
    // batch, every untagged instance) is no room light and the neutral sun,
    // so nothing that never sets a tag changes. See `RoomLight` and
    // `instance_tag`.
    let tag = mesh[in.instance_index].tag;
    let spawn_light = vec3<f32>(
        f32((tag >> 16u) & 0xFFu),
        f32((tag >> 8u) & 0xFFu),
        f32(tag & 0xFFu),
    ) / 255.0;
    // When the tag is a tint, all four bytes are the batch's own animated
    // colour and none of them is a sun scale. The top
    // byte is opacity, so reading it as 1/32nds would scale a half-faded
    // effect's sun by four. `params.particle.y` is the only thing that says
    // which of the two payloads the word holds.
    let is_tint = params.particle.y > 0.5;
    let scale_bits = tag >> 24u;
    let sun_scale = select(
        select(f32(scale_bits) / 32.0, 1.0, scale_bits == 0u),
        1.0,
        is_tint,
    );
    let tint = select(
        vec4<f32>(1.0),
        vec4<f32>(spawn_light, f32(scale_bits) / 255.0),
        is_tint,
    );

    // The lamps standing near this fragment, which is the one term in this
    // shader that is not the game's; see `render::lamps`. Black in daylight
    // and black wherever nothing is burning, so every batch this client has
    // measured draws the same pixel as without it.
    //
    // Not computed for an `unlit` batch. The `mix` at the end of the lighting
    // block discards `light` entirely when `unlit` is 1, so the cluster walk
    // would be work whose result is thrown away, and the unlit batches are the
    // ones with the worst overdraw in this game: a building's lit windows, a
    // lamp's own glow, every additive quad drawn with no depth write. A house
    // of lit windows is several of those over every pixel it covers, which
    // caused the report "the GPU time doubles when I look at that building".
    // `params.unlit` is a uniform, so the branch is coherent over the whole
    // draw.
    var lamps = vec3<f32>(0.0);
    if params.unlit < 0.5 {
        lamps = lamplight(in.world_position, in.world_normal, in.position);
    }
    // The same sun as terrain.wgsl, from the same code; see `atmosphere.wgsl`
    // for why that matters more here than anywhere else in the renderer. The
    // interior lean below needs the bare lambert against the same direction,
    // which is why both come from there.
    //
    // The sun term is the clamped cosine the 1.12.1 client's Direct3D path
    // uses. On Direct3D, the path every reference picture is of, the client
    // lights a model with fixed-function state: its vertex colour is
    // `sun × max(N·L, 0) × scale + fill`, saturated, and the texture modulates
    // that. `Model2.bls`'s spherical-harmonic wrap belongs to the OpenGL path
    // and stays in `atmosphere.wgsl` as `daylight_scaled_lamplit_sh` for an
    // A/B comparison.
    let sun = daylight_scaled_lamplit(in.world_normal, sun_scale, lamps);
    let lambert = sun_lambert(in.world_normal);

    // Only a WMO batch has these; a doodad's pipeline is compiled without the
    // attribute and takes the `else`, where `vertex_lit` is 0 anyway. The
    // alpha is two different payloads and both are per vertex: on a liquid
    // surface it is `MLIQ`'s depth (read below), and on a room's masonry it
    // is the authored emissive mask; see `emissive` at the room sum.
#ifdef VERTEX_COLORS
    let baked = in.color.rgb;
    let vertex_alpha = in.color.a;
    let emissive = in.color.a;
#else
    let baked = vec3<f32>(0.0);
    let vertex_alpha = 1.0;
    let emissive = 0.0;
#endif

    // The room light: summed in the file's space, then decoded, in that order.
    // The client adds `MOHD`'s ambient to a `MOCV` byte and draws the result,
    // and `WmoGroup::shaded_colours` subtracts the same ambient back out in the
    // same space, so the two halves only cancel if they are added before
    // anything else happens to them. The decode goes after, because every
    // texel this multiplies is already linear.
    //
    // A WMO batch sums `ambient + baked` with a zero tag; a room-lit M2 sums
    // its tag with a zero ambient and no colour attribute. One expression, and
    // the unused terms are zero rather than branched over.
    //
    // The lean is 0.9..1.1, the documented ratio; it was previously 0.7..1.1.
    // The lean is Noggit's addition, not the file's (the 1.12 client draws
    // pre-lit geometry flat), so its only job is relief, and it has to average
    // out to no darkening: at 0.7..1.1 every surface facing away from the
    // sun's azimuth lost a fifth of the light the file states, which is a fifth
    // of every room, since a room's walls face every direction.
    //
    // The sum is multiplied by the vertex's own emissive mask, after the clamp
    // and inside the decode. The game's `MapObjOverbright.bls`, its standard
    // WMO diffuse pixel shader wherever the hardware allows, is
    // `tex * MOCV * (1 + 4 * MOCV.a)`: an interior vertex's alpha is authored
    // self-illumination worth up to five times the base (hearths, forges,
    // portal seams; measured 90% zero / 1% high over Stormwind's 486k coloured
    // verts). The client lets the product overdrive with only the framebuffer's
    // clamp under it, so the gain goes outside this clamp; and it multiplies a
    // byte-space product, so it goes inside the decode with the sum.
    // `srgb_to_linear`'s power branch extends past 1.0, which carries
    // "byte-space ×5" into linear exactly.
    //
    // The alpha is an emissive mask only on the batches that read it as one.
    // `vertex_lit` is three laws rather than a flag (0 the sun, 1 this bake,
    // 2 the seam), and the third is where a building's inside meets its
    // outside: the transition batches the group's own `MOGP` header names,
    // whose alpha is a per-vertex weight toward the daylight instead. One
    // channel carries two payloads, and the batch's section says which.
    // `vale wmos` prints the two distributions, and they differ (a transition
    // run is 31/18/39/10 across the range on Stormwind, an interior one
    // 94/5/0/0). Applying the emissive gain to a seam would multiply a doorway
    // by five; applying the fade to a hearth would put the sun inside the room.
    let seam_lit = params.vertex_lit > 1.5;
    let gain = select(1.0 + 4.0 * emissive, 1.0, seam_lit);
    // The lamps are a term of the room's sum too, and matter most here: a
    // building's inside is lit by a bake that knows nothing about what is
    // standing in it, so without this term a torch in a corridor lit nothing.
    // Inside the clamp with the other three, so a hearth that is already white
    // stays white.
    let room = srgb_to_linear(clamp(
        params.ambient.rgb + spawn_light + baked + lamps,
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    ) * gain) * mix(0.9, 1.1, 0.5 + 0.5 * lambert);

    // The seam, which is how a doorway is lit. A transition batch is the same
    // bake faded per vertex toward the surface as the sun would light it:
    // `MOCV * mix(1, sun, alpha)`. The 1.12.1 client draws these batches twice
    // (lit, blended `SRC_ALPHA`, then the bake, blended `ONE_MINUS_SRC_ALPHA`),
    // and the two passes reduce to exactly that product, so this is one draw
    // and not two.
    //
    // At alpha 0 it is the room, at alpha 1 it is the room's own colour under
    // full daylight, and the arch between them is a gradient. The parser
    // previously folded Noggit's reduction of the same two passes, against a
    // black lit pass, into the vertex colour: an outdoor-facing vertex came out
    // black, so walking out of a building crossed a step of darkness. See
    // `WmoGroup::shaded_colours`, which no longer folds it.
    let seam = room * mix(vec3<f32>(1.0), sun, vertex_alpha);
    // The third lighting model, for a model that carries its own lights. It
    // replaces the sun rather than adding to it: a scene that states its own
    // lamps and its own near-black fill states everything that lights it, and
    // `Light.dbc`'s values for a map nobody is on do not apply.
    //
    // A shader def and not a branch, so that the world does not pay for two
    // screens; see `M2MaterialKey::scene_lit` for the reasoning. Outside those
    // screens the scene path is not compiled.
#ifdef SCENE_LIT
    let light = model_light(params, in.world_position.xyz, in.world_normal);
#else
    let light = select(sun, select(room, seam, seam_lit), params.vertex_lit > 0.5);
#endif
    // The highlight is added to the light, not to the colour, as in the 1.12.1
    // client, which adds it as an emissive term in the fixed-function lighting
    // sum before the texture modulate. A black texel stays black and a lit one
    // brightens by the lift times its own colour. Adding it after the modulate
    // would wash a dark model out to grey. An `unlit` batch has no lighting sum
    // to add to and takes nothing, which also keeps a caster's additive spell
    // effects out of it.
    //
    // The colour an aura paints the model with is the other per-unit
    // appearance change, and it multiplies the result where the highlight is
    // added: a `charProc` colour under 1 can only darken, and every spell that
    // states one (Stone Skin's 50% grey, Shadowform's tenth, Stoneform's dark
    // blue-grey) is visibly darker than the model it is cast on.
    //
    // The colour is decoded before it multiplies. The row states sRGB bytes
    // and `texel` is already linear; `srgb_to_linear` is a power law over
    // almost its whole range, so decoding the ratio and multiplying in linear
    // reproduces the fixed-function client's byte-space product exactly rather
    // than approximately.
    //
    // An `unlit` batch takes no aura colour, as it takes no highlight, which
    // keeps the wearer's own additive spell glows out of it: a buff's art is
    // not part of the body the aura is painting.
    let painted = u32(params.particle.w);
    let paint_bytes = painted - 1u;
    let paint = srgb_to_linear(vec3<f32>(
        f32((paint_bytes >> 16u) & 0xFFu),
        f32((paint_bytes >> 8u) & 0xFFu),
        f32(paint_bytes & 0xFFu)
    ) / 255.0);
    let tinted_light = select(vec3<f32>(1.0), paint, painted != 0u);
    let lit = mix(
        texel.rgb * (light + vec3<f32>(params.particle.z)) * tinted_light,
        texel.rgb,
        params.unlit,
    );

    // A liquid surface takes neither its colour nor its alpha from its
    // texture, because its texture has neither. `lake_a` decodes to a peak
    // channel of 41 of 255 over the whole image, greyscale, and `ocean_h` to 82:
    // they are foam masks, and a canal drawn as `texel.rgb` is a black sheet.
    // The colour is the light table's, already lit, so it is not multiplied by
    // the sun, and the texture's own faint luminance is added on top, which
    // gives the glints. The alpha runs from the shallow value at the bank to
    // the deep one in the middle, `vertex_alpha` being `MLIQ`'s own per-vertex
    // depth byte. `liquid_far` is carried for the distance blend the 1.12.1
    // client does and this one does not yet; only its `w` is read here.
    //
    // Added in the file's own space and decoded once, like every other sum
    // the atmosphere takes. The band is sRGB bytes from `Light.dbc` and the
    // texel is already linear, so adding them as they stand mixes two spaces
    // and then treats the result as linear: for a river's `0/29/41` that draws
    // the band at 0.37 of full where the file says 0.11, and the canals looked
    // pale rather than like water.
    let water = srgb_to_linear(clamp(
        params.liquid_close.rgb + linear_to_srgb(texel.rgb),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    ));
    let water_alpha = mix(params.liquid_close.a, params.liquid_far.a, vertex_alpha);

    // The folded environment-map layers, evaluated here instead of as separate
    // draw calls.
    //
    // Nearly every piece of armour and every weapon in the game is a base batch
    // plus one or two blended layers over exactly the same triangles, not
    // writing depth, drawn straight after it: a metal sheen and a soft glow.
    // Because the geometry and the depth are the same and nothing can be drawn
    // between them (anything nearer draws later, anything farther is rejected
    // by the base's own depth write), the destination each layer's blend reads
    // is the base's own output. So the blend equation evaluates here, in the
    // fragment that computed that output, instead of in a second and third
    // sorted draw at ~18 µs apiece. See `vale_assets::world::m2::overlay_layers`
    // for the rule and every condition it refuses on.
    //
    // A layer is shaded exactly as its base is (the rule requires the two to
    // agree about `unlit`), so `shaded` is the base's own modulate applied to a
    // different texel.
    var layered = lit;
    for (var i = 0u; i < 2u; i += 1u) {
        let mode = select(params.overlay.y, params.overlay.x, i == 0u);
        if mode < 0.5 {
            continue;
        }
#ifdef BINDLESS
        let sample_a = textureSample(
            bindless_textures_2d[material_indices[slot].overlay_a],
            bindless_samplers_filtering[material_indices[slot].overlay_a_sampler],
            transformed_uv(params, in.uv));
        let sample_b = textureSample(
            bindless_textures_2d[material_indices[slot].overlay_b],
            bindless_samplers_filtering[material_indices[slot].overlay_b_sampler],
            transformed_uv(params, in.uv));
#else   // BINDLESS
        let sample_a = textureSample(
            overlay_a_texture, overlay_a_sampler, transformed_uv(params, in.uv));
        let sample_b = textureSample(
            overlay_b_texture, overlay_b_sampler, transformed_uv(params, in.uv));
#endif  // BINDLESS
        let layer = select(sample_b, sample_a, i == 0u);
        let shaded = mix(
            layer.rgb * (light + vec3<f32>(params.particle.z)) * tinted_light,
            layer.rgb,
            params.unlit,
        );
        // The five blend equations `Models.applyBlend` states, in byte space,
        // with the destination being the result of the stack so far. Modes 0
        // and 1 cannot appear: `overlay_layers` only folds a batch that blends.
        //
        // The byte-space round trip is chosen by measurement. A blend against
        // a linear render target might be expected to be linear arithmetic, but
        // compared with the unfolded draw, over the character on the
        // character-select plinth, linear arithmetic differs on 3.53% of the
        // pixels by more than 16/255, and this code differs on 0.18%, against a
        // 0.11% floor measured between two runs of the same build; at >64/255
        // this code's difference is below the floor. So the destination the blend factors
        // apply to is the encoded value: the same fixed-function byte
        // arithmetic the particle ramp and the `M2Color` tint below take the
        // round trip for. It matters visibly here: `SHOULDERREFLECT01` is a
        // flat grey at 120/255, so a modulate2x is ×0.94 in bytes and ×0.37 in
        // linear, which draws pauldrons three times too dark.
        let d = linear_to_srgb(layered);
        let s = linear_to_srgb(shaded);
        if mode < 2.5 {            // 2 — alpha
            layered = srgb_to_linear(mix(d, s, layer.a));
        } else if mode < 3.5 {     // 3 — additive
            layered = srgb_to_linear(d + s);
        } else if mode < 4.5 {     // 4 — add-alpha
            layered = srgb_to_linear(d + s * layer.a);
        } else if mode < 5.5 {     // 5 — modulate
            layered = srgb_to_linear(d * s);
        } else {                   // 6 — modulate2x
            layered = srgb_to_linear(d * s * 2.0);
        }
    }

    let rgb = mix(layered, water, params.liquid);
    let alpha = mix(texel.a, water_alpha, params.liquid);

    // A particle quad is texel × its own over-life colour, in byte space.
    // The 1.12 particle path is fixed-function gamma arithmetic like every
    // other sum in this renderer: the client multiplies the texture's bytes by
    // the authored ramp colour and blends the product as bytes, so the texel
    // is re-encoded, multiplied, and the product decoded, the same round trip
    // the fog takes, for the same reason. The colour is carried in
    // `ATTRIBUTE_COLOR` (the `baked`/`vertex_alpha` pair above; a particle mesh
    // is the only unlit mesh that carries the attribute), and the alpha is the
    // ramp's opacity: there is no separate alpha channel anywhere in the record.
    let spray = srgb_to_linear(linear_to_srgb(texel.rgb) * baked);
    let spray_alpha = texel.a * vertex_alpha;
    let is_particle = min(params.particle.x, 1.0);
    let drawn = vec4<f32>(
        mix(rgb, spray, is_particle),
        mix(alpha, spray_alpha, is_particle),
    );

    // The batch's own animated colour, in byte space. `M2Color` and the
    // transparency block are what end a spell effect: they multiply the drawn
    // colour and its opacity, sampled per frame on the instance's own clock and
    // delivered as its `MeshTag` (see the note on `particle.y`). The colour
    // takes the same re-encode/multiply/decode round trip the particle ramp
    // above does, because it is the same fixed-function arithmetic: the client
    // multiplies bytes by bytes. The alpha is a plain factor, because a fade to
    // zero is the same in any space.
    let out = select(
        drawn,
        vec4<f32>(
            srgb_to_linear(linear_to_srgb(drawn.rgb) * tint.rgb),
            drawn.a * tint.a,
        ),
        is_tint,
    );

    // The alpha test, here rather than at the texel; see the header. The
    // 1.12.1 client's alpha-test reference depends only on the blend mode: 224
    // for the alpha key, 1 for every translucent mode, 0 for opaque. So a
    // mode-2..6 draw is tested too: it discards a fully transparent fragment,
    // which stops a dead particle contributing to the depth-sorted list at all.
    //
    // Fog is below this rather than above it. Fog multiplies the colour and
    // leaves the alpha alone, so testing before or after it is the same test,
    // and testing here keeps discarded fragments out of the fog arithmetic.
    if out.a < params.alpha_cutoff {
        discard;
    }

    // After the test, how solid this body is. `params.body.x` is 1.0 for
    // everything in the world; a unit the server has marked creeping or ghostly
    // is drawn through a flat factor here. It is below the test because the
    // cutoff is the blend mode's own number (224/255 for an alpha key), so
    // scaling the alpha before it would push a cut-out's own texels under their
    // own threshold and cut holes in the model: a stealthed rogue with gaps in
    // his cloak. It is above the fog, which multiplies the colour and leaves
    // the alpha alone, so the order between those two does not matter.
    let solid = vec4<f32>(out.rgb, out.a * params.body.x);

    // Fog last, and over everything: a lamp inside a distant building fades
    // with the building. It multiplies the colour and leaves the alpha alone,
    // so an alpha-blended leaf keeps its own transparency and becomes the
    // colour of the air.
    //
    // A batch whose colour is a multiplier is not fogged. A `modulate` draw is
    // blended `dst * src`, so its texel is not something the air can be mixed
    // into: fading a shadow blob toward the fog colour does not fade it out, it
    // multiplies the distant ground by 0.05 and paints a black disc under every
    // far-off character, and where the light table starts its fog at the
    // camera (Alterac Valley all day, map 0 at dawn) that happens at arm's
    // length. The identity for a multiply is white, not the air; `ambient.w`
    // marks such a batch.
    //
    // An additive particle (`particle.x` = 2) fades toward black instead of
    // toward the air: under `(src·α, ONE)` blending, black adds nothing.
    //
    // These are branches rather than `select` for performance. `select` is an
    // expression: both of its arms are evaluated and one result is thrown away.
    // With `select`, this fragment fogged itself twice, once toward the air and
    // once toward black, and then discarded both for a batch that is not
    // fogged at all.
    //
    // That was affordable while each was a `linear_fog`: a lerp and a clamp.
    // It is not with the deep night inside them, because `night_air` is two
    // `exp`s, a `pow`, a `length` and a `normalize`, and the batches that pay
    // it most are the ones with the worst overdraw in the game. Goldshire's inn
    // measured 11 ms against 5 ms looking away with the night on, and 5 against
    // 4 with it off: its lit windows are large additive quads drawn with no
    // depth write, several deep over every pixel of the building, and each of
    // them ran the mist twice.
    //
    // Both conditions are material uniforms, so the branches are coherent over
    // the whole draw rather than per fragment, the cheapest kind of branch a
    // GPU takes. The outer one saves more: `ambient.w` is the shader's "leave
    // this alone" flag, and a batch that carries it does no fog work at all.
    var shipped = solid;
    if params.ambient.w <= 0.5 {
        if params.particle.x > 1.5 {
            shipped = fogged_to_black(solid, in.world_position);
        } else {
            shipped = fogged(solid, in.world_position);
        }
    }
    // Encoded on the way out, because the blend is a sum too; see `to_frame`.
    // This is the surface it matters most on: 1,137 of the game's 1,332
    // spell-effect batches are `SRCALPHA, ONE`, and a stack of them summed in
    // linear never reaches the saturation the client's byte-space add does.
    return to_frame(shipped);
}
