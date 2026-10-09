// The editor's marks: handles, waypoint domes, brush dashes, trigger volumes.
// See `crates/editor/src/marks.rs`.
//
// One flat colour, shaded by how squarely the surface faces the camera, so a
// sphere reads as a sphere and an arrow as a solid. Not lit by the world's
// light and not fogged: a mark has to read the same at noon and at midnight.
//
// The colour arrives as the bytes `Color::srgb` states, divided by 255. It is
// decoded, shaded and encoded again by `to_frame`, because the world is drawn
// into a byte-space target (see `render::present` in the client).
//
// The ghost pass is the same mesh drawn only where something is in front of
// it (the pipeline's depth test is reversed, see `marks.rs`), flat and faint.

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view
#import vale::gamma::{srgb_to_linear, to_frame}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> colour: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let base = srgb_to_linear(colour.rgb);
#ifdef GHOST
    return to_frame(vec4<f32>(base, colour.a));
#else
    let toward = normalize(view.world_position.xyz - in.world_position.xyz);
    // Two-sided volumes are seen from inside too, so the side facing away
    // shades the same as the side facing the camera.
    let facing = abs(dot(normalize(in.world_normal), toward));
    let shade = 0.45 + 0.55 * facing;
    return to_frame(vec4<f32>(base * shade, colour.a));
#endif
}
