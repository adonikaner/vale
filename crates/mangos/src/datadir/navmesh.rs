//! A navmesh tile the server walks its creatures on: `mmaps\MMMYYXX.mmtile`,
//! read into triangles in the world's own axes.
//!
//! ## The file
//!
//! `MoveMapGenerator` writes one `.mmtile` per terrain tile. It is a 20-byte
//! `MmapTileHeader` followed by one Detour tile exactly as `dtNavMesh::addTile`
//! takes it, `size` bytes long. Read from a vmangos checkout on
//! 2026-09-23:
//!
//! ```text
//! MmapTileHeader    src/game/Maps/MoveMapSharedDefines.h
//!   u32 mmapMagic     'MMAP', 0x4d4d4150
//!   u32 dtVersion     DT_NAVMESH_VERSION, 7
//!   u32 mmapVersion   MMAP_VERSION, 6
//!   u32 size          bytes of Detour data after this header
//!   u32 usesLiquids   whether liquid was fed to the build
//!
//! Detour tile       dep/recastnavigation/Detour/Source/DetourNavMesh.cpp:971
//!   dtMeshHeader      100 bytes: 15 ints, then 10 floats
//!   verts             12 bytes each: x, y, z
//!   polys             32 bytes each (dtPoly)
//!   links             16 bytes each (dtLink with a 64-bit dtPolyRef)
//!   detailMeshes      12 bytes each (dtPolyDetail)
//!   detailVerts       12 bytes each
//!   detailTris         4 bytes each: three indices and an edge-flag byte
//!   bvTree            16 bytes each (dtBVNode)
//!   offMeshCons       36 bytes each (dtOffMeshConnection)
//! ```
//!
//! Every section is padded to a multiple of four. The link size depends on
//! `DT_POLYREF64`, which the checkout's `DetourNavMesh.h` defines, so a link is
//! 16 bytes rather than 12. The sum of the nine sections equals `size` on all
//! 1,948 tiles of the reference install's `DataDir`, which is the check that
//! every size above is right: one wrong size moves every later section and the
//! sum stops matching.
//!
//! ## Axes
//!
//! Recast works with y up. The generator writes a world point `(x, y, z)` as
//! `(y, z, x)`, so a stored vertex `(a, b, c)` is the world point `(c, a, b)`.
//! Measured on Azeroth 32,48: the tile's stored x runs from -533 to 0, which is
//! world y for column 32, and its stored z from -9,067 to -8,533, which is
//! world x for row 48.
//!
//! ## What is drawn
//!
//! A polygon is a convex outline of up to six vertices. Its detail mesh is a
//! set of triangles over the polygon's own vertices and extra detail vertices,
//! which follows the ground more closely than the outline does. [`parse_tile`]
//! returns the detail triangles, because they are the surface a creature is
//! placed on. A detail triangle's index below the polygon's `vertCount` names
//! one of the polygon's vertices; an index at or above it names detail vertex
//! `vertBase + (index - vertCount)` (`dtNavMesh::getPolyHeight`).
//!
//! The polygon **flags** are what the server's path queries filter on, so they
//! decide the colour; see [`Surface`]. The **area** byte is the generator's
//! own intermediate classification and the server does not read it.
//!
//! An edge whose `neis` entry is 0 has no polygon on the other side, inside or
//! outside the tile. Those edges are the outline of the walkable surface and
//! are returned in [`NavTile::edges`]. An entry with `DT_EXT_LINK` set is a
//! portal to the neighbouring tile and is not an outline.

use std::path::{Path, PathBuf};

use crate::datadir::Tile;

/// `MmapTileHeader::mmapMagic`, `'MMAP'`.
pub const MMAP_MAGIC: u32 = 0x4d4d_4150;
/// `MMAP_VERSION` in `MoveMapSharedDefines.h`.
pub const MMAP_VERSION: u32 = 6;
/// `DT_NAVMESH_MAGIC`, `'DNAV'`.
pub const DETOUR_MAGIC: u32 = 0x444e_4156;
/// `DT_NAVMESH_VERSION`.
pub const DETOUR_VERSION: u32 = 7;

/// `DT_VERTS_PER_POLYGON`.
const VERTS_PER_POLYGON: usize = 6;

const FILE_HEADER: usize = 20;
const MESH_HEADER: usize = 100;
const VERT: usize = 12;
const POLY: usize = 32;
const LINK: usize = 16;
const DETAIL_MESH: usize = 12;
const DETAIL_TRI: usize = 4;
const BV_NODE: usize = 16;
const OFF_MESH: usize = 36;

/// The `NavTerrain` flag bits, `MoveMapSharedDefines.h`.
pub const NAV_GROUND: u16 = 0x01;
pub const NAV_MAGMA: u16 = 0x02;
pub const NAV_SLIME: u16 = 0x04;
pub const NAV_WATER: u16 = 0x08;
pub const NAV_STEEP_SLOPES: u16 = 0x10;

/// What a polygon is to the server's path queries, from its flags.
///
/// `TileWorker.cpp:756` sets the flags from the generator's area class:
/// ground is `NAV_GROUND`; a steep slope is `NAV_GROUND | NAV_STEEP_SLOPES`;
/// the edge of water is `NAV_GROUND | NAV_WATER`; open water, magma and slime
/// are their one bit. `Map.cpp` and `PathFinder.cpp` exclude
/// `NAV_STEEP_SLOPES` from most queries, and include water, magma and slime
/// only for a creature that can enter them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Surface {
    Ground,
    Steep,
    Shore,
    Water,
    Magma,
    Slime,
    /// No flag bit set: in the mesh, and matched by no query.
    Unflagged,
}

impl Surface {
    pub const ALL: [Surface; 7] = [
        Surface::Ground,
        Surface::Steep,
        Surface::Shore,
        Surface::Water,
        Surface::Magma,
        Surface::Slime,
        Surface::Unflagged,
    ];

    /// The class of a polygon's flags. The liquid bits are tested first, so a
    /// polygon carrying both ground and water is the shore rather than ground.
    pub fn of(flags: u16) -> Surface {
        if flags & NAV_MAGMA != 0 {
            Surface::Magma
        } else if flags & NAV_SLIME != 0 {
            Surface::Slime
        } else if flags & NAV_WATER != 0 {
            match flags & NAV_GROUND != 0 {
                true => Surface::Shore,
                false => Surface::Water,
            }
        } else if flags & NAV_STEEP_SLOPES != 0 {
            Surface::Steep
        } else if flags & NAV_GROUND != 0 {
            Surface::Ground
        } else {
            Surface::Unflagged
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Surface::Ground => "ground",
            Surface::Steep => "steep slope",
            Surface::Shore => "water's edge",
            Surface::Water => "water",
            Surface::Magma => "magma",
            Surface::Slime => "slime",
            Surface::Unflagged => "unflagged",
        }
    }

    /// The flag bits, as vmangos names them.
    pub fn flags(self) -> &'static str {
        match self {
            Surface::Ground => "NAV_GROUND",
            Surface::Steep => "NAV_GROUND | NAV_STEEP_SLOPES",
            Surface::Shore => "NAV_GROUND | NAV_WATER",
            Surface::Water => "NAV_WATER",
            Surface::Magma => "NAV_MAGMA",
            Surface::Slime => "NAV_SLIME",
            Surface::Unflagged => "no flags",
        }
    }
}

/// The triangles of one [`Surface`] in one tile, in world axes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Part {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// How many polygons the triangles came from.
    pub polygons: usize,
}

/// One `.mmtile`, read.
#[derive(Debug, Clone, PartialEq)]
pub struct NavTile {
    /// The tile's cell in Detour's grid, `dtMeshHeader::x` and `y`. It is not
    /// the terrain tile's number; the generator counts from the map's origin.
    pub grid: (i32, i32),
    /// `MmapTileHeader::usesLiquids`.
    pub uses_liquids: bool,
    /// Ground polygons read. Off-mesh connections are not counted.
    pub polygons: usize,
    /// Off-mesh connections, which are not drawn.
    pub off_mesh: usize,
    /// The triangles, by the class of the polygon they belong to, in
    /// [`Surface::ALL`] order. Classes with no polygons are absent.
    pub parts: Vec<(Surface, Part)>,
    /// Polygon edges with no neighbour, as pairs of world points.
    pub edges: Vec<[[f32; 3]; 2]>,
    /// The lowest and highest world point of the tile, from `dtMeshHeader`'s
    /// box.
    pub bounds: [[f32; 3]; 2],
}

impl NavTile {
    pub fn triangles(&self) -> usize {
        self.parts.iter().map(|(_, part)| part.indices.len() / 3).sum()
    }
}

/// `DataDir\mmaps\MMMYYXX.mmtile` for a tile.
pub fn tile_path(data_dir: &Path, tile: Tile) -> PathBuf {
    data_dir.join("mmaps").join(tile.mmtile_file())
}

/// A stored vertex `(a, b, c)` as the world point `(c, a, b)`. See the module
/// comment.
fn world(stored: [f32; 3]) -> [f32; 3] {
    [stored[2], stored[0], stored[1]]
}

/// Detour pads every section to a multiple of four bytes.
fn align4(n: usize) -> usize {
    (n + 3) & !3
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn u8(&self, at: usize) -> Result<u8, String> {
        self.bytes
            .get(at)
            .copied()
            .ok_or_else(|| format!("read past the end at byte {at}"))
    }

    fn u16(&self, at: usize) -> Result<u16, String> {
        let b = self
            .bytes
            .get(at..at + 2)
            .ok_or_else(|| format!("read past the end at byte {at}"))?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&self, at: usize) -> Result<u32, String> {
        let b = self
            .bytes
            .get(at..at + 4)
            .ok_or_else(|| format!("read past the end at byte {at}"))?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&self, at: usize) -> Result<i32, String> {
        self.u32(at).map(|v| v as i32)
    }

    fn f32(&self, at: usize) -> Result<f32, String> {
        self.u32(at).map(f32::from_bits)
    }

    fn vec3(&self, at: usize) -> Result<[f32; 3], String> {
        Ok([self.f32(at)?, self.f32(at + 4)?, self.f32(at + 8)?])
    }

    /// A count from the header, which must not be negative.
    fn count(&self, at: usize, what: &str) -> Result<usize, String> {
        let n = self.i32(at)?;
        usize::try_from(n).map_err(|_| format!("{what} is negative ({n})"))
    }
}

/// Read one `.mmtile`.
///
/// Refuses a file whose magic numbers or versions are not the ones above,
/// and one whose section sizes do not add up to the header's `size`: either
/// means the struct sizes in the module comment do not describe the file, and
/// every triangle read from it would be wrong.
pub fn parse_tile(bytes: &[u8]) -> Result<NavTile, String> {
    let r = Reader { bytes };
    let magic = r.u32(0)?;
    if magic != MMAP_MAGIC {
        return Err(format!("magic is {magic:#010x}, not 'MMAP'"));
    }
    let dt_version = r.u32(4)?;
    let mmap_version = r.u32(8)?;
    if dt_version != DETOUR_VERSION || mmap_version != MMAP_VERSION {
        return Err(format!(
            "Detour version {dt_version} and mmap version {mmap_version}; expected Detour {DETOUR_VERSION} and mmap {MMAP_VERSION}"
        ));
    }
    let size = r.u32(12)? as usize;
    let uses_liquids = r.u32(16)? != 0;
    if bytes.len() != FILE_HEADER + size {
        return Err(format!(
            "the header says {size} bytes of Detour data and the file holds {}",
            bytes.len().saturating_sub(FILE_HEADER)
        ));
    }

    // dtMeshHeader.
    let h = FILE_HEADER;
    let detour_magic = r.u32(h)?;
    if detour_magic != DETOUR_MAGIC {
        return Err(format!("Detour magic is {detour_magic:#010x}, not 'DNAV'"));
    }
    let grid = (r.i32(h + 8)?, r.i32(h + 12)?);
    let poly_count = r.count(h + 24, "polyCount")?;
    let vert_count = r.count(h + 28, "vertCount")?;
    let max_links = r.count(h + 32, "maxLinkCount")?;
    let detail_mesh_count = r.count(h + 36, "detailMeshCount")?;
    let detail_vert_count = r.count(h + 40, "detailVertCount")?;
    let detail_tri_count = r.count(h + 44, "detailTriCount")?;
    let bv_count = r.count(h + 48, "bvNodeCount")?;
    let off_mesh = r.count(h + 52, "offMeshConCount")?;
    let bmin = r.vec3(h + 72)?;
    let bmax = r.vec3(h + 84)?;

    let verts_at = h + align4(MESH_HEADER);
    let polys_at = verts_at + align4(VERT * vert_count);
    let links_at = polys_at + align4(POLY * poly_count);
    let details_at = links_at + align4(LINK * max_links);
    let detail_verts_at = details_at + align4(DETAIL_MESH * detail_mesh_count);
    let detail_tris_at = detail_verts_at + align4(VERT * detail_vert_count);
    let bv_at = detail_tris_at + align4(DETAIL_TRI * detail_tri_count);
    let off_mesh_at = bv_at + align4(BV_NODE * bv_count);
    let end = off_mesh_at + align4(OFF_MESH * off_mesh);
    if end - FILE_HEADER != size {
        return Err(format!(
            "the sections add up to {} bytes and the header says {size}",
            end - FILE_HEADER
        ));
    }

    let vert = |i: usize| -> Result<[f32; 3], String> {
        if i >= vert_count {
            return Err(format!("vertex index {i} is out of range; the tile has {vert_count} vertices"));
        }
        r.vec3(verts_at + VERT * i).map(world)
    };
    let detail_vert = |i: usize| -> Result<[f32; 3], String> {
        if i >= detail_vert_count {
            return Err(format!("detail vertex index {i} is out of range; the tile has {detail_vert_count} detail vertices"));
        }
        r.vec3(detail_verts_at + VERT * i).map(world)
    };

    let mut parts: Vec<(Surface, Part)> = Surface::ALL.iter().map(|&s| (s, Part::default())).collect();
    let mut edges = Vec::new();
    let mut polygons = 0;
    // A polygon's own vertices then its detail vertices, reused per polygon.
    let mut local: Vec<[f32; 3]> = Vec::with_capacity(16);

    for p in 0..poly_count {
        // dtPoly: firstLink at 0, verts[6] at 4, neis[6] at 16, flags at 28,
        // vertCount at 30, areaAndtype at 31.
        let at = polys_at + POLY * p;
        let flags = r.u16(at + 28)?;
        let count = r.u8(at + 30)? as usize;
        let area_and_type = r.u8(at + 31)?;
        // Type 1 is DT_POLYTYPE_OFFMESH_CONNECTION: two points and no surface.
        if area_and_type >> 6 != 0 {
            continue;
        }
        if count > VERTS_PER_POLYGON {
            return Err(format!("polygon {p} has {count} vertices; the limit is {VERTS_PER_POLYGON}"));
        }
        polygons += 1;

        local.clear();
        for k in 0..count {
            local.push(vert(r.u16(at + 4 + 2 * k)? as usize)?);
        }
        for k in 0..count {
            // 0 is no neighbour. Any other value, with or without
            // `EXT_LINK`, is a polygon on the other side of the edge.
            if r.u16(at + 16 + 2 * k)? == 0 {
                edges.push([local[k], local[(k + 1) % count]]);
            }
        }

        let part = &mut parts
            .iter_mut()
            .find(|(s, _)| *s == Surface::of(flags))
            .expect("every surface has a part")
            .1;
        part.polygons += 1;

        if p >= detail_mesh_count {
            // No detail mesh: fan the outline.
            let base = part.positions.len() as u32;
            part.positions.extend_from_slice(&local);
            for k in 1..count.saturating_sub(1) {
                part.indices.extend([base, base + k as u32, base + k as u32 + 1]);
            }
            continue;
        }
        let d = details_at + DETAIL_MESH * p;
        let vert_base = r.u32(d)? as usize;
        let tri_base = r.u32(d + 4)? as usize;
        let extra = r.u8(d + 8)? as usize;
        let tris = r.u8(d + 9)? as usize;
        for k in 0..extra {
            local.push(detail_vert(vert_base + k)?);
        }
        let base = part.positions.len() as u32;
        part.positions.extend_from_slice(&local);
        for t in 0..tris {
            let tri = tri_base + t;
            if tri >= detail_tri_count {
                return Err(format!("detail triangle index {tri} is out of range; the tile has {detail_tri_count} detail triangles"));
            }
            let at = detail_tris_at + DETAIL_TRI * tri;
            for k in 0..3 {
                let index = r.u8(at + k)? as usize;
                if index >= local.len() {
                    return Err(format!(
                        "polygon {p} triangle {t} names vertex {index}; only {} are available",
                        local.len()
                    ));
                }
                part.indices.push(base + index as u32);
            }
        }
    }

    parts.retain(|(_, part)| part.polygons > 0);
    let (low, high) = (world(bmin), world(bmax));
    Ok(NavTile {
        grid,
        uses_liquids,
        polygons,
        off_mesh,
        parts,
        edges,
        bounds: [low, high],
    })
}

/// Read one tile off disk. `Ok(None)` when the file is absent, which is the
/// ordinary state of a tile with no terrain.
pub fn read_tile(data_dir: &Path, tile: Tile) -> Result<Option<NavTile>, String> {
    let path = tile_path(data_dir, tile);
    match std::fs::read(&path) {
        Ok(bytes) => parse_tile(&bytes)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `DT_EXT_LINK`: a `neis` entry with this bit is an edge shared with the
    /// neighbouring tile.
    const EXT_LINK: u16 = 0x8000;

    /// A polygon for [`tile`]: its vertex indices, its `neis`, its flags, and
    /// its detail triangles over those vertices.
    struct Poly {
        verts: Vec<u16>,
        neis: Vec<u16>,
        flags: u16,
        tris: Vec<[u8; 3]>,
        offmesh: bool,
    }

    fn put_u32(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn put_f32(out: &mut Vec<u8>, v: f32) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn pad(out: &mut Vec<u8>) {
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }

    /// A whole `.mmtile` in the layout the module comment describes, with no
    /// links, no extra detail vertices and no bounding tree.
    fn tile(verts: &[[f32; 3]], polys: &[Poly]) -> Vec<u8> {
        let tri_count: usize = polys.iter().map(|p| p.tris.len()).sum();
        let mut d = Vec::new();
        let header: [i32; 15] = [
            DETOUR_MAGIC as i32,
            DETOUR_VERSION as i32,
            12,
            13,
            0,
            0,
            polys.len() as i32,
            verts.len() as i32,
            0,
            polys.len() as i32,
            0,
            tri_count as i32,
            0,
            0,
            0,
        ];
        for v in header {
            put_u32(&mut d, v as u32);
        }
        for v in [1.5, 0.2, 1.8, -533.0, 0.0, -9000.0, 0.0, 100.0, -8533.0, 1.0] {
            put_f32(&mut d, v);
        }
        for v in verts {
            for c in v {
                put_f32(&mut d, *c);
            }
        }
        pad(&mut d);
        for p in polys {
            put_u32(&mut d, 0xffff_ffff);
            for k in 0..VERTS_PER_POLYGON {
                d.extend_from_slice(&p.verts.get(k).copied().unwrap_or(0).to_le_bytes());
            }
            for k in 0..VERTS_PER_POLYGON {
                d.extend_from_slice(&p.neis.get(k).copied().unwrap_or(0).to_le_bytes());
            }
            d.extend_from_slice(&p.flags.to_le_bytes());
            d.push(p.verts.len() as u8);
            d.push(if p.offmesh { 1 << 6 } else { 1 });
        }
        pad(&mut d);
        let mut tri_base = 0u32;
        for p in polys {
            put_u32(&mut d, 0);
            put_u32(&mut d, tri_base);
            d.push(0);
            d.push(p.tris.len() as u8);
            d.extend_from_slice(&[0, 0]);
            tri_base += p.tris.len() as u32;
        }
        pad(&mut d);
        for p in polys {
            for t in &p.tris {
                d.extend_from_slice(t);
                d.push(0);
            }
        }
        pad(&mut d);

        let mut out = Vec::new();
        put_u32(&mut out, MMAP_MAGIC);
        put_u32(&mut out, DETOUR_VERSION);
        put_u32(&mut out, MMAP_VERSION);
        put_u32(&mut out, d.len() as u32);
        put_u32(&mut out, 1);
        out.extend_from_slice(&d);
        out
    }

    fn quad(flags: u16, neis: Vec<u16>) -> Poly {
        Poly {
            verts: vec![0, 1, 2, 3],
            neis,
            flags,
            tris: vec![[0, 1, 2], [0, 2, 3]],
            offmesh: false,
        }
    }

    /// Stored `(y, z, x)`: a square one yard on a side at height 50, whose
    /// world x runs from -9000 to -8999.
    const SQUARE: [[f32; 3]; 4] = [
        [-10.0, 50.0, -9000.0],
        [-9.0, 50.0, -9000.0],
        [-9.0, 50.0, -8999.0],
        [-10.0, 50.0, -8999.0],
    ];

    #[test]
    fn a_stored_vertex_is_read_as_the_world_point() {
        let nav = parse_tile(&tile(&SQUARE, &[quad(NAV_GROUND, vec![0; 4])])).unwrap();
        assert_eq!(nav.grid, (12, 13));
        assert!(nav.uses_liquids);
        let (surface, part) = &nav.parts[0];
        assert_eq!(*surface, Surface::Ground);
        assert_eq!(part.positions[0], [-9000.0, -10.0, 50.0]);
        assert_eq!(part.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(nav.triangles(), 2);
        // The header's box in world axes: stored x is world y.
        assert_eq!(nav.bounds[0], [-9000.0, -533.0, 0.0]);
    }

    #[test]
    fn an_edge_with_no_neighbour_is_an_outline_and_a_portal_is_not() {
        // Edge 0 is inside the tile, edge 1 a portal, edges 2 and 3 open.
        let nav = parse_tile(&tile(&SQUARE, &[quad(NAV_GROUND, vec![2, EXT_LINK | 1, 0, 0])])).unwrap();
        assert_eq!(nav.edges.len(), 2);
        assert_eq!(nav.edges[0], [[-8999.0, -9.0, 50.0], [-8999.0, -10.0, 50.0]]);
    }

    #[test]
    fn polygons_are_grouped_by_what_their_flags_mean_to_the_server() {
        let polys = [
            quad(NAV_GROUND, vec![0; 4]),
            quad(NAV_GROUND | NAV_STEEP_SLOPES, vec![0; 4]),
            quad(NAV_GROUND | NAV_WATER, vec![0; 4]),
            Poly { offmesh: true, ..quad(NAV_GROUND, vec![0; 4]) },
        ];
        let nav = parse_tile(&tile(&SQUARE, &polys)).unwrap();
        let surfaces: Vec<Surface> = nav.parts.iter().map(|(s, _)| *s).collect();
        assert_eq!(surfaces, vec![Surface::Ground, Surface::Steep, Surface::Shore]);
        // The off-mesh connection is neither drawn nor counted as a polygon.
        assert_eq!(nav.polygons, 3);
    }

    #[test]
    fn the_flag_classes_follow_the_generator() {
        assert_eq!(Surface::of(0x01), Surface::Ground);
        assert_eq!(Surface::of(0x11), Surface::Steep);
        assert_eq!(Surface::of(0x09), Surface::Shore);
        assert_eq!(Surface::of(0x08), Surface::Water);
        assert_eq!(Surface::of(0x02), Surface::Magma);
        assert_eq!(Surface::of(0x04), Surface::Slime);
        assert_eq!(Surface::of(0x00), Surface::Unflagged);
    }

    #[test]
    fn a_file_the_layout_does_not_describe_is_refused() {
        let good = tile(&SQUARE, &[quad(NAV_GROUND, vec![0; 4])]);

        let mut magic = good.clone();
        magic[0] = 0;
        assert!(parse_tile(&magic).unwrap_err().contains("magic"));

        // One byte short of what the header says.
        let short = &good[..good.len() - 1];
        assert!(parse_tile(short).is_err());

        // A header size that agrees with the file but not with the sections.
        let mut padded = good.clone();
        padded.extend_from_slice(&[0; 4]);
        let size = (padded.len() - FILE_HEADER) as u32;
        padded[12..16].copy_from_slice(&size.to_le_bytes());
        assert!(parse_tile(&padded).unwrap_err().contains("add up"));

        // A triangle naming a vertex the polygon does not have.
        let bad = tile(
            &SQUARE,
            &[Poly { tris: vec![[0, 1, 7]], ..quad(NAV_GROUND, vec![0; 4]) }],
        );
        assert!(parse_tile(&bad).unwrap_err().contains("names vertex 7"));
    }

    #[test]
    fn the_file_is_named_row_then_column() {
        let path = tile_path(Path::new("D"), Tile::new(0, 32, 48));
        assert!(path.ends_with("mmaps/0004832.mmtile") || path.ends_with("mmaps\\0004832.mmtile"));
    }
}
