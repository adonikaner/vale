//! What an edit is: applied, inverted, and named.
//!
//! ```text
//! area.rs      the area brush: which AreaTable id each chunk it reaches
//!              carries
//! water.rs     the water brush: water flooded onto chunks or drained from
//!              them
//! brush.rs     the height brush, and the shape, falloff and noise every round
//!              brush shares
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
//! both sides of it in hand. That is what [`crate::undo`] replays in either
//! direction, and it is why the editor never has to re-derive what a stroke did:
//! an [`Edit`] carries the bytes, not the intent.
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
//! ## …and why a paint stroke records the whole of a chunk's paint
//!
//! For a third reason, which is stronger than either: **a layer edit changes the
//! length of a region.** The blend maps are packed one after another in `MCAL`,
//! so painting with a texture the chunk does not yet carry adds a layer, moves
//! every map after it and renumbers every offset in `MCLY` — and it appends to
//! the tile's `MTEX` as well. There is no smaller unit than "the chunk's paint,
//! and the tile's texture list" that inverts correctly, which is what
//! [`ChunkPaint`] is.

use crate::adt::{heights, holes, AdtFile, Building, Doodad, Region};

/// One reversible change to one tile.
#[derive(Debug, Clone, PartialEq)]
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
    /// **It has no scale.** `MODF` in 1.12 carries a position, three angles, a
    /// world-space box, a doodad set and a name set, and the field later versions
    /// put a scale in is two bytes of padding here. A tool that offered one would
    /// be offering to write a number the reference client does not read.
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
    /// One map chunk's **liquid**, whole — see [`ChunkLiquid`], and the module
    /// comment for why there is no smaller unit.
    Liquid {
        chunk: usize,
        before: Box<ChunkLiquid>,
        after: Box<ChunkLiquid>,
    },
    /// One map chunk's area id — one `u32` in its header, an `AreaTable.dbc`
    /// row.
    ///
    /// **The one edit in this crate that reaches the screen by no route at
    /// all**: nothing the renderer builds is made of it. See
    /// `crate::adt::area`, where what *does* read it is.
    Area {
        chunk: usize,
        before: u32,
        after: u32,
    },
    /// One map chunk's `MCCV` — the shading painted onto its 145 vertices,
    /// whole, and **`None` for the chunk having none at all**.
    ///
    /// Whole rather than per vertex for the reason [`Edit::Heights`] is: a
    /// stroke touches an arbitrary set of them and 580 bytes recorded twice is
    /// a kilobyte a chunk. The `Option` is the part that is not like the
    /// heights — no shipped 1.12 chunk carries this region, so *creating* it is
    /// half of what the first stroke on a chunk does and an undo has to be able
    /// to take it away again. See `crate::adt::colours`.
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
    /// **A map chunk's flags word, whole.**
    ///
    /// The word carries the shadow bit, the four liquid bits and the
    /// impassability bit, and an edit that set one by writing a value it had
    /// computed would clobber the rest. So what is recorded is the *whole* word
    /// before and after, which makes undo exact however many bits an operation
    /// happened to move.
    ///
    /// Today only `crate::adt::impass` writes it.
    Flags {
        chunk: usize,
        before: u32,
        after: u32,
    },
    /// **The tile's three path lists, whole** — see [`TileNames`].
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
}

/// A tile's `MTEX`, `MMDX`/`MMID` and `MWMO`/`MWID`, as they stand.
///
/// Every index in the file is a position in one of these lists — `MCLY` into
/// `MTEX`, `MDDF` into `MMID`, `MODF` into `MWID` — so an operation that keeps
/// the order of all three can change what the names *are* and renumber nothing.
/// That is what makes a map-wide swap one undo entry per tile rather than a
/// rewrite of every record on it.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// `MCLY` and `MCAL` are the chunk's; `MTEX` is the tile's and is in here anyway,
/// because painting with a texture the tile does not name appends to it. A
/// stroke that crosses several chunks therefore carries several copies of a list
/// a few hundred bytes long, and reverting them in the order [`Change::revert`]
/// already uses — last first — leaves the earliest `before` standing, which is
/// the list as it was when the stroke began.
///
/// [`Change::revert`]: crate::undo::Change::revert
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkPaint {
    mcly: Vec<u8>,
    mcal: Vec<u8>,
    layer_count: u32,
    textures: Vec<u8>,
}

impl ChunkPaint {
    pub fn capture(tile: &AdtFile, index: usize) -> ChunkPaint {
        let region = |region: Region| {
            tile.chunk(index)
                .and_then(|chunk| chunk.region(region))
                .map(|sub| sub.data.clone())
                .unwrap_or_default()
        };
        ChunkPaint {
            mcly: region(Region::Layers),
            mcal: region(Region::Alpha),
            layer_count: tile
                .chunk(index)
                .map(|chunk| chunk.head().layer_count())
                .unwrap_or(0),
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
            match chunk.region_mut(region) {
                Some(sub) => sub.set(data.clone()),
                None => {
                    chunk.regions[region as usize] =
                        Some(crate::adt::SubChunk::new(data.clone()))
                }
            }
        }
        chunk.head_mut().set_layer_count(self.layer_count);
    }
}

/// One map chunk's liquid, and the header flags that declare it.
///
/// **Both, because neither is the whole truth.** `MCLQ` holds one block per
/// liquid and says nothing about how many or of what kind; the `MCNK` flags say
/// that and hold no geometry. An undo that put the bytes back and left the flags
/// would describe a chunk with water nothing reads, or read a block that is not
/// there. See `crate::adt::liquid`.
///
/// The whole region rather than one cell, for the reason [`ChunkPaint`] is the
/// whole of a chunk's paint: giving a dry chunk water adds 804 bytes in the
/// middle of a two-megabyte file, and there is no smaller unit that inverts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkLiquid {
    mclq: Vec<u8>,
    /// …and the size field as the file wrote it, which for `MCLQ` is zero and
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
            mclq: region.map(|sub| sub.data.clone()).unwrap_or_default(),
            declared: region.map(|sub| sub.declared).unwrap_or(0),
            flags: chunk.map(|chunk| chunk.head().flags()).unwrap_or(0),
        }
    }

    pub fn restore(&self, tile: &mut AdtFile, index: usize) {
        let Some(chunk) = tile.chunk_mut(index) else {
            return;
        };
        chunk.head_mut().set_flags(self.flags);
        match chunk.region_mut(Region::Liquid) {
            Some(sub) => {
                sub.set_keeping_size(self.mclq.clone());
                sub.declared = self.declared;
            }
            None => {
                let mut sub = crate::adt::SubChunk::new(self.mclq.clone());
                sub.declared = self.declared;
                chunk.regions[Region::Liquid as usize] = Some(sub);
            }
        }
    }
}

/// Everything a placement change touches, captured whole.
#[derive(Debug, Clone, PartialEq)]
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
    /// Put the change in.
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
        }
    }

    /// Take it back out.
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
        }
    }

    /// Which map chunk's **ground** this edit moved, for a caller that patches
    /// vertices. `None` for a placement change, which is about the whole tile,
    /// and **`None` for a paint change**, which moves no vertex: a repaint is
    /// [`Edit::painted`]'s business and takes a different route to the screen.
    pub fn chunk(&self) -> Option<usize> {
        match self {
            // **A colour edit is a vertex edit**, and that is the whole reason
            // it is cheap: `MCCV` is one length or absent, so the mesh keeps
            // its shape, its indices and its length and only an attribute
            // moves. It goes down the same live path a height stroke does.
            Edit::Heights { chunk, .. }
            | Edit::Normals { chunk, .. }
            | Edit::Colours { chunk, .. } => Some(*chunk),
            Edit::Doodad { .. }
            | Edit::Building { .. }
            | Edit::Placements { .. }
            | Edit::Paint { .. }
            // **A hole is not a moved vertex.** It takes cells out of the mesh
            // rather than moving any, so a caller that patched the chunk's
            // vertices from this would write 320 positions into a run that is
            // now a different length. See [`Edit::remeshes`].
            | Edit::Holes { .. }
            | Edit::Flags { .. }
            // …and an area id is not anything the screen holds at all, and a
            // liquid is a surface the tile was *built* with. See
            // [`Edit::remeshes`].
            | Edit::Area { .. }
            | Edit::Liquid { .. }
            // …and a path list is about the whole tile, which is why it
            // remeshes rather than patches. See [`Edit::remeshes`].
            | Edit::Names { .. } => None,
        }
    }

    /// …and which map chunk's **paint** it changed, for a caller that has the
    /// blend maps on the GPU.
    pub fn painted(&self) -> Option<usize> {
        match self {
            Edit::Paint { chunk, .. } => Some(*chunk),
            _ => None,
        }
    }

    /// Whether this edit changes which *textures* a chunk carries, rather than
    /// only how much of each is showing.
    ///
    /// The distinction decides how it reaches the screen, and it is the whole of
    /// why the two are separated. A chunk's blend maps are texels in an atlas
    /// and can be written into the one already drawn; the set of textures a
    /// chunk names is part of the material its draw group was built with, so
    /// changing it means the tile has to be read again.
    pub fn changes_the_texture_set(&self) -> bool {
        match self {
            Edit::Paint { before, after, .. } => before.mcly != after.mcly,
            _ => false,
        }
    }

    /// …and which `MDDF` entry it is about, for a caller that has that
    /// placement drawn and wants to move what is on screen rather than read the
    /// tile again.
    ///
    /// `None` for everything else, **including [`Edit::Placements`]** — adding
    /// or removing one entry renumbers the rest, so there is no single index
    /// that names what changed and a caller has to reconcile the whole list.
    pub fn placement(&self) -> Option<usize> {
        match self {
            Edit::Doodad { index, .. } => Some(*index),
            _ => None,
        }
    }

    /// …and which `MODF` entry, on exactly the same terms.
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

    /// One placement moved, with **`before` read from the tile** rather than
    /// supplied by the caller.
    ///
    /// ## This is the only correct way to build an `Edit`, and the reason is the
    /// invariant the whole stack rests on
    ///
    /// A change is only reversible if its `before` is the state the file was
    /// actually in when the change was made. Every caller that supplies one is
    /// supplying its own *idea* of that state, and an editor is full of places
    /// where that idea goes stale without anything saying so: an undo puts the
    /// file back without telling the panel holding a copy of the record, a tile
    /// is read again and every cached record with it, a second tool writes the
    /// same placement.
    ///
    /// The symptom is not a crash and not a wrong picture. It is a stack that
    /// **stops chaining**: entry *n*'s `before` is no longer entry *n-1*'s
    /// `after`, so undoing them in order walks through states the file was never
    /// in and does not arrive back at the start. It was reported as "the history
    /// gets out of sync after a few moves", which is exactly what it looks like
    /// from outside.
    ///
    /// Reading `before` off the tile makes the invariant hold **by
    /// construction** rather than by every caller being careful, which is the
    /// only way it can hold across four callers that each have their own reason
    /// to think they know the current value.
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

    /// One building moved, on exactly the terms [`Edit::move_doodad`] sets out —
    /// `before` read from the tile, so the stack chains by construction.
    pub fn move_building(tile: &AdtFile, index: usize, after: Building) -> Option<Edit> {
        let before = tile.building_at(index)?;
        (before != after).then_some(Edit::Building {
            index,
            before,
            after,
        })
    }

    /// Whether this edit renumbers the placement lists, so a caller holding an
    /// index into them has to let go of it.
    pub(crate) fn renumbers_placements(&self) -> bool {
        matches!(self, Edit::Placements { .. })
    }

    /// Whether the tile has to be **read again** for this edit to be on screen,
    /// because what it changed is the ground mesh itself rather than anything
    /// the mesh holds.
    ///
    /// The third fork of the same question [`Edit::changes_the_texture_set`] and
    /// [`Edit::renumbers_placements`] ask, and the bluntest: a hole mask decides
    /// how many vertices a chunk contributes at all, so there is no patch that
    /// reaches it — not the vertices (the run changed length), not the indices
    /// (every group after it moved), not the foliage (its cells are gone).
    ///
    /// Answering `false` here and patching anyway is a tile whose draw ranges
    /// name other chunks' vertices, which draws as ground folded through itself.
    pub fn remeshes(&self) -> bool {
        // **Liquid as well as holes**, and for the same reason one step along:
        // a tile's water is a mesh built at load from `MCLQ` — one quad per wet
        // cell, merged into one draw per liquid kind — so a cell going wet or
        // dry changes the geometry rather than anything in it.
        // **…and a rename**, which is the bluntest of the three: what a tile's
        // paths name reaches the screen as the *textures a draw group's
        // material was built with* and the *model each placement resolved to*,
        // and neither is anything the drawn tile still holds a reference to.
        matches!(
            self,
            Edit::Holes { .. } | Edit::Liquid { .. } | Edit::Names { .. }
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

/// **Recompute the shading of every chunk a run of height edits moved**, and
/// record it beside them.
///
/// `edits` is read for which chunks moved and appended to with one
/// [`Edit::Normals`] per chunk whose `MCNR` actually changed.
///
/// Two properties, and each of them is a bug this has already been:
///
/// * **The four neighbours are reshaded as well as the chunk itself.**
///   `heights::recompute_normals` takes a central difference, so a vertex on a
///   chunk's edge is a vertex of the chunk beside it too — reshading only what
///   moved leaves a lit seam along every boundary the edit reached.
/// * **The result is recorded.** A height edit that recomputes normals without
///   putting them on the stack is a change undo cannot take back, so the ground
///   returns to its old shape wearing the shading of the new one. That reads as
///   flat ground lit as though a ramp were still cut into it.
///
/// It is shared because there is more than one way to move the ground — see
/// [`Brush::stroke`](brush::Brush::stroke) and [`grade::Grade::write_into`] —
/// and every one of them owes both properties.
fn reshade(tile: &mut AdtFile, edits: &mut Vec<Edit>) {
    // **`Edit::Heights` and not `Edit::chunk`**, which answers for the paint,
    // the area and the holes too. Only a height moves a normal, and a caller
    // handing over a mixed list should not have the rest of it reshaded.
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
}
