//! The mesh a rebuilt-every-frame pass holds when it has nothing to draw.
//!
//! **No mesh this client hands Bevy is ever empty**, and this is the one place
//! that says what "not empty" means. Four passes here rewrite a `Mesh` from the
//! CPU every frame — the emitters, the trails, the blob-shadow field and the
//! ground decals — and each of them has frames with no geometry at all: an
//! emitter whose whole pool is sitting on a zero scale key, a trail with fewer
//! than two edges, a field with every blob behind the camera, a decal cast in
//! mid-air. The obvious thing to write for those frames is an empty vertex
//! array, and it is the one thing that must not be written.
//!
//! ## Why an empty mesh is not free
//!
//! `bevy_render`'s `mesh/allocator.rs` **skips** a mesh whose vertex buffer is
//! empty when it allocates —
//!
//! ```text
//! let vertex_buffer_size = mesh.get_vertex_buffer_size() as u64;
//! if vertex_buffer_size == 0 { continue; }
//! ```
//!
//! — and then walks *every* extracted mesh in the copy loop under it. So the
//! key `copy_element_data` looks up was never allocated and it reports
//!
//! ```text
//! ERROR bevy_render::slab_allocator: Use-after-free: attempted to copy element
//! data for an unallocated key
//! ```
//!
//! **twice** per mesh — once for the vertices, once for the indices, which is
//! why the lines arrive in pairs a fraction of a millisecond apart. Nothing
//! leaks and nothing dangles: the message is mis-attributed, which is worse than
//! a real error, because it has now sent two investigations looking for a
//! dropped handle in the spell effects.
//!
//! **`Visibility::Hidden` does not avoid it.** `ExtractedAssets<RenderMesh>` is
//! driven by asset add/modify events rather than by what is drawn, so a mesh
//! that is never visible still logs the moment it is added — which is why the
//! errors read as a streaming fault rather than as a spell-effect one. Hiding
//! the *entity* is still worth doing (it is a draw call saved); it is simply not
//! what stops the log.
//!
//! ## What is written instead
//!
//! One degenerate quad: four coincident vertices at the origin and the two
//! zero-area triangles over them, which the rasteriser discards before it costs
//! a fragment. That is cheaper than the string formatting one line of the error
//! costs, let alone sixty of them a second.
//!
//! The three geometry builders that do *not* rewrite per frame hold the same
//! invariant the other way, by refusing to build at all:
//! `terrain::group_mesh`, `models::batch_draw` and `wmos::batch_draw` each
//! return `None` for an empty index range.

use bevy::mesh::{Indices, Mesh};

/// How many vertices [`nothing_drawn`] writes.
///
/// Public because a caller that carries an attribute this function does not
/// write — `COLOR`, on the two passes that have one — has to agree with it about
/// the count, or Bevy's `count_vertices` picks the shortest array and the mesh
/// describes something neither of them meant.
pub const VERTICES: usize = 4;

/// Fill `mesh` with a single degenerate quad, for the reason the module doc
/// gives.
///
/// `POSITION`, `NORMAL` and `UV_0` are written unconditionally; **`COLOR` only
/// when the mesh already declares one.** An attribute's *presence* is what
/// selects the pipeline variant (see `models::loader::draw_mesh`, which makes
/// the same argument about `VERTEX_COLORS` and the skinning pair), so handing a
/// colour to a pass whose material was never specialised against one is a
/// different bug from the one this function exists to fix.
pub fn nothing_drawn(mesh: &mut Mesh) {
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; VERTICES]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; VERTICES]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; VERTICES]);
    if mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; VERTICES]);
    }
    mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;

    fn blank() -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
    }

    /// The whole point: the vertex count is never zero, because zero is what
    /// `allocate_meshes` skips and the copy loop then reports as a
    /// use-after-free.
    #[test]
    fn the_quad_is_not_empty() {
        let mut mesh = blank();
        nothing_drawn(&mut mesh);
        assert_eq!(mesh.count_vertices(), VERTICES);
        assert_eq!(mesh.get_vertex_buffer_size() % VERTICES, 0);
        assert_ne!(mesh.get_vertex_buffer_size(), 0);
    }

    /// …and it draws nothing, which is the other half: four coincident vertices
    /// are two triangles of zero area.
    #[test]
    fn the_quad_has_no_area() {
        let mut mesh = blank();
        nothing_drawn(&mut mesh);
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        let bevy::mesh::VertexAttributeValues::Float32x3(p) = positions else {
            panic!("positions are Float32x3");
        };
        assert!(p.iter().all(|v| *v == [0.0, 0.0, 0.0]));
        assert_eq!(mesh.indices().map(|i| i.len()), Some(6));
    }

    /// A colour is refreshed when the mesh has one and **not invented** when it
    /// does not — the pipeline-variant argument in the function's own doc. Two
    /// of the four passes carry `COLOR` and two do not.
    #[test]
    fn colour_is_kept_but_never_added() {
        let mut plain = blank();
        nothing_drawn(&mut plain);
        assert!(!plain.contains_attribute(Mesh::ATTRIBUTE_COLOR));

        let mut coloured = blank();
        coloured.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; 9]);
        nothing_drawn(&mut coloured);
        assert!(coloured.contains_attribute(Mesh::ATTRIBUTE_COLOR));
        // …and at the same count as everything else, or `count_vertices` reads
        // the shortest array.
        assert_eq!(coloured.count_vertices(), VERTICES);
    }
}
