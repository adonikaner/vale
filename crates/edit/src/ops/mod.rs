//! What an edit is: applied, inverted, and named.
//!
//! ```text
//! area.rs      the area brush: which AreaTable id each chunk it reaches
//!              carries
//! water.rs     the water brush: water flooded onto chunks or drained from
//!              them
//! brush.rs     the height brush, and the shape, falloff and noise every round
//!              brush shares
//! lattice.rs   the ground under a brush as one grid across chunks and tiles,
//!              with the blur and the erosion that run on it
//! shade.rs     the shading brush: vertex colours painted onto the ground
//! paint.rs     the paint brush: texture layers and their blend maps
//! grade.rs     a ramp between two points, the one shape a round brush cannot
//!              make
//! road.rs      a road along a run of points: the ground graded along it and
//!              its textures painted
//! follow.rs    carrying the placements that stand on ground an edit moved
//! image.rs     a tile's heights and texture blends as pictures, written out
//!              and read back
//! vertices.rs  a set of vertices chosen first and moved together, or kept
//!              from a brush
//! chunks.rs    a selection of map chunks on the map's chunk grid: its census,
//!              its stitch to the ground around it, and copy, turn and paste
//! tests.rs     what a stroke leaves behind, measured on a real tile
//! ```
//!
//! Every change to a tile is recorded as an `Edit` before it is made, with
//! both sides of it in hand. [`crate::undo`] replays it in either direction,
//! and the editor never re-derives what a stroke did: an [`Edit`] carries the
//! bytes before and after, not the operation that produced them.
//!
//! ## Why a height stroke records whole chunks
//!
//! A brush touches an arbitrary set of vertices across up to four chunks, and
//! recording each vertex separately would make an undo a scatter of writes whose
//! order matters. A chunk's heights are 580 bytes; recording all 145 of them
//! twice costs about a kilobyte per chunk per stroke and makes the inverse
//! exact. The same argument holds for a placement, where adding or removing one
//! renumbers every reference in the tile.
//!
//! ## Why a paint stroke records a chunk's whole paint
//!
//! A layer edit changes the length of a region. The blend maps are packed one
//! after another in `MCAL`, so painting with a texture the chunk does not yet
//! carry adds a layer, moves every map after it and renumbers every offset in
//! `MCLY`, and it appends to the tile's `MTEX` as well. There is no smaller
//! unit than "the chunk's paint, and the tile's texture list" that inverts
//! correctly, which is what [`ChunkPaint`] is.

use crate::adt::{heights, holes, AdtFile, Building, Doodad, Region};

/// One reversible change to one tile.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Edit {
    /// One map chunk's 145 heights, in world terms.
    Heights {
        chunk: usize,
        before: Vec<f32>,
        after: Vec<f32>,
    },
    /// One map chunk's `MCNR` payload. Recorded beside the heights that caused
    /// it, so an undo restores the shading and not only the shape.
    Normals {
        chunk: usize,
        before: Vec<u8>,
        after: Vec<u8>,
    },
    /// One placement moved, turned or scaled. Its index does not change, so no
    /// reference list does either.
    Doodad {
        index: usize,
        before: Doodad,
        after: Doodad,
    },
    /// The placement lists and every chunk's references, whole. Adding or
    /// removing one entry renumbers the rest, so there is no smaller unit that
    /// inverts correctly.
    Placements {
        before: Box<Placements>,
        after: Box<Placements>,
    },
    /// One `MODF` building moved or turned. Its index does not change, so no
    /// reference list does either — the same shape as [`Edit::Doodad`], one list
    /// along.
    ///
    /// It has no scale. `MODF` in 1.12 carries a position, three angles, a
    /// world-space box, a doodad set and a name set, and the field later versions
    /// put a scale in is two bytes of padding here. The 1.12.1 client does not
    /// read those bytes, so no tool writes a scale.
    Building {
        index: usize,
        before: Building,
        after: Building,
    },
    /// One map chunk's texture layers and blend maps, whole — see
    /// [`ChunkPaint`], and the module comment for why there is no smaller unit.
    Paint {
        chunk: usize,
        before: Box<ChunkPaint>,
        after: Box<ChunkPaint>,
    },
    /// One map chunk's liquid, whole — see [`ChunkLiquid`], and the module
    /// comment for why there is no smaller unit.
    Liquid {
        chunk: usize,
        before: Box<ChunkLiquid>,
        after: Box<ChunkLiquid>,
    },
    /// One map chunk's area id — one `u32` in its header, an `AreaTable.dbc`
    /// row.
    ///
    /// The only edit in this crate that changes nothing on screen: the renderer
    /// builds nothing from it. `crate::adt::area` describes what reads it.
    Area {
        chunk: usize,
        before: u32,
        after: u32,
    },
    /// One map chunk's `MCCV` — the shading painted onto its 145 vertices,
    /// whole, and `None` when the chunk has no `MCCV`.
    ///
    /// Whole rather than per vertex for the reason [`Edit::Heights`] is: a
    /// stroke touches an arbitrary set of them and 580 bytes recorded twice is
    /// a kilobyte a chunk. The `Option` differs from the heights: no shipped
    /// 1.12 chunk carries this region, so the first stroke on a chunk creates
    /// it, and an undo of that stroke removes it again. See
    /// `crate::adt::colours`.
    Colours {
        chunk: usize,
        before: Option<Vec<u8>>,
        after: Option<Vec<u8>>,
    },
    /// One map chunk's hole mask — sixteen bits, one `u16` in its header.
    ///
    /// The smallest edit in this crate and the only one with no live path at
    /// all: the mask decides how many vertices the chunk contributes, so
    /// changing it changes the length of every draw group after it. See
    /// [`Edit::remeshes`] and `crate::adt::holes`.
    Holes {
        chunk: usize,
        before: u16,
        after: u16,
    },
    /// A map chunk's flags word, whole.
    ///
    /// The word carries the shadow bit, the four liquid bits and the
    /// impassability bit, and an edit that set one by writing a value it had
    /// computed would overwrite the rest. The whole word is recorded before and
    /// after, so undo is exact however many bits an operation changed.
    ///
    /// Only `crate::adt::impass` writes it.
    Flags {
        chunk: usize,
        before: u32,
        after: u32,
    },
    /// The tile's three path lists, whole. See [`TileNames`].
    ///
    /// The unit a find-and-replace over a model or a tileset works in. Whole
    /// rather than "entry 4 of `MMDX` became this", because changing a name's
    /// length moves every offset in `MMID` after it, so the smallest thing that
    /// inverts correctly is the pair of blobs. They are a few hundred bytes to
    /// a few kilobytes, once per tile, against the 580 bytes a height stroke
    /// records per chunk.
    Names {
        before: Box<TileNames>,
        after: Box<TileNames>,
    },
    /// One map chunk's `MCSH`, whole, and `None` when the chunk declares no
    /// shadow; the flags word's shadow bit follows it. What a shadow rebake
    /// writes, so a rebake is an edit like any other: undone, logged and
    /// shared. After every other variant, so an edit stored before it existed
    /// still reads.
    Shadow {
        chunk: usize,
        before: Option<Vec<u8>>,
        after: Option<Vec<u8>>,
    },
}

/// The kinds of edit that meet across a chunk border. See [`Edit::seam`].
/// Heights and their shading are one, since the shading follows the heights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seam {
    Shape,
    Colours,
    Liquid,
    Paint,
}

/// A tile's `MTEX`, `MMDX`/`MMID` and `MWMO`/`MWID`, as they stand.
///
/// Every index in the file is a position in one of these lists — `MCLY` into
/// `MTEX`, `MDDF` into `MMID`, `MODF` into `MWID` — so an operation that keeps
/// the order of all three can change the names and renumber nothing. A
/// map-wide swap is therefore one undo entry per tile rather than a rewrite of
/// every record on it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TileNames {
    textures: Vec<u8>,
    models: Vec<u8>,
    model_offsets: Vec<u8>,
    buildings: Vec<u8>,
    building_offsets: Vec<u8>,
}

impl TileNames {
    pub fn capture(tile: &AdtFile) -> TileNames {
        TileNames {
            textures: tile.textures.clone(),
            models: tile.models.clone(),
            model_offsets: tile.model_offsets.clone(),
            buildings: tile.buildings.clone(),
            building_offsets: tile.building_offsets.clone(),
        }
    }

    pub fn restore(&self, tile: &mut AdtFile) {
        tile.textures.clone_from(&self.textures);
        tile.models.clone_from(&self.models);
        tile.model_offsets.clone_from(&self.model_offsets);
        tile.buildings.clone_from(&self.buildings);
        tile.building_offsets.clone_from(&self.building_offsets);
    }
}

/// One map chunk's paint, and the tile's texture list with it.
///
/// `MCLY`, `MCAL`, and the header's layer count and detail-layer map are the
/// chunk's; `MTEX` is the tile's and is in here anyway, because painting with a
/// texture the tile does not name appends to it. A stroke that crosses several
/// chunks therefore carries several copies of a list a few hundred bytes long,
/// and reverting them in the order [`Change::revert`] already uses, last first,
/// leaves the earliest `before` standing, which is the list as it was when the
/// stroke began.
///
/// `MCLY` and `MCAL` are `None` when the chunk has no such region. Every
/// shipped 1.12 chunk has both, empty or not, but a chunk made by
/// `crate::adt::blank` has no `MCAL`, and painting it creates one. A restore
/// removes a region the capture did not find, so an undone stroke writes the
/// tile back byte for byte.
///
/// [`Change::revert`]: crate::undo::Change::revert
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChunkPaint {
    mcly: Option<Vec<u8>>,
    mcal: Option<Vec<u8>>,
    layer_count: u32,
    detail_layers: [u16; 8],
    textures: Vec<u8>,
}

impl ChunkPaint {
    pub fn capture(tile: &AdtFile, index: usize) -> ChunkPaint {
        let region = |region: Region| {
            tile.chunk(index)
                .and_then(|chunk| chunk.region(region))
                .map(|sub| sub.data.clone())
        };
        ChunkPaint {
            mcly: region(Region::Layers),
            mcal: region(Region::Alpha),
            layer_count: tile
                .chunk(index)
                .map(|chunk| chunk.head().layer_count())
                .unwrap_or(0),
            detail_layers: tile
                .chunk(index)
                .map(|chunk| chunk.head().detail_layers())
                .unwrap_or_default(),
            textures: tile.textures.clone(),
        }
    }

    pub fn restore(&self, tile: &mut AdtFile, index: usize) {
        tile.textures = self.textures.clone();
        let Some(chunk) = tile.chunk_mut(index) else {
            return;
        };
        for (region, data) in [
            (Region::Layers, &self.mcly),
            (Region::Alpha, &self.mcal),
        ] {
            match (chunk.region_mut(region), data) {
                (Some(sub), Some(data)) => sub.set(data.clone()),
                (None, Some(data)) => {
                    chunk.regions[region as usize] =
                        Some(crate::adt::SubChunk::new(data.clone()))
                }
                (_, None) => chunk.regions[region as usize] = None,
            }
        }
        let mut head = chunk.head_mut();
        head.set_layer_count(self.layer_count);
        head.set_detail_layers(self.detail_layers);
    }
}

/// One map chunk's liquid, and the header flags that declare it.
///
/// Both are recorded because each holds half of the description. `MCLQ` holds
/// one block per liquid and says nothing about how many or of what type; the
/// `MCNK` flags say that and hold no geometry. An undo that put the bytes back
/// and left the flags would describe a chunk with water nothing reads, or read
/// a block that is not there. See `crate::adt::liquid`.
///
/// The whole region rather than one cell, for the reason [`ChunkPaint`] is the
/// whole of a chunk's paint: giving a dry chunk water adds 804 bytes in the
/// middle of a two-megabyte file, and there is no smaller unit that inverts.
///
/// `MCLQ` is `None` when the chunk has no such region. Every shipped 1.12
/// chunk has one, empty when it is dry, but a chunk made by
/// `crate::adt::blank` has none, and giving it water creates one. A restore
/// removes a region the capture did not find, so an undone stroke writes the
/// tile back byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChunkLiquid {
    mclq: Option<Vec<u8>>,
    /// The size field as the file wrote it, which for `MCLQ` is zero and
    /// is not derivable from the length. See
    /// `crate::adt::SubChunk::set_keeping_size`.
    declared: u32,
    flags: u32,
}

impl ChunkLiquid {
    pub fn capture(tile: &AdtFile, index: usize) -> ChunkLiquid {
        let chunk = tile.chunk(index);
        let region = chunk.and_then(|chunk| chunk.region(Region::Liquid));
        ChunkLiquid {
            mclq: region.map(|sub| sub.data.clone()),
            declared: region.map(|sub| sub.declared).unwrap_or(0),
            flags: chunk.map(|chunk| chunk.head().flags()).unwrap_or(0),
        }
    }

    pub fn restore(&self, tile: &mut AdtFile, index: usize) {
        let Some(chunk) = tile.chunk_mut(index) else {
            return;
        };
        chunk.head_mut().set_flags(self.flags);
        match (chunk.region_mut(Region::Liquid), &self.mclq) {
            (Some(sub), Some(data)) => {
                sub.set_keeping_size(data.clone());
                sub.declared = self.declared;
            }
            (None, Some(data)) => {
                let mut sub = crate::adt::SubChunk::new(data.clone());
                sub.declared = self.declared;
                chunk.regions[Region::Liquid as usize] = Some(sub);
            }
            (_, None) => chunk.regions[Region::Liquid as usize] = None,
        }
    }
}

/// Everything a placement change touches, captured whole.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Placements {
    doodads: Vec<u8>,
    placements: Vec<u8>,
    models: Vec<u8>,
    model_offsets: Vec<u8>,
    buildings: Vec<u8>,
    building_offsets: Vec<u8>,
    /// One `MCRF` payload per chunk, with the header's two counts beside it.
    refs: Vec<(Vec<u8>, u32, u32)>,
}

impl Placements {
    pub fn capture(tile: &AdtFile) -> Placements {
        Placements {
            doodads: tile.doodads.clone(),
            placements: tile.placements.clone(),
            models: tile.models.clone(),
            model_offsets: tile.model_offsets.clone(),
            buildings: tile.buildings.clone(),
            building_offsets: tile.building_offsets.clone(),
            refs: tile
                .chunks
                .iter()
                .map(|chunk| {
                    let head = chunk.head();
                    (
                        chunk
                            .region(Region::Refs)
                            .map(|sub| sub.data.clone())
                            .unwrap_or_default(),
                        head.doodad_ref_count(),
                        head.building_ref_count(),
                    )
                })
                .collect(),
        }
    }

    pub fn restore(&self, tile: &mut AdtFile) {
        tile.doodads = self.doodads.clone();
        tile.placements = self.placements.clone();
        tile.models = self.models.clone();
        tile.model_offsets = self.model_offsets.clone();
        tile.buildings = self.buildings.clone();
        tile.building_offsets = self.building_offsets.clone();
        for (i, (data, doodads, buildings)) in self.refs.iter().enumerate() {
            let Some(chunk) = tile.chunk_mut(i) else {
                continue;
            };
            match chunk.region_mut(Region::Refs) {
                Some(mcrf) => mcrf.set(data.clone()),
                None => {
                    chunk.regions[Region::Refs as usize] =
                        Some(crate::adt::SubChunk::new(data.clone()))
                }
            }
            let mut head = chunk.head_mut();
            head.set_doodad_ref_count(*doodads);
            head.set_building_ref_count(*buildings);
        }
    }
}

impl Edit {
    /// Write the `after` side into the tile.
    pub fn apply(&self, tile: &mut AdtFile) {
        match self {
            Edit::Heights { chunk, after, .. } => write_heights(tile, *chunk, after),
            Edit::Normals { chunk, after, .. } => write_normals(tile, *chunk, after),
            Edit::Doodad { index, after, .. } => write_doodad(tile, *index, *after),
            Edit::Building { index, after, .. } => write_building(tile, *index, *after),
            Edit::Placements { after, .. } => after.restore(tile),
            Edit::Paint { chunk, after, .. } => after.restore(tile, *chunk),
            Edit::Holes { chunk, after, .. } => write_holes(tile, *chunk, *after),
            Edit::Flags { chunk, after, .. } => write_flags(tile, *chunk, *after),
            Edit::Area { chunk, after, .. } => write_area(tile, *chunk, *after),
            Edit::Liquid { chunk, after, .. } => after.restore(tile, *chunk),
            Edit::Colours { chunk, after, .. } => {
                crate::adt::colours::restore(tile, *chunk, after.as_ref())
            }
            Edit::Names { after, .. } => after.restore(tile),
            Edit::Shadow { chunk, after, .. } => {
                crate::shadow::set_shadow(tile, *chunk, after.as_deref());
            }
        }
    }

    /// Write the `before` side into the tile.
    pub fn revert(&self, tile: &mut AdtFile) {
        match self {
            Edit::Heights { chunk, before, .. } => write_heights(tile, *chunk, before),
            Edit::Normals { chunk, before, .. } => write_normals(tile, *chunk, before),
            Edit::Doodad { index, before, .. } => write_doodad(tile, *index, *before),
            Edit::Building { index, before, .. } => write_building(tile, *index, *before),
            Edit::Placements { before, .. } => before.restore(tile),
            Edit::Paint { chunk, before, .. } => before.restore(tile, *chunk),
            Edit::Holes { chunk, before, .. } => write_holes(tile, *chunk, *before),
            Edit::Flags { chunk, before, .. } => write_flags(tile, *chunk, *before),
            Edit::Area { chunk, before, .. } => write_area(tile, *chunk, *before),
            Edit::Liquid { chunk, before, .. } => before.restore(tile, *chunk),
            Edit::Colours { chunk, before, .. } => {
                crate::adt::colours::restore(tile, *chunk, before.as_ref())
            }
            Edit::Names { before, .. } => before.restore(tile),
            Edit::Shadow { chunk, before, .. } => {
                crate::shadow::set_shadow(tile, *chunk, before.as_deref());
            }
        }
    }

    /// Which map chunk's ground this edit moved, for a caller that patches
    /// vertices. `None` for a placement change, which is about the whole tile,
    /// and `None` for a paint change, which moves no vertex: a repaint is
    /// reported by [`Edit::painted`] and takes a different route to the screen.
    pub fn chunk(&self) -> Option<usize> {
        match self {
            // A colour edit is a vertex edit, which is why it is cheap: `MCCV`
            // is one length or absent, so the mesh keeps its shape, its indices
            // and its length and only an attribute changes. It goes down the
            // same live path a height stroke does.
            Edit::Heights { chunk, .. }
            | Edit::Normals { chunk, .. }
            | Edit::Colours { chunk, .. } => Some(*chunk),
            Edit::Doodad { .. }
            | Edit::Building { .. }
            | Edit::Placements { .. }
            | Edit::Paint { .. }
            // A hole edit moves no vertex. It takes cells out of the mesh, so
            // a caller that patched the chunk's vertices from this would write
            // 320 positions into a run that is now a different length. See
            // [`Edit::remeshes`].
            | Edit::Holes { .. }
            | Edit::Flags { .. }
            // An area id is not held by anything on screen, and a liquid is a
            // surface built when the tile is read. See [`Edit::remeshes`].
            | Edit::Area { .. }
            | Edit::Liquid { .. }
            // A path list is about the whole tile, so it remeshes rather than
            // patches. See [`Edit::remeshes`].
            | Edit::Names { .. }
            // A shadow is folded into the ground's shading when a draw group
            // is built, so the tile is read again. See [`Edit::remeshes`].
            | Edit::Shadow { .. } => None,
        }
    }

    /// Which map chunk's paint this edit changed, for a caller that has the
    /// blend maps on the GPU.
    pub fn painted(&self) -> Option<usize> {
        match self {
            Edit::Paint { chunk, .. } => Some(*chunk),
            _ => None,
        }
    }

    /// Whether this edit changes which textures a chunk carries, rather than
    /// only how much of each is showing.
    ///
    /// The two cases reach the screen by different routes. A chunk's blend maps
    /// are texels in an atlas and can be written into the one already drawn;
    /// the set of textures a chunk names is part of the material its draw group
    /// was built with, so changing it means the tile has to be read again.
    pub fn changes_the_texture_set(&self) -> bool {
        match self {
            Edit::Paint { before, after, .. } => before.mcly != after.mcly,
            _ => false,
        }
    }

    /// Which `MDDF` entry this edit is about, for a caller that has that
    /// placement drawn and wants to move what is on screen rather than read the
    /// tile again.
    ///
    /// `None` for everything else, including [`Edit::Placements`]: adding
    /// or removing one entry renumbers the rest, so there is no single index
    /// that names what changed and a caller has to reconcile the whole list.
    pub fn placement(&self) -> Option<usize> {
        match self {
            Edit::Doodad { index, .. } => Some(*index),
            _ => None,
        }
    }

    /// Which `MODF` entry this edit is about, on the same terms as
    /// [`Edit::placement`].
    ///
    /// A separate accessor and not a shared one, because the two lists are
    /// numbered separately: `MDDF` entry 3 and `MODF` entry 3 are different
    /// things, and a caller that reconciles them from one list of indices moves
    /// the wrong object.
    pub fn building(&self) -> Option<usize> {
        match self {
            Edit::Building { index, .. } => Some(*index),
            _ => None,
        }
    }

    /// One placement moved, with `before` read from the tile rather than
    /// supplied by the caller.
    ///
    /// ## Why `before` is read from the tile
    ///
    /// A change is only reversible if its `before` is the state the file was
    /// in when the change was made. A caller that supplies `before` supplies
    /// its own copy of that state, and in the editor such a copy can go stale
    /// without notice: an undo puts the file back without telling the panel
    /// holding a copy of the record, a tile is read again and every cached
    /// record with it, a second tool writes the same placement.
    ///
    /// A stale `before` does not crash and does not draw a wrong picture. It
    /// breaks the chain of the undo stack: entry n's `before` is no longer
    /// entry n-1's `after`, so undoing them in order walks through states the
    /// file was never in and does not arrive back at the start. The visible
    /// symptom is a history that gets out of sync after a few moves.
    ///
    /// Reading `before` off the tile keeps the stack chained by construction,
    /// without each of the four callers having to keep its copy current.
    ///
    /// `None` when the index is not in the list, which is a caller holding an
    /// index the tile no longer has.
    pub fn move_doodad(tile: &AdtFile, index: usize, after: Doodad) -> Option<Edit> {
        let before = tile.doodad_at(index)?;
        (before != after).then_some(Edit::Doodad {
            index,
            before,
            after,
        })
    }

    /// One building moved, on the terms [`Edit::move_doodad`] sets out:
    /// `before` read from the tile, so the stack chains by construction.
    pub fn move_building(tile: &AdtFile, index: usize, after: Building) -> Option<Edit> {
        let before = tile.building_at(index)?;
        (before != after).then_some(Edit::Building {
            index,
            before,
            after,
        })
    }

    /// The same edit with its starting state read from `tile`: the `before`
    /// when it is applied forward, the `after` when it is reverted.
    ///
    /// For an edit made on a state that has changed since. Every side of an
    /// edit is a whole state of its subject, so the rebased edit takes the
    /// subject to the state the edit meant, and its starting side is the state
    /// it is applied over, so it can be undone. `None` when the subject is not
    /// in the tile: a chunk it does not have, or a placement index past the
    /// end of its list.
    ///
    /// Heights and vertex colours are the exception: the other side becomes
    /// the values there now plus the change the edit made at each vertex. A
    /// vertex on a chunk border is stored in both chunks and a stroke changes
    /// both copies by the same amount, so adding the change keeps the copies
    /// equal over ground someone else has changed since, where writing the
    /// edit's own values would leave a step against the chunk next to it. A
    /// colour is clamped to 0..=255. Colours that a side of the edit does not
    /// have, or have a length that differs, are rebased as a whole state. A
    /// [`Edit::Normals`] is rebased with [`Edit::reshaded`] instead, since
    /// shading follows the heights.
    pub fn rebased(&self, forward: bool, tile: &AdtFile) -> Option<Edit> {
        fn from<'a, T>(forward: bool, before: &'a mut T, after: &'a mut T) -> &'a mut T {
            match forward {
                true => before,
                false => after,
            }
        }
        let mut out = self.clone();
        match &mut out {
            Edit::Heights { chunk, before, after } => {
                let now = heights::heights(tile.chunk(*chunk)?);
                let (start, end) = match forward {
                    true => (before, after),
                    false => (after, before),
                };
                if start.len() == now.len() && end.len() == now.len() {
                    *end = now.iter().zip(start.iter()).zip(end.iter()).map(|((now, start), end)| now + (end - start)).collect();
                }
                *start = now;
            }
            Edit::Normals { chunk, before, after } => *from(forward, before, after) = normals_of(tile, *chunk)?,
            // A placement is found by its `uniqueId`, since placements another
            // editor added or removed since move its index. Its model stays
            // the one it has now unless the edit changes the model.
            Edit::Doodad { index, before, after } => {
                let at = *tile.doodads_with_id(before.unique_id).first()?;
                let now = tile.doodad_at(at)?;
                let (start, end) = match forward {
                    true => (before, after),
                    false => (after, before),
                };
                if start.name_id == end.name_id {
                    end.name_id = now.name_id;
                }
                *start = now;
                *index = at;
            }
            Edit::Building { index, before, after } => {
                let at = *tile.buildings_with_id(before.unique_id).first()?;
                let now = tile.building_at(at)?;
                let (start, end) = match forward {
                    true => (before, after),
                    false => (after, before),
                };
                if start.name_id == end.name_id {
                    end.name_id = now.name_id;
                }
                *start = now;
                *index = at;
            }
            // The lists are merged by `uniqueId` (`AdtFile::merge_placements`),
            // so what another editor placed or removed since stays.
            Edit::Placements { before, after } => {
                let (start, end) = match forward {
                    true => (before.clone(), after.clone()),
                    false => (after.clone(), before.clone()),
                };
                let mut start_tile = tile.clone();
                start.restore(&mut start_tile);
                let mut end_tile = tile.clone();
                end.restore(&mut end_tile);
                let mut merged = tile.clone();
                merged.merge_placements(&start_tile, &end_tile);
                let (now, made) = (Box::new(Placements::capture(tile)), Box::new(Placements::capture(&merged)));
                match forward {
                    true => (*before, *after) = (now, made),
                    false => (*after, *before) = (now, made),
                }
            }
            Edit::Paint { chunk, before, after } => {
                tile.chunk(*chunk)?;
                *from(forward, before, after) = Box::new(ChunkPaint::capture(tile, *chunk));
            }
            Edit::Liquid { chunk, before, after } => {
                tile.chunk(*chunk)?;
                *from(forward, before, after) = Box::new(ChunkLiquid::capture(tile, *chunk));
            }
            Edit::Area { chunk, before, after } => *from(forward, before, after) = crate::adt::area::area(tile, *chunk)?,
            Edit::Shadow { chunk, before, after } => {
                let now = tile.chunk(*chunk)?.region(Region::Shadow).map(|sub| sub.data.clone());
                *from(forward, before, after) = now;
            }
            Edit::Colours { chunk, before, after } => {
                tile.chunk(*chunk)?;
                let now = crate::adt::colours::capture(tile, *chunk);
                let (start, end) = match forward {
                    true => (before, after),
                    false => (after, before),
                };
                if let (Some(now), Some(had), Some(made)) = (&now, start.as_ref(), end.as_mut()) {
                    if had.len() == now.len() && made.len() == now.len() {
                        for ((made, had), now) in made.iter_mut().zip(had).zip(now) {
                            *made = (i16::from(*now) + i16::from(*made) - i16::from(*had)).clamp(0, 255) as u8;
                        }
                    }
                }
                *start = now;
            }
            Edit::Holes { chunk, before, after } => *from(forward, before, after) = holes::holes(tile, *chunk)?,
            Edit::Flags { chunk, before, after } => *from(forward, before, after) = tile.chunk(*chunk)?.head().flags(),
            Edit::Names { before, after } => *from(forward, before, after) = Box::new(TileNames::capture(tile)),
        }
        Some(out)
    }

    /// A [`Edit::Normals`] rebased over `tile`, whose heights are already the
    /// ones it is to shade: the starting side is the shading there now, and
    /// the other side is that chunk's shading recomputed, which is also
    /// written to `tile`. `None` for any other edit, or a chunk the tile does
    /// not have. See [`Edit::rebased`].
    pub fn reshaded(&self, forward: bool, tile: &mut AdtFile) -> Option<Edit> {
        let Edit::Normals { chunk, .. } = self else { return None };
        let now = normals_of(tile, *chunk)?;
        heights::recompute_normals(tile, *chunk);
        let shaded = normals_of(tile, *chunk)?;
        let (before, after) = match forward {
            true => (now, shaded),
            false => (shaded, now),
        };
        Some(Edit::Normals { chunk: *chunk, before, after })
    }

    /// What this edit changes that meets the chunk next to it, or `None`.
    /// Neighbouring chunks each store the vertices, liquid corners and blend
    /// map edge on their shared border, and a stroke changes both copies, so a
    /// step's edits of one [`Seam`] are applied all together or not at all:
    /// half of them leave a step, a line of colour or a seam in the paint at
    /// the border between the half made and the half not made.
    pub fn seam(&self) -> Option<Seam> {
        match self {
            Edit::Heights { .. } | Edit::Normals { .. } => Some(Seam::Shape),
            Edit::Colours { .. } => Some(Seam::Colours),
            Edit::Liquid { .. } => Some(Seam::Liquid),
            Edit::Paint { .. } => Some(Seam::Paint),
            _ => None,
        }
    }

    /// Whether this edit renumbers the placement lists, so a caller holding an
    /// index into them has to let go of it.
    pub(crate) fn renumbers_placements(&self) -> bool {
        matches!(self, Edit::Placements { .. })
    }

    /// Whether the tile has to be read again for this edit to be on screen,
    /// because what it changed is the ground mesh itself rather than anything
    /// the mesh holds.
    ///
    /// The same question [`Edit::changes_the_texture_set`] and
    /// [`Edit::renumbers_placements`] answer for paint and placements. A hole
    /// mask decides how many vertices a chunk contributes, so no patch can
    /// apply it: not to the vertices (the run changed length), not to the
    /// indices (every group after it moved), not to the foliage (its cells are
    /// gone).
    ///
    /// If this answers `false` and the tile is patched anyway, its draw ranges
    /// name other chunks' vertices and the ground draws folded through itself.
    pub fn remeshes(&self) -> bool {
        // Liquid remeshes as well as holes: a tile's water is a mesh built at
        // load from `MCLQ`, one quad per wet cell merged into one draw per
        // liquid type, so a cell going wet or dry changes the geometry.
        // A rename remeshes too: the paths a tile names reach the screen as
        // the textures a draw group's material was built with and the model
        // each placement resolved to, and the drawn tile holds a reference to
        // neither.
        // A shadow too: `MCSH` is folded into the vertex shading when a
        // draw group is built.
        matches!(
            self,
            Edit::Holes { .. } | Edit::Liquid { .. } | Edit::Names { .. } | Edit::Shadow { .. }
        )
    }

}

fn write_area(tile: &mut AdtFile, chunk: usize, id: u32) {
    if let Some(chunk) = tile.chunk_mut(chunk) {
        crate::adt::area::set_area(chunk, id);
    }
}

/// Put a chunk's whole flags word back.
fn write_flags(tile: &mut AdtFile, chunk: usize, flags: u32) {
    if let Some(chunk) = tile.chunks.get_mut(chunk) {
        chunk.head_mut().set_flags(flags);
    }
}

fn write_holes(tile: &mut AdtFile, chunk: usize, mask: u16) {
    if let Some(chunk) = tile.chunk_mut(chunk) {
        holes::set_holes(chunk, mask);
    }
}

fn write_heights(tile: &mut AdtFile, chunk: usize, values: &[f32]) {
    if let Some(chunk) = tile.chunk_mut(chunk) {
        heights::set_heights(chunk, values);
    }
}

fn write_normals(tile: &mut AdtFile, chunk: usize, data: &[u8]) {
    if let Some(mcnr) = tile
        .chunk_mut(chunk)
        .and_then(|c| c.region_mut(Region::Normals))
    {
        mcnr.set(data.to_vec());
    }
}

fn write_doodad(tile: &mut AdtFile, index: usize, value: Doodad) {
    let mut list = tile.doodad_list();
    if index < list.len() {
        list[index] = value;
        tile.set_doodad_list(&list);
    }
}

fn write_building(tile: &mut AdtFile, index: usize, value: Building) {
    let mut list = tile.building_list();
    if index < list.len() {
        list[index] = value;
        tile.set_building_list(&list);
    }
}

fn normals_of(tile: &AdtFile, chunk: usize) -> Option<Vec<u8>> {
    tile.chunk(chunk)
        .and_then(|c| c.region(Region::Normals))
        .map(|sub| sub.data.clone())
}

/// Recompute the shading of every chunk a run of height edits moved, and
/// record it beside them.
///
/// `edits` is read for which chunks moved and appended to with one
/// [`Edit::Normals`] per chunk whose `MCNR` actually changed.
///
/// Two properties, each needed to avoid a visible fault:
///
/// * The four neighbours are reshaded as well as the chunk itself.
///   `heights::recompute_normals` takes a central difference, so a vertex on a
///   chunk's edge is a vertex of the chunk beside it too. Reshading only what
///   moved leaves a lit seam along every boundary the edit reached.
/// * The result is recorded. A height edit that recomputes normals without
///   putting them on the stack is a change undo cannot take back, so the ground
///   returns to its old shape with the shading of the new one: flat ground lit
///   as though a ramp were still cut into it.
///
/// It is shared because more than one operation moves the ground (see
/// [`Brush::stroke`](brush::Brush::stroke) and [`grade::Grade::write_into`]),
/// and each of them needs both properties.
fn reshade(tile: &mut AdtFile, edits: &mut Vec<Edit>) {
    // `Edit::Heights` and not `Edit::chunk`, which also answers for
    // `Edit::Normals` and `Edit::Colours`. Only a height moves a normal, and a
    // caller handing over a mixed list should not have the rest of it
    // reshaded.
    let changed: Vec<usize> = edits
        .iter()
        .filter_map(|edit| match edit {
            Edit::Heights { chunk, .. } => Some(*chunk),
            _ => None,
        })
        .collect();
    let mut shade: Vec<usize> = Vec::new();
    for &index in &changed {
        for neighbour in with_neighbours(index) {
            if !shade.contains(&neighbour) {
                shade.push(neighbour);
            }
        }
    }
    for index in shade {
        let Some(before) = normals_of(tile, index) else {
            continue;
        };
        heights::recompute_normals(tile, index);
        let after = normals_of(tile, index).unwrap_or_default();
        if after != before {
            edits.push(Edit::Normals {
                chunk: index,
                before,
                after,
            });
        }
    }
}

/// A chunk and the four next to it inside the same tile, by `MCIN` index.
fn with_neighbours(index: usize) -> Vec<usize> {
    let (x, y) = vale_assets::world::adt::chunk_xy(index);
    [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)]
        .into_iter()
        .filter_map(|(dx, dy)| vale_assets::world::adt::chunk_on_grid(x as i32 + dx, y as i32 + dy))
        .collect()
}

/// The area brush.
pub mod area;

/// The water brush.
pub mod water;

/// The height brush, and the shape, falloff and noise every round brush
/// shares.
pub mod brush;

/// The ground under a brush as one grid of heights across chunks and tiles,
/// for the modes that read a vertex's neighbours.
pub mod lattice;

/// The shading brush.
pub mod shade;

/// The paint brush.
pub mod paint;

/// A ramp between two points — the one shape a round brush cannot make.
pub mod grade;

/// A road along a run of points: the ground graded along it and its
/// textures painted.
pub mod road;

/// Carrying the placements that stand on ground an edit moved.
pub mod follow;

/// A tile's heights and texture blends as pictures, written out and read back.
pub mod image;

/// A set of vertices chosen first and moved together, or kept from a brush.
pub mod vertices;

/// A selection of map chunks: its census, its stitch, and copy and paste.
pub mod chunks;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod unit {
    use super::*;

    /// A chunk on the edge of the tile has three neighbours, one in the corner
    /// has two, and one in the middle has four.
    #[test]
    fn neighbours_stop_at_the_tile_border() {
        assert_eq!(with_neighbours(0).len(), 3);
        assert_eq!(with_neighbours(15).len(), 3);
        assert_eq!(with_neighbours(255).len(), 3);
        assert_eq!(with_neighbours(17 * 4).len(), 5);
    }

    /// A rebased edit starts from what the tile holds. A height edit adds the
    /// change it made to the heights there now; going back, the side it
    /// starts from is the `after`. A chunk the tile does not have cannot be
    /// rebased.
    #[test]
    fn a_rebased_edit_starts_from_the_tile() {
        let mut tile = crate::adt::blank::blank_tile(32, 48, r"Tileset\Elwynn\ElwynnGrass01.blp", 10.0, 12);
        let made = Edit::Heights { chunk: 5, before: vec![0.0; 145], after: vec![3.0; 145] };
        let now = heights::heights(tile.chunk(5).unwrap());
        let raised: Vec<f32> = now.iter().map(|height| height + 3.0).collect();
        let rebased = made.rebased(true, &tile).unwrap();
        assert_eq!(rebased, Edit::Heights { chunk: 5, before: now.clone(), after: raised.clone() });
        rebased.apply(&mut tile);
        assert_eq!(heights::heights(tile.chunk(5).unwrap()), raised);
        rebased.revert(&mut tile);
        assert_eq!(heights::heights(tile.chunk(5).unwrap()), now);

        let lowered: Vec<f32> = now.iter().map(|height| height - 3.0).collect();
        let back = made.rebased(false, &tile).unwrap();
        assert_eq!(back, Edit::Heights { chunk: 5, before: lowered, after: now });
        let missing = Edit::Holes { chunk: 300, before: 0, after: 1 };
        assert_eq!(missing.rebased(true, &tile), None);
    }

    /// Shading rebased over moved heights is the shading of those heights,
    /// written to the tile, and starts from the shading there before.
    #[test]
    fn rebased_shading_follows_the_heights() {
        let mut tile = crate::adt::blank::blank_tile(32, 48, r"Tileset\Elwynn\ElwynnGrass01.blp", 10.0, 12);
        let flat = normals_of(&tile, 5).unwrap();
        let mut slope = heights::heights(tile.chunk(5).unwrap());
        for (index, height) in slope.iter_mut().enumerate() {
            *height += index as f32;
        }
        Edit::Heights { chunk: 5, before: heights::heights(tile.chunk(5).unwrap()), after: slope }.apply(&mut tile);
        let made = Edit::Normals { chunk: 5, before: flat.clone(), after: flat.clone() };
        let Some(Edit::Normals { before, after, .. }) = made.reshaded(true, &mut tile) else { panic!("not shading") };
        assert_eq!(before, flat);
        assert_ne!(after, flat, "the slope is shaded");
        assert_eq!(normals_of(&tile, 5).unwrap(), after);
        assert!(Edit::Holes { chunk: 5, before: 0, after: 1 }.reshaded(true, &mut tile).is_none());
    }

    /// A placement edit rebased over a tile where another editor placed a
    /// doodad since keeps that doodad, and adds and moves its own by
    /// `uniqueId`, whatever index each has now.
    #[test]
    fn a_rebased_placement_edit_keeps_what_was_placed_since() {
        use crate::adt::place::Doodad;
        let doodad = |unique_id: u32, x: f32| Doodad {
            name_id: 0,
            unique_id,
            position: [x, 0.0, 0.0],
            rotation: [0.0; 3],
            scale: 1024,
            flags: 0,
        };
        let mut start = crate::adt::blank::blank_tile(32, 48, r"Tileset\Elwynn\ElwynnGrass01.blp", 10.0, 12);
        start.name_model(r"World\Tree.m2");
        start.add_doodad(doodad(1, 10.0), 2.0);
        let mut end = start.clone();
        end.name_model(r"World\Rock.m2");
        end.add_doodad(Doodad { name_id: 1, ..doodad(2, 20.0) }, 2.0);
        let made = Edit::Placements { before: Box::new(Placements::capture(&start)), after: Box::new(Placements::capture(&end)) };

        // Another editor's doodad, placed first, with a model of its own.
        let mut now = start.clone();
        now.name_model(r"World\Bush.m2");
        now.add_doodad(Doodad { name_id: 1, ..doodad(3, 30.0) }, 2.0);
        let rebased = made.rebased(true, &now).unwrap();
        rebased.apply(&mut now);
        let ids: Vec<u32> = now.doodad_list().iter().map(|placed| placed.unique_id).collect();
        assert_eq!(ids, vec![1, 3, 2]);
        let names = now.model_names();
        let rock = now.doodad_list()[2].name_id as usize;
        assert_eq!(names[rock], r"World\Rock.m2");
        rebased.revert(&mut now);
        assert_eq!(now.doodad_list().len(), 2);

        // A move of doodad 1 recorded at index 0 finds it at index 1.
        let mut moved = start.clone();
        moved.remove_doodad(0);
        moved.add_doodad(doodad(9, 0.0), 2.0);
        moved.add_doodad(doodad(1, 10.0), 2.0);
        let nudge = Edit::Doodad { index: 0, before: doodad(1, 10.0), after: doodad(1, 15.0) };
        let Some(Edit::Doodad { index, .. }) = nudge.rebased(true, &moved) else { panic!("not rebased") };
        assert_eq!(index, 1);
    }
}
