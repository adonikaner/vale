// One textured quad batch of the game's interface.
//
// The whole fragment is a sample times a vertex colour; everything that makes
// the five blend modes different is in the pipeline's blend state, set by
// `InterfaceMaterial::specialize` — a shader cannot see how two draws combine,
// which is the same argument `render::present` makes at length.
//
// The colour arithmetic matches the egui painter it replaces: the texture is
// `Rgba8UnormSrgb` so the sample arrives linear, the vertex colour is
// linearised on the CPU, and the alpha was compensated at upload and at tint
// time (`ui::framexml::byte_space_alpha`) — so the linear ROP mixes by the
// same visible fractions 1.12's byte-space one did.

// `bevy_sprite`, not `bevy_sprite_render`: the crate moved and the WGSL
// `#define_import_path` did not — see `render/shaders/present.wgsl`, which
// learned this first.
#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var art_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var art_sampler: sampler;

// The reference's alpha-key cutoff — its flat per-blend table says 224 — as the *compensated* alpha this pipeline actually carries:
// `byte_space_alpha(224/255)`, because every texel's alpha went through that
// compensation at upload and the threshold must follow it. Pinned by a test
// in `material.rs` so the literal cannot drift from the function.
const ALPHA_KEY_CUTOFF: f32 = 0.9863;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var colour = textureSample(art_texture, art_sampler, in.uv);
#ifdef VERTEX_COLORS
    colour = colour * in.color;
#endif
#ifdef ALPHA_KEY
    if colour.a < ALPHA_KEY_CUTOFF {
        discard;
    }
#endif
    return colour;
}
