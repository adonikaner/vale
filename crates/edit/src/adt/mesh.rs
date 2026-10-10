//! Where an edited chunk's vertices sit in a tile mesh, and what they are now.
//!
//! ## What this is for
//!
//! A renderer that has already uploaded a tile holds the ground as one buffer
//! per draw group, in the order `Adt::to_mesh` emitted it. Rewriting a chunk's
//! heights in that buffer — rather than reading the file again and rebuilding
//! 256 chunks and eleven tilesets — is what lets a brush move the ground within
//! the frame the mouse moved.
//!
//! Two facts make it possible and both are `vale_assets`':
//!
//! * a chunk's vertices are one **contiguous run**, because `to_mesh` emits the
//!   chunks in order and each contributes five vertices per cell it draws;
//! * the run's **length depends only on the hole mask**, which a height edit
//!   does not change — so a span taken before an edit is still the span after
//!   it, and nothing has to be renumbered.
//!
//! The emission order itself is stated once, in
//! [`vale_assets::world::adt::chunk_mesh_vertices`], and called from here.
//! Two orders would not disagree loudly: they would put a handful of vertices at
//! each other's indices, which is a few spikes in the ground and nothing in any
//! count.

use super::{heights, AdtFile};
use vale_assets::world::adt::{chunk_mesh_vertices, drawn_cells, ChunkGrid, MeshVertex};

/// Where a chunk's vertices begin in the tile's mesh, and how many there are.
///
/// The same arithmetic as `Adt::chunk_vertex_span`, over the container this
/// crate edits rather than over the parsed tile.
pub fn chunk_span(tile: &AdtFile, chunk: usize) -> (usize, usize) {
    let cells = |i: usize| {
        tile.chunk(i)
            .map(|c| drawn_cells(c.head().holes()) * 5)
            .unwrap_or(0)
    };
    ((0..chunk).map(cells).sum(), cells(chunk))
}

/// One chunk's vertices as the tile mesh wants them now, in the order they were
/// emitted in.
///
/// The position is in the world's own axes; a renderer converts. `None` for a
/// chunk with no heights, which no 1.12 tile has.
pub fn chunk_vertices(tile: &AdtFile, chunk: usize) -> Option<Vec<MeshVertex>> {
    let map_chunk = tile.chunk(chunk)?;
    let head = map_chunk.head();
    let heights = heights::heights(map_chunk);
    if heights.is_empty() {
        return None;
    }
    // Stitched as `Adt::to_mesh` stitches them, so a patched edge vertex has
    // the value the tile mesh gave it; see `ChunkGrid::stitched`.
    let grid = ChunkGrid::new((0..).map_while(|i| tile.chunk(i)).map(|c| c.head().position()));
    let heights = grid.stitched(head.position(), &heights, |i| {
        tile.chunk(i).map(heights::heights).filter(|h| !h.is_empty())
    });
    let normals = heights::normals(map_chunk);
    // **The shading too**, because a live patch writes every attribute a vertex
    // has and a colour stroke is a vertex edit. Decoded through
    // `vale_assets`' own reading of the region rather than this crate's, so
    // the two cannot drift — see `crate::adt::colours`, which is the writing
    // half. Empty for a chunk with no `MCCV`, which is every shipped one.
    let colours = map_chunk
        .region(crate::adt::Region::Colours)
        .map(|mccv| vale_assets::world::adt::decode_colours(&mccv.data))
        .unwrap_or_default();
    let mut out = Vec::with_capacity(drawn_cells(head.holes()) * 5);
    chunk_mesh_vertices(
        head.position(),
        &heights,
        &normals,
        &colours,
        head.holes(),
        |vertex| out.push(vertex),
    );
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adt::AdtFile;

    /// The span this crate computes is the span `Adt::to_mesh` actually laid
    /// out, and the vertices in it are the ones it emitted.
    ///
    /// **The check that the live edit is aimed at the right bytes.** Two
    /// readings of one order do not disagree loudly: they put a handful of
    /// vertices at each other's indices, which on screen is a few spikes and in
    /// every count is nothing. So it is compared against the real thing, on a
    /// real tile, chunk by chunk.
    #[test]
    fn a_chunks_span_and_vertices_are_the_ones_to_mesh_built() {
        let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("Data")
                .to_string_lossy()
                .into_owned()
        });
        let Ok(mut assets) = vale_assets::Assets::open(&root) else {
            eprintln!("no archives at {root} — the mesh span check did not run");
            return;
        };
        let path = vale_assets::adt_path("Azeroth", 32, 48);
        let Ok(bytes) = assets.read(&path) else { return };

        let edited = AdtFile::parse(&bytes).unwrap();
        let parsed = vale_assets::Adt::parse(&bytes).unwrap();
        let mesh = parsed.to_mesh();

        let mut checked = 0;
        for chunk in 0..parsed.chunks.len() {
            let (start, count) = chunk_span(&edited, chunk);
            assert_eq!(
                (start, count),
                parsed.chunk_vertex_span(chunk),
                "chunk {chunk}: the container and the parser disagree about the span"
            );
            let Some(vertices) = chunk_vertices(&edited, chunk) else {
                continue;
            };
            assert_eq!(vertices.len(), count, "chunk {chunk}");
            for (i, vertex) in vertices.iter().enumerate() {
                let at = start + i;
                // Exactly: both sides put x and y on the lattice and stitch
                // the edge heights, so a live patch writes the bits the tile
                // mesh was built with and opens no seam.
                let was = mesh.positions[at];
                assert_eq!(
                    vertex.position, was,
                    "chunk {chunk} vertex {i} (tile vertex {at}) against to_mesh's"
                );
                checked += 1;
            }
        }
        // A tile is 256 chunks of at most 64 cells at five vertices each, less
        // the holes: 81,840 on `Azeroth_32_48`. The bound is well under that so
        // that a tile with more holes still counts as checked.
        assert!(checked > 50_000, "only {checked} vertices were checked");
        // The skirt along the tile's edges follows the chunk vertices.
        assert_eq!(
            mesh.positions.len() - mesh.skirt_sources.len(),
            parsed.chunk_vertex_span(parsed.chunks.len() - 1).0
                + parsed.chunk_vertex_span(parsed.chunks.len() - 1).1,
            "the spans account for every chunk vertex in the tile"
        );
    }
}
