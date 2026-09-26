#define_import_path vale::atmosphere

// **The two things every surface in the world does with the light, in one
// place.** Terrain and models are separate shaders with separate bindings, and
// before this they each carried their own copy of the sun — a `0.55 + 0.45 *
// lambert` written out twice, with a comment in each saying the other one had
// to match. That comment is the whole argument for this file: the two are only
// ever *checked* against each other by looking at a tree standing on the
// ground, which costs a login, and nothing fails on the day one of them drifts.
//
// The values come off `Light.dbc` per map and per hour and ride in Bevy's own
// view bind group, which every material pipeline already has bound at group 0.
// See `crate::sky` for how they get there.
//
// **They arrive in the file's own space, undecoded, and that is the whole of
// this file's contract.** Every colour here is *combined* before it is used —
// the fill is added to the sun, the shadow band is mixed into the sum, the air
// is mixed into the result — and a sum does not survive a transfer function:
// `srgb_to_linear(a) + srgb_to_linear(b)` is not `srgb_to_linear(a + b)`. A
// *product* very nearly does, which is why the texel can stay linear and only
// the sums have to move.
//
// Measured, on map 0 at 10:26 with the fill at 102/129/155 and the sun at
// 255/128/0 on ground facing 0.76 into it: summed in the file's space the
// ground is lit by `255/227/155`, summed in linear it is lit by `243/167/155`.
// The green channel is the whole difference between grass and rust, and it is
// what "the lighting does not match" was.
//
// The 1.12 client settles it outright and cheaply: it is
// fixed-function, and the only gamma anywhere in it is
// `SetDeviceGammaRamp` — the display slider, applied to the finished frame by
// the driver. There is no sRGB sampler state, no linear working space and no
// display transform, so every colour the client adds together it adds as the
// bytes the file states. This does the same and decodes once, at the end.

#import bevy_pbr::mesh_view_bindings::{lights, view}
// **The fog binding only exists for a view whose camera carries `DistanceFog`.**
// `mesh_view_bindings.wgsl` gates `@binding(13) fog` behind `#ifdef
// DISTANCE_FOG`, and two whole populations of views legitimately lack the
// component: the portrait cameras (`render::portraits`, nine of them), and the
// world camera itself under `--without fog`, whose subtraction *removes* the
// component rather than pushing the falloff out — see `render::sky`, which says
// why. An unguarded import here fails pipeline specialization for every one of
// those views, which for the world camera is an invisible world and for a
// portrait is a face that never draws.
#ifdef DISTANCE_FOG
#import bevy_pbr::mesh_view_bindings::fog
#import bevy_pbr::mesh_view_types::FOG_MODE_OFF
#import bevy_pbr::fog::linear_fog
#endif

// **The two transfer functions this file is built on live in `gamma.wgsl`**, and
// they moved there for a mechanical reason rather than a tidy one: naga_oil does
// not strip an imported module's unused functions, so anything importing
// `srgb_to_linear` from *here* also pulls in `fogged` and `daylight` and with
// them `bevy_pbr::mesh_view_bindings` — bindings a 2D pipeline does not have.
// `present.wgsl` is that importer. `to_frame` — what a fragment hands to the
// framebuffer, which is where the *blend* obeys everything below — is there too,
// for the same reason and because it is the same subject.
//
// **And what is deliberately *not* here is the deep night**, which lives in
// `night.wgsl` beside this file — see the import below. That is a deviation from
// the game rather than a reading of it, and this file's whole argument is about
// the difference.
//
// What is still here, and belongs here: **the one light that cannot be decoded
// on the CPU.** A room's is `MOHD`'s ambient plus a `MOCV` vertex colour, and
// the sum has to be taken in the file's own space before it is decoded — the
// client did that arithmetic in bytes, and `WmoGroup::shaded_colours` subtracts
// the ambient back out in bytes too, so the decode has to happen after the add.
//
// Why the decode is not optional: the model and terrain textures are uploaded
// `Rgba8UnormSrgb`, so `textureSample` already returns linear. A linear texel
// times an sRGB-encoded light is not a darker picture, it is a picture with the
// wrong hue relationships — mid-tones lifted, contrast flattened. Skipping it
// for one surface and not the others is worse still: interiors were the last
// thing in the world still lit in gamma space, so a tavern read as a flat,
// bright cut-out standing in a world that had been converted around it.
#import vale::gamma::{srgb_to_linear, linear_to_srgb}
// **And the one thing in this renderer that is not a reading of anything**,
// which is why it is a module of its own rather than a section of this one.
//
// `night.wgsl` is the deep night: a grade and a mist that 1.12 does not have and
// that no file, shader or behaviour of the client states. Everything above and
// below this line is an attempt to draw what the 5875 client drew; that is a
// deliberate deviation from it, and the two must not be read as the same kind
// of claim. See `render::night`, which owns the option.
//
// It hooks into this file at exactly three places and they are all thin: the
// three `*_lamplit` entry points below take a colour it computed and add it to
// the sum, and `fogged`/`fogged_to_black` hand their result to `night_air` while
// it is still in the file's own space. At a night factor of zero every one of
// them is the identity, which is the property `render::night`'s own tests pin.
#import vale::night::night_air

// How square-on to the sun a surface is, 0..1.
//
// Exposed on its own because a WMO's interior lighting needs the *geometry* of
// the sun without its colour — a room is lit by its own baked `MOCV` colours and
// then leaned along this, so a wall facing the sky is not painted as flat as the
// floor. Taking it from here rather than recomputing it is what stops an
// interior from leaning toward one sun while the field outside is lit by
// another.
//
// Every entry point guards on the light count, because the glue screens run
// before a sun is spawned and an unwritten light slot is not zero — it is
// whatever was in the buffer.
fn sun_lambert(world_normal: vec3<f32>) -> f32 {
    if lights.n_directional_lights == 0u {
        return 1.0;
    }
    return max(
        dot(normalize(world_normal), lights.directional_lights[0].direction_to_light),
        0.0,
    );
}

// **The OpenGL path's sun, and no longer the model shader's.** A Direct3D
// trace of the reference (rendering facts, *The reference client traced*)
// creates no vertex shader at all: on D3D — the path every reference picture
// is of — a model is lit by fixed-function `max(N·L, 0)`, and `m2.wgsl` takes
// `daylight_scaled_lamplit` above. What follows is what `Model2.bls`'s ARB
// program computes, kept for the record and for an A/B.
//
// **The sun as `Model2.bls` sees it**: not a clamped cosine but the second-order
// spherical-harmonic projection of one. The lit permutation of the model
// vertex program evaluates a nine-coefficient SH (`c10[0..6]`: the linear terms
// against the normal, the quadratic against its products, and the `x²-y²`
// term) and writes `c28[0] * that + c28[1]` to `result.color.front.primary`;
// there is no model pixel shader in 5875 at all, so that colour is what the
// texture stage modulates. A directional light projected into SH2 and read
// back at a normal `t = n·l` from it is the Ramamoorthi–Hanrahan irradiance
// with Â = (π, 2π/3, π/4): `E(t) = 0.09375 + 0.5 t + 0.46875 t²`, which is 1.06
// facing the light, 0.09 side-on and 0.06 facing away — the wrap that keeps a
// character's flank from going black, and the reason a model here read
// harsher than the reference's. Clamped at zero where the quadratic dips
// under (t ≈ -0.5).
fn sun_sh_lambert(world_normal: vec3<f32>) -> f32 {
    if lights.n_directional_lights == 0u {
        return 1.0;
    }
    let t = dot(normalize(world_normal), lights.directional_lights[0].direction_to_light);
    return max(0.09375 + 0.5 * t + 0.46875 * t * t, 0.0);
}

// The sun against a surface normal, plus the fill, as a multiplier for the
// texel.
//
// **Clamped, and the clamp is the game's rather than a safety net.** The table's
// noon sun on map 0 is a saturated orange (255/136/0) and its fill a cool blue,
// which sum past 1.0 on any surface facing the sun — and that is the point:
// what saturates is the *warm* channel first, which is what makes lit ground
// read yellow-green where its shadowed side reads blue. Normalising instead of
// clamping would average those two into the flat grey this renderer had.
//
// **And the clamp only means that in the file's own space**, which is the other
// half of why the sum lives here rather than after a decode: it is the *warm*
// channel the client saturates first, and in linear the same sum reaches 0.90
// where the file's reaches 1.16 — so nothing clips at all and the ground never
// goes yellow. `daylight` below is this decoded and is what a surface actually
// multiplies by; `daylight_shadowed` needs the undecoded form.
fn daylight_srgb(world_normal: vec3<f32>) -> vec3<f32> {
    return daylight_srgb_scaled(world_normal, 1.0);
}

// The same sum with the sun term scaled by a per-instance intensity, which is
// the multiplier the client's own `Model2.bls` ends in (`MAD result.color,
// c28[0], R1, c28[1]`): 2.5 for a terrain doodad on lit ground, 0.5 for one
// standing in the ground's baked `MCSH` shadow, 1.0 for everything else — the
// values are the client's; see `models::sun_scale`.
//
// **The scale multiplies the sun and never the fill, and it is applied before
// the clamp, inside the byte-space sum.** Clamping `sun × scale` on its own
// first would bleach a warm dawn sun to white before it is used; scaling the
// fill too would blow out the anti-sun side, which is exactly the side that
// keeps a boosted tree readable. What saturates under the ×2.5 boost is the
// warm channel of the lit side — the same saturation `daylight`'s own clamp
// exists to produce, only reached sooner.
fn daylight_srgb_scaled(world_normal: vec3<f32>, sun_scale: f32) -> vec3<f32> {
    return daylight_srgb_lamplit(world_normal, sun_scale, vec3<f32>(0.0));
}

// …and the same sum with **the lamps standing near this fragment added into
// it**, which is `render::lamps`' whole contribution to the picture.
//
// It goes *inside* the clamp and beside the fill rather than being added to the
// finished colour, because that is what a light is: a torch three yards away
// and the sun are two terms of one sum, and the surface saturates on whichever
// of them arrives first. Adding it afterwards would let a lamp brighten a
// surface that is already white.
//
// **`lamps` is in the file's own space like every other term here** — see
// `render::lamps::LampLight`, which builds the `PointLight` colour as a
// `Color::LinearRgba` container so that Bevy's own extraction hands the shader
// the bytes unchanged. The same device `render::sky` uses for the sun and the
// fill, for the same reason.
//
// At `lamps == 0` this is bit-for-bit `daylight_srgb_scaled`'s old body, which
// is what keeps every daylight measurement in this project standing.
fn daylight_srgb_lamplit(
    world_normal: vec3<f32>,
    sun_scale: f32,
    lamps: vec3<f32>,
) -> vec3<f32> {
    if lights.n_directional_lights == 0u {
        return vec3<f32>(1.0);
    }
    return clamp(
        lights.ambient_color.rgb
            + lights.directional_lights[0].color.rgb * (sun_lambert(world_normal) * sun_scale)
            + lamps,
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
}

fn daylight(world_normal: vec3<f32>) -> vec3<f32> {
    return srgb_to_linear(daylight_srgb(world_normal));
}

fn daylight_scaled(world_normal: vec3<f32>, sun_scale: f32) -> vec3<f32> {
    return srgb_to_linear(daylight_srgb_scaled(world_normal, sun_scale));
}

// …and what a model actually multiplies its texel by, lamps included. See
// `daylight_srgb_lamplit`.
fn daylight_scaled_lamplit(
    world_normal: vec3<f32>,
    sun_scale: f32,
    lamps: vec3<f32>,
) -> vec3<f32> {
    return srgb_to_linear(daylight_srgb_lamplit(world_normal, sun_scale, lamps));
}

// The same sum with the model program's SH sun in place of the clamped
// cosine — see `sun_sh_lambert`. What an M2 multiplies its texel by.
fn daylight_scaled_lamplit_sh(
    world_normal: vec3<f32>,
    sun_scale: f32,
    lamps: vec3<f32>,
) -> vec3<f32> {
    if lights.n_directional_lights == 0u {
        return vec3<f32>(1.0);
    }
    return srgb_to_linear(clamp(
        lights.ambient_color.rgb
            + lights.directional_lights[0].color.rgb * (sun_sh_lambert(world_normal) * sun_scale)
            + lamps,
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    ));
}

// How much light a fully shadowed ground texel loses: the `0.30000001` literal
// in `terrain1.bls`, quoted below.
const MCSH_SHADE: f32 = 0.30000001;

// The same sun, dimmed where the ground carries a baked shadow.
//
// **1.12 casts no shadows at runtime and this is not one.** `coverage` is
// `MCSH` — a 64x64 bitmap per terrain chunk, baked by Blizzard's tools where a
// building or a tree stands, riding in the alpha of the same atlas the blend
// weights do (see `vale_assets::adt`).
//
// **It is a flat 30% dimming and nothing else, and that is out of the client's
// own fragment program rather than reasoned from the table.**
// `Shaders\Pixel\terrain1.bls`, whole:
//
//     PARAM c[1] = { { 1, 0.30000001, 0.69999999 } };
//     TEX  R0.w, fragment.texcoord[1], texture[1], 2D;
//     MAD  R0.w, R0, c[0].y, c[0].z;          // shadow * 0.3 + 0.7
//     TEX  R0.xyz, fragment.texcoord[0], texture[0], 2D;
//     MUL  R0.xyz, R0, R0.w;
//     MUL  result.color.xyz, R0, fragment.color.primary;
//
// Its shadow texel is 1 where the ground is *lit*, so the factor runs 0.7..1.0;
// ours is the other way up (`Adt::alpha_atlas` packs 1 = shadowed), hence
// `1 - 0.3 * coverage`. `terrain1_s.bls` is the same expression plus one thing:
// it gates the specular sheen to zero in shadow — see `sun_sheen` below, which
// is that one thing.
//
// **This replaced a mix toward `Light.dbc`'s band 17**, read as "the colour
// ground in shadow is lit by instead of the sun". Every measurement behind that
// reading was true — band 17 is dimmer than the fill in 7 of 7 open-sky lights,
// is cool where the sun is warm, and does close on the fill at night — and the
// conclusion drawn from them was still wrong. A shadow that mixed to
// `51/82/85` at noon was several times too dark and the wrong hue.
//
// **The dimming goes inside the decode, with the sum**, because the client
// applies it to a byte-space product: under a power-law transfer function
// `s2l(tex · f · light)` equals `tex_linear · s2l(light · f)`, but applying `f`
// out here in linear would be a byte-space factor of `f^(1/2.2)` — 0.85 where
// the client says 0.7, which is half the shadow.
//
// Terrain only. A building's own shadow on itself is its baked `MOCV`, and a
// doodad has neither: its sun term is scaled per instance by the `MCSH` bit
// under its origin instead — `daylight_scaled` above, fed from the mesh tag.
fn daylight_shadowed(world_normal: vec3<f32>, coverage: f32) -> vec3<f32> {
    return daylight_shadowed_lamplit(world_normal, coverage, vec3<f32>(0.0));
}

// …and the same with the lamps standing on this patch of ground.
//
// **The baked shadow dims the sun and not the lamp**, which is the one
// arrangement of these three terms that is not arbitrary: `MCSH` records where
// a building or a tree stood between this texel and the *sun*, and it has
// nothing to say about a torch planted underneath it. So the 30% goes on the
// daylight, the lamp is added after it, and the clamp is taken over the sum —
// which is also what keeps a lamp from lifting ground that is already white.
fn daylight_shadowed_lamplit(
    world_normal: vec3<f32>,
    coverage: f32,
    lamps: vec3<f32>,
) -> vec3<f32> {
    return srgb_to_linear(clamp(
        daylight_srgb(world_normal) * (1.0 - MCSH_SHADE * coverage) + lamps,
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    ));
}

// **How shiny the ground is**, and the one constant in this file that is a
// reading rather than a measurement. See `sun_sheen`.
const SHEEN_POWER: f32 = 20.0;

// **The sheen on the ground** — the sun's own highlight, added to the lit
// texel rather than multiplied into it.
//
// This is the whole difference between `terrain1.bls` and `terrain1_s.bls`,
// and the second is short enough to quote whole beside the first (which is
// quoted above `daylight_shadowed`):
//
//     TEX R0, fragment.texcoord[0], texture[0], 2D;      // the ground texel
//     TEX R1.w, fragment.texcoord[1], texture[1], 2D;    // the MCSH shadow
//     MUL R1.x, R0.w, R1.w;                              // gloss * shadow
//     MAD R0.w, R1, c[0].y, c[0].z;                      // shadow * 0.3 + 0.7
//     MUL R1.xyz, R1.x, fragment.color.secondary;        // ...times the sheen
//     MUL R0.xyz, R0, R0.w;
//     MAD result.color.xyz, R0, fragment.color.primary, R1;
//
// Three things are in those seven instructions and all three are the picture:
//
// * **the gloss mask is the ground texture's own alpha** (`R0.w`), which is
//   why the client loads a *different file* under `specular` — see
//   `vale_assets::adt::specular_texture` and `terrain::ground_texture`.
//   `terrain2_s.bls` blends the four layers' RGB**A** together and takes the
//   blended `.w`, so the mask splats exactly as the colour does;
// * **the shadow gates it to zero** rather than dimming it by 30% as it dims
//   the diffuse: there is no sheen at all inside a baked shadow;
// * **it is added after the modulate**, which is what makes it a highlight
//   sitting *on* the ground rather than a brighter ground.
//
// **What is not in them is the colour and the exponent**, because
// `fragment.color.secondary` is fixed-function output: the client's terrain
// has no vertex program at all (there is no `Shaders\Vertex\terrain*.bls` —
// the four `_s` terrain programs are all pixel programs), so
// the specular term is D3D lighting state that no shipped file states.
// This takes it as `Light.dbc`'s band 9 — the sun's halo — at a shininess of
// 20, Blinn `(N·H)^20` with a local viewer. That reading is cited rather than
// measured, and it is the half of this law to distrust. Two things make it plausible rather than merely available: band 9
// is already the only warm near-white in the table (`255/247/222` on map 0 at
// noon against the sun's saturated `255/136/0`), and a reference screenshot
// shows a **cream** highlight down a brown road, which the
// sun's own colour could not produce.
//
// The caller supplies the colour, in the file's own space, because it is
// per-frame data with nowhere else to ride — see `terrain::sheen_tag`. Black
// is off, which is what the switch writes.
fn sun_sheen(world_normal: vec3<f32>, world_position: vec3<f32>, halo: vec3<f32>) -> vec3<f32> {
    if lights.n_directional_lights == 0u {
        return vec3<f32>(0.0);
    }
    let to_light = lights.directional_lights[0].direction_to_light;
    // **A local viewer**, per fragment: the client's is per *vertex*, over a
    // 4-yard MCVT grid, so this is a smoother highlight of the same shape
    // rather than a different one — the same deviation `model_light` makes and
    // for the same reason (this renderer has no fixed-function T&L stage to
    // put it in).
    let to_eye = normalize(view.world_position.xyz - world_position);
    let half = normalize(to_light + to_eye);
    let facing = max(dot(normalize(world_normal), half), 0.0);
    return halo * pow(facing, SHEEN_POWER);
}

// Fade a fragment toward the fog colour with distance from the camera.
//
// **Linear start/end, which is the falloff this game's own table states** — two
// distances per light, not a density — so the exponential curves Bevy also
// offers would be a different fog wearing the same numbers.
//
// The colour it fades to is the sky's own horizon band, so the far edge of the
// terrain does not end, it becomes the sky. That is the whole reason a nine-tile
// world does not look like a nine-tile world.
//
// **And the fade itself is a lerp between two colours, so it is taken in the
// file's space like every other one here** — which costs a round trip through
// the transfer function, because unlike the bands the fragment reaching this
// point has already been shaded and is linear. Worth it: a lerp in linear
// reaches the air's own colour far too early, so the middle distance washes out
// while the near ground stays sharp, and 500 yards of visibility read as about
// 150. `DistanceFog::color` is written undecoded by `crate::sky` to match, and
// `ClearColor` beside it is *not* — that one is a framebuffer value nothing
// mixes, and the swapchain encodes it on the way out.
// **How far into the view a point is — the fog's argument, and it is depth
// rather than distance.** The reference sets `D3DRS_FOGVERTEXMODE = LINEAR`
// and never `RANGEFOGENABLE`, so its fog is Direct3D's per-vertex fog on
// view-space *z*: a point at the edge of a wide view is fogged by how far in
// front of the camera it is, not by how far from it. Measured on the trace
// (rendering facts, *The reference client traced*). Bevy's view looks down
// -z, hence the sign; clamped at the near plane so nothing behind the camera
// un-fogs.
fn fog_depth(world_position: vec4<f32>) -> f32 {
    let in_view = view.view_from_world * vec4<f32>(world_position.xyz, 1.0);
    return max(-in_view.z, 0.0);
}

fn fogged(colour: vec4<f32>, world_position: vec4<f32>) -> vec4<f32> {
// **A view with no fog binding has no fog** — the portrait cameras, and the
// world under `--without fog`. See the note on the imports above.
#ifndef DISTANCE_FOG
    return colour;
#else
    if fog.mode == FOG_MODE_OFF {
        return colour;
    }
    let distance = fog_depth(world_position);
    // The scattering argument is for fog that brightens toward the sun, which
    // needs a sun *disc* to brighten toward; this client has none, and the
    // table's `directional_light_color` alpha is left at zero so the term is
    // skipped rather than fed a made-up direction.
    let mixed = linear_fog(
        fog,
        vec4<f32>(linear_to_srgb(colour.rgb), colour.a),
        distance,
        vec3<f32>(0.0),
    );
    // **The night's own air, over the table's**, and taken here rather than
    // after the decode for the reason every other mix in this file is: it is a
    // lerp between two colours and a lerp does not survive a transfer function.
    // `1.0` is the inscattering switch — see `night_air`.
    let aired = night_air(mixed.rgb, world_position.xyz, 1.0);
    return vec4<f32>(srgb_to_linear(aired), mixed.a);
#endif
}

// The same fade toward **black** instead of the air, for a draw that is *added*
// to the frame — an additive particle. Mixing a glow toward the fog colour does
// not fade it out, it adds the air's own colour to the sky around every distant
// flame; under `(src·α, ONE)` blending the identity is black, exactly as the
// identity for a `modulate` draw is white (see `M2Params.ambient.w`). This is
// the client's own per-blend fog table — the M2 render state fogs additive
// draws to black — measured rather than reasoned about. The same distance and the same
// curve as `fogged`, by construction: the fog uniform is copied with only its
// colour replaced.
fn fogged_to_black(colour: vec4<f32>, world_position: vec4<f32>) -> vec4<f32> {
// The same guard as `fogged`'s, for the same two view populations.
#ifndef DISTANCE_FOG
    return colour;
#else
    if fog.mode == FOG_MODE_OFF {
        return colour;
    }
    var dark = fog;
    dark.base_color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    let distance = fog_depth(world_position);
    let mixed = linear_fog(
        dark,
        vec4<f32>(linear_to_srgb(colour.rgb), colour.a),
        distance,
        vec3<f32>(0.0),
    );
    // **Extinction without inscattering**, which is the same substitution this
    // whole function is: a glow behind a bank of mist is *dimmed* by it, and
    // adding the mist's own colour to an additive draw would put a second copy
    // of the air into the frame for every flame standing in it. `0.0` is what
    // says so; see `night_air`.
    let aired = night_air(mixed.rgb, world_position.xyz, 0.0);
    return vec4<f32>(srgb_to_linear(aired), mixed.a);
#endif
}
