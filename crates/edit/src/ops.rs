//! What an edit is: applied, inverted, and named.
//!
//! Every change to a tile is recorded as one of these before it is made, with
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

use crate::adt::{alpha, area, heights, holes, liquid, AdtFile, Building, Doodad, Region};
use vale_assets::world::adt::{TextureLayer, ALPHA_LEN, ALPHA_SIDE, CHUNK_SIZE};

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
    pub fn renumbers_placements(&self) -> bool {
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

    /// …and which chunk that was, for a caller that only wants to know whether
    /// anything changed.
    pub fn holed(&self) -> Option<usize> {
        match self {
            Edit::Holes { chunk, .. } => Some(*chunk),
            Edit::Flags { chunk, .. } => Some(*chunk),
            _ => None,
        }
    }

    /// …and which chunk's **area** it changed.
    ///
    /// There is no matching "and therefore do X" here, which is the point: an
    /// area id is read by the panel and by a playtest's own terrain and by
    /// nothing that is drawn, so a caller that has one has nothing to catch up.
    /// See `crate::adt::area`.
    pub fn rezoned(&self) -> Option<usize> {
        match self {
            Edit::Area { chunk, .. } => Some(*chunk),
            _ => None,
        }
    }
}

/// **Assigning an area to the ground**, by the chunk, under a circle.
///
/// ## The unit is a chunk and there is nothing smaller
///
/// `MCNK` carries one `areaId` for all 145 of its vertices, so a zone boundary
/// is a 33-yard staircase in the shipped game as much as in anything edited
/// here. A brush is still the right shape — what a person is doing is *making
/// this hillside part of that place* — but its radius chooses **how many
/// chunks**, not where within one.
///
/// ## Which chunks a circle takes, and why it is stated
///
/// The chunk under the pointer **always**, plus every chunk whose centre is
/// inside the circle. Two properties come out of that and both are wanted:
///
/// * a radius under half a chunk is exactly one chunk, which is the common
///   case and the one that must never surprise anybody;
/// * a stroke never does *nothing*, which is what "the circle must overlap the
///   square" gives you when the pointer is near a corner — a brush that
///   sometimes ignores a click is a brush nobody trusts.
///
/// It is deliberately not a falloff. There is nothing to blend: an area id is
/// a row id, and half of one is not a place.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaBrush {
    /// Yards. See the rule above for what it means in chunks.
    pub radius: f32,
    /// The `AreaTable.dbc` row to write. **`0` is a real value** — a chunk
    /// belonging to no area — so there is no "unset" and no `Option` here; see
    /// `crate::adt::area`.
    pub area: u32,
}

impl Default for AreaBrush {
    fn default() -> AreaBrush {
        AreaBrush {
            // One chunk: half of `CHUNK_SIZE` less a little, so the default
            // stroke is the square under the pointer and its immediate
            // neighbours are only reached deliberately.
            radius: 16.0,
            area: 0,
        }
    }
}

impl AreaBrush {
    /// Which of a tile's chunks this stroke covers, at a world position.
    ///
    /// Separate from [`Self::stroke`] so that a tool can **draw what it is about
    /// to do** from the same rule that does it. A preview computed a second way
    /// is a preview that lies, and for a gesture whose result is invisible —
    /// nothing on screen changes when an area id does — the preview is the only
    /// feedback there is.
    pub fn covers(&self, tile: &AdtFile, at: [f32; 2]) -> Vec<usize> {
        let under = heights::chunk_at(tile, at[0], at[1]);
        let mut found: Vec<usize> = Vec::new();
        for (index, chunk) in tile.chunks.iter().enumerate() {
            let origin = chunk.head().position();
            // The chunk's centre: the origin is its *maximum* corner and the
            // cells run in decreasing x and y from it.
            let centre = [origin[0] - CHUNK_SIZE * 0.5, origin[1] - CHUNK_SIZE * 0.5];
            let (dx, dy) = (centre[0] - at[0], centre[1] - at[1]);
            if dx * dx + dy * dy <= self.radius * self.radius || under == Some(index) {
                found.push(index);
            }
        }
        found
    }

    /// …and apply it: every covered chunk that does not already carry the area
    /// takes it.
    ///
    /// **A chunk that already reads the wanted id yields no edit at all**, which
    /// is what keeps a held stroke from putting one entry on the history per
    /// frame for ground it has already painted.
    pub fn stroke(&self, tile: &mut AdtFile, at: [f32; 2]) -> Vec<Edit> {
        let mut edits: Vec<Edit> = Vec::new();
        for index in self.covers(tile, at) {
            let Some(before) = area::area(tile, index) else {
                continue;
            };
            if before == self.area {
                continue;
            }
            let edit = Edit::Area {
                chunk: index,
                before,
                after: self.area,
            };
            edit.apply(tile);
            edits.push(edit);
        }
        edits
    }
}

/// **Putting water on the ground**, by the liquid cell, under a circle.
///
/// ## The unit is a cell and the height is not the pointer's
///
/// `MCLQ`'s wet flags are the chunk's own 8x8 grid — 4.17-yard cells, the same
/// ones the ground is meshed in — so this is four times finer than the hole
/// stamp and the same resolution as the terrain itself.
///
/// **The surface is flat at a level the caller holds, not at the ground under
/// the pointer.** Water is level; a brush that took its height from wherever
/// the pointer happened to be would paint a staircase down a hillside, which is
/// not a thing the format can even represent well and is never what anybody
/// means. Choosing the level is therefore a separate gesture from painting with
/// it — see `crate::tools::water`, where the three ways to choose one are.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterBrush {
    /// Yards.
    pub radius: f32,
    /// The surface height every cell this paints is set to.
    pub level: f32,
    pub kind: vale_assets::world::wmo::Liquid,
    /// **The high nibble written on every cell this wets** —
    /// [`liquid::FISHABLE`], [`liquid::FATIGUE`], or neither. A pool the
    /// brush makes carried zero there before this existed, so nothing painted
    /// here could be fished; the shipped sea carries `FISHABLE` on nearly
    /// every cell. [`WaterAction::Mark`] writes it without moving the water.
    pub cell_flags: u8,
}

impl Default for WaterBrush {
    fn default() -> WaterBrush {
        WaterBrush {
            radius: 12.0,
            level: 0.0,
            kind: vale_assets::world::wmo::Liquid::Water,
            cell_flags: liquid::FISHABLE,
        }
    }
}

/// What one step of a water stroke does to the cells under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaterAction {
    /// Wet them at the level, with the brush's flags.
    Flood,
    /// Dry them, whatever liquid is on them.
    Drain,
    /// Write the brush's flags on the cells that are already wet, and leave
    /// the water where it is — the way to make an existing lake fishable, or
    /// an existing sea deep, without repainting it.
    Mark,
}

/// What one step of a water stroke did.
pub struct Flooded {
    pub edits: Vec<Edit>,
    /// How many cells it wet or dried, for the status line. A stroke over ground
    /// that is already as asked reports zero and records nothing.
    pub cells: usize,
}

impl WaterBrush {
    /// Wet every cell the circle covers, or dry them.
    ///
    /// `ground` answers the terrain height at a world position, and is the
    /// caller's because this crate has the heights and not the tile a position
    /// falls in — it is what the depth byte is computed from, so that a pool has
    /// shallows at its shore instead of one flat opacity. See
    /// [`liquid::Pool::depth_for`].
    pub fn stroke(
        &self,
        tile: &mut AdtFile,
        at: [f32; 2],
        action: WaterAction,
        ground: impl Fn(f32, f32) -> Option<f32>,
    ) -> Flooded {
        let mut edits = Vec::new();
        let mut cells = 0;
        for index in self.chunks_under(tile, at) {
            let Some(chunk) = tile.chunk(index) else { continue };
            let origin = chunk.head().position();
            // **The cell under the pointer is always taken**, and the rest by
            // their centres — the rule `AreaBrush::covers` states and for the
            // same reason: a radius smaller than a cell must still do
            // something, or a click near a corner is a click that is ignored.
            let under = vale_assets::world::adt::cell_at(origin, at[0], at[1]);
            let before = ChunkLiquid::capture(tile, index);
            let mut pools = liquid::pools(tile, index);
            let mut touched = 0;

            for row in 0..vale_assets::world::adt::INNER_SIDE {
                for col in 0..vale_assets::world::adt::INNER_SIDE {
                    let (high, low) =
                        vale_assets::world::adt::cell_square(origin, row, col, 1);
                    let centre = [(high[0] + low[0]) * 0.5, (high[1] + low[1]) * 0.5];
                    let (dx, dy) = (centre[0] - at[0], centre[1] - at[1]);
                    let reached = dx * dx + dy * dy <= self.radius * self.radius;
                    if !reached && under != Some((row, col)) {
                        continue;
                    }
                    // **Drying takes every liquid, wetting takes one.** A cell
                    // under a river and the ocean both is not a thing to make a
                    // person clear twice — and marking takes every liquid for
                    // the same reason.
                    match action {
                        WaterAction::Drain => {
                            for pool in pools.iter_mut() {
                                if pool.is_wet(row, col) {
                                    pool.set_wet(row, col, false);
                                    touched += 1;
                                }
                            }
                            continue;
                        }
                        WaterAction::Mark => {
                            for pool in pools.iter_mut() {
                                if pool.is_wet(row, col) && pool.cell_flags(row, col) != self.cell_flags & 0xF0
                                {
                                    pool.set_cell_flags(row, col, self.cell_flags);
                                    touched += 1;
                                }
                            }
                            continue;
                        }
                        WaterAction::Flood => {}
                    }
                    let slot = match pools.iter().position(|pool| pool.kind == self.kind) {
                        Some(slot) => slot,
                        None => {
                            pools.push(liquid::Pool::flat(self.kind, self.level));
                            pools.len() - 1
                        }
                    };
                    let pool = &mut pools[slot];
                    if pool.is_wet(row, col)
                        && (pool.height(row, col) - self.level).abs() < 1e-3
                        && pool.cell_flags(row, col) == self.cell_flags & 0xF0
                    {
                        continue;
                    }
                    pool.set_wet(row, col, true);
                    pool.set_cell_flags(row, col, self.cell_flags);
                    let above = ground(centre[0], centre[1]).map(|g| self.level - g);
                    let depth = liquid::Pool::depth_for(above.unwrap_or(0.0));
                    pool.fill_cell(row, col, self.level, depth);
                    touched += 1;
                }
            }
            if touched == 0 {
                continue;
            }
            let Some(chunk) = tile.chunk_mut(index) else { continue };
            liquid::set_pools(chunk, &pools);
            let after = ChunkLiquid::capture(tile, index);
            if after == before {
                continue;
            }
            cells += touched;
            edits.push(Edit::Liquid {
                chunk: index,
                before: Box::new(before),
                after: Box::new(after),
            });
        }
        Flooded { edits, cells }
    }

    /// Which of a tile's chunks the circle can reach at all.
    fn chunks_under(&self, tile: &AdtFile, at: [f32; 2]) -> Vec<usize> {
        (0..tile.chunks.len())
            .filter(|&i| {
                let Some(chunk) = tile.chunk(i) else {
                    return false;
                };
                let origin = chunk.head().position();
                let near =
                    |o: f32, p: f32| p >= o - CHUNK_SIZE - self.radius && p <= o + self.radius;
                near(origin[0], at[0]) && near(origin[1], at[1])
            })
            .collect()
    }
}

fn write_area(tile: &mut AdtFile, chunk: usize, id: u32) {
    if let Some(chunk) = tile.chunk_mut(chunk) {
        area::set_area(chunk, id);
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

/// **What a brush's footprint is**, as a distance rather than as a mask.
///
/// The whole of a shape here is *which norm measures `t`*, and that is what
/// makes it free: every [`Falloff`] curve composes with every shape without
/// either knowing about the other, and a square brush still has a soft edge.
/// A mask would have given a hard-edged square with a circular falloff inside
/// it, which is neither shape.
///
/// The three are the three norms anybody wants: round, a square aligned to the
/// world, and the square turned forty-five degrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// The Euclidean norm. A circle.
    Circle,
    /// The Chebyshev norm, `max(|dx|, |dy|)`. A square, axis-aligned — which for
    /// this game's ground is also **chunk-aligned**, so it is the shape that
    /// squares off a plateau along the grid the file is stored in.
    Square,
    /// The Manhattan norm, `|dx| + |dy|`. A diamond.
    Diamond,
}

impl Shape {
    /// How far an offset is from the centre, in this shape's own measure.
    pub fn distance(self, dx: f32, dy: f32) -> f32 {
        match self {
            Shape::Circle => (dx * dx + dy * dy).sqrt(),
            Shape::Square => dx.abs().max(dy.abs()),
            Shape::Diamond => dx.abs() + dy.abs(),
        }
    }

    /// …and the outline of the shape at one radius, as a closed run of points
    /// around the centre.
    ///
    /// **Here rather than in the tool that draws it**, because a ring drawn from
    /// a second reading of the shape is a ring that lies about where the brush
    /// is — and the two would drift the first time a norm was corrected. The
    /// caller drops each point onto the ground.
    pub fn outline(self, radius: f32, segments: usize) -> Vec<(f32, f32)> {
        (0..segments)
            .map(|i| {
                let t = i as f32 / segments as f32;
                let angle = t * std::f32::consts::TAU;
                let (sin, cos) = angle.sin_cos();
                // The point on the unit shape along this direction: divide by
                // the direction's own distance, which is exactly the definition
                // of the shape's rim.
                let scale = radius / self.distance(cos, sin).max(1e-6);
                (cos * scale, sin * scale)
            })
            .collect()
    }
}

/// How a brush's strength falls off toward its edge.
///
/// Five curves over one parameter: `t`, how far out a point is as a fraction of
/// the radius, measured in whatever [`Shape`] the brush is. Every one of them is
/// zero at and beyond the rim, which is what lets a brush walk over ground it
/// does not reach without special-casing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Falloff {
    /// Full strength everywhere inside the radius. Leaves a visible rim.
    Flat,
    /// Straight from full at the centre to nothing at the rim.
    Linear,
    /// `1 — t^2` squared: flat in the middle and tangent to zero at the rim,
    /// which is what makes repeated strokes blend instead of terracing.
    Smooth,
    /// `(1 — t)^2` — **concave**, so the strength collapses as soon as it leaves
    /// the centre. A spike rather than a dome, and the one to reach for when
    /// what is wanted is a peak or a well rather than a hill.
    Sharp,
    /// `sqrt(1 — t^2)` — **convex**: a sphere's own profile, full for most of
    /// the radius and falling off a cliff at the rim. What a dome of ground
    /// actually looks like, where [`Falloff::Smooth`] gives a softer mound.
    Dome,
}

impl Falloff {
    /// **The weight at `t` with an inner core held at full strength.**
    ///
    /// `core` is the fraction of the radius that does not fall off at all, and
    /// the curve runs over what is left. It is what the brush preview's inner
    /// ring is drawn at, and what a person means by *where the pressure is*: a
    /// curve alone says how the strength decays but not how much of the brush is
    /// under it, and those are two different wants. A core of zero is
    /// [`Falloff::weight`] exactly, which is what every existing brush had.
    ///
    /// Clamped below 1 because a core of the whole radius is [`Falloff::Flat`]
    /// and would divide by nothing to get there.
    pub fn over(self, t: f32, core: f32) -> f32 {
        let core = core.clamp(0.0, 0.95);
        match t < core {
            true => 1.0,
            false => self.weight((t - core) / (1.0 - core)),
        }
    }

    /// The weight at `t`, the distance from the centre as a fraction of the
    /// radius. Zero at and beyond the rim in every mode.
    pub fn weight(self, t: f32) -> f32 {
        if !(0.0..1.0).contains(&t) {
            return 0.0;
        }
        match self {
            Falloff::Flat => 1.0,
            Falloff::Linear => 1.0 - t,
            Falloff::Smooth => {
                let f = 1.0 - t * t;
                f * f
            }
            Falloff::Sharp => {
                let f = 1.0 - t;
                f * f
            }
            Falloff::Dome => (1.0 - t * t).max(0.0).sqrt(),
        }
    }
}

/// **A signed value in `-1..1` that depends only on where a point is, and
/// varies over `scale` yards**, for [`Mode::Noise`].
///
/// Value noise: the plane is divided into a lattice of `scale`-yard cells, each
/// corner is hashed to its own number, and a point takes the smooth blend of the
/// four corners around it.
///
/// **The scale is the whole of what makes it usable.** The first version hashed
/// the position directly at a tenth of a yard, which is finer than the 4.17
/// yards between vertices — so every vertex got an *unrelated* number and the
/// result was white noise at the vertex spacing. That is the roughest thing the
/// format can hold, it is the same roughness at every strength, and it was
/// reported from the window as *"too jagged"*. With a lattice coarser than the
/// vertex spacing, neighbouring vertices sit inside one cell and get *similar*
/// numbers, which is what a rolling surface is. The scale is how many yards a
/// bump is across, and it is the brush's to choose.
///
/// The blend uses the fifth-order ease rather than a straight line, because a
/// linear blend is flat-sided inside each cell and creases at every lattice
/// line: the creases are a grid, and a grid is more obviously wrong than the
/// noise it replaced.
///
/// **Deterministic is still the point.** A brush that drew a fresh random number
/// each frame would shake the ground while it was held and settle nowhere; with
/// the value fixed by where the vertex is, holding it deepens the *same* bumps,
/// so roughness grows the way every other mode's effect grows and an undo takes
/// back one coherent shape. It is the same argument `ChunkFoliage`'s scatter
/// makes for hashing a chunk's world position rather than keeping a seed.
pub fn wobble(x: f32, y: f32, scale: f32) -> f32 {
    // A cell smaller than this is finer than the vertices can express, and zero
    // would divide by nothing.
    let scale = scale.max(0.5);
    let (u, v) = (x / scale, y / scale);
    let (cx, cy) = (u.floor(), v.floor());
    let (fx, fy) = (u - cx, v - cy);
    // Smootherstep: zero slope *and* zero curvature at both ends, so neither the
    // surface nor its shading shows the lattice.
    let ease = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (sx, sy) = (ease(fx), ease(fy));
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let corner = |i: f32, j: f32| lattice(cx + i, cy + j);
    mix(
        mix(corner(0.0, 0.0), corner(1.0, 0.0), sx),
        mix(corner(0.0, 1.0), corner(1.0, 1.0), sx),
        sy,
    )
}

/// One lattice corner's own number, in `-1..1`. The hash [`wobble`] is built on.
fn lattice(i: f32, j: f32) -> f32 {
    let mut hash = i as i64 as u64;
    hash = hash.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    hash ^= (j as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    hash ^= hash >> 32;
    // The top 24 bits, which are the ones the mixing has moved most.
    ((hash >> 40) as f32 / (1u32 << 23) as f32) - 1.0
}

/// What a stroke does to the height under it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Add `strength` yards a second.
    Raise,
    /// Subtract them.
    Lower,
    /// Move toward one height, at `strength` of the remaining distance a second.
    Flatten { to: f32 },
    /// Move toward the mean height of the vertices the brush covers, which
    /// pulls a ridge down and a ditch up at once.
    Smooth,
    /// **Roughen**: move each vertex by its own signed share of `strength`, at
    /// `strength` yards a second. See [`wobble`] for why the sign is a function
    /// of *where the vertex is* rather than of the frame — holding the brush has
    /// to deepen the same bumps rather than shake the ground.
    ///
    /// The one mode that is not a statement about a target. Raise, lower,
    /// flatten and smooth all converge on something; this is the only one whose
    /// whole purpose is that neighbouring vertices disagree.
    Noise,
}

/// A terrain brush.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brush {
    /// Yards.
    pub radius: f32,
    /// Yards a second for [`Mode::Raise`], [`Mode::Lower`] and [`Mode::Noise`];
    /// a fraction of the remaining distance a second for the other two.
    pub strength: f32,
    pub falloff: Falloff,
    /// How much of the radius is held at **full strength** before the falloff
    /// starts, as a fraction — see [`Falloff::over`]. Zero is the plain curve.
    pub core: f32,
    /// The footprint — see [`Shape`], which is a distance and not a mask.
    pub shape: Shape,
    /// How many yards a bump is across, for [`Mode::Noise`] — see [`wobble`].
    /// Ignored by every other mode.
    pub scale: f32,
    pub mode: Mode,
    /// Which way [`Mode::Flatten`] may move a vertex. See [`Only`].
    pub only: Only,
    /// The slope of the plane [`Mode::Flatten`] flattens toward, as the rise
    /// in yards per yard along world x and along world y. Zero is level, and
    /// then the target is the mode's own height everywhere. See
    /// [`Brush::tilted`].
    pub tilt: [f32; 2],
    /// The point the tilted plane passes through at the mode's own height.
    /// A caller sets it where the stroke begins. Unused while [`Self::tilt`]
    /// is zero.
    pub pivot: [f32; 2],
}

/// Which way a flatten may move the ground.
///
/// A flatten toward a height does two things at once: it fills what is under
/// the height and cuts what is over it. Each alone is a tool of its own. Fill
/// makes a causeway across a hollow and leaves the hills beside it; cut
/// makes a shelf in a hillside and leaves the valley.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Only {
    /// Up and down, toward the target.
    #[default]
    Both,
    /// Raise what is under the target and leave what is over it.
    Fill,
    /// Lower what is over the target and leave what is under it.
    Cut,
}

impl Default for Brush {
    fn default() -> Brush {
        Brush {
            radius: 20.0,
            strength: 8.0,
            falloff: Falloff::Smooth,
            core: 0.0,
            shape: Shape::Circle,
            // Six times the 4.17 yards between vertices, so the default reads as
            // rolling ground rather than as the vertex grid.
            scale: 25.0,
            mode: Mode::Raise,
            only: Only::Both,
            tilt: [0.0; 2],
            pivot: [0.0; 2],
        }
    }
}

impl Brush {
    /// Apply one step of the stroke at a world position, and return the edits it
    /// made.
    ///
    /// `seconds` is how long the stroke has been held since the last step, so a
    /// brush moves the ground at the same rate whatever the frame rate is.
    ///
    /// The normals of every chunk the heights changed are recomputed, and so are
    /// those of the chunks around them: a vertex on a chunk edge takes its
    /// gradient from the chunk next door, so raising ground next to a boundary
    /// changes the shading on both sides of it.
    pub fn stroke(
        &self,
        tile: &mut AdtFile,
        at: [f32; 2],
        seconds: f32,
        level: Option<f32>,
    ) -> Vec<Edit> {
        self.stroke_keeping(tile, at, seconds, level, None)
    }

    /// The slope [`Self::tilt`] holds for a plane that rises `angle` degrees
    /// toward the compass bearing `toward`: 0 is north, which is world +x, and
    /// 90 is east, which is world -y.
    pub fn tilted(angle: f32, toward: f32) -> [f32; 2] {
        let rise = angle.clamp(0.0, 89.0).to_radians().tan();
        let bearing = toward.to_radians();
        [rise * bearing.cos(), -rise * bearing.sin()]
    }

    /// [`Self::stroke`], leaving every vertex in `kept` where it is.
    ///
    /// `kept` is a set of this tile's vertices that the stroke may not move:
    /// a selection used as a lock. The shading is still recomputed for every
    /// chunk the stroke reached, since a kept vertex's normal depends on its
    /// neighbours.
    pub fn stroke_keeping(
        &self,
        tile: &mut AdtFile,
        at: [f32; 2],
        seconds: f32,
        level: Option<f32>,
        kept: Option<&vertices::Selected>,
    ) -> Vec<Edit> {
        let touched = self.chunks_under(tile, at);
        let mut edits = Vec::new();
        // **What [`Mode::Smooth`] converges on, decided once for the whole
        // stroke.** See [`Brush::weighed`]: a mean taken per chunk is a different
        // height in every chunk, which is a step at every boundary. The caller
        // supplies it when the brush can reach more than one tile, which is the
        // only way the two sides of a border can agree.
        let level = match (self.mode, level) {
            (Mode::Smooth, None) => {
                let (sum, weight) = self.weighed(tile, at);
                (weight > 0.0).then(|| (sum / weight) as f32)
            }
            _ => level,
        };

        for &index in &touched {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            let before = heights::heights(chunk);
            if before.is_empty() {
                continue;
            }
            let mut after = self.moved(&before, origin, at, seconds, level);
            if let Some(kept) = kept {
                for (vertex, height) in after.iter_mut().enumerate() {
                    if kept.holds(index, vertex) {
                        *height = before[vertex];
                    }
                }
            }
            if after == before {
                continue;
            }
            edits.push(Edit::Heights {
                chunk: index,
                before,
                after: after.clone(),
            });
            heights::set_heights(tile.chunk_mut(index).unwrap(), &after);
        }
        if edits.is_empty() {
            return edits;
        }

        // The shading, on the chunks whose heights moved and on their four
        // neighbours.
        reshade(tile, &mut edits);
        edits
    }

    /// The heights this brush would leave, without touching the tile.
    ///
    /// `level` is what [`Mode::Smooth`] and [`Mode::Flatten`] converge on, and it
    /// comes from outside because it is a property of the whole stroke rather
    /// than of this chunk.
    fn moved(
        &self,
        before: &[f32],
        origin: [f32; 3],
        at: [f32; 2],
        seconds: f32,
        level: Option<f32>,
    ) -> Vec<f32> {
        // Where a vertex is, which both the falloff and the noise need.
        let world = |i: usize| {
            let (dx, dy) = heights::vertex_offset(i);
            (origin[0] - dx, origin[1] - dy)
        };
        // …and how far out it is, **in the brush's own shape** — see [`Shape`],
        // where the whole of a shape being a distance is argued.
        let distance = |i: usize| {
            let (x, y) = world(i);
            self.shape.distance(x - at[0], y - at[1])
        };
        // **The height the two converging modes converge on**, and it is handed in
        // rather than worked out here. This used to take the mean of `before` —
        // *this chunk's own 145 heights* — so each chunk under one brush smoothed
        // toward a different height and left a step at every 33-yard boundary,
        // worst where that boundary was also a tile seam and the two sides were
        // in different files. Reported from the window as *"smooth is broken
        // across tiles"*; it was broken across chunks too.
        let target = match self.mode {
            Mode::Smooth => level,
            Mode::Flatten { to } => Some(to),
            _ => None,
        };
        // A flatten's target at one vertex: the plane through the pivot at
        // the mode's height, which is that height everywhere while the tilt
        // is zero. A smooth has no plane.
        let flattening = matches!(self.mode, Mode::Flatten { .. });
        let target_at = |i: usize, to: f32| match flattening {
            true => {
                let (x, y) = world(i);
                to + self.tilt[0] * (x - self.pivot[0]) + self.tilt[1] * (y - self.pivot[1])
            }
            false => to,
        };

        before
            .iter()
            .enumerate()
            .map(|(i, &h)| {
                let w = self.falloff.over(distance(i) / self.radius, self.core);
                if w == 0.0 {
                    return h;
                }
                match self.mode {
                    Mode::Raise => h + self.strength * w * seconds,
                    Mode::Lower => h - self.strength * w * seconds,
                    // **Its own direction per vertex**, fixed by where the
                    // vertex is — see [`wobble`].
                    Mode::Noise => {
                        let (x, y) = world(i);
                        h + self.strength * w * seconds * wobble(x, y, self.scale)
                    }
                    Mode::Flatten { .. } | Mode::Smooth => match target {
                        Some(to) => {
                            let to = target_at(i, to);
                            // A fill leaves what is over the target and a cut
                            // what is under it. A smooth moves both ways.
                            let allowed = match (flattening, self.only) {
                                (true, Only::Fill) => to > h,
                                (true, Only::Cut) => to < h,
                                _ => true,
                            };
                            match allowed {
                                // Clamped so a long frame cannot overshoot the
                                // target and oscillate around it.
                                true => h + (to - h) * (self.strength * w * seconds).min(1.0),
                                false => h,
                            }
                        }
                        None => h,
                    },
                }
            })
            .collect()
    }

    /// **The two halves of the mean height under this brush, in one tile**: the
    /// weighted sum, and the total weight.
    ///
    /// Kept apart rather than divided because the caller adds up several tiles
    /// before dividing, which is the whole point. A brush 58 yards across spans
    /// four chunks and can span two files, and [`Mode::Smooth`] has to pull all
    /// of them toward *one* height or it leaves a step exactly where the boundary
    /// is, with nothing about either file wrong.
    ///
    /// Weighted by the same curve the move is, so a vertex that barely moves
    /// barely counts toward where everything is moving to.
    ///
    /// `f64` because this sums a few thousand terms and the heights are hundreds
    /// of yards; in `f32` the tail of the sum stops landing.
    pub fn weighed(&self, tile: &AdtFile, at: [f32; 2]) -> (f64, f64) {
        let mut sum = 0.0;
        let mut weight = 0.0;
        for index in self.chunks_under(tile, at) {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            for (i, h) in heights::heights(chunk).iter().enumerate() {
                let (dx, dy) = heights::vertex_offset(i);
                let (x, y) = (origin[0] - dx, origin[1] - dy);
                let t = self.shape.distance(x - at[0], y - at[1]) / self.radius;
                let w = self.falloff.over(t, self.core) as f64;
                sum += *h as f64 * w;
                weight += w;
            }
        }
        (sum, weight)
    }

    /// The chunks whose square the brush's circle reaches.
    fn chunks_under(&self, tile: &AdtFile, at: [f32; 2]) -> Vec<usize> {
        (0..tile.chunks.len())
            .filter(|&i| {
                let Some(chunk) = tile.chunk(i) else {
                    return false;
                };
                let origin = chunk.head().position();
                let near = |o: f32, p: f32| p >= o - CHUNK_SIZE - self.radius && p <= o + self.radius;
                near(origin[0], at[0]) && near(origin[1], at[1])
            })
            .collect()
    }
}

/// What one step of a paint stroke did, and what it could not do.
///
/// **A stroke reports its refusals rather than returning fewer edits**, and that
/// is not tidiness. A chunk already carrying [`alpha::MAX_LAYERS`] textures
/// cannot take another, and about half the chunks in the shipped tiles are
/// already at four — measured: 144 of 256 on `Azeroth_32_48`, 194 of 256 on
/// `Azeroth_34_51`, 120 of 256 on `Kalimdor_30_41`. A brush that simply painted
/// nothing there stops dead part-way across the ground with no error anywhere,
/// which is what it was reported as: *"you can be painting a texture and
/// suddenly run into a chunk where it just stops"*.
#[derive(Debug, Default)]
pub struct Painted {
    pub edits: Vec<Edit>,
    /// **Which** chunks under the brush were full and do not already carry this
    /// texture — not how many. The caller wants to put one of them in front of
    /// the person painting, and a count cannot say which.
    /// See [`alpha::Paint::remove_layer`], which is the way out.
    pub full: Vec<usize>,
    /// …and which had **no texture at all** and have just been given this one as
    /// their base.
    ///
    /// Worth reporting for the same reason as a refusal, and the opposite one: a
    /// base cannot be partial, so the whole chunk becomes that texture however
    /// little of it the brush covered. That is the only thing an empty chunk can
    /// be given and it is not what a brush stroke usually means, so the caller
    /// says so rather than letting it look like the brush overshot.
    pub based: Vec<usize>,
    /// The chunks an erasing stroke reached whose base is the texture. The
    /// base has no blend map, so there is nothing to erase.
    pub base: Vec<usize>,
    /// The chunks a stroke confined to existing layers reached that do not
    /// carry the texture. See [`PaintBrush::existing_only`].
    pub absent: Vec<usize>,
    /// The full chunks in which a layer that showed almost nothing was given
    /// to the texture. See [`PaintBrush::reuse_hidden`].
    pub reused: Vec<usize>,
}

impl Painted {
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
            && self.full.is_empty()
            && self.based.is_empty()
            && self.base.is_empty()
            && self.absent.is_empty()
            && self.reused.is_empty()
    }
}

/// What a shading stroke does to the vertices under it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shade {
    /// Toward the brush's own colour.
    Paint,
    /// …toward white, which in `MCCV`'s scale is *twice* the light rather than
    /// all of it — see [`vale_assets::world::adt::decode_colours`].
    Lighten,
    /// …and toward black.
    Darken,
    /// Toward the mean of what is under the brush, which is what takes a hard
    /// edge out without deciding what the colour should be.
    Smooth,
    /// Back to neutral, which for a chunk that ends up wholly neutral means
    /// **taking the region away** rather than filling it with 0x7F — see
    /// [`crate::adt::colours::clear`].
    Clear,
}

impl Shade {
    /// What to call it in an interface.
    pub fn name(self) -> &'static str {
        match self {
            Shade::Paint => "Paint",
            Shade::Lighten => "Lighten",
            Shade::Darken => "Darken",
            Shade::Smooth => "Smooth",
            Shade::Clear => "Clear",
        }
    }
}

/// A brush that paints `MCCV`.
///
/// ## It shares the height brush's footprint and nothing else
///
/// [`Shape`], [`Falloff`] and the core are the same three controls over the same
/// two numbers, and they are *called* rather than restated: a shading brush that
/// had its own idea of what a square footprint is would be a second answer to a
/// question [`Shape::distance`] already answers. What differs is everything past
/// the weight — the value moved is four bytes rather than one float, and it is
/// clamped to a range with a meaningful midpoint.
///
/// ## A stroke cannot work in the file's own numbers
///
/// The same fault the texture brush has and for the same reason, one region
/// along: a byte is 256 levels, so a step of a third of a level is a step that
/// rounds away and never happens. At the strengths a person actually paints
/// shading at — a slow darkening under a tree — every frame's step is a fraction
/// of a byte. So a stroke accumulates in [`Working`]'s `f32` copy and the file is
/// written from it, quantised once on the way out. See [`Working`], where the
/// argument is made at length about alpha.
///
/// ## Alpha is painted and is not read by anything here
///
/// `CImVector`'s fourth byte has no meaning this renderer uses — the shading is
/// the `rgb` product and nothing samples the alpha — so it is written neutral
/// and left there. It is kept in the record rather than dropped because the
/// region has a fixed length either way and a later version of this does have a
/// use for it (`MCLV`'s lighting pass), and a file written with a garbage byte
/// in it is a file that has to be rewritten then.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shading {
    pub radius: f32,
    /// How much of the distance to the target is closed per second at full
    /// weight, 0..1-ish. The same shape [`Mode::Flatten`]'s strength has.
    pub strength: f32,
    pub falloff: Falloff,
    pub core: f32,
    pub shape: Shape,
    pub mode: Shade,
    /// What [`Shade::Paint`] paints toward, as the multiplier a person chose:
    /// 1.0 is neutral, 0 is black, 2 is the brightest the byte can say.
    pub colour: [f32; 3],
}

impl Default for Shading {
    fn default() -> Shading {
        Shading {
            radius: 20.0,
            // A quarter of the way to the target per second at full weight:
            // slow enough that a person can stop where they meant to, which is
            // the whole difficulty of painting light.
            strength: 0.25,
            falloff: Falloff::Smooth,
            core: 0.0,
            shape: Shape::Circle,
            mode: Shade::Paint,
            // Neutral, so a brush that has not been given a colour paints
            // nothing rather than painting mid-grey over the world.
            colour: [1.0, 1.0, 1.0],
        }
    }
}

/// The largest multiplier `MCCV` can state: 255 / 127.
pub const SHADE_MAX: f32 = 255.0 / crate::adt::colours::NEUTRAL as f32;

impl Shading {
    /// Apply one step of the stroke, and return the edits it made.
    ///
    /// `working` is the stroke's own `f32` copy — see [`Working`]. `level` is
    /// what [`Shade::Smooth`] converges on, handed in for the reason the height
    /// brush's is: a mean taken per chunk is a different colour in every chunk
    /// and leaves a step at every 33-yard boundary.
    pub fn stroke(
        &self,
        tile: &mut AdtFile,
        at: [f32; 2],
        seconds: f32,
        working: &mut Working,
        level: Option<[f32; 3]>,
    ) -> Vec<Edit> {
        let mut edits = Vec::new();
        for index in self.chunks_under(tile, at) {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            let before = crate::adt::colours::capture(tile, index);
            let live = working.shading(index, || read_shading(tile, index));
            let mut moved = false;
            for i in 0..vale_assets::world::adt::HEIGHTS_PER_CHUNK {
                let (dx, dy) = heights::vertex_offset(i);
                let (x, y) = (origin[0] - dx, origin[1] - dy);
                let w = self
                    .falloff
                    .over(self.shape.distance(x - at[0], y - at[1]) / self.radius, self.core);
                if w == 0.0 {
                    continue;
                }
                let step = (self.strength * w * seconds).min(1.0);
                let target = match self.mode {
                    Shade::Paint => self.colour,
                    Shade::Lighten => [SHADE_MAX; 3],
                    Shade::Darken => [0.0; 3],
                    Shade::Clear => [1.0; 3],
                    Shade::Smooth => match level {
                        Some(level) => level,
                        None => continue,
                    },
                };
                for k in 0..3 {
                    let was = live[i][k];
                    let now = (was + (target[k] - was) * step).clamp(0.0, SHADE_MAX);
                    if now != was {
                        live[i][k] = now;
                        moved = true;
                    }
                }
            }
            if !moved {
                continue;
            }
            let quantised = quantise(live);
            let Some(chunk) = tile.chunk_mut(index) else {
                continue;
            };
            crate::adt::colours::set_all(&mut *chunk, &quantised);
            // **A chunk painted back to neutral loses the region.** Anything
            // else leaves 580 bytes behind on a tile that was shipped without
            // them, which is the difference between a file somebody edited and
            // a file somebody edited *and then undid*.
            if crate::adt::colours::is_neutral(chunk) {
                crate::adt::colours::clear(chunk);
            }
            let after = crate::adt::colours::capture(tile, index);
            if after != before {
                edits.push(Edit::Colours {
                    chunk: index,
                    before,
                    after,
                });
            }
        }
        edits
    }

    /// **The two halves of the mean shade under this brush, in one tile**: the
    /// weighted sum and the total weight.
    ///
    /// Kept apart for the reason `Brush::weighed` keeps its apart — the caller
    /// adds up several tiles before dividing, because a brush can span two
    /// files and [`Shade::Smooth`] has to pull both toward one colour.
    pub fn weighed(&self, tile: &AdtFile, at: [f32; 2]) -> ([f64; 3], f64) {
        let mut sum = [0.0f64; 3];
        let mut weight = 0.0f64;
        for index in self.chunks_under(tile, at) {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            let live = read_shading(tile, index);
            for i in 0..vale_assets::world::adt::HEIGHTS_PER_CHUNK {
                let (dx, dy) = heights::vertex_offset(i);
                let (x, y) = (origin[0] - dx, origin[1] - dy);
                let w = self
                    .falloff
                    .over(self.shape.distance(x - at[0], y - at[1]) / self.radius, self.core);
                if w == 0.0 {
                    continue;
                }
                for k in 0..3 {
                    sum[k] += f64::from(live[i][k]) * f64::from(w);
                }
                weight += f64::from(w);
            }
        }
        (sum, weight)
    }

    /// The chunks whose square the brush's bounding box reaches.
    fn chunks_under(&self, tile: &AdtFile, at: [f32; 2]) -> Vec<usize> {
        (0..tile.chunks.len())
            .filter(|&i| {
                let Some(chunk) = tile.chunk(i) else {
                    return false;
                };
                let origin = chunk.head().position();
                let near =
                    |o: f32, p: f32| p >= o - CHUNK_SIZE - self.radius && p <= o + self.radius;
                near(origin[0], at[0]) && near(origin[1], at[1])
            })
            .collect()
    }
}

/// One chunk's shading as multipliers, neutral where it has no region.
fn read_shading(tile: &AdtFile, index: usize) -> Vec<[f32; 3]> {
    let neutral = vec![[1.0f32; 3]; vale_assets::world::adt::HEIGHTS_PER_CHUNK];
    let Some(chunk) = tile.chunk(index) else {
        return neutral;
    };
    match chunk.region(Region::Colours) {
        None => neutral,
        Some(mccv) => vale_assets::world::adt::decode_colours(&mccv.data)
            .into_iter()
            .map(|c| [c[0], c[1], c[2]])
            .collect(),
    }
}

/// …and back into the file's bytes, once, on the way out.
///
/// The alpha is written neutral — see [`Shading`], where the reason it is
/// written at all is.
fn quantise(live: &[[f32; 3]]) -> Vec<crate::adt::colours::Colour> {
    live.iter()
        .map(|c| {
            let byte = |v: f32| {
                (v * crate::adt::colours::NEUTRAL as f32)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            [byte(c[0]), byte(c[1]), byte(c[2]), crate::adt::colours::NEUTRAL]
        })
        .collect()
}

/// A stroke's own copy of the chunks it is over, at full precision.
///
/// ## A stroke cannot work in the numbers the file holds
///
/// A 4-bit chunk stores sixteen levels, so its alphas are multiples of 17. Read
/// the value, move it a little, write it back, read it again — and any step
/// smaller than half a quantum is a step that did not happen. That is the fault
/// the snap-away rule in the round before this one was written to fix, and
/// snapping is the wrong fix: it makes **every** step a whole quantum, so the
/// smallest possible stroke advances a fifteenth of the range per frame and the
/// strength setting controls nothing at all. Measured from the window: a quarter
/// of a second to full opacity at any strength.
///
/// So the stroke keeps its own copy of the texels it is working on, in `f32`,
/// seeded from the file the first time it touches a chunk. Every step
/// accumulates there and the file is written from it — quantised once, on the
/// way out, instead of once per frame. A step of a thousandth of a unit is a
/// step that is still there after a thousand frames.
///
/// Held by the caller for the length of one stroke and cleared when the button
/// comes up: it is a *stroke's* state, and a copy kept past the stroke that made
/// it is a copy that disagrees with the file the moment anything else writes.
#[derive(Debug, Default)]
pub struct Working {
    /// Per chunk, per layer, per texel: alpha in 0..255 with the fraction kept.
    chunks: std::collections::HashMap<usize, Vec<Vec<f32>>>,
    /// Per chunk, per vertex: the `MCCV` multiplier with the fraction kept.
    ///
    /// A second map rather than a second `Working`, because the two are the same
    /// idea over the same lifetime and a tool holds one of these per stroke. The
    /// shapes differ — 145 vertices against a layer's 4,096 texels — so they
    /// cannot share a map, and nothing wants them to: no stroke paints both.
    shading: std::collections::HashMap<usize, Vec<[f32; 3]>>,
}

impl Working {
    /// Forget everything. Called when the button comes up.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.shading.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty() && self.shading.is_empty()
    }

    /// This stroke's copy of one chunk's shading, seeded from `read` the first
    /// time it is asked for.
    pub fn shading(
        &mut self,
        chunk: usize,
        read: impl FnOnce() -> Vec<[f32; 3]>,
    ) -> &mut Vec<[f32; 3]> {
        self.shading.entry(chunk).or_insert_with(read)
    }
}

/// A texture brush.
///
/// ## Painting a texture is making it the topmost thing at a texel
///
/// A chunk's layers are drawn in order over an opaque base: layer 1 over the
/// base at its own alpha, layer 2 over that, layer 3 over that. So the fraction
/// of a texel that ends up showing layer *k* is `a_k` times what the layers
/// **above** it leave — and making texture *k* visible means two things and not
/// one: raising `a_k`, and lowering every `a_j` for `j > k`. A brush that raised
/// only its own layer paints nothing at all under a layer that is already opaque
/// there, which reads as "the brush does not work on this half of the chunk".
///
/// Layers below *k* are left alone. They are hidden by *k* wherever it is
/// opaque, so moving them changes nothing visible and would destroy the paint a
/// person would see again if they took *k* away.
///
/// ## …and it is what puts a fourth texture on a chunk
///
/// A chunk that does not carry the texture yet is given it as a **new topmost
/// layer**, transparent everywhere, and then painted. That is the one thing this
/// brush does that changes the length of a region, and the one that a caller
/// with the chunk drawn has to notice — see [`Edit::changes_the_texture_set`].
/// A chunk already carrying [`alpha::MAX_LAYERS`] cannot take another and is
/// skipped rather than having one of its layers thrown away.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintBrush {
    /// Yards.
    pub radius: f32,
    /// How much of the radius is held at **full strength** before the falloff
    /// starts, as a fraction — see [`Falloff::over`]. The height brush's own, for
    /// the same reason: how hard the middle of a stroke bites is a separate want
    /// from which curve it fades along.
    pub core: f32,
    /// The footprint — see [`Shape`]. The same three the height brush has, for
    /// the same reason: a road is painted with a square and a clearing with a
    /// circle, and neither wants the other.
    pub shape: Shape,
    /// A fraction of the remaining distance a second, so a held brush converges
    /// on the texture rather than stepping to it.
    pub strength: f32,
    pub falloff: Falloff,
    /// **The archive path of the tileset, not an `MTEX` index.**
    ///
    /// `MTEX` is per tile, so a stroke that crosses a border has two different
    /// numbers for one texture and a brush holding a number could only paint one
    /// side of it. Naming it here also puts [`AdtFile::name_texture`] *inside*
    /// the stroke, after the chunk's paint has been captured, which is what makes
    /// an undo take the appended name back out again.
    pub texture: String,
    /// **What a layer of this texture grows**: the `MCLY` `effectId` written
    /// on a layer this brush adds, a `GroundEffectTexture` row — see
    /// `vale_assets::tables::foliage`. Zero grows nothing, and zero is what
    /// every layer this brush wrote carried before the field existed, so
    /// painting grass over dirt painted grass and planted none. The caller
    /// sets it when the texture is chosen, from what the shipped ground pairs
    /// that texture with.
    pub effect_id: u32,
    /// How visible the texture is where a held stroke ends up, 0 to 1. At 1
    /// the stroke converges on the texture alone. At 0.6 it converges on the
    /// texture at 60% over what is under it, from either side: ground already
    /// more opaque than that is brought down to it.
    pub opacity: f32,
    /// Take the texture away instead of putting it down: its layer moves
    /// toward transparent and nothing else moves, so what is under it shows
    /// again. Erasing never adds a layer. The base cannot be erased, since it
    /// has no blend map; see [`Painted::base`].
    pub erase: bool,
    /// Paint only chunks that already carry the texture. A chunk without it
    /// is left alone and reported in [`Painted::absent`], so a stroke changes
    /// blends and never the set of textures a chunk names.
    pub existing_only: bool,
    /// When a chunk is full, give the texture the layer that shows least,
    /// provided it shows less than [`REUSE_UNDER`] of the chunk. See
    /// [`Painted::reused`].
    pub reuse_hidden: bool,
    /// How much of the ground under the brush is painted, 0 to 1. At 1 the
    /// stroke is solid. Below 1 it paints in patches, the fraction of the
    /// footprint that [`speckle`] puts under this value, which is how one
    /// texture is broken up into another.
    pub density: f32,
    /// How many yards a patch is across, for a [`Self::density`] under 1.
    pub grain: f32,
}

/// The most of a chunk a layer may show and still be given to another
/// texture by [`PaintBrush::reuse_hidden`]: two hundredths. Below it the layer
/// is a few texels at a border, and replacing it changes nothing a person
/// would find.
pub const REUSE_UNDER: f32 = 0.02;

/// A value from 0 to 1 that depends on where a texel is and on nothing else,
/// varying over `grain` yards: [`wobble`] moved into that range.
///
/// A stroke at a density under 1 paints the texels where this is under the
/// density. It is a function of the place for the reason `wobble` is: a held
/// stroke has to deepen the same patches and not paint new ones every frame.
pub fn speckle(x: f32, y: f32, grain: f32) -> f32 {
    wobble(x, y, grain.max(0.25)) * 0.5 + 0.5
}

impl Default for PaintBrush {
    fn default() -> PaintBrush {
        PaintBrush {
            radius: 12.0,
            core: 0.0,
            shape: Shape::Circle,
            strength: 3.0,
            falloff: Falloff::Smooth,
            texture: String::new(),
            effect_id: 0,
            opacity: 1.0,
            erase: false,
            existing_only: false,
            reuse_hidden: false,
            density: 1.0,
            // About four texels of a blend map, which is the smallest patch
            // that still reads as a patch after the map's own filtering.
            grain: 2.0,
        }
    }
}

impl PaintBrush {
    /// Apply one step of the stroke at a world position, and return the edits it
    /// made.
    ///
    /// `seconds` is how long since the last step, exactly as [`Brush::stroke`]'s
    /// is and for the same reason: a brush covers ground at the same rate
    /// whatever the frame rate is.
    pub fn stroke(
        &self,
        tile: &mut AdtFile,
        working: &mut Working,
        at: [f32; 2],
        seconds: f32,
    ) -> Painted {
        let mut painted = Painted::default();
        if self.texture.is_empty() {
            return painted;
        }
        for index in self.chunks_under(tile, at) {
            let before = ChunkPaint::capture(tile, index);
            match self.paint_chunk(tile, working, index, at, seconds) {
                Step::Painted => {}
                Step::Nothing => continue,
                Step::Based => painted.based.push(index),
                Step::Reused => painted.reused.push(index),
                Step::Full => {
                    painted.full.push(index);
                    continue;
                }
                Step::Base => {
                    painted.base.push(index);
                    continue;
                }
                Step::Absent => {
                    painted.absent.push(index);
                    continue;
                }
            }
            let after = ChunkPaint::capture(tile, index);
            if after == before {
                continue;
            }
            painted.edits.push(Edit::Paint {
                chunk: index,
                before: Box::new(before),
                after: Box::new(after),
            });
        }
        painted
    }

    /// One chunk's share of a step.
    fn paint_chunk(
        &self,
        tile: &mut AdtFile,
        working: &mut Working,
        index: usize,
        at: [f32; 2],
        seconds: f32,
    ) -> Step {
        let Some(chunk) = tile.chunk(index) else {
            return Step::Nothing;
        };
        let origin = chunk.head().position();
        let mut paint = alpha::paint(chunk);

        // **A chunk with no `MCLY` at all takes this texture as its base.**
        //
        // Not a corner case: `development` has 543 such chunks and one whole
        // tile of them, and this returned `Nothing` on every one — so a blank
        // tile could never be given its first texture, which is exactly how it
        // was reported.
        //
        // The base is the one layer that has no blend map — it is the opaque
        // thing every other layer is painted *over* — so there is no partial
        // version of it and the falloff has nothing to act on. The whole chunk
        // becomes this texture. That is the only thing an empty chunk can be
        // given, and [`Painted::based`] is how the caller says so.
        if paint.is_empty() {
            // Neither an eraser nor a stroke confined to existing layers has
            // anything to do with a chunk that has no layers.
            if self.erase || self.existing_only {
                return match self.erase {
                    true => Step::Nothing,
                    false => Step::Absent,
                };
            }
            paint.layers.push(TextureLayer {
                texture_id: tile.name_texture(&self.texture),
                flags: 0,
                alpha_offset: 0,
                effect_id: self.effect_id,
            });
            paint.maps.push(vec![255u8; ALPHA_LEN]);
            if let Some(chunk) = tile.chunk_mut(index) {
                alpha::set_paint(chunk, &paint);
            }
            // The working copy is dropped with it: the chunk it was seeded from
            // had no layers, and the next step of this stroke re-seeds from a
            // chunk that has one.
            working.chunks.remove(&index);
            return Step::Based;
        }

        // **The texture is named only once it is known the chunk can take it.**
        // Appending to `MTEX` for a chunk that is already full would leave the
        // tile carrying a tileset nothing draws.
        let named = tile
            .texture_names()
            .iter()
            .position(|name| name.eq_ignore_ascii_case(&self.texture))
            .map(|at| at as u32);
        let mut reused = false;
        let target = match named.and_then(|id| paint.layer_of(id)) {
            Some(layer) => layer,
            // An eraser takes a texture away and has none to take here.
            None if self.erase => return Step::Nothing,
            None if self.existing_only => return Step::Absent,
            None if !paint.has_room() => {
                // A full chunk. The layer that shows least is given to this
                // texture when it shows almost nothing; otherwise the stroke
                // is refused here, as it always was.
                let hidden = match self.reuse_hidden {
                    true => least_shown(&paint).filter(|&(_, shown)| shown < REUSE_UNDER),
                    false => None,
                };
                let Some((layer, _)) = hidden else {
                    return Step::Full;
                };
                paint.layers[layer].texture_id = tile.name_texture(&self.texture);
                paint.layers[layer].effect_id = self.effect_id;
                paint.maps[layer] = vec![0u8; ALPHA_LEN];
                // The stroke's own copy of that layer was the old texture's.
                if let Some(wet) = working.chunks.get_mut(&index) {
                    if let Some(map) = wet.get_mut(layer) {
                        map.fill(0.0);
                    }
                }
                reused = true;
                layer
            }
            None => {
                paint.layers.push(TextureLayer {
                    texture_id: tile.name_texture(&self.texture),
                    flags: 0,
                    alpha_offset: 0,
                    effect_id: self.effect_id,
                });
                paint.maps.push(vec![0u8; ALPHA_LEN]);
                paint.len() - 1
            }
        };
        // The base has no blend map. Painting it clears what is over it, which
        // is the same picture; erasing it has no meaning.
        if self.erase && target == 0 {
            return Step::Base;
        }

        // **The stroke's own copy, at full precision** — see [`Working`], which
        // is where the whole argument is. Seeded from the file the first time
        // this stroke touches this chunk, and grown when the step above has just
        // given the chunk a layer it did not have.
        let wet = working
            .chunks
            .entry(index)
            .or_insert_with(|| as_working(&paint.maps));
        while wet.len() < paint.maps.len() {
            wet.push(vec![0.0; ALPHA_LEN]);
        }

        for texel in 0..ALPHA_LEN {
            let (tx, ty) = (texel % ALPHA_SIDE, texel / ALPHA_SIDE);
            let [x, y] = alpha::texel_position(origin, tx, ty);
            // **In the brush's own shape** — see [`Shape`]. A texel is 0.52
            // yards, so a square brush here has a genuinely straight edge
            // rather than a stepped one.
            let distance = self.shape.distance(x - at[0], y - at[1]);
            let weight = self.falloff.over(distance / self.radius, self.core);
            if weight == 0.0 {
                continue;
            }
            // A density under 1 paints in patches: the texels where the
            // place's own value is under the density, with a soft edge a
            // twelfth of the range wide so a patch does not end on a texel.
            let patch = match self.density < 1.0 {
                true => ((self.density - speckle(x, y, self.grain)) * 12.0 + 0.5).clamp(0.0, 1.0),
                false => 1.0,
            };
            if patch == 0.0 {
                continue;
            }
            // Clamped so a long frame cannot overshoot and oscillate, which is
            // the same guard `Brush::moved` puts on its two converging modes.
            let step = (self.strength * weight * patch * seconds).clamp(0.0, 1.0);
            if self.erase {
                // The texture's own layer toward transparent, and nothing
                // else: what is under it shows again.
                wet[target][texel] -= wet[target][texel] * step;
                continue;
            }
            let opacity = self.opacity.clamp(0.0, 1.0);
            // The target layer toward the opacity asked for, from either
            // side…
            wet[target][texel] += (255.0 * opacity - wet[target][texel]) * step;
            // …and everything painted over it toward transparent, which is the
            // half a brush that "does not work" is missing. Scaled by the
            // opacity, so a faint stroke also uncovers the texture faintly.
            for above in target + 1..wet.len() {
                wet[above][texel] -= wet[above][texel] * step * opacity;
            }
        }

        // …and the file, written from it. Quantised **once, here**, rather than
        // once per frame: what the chunk can hold is sixteen levels or 256, and
        // `encode_alpha_maps` is what applies that on the way out.
        let mut moved = false;
        for (layer, map) in paint.maps.iter_mut().enumerate() {
            let Some(source) = wet.get(layer) else { continue };
            for (texel, slot) in map.iter_mut().enumerate() {
                let value = source[texel].clamp(0.0, 255.0).round() as u8;
                if *slot != value {
                    *slot = value;
                    moved = true;
                }
            }
        }
        if !moved && !reused {
            return Step::Nothing;
        }
        // **The base layer is the one that cannot be blended.** If the brush was
        // asked for a texture that is layer 0 there is nothing to raise — it is
        // already under everything — so what the loop above did was clear the
        // layers over it, which is the same picture and the right one.
        if let Some(chunk) = tile.chunk_mut(index) {
            alpha::set_paint(chunk, &paint);
        }
        match reused {
            true => Step::Reused,
            false => Step::Painted,
        }
    }

    /// The chunks whose square the brush's circle reaches. The same rule
    /// [`Brush::chunks_under`] uses, kept beside its own brush rather than
    /// shared, because the two are free to grow apart — a paint brush has no
    /// reason to reach a neighbour the way a height brush does for shading.
    fn chunks_under(&self, tile: &AdtFile, at: [f32; 2]) -> Vec<usize> {
        (0..tile.chunks.len())
            .filter(|&i| {
                let Some(chunk) = tile.chunk(i) else {
                    return false;
                };
                let origin = chunk.head().position();
                let near = |o: f32, p: f32| p >= o - CHUNK_SIZE - self.radius && p <= o + self.radius;
                near(origin[0], at[0]) && near(origin[1], at[1])
            })
            .collect()
    }
}

/// The layer over the base that shows least of the chunk, with how much it
/// shows. `None` for a chunk with only a base.
fn least_shown(paint: &alpha::Paint) -> Option<(usize, f32)> {
    paint
        .coverage()
        .into_iter()
        .enumerate()
        .skip(1)
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// One chunk's blend maps as the stroke works on them.
fn as_working(maps: &[Vec<u8>]) -> Vec<Vec<f32>> {
    maps.iter()
        .map(|map| map.iter().map(|&alpha| f32::from(alpha)).collect())
        .collect()
}

/// What one chunk's share of a paint step came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Painted,
    /// The chunk had no layers and this texture is now its base — see
    /// [`Painted::based`].
    Based,
    /// The brush reached this chunk and changed nothing — outside the falloff,
    /// or already where it is going.
    Nothing,
    /// **It carries four textures and none of them is this one.** Reported
    /// rather than swallowed; see [`Painted`].
    Full,
    /// It was full, and a layer that showed almost nothing now carries this
    /// texture. See [`Painted::reused`].
    Reused,
    /// An eraser reached a chunk whose base is the texture. See
    /// [`Painted::base`].
    Base,
    /// The stroke is confined to existing layers and the chunk does not carry
    /// the texture. See [`Painted::absent`].
    Absent,
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
/// [`Brush::stroke`] and [`grade::Grade::write_into`] — and every one of them
/// owes both properties.
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
    let (x, y) = ((index % 16) as i32, (index / 16) as i32);
    [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)]
        .into_iter()
        .filter_map(|(dx, dy)| {
            let (x, y) = (x + dx, y + dy);
            ((0..16).contains(&x) && (0..16).contains(&y)).then(|| y as usize * 16 + x as usize)
        })
        .collect()
}

/// A ramp between two points — the one shape a round brush cannot make.
pub mod grade;

/// Carrying the placements that stand on ground an edit moved.
pub mod follow;

/// A tile's heights and texture blends as pictures, written out and read back.
pub mod image;

/// A set of vertices chosen first and moved together, or kept from a brush.
pub mod vertices;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod unit {
    use super::*;

    /// Every falloff is one at the centre and nothing at the rim, which is what
    /// stops a stroke from leaving a step where the brush ended.
    ///
    /// **The list is every variant and not a sample.** A curve added without
    /// this property is a brush that terraces, and the terracing is a thin step
    /// exactly at the radius — which reads as a tool that cannot blend rather
    /// than as one curve being wrong.
    #[test]
    fn a_falloff_runs_from_one_to_nothing() {
        for falloff in [
            Falloff::Flat,
            Falloff::Linear,
            Falloff::Smooth,
            Falloff::Sharp,
            Falloff::Dome,
        ] {
            assert_eq!(falloff.weight(0.0), 1.0, "{falloff:?} at the centre");
            assert_eq!(falloff.weight(1.0), 0.0, "{falloff:?} at the rim");
            assert_eq!(falloff.weight(2.0), 0.0, "{falloff:?} outside");
            // …and it never climbs on the way out, which is what "falloff"
            // means and what a mistyped exponent quietly breaks.
            let mut last = 1.0;
            for step in 0..=20 {
                let w = falloff.weight(step as f32 / 20.0);
                assert!(w <= last + 1e-6, "{falloff:?} rises at {step}");
                last = w;
            }
        }
        assert_eq!(Falloff::Linear.weight(0.5), 0.5);
        assert!(Falloff::Smooth.weight(0.5) > 0.5, "smooth is flatter in the middle");
        // **The two new ones are each other's opposite**, which is the whole of
        // why both exist: at the half-way point a dome still has most of its
        // strength and a sharp has almost none.
        assert!(Falloff::Dome.weight(0.5) > Falloff::Smooth.weight(0.5), "dome is fuller");
        assert!(Falloff::Sharp.weight(0.5) < Falloff::Linear.weight(0.5), "sharp collapses");
    }

    /// **A shape is a distance, and its outline is the rim that distance
    /// describes.**
    ///
    /// The property that makes shapes free: the falloff never learns about them,
    /// so every point the outline draws has to be exactly at `t == 1`. A ring
    /// drawn from a second reading of the shape would say the brush reaches
    /// somewhere it does not, which is the one thing a preview must not do.
    #[test]
    fn a_shapes_outline_is_its_own_rim() {
        for shape in [Shape::Circle, Shape::Square, Shape::Diamond] {
            for (x, y) in shape.outline(17.0, 64) {
                let t = shape.distance(x, y) / 17.0;
                assert!((t - 1.0).abs() < 1e-3, "{shape:?} at ({x}, {y}) is {t} out");
            }
        }
        // …and the three really are different shapes: along a diagonal, a square
        // reaches furthest and a diamond least.
        let diagonal = |shape: Shape| shape.distance(1.0, 1.0);
        assert!(diagonal(Shape::Square) < diagonal(Shape::Circle));
        assert!(diagonal(Shape::Circle) < diagonal(Shape::Diamond));
        // …and along an axis all three agree, which is what says they are norms
        // on one radius rather than three unrelated sizes.
        for shape in [Shape::Circle, Shape::Square, Shape::Diamond] {
            assert!((shape.distance(3.0, 0.0) - 3.0).abs() < 1e-6, "{shape:?}");
        }
    }

    /// **Noise depends on where a vertex is and on nothing else.**
    ///
    /// The property a held stroke rests on: asked twice for the same place it
    /// answers the same, so the brush deepens the same bumps instead of shaking
    /// the ground.
    #[test]
    fn noise_is_a_function_of_the_place_and_not_of_the_call() {
        assert_eq!(wobble(-9450.0, -50.0, 25.0), wobble(-9450.0, -50.0, 25.0));
        assert!(wobble(-9450.0, -50.0, 25.0) != wobble(-9445.8, -50.0, 25.0));
        assert!(wobble(-9450.0, -50.0, 25.0) != wobble(-9450.0, -45.8, 25.0));
        // A different scale is a different field, or the setting does nothing.
        assert!(wobble(-9450.0, -50.0, 25.0) != wobble(-9450.0, -50.0, 60.0));
        // In range, and not stuck to one side of it: a hundred samples a cell
        // apart have to straddle zero, or "roughen" is "raise".
        let mut low = 0;
        for step in 0..100 {
            let w = wobble(step as f32 * 4.17, 0.0, 25.0);
            assert!((-1.0..=1.0).contains(&w), "{w} out of range");
            if w < 0.0 {
                low += 1;
            }
        }
        assert!((20..80).contains(&low), "{low} of 100 below zero");
    }

    /// **The noise is smooth at the scale it is given**, which is what the scale
    /// is for.
    ///
    /// The reported fault was *"too jagged"*, and the measurement behind it is
    /// this: hashing the position directly gives neighbouring vertices
    /// *unrelated* numbers, so the step between them is as large as the range
    /// itself. Value noise on a lattice coarser than the vertex spacing gives
    /// them nearly the same number. The check is the mean step between vertices
    /// 4.17 yards apart, and it has to fall as the scale rises — otherwise the
    /// panel's control is a number that changes nothing.
    #[test]
    fn a_coarser_noise_is_a_smoother_one() {
        let roughness = |scale: f32| {
            let mut total = 0.0;
            let mut steps = 0;
            for i in 0..200 {
                let x = i as f32 * 4.17;
                total += (wobble(x + 4.17, 0.0, scale) - wobble(x, 0.0, scale)).abs();
                steps += 1;
            }
            total / steps as f32
        };
        let fine = roughness(4.17);
        let coarse = roughness(50.0);
        assert!(
            coarse < fine * 0.5,
            "a 50 yd scale steps {coarse} between vertices against {fine} at 4.17"
        );
        // …and it keeps falling, rather than bottoming out at some middle value.
        assert!(roughness(200.0) < coarse, "200 yd is not smoother than 50");
    }

    /// **A core holds the middle of the brush at full strength**, and a core of
    /// nothing is the curve it was before.
    ///
    /// The second half is what makes it safe to add: every brush in the crate
    /// defaults to zero, so nothing that was tuned against the old behaviour
    /// moved.
    #[test]
    fn a_core_is_the_part_that_does_not_fall_off() {
        for falloff in [
            Falloff::Flat,
            Falloff::Linear,
            Falloff::Smooth,
            Falloff::Sharp,
            Falloff::Dome,
        ] {
            for step in 0..=20 {
                let t = step as f32 / 20.0;
                assert_eq!(falloff.over(t, 0.0), falloff.weight(t), "{falloff:?} at {t}");
            }
            // Everything inside the core is full strength, the rim is still
            // nothing, and it never climbs on the way out.
            assert_eq!(falloff.over(0.49, 0.5), 1.0, "{falloff:?} inside the core");
            assert_eq!(falloff.over(1.0, 0.5), 0.0, "{falloff:?} at the rim");
            let mut last = 1.0;
            for step in 0..=20 {
                let w = falloff.over(step as f32 / 20.0, 0.5);
                assert!(w <= last + 1e-6, "{falloff:?} rises at {step}");
                last = w;
            }
            // …and a bigger core is never weaker anywhere, which is the whole
            // of what the control promises.
            for step in 0..=20 {
                let t = step as f32 / 20.0;
                assert!(
                    falloff.over(t, 0.6) >= falloff.over(t, 0.3) - 1e-6,
                    "{falloff:?} at {t} got weaker with a larger core"
                );
            }
        }
    }

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
