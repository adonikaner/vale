// The M2 material's forward vertex stage.
//
// **This is Bevy 0.19.1's own `bevy_pbr::mesh::vertex`, copied verbatim, with
// exactly one block added** — the `#ifdef WIND` displacement, which is the
// ground foliage's sway (see `wind.wgsl`, which holds the maths and the
// measurement behind it). Everything else here, comments included, is upstream:
// morph targets, skinning, tangents, the three dx12 `instance_index`
// work-arounds, visibility-range dither.
//
// ## Why a copy, and what it costs
//
// A `Material` may override the vertex shader or take Bevy's, and there is no
// hook in between — so displacing a vertex means owning the whole stage. Every
// M2 in the game is drawn through this material: the trees, the buildings'
// batches, and every **skinned** character. So the copy has to be exact, and
// the added block is gated behind a shader def that only the foliage materials
// set, which means every other batch in the world compiles the byte-for-byte
// upstream program.
//
// **It has to be re-synced on a Bevy upgrade.** That is the standing cost of
// this file and there is nothing clever to do about it: diff it against
// `bevy_pbr-<version>/src/render/mesh.wgsl` and re-apply the one block. The
// symptom of forgetting is not a compile error — it is a feature Bevy added to
// its vertex stage quietly not happening here.

#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    skinning,
    morph::{morph_position, morph_normal, morph_tangent},
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
}

#ifdef WIND
#import vale::wind::sway
#endif

#ifdef MORPH_TARGETS
// The instance_index parameter must match vertex_in.instance_index. This is a work around for a wgpu dx12 bug.
// See https://github.com/gfx-rs/naga/issues/2416
fn morph_vertex(vertex_in: Vertex, instance_index: u32) -> Vertex {
    var vertex = vertex_in;
    let first_vertex = mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;

    let weight_count = bevy_pbr::morph::layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = bevy_pbr::morph::weight_at(i, instance_index);
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
#endif

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
    // See https://github.com/gfx-rs/naga/issues/2416 .
    var world_from_local = skinning::skin_model(
        vertex.joint_indices,
        vertex.joint_weights,
        vertex_no_morph.instance_index
    );
#else
    var world_from_local = mesh_world_from_local;
#endif

#ifdef VERTEX_NORMALS
#ifdef SKINNED
    out.world_normal = skinning::skin_normals(world_from_local, vertex.normal);
#else
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        vertex.normal,
        // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
        // See https://github.com/gfx-rs/naga/issues/2416
        vertex_no_morph.instance_index
    );
#endif
#endif

#ifdef VERTEX_POSITIONS
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
// ---- the one addition: the ground foliage's sway. See `wind.wgsl`. ----------
// In **world** space and after the model matrix, so a tuft leans along the
// wind's own direction rather than along whichever way its own hashed yaw
// happened to point it. `WIND` is set only on foliage materials, and the
// prepass stage applies the identical offset from the identical function.
#ifdef WIND
    out.world_position = vec4<f32>(out.world_position.xyz + sway(vertex.uv_b), out.world_position.w);
#endif
// ---------------------------------------------------------------------------
    out.position = position_world_to_clip(out.world_position.xyz);
#endif

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif

#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        vertex.tangent,
        // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
        // See https://github.com/gfx-rs/naga/issues/2416
        vertex_no_morph.instance_index
    );
#endif

#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
    // See https://github.com/gfx-rs/naga/issues/2416
    out.instance_index = vertex_no_morph.instance_index;
#endif

#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex_no_morph.instance_index, mesh_world_from_local[3]);
#endif

    return out;
}
