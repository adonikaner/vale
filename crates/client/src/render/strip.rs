//! The write of a triangle strip that a pass rebuilds from the CPU every
//! frame: the ribbon trails, the weapon trails and the lightning bolts.
//!
//! Each of the three keeps its own geometry rule and hands the finished
//! vertices here. [`write`] replaces the mesh's attributes and indices;
//! [`indices`] is the index list of a strip of quads between consecutive pairs
//! of vertices. A frame with no strip is written by
//! [`super::nothing::nothing_drawn`] instead, never as an empty mesh.

use bevy::mesh::{Indices, Mesh};

/// The colour of a strip's vertices.
pub(crate) enum Colour {
    /// One colour for every vertex.
    All([f32; 4]),
    /// One colour per vertex, in vertex order.
    Each(Vec<[f32; 4]>),
}

/// Replace `mesh`'s geometry with a strip.
///
/// The normal is `+Y` on every vertex; the strip materials do not light it.
/// `uvs` of `None` writes zero texture coordinates, for a strip drawn in a
/// flat colour.
pub(crate) fn write(
    mesh: &mut Mesh,
    positions: Vec<[f32; 3]>,
    uvs: Option<Vec<[f32; 2]>>,
    colour: Colour,
    indices: Vec<u32>,
) {
    let count = positions.len();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; count]);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        uvs.unwrap_or_else(|| vec![[0.0f32, 0.0]; count]),
    );
    let colours = match colour {
        Colour::All(rgba) => vec![rgba; count],
        Colour::Each(colours) => colours,
    };
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_indices(Indices::U32(indices));
}

/// The indices of a strip of `pairs` vertex pairs, laid out
/// `[a0, b0, a1, b1, ...]`: two triangles between each pair and the next.
pub(crate) fn indices(pairs: usize) -> Vec<u32> {
    let quads = pairs.saturating_sub(1);
    let mut indices = Vec::with_capacity(quads * 6);
    for k in 0..quads as u32 {
        let b = k * 2;
        indices.extend_from_slice(&[b, b + 1, b + 2, b + 1, b + 3, b + 2]);
    }
    indices
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;

    /// Three pairs make two quads, each two triangles that share the pairs'
    /// diagonal; one pair makes none.
    #[test]
    fn a_strip_of_three_pairs_is_two_quads() {
        assert_eq!(indices(3), vec![0, 1, 2, 1, 3, 2, 2, 3, 4, 3, 5, 4]);
        assert!(indices(1).is_empty());
        assert!(indices(0).is_empty());
    }

    /// Every attribute is written at the position count, so `count_vertices`,
    /// which reads the shortest array, reads the strip.
    #[test]
    fn every_attribute_has_one_value_per_vertex() {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        write(&mut mesh, vec![[0.0; 3]; 6], None, Colour::All([1.0; 4]), indices(3));
        assert_eq!(mesh.count_vertices(), 6);
        assert_eq!(mesh.attribute(Mesh::ATTRIBUTE_COLOR).map(|c| c.len()), Some(6));
        assert_eq!(mesh.attribute(Mesh::ATTRIBUTE_UV_0).map(|c| c.len()), Some(6));
    }
}
