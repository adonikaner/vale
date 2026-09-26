// **The rain, the snow and the sand, moved on the GPU.** One quad per
// particle in a mesh built once per weather; where each one is this frame,
// what shape it takes and how faded it is are decided here from five vec4s
// of parameters and the view. See `render::weather`, which states every
// rule below beside where it was measured — this file is the arithmetic.
//
// ## The motion
//
// A particle is world-anchored and lives in a torus: a box centred on the
// eye whose faces wrap. Its world position is `seed + velocity * pace * s +
// W(t)`, where `seed` is a random point hashed from its index and the cycle
// of its life it is in (so a rebirth lands anywhere, as the reference's
// respawn does), `s` is the seconds since that birth, `pace` the density's
// fall multiplier and `W(t)` the running integral of the shared wind — kept
// on the CPU and wrapped to the box, which is exact under a torus. What is
// drawn is that position relative to the eye, wrapped into the box, and the
// mesh sits at the eye so `world = eye + rel`.
//
// ## The shapes
//
// `rain.bls` builds a streak along the drop's own fall, turned about it to
// face the eye; `snowpoint.bls` and `sand.bls` draw point sprites whose size
// is pixels falling off with distance; the mist is the one layer sized in
// yards. The corner a vertex is comes from its uv: `(0,1) (1,1) (1,0) (0,0)`
// are the base pair and the trailing pair of a streak, and the four corners
// of a square.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
}
#import vale::atmosphere::fogged
#import vale::gamma::{srgb_to_linear, linear_to_srgb, to_frame}

struct WeatherParams {
    // half_xy, half_z, alive, pace
    half: vec4<f32>,
    // the integral of the wind, wrapped to this box (x, 0, z); w = time
    wind: vec4<f32>,
    // the wind this second (the streak's slant); w = the point size's fade end
    gust: vec4<f32>,
    tint: vec4<f32>,
    // kind (0 world, 1 point, 2 streak); a streak's half-width or a point
    // sprite's floor in pixels; a streak's yards of fall; the near fade
    // distance
    shape: vec4<f32>,
    // One bit per particle: standing inside a building's room, so it falls
    // unseen. Written by the CPU a quarter of the pool a frame; word `i` is
    // component `i % 4` of element `i / 4`.
    mask: array<vec4<u32>, 72>,
}

// **A streak is never thinner than a pixel and a half**, whatever its
// width in yards — the streak's own form of `snowpoint.bls`'s `MAX
// result.pointsize, 1`. A centimetre is under a pixel from ten yards, and a
// quad under a pixel wide covers no sample on most frames, so most of the
// rain was never rasterised at all.
const STREAK_MIN_HALF_PIXELS: f32 = 0.75;

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: WeatherParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var sprite: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var sprite_sampler: sampler;

// The same generator `render::weather::pcg` is, bit for bit, so the CPU's
// room test lands on the position that is drawn.
fn pcg(x: u32) -> u32 {
    let h = x * 747796405u + 2891336453u;
    let w = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (w >> 22u) ^ w;
}

fn unit(x: u32) -> f32 {
    return f32(pcg(x) & 0xffffffu) / 16777216.0;
}

// Into `[-half, half)`, however far out — `rem_euclid` written out, and
// the `+ half` has to be inside the subtraction as well as the floor: with
// it only in the floor every coordinate came out a half-box low, which put
// nine particles in ten behind the eye and drew nothing.
fn wrap(v: f32, half: f32) -> f32 {
    let period = 2.0 * half;
    let shifted = v + half;
    return shifted - period * floor(shifted / period) - half;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    // The mesh sits at the eye.
    let eye = world_from_local[3].xyz;
    let half_xy = params.half.x;
    let half_z = params.half.y;
    let index = u32(vertex.color.w + 0.5);
    let life = vertex.color.x;
    let phase = vertex.color.y;
    let size = vertex.color.z;

    // Nowhere: one point far under the camera, so the quad has no area.
    var world = eye + vec3<f32>(0.0, -1000.0, 0.0);
    var colour = vec4<f32>(0.0);
    let word = index >> 5u;
    let hidden = (params.mask[word >> 2u][word & 3u] >> (index & 31u)) & 1u;
    if f32(index) < params.half.z && hidden == 0u {
        let cycles = (params.wind.w + phase * life) / life;
        let cycle = floor(cycles);
        let age = cycles - cycle;
        let k = index * 3u + u32(cycle) * 0x9E3779B1u;
        let seed = vec3<f32>(unit(k) * 2.0 - 1.0, unit(k + 1u) * 2.0 - 1.0, unit(k + 2u) * 2.0 - 1.0)
            * vec3<f32>(half_xy, half_z, half_xy);
        let s = age * life;
        let raw = seed + vertex.normal * (params.half.w * s) + params.wind.xyz - eye;
        let rel = vec3<f32>(wrap(raw.x, half_xy), wrap(raw.y, half_z), wrap(raw.z, half_xy));

        let right = view.world_from_view[0].xyz;
        let up = view.world_from_view[1].xyz;
        let forward = -view.world_from_view[2].xyz;
        let kind = params.shape.x;
        let distance = length(rel);
        // How far in front of its centre a quad of this shape can reach — the
        // margin the behind-the-camera cull keeps.
        var reach = 2.0;
        if kind < 0.5 {
            reach = size;
        } else if kind > 1.5 {
            reach = params.shape.z + 1.0;
        }
        let behind = dot(rel, forward) < -reach;
        // In over the first tenth of the life and out over the last fifth —
        // the window both of the reference's shaders carry in `texcoord` —
        // and out again over the near fade to the eye, squared.
        let life_fade = min(age / 0.1, 1.0) * min((1.0 - age) / 0.2, 1.0);
        let near = min(distance / params.shape.w, 1.0);
        let alpha = params.tint.w * clamp(life_fade * near * near, 0.0, 1.0);
        // A mist sheet the camera stands inside is a screenful of blending for
        // a wash the fog already paints.
        let inside_sheet = kind < 0.5 && distance < size;
        if !behind && alpha >= 0.01 && !inside_sheet {
            let sx = vertex.uv.x * 2.0 - 1.0;
            let sy = 1.0 - 2.0 * vertex.uv.y;
            // One pixel's worth of yards at this distance, for the two shapes
            // sized in pixels.
            let yards_per_pixel = 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
            var offset = vec3<f32>(0.0);
            if kind < 0.5 {
                offset = right * (sx * size) + up * (sy * size);
            } else if kind < 1.5 {
                // `snowpoint.bls`: a size in pixels, falling off linearly with
                // distance and never below the floor (`MAX result.pointsize,
                // 1`, in `shape.y`) — the half-quad that covers it is that
                // many pixels' worth of yards at this distance.
                let pixels = max(size * clamp(1.0 - distance / params.gust.w, 0.0, 1.0), params.shape.y);
                let h = pixels * 0.5 * distance * yards_per_pixel;
                offset = right * (sx * h) + up * (sy * h);
            } else {
                // `rain.bls`: the trailing corners are the drop's motion scaled
                // to a fixed length of *fall*, and the width is across the eye
                // direction, turned about the fall.
                let moving = vertex.normal + params.gust.xyz;
                let along = moving * (params.shape.z / max(-moving.y, 0.5));
                var to_eye = vec3<f32>(0.0);
                if distance > 1e-6 {
                    to_eye = -rel / distance;
                }
                var side = cross(along, to_eye);
                let l2 = dot(side, side);
                if l2 < 1e-6 {
                    side = right;
                } else {
                    side = side / sqrt(l2);
                }
                let half_width = max(params.shape.y, STREAK_MIN_HALF_PIXELS * distance * yards_per_pixel);
                offset = side * (sx * half_width) + along * (1.0 - vertex.uv.y);
            }
            world = eye + rel + offset;
            colour = vec4<f32>(params.tint.rgb, alpha);
        }
    }
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
#ifdef VERTEX_NORMALS
    out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_COLORS
    out.color = colour;
#endif
    return out;
}

// **Texel times the quad's own colour, in byte space**, the particle path
// `m2.wgsl` takes and the same fixed-function arithmetic; then the fog, since
// `rain.bls` writes `fogcoord`, and the frame's own encode.
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(sprite, sprite_sampler, in.uv);
#ifdef VERTEX_COLORS
    let tint = in.color;
#else
    let tint = vec4<f32>(1.0);
#endif
    let rgb = srgb_to_linear(linear_to_srgb(texel.rgb) * tint.rgb);
    let alpha = texel.a * tint.a;
    if alpha < 1.0 / 255.0 {
        discard;
    }
    return to_frame(fogged(vec4<f32>(rgb, alpha), in.world_position));
}
