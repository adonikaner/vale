#define_import_path vale::night

// **The deep night — the one thing in this renderer that is not a reading of
// anything.**
//
// Every other shader in this directory is an attempt to draw what the 5875
// client drew, checked against the game's own `.bls` programs or against
// the 1.12.1 client. This one is a deliberate deviation: 1.12 has no volumetric
// anything and no local lights at all, and nothing here is claimed to be the
// game's. It is stated first so that nobody goes looking for the authority
// behind it, and it is a file of its own for the same reason — it had grown to
// nearly half of `atmosphere.wgsl`, whose entire doc-comment is an argument
// about fidelity, and a deviation sitting inside that argument reads as part of
// it.
//
// `crate::render::night` carries the whole design. What is here is the arithmetic, in two functions:
//
// * `night_air`  — the mist a fragment is seen through, and what it glows with.
// * `lamplight`  — what the things that burn near it add to the light on it.
//
// **Both are off by one uniform-coherent branch when the night factor is zero**,
// which is every fragment of every daylight frame. See `NightAir::pack` for
// where that factor rides and why it is free.
//
// Nothing in this file depends on `atmosphere.wgsl`; the dependency runs the
// other way, which is what made the split clean.

#import bevy_pbr::mesh_view_bindings::{lights, view}
// **The fog binding, and the same guard `atmosphere.wgsl` takes on it.** Two
// view populations legitimately lack `DistanceFog` — the portrait cameras, and
// the world under `--without fog` — and for both of them the night is off
// rather than absent: see each function's own `#ifndef`.
#ifdef DISTANCE_FOG
#import bevy_pbr::mesh_view_bindings::fog
#endif


// **The cluster, which is how a fragment finds the lamps near it.** Bindings 8,
// 9 and 10 of group 0, and unlike the fog they are *not* behind an `#ifdef`:
// every view in this client has them, including the portrait cameras. Bevy
// assigns each `PointLight` to the screen-space cells its sphere touches on the
// CPU, so what is left here is a walk over one cell's list — see `lamplight`,
// and `render::lamps` for where the lights come from and why there are never
// many.
#import bevy_pbr::clustered_forward as clustering
#import bevy_pbr::mesh_view_bindings::clustered_lights

// **The deep night's own air** — this renderer's one deviation from the game,
// and the only thing in this file that is not a reading of something.
//
// `crate::render::night` carries the whole argument for why it exists and what
// each of its five numbers is; what is here is the arithmetic, and three things
// about it are worth stating where the instructions are.
//
// **1. It is analytic, not marched.** The density of the mist at height `h` is
// `rho0 * exp(-(h - deck) / H)` — an exponential deck sitting on the ground the
// player is standing on. The light lost between the eye and a fragment is the
// integral of that along the segment between them, and the integral has a
// closed form:
//
//     tau = rho0 * ke * H * (L / dz) * (1 - exp(-dz / H)),   ke = exp(-(eye - deck)/H)
//
// with `L` the distance, `dz` the rise, and the `dz -> 0` limit `rho0 * ke * L`
// taken separately because the quotient is 0/0 there. That is two `exp`s and
// about twenty other instructions in a fragment shader that was already
// sampling four textures; there is no second pass, no volume texture and no
// per-frame allocation anywhere in it. **A raymarch was never considered** —
// the whole point of the option is that it costs nothing measurable, and
// sixteen steps per fragment over a full screen is not that.
//
// `tau` is positive in both directions and the sign works out without a branch:
// looking up, `dz > 0` makes `(1 - exp(-dz/H))` positive and `L/dz` positive;
// looking down, both are negative.
//
// **2. Both ceilings are load-bearing rather than defensive.** `ke` is
// `exp(+something)` whenever the eye is below the deck, which happens every time
// the character walks downhill between two resolves, and an unclamped one
// multiplies the whole integral by `e^40`. `tau` is clamped for the same reason
// one step further on: `exp(-tau)` underflows to zero long before `tau` reaches
// the values a steep downhill ray produces, and clamping it is what stops a
// NaN reaching the framebuffer on the frame the camera drops into a gully.
//
// **3. What the mist is lit by is the horizon band plus the moon.** The band is
// `fog.base_color`, which `render::night` has already graded — so the air the
// world fades into and the air standing in front of it are the same colour by
// construction, exactly as the dome's bottom stop and the fog colour are the
// same band. The moon term is a phase function: the more directly the ray looks
// into `directional_lights[0]`, the more of that light's own colour the mist
// gives back. It is what makes a bank of it read as *volume* rather than as a
// grey wash, and it is one `dot` and one `pow`.
//
// `inscatter` is 1 for a surface and 0 for an additive draw. See the two
// callers.
//
// **Defined for every view, and guarded inside** — the shape `lamplight` below
// already had, and the one that matters here for a reason that is not
// symmetry. Wrapping the whole function in `#ifdef DISTANCE_FOG` makes
// `vale::night::night_air` a name that does not exist in the portrait
// cameras' permutation, and an import naga_oil cannot resolve is not an error:
// the pipeline simply stays pending for ever, so the face never draws and
// nothing appears in the log. See `present.wgsl`, which carries the same
// warning about the same failure.

// How concentrated the glow toward the moon is. Higher is tighter. See
// `render::night`'s `GLOW`, which is the other half of it: at 8 the lobe
// covered most of the sky, which at any hour the table lights warm reads as an
// orange wash rather than as a halo around a moon.
const NIGHT_GLOW_POWER: f32 = 16.0;

// The most the density at the eye may exceed the density at the deck. See note
// 2 above: the eye is below the deck for as long as it takes the light to
// resolve again, which is every four yards of downhill travel.
const NIGHT_DECK_CEILING: f32 = 4.0;

// …and the most optical depth any one ray may accumulate. `exp(-16)` is 1e-7,
// which is opaque in eight bits several times over.
const NIGHT_DEPTH_CEILING: f32 = 16.0;

fn night_air(colour: vec3<f32>, world_position: vec3<f32>, inscatter: f32) -> vec3<f32> {
// **A view with no fog binding has no night** — the portrait cameras, and the
// world under `--without fog`. The parameters ride in that uniform, so without
// it there is nothing to read.
#ifndef DISTANCE_FOG
    return colour;
#else
    // (night, density at the deck, 1/e height, glow) — see `NightAir::pack`,
    // which is the one place this reinterpretation of Bevy's own scattering
    // fields is written down.
    let air = fog.directional_light_color;
    // **The daylight early-out**, and the reason the option is free when it is
    // not doing anything: `night` is a uniform, so this branch is coherent
    // across the whole draw rather than across a warp.
    if air.x <= 0.0 {
        return colour;
    }
    let eye = view.world_position.xyz;
    let ray = world_position - eye;
    let travelled = length(ray);
    if travelled <= 0.0 {
        return colour;
    }
    // `directional_light_exponent`, which under `FogFalloff::Linear` Bevy's own
    // `linear_fog` never reads — see `NightAir::deck`. World Z is Bevy Y
    // unchanged (`render::axes`), so nothing is converted here.
    let deck = fog.directional_light_exponent;
    let height = max(air.z, 1.0);
    let at_eye = min(exp(-(eye.y - deck) / height), NIGHT_DECK_CEILING);
    let rise = ray.y;
    var depth: f32;
    if abs(rise) < 1e-3 {
        // The level-ray limit of the expression below, taken separately because
        // the quotient is 0/0 rather than merely imprecise there.
        depth = air.y * at_eye * travelled;
    } else {
        depth = air.y * at_eye * height * (travelled / rise) * (1.0 - exp(-rise / height));
    }
    let through = exp(-clamp(depth, 0.0, NIGHT_DEPTH_CEILING));
    var lit = fog.base_color.rgb;
    if lights.n_directional_lights > 0u {
        let toward = max(
            dot(ray / travelled, lights.directional_lights[0].direction_to_light),
            0.0,
        );
        lit += lights.directional_lights[0].color.rgb
            * (pow(toward, NIGHT_GLOW_POWER) * air.w);
    }
    return mix(colour, lit * inscatter, 1.0 - through);
#endif
}

// **What the lamps standing near this fragment add to it**, in the file's own
// space, or black when there are none — which in daylight is every fragment,
// because `render::lamps` spawns no light at all above the night ramp.
//
// The cost is a walk over one cluster's point-light list. Bevy has already
// bounded that: a cluster is a screen-space cell of a frustum slice, and a
// light only appears in the cells its own sphere touches, so a fragment with no
// lamp near it reads two `u32`s and leaves. That is the whole reason this is
// worth doing at all in a client that draws thirty thousand meshes a frame.
//
// **The falloff is not inverse-square and that is deliberate.** A physical
// point light in yards is either blinding at one yard or invisible at ten;
// what a torch has to do here is light a readable patch of ground and stop, so
// this is `(1 - d/range)^2` — the light's own value at its centre, zero at its
// range, and smooth in between. `range` is `PointLight::range` unchanged, so
// the sphere Bevy clustered against and the sphere this draws are the same one
// and a lamp cannot light a fragment outside the cell it was assigned to.
//
// **And the diffuse term is wrapped — a little.** A hard `max(dot(n, l), 0)`
// terminator on this game's low-polygon geometry reads as a crease rather than
// as a curve, and a torch is a small source in a dark room where the bounce
// this client does not simulate would be most of what you see. `LAMP_WRAP` is
// that bounce, flat and cheap.
//
// **It was 0.35 and that was most of the wash.** A wrap term is light with no
// direction in it: at a third of full strength every surface within a lamp's
// reach — walls facing away, the underside of a cart, the shaded half of a
// tree — came out lit, and a light that falls on everything equally is
// indistinguishable from raising the ambient. What a torch has to do here is
// pick things *out* of the dark, so the bounce is small and the terminator is
// most of the term.
const LAMP_WRAP: f32 = 0.10;

fn lamplight(
    world_position: vec4<f32>,
    world_normal: vec3<f32>,
    frag_position: vec4<f32>,
) -> vec3<f32> {
// **A view with no fog binding has no night and therefore no lamps** — the
// portrait cameras, and the world under `--without fog`. The same guard
// `fogged` takes and for the same reason; see the note on the imports.
#ifndef DISTANCE_FOG
    return vec3<f32>(0.0);
#else
    // **The daylight early-out, and it is not merely tidiness.** Without it
    // every world fragment in every frame reads two words out of the cluster
    // buffer to discover that the list is empty, which is a cost the option
    // would be charging a noon session for. `night` is a uniform, so the branch
    // is coherent across the whole draw. See `NightAir::pack` for what is in
    // this field.
    if fog.directional_light_color.x <= 0.0 {
        return vec3<f32>(0.0);
    }
    let view_z = dot(
        vec4<f32>(
            view.view_from_world[0].z,
            view.view_from_world[1].z,
            view.view_from_world[2].z,
            view.view_from_world[3].z,
        ),
        world_position,
    );
    let is_orthographic = view.clip_from_view[3].w == 1.0;
    let cell = clustering::view_fragment_cluster_index(
        frag_position.xy,
        view_z,
        is_orthographic,
    );
    let ranges = clustering::unpack_clusterable_object_index_ranges(cell);
    var sum = vec3<f32>(0.0);
    let normal = normalize(world_normal);
    for (
        var i = ranges.first_point_light_index_offset;
        i < ranges.first_spot_light_index_offset;
        i = i + 1u
    ) {
        let lamp = clustered_lights.data[clustering::get_clusterable_object_id(i)];
        let toward = lamp.position_radius.xyz - world_position.xyz;
        let distance = length(toward);
        let reach = max(lamp.range, 1e-3);
        let edge = max(1.0 - distance / reach, 0.0);
        // **`smoothstep`, not `edge^2`, and the difference is the whole
        // picture.** A squared falloff is at a quarter of its value half way
        // out, so a torch draws a small bright core and nothing else — which is
        // what the first draft of this looked like. This is flat near the
        // source and steep at the rim, which is how a pool of lamplight on a
        // floor actually reads.
        let fall = edge * edge * (3.0 - 2.0 * edge);
        let facing = max(dot(normal, toward / max(distance, 1e-4)), 0.0);
        sum += lamp.color_inverse_square_range.rgb * (fall * mix(LAMP_WRAP, 1.0, facing));
    }
    return sum;
#endif
}
