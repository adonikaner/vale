// One texture, an alpha test, and three lighting models: the sun for anything
// outdoors, baked `MOCV` vertex colours for the inside of a building, and — on
// the batches its own `MOGP` header calls transition — the second faded into
// the first per vertex, which is what a doorway is.
//
// A port of MODEL_FRAG in web/models.js. Its lighting is no longer literal and
// is not meant to be: the 0.55/0.45 split it carried was a colourless stand-in
// for a sun and a fill the game states per zone and per hour, and both halves
// now come out of `atmosphere.wgsl` — literally the same code the ground is lit
// by, so a tree and the ground under it cannot read as different times of day.
//
// **Interiors are not lit by the sun, and cannot be.** A room lit by the same
// directional light as the field outside is lit *through its own roof*: the
// ceiling faces down, the lambert term goes to zero, and every interior reads as
// a black box. So a vertex-lit batch takes `clamp(ambient + colour, 0, 1)` scaled
// by a 0.9..1.1 ground-to-sky lean off the normal — the ratio is Noggit's, and it
// is what keeps a wall from reading as flat paint. The ambient is *added back*
// here because `WmoGroup::shaded_colours` subtracted it out; the two halves are
// one round trip and dropping either leaves every room too dark by exactly the
// ambient.
//
// A WMO batch is the only thing that supplies vertex colours, and Bevy compiles
// `VERTEX_COLORS` from the mesh's own attributes — so a doodad does not pay for
// the branch, it gets a variant without it.
//
// The blend *mode* is not here. Blend modes 0..6 are pipeline state — blend
// factors, depth write, cull mode — and are set in `M2Material::specialize`
// from the key, so a batch's mode costs a pipeline variant rather than a branch
// in every fragment.
//
// What is here is the part the shader has to do itself: `alpha_cutoff` is the
// alpha-key test (mode 1), which is a `discard` and not a blend. A leaf is
// either there or it is not, and blending it instead of discarding it turns
// every tree into a grey haze. Zero means no test at all.
//
// **And it is taken at the end, against the fragment's own alpha.** 1.12's
// alpha test is a ROP stage: it runs after every texture stage and after the
// vertex diffuse has been modulated in, and only then does the blend happen.
// Testing the *texel* instead — which is what this shader did — makes the test
// blind to the two things that actually carry a fade: a particle's over-life
// ramp and a batch's `M2Color` transparency track. The visible cost was a
// family of effects textured with a file that has no alpha channel at all
// (`SPELLS\CLOUDS.BLP` decodes 0% transparent), where every quad passed a test
// its own opacity ramp says it should have failed and drew as a hard opaque
// tile — the wall of blue squares an Evocation stood in.
//
// **Two binding layouts, one set of maths.** `M2Material` is `#[bindless]`, and
// on a machine that supports it the `BINDLESS` def is set: the texture and
// sampler live in global binding arrays (`bevy_render::bindless`), the params
// in one storage array, and this material's indices into all three come from
// the index table at binding 0, looked up by the mesh's own material slot. On a
// machine that does not, the def is absent and the three classic bindings are
// exactly what they always were. The fragment body reads `params` and `texel`
// and does not know which world it is in.

#import bevy_pbr::forward_io::VertexOutput
#import vale::atmosphere::{daylight_scaled_lamplit, fogged, fogged_to_black, sun_lambert}
// …and the deviation, from the module that owns it rather than through the
// one that describes the game — see `render/shaders/night.wgsl`.
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
    // **`w` is "do not fog"**, and it is here rather than in a field of its own
    // because a field of its own would change the size of this struct — which
    // in the bindless path is the *stride* `material_array` is indexed by, and
    // `m2_prepass.wgsl` would have to change with it or every alpha-masked
    // batch in the world would read some other material. One surface sets it:
    // the shadow blob under a unit, which is a `modulate` draw. See the
    // fragment.
    ambient: vec4<f32>,
    // The alpha-key cutoff, or 0 for no test. M2 cuts at 0.5; a WMO cuts at
    // 224/255, which is why this is a number and not a flag.
    alpha_cutoff: f32,
    // Material flag 0x01 — ignore lighting. 1.0 or 0.0, because a uniform
    // cannot be a bool without a shader def and this costs nothing.
    unlit: f32,
    // **Which of three lighting laws this batch takes** — 0 the sun, 1 its own
    // baked `MOCV`, 2 the bake faded per vertex toward the sun by the vertex's
    // alpha. The third is the seam between a building's inside and its outside;
    // see `vale_assets::wmo::BatchLight`, and the room sum below for why the
    // alpha channel means two different things under two of them.
    vertex_lit: f32,
    // This batch is an `MLIQ` surface: take both its colour and its alpha from
    // the two below rather than from the texel. 1.0 or 0.0, and nothing but
    // water, lava and slime sets it.
    liquid: f32,
    // The liquid's near colour, with the **shallow** opacity in `w`; and its far
    // colour, with the **deep** opacity in `w`. Both out of `Light.dbc` — see
    // `vale_assets::light`, and see the fragment below for why the texture
    // cannot supply either.
    liquid_close: vec4<f32>,
    liquid_far: vec4<f32>,
    // The batch's **texture matrix**, rows `(a, b, tx, active)` and
    // `(c, d, ty, _)`: `u' = a*u + b*v + tx`. `active` is 0 for everything that
    // does not state one — which is all but two dozen batches in the game — and
    // the identity has to be *skipped* rather than applied, because a zero row
    // would collapse every other batch's texture onto one texel.
    uv_row0: vec4<f32>,
    uv_row1: vec4<f32>,
    // `.x`: 0 for everything that is not a particle quad; 1 for one; 2 for one
    // whose blend is additive, which fogs toward *black* — the client's own
    // per-blend fog table: the air cannot be mixed into a glow that is *added* to the
    // frame, or every distant flame paints a fog-coloured square around
    // itself.
    //
    // `.y`: this batch's `MeshTag` is an **animated colour** (`0xAARRGGBB`)
    // rather than a room light and a sun scale. `M2Color` and the transparency
    // block are what make a spell effect end — an explosion's opacity runs
    // 0 -> 0.91 -> 0 over 767 ms — and a value that changes every frame cannot
    // be a material, so it rides the one per-instance word there is. See
    // `models::tint_tag`.
    //
    // `.z`: the **mouseover/target highlight**, as a flat lift added to this
    // batch's lighting factor before the texel is modulated by it. The client's
    // `SetHighlight` writes a configured RGB into the model and the
    // animate kernel hands it to `glMaterialfv(GL_EMISSION)`, whose shipped
    // default is `0xff404040` — 0x40/255 per channel. Zero on every batch
    // in the world but the two `render::selection` is lifting this frame.
    //
    // `.w`: **the colour an aura paints this unit's own model through**, as
    // `0xRRGGBB + 1` held in a float — Stoneform's `#44465e`, a ghost's
    // `#8cb9fd`, Shadowform's `#270042`. Zero on everything in the world that
    // no aura is painting, which is why the encoding is offset by one: black is
    // a colour a row really states (`Glowy (Black)`) and would otherwise be
    // indistinguishable from "nothing here". An f32 holds every 24-bit integer
    // exactly, so the pack is lossless — and it is the same trick the shipped
    // `SpellVisualKit` row uses to carry the colour in the first place.
    //
    // Here rather than in a vec4 of its own because this slot was already
    // allocated and paid for: the struct's size is the bindless stride and
    // every material in the slab carries it. See `world::entities::tint`.
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
    // **The two folded environment-map layers' blend modes**, `.x` then `.y`,
    // as the file's own numbering — 2 alpha, 3 additive, 4 add-alpha, 5
    // modulate, 6 modulate2x — and 0 for "no layer", which is every batch in
    // the world that is not a piece of armour. `.zw` unused. See
    // `models::material::M2Material::overlay_a`.
    overlay: vec4<f32>,
    scene_ambient: vec4<f32>,
    scene_lamps: array<vec4<f32>, 8>,
};

#ifdef BINDLESS
// The bindless index table: one entry per material in the slab, whose fields
// are, in binding order, where each of this material's resources landed —
// the `M2Params` element in `material_array` (binding 0), the texture in
// `bindless_textures_2d` (binding 1), the sampler in
// `bindless_samplers_filtering` (binding 2). The field names are this file's;
// the *order* is the contract with the `AsBindGroup` derive.
struct M2MaterialBindings {
    material: u32,
    texture: u32,
    texture_sampler: u32,
    // …and the two folded layers, bindless indices 3..6. A material with no
    // layer names its own base texture here, so these are always valid.
    overlay_a: u32,
    overlay_a_sampler: u32,
    overlay_b: u32,
    overlay_b_sampler: u32,
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
#endif  // BINDLESS

// This batch's UV with its texture matrix applied — or unchanged, which is what
// `active == 0` means and what every batch but two dozen gets.
//
// **In the fragment and not the vertex program**, which is where the 1.12
// client puts it: `Model2.bls` passes `vertex.texcoord[0]` through untouched
// and the transform is fixed-function texture-matrix state. Doing it per pixel
// is the same arithmetic on an affine map, and it keeps the vertex program
// Bevy's own.
fn transformed_uv(params: M2Params, uv: vec2<f32>) -> vec2<f32> {
    let moved = vec2<f32>(
        dot(params.uv_row0.xy, uv) + params.uv_row0.z,
        dot(params.uv_row1.xy, uv) + params.uv_row1.z,
    );
    return select(uv, moved, params.uv_row0.w > 0.5);
}

// **What a model that carries its own lights is lit by**, in place of the sun.
//
// The screens before the world are the whole population — see
// `vale_assets::m2::M2Light` and `models::material::SceneLighting` — and
// there it is not a refinement: `UI_MainMenu` states a near-black warm fill and
// three short-range lamps, its far scenery is flagged `unlit` and draws at full
// texture, and its near stone is authored to be dark except where the brazier
// reaches it. Lit by `Light.dbc`'s daylight instead, that stone draws at nearly
// its own texture value, which was the flat pale login screen.
//
// **Summed in the file's own space and decoded once**, exactly as `daylight`
// is and for the same reason — see `atmosphere.wgsl`, whose whole opening note
// is that this game adds colours as the bytes its files state. The clamp is
// the game's own saturation and not a safety net: a surface inside the
// brazier's radius reaches past 1.0 in the warm channel first, which is what
// makes lit stone read orange where its shadowed side reads the ambient.
//
// **Two kinds of light and the difference is the whole picture** — see
// `vale_assets::m2::M2Light::kind`, where the arithmetic that settles it is.
// 1.12's own lighting is fixed-function *per vertex* where this is per
// fragment, which makes the result smoother than the original rather than
// differently coloured.
#ifdef SCENE_LIT
fn model_light(params: M2Params, world_position: vec3<f32>, world_normal: vec3<f32>) -> vec3<f32> {
    var sum = params.scene_ambient.rgb;
    let count = u32(params.scene_ambient.w);
    let normal = normalize(world_normal);
    for (var i = 0u; i < count; i = i + 1u) {
        let place = params.scene_lamps[i * 2u];
        let colour = params.scene_lamps[i * 2u + 1u];
        if place.w > 0.5 {
            // **A point light**, placed, with the client's own fixed falloff —
            // `1 / (0.7d + 0.03d²)`, which has no radius in it. The file's two
            // attenuation distances are *not* used and that is a reading rather
            // than a shortcut: nearly every light in these scenes carries the
            // same 2.222..5.556, a directional one carries it as meaninglessly
            // as a lamp 127 yards from what it lights, and the curve is the
            // client's. Clamped at 1 because the expression diverges at
            // the light itself.
            let toward = place.xyz - world_position;
            let distance = max(length(toward), 1e-4);
            let fade = min(1.0 / (0.7 * distance + 0.03 * distance * distance), 1.0);
            sum += colour.rgb * (max(dot(normal, toward / distance), 0.0) * fade);
        } else {
            // **A directional light**: `place.xyz` is already a unit vector
            // pointing toward it and there is no distance in it at all.
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
    let params = material_array[material_indices[slot].material];
    let texel = textureSample(
        bindless_textures_2d[material_indices[slot].texture],
        bindless_samplers_filtering[material_indices[slot].texture_sampler],
        transformed_uv(params, in.uv));
#else   // BINDLESS
    let params = material;
    let texel = textureSample(base_texture, base_sampler, transformed_uv(params, in.uv));
#endif  // BINDLESS
    // **The instance's own light, off the mesh tag — both halves of it.** For
    // a `MODD` doodad — a chair, a barrel, a mug — and for an entity standing
    // in a room, the room's colour is per *spawn*, so it rides in `MeshTag`
    // (`0x00RRGGBB`, sRGB bytes) rather than in the material: a material is a
    // batch set, and one tile of Stormwind's furniture holds 2,249 distinct
    // lights. The top byte is the instance's **sun scale** in 1/32nds — the
    // per-instance multiplier the client's own `Model2.bls` ends in, 2.5 for
    // a terrain doodad on lit ground and 0.5 for one in the ground's baked
    // `MCSH` shadow (`models::sun_scale`). Zero — every entity, every WMO
    // batch, every untagged instance — is no room light and the neutral sun,
    // so nothing that never sets a tag changes. See `RoomLight` and
    // `instance_tag`.
    let tag = mesh[in.instance_index].tag;
    let spawn_light = vec3<f32>(
        f32((tag >> 16u) & 0xFFu),
        f32((tag >> 8u) & 0xFFu),
        f32(tag & 0xFFu),
    ) / 255.0;
    // …**unless the tag is a tint**, in which case all four bytes are the
    // batch's own animated colour and none of them is a sun scale. The top
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

    // The same sun as terrain.wgsl, and now literally the same code — see
    // `atmosphere.wgsl` for why that matters more here than anywhere else in
    // the renderer. The interior lean below wants the bare lambert against the
    // same direction, which is why both come from there.
    // **The lamps standing near this fragment**, which is the one term in this
    // shader that is not the game's — see `render::lamps`. Black in daylight
    // and black wherever nothing is burning, so every batch this client has
    // ever measured comes out the pixel it did.
    //
    // **Not computed at all for an `unlit` batch, and that is a real saving
    // rather than a tidy-up.** The `mix` at the end of the lighting block
    // discards `light` entirely when `unlit` is 1, so the cluster walk below
    // would be work whose result is thrown away — and the batches that are
    // unlit are exactly the ones with the worst overdraw in this game: a
    // building's lit windows, a lamp's own glow, every additive quad drawn with
    // no depth write. A house of lit windows is several of those over every
    // pixel it covers, which is where "the GPU time doubles when I look at that
    // building" came from. `params.unlit` is a uniform, so the branch is
    // coherent over the whole draw.
    var lamps = vec3<f32>(0.0);
    if params.unlit < 0.5 {
        lamps = lamplight(in.world_position, in.world_normal, in.position);
    }
    // **The clamped cosine, which is Direct3D's.** A trace of the reference
    // creates no vertex shader: on D3D — the path every reference picture is
    // of — a model's vertex colour is fixed-function `sun × max(N·L, 0) × scale
    // + fill`, saturated, and the texture modulates that. `Model2.bls`'s
    // spherical-harmonic wrap is the OpenGL path's and stays in
    // `atmosphere.wgsl` as `daylight_scaled_lamplit_sh` for an A/B. See the
    // rendering facts under *The reference client traced*.
    let sun = daylight_scaled_lamplit(in.world_normal, sun_scale, lamps);
    let lambert = sun_lambert(in.world_normal);

    // Only a WMO batch has these; a doodad's pipeline is compiled without the
    // attribute and takes the `else`, where `vertex_lit` is 0 anyway. The
    // alpha is two different payloads and both are per vertex: on a liquid
    // surface it is `MLIQ`'s depth (read below), and on a room's masonry it
    // is the authored **emissive mask** — see `emissive` at the room sum.
#ifdef VERTEX_COLORS
    let baked = in.color.rgb;
    let vertex_alpha = in.color.a;
    let emissive = in.color.a;
#else
    let baked = vec3<f32>(0.0);
    let vertex_alpha = 1.0;
    let emissive = 0.0;
#endif

    // **Summed in the file's space, then decoded — in that order.** The client
    // added `MOHD`'s ambient to a `MOCV` byte and drew the result, and
    // `WmoGroup::shaded_colours` subtracts the same ambient back out in the same
    // space, so the two halves only cancel if they are added before anything
    // else happens to them. The decode goes after, because every texel this
    // multiplies is already linear.
    //
    // A WMO batch sums `ambient + baked` with a zero tag; a room-lit M2 sums
    // its tag with a zero ambient and no colour attribute. One expression, and
    // the unused terms are zero rather than branched over.
    //
    // **0.9..1.1, which is the documented ratio and not the 0.7 this carried
    // for two rounds.** The lean is Noggit's flourish, not the file's — the
    // 1.12 client draws pre-lit geometry flat — so its one job is relief, and
    // it has to average out to no darkening: at 0.7..1.1 every surface facing
    // away from the sun's azimuth lost a fifth of the light the file states,
    // which is a fifth of every room, since a room's walls face everywhere.
    // **…times the vertex's own emissive mask, after the clamp and inside the
    // decode.** The game's own `MapObjOverbright.bls` — its standard WMO
    // diffuse pixel shader wherever the hardware allows — is `tex * MOCV * (1 + 4 * MOCV.a)`: an interior vertex's
    // alpha is authored self-illumination worth up to five times the base
    // (hearths, forges, portal seams — measured 90% zero / 1% high over
    // Stormwind's 486k coloured verts). The client lets the product overdrive
    // with only the framebuffer's clamp under it, so the gain goes *outside*
    // this clamp; and it multiplies a byte-space product, so it goes inside
    // the decode with the sum — `srgb_to_linear`'s power branch extends past
    // 1.0, which is exactly "byte-space ×5" carried into linear.
    //
    // **…and the alpha is only an emissive mask on the batches that read it as
    // one.** `vertex_lit` is three laws rather than a flag — 0 the sun, 1 this
    // bake, 2 the *seam* — and the third is where a building's inside meets its
    // outside: the transition batches the group's own `MOGP` header names, whose
    // alpha is a per-vertex weight toward the daylight instead. One channel,
    // two payloads, and the batch's section is what says which; measured apart
    // by `vale wmos`, which prints the two distributions and they look
    // nothing alike (a transition run is 31/18/39/10 across the range on
    // Stormwind, an interior one 94/5/0/0). Applying the emissive gain to a
    // seam would multiply a doorway by five; applying the fade to a hearth
    // would put the sun inside the room.
    let seam_lit = params.vertex_lit > 1.5;
    let gain = select(1.0 + 4.0 * emissive, 1.0, seam_lit);
    // **The lamps are a term of the room's sum too**, and this is where they
    // matter most: a building's inside is lit by a bake that knows nothing
    // about what is standing in it, so a torch in a corridor was previously the
    // one light in this game that lit nothing at all. Inside the clamp with the
    // other three, so a hearth that is already white stays white.
    let room = srgb_to_linear(clamp(
        params.ambient.rgb + spawn_light + baked + lamps,
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    ) * gain) * mix(0.9, 1.1, 0.5 + 0.5 * lambert);

    // **…and a third lighting model, for a model that carries its own.** It
    // replaces the sun outright rather than adding to it: a scene that states
    // its own lamps and its own near-black fill is stating the whole of what is
    // in it, and `Light.dbc`'s answer for a map nobody is on has nothing to do
    // with the picture.
    //
    // **A shader def and not a branch**, which is not an optimisation so much as
    // a refusal to make the world pay for two screens — see
    // `M2MaterialKey::scene_lit`, where the argument is. Outside those screens
    // this whole block is not compiled.
    // **The seam, which is the whole of how a doorway is lit.** A transition
    // batch is the same bake faded per vertex toward the surface as the sun
    // would light it: `MOCV * mix(1, sun, alpha)`. The reference draws these
    // batches twice — lit blended `SRC_ALPHA`, then the bake blended
    // `ONE_MINUS_SRC_ALPHA` — and the two passes collapse to exactly that
    // product, so this is one draw and not two.
    //
    // At alpha 0 it is the room, at alpha 1 it is the room's own colour under
    // full daylight, and the arch between them is the gradient that was
    // missing. What stood here instead was Noggit's collapse of the same two
    // passes against a *black* lit pass, folded into the vertex colour by the
    // parser: an outdoor-facing vertex came out black, which is why walking out
    // of a building crossed a step of darkness rather than a threshold. See
    // `WmoGroup::shaded_colours`, which no longer folds it.
    let seam = room * mix(vec3<f32>(1.0), sun, vertex_alpha);
#ifdef SCENE_LIT
    let light = model_light(params, in.world_position.xyz, in.world_normal);
#else
    let light = select(sun, select(room, seam, seam_lit), params.vertex_lit > 0.5);
#endif
    // **The highlight is added to the light, not to the colour**, which is the
    // reference's own placement: it goes into `GL_EMISSION` and therefore into
    // the fixed-function lighting sum, *before* the texture modulate — so a
    // black texel stays black and a lit one brightens by the lift times its own
    // colour. Adding it after the modulate would wash a dark model out to grey.
    // An `unlit` batch has no lighting sum to add to and takes nothing, which is
    // also what keeps a caster's additive spell effects out of it.
    // **…and the colour an aura paints the model itself**, which is the other
    // per-unit appearance change and goes the other way: the highlight is added
    // to the light and this *multiplies* the result, because a `charProc` colour
    // under 1 can only darken and every spell that states one — Stone Skin's
    // 50% grey, Shadowform's tenth, Stoneform's dark blue-grey — is visibly
    // darker than the model it is cast on.
    //
    // **Decoded before it multiplies.** The row states sRGB bytes and `texel` is
    // already linear; `srgb_to_linear` is a power law over almost its whole
    // range, so decoding the ratio and multiplying in linear reproduces the
    // fixed-function client's byte-space product exactly rather than
    // approximately.
    //
    // An `unlit` batch takes nothing, exactly as it takes no highlight — which
    // is what keeps the wearer's own additive spell glows out of it: a buff's
    // art is not part of the body the aura is painting.
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

    // **A liquid surface takes neither its colour nor its alpha from its
    // texture, because its texture has neither.** `lake_a` decodes to a peak
    // channel of 41 of 255 over the whole image, greyscale, and `ocean_h` to 82:
    // they are foam masks, and a canal drawn as `texel.rgb` is a black sheet.
    // The colour is the light table's, already lit — so it is not multiplied by
    // the sun — and the texture's own faint luminance is *added* on top, which
    // is where the glints come back. The alpha runs from the shallow value at
    // the bank to the deep one in the middle, `vertex_alpha` being `MLIQ`'s own
    // per-vertex depth byte. `liquid_far` is carried for the distance blend the
    // real client does and this one does not yet; only its `w` is read here.
    //
    // **Added in the file's own space and decoded once**, like every other sum
    // the atmosphere takes: the band is sRGB bytes out of `Light.dbc` and the
    // texel is already linear, so adding them as they stand mixes two spaces
    // and then treats the result as linear — which for a river's `0/29/41`
    // draws the band at 0.37 of full where the file says 0.11, and is why the
    // canals read as pale rather than as water.
    let water = srgb_to_linear(clamp(
        params.liquid_close.rgb + linear_to_srgb(texel.rgb),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    ));
    let water_alpha = mix(params.liquid_close.a, params.liquid_far.a, vertex_alpha);

    // **The folded environment-map layers**, and this is the whole of what a
    // draw call each was buying.
    //
    // Nearly every piece of armour and every weapon in the game is a base batch
    // plus one or two blended layers over *exactly the same triangles*, not
    // writing depth, drawn straight after it — a metal sheen and a soft glow.
    // Because the geometry and the depth are the same and nothing can be drawn
    // between them (anything nearer draws later, anything farther is rejected
    // by the base's own depth write), the destination each layer's blend reads
    // **is** the base's own output. So the blend equation evaluates here, in
    // the fragment that computed it, instead of in a second and third sorted
    // draw at ~18 µs apiece. See
    // `vale_assets::world::m2::overlay_layers` for the rule, and every
    // condition it refuses on.
    //
    // A layer is shaded exactly as its base is — the rule requires the two to
    // agree about `unlit` — so `shaded` is the base's own modulate applied to a
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
        // **The five blend equations `Models.applyBlend` states, in byte
        // space**, with the destination being what the stack has come to so
        // far. Modes 0 and 1 cannot appear: `overlay_layers` only folds a batch
        // that blends.
        //
        // **The round trip is measured rather than reasoned.** The obvious
        // reading is that a blend against a linear render target is linear
        // arithmetic and the fold should be too — and the pictures say
        // otherwise. Against the unfolded draw, over the character on the
        // character-select plinth: **linear differs on 3.53% of the pixels by
        // more than 16/255, and this differs on 0.18%, against a 0.11% floor
        // measured between two runs of the same build.** Below the floor at
        // >64/255. So the destination the hardware hands these factors is the
        // *encoded* value, which is the same fixed-function byte arithmetic the
        // particle ramp and the `M2Color` tint below already take the round trip
        // for — and it visibly matters here: `SHOULDERREFLECT01` is a flat grey
        // at 120/255, so a modulate2x is ×0.94 in bytes and ×0.37 in linear.
        // Pauldrons three times too dark, which is exactly what the first draft
        // drew.
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

    // **A particle quad is texel × its own over-life colour, in byte space.**
    // The 1.12 particle path is fixed-function gamma arithmetic like every
    // other sum in this renderer: the client multiplies the texture's bytes by
    // the authored ramp colour and blends the product as bytes, so the texel
    // is re-encoded, multiplied, and the product decoded — the same round trip
    // the fog takes, for the same reason. The colour rides `ATTRIBUTE_COLOR`
    // (the `baked`/`vertex_alpha` pair above — a particle mesh is the only
    // unlit mesh that carries the attribute), and the alpha *is* the ramp's
    // opacity: there is no separate alpha channel anywhere in the record.
    let spray = srgb_to_linear(linear_to_srgb(texel.rgb) * baked);
    let spray_alpha = texel.a * vertex_alpha;
    let is_particle = min(params.particle.x, 1.0);
    let drawn = vec4<f32>(
        mix(rgb, spray, is_particle),
        mix(alpha, spray_alpha, is_particle),
    );

    // **The batch's own animated colour, in byte space.** `M2Color` and the
    // transparency block are what end a spell effect: they multiply the drawn
    // colour and its opacity, sampled per frame on the instance's own clock and
    // delivered as its `MeshTag` (see the note on `particle.y`). The colour
    // takes the same re-encode/multiply/decode round trip the particle ramp
    // above does, because it is the same fixed-function arithmetic — the client
    // multiplied bytes by bytes. The alpha is a plain factor: a fade to zero is
    // a fade to zero in any space.
    let out = select(
        drawn,
        vec4<f32>(
            srgb_to_linear(linear_to_srgb(drawn.rgb) * tint.rgb),
            drawn.a * tint.a,
        ),
        is_tint,
    );

    // **The alpha test, here rather than at the texel** — see the header. The
    // reference's reference is a flat per-blend table (read on every write
    // of the blend state): 224 for the alpha key,
    // **1** for every translucent mode, 0 for opaque. So a mode-2..6 draw is not
    // untested — it discards a fully transparent fragment, which is what stops a
    // dead particle contributing to the depth-sorted list at all.
    //
    // Fog is deliberately below this and not above it: fog multiplies the
    // colour and leaves the alpha alone, so testing before or after it is the
    // same test — and taking it here keeps the discard out of the fog's own
    // arithmetic.
    if out.a < params.alpha_cutoff {
        discard;
    }

    // **…and only then, how solid this body is.** `params.body.x` is 1.0 for
    // everything in the world; a unit the server has marked creeping or ghostly
    // is drawn through a flat factor here. Below the test on purpose: the
    // cutoff is the blend mode's own number (224/255 for an alpha key), so
    // scaling the alpha before it would push a cut-out's own texels under their
    // own threshold and eat holes in the model — a stealthed rogue with gaps in
    // his cloak. Above the fog, which multiplies the colour and leaves the alpha
    // alone, so the order between those two does not matter.
    let solid = vec4<f32>(out.rgb, out.a * params.body.x);

    // Fog last, and over everything — a lamp inside a distant building fades
    // with the building. It multiplies the *colour* and leaves the alpha alone,
    // so an alpha-blended leaf keeps its own transparency and simply becomes
    // the colour of the air.
    //
    // **Except where the colour is a multiplier rather than a colour.** A
    // `modulate` draw is blended `dst * src`, so its texel is not something the
    // air can be mixed into: fading a shadow blob toward the fog colour does not
    // fade it out, it multiplies the distant ground by 0.05 and paints a black
    // disc under every far-off character — and where the light table starts its
    // fog at the camera (Alterac Valley all day, map 0 at dawn) that happens at
    // arm's length. The identity for a multiply is white, not the air.
    // An additive particle (`particle.x` = 2) fades toward black instead of
    // toward the air — under `(src·α, ONE)` blending, black *is* "not there".
    // **Branches rather than `select`, and this is not a style choice.**
    // `select` is an expression: **both** of its arms are evaluated and one
    // result is thrown away. So this fragment used to fog itself twice — once
    // toward the air and once toward black — and then, one line down, throw
    // *both* away for a batch that is not fogged at all.
    //
    // That was affordable while each was a `linear_fog`: a lerp and a clamp.
    // It stopped being affordable when the deep night went inside them, because
    // `night_air` is two `exp`s, a `pow`, a `length` and a `normalize` — and the
    // batches that pay it most are the ones with the worst overdraw in the
    // game. Goldshire's inn measured 11 ms against 5 ms looking away with the
    // night on, and 5 against 4 with it off: its lit windows are large additive
    // quads drawn with no depth write, several deep over every pixel of the
    // building, and each of them was running the mist twice.
    //
    // Both conditions are **material uniforms**, so these are coherent over the
    // whole draw rather than per fragment — the cheapest kind of branch a GPU
    // takes. And the outer one is the bigger of the two: `ambient.w` is the
    // shader's "leave this alone" flag, and a batch that carries it now does no
    // fog work at all instead of two lots of it.
    var shipped = solid;
    if params.ambient.w <= 0.5 {
        if params.particle.x > 1.5 {
            shipped = fogged_to_black(solid, in.world_position);
        } else {
            shipped = fogged(solid, in.world_position);
        }
    }
    // Encoded on the way out, because the *blend* is a sum too — see `to_frame`.
    // This is the surface it matters most on: 1,137 of the game's 1,332
    // spell-effect batches are `SRCALPHA, ONE`, and a stack of them summed in
    // linear never reaches the saturation the client's byte-space add does.
    return to_frame(shipped);
}
