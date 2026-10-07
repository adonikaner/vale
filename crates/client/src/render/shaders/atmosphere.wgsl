#define_import_path vale::atmosphere

// The two things every surface in the world does with the light: the
// daylight sum and the fog. Terrain and models are separate shaders with
// separate bindings, and each used to carry its own copy of the sun, a
// `0.55 + 0.45 * lambert` written out twice with a comment in each saying the
// other had to match. The two could only be compared by looking at a model
// standing on the ground in the running client, so nothing failed when one of
// them changed. Both now import their lighting from this file.
//
// The values come from `Light.dbc` per map and per hour and are carried in
// Bevy's own view bind group, which every material pipeline already has bound
// at group 0. See `crate::sky` for how they get there.
//
// The colours arrive in the file's own space, undecoded. Every colour here is
// combined before it is used (the fill is added to the sun, the shadow band is
// mixed into the sum, the air is mixed into the result), and a sum does not
// survive a transfer function: `srgb_to_linear(a) + srgb_to_linear(b)` is not
// `srgb_to_linear(a + b)`. A product very nearly does, which is why the texel
// can stay linear and only the sums have to be taken in the file's space.
//
// Measured on map 0 at 10:26, with the fill at 102/129/155 and the sun at
// 255/128/0, on ground facing 0.76 into the sun: summed in the file's space
// the ground is lit by `255/227/155`, and summed in linear it is lit by
// `243/167/155`. The difference in the green channel is the difference between
// grass and rust.
//
// The 1.12 client is fixed-function, and the only gamma in it is
// `SetDeviceGammaRamp`, the display slider, which the driver applies to the
// finished frame. There is no sRGB sampler state, no linear working space and
// no display transform, so every colour the client adds together it adds as the
// bytes the file states. This file does the same and decodes once, at the end.

#import bevy_pbr::mesh_view_bindings::{lights, view}
// The fog binding exists only for a view whose camera carries `DistanceFog`.
// `mesh_view_bindings.wgsl` gates `@binding(13) fog` behind `#ifdef
// DISTANCE_FOG`, and two populations of views lack the component: the
// portrait cameras (`render::portraits`, nine of them), and the world camera
// itself under `--without fog`, which removes the component rather than
// pushing the falloff out (see `render::sky` for why). An unguarded import
// fails pipeline specialization for every one of those views: for the world
// camera the world is not drawn, and a portrait's face never draws.
#ifdef DISTANCE_FOG
#import bevy_pbr::mesh_view_bindings::fog
#import bevy_pbr::mesh_view_types::FOG_MODE_OFF
#import bevy_pbr::fog::linear_fog
#endif

// The two transfer functions this file uses are in `gamma.wgsl`. naga_oil does
// not strip an imported module's unused functions, so anything importing
// `srgb_to_linear` from this file would also pull in `fogged` and `daylight`
// and with them `bevy_pbr::mesh_view_bindings`, bindings a 2D pipeline does not
// have. `present.wgsl` is such an importer. `to_frame`, the conversion a
// fragment applies before handing its colour to the framebuffer (where the
// blend applies the same rule as this file), is in `gamma.wgsl` for the same
// reason.
//
// The deep night is not in this file; it is in `night.wgsl`, imported below.
// It is a deviation from the game rather than a reproduction of it, and this
// file contains only the reproduction.
//
// The one light that cannot be decoded on the CPU is handled here: a room's
// light is `MOHD`'s ambient plus a `MOCV` vertex colour, and the sum has to be
// taken in the file's own space before it is decoded. The client does that
// arithmetic in bytes, and `WmoGroup::shaded_colours` subtracts the ambient
// back out in bytes too, so the decode has to come after the add.
//
// The decode is required. The model and terrain textures are uploaded as
// `Rgba8UnormSrgb`, so `textureSample` already returns linear values. A linear
// texel times an sRGB-encoded light gives the wrong hue relationships:
// mid-tones lifted and contrast flattened. When interiors were the one surface
// still lit in gamma space, a tavern read as a flat, bright cut-out against
// the rest of the world.
#import vale::gamma::{srgb_to_linear, linear_to_srgb}
// The deep night, which is not a reproduction of the 1.12 client and is
// therefore a separate module.
//
// `night.wgsl` is a grade and a mist that 1.12 does not have and that no file,
// shader or behaviour of the client states. Everything else in this file
// reproduces what the 5875 client draws; the deep night deviates from it, and
// the two are different kinds of claim. See `render::night`, which owns the
// option.
//
// It hooks into this file at three places: the three `*_lamplit` entry points
// below take a colour it computed and add it to the sum, and
// `fogged`/`fogged_to_black` hand their result to `night_air` while it is
// still in the file's own space. At a night factor of zero each of them is the
// identity, which `render::night`'s own tests check.
#import vale::night::night_air

// How square-on to the sun a surface is, 0..1.
//
// Exposed on its own because a WMO's interior lighting needs the direction of
// the sun without its colour: a room is lit by its own baked `MOCV` colours and
// then leaned along this, so a wall facing the sky is not painted as flat as the
// floor. Taking it from here rather than recomputing it keeps an interior's
// lean and the outdoor lighting on the same sun direction.
//
// Every entry point checks the light count, because the glue screens run
// before a sun is spawned and an unwritten light slot is not zero: it holds
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

// The sun term of the 1.12.1 client's OpenGL path, which the model shader does
// not use. On Direct3D, the path every reference picture is of, the client
// creates no vertex shader and lights a model with fixed-function
// `max(N·L, 0)`; `m2.wgsl` takes `daylight_scaled_lamplit` above for that.
// This function is what `Model2.bls`'s ARB program computes, kept for an A/B
// comparison.
//
// `Model2.bls` does not use a clamped cosine but the second-order
// spherical-harmonic projection of one. The lit permutation of the model
// vertex program evaluates a nine-coefficient SH (`c10[0..6]`: the linear terms
// against the normal, the quadratic against its products, and the `x²-y²`
// term) and writes `c28[0] * that + c28[1]` to `result.color.front.primary`.
// There is no model pixel shader in 5875, so that colour is what the texture
// stage modulates. A directional light projected into SH2 and read back at a
// normal `t = n·l` from it is the Ramamoorthi–Hanrahan irradiance with
// Â = (π, 2π/3, π/4): `E(t) = 0.09375 + 0.5 t + 0.46875 t²`, which is 1.06
// facing the light, 0.09 side-on and 0.06 facing away. This wrap keeps a
// character's flank from going black. Clamped at zero where the quadratic dips
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
// The clamp reproduces the game's saturation. The table's noon sun on map 0 is
// a saturated orange (255/136/0) and its fill a cool blue, which sum past 1.0
// on any surface facing the sun. The warm channel saturates first, which makes
// lit ground read yellow-green where its shadowed side reads blue. Normalising
// instead of clamping would average the two into a flat grey.
//
// The clamp has that effect only in the file's own space, which is the second
// reason the sum is taken here rather than after a decode. In linear the same
// sum reaches 0.90 where the file's reaches 1.16, so nothing clips and the
// ground does not turn yellow. `daylight` below is this sum decoded and is what
// a surface multiplies by; `daylight_shadowed` needs the undecoded form.
fn daylight_srgb(world_normal: vec3<f32>) -> vec3<f32> {
    return daylight_srgb_scaled(world_normal, 1.0);
}

// The same sum with the sun term scaled by a per-instance factor: the
// multiplier the 1.12.1 client applies to a model's sun colour. It is 2.5 for
// an entity (unit, player, game object) on lit ground or standing on a
// building outdoors, 0.5 for an entity or a doodad over the ground's baked
// `MCSH` shadow, and 1.0 for a doodad on lit ground, terrain `MDDF` doodads and
// exterior WMO props alike. See `models::sun_scale`.
//
// The scale multiplies the sun and never the fill, and it is applied inside
// the byte-space sum, before the clamp. Clamping `sun × scale` on its own first
// would turn a warm low sun white before it is used. Scaling the fill too
// would overexpose the side facing away from the sun, which the fill alone
// lights. What saturates under ×2.5 is the warm channel of a unit's lit side:
// the same saturation `daylight`'s own clamp produces, reached sooner.
fn daylight_srgb_scaled(world_normal: vec3<f32>, sun_scale: f32) -> vec3<f32> {
    return daylight_srgb_lamplit(world_normal, sun_scale, vec3<f32>(0.0));
}

// The same sum with the lamps standing near this fragment added into it. This
// is the only way `render::lamps` affects the picture.
//
// The lamps go inside the clamp, beside the fill, rather than being added to
// the finished colour: a torch three yards away and the sun are two terms of
// one sum, and the surface saturates on whichever of them arrives first.
// Adding the lamps afterwards would let a lamp brighten a surface that is
// already white.
//
// `lamps` is in the file's own space like every other term here. See
// `render::lamps::LampLight`, which builds the `PointLight` colour as a
// `Color::LinearRgba` container so that Bevy's own extraction hands the shader
// the bytes unchanged. `render::sky` does the same for the sun and the fill,
// for the same reason.
//
// At `lamps == 0` this is bit-for-bit the body `daylight_srgb_scaled` had
// before the lamps were added, so every daylight measurement taken before then
// still applies.
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

// What a model multiplies its texel by, lamps included. See
// `daylight_srgb_lamplit`.
fn daylight_scaled_lamplit(
    world_normal: vec3<f32>,
    sun_scale: f32,
    lamps: vec3<f32>,
) -> vec3<f32> {
    return srgb_to_linear(daylight_srgb_lamplit(world_normal, sun_scale, lamps));
}

// The same sum with the model program's SH sun in place of the clamped
// cosine; see `sun_sh_lambert`. The OpenGL path's model lighting, kept for an
// A/B comparison.
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
// 1.12 casts no shadows at run time and this is not one. `coverage` is `MCSH`:
// a 64x64 bitmap per terrain chunk, baked by Blizzard's tools where a building
// or a tree stands, carried in the alpha of the same atlas as the blend
// weights (see `vale_assets::adt`).
//
// It is a flat 30% dimming and nothing else, as stated by the game's terrain
// pixel program, `Shaders\Pixel\terrain1.bls`, in full:
//
//     PARAM c[1] = { { 1, 0.30000001, 0.69999999 } };
//     TEX  R0.w, fragment.texcoord[1], texture[1], 2D;
//     MAD  R0.w, R0, c[0].y, c[0].z;          // shadow * 0.3 + 0.7
//     TEX  R0.xyz, fragment.texcoord[0], texture[0], 2D;
//     MUL  R0.xyz, R0, R0.w;
//     MUL  result.color.xyz, R0, fragment.color.primary;
//
// Its shadow texel is 1 where the ground is lit, so the factor runs 0.7..1.0.
// This project's is the other way up (`Adt::alpha_atlas` packs 1 = shadowed),
// hence `1 - 0.3 * coverage`. `terrain1_s.bls` is the same expression plus one
// addition: it gates the specular sheen to zero in shadow. See `sun_sheen`
// below.
//
// This replaced a mix toward `Light.dbc`'s band 17, read as "the colour ground
// in shadow is lit by instead of the sun". Band 17 is dimmer than the fill in 7
// of 7 open-sky lights, is cool where the sun is warm, and closes on the fill
// at night, but it is not the shadow colour: a shadow that mixed to `51/82/85`
// at noon was several times too dark and the wrong hue.
//
// The dimming goes inside the decode, with the sum, because the client applies
// it to a byte-space product. Under a power-law transfer function
// `s2l(tex · f · light)` equals `tex_linear · s2l(light · f)`, but applying `f`
// after the decode, in linear, would be a byte-space factor of `f^(1/2.2)`:
// 0.85 where the client uses 0.7, which is half the shadow.
//
// Terrain only. A building's own shadow on itself is in its baked `MOCV`. A
// doodad has neither: its sun term is scaled per instance by the `MCSH` bit
// under its origin instead, and an entity's by the bit under its position,
// through `daylight_scaled` above, fed from the mesh tag.
fn daylight_shadowed(world_normal: vec3<f32>, coverage: f32) -> vec3<f32> {
    return daylight_shadowed_lamplit(world_normal, coverage, vec3<f32>(0.0));
}

// The same, with the lamps standing on this patch of ground.
//
// The baked shadow dims the sun and not the lamp. `MCSH` records where a
// building or a tree stood between this texel and the sun, and says nothing
// about a torch underneath it. So the 30% goes on the daylight, the lamp is
// added after it, and the clamp is taken over the sum, which also keeps a lamp
// from lifting ground that is already white.
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

// The specular exponent of the ground. The one constant in this file that is
// chosen rather than measured; see `sun_sheen`.
const SHEEN_POWER: f32 = 20.0;

// The sheen on the ground: the sun's highlight, added to the lit texel rather
// than multiplied into it.
//
// This is the difference between `terrain1.bls` and `terrain1_s.bls`. The
// second, in full (the first is quoted above `daylight_shadowed`):
//
//     TEX R0, fragment.texcoord[0], texture[0], 2D;      // the ground texel
//     TEX R1.w, fragment.texcoord[1], texture[1], 2D;    // the MCSH shadow
//     MUL R1.x, R0.w, R1.w;                              // gloss * shadow
//     MAD R0.w, R1, c[0].y, c[0].z;                      // shadow * 0.3 + 0.7
//     MUL R1.xyz, R1.x, fragment.color.secondary;        // ...times the sheen
//     MUL R0.xyz, R0, R0.w;
//     MAD result.color.xyz, R0, fragment.color.primary, R1;
//
// It states three things:
//
// * The gloss mask is the ground texture's own alpha (`R0.w`), which is why
//   the client loads a different file under `specular`; see
//   `vale_assets::adt::specular_texture` and `terrain::ground_texture`.
//   `terrain2_s.bls` blends the four layers' RGBA together and takes the
//   blended `.w`, so the mask is splatted the same way as the colour.
// * The shadow gates the sheen to zero rather than dimming it by 30% as it
//   dims the diffuse: there is no sheen inside a baked shadow.
// * The sheen is added after the modulate, so it is a highlight on the ground
//   rather than a brighter ground.
//
// The program does not state the colour or the exponent, because
// `fragment.color.secondary` is fixed-function output. The client's terrain
// has no vertex program (there is no `Shaders\Vertex\terrain*.bls`; the four
// `_s` terrain programs are all pixel programs), so the specular term is
// Direct3D lighting state that no shipped file states. This function takes the
// colour as `Light.dbc`'s band 9, the sun's halo, at a shininess of 20: Blinn
// `(N·H)^20` with a local viewer. That choice is cited rather than measured and
// is the least certain part of this function. Two things support it: band 9 is
// the only warm near-white in the table (`255/247/222` on map 0 at noon,
// against the sun's saturated `255/136/0`), and a reference screenshot shows a
// cream highlight down a brown road, which the sun's own colour could not
// produce.
//
// The caller supplies the colour, in the file's own space, because it is
// per-frame data with no other channel to the shader; see `terrain::sheen_tag`.
// Black is off, which is what the switch writes.
fn sun_sheen(world_normal: vec3<f32>, world_position: vec3<f32>, halo: vec3<f32>) -> vec3<f32> {
    if lights.n_directional_lights == 0u {
        return vec3<f32>(0.0);
    }
    let to_light = lights.directional_lights[0].direction_to_light;
    // A local viewer, per fragment. The client's is per vertex, over a 4-yard
    // MCVT grid, so this is a smoother highlight of the same shape. It is the
    // same deviation `model_light` makes, for the same reason: this renderer
    // has no fixed-function T&L stage to put it in.
    // An orthographic view has no eye point: every view ray is parallel to the
    // camera's forward axis, so the direction to the viewer is the camera's
    // back axis (the third column of `world_from_view`). Using the camera's
    // position there gives a direction that depends on where the camera sits
    // rather than on the view. The editor's top-down map view is orthographic.
    let orthographic = view.clip_from_view[3].w == 1.0;
    var to_eye = normalize(view.world_position.xyz - world_position);
    if orthographic {
        to_eye = normalize(view.world_from_view[2].xyz);
    }
    let half = normalize(to_light + to_eye);
    let facing = max(dot(normalize(world_normal), half), 0.0);
    return halo * pow(facing, SHEEN_POWER);
}

// Fade a fragment toward the fog colour with distance from the camera.
//
// Linear start and end, which is the falloff this game's own table states: two
// distances per light, not a density. The exponential curves Bevy also offers
// would be a different fog using the same numbers.
//
// The colour it fades to is the sky's own horizon band, so the far edge of the
// terrain blends into the sky instead of ending. This is what hides the edge of
// the nine loaded tiles.
//
// The fade is a lerp between two colours, so it is taken in the file's space
// like every other mix here. That costs a round trip through the transfer
// function, because the fragment reaching this point has already been shaded
// and is linear. A lerp in linear reaches the air's own colour too early: the
// middle distance washes out while the near ground stays sharp, and 500 yards
// of visibility read as about 150. `crate::sky` writes `DistanceFog::color`
// undecoded to match. It does not write `ClearColor` undecoded: that is a
// framebuffer value nothing mixes, and the swapchain encodes it on the way out.
// How far into the view a point is: the fog's argument, which is depth rather
// than distance. The 1.12.1 client uses Direct3D's linear per-vertex fog
// (`D3DRS_FOGVERTEXMODE = LINEAR`) without range-based fog
// (`D3DRS_RANGEFOGENABLE`), so its fog is computed on view-space z: a point at
// the edge of a wide view is fogged by how far in front of the camera it is,
// not by how far from it. Bevy's view looks down -z, hence the sign; clamped at
// the near plane so nothing behind the camera loses its fog.
fn fog_depth(world_position: vec4<f32>) -> f32 {
    let in_view = view.view_from_world * vec4<f32>(world_position.xyz, 1.0);
    return max(-in_view.z, 0.0);
}

fn fogged(colour: vec4<f32>, world_position: vec4<f32>) -> vec4<f32> {
// A view with no fog binding has no fog: the portrait cameras, and the world
// under `--without fog`. See the note on the imports above.
#ifndef DISTANCE_FOG
    return colour;
#else
    if fog.mode == FOG_MODE_OFF {
        return colour;
    }
    let distance = fog_depth(world_position);
    // The scattering argument is for fog that brightens toward the sun, which
    // needs a sun disc to brighten toward. This client draws none, and the
    // table's `directional_light_color` alpha is left at zero so the term is
    // skipped rather than given an invented direction.
    let mixed = linear_fog(
        fog,
        vec4<f32>(linear_to_srgb(colour.rgb), colour.a),
        distance,
        vec3<f32>(0.0),
    );
    // The night's own air, over the table's, taken here rather than after the
    // decode for the same reason as every other mix in this file: it is a lerp
    // between two colours, and a lerp does not survive a transfer function.
    // `1.0` is the inscattering switch; see `night_air`.
    let aired = night_air(mixed.rgb, world_position.xyz, 1.0);
    return vec4<f32>(srgb_to_linear(aired), mixed.a);
#endif
}

// The same fade toward black instead of the air, for a draw that is added to
// the frame: an additive particle. Mixing a glow toward the fog colour does not
// fade it out; it adds the air's own colour to the sky around every distant
// flame. Under `(src·α, ONE)` blending the identity is black, as the identity
// for a `modulate` draw is white (see `M2Params.ambient.w`). The 1.12.1 client
// fogs additive M2 draws toward black. The distance and the curve are the same
// as `fogged`'s by construction: the fog uniform is copied with only its
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
    // Extinction without inscattering, the same substitution this function
    // makes for the fog colour: a glow behind a bank of mist is dimmed by it,
    // and adding the mist's own colour to an additive draw would add a second
    // copy of the air to the frame for every flame standing in it. `0.0`
    // selects this; see `night_air`.
    let aired = night_air(mixed.rgb, world_position.xyz, 0.0);
    return vec4<f32>(srgb_to_linear(aired), mixed.a);
#endif
}
