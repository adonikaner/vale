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
//
// A line's mesh has every vertex on its segment, with the direction to push it
// out in the tangent slot (`xyz`) and the line's width (`w`), negated for a
// line that rests on the ground. The vertex stage pushes it out by the width
// times LINE_PIXELS pixels at its own depth, and up by the same distance for a
// line on the ground. The tangent slot is used so the mesh pipeline lays the
// attribute out and defines VERTEX_TANGENTS for a line's mesh and for no other.

#import bevy_pbr::forward_io::{Vertex, VertexOutput}
#import bevy_pbr::mesh_functions
#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::view_transformations::position_world_to_clip
#import vale::gamma::{srgb_to_linear, to_frame}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> colour: vec4<f32>;

// A line's radius in pixels: `LINE_PIXELS` in `marks.rs`.
const LINE_PIXELS: f32 = 1.5;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    var world = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    ).xyz;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
#ifdef VERTEX_TANGENTS
    // Yards per pixel at this depth: the clip w is the depth in front of the
    // eye in perspective and 1 in the orthographic map view. A point beside or
    // behind the eye is given a yard of depth, as `Marks::pixel` does.
    let depth = max((view.clip_from_world * vec4<f32>(world, 1.0)).w, 1.0);
    let pixel = 2.0 * depth / (view.clip_from_view[1][1] * view.viewport.w);
    let radius = pixel * LINE_PIXELS * abs(vertex.tangent.w);
    let lift = select(0.0, radius, vertex.tangent.w < 0.0);
    world = world + vertex.tangent.xyz * radius + vec3<f32>(0.0, lift, 0.0);
#endif
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    return out;
}

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
