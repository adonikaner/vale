// The M2 material's **prepass** vertex stage.
//
// **Bevy 0.19.1's own `bevy_pbr::prepass::vertex`, copied verbatim, with one
// block added** — the same `#ifdef WIND` displacement the forward stage makes,
// through the same function in `wind.wgsl`. Read `m2_vertex.wgsl`'s header
// first: everything it says about why this is a copy and about re-syncing it on
// a Bevy upgrade applies here too. Upstream's `fragment` half is **not** copied;
// the prepass fragment stays `m2_prepass.wgsl`, which is this project's own and
// exists for the alpha test.
//
// ## Why the prepass has to sway too
//
// The depth prepass writes the depth the forward pass is then tested against
// (`GreaterEqual`), so a vertex that moves in one stage and not the other lands
// behind its own recorded depth and is discarded. Swaying only the forward
// stage would put holes in the grass — and only when `F6` is on, which is off by
// default, so it would have been a bug nobody found for months.
//
// The two stages therefore call **one** function, in one file, rather than each
// carrying a copy of the maths.

#import bevy_pbr::{
    prepass_bindings,
    mesh_bindings::mesh,
    mesh_functions,
    prepass_io::{Vertex, VertexOutput, FragmentOutput},
    skinning,
    morph,
    morph::{morph_position, morph_normal, morph_tangent},
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

#ifdef WIND
#import vale::wind::sway
#endif

#ifdef DEFERRED_PREPASS
#import bevy_pbr::rgb9e5
#endif

#ifdef MORPH_TARGETS
// The instance_index parameter must match vertex_in.instance_index. This is a work around for a wgpu dx12 bug.
// See https://github.com/gfx-rs/naga/issues/2416
fn morph_vertex(vertex_in: Vertex, instance_index: u32) -> Vertex {
    var vertex = vertex_in;
    let first_vertex = mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;

    let weight_count = morph::layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = morph::weight_at(i, instance_index);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph_position(vertex_index, i, instance_index);
#ifdef VERTEX_NORMALS
        vertex.normal += weight * morph_normal(vertex_index, i, instance_index);
#endif
#ifdef VERTEX_TANGENTS
        vertex.tangent += vec4(weight * morph_tangent(vertex_index, i, instance_index), 0.0);
#endif
    }
    return vertex;
}

// Returns the morphed position of the given vertex from the previous frame.
//
// This function is used for motion vector calculation, and, as such, it doesn't
// bother morphing the normals and tangents.
//
// The instance_index parameter must match vertex_in.instance_index. This is a work around for a wgpu dx12 bug.
// See https://github.com/gfx-rs/naga/issues/2416
fn morph_prev_vertex(vertex_in: Vertex, instance_index: u32) -> Vertex {
    var vertex = vertex_in;
    let first_vertex = mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;
    let weight_count = morph::layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = morph::prev_weight_at(i, instance_index);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph_position(vertex_index, i, instance_index);
        // Don't bother morphing normals and tangents; we don't need them for
        // motion vector calculation.
    }
    return vertex;
}
#endif  // MORPH_TARGETS

@vertex
fn vertex(vertex_no_morph: Vertex) -> VertexOutput {
    var out: VertexOutput;

#ifdef MORPH_TARGETS
    var vertex = morph_vertex(vertex_no_morph, vertex_no_morph.instance_index);
#else
    var vertex = vertex_no_morph;
#endif

    let mesh_world_from_local = mesh_functions::get_world_from_local(vertex_no_morph.instance_index);

#ifdef SKINNED
    // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
    // See https://github.com/gfx-rs/naga/issues/2416
    var world_from_local = skinning::skin_model(
        vertex.joint_indices,
        vertex.joint_weights,
        vertex_no_morph.instance_index
    );
#else // SKINNED
    var world_from_local = mesh_world_from_local;
#endif // SKINNED

    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
// ---- the one addition: the ground foliage's sway. See `wind.wgsl`. ----------
// Byte for byte what the forward stage does, through the same function, because
// the depth this writes is the depth that pass is tested against.
#ifdef WIND
    out.world_position = vec4<f32>(out.world_position.xyz + sway(vertex.uv_b), out.world_position.w);
#endif
// ---------------------------------------------------------------------------
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0); // Clamp depth to avoid clipping
#endif // UNCLIPPED_DEPTH_ORTHO_EMULATION

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif // VERTEX_UVS_A

#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif // VERTEX_UVS_B

#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
#ifdef SKINNED
    out.world_normal = skinning::skin_normals(world_from_local, vertex.normal);
#else // SKINNED
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        vertex.normal,
        // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
        // See https://github.com/gfx-rs/naga/issues/2416
        vertex_no_morph.instance_index
    );
#endif // SKINNED
#endif // VERTEX_NORMALS

#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        vertex.tangent,
        // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
        // See https://github.com/gfx-rs/naga/issues/2416
        vertex_no_morph.instance_index
    );
#endif // VERTEX_TANGENTS
#endif // NORMAL_PREPASS_OR_DEFERRED_PREPASS

#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif

    // Compute the motion vector for TAA among other purposes. For this we need
    // to know where the vertex was last frame.
#ifdef MOTION_VECTOR_PREPASS

    // Take morph targets into account.
#ifdef MORPH_TARGETS

#ifdef HAS_PREVIOUS_MORPH
    let prev_vertex = morph_prev_vertex(vertex_no_morph, vertex_no_morph.instance_index);
#else   // HAS_PREVIOUS_MORPH
    let prev_vertex = vertex_no_morph;
#endif  // HAS_PREVIOUS_MORPH

#else   // MORPH_TARGETS
    let prev_vertex = vertex_no_morph;
#endif  // MORPH_TARGETS

    // Take skinning into account.
#ifdef SKINNED

#ifdef HAS_PREVIOUS_SKIN
    let prev_model = skinning::skin_prev_model(
        prev_vertex.joint_indices,
        prev_vertex.joint_weights,
        vertex_no_morph.instance_index
    );
#else   // HAS_PREVIOUS_SKIN
    // Use vertex_no_morph.instance_index instead of prev_vertex.instance_index to work around a wgpu dx12 bug.
    // See https://github.com/gfx-rs/naga/issues/2416
    let prev_model = mesh_functions::get_previous_world_from_local(vertex_no_morph.instance_index);
#endif  // HAS_PREVIOUS_SKIN

#else   // SKINNED
    // Use vertex_no_morph.instance_index instead of prev_vertex.instance_index to work around a wgpu dx12 bug.
    // See https://github.com/gfx-rs/naga/issues/2416
    let prev_model = mesh_functions::get_previous_world_from_local(vertex_no_morph.instance_index);
#endif  // SKINNED

    out.previous_world_position = mesh_functions::mesh_position_local_to_world(
        prev_model,
        vec4<f32>(prev_vertex.position, 1.0)
    );
#endif // MOTION_VECTOR_PREPASS

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
    // See https://github.com/gfx-rs/naga/issues/2416
    out.instance_index = vertex_no_morph.instance_index;
#endif

#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex_no_morph.instance_index, mesh_world_from_local[3]);
#endif  // VISIBILITY_RANGE_DITHER

    return out;
}
