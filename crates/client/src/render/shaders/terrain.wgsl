// Up to four ground textures blended by the chunk's MCAL alpha maps, then a
// lambert term against MCNR's real per-vertex normals.
//
// A port of TERRAIN_FRAG in web/tiles.js, kept deliberately literal in its blend
// order. **Its lighting is no longer literal and is not meant to be**: the
// 0.55/0.45 split it carried was a colourless stand-in for a sun and a fill the
// game states per zone and per hour, and reading them is what this pass is now
// lit by. See `atmosphere.wgsl`.
//
// Layer 0 is the opaque base and has no alpha map; layers 1..3 are blended over
// it using the R, G and B channels of the tile's alpha atlas, in that order. The
// blend is a plain `mix` per layer applied in sequence, which is what the game
// does — each layer paints over everything already accumulated, so the order the
// layers appear in MCLY is load-bearing.
//
// The blend is sampled at `uv_b` and the ground textures at `uv`: the first is
// the chunk's cell of the tile-wide atlas, the second is chunk-local 0..1 scaled
// by the repeat. Two coordinates because they live in different spaces now,
// where a per-chunk alpha texture let one serve both.

// **The sun and the fog are not this shader's to choose**, and are not written
// out here — both come off `Light.dbc` per map and per hour, and both are shared
// with `m2.wgsl` so that a tree and the ground it stands on are lit by the same
// time of day. See `atmosphere.wgsl`.
#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_bindings::mesh
#import vale::atmosphere::{daylight_shadowed_lamplit, fogged, sun_sheen}
// …and the deviation, from the module that owns it rather than through the
// one that describes the game — see `night.wgsl`.
#import vale::night::lamplight
// …and the frame's own clock, for the layers that crawl. The forward binding
// only: this material declares a fragment shader and nothing else, so it never
// runs under the prepass group `wind.wgsl` has to fork for.
#import bevy_pbr::mesh_view_bindings::globals
#import vale::gamma::{to_frame, srgb_to_linear, linear_to_srgb}

// What a chunk that names no texture at all is drawn as — see the branch that
// reads it, at the foot of the layer cascade. A mid grey, so that ground with
// nothing painted on it reads as unpainted rather than as whatever its
// neighbour happens to carry.
const NO_TEXTURE: vec3<f32> = vec3<f32>(0.45, 0.45, 0.45);

struct TerrainParams {
    // How many of the four layers this draw group actually uses.
    layer_count: u32,
    // Ground textures repeat this many times across a chunk.
    repeat: f32,
    // **How much of the per-chunk tint to show**, 0 for none — which is every
    // frame of the client. See `TerrainMaterial::tint`.
    tint: f32,
    _pad: f32,
    // **How fast each layer crawls**, in texture widths a second, two layers to
    // a vector: `(u0, v0, u1, v1)` and `(u2, v2, u3, v3)`. `MCLY`'s animation
    // bits — the Burning Steppes lava and nothing else in Azeroth. A velocity
    // rather than an offset, so the CPU writes it once per tile and this
    // multiplies it by the clock. See `TerrainParams::scroll_a`.
    scroll_a: vec4<f32>,
    scroll_b: vec4<f32>,
};

// **Two binding layouts, one blend.** `TerrainMaterial` is `#[bindless]`, the
// same shape `m2.wgsl` documents: with the `BINDLESS` def the five textures
// live in the global binding arrays and the params in one storage array, and
// this material's indices into all of them come from the index table at
// binding 0, looked up by the mesh's own material slot. Without it, the
// classic bindings below are exactly what they always were. The two arms of
// the fragment sample the same cascade in the same order — WGSL cannot hold a
// binding-array element in a local, so the cascade is written twice and the
// two copies must be edited together.
#ifdef BINDLESS
#import bevy_render::bindless::{bindless_textures_2d, bindless_samplers_filtering}

// One entry per material in the slab; the fields are, in binding order, where
// each of this material's resources landed. The field names are this file's;
// the *order* is the contract with the `AsBindGroup` derive.
struct TerrainMaterialBindings {
    material: u32,
    alpha: u32,
    alpha_sampler: u32,
    layer0: u32,
    layer0_sampler: u32,
    layer1: u32,
    layer1_sampler: u32,
    layer2: u32,
    layer2_sampler: u32,
    layer3: u32,
    layer3_sampler: u32,
    tint: u32,
    tint_sampler: u32,
};

@group(3) @binding(0) var<storage> material_indices: array<TerrainMaterialBindings>;
@group(3) @binding(13) var<storage> material_array: array<TerrainParams>;
#else   // BINDLESS
@group(3) @binding(0) var<uniform> params: TerrainParams;
@group(3) @binding(1) var alpha_atlas: texture_2d<f32>;
@group(3) @binding(2) var alpha_sampler: sampler;
@group(3) @binding(3) var layer0: texture_2d<f32>;
@group(3) @binding(4) var layer0_sampler: sampler;
@group(3) @binding(5) var layer1: texture_2d<f32>;
@group(3) @binding(6) var layer1_sampler: sampler;
@group(3) @binding(7) var layer2: texture_2d<f32>;
@group(3) @binding(8) var layer2_sampler: sampler;
@group(3) @binding(9) var layer3: texture_2d<f32>;
@group(3) @binding(10) var layer3_sampler: sampler;
@group(3) @binding(11) var tint_map: texture_2d<f32>;
@group(3) @binding(12) var tint_sampler: sampler;
#endif  // BINDLESS

// **The colour of the sheen over this draw**, unpacked from Bevy's
// per-instance `u32` as `0x00RRGGBB` in the file's own space — the same
// packing `models::RoomLight` uses and for the same reason: it is the only
// per-frame channel a material-bound pass has that does not make the value a
// material dimension. Here the value is *global* rather than per instance —
// it is `Light.dbc`'s band 9 for the hour — and the tag is simply where a
// per-frame colour can ride without rewriting every ground material when the
// clock moves. See `terrain::sheen_tag`, which says why that matters.
//
// Zero is off, and it is what an untagged draw and a subtracted switch both
// read as.
fn sheen_colour(instance: u32) -> vec3<f32> {
    let tag = mesh[instance].tag;
    return vec3<f32>(
        f32((tag >> 16u) & 0xFFu),
        f32((tag >> 8u) & 0xFFu),
        f32(tag & 0xFFu),
    ) / 255.0;
}

@fragment
fn fragment(mesh_in: VertexOutput) -> @location(0) vec4<f32> {
#ifdef BINDLESS
    // The mesh's own material slot, exactly as `m2.wgsl` reads it.
    let slot = mesh[mesh_in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    let params = material_array[material_indices[slot].material];
#endif
    let t = mesh_in.uv * params.repeat;
    // **One UV per layer, because each one may crawl at its own rate.** Still
    // layers carry a zero velocity, so this is the same `t` for every group in
    // the game but the 164 records `vale textures` counts — no branch, and
    // nothing to get wrong on the common path.
    let drift = globals.time;
    let t0 = t + params.scroll_a.xy * drift;
    let t1 = t + params.scroll_a.zw * drift;
    let t2 = t + params.scroll_b.xy * drift;
    let t3 = t + params.scroll_b.zw * drift;
    // **Four channels, and the fourth is not transparency.** R, G and B are the
    // blend weights of layers 1..3; the alpha is the chunk's `MCSH` shadow,
    // packed into the same cell because it is the same 64x64 grid over the same
    // chunk at the same texel centres. See `Adt::alpha_atlas`.
#ifdef BINDLESS
    let atlas = textureSample(
        bindless_textures_2d[material_indices[slot].alpha],
        bindless_samplers_filtering[material_indices[slot].alpha_sampler],
        mesh_in.uv_b,
    );
#else
    let atlas = textureSample(alpha_atlas, alpha_sampler, mesh_in.uv_b);
#endif
    let blend = atlas.rgb;

    // **All four channels, and the fourth is the gloss mask** — the `_s`
    // texture's own alpha, splatted by the same weights as the colour because
    // `terrain2_s.bls` mixes the whole `RGBA` and then reads `.w`. A tileset
    // with no `_s` variant was forced matte at load; see
    // `terrain::ground_texture`.
#ifdef BINDLESS
    // **All four layers sampled unconditionally**, where the classic arm
    // samples inside the `layer_count` branches. It cannot: `params` is
    // per-slot here rather than a uniform, so the branch is non-uniform
    // control flow and naga rejects a `textureSample` (its derivatives)
    // inside it. An unused slot holds the layer-0 handle (`pick`'s fallback),
    // so the extra samples are valid and hit the same texels; the *mixes*
    // stay conditional, so the picture is the branch-for-branch same.
    let s0 = textureSample(
        bindless_textures_2d[material_indices[slot].layer0],
        bindless_samplers_filtering[material_indices[slot].layer0_sampler],
        t0,
    );
    let s1 = textureSample(
        bindless_textures_2d[material_indices[slot].layer1],
        bindless_samplers_filtering[material_indices[slot].layer1_sampler],
        t1,
    );
    let s2 = textureSample(
        bindless_textures_2d[material_indices[slot].layer2],
        bindless_samplers_filtering[material_indices[slot].layer2_sampler],
        t2,
    );
    let s3 = textureSample(
        bindless_textures_2d[material_indices[slot].layer3],
        bindless_samplers_filtering[material_indices[slot].layer3_sampler],
        t3,
    );
    var texel = s0;
    if params.layer_count > 1u {
        texel = mix(texel, s1, blend.r);
    }
    if params.layer_count > 2u {
        texel = mix(texel, s2, blend.g);
    }
    if params.layer_count > 3u {
        texel = mix(texel, s3, blend.b);
    }
#else
    var texel = textureSample(layer0, layer0_sampler, t0);
    if params.layer_count > 1u {
        texel = mix(texel, textureSample(layer1, layer1_sampler, t1), blend.r);
    }
    if params.layer_count > 2u {
        texel = mix(texel, textureSample(layer2, layer2_sampler, t2), blend.g);
    }
    if params.layer_count > 3u {
        texel = mix(texel, textureSample(layer3, layer3_sampler, t3), blend.b);
    }
#endif
    // **A chunk with no layers at all draws none of them**, and that is the
    // only reason this branch exists. `MCLY` is empty on 543 chunks of the
    // `development` map and on nothing the shipped continents ship, so the
    // renderer's fallback — layer 0, which is the *tile's* first texture,
    // because a draw group with no textures has none of its own — was
    // unobservable until an editor added a texture to the tile. Then every
    // blank chunk in the tile put on whatever had just been painted on one of
    // them, while its own `MCLY` was still empty and the panel still said
    // "0 of 4 textures".
    //
    // A flat neutral instead. It is **this client's own answer and not a
    // measured one**: nothing says what 5875 draws for a chunk that names no
    // texture, and the shipped maps never ask. What it is chosen for is that it
    // does not change when a neighbour is painted, which is the property the
    // fallback did not have.
    if params.layer_count == 0u {
        texel = vec4<f32>(NO_TEXTURE, 1.0);
    }

    // **The per-chunk tint**, which is off in every frame of the game and is
    // what lets a host paint the ground by the chunk without touching the
    // geometry. `uv_b` already addresses the alpha atlas's 16x16 grid of chunk
    // cells, so a 16x16 image sampled with the same coordinate — nearest — is
    // exactly this chunk's own texel and nothing else's. See
    // `TerrainMaterial::tint`.
    if params.tint > 0.0 {
#ifdef BINDLESS
        let wash = textureSample(
            bindless_textures_2d[material_indices[slot].tint],
            bindless_samplers_filtering[material_indices[slot].tint_sampler],
            mesh_in.uv_b,
        );
#else
        let wash = textureSample(tint_map, tint_sampler, mesh_in.uv_b);
#endif
        texel = vec4<f32>(mix(texel.rgb, wash.rgb, wash.a * params.tint), texel.a);
    }
    // **`MCCV` — the shading painted onto the chunk's own vertices**, and it
    // multiplies the blended texel before any light touches it. That order is
    // the point: it is paint on the ground rather than a light in the air, so a
    // darkened hollow stays darker at noon and at midnight and takes the sun's
    // colour like everything else.
    //
    // 0x7F is 1.0 and 0xFF is 2.0, so the attribute arrives centred on one and
    // reaching to two — see `vale_assets::world::adt::decode_colours`. The
    // alpha is not read: nothing in this renderer has a use for it yet, and a
    // fragment that multiplied by it would darken every painted vertex by half.
    //
    // **The whole branch is absent unless the attribute is.** `VERTEX_COLORS` is
    // pushed by bevy's mesh pipeline from the presence of `ATTRIBUTE_COLOR`, and
    // `render::terrain::shaded` only uploads it for a tile that carries an
    // `MCCV` — which no shipped 1.12 tile does. So the client's own frames
    // compile and run exactly the shader they did before this existed.
    //
    // It is a deviation from what 5875 draws and is stated as one.
#ifdef VERTEX_COLORS
    let colour = texel.rgb * mesh_in.color.rgb;
#else
    let colour = texel.rgb;
#endif

    // **What is standing on this patch of ground with a flame on it** — a
    // torch, a brazier, a campfire, a spell landing. Black in daylight, and
    // black at night wherever there is nothing near; see `render::lamps`, which
    // is this branch's own addition and which the game has no equivalent of.
    let lamps = lamplight(mesh_in.world_position, mesh_in.world_normal, mesh_in.position);

    // `MCNR`'s own per-vertex normals against the table's sun and fill, dimmed
    // by 30% where the atlas says the game baked a shadow — and the lamps added
    // to that sum rather than to the finished colour, because `MCSH` is about
    // the sun and has nothing to say about a torch.
    let light = daylight_shadowed_lamplit(mesh_in.world_normal, atlas.a, lamps);
    var lit = colour * light;

    // **And the sheen on top of it, in the game's own space.** The colour is
    // this instance's tag — `Light.dbc`'s band 9, or zero when the switch is
    // off — and the mask is the blended gloss times the shadow, which the
    // client gates rather than dims (`terrain1_s.bls`, quoted in
    // `atmosphere.wgsl`). Our atlas alpha is 1 where the client's shadow texel
    // is 0, hence `1 - atlas.a`.
    //
    // **The add is a sum and therefore happens in bytes**, which is this
    // renderer's standing rule and the reason for the round trip: the client
    // adds its specular to a byte-space product, and adding it to a linear one
    // instead lands a visibly weaker highlight on bright ground. The encode
    // and decode are exact inverses, so a fragment with no sheen — every one
    // of them with the switch off, and every matte texel with it on — comes
    // out the pixel it did before.
    let halo = sheen_colour(mesh_in.instance_index);
    if halo.r + halo.g + halo.b > 0.0 {
        let mask = texel.a * (1.0 - atlas.a);
        let sheen = sun_sheen(mesh_in.world_normal, mesh_in.world_position.xyz, halo) * mask;
        lit = srgb_to_linear(clamp(linear_to_srgb(lit) + sheen, vec3<f32>(0.0), vec3<f32>(1.0)));
    }

    // Encoded on the way out — the target holds the game's own bytes so that
    // what is drawn *over* this ground blends against it the way the client's
    // fixed-function ROP did. See `to_frame`. The ground itself is opaque, so
    // this is bit-for-bit the pixel it was: the encode moved one instruction
    // earlier, out of the target's sRGB view and into the shader.
    return to_frame(fogged(vec4<f32>(lit, 1.0), mesh_in.world_position));
}
