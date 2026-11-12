//! Where an edited chunk's vertices sit in a tile mesh, and what they are now.
//!
//! ## What this is for
//!
//! A renderer that has already uploaded a tile holds the ground as one buffer
//! per draw group, in the order `Adt::to_mesh` emitted it. A brush moves the
//! ground in the frame the mouse moved by rewriting a chunk's heights in that
//! buffer. The alternative is reading the file again and rebuilding 256 chunks
//! and eleven tilesets.
//!
//! Two facts make it possible, and both are `vale_assets`':
//!
//! * A chunk's vertices are one contiguous run, because `to_mesh` emits the
//!   chunks in order and each contributes five vertices per cell it draws.
//! * The run's length depends only on the hole mask, which a height edit does
//!   not change. A span taken before an edit is still the span after it, and
//!   nothing has to be renumbered.
//!
//! The emission order is stated once, in
//! [`vale_assets::world::adt::chunk_mesh_vertices`], and called from here. A
//! second statement of the order that differed from it would put a few
//! vertices at each other's indices. That shows as a few spikes in the ground
//! and changes no count.

use super::{heights, AdtFile};
use vale_assets::world::adt::{chunk_mesh_vertices, drawn_cells, ChunkGrid, MeshVertex};

/// Where a chunk's vertices begin in the tile's mesh, and how many there are.
///
/// The same arithmetic as `Adt::chunk_vertex_span`, over the container this
/// crate edits rather than over the parsed tile. For more than a few chunks of
/// one tile, build a [`TileLayout`] and ask it instead.
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
/// chunk with no heights, which no 1.12 tile has. For more than a few chunks
/// of one tile, build a [`TileLayout`] and ask it instead.
pub fn chunk_vertices(tile: &AdtFile, chunk: usize) -> Option<Vec<MeshVertex>> {
    let mut out = Vec::new();
    TileLayout::of(tile).vertices_into(tile, chunk, &mut out).then_some(out)
}

/// Where every chunk's vertices sit in a tile's mesh, and which chunk is beside
/// which, read from the tile once.
///
/// [`chunk_span`] adds up the chunks before the one asked for, and
/// [`chunk_vertices`] reads every chunk's origin to find the chunk's
/// neighbours, on each call. A stroke with a wide brush patches most of a
/// tile's 256 chunks every frame; asked chunk by chunk, that was 256 readings
/// of 256 headers a tile, 11 ms a frame at a 250-yard brush. A layout reads the
/// headers once and answers each chunk from what it kept.
///
/// A layout holds the hole masks' vertex counts and the chunks' places, and
/// neither a height edit nor a colour edit changes those. It is out of date
/// after a hole edit, as a span taken before one is.
pub struct TileLayout {
    grid: ChunkGrid,
    /// Where each chunk's vertices begin, and after the last chunk, where they
    /// end: one entry more than the tile has chunks.
    starts: Vec<usize>,
}

impl TileLayout {
    pub fn of(tile: &AdtFile) -> TileLayout {
        let mut starts = Vec::with_capacity(tile.chunks.len() + 1);
        let mut next = 0;
        starts.push(next);
        for chunk in &tile.chunks {
            next += drawn_cells(chunk.head().holes()) * 5;
            starts.push(next);
        }
        TileLayout {
            grid: ChunkGrid::new(tile.chunks.iter().map(|chunk| chunk.head().position())),
            starts,
        }
    }

    /// [`chunk_span`]'s answer. A chunk the tile does not have begins where
    /// the vertices end and has none.
    pub fn span(&self, chunk: usize) -> (usize, usize) {
        match (self.starts.get(chunk), self.starts.get(chunk + 1)) {
            (Some(start), Some(end)) => (*start, end - start),
            _ => (self.vertex_count(), 0),
        }
    }

    /// How many vertices the tile's chunks have between them, which is where
    /// the last chunk's span ends.
    pub fn vertex_count(&self) -> usize {
        self.starts.last().copied().unwrap_or(0)
    }

    /// [`chunk_vertices`]' answer, added to the end of `out`. False, with
    /// nothing added, for a chunk with no heights.
    pub fn vertices_into(&self, tile: &AdtFile, chunk: usize, out: &mut Vec<MeshVertex>) -> bool {
        out.reserve(self.span(chunk).1);
        self.each_vertex(tile, chunk, |vertex| out.push(vertex))
    }

    /// [`chunk_vertices`]' answer, written over `out`, which is as long as
    /// the chunk's span. False, with `out` as it was, for a chunk with no
    /// heights.
    ///
    /// For a caller that keeps one buffer for many chunks and hands each
    /// chunk its own part of it, so that several chunks can be written at the
    /// same time and none of them allocates.
    pub fn vertices_over(&self, tile: &AdtFile, chunk: usize, out: &mut [MeshVertex]) -> bool {
        let mut slots = out.iter_mut();
        self.each_vertex(tile, chunk, |vertex| {
            if let Some(slot) = slots.next() {
                *slot = vertex;
            }
        })
    }

    /// Hand `emit` one chunk's vertices as the tile mesh wants them now, in
    /// the order they were emitted in. False, with nothing handed over, for a
    /// chunk with no heights.
    fn each_vertex(&self, tile: &AdtFile, chunk: usize, emit: impl FnMut(MeshVertex)) -> bool {
        let Some(map_chunk) = tile.chunk(chunk) else {
            return false;
        };
        let head = map_chunk.head();
        let heights = heights::heights(map_chunk);
        if heights.is_empty() {
            return false;
        }
        // Stitched as `Adt::to_mesh` stitches them, so a patched edge vertex
        // has the value the tile mesh gave it; see `ChunkGrid::stitched`.
        let heights = self.grid.stitched(head.position(), &heights, |i| {
            tile.chunk(i).map(heights::heights).filter(|h| !h.is_empty())
        });
        let normals = heights::normals(map_chunk);
        // The shading too: a live patch writes every attribute a vertex has,
        // and a colour stroke is a vertex edit. It is decoded by
        // `vale_assets`' reading of the region, so this crate keeps no second
        // decoder that could differ from it; `crate::adt::colours` is the
        // writing half. Empty for a chunk with no `MCCV`, which is every
        // shipped one.
        let colours = map_chunk
            .region(crate::adt::Region::Colours)
            .map(|mccv| vale_assets::world::adt::decode_colours(&mccv.data))
            .unwrap_or_default();
        chunk_mesh_vertices(head.position(), &heights, &normals, &colours, head.holes(), emit);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adt::AdtFile;

    /// A layout's spans and vertices are the ones `Adt::to_mesh` lays out for
    /// the same bytes, on a tile with a hole and with a chunk whose edge
    /// heights differ from its neighbours' copies. The hole moves every span
    /// after it, and the differing edge is what the stitch decides.
    #[test]
    fn a_layout_answers_what_to_mesh_built() {
        let mut tile = crate::adt::blank_tile(32, 48, r"Tileset\Generic\Black.blp", 10.0, 0);
        crate::ops::Edit::Holes { chunk: 5, before: 0, after: 0b101 }.apply(&mut tile);
        let ramp: Vec<f32> = (0..145).map(|i| 10.0 + i as f32 * 0.1).collect();
        heights::set_heights(tile.chunk_mut(17).unwrap(), &ramp);

        let parsed = vale_assets::Adt::parse(&tile.write()).unwrap();
        let mesh = parsed.to_mesh();
        let layout = TileLayout::of(&tile);
        let mut checked = 0;
        for chunk in 0..tile.chunks.len() {
            assert_eq!(layout.span(chunk), parsed.chunk_vertex_span(chunk), "chunk {chunk}");
            assert_eq!(layout.span(chunk), chunk_span(&tile, chunk), "chunk {chunk}");
            let (start, count) = layout.span(chunk);
            let mut vertices = Vec::new();
            assert!(layout.vertices_into(&tile, chunk, &mut vertices));
            assert_eq!(vertices.len(), count, "chunk {chunk}");
            let mut over = vec![vertices[0]; count];
            assert!(layout.vertices_over(&tile, chunk, &mut over));
            assert!(over == vertices, "chunk {chunk}: written over a buffer");
            for (i, vertex) in vertices.iter().enumerate() {
                assert_eq!(vertex.position, mesh.positions[start + i], "chunk {chunk} vertex {i}");
                checked += 1;
            }
        }
        assert_eq!(layout.vertex_count(), checked);
        assert_eq!(layout.vertex_count(), 256 * 64 * 5 - 2 * 4 * 5, "two hole bits are eight cells");
        assert_eq!(layout.span(256), (checked, 0));
    }

    /// The span this crate computes is the span `Adt::to_mesh` laid out, and
    /// the vertices in it are the ones it emitted.
    ///
    /// This checks that a live edit writes to the right vertices. Two readings
    /// of one order that differ put a few vertices at each other's indices,
    /// which shows as a few spikes and changes no count. So the comparison is
    /// against `to_mesh` itself, on a shipped tile, chunk by chunk.
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
