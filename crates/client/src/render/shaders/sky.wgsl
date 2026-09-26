// The sky dome: six colours out of `Light.dbc`, interpolated by how far up the
// fragment is.
//
// **The one surface in the world that is not fogged, and that is the point.**
// Everything else fades toward `fog()` with distance; the sky *is* what it fades
// to, so fogging it would be mixing a colour with itself at the horizon and
// painting the haze over the zenith everywhere else — which is exactly the
// "fog blocks the sky" complaint. A dome is also why there is anything to fog
// *to*: before this, `ClearColor` was the horizon band, so looking up showed the
// colour of the ground's own vanishing point.
//
// Nothing here is lit, either: these are colours the table states for this map
// and this hour, already sRGB-decoded on the way in (see `crate::sky`). A sun
// term over them would be lighting the light.
//
// The stops are `w`-tagged with **the sine of the altitude they sit at**, not
// the angle: `direction.y` of a unit vector is that sine already, so the
// fragment needs no trigonometry. Where they sit is
// `vale_assets::light::SKY_ALTITUDES`, which is the one number in this chain
// that is not out of a file — see its note.

//
// **And it is written at the far plane, whatever the dome's own radius says.**
// See `render::sky::SkyMaterial::specialize` for the argument; the one line
// here is `@builtin(frag_depth) = 0.0`, which under Bevy's reversed depth is
// as far away as anything can be. It costs the early-depth rejection this
// surface used to get for free — a shader that writes its own depth cannot be
// culled before it runs — which is why the whole of it is one loop over six
// stops and no texture fetch.

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view
#import vale::gamma::to_frame

struct SkyStops {
    // rgb: the linear colour. w: the altitude, 1 straight up and 0 at the
    // horizon. Descending, which the loop below depends on.
    stops: array<vec4<f32>, 6>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> sky: SkyStops;

struct SkyOutput {
    @location(0) colour: vec4<f32>,
    // Reversed depth: 1.0 is the near plane and 0.0 is infinitely far.
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fragment(in: VertexOutput) -> SkyOutput {
    // The dome follows the camera, so this is the view direction — but taken
    // from the fragment rather than assumed, because the mesh is only *nearly*
    // centred: it is moved once a frame and the camera moves within it.
    let direction = normalize(in.world_position.xyz - view.world_position.xyz);
    let altitude = direction.y;

    // Above the first stop and below the last, the dome is that stop's colour
    // flat. The bottom one matters: it is the fog band, so everything under the
    // horizon is already the colour the distant ground fades to, and the two
    // meet with no seam.
    var colour = sky.stops[0].rgb;
    for (var i = 1u; i < 6u; i = i + 1u) {
        let above = sky.stops[i - 1u];
        let below = sky.stops[i];
        if altitude < above.w {
            let span = max(above.w - below.w, 1e-5);
            colour = mix(above.rgb, below.rgb, clamp((above.w - altitude) / span, 0.0, 1.0));
        }
    }
    var out: SkyOutput;
    // Encoded, like every other surface that writes to the frame — see
    // `to_frame`. The dome is opaque and nothing blends with it, so the pixel is
    // unchanged; what it has to be is in the same space as the terrain drawn
    // over it, or the horizon the fog fades to stops being the horizon.
    //
    // **The lerp between the six stops is still taken in linear**, because that
    // is where `crate::sky` decodes them. It is a mix of two `Light.dbc` bands
    // and by this file's own rule it belongs in the file's space — a separate
    // change from this one, and one that moves a pixel rather than preserving it.
    out.colour = to_frame(vec4<f32>(colour, 1.0));
    out.depth = 0.0;
    return out;
}
