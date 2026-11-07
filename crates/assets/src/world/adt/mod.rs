//! ADT — one terrain tile, 16x16 map chunks (`MCNK`) of heightmap, texture
//! layers and object placements.
//!
//! ```text
//! tests.rs   placements, normals, vertex colours, alpha maps and the writers'
//!            round trips
//! ```
//!
//! Top-level chunks:
//! ```text
//! MVER  u32 version (18)
//! MHDR  64-byte header of offsets (we do not rely on it; see below)
//! MCIN  256 x { u32 offset, u32 size, u32 flags, u32 asyncId } — MCNK index
//! MTEX  \0-separated texture (.blp) filenames
//! MMDX  \0-separated M2 model filenames        MMID  u32 offsets into MMDX
//! MWMO  \0-separated WMO filenames             MWID  u32 offsets into MWMO
//! MDDF  36-byte doodad (M2) placements
//! MODF  64-byte WMO placements
//! MCNK  x256, each a 128-byte header followed by sub-chunks
//! ```
//!
//! We walk the top-level chunks sequentially rather than following `MHDR`
//! offsets: the offsets are relative to different bases across documentation
//! and tooling, and a sequential walk is both simpler and version-tolerant.

use crate::world::chunk::{self, ChunkReader};
use crate::world::wmo::{Liquid, WmoLiquid, LIQUID_VERTEX_SIZE};
use crate::AssetError;

/// Side length of one ADT tile in world units (yards). 1600/3.
pub const TILE_SIZE: f32 = 533.333_33;

/// Side length of one MCNK map chunk. 16 chunks per tile side.
pub const CHUNK_SIZE: f32 = TILE_SIZE / 16.0;

/// Spacing between adjacent heightmap samples. 8 cells per chunk side.
pub const UNIT_SIZE: f32 = CHUNK_SIZE / 8.0;

/// World coordinate of tile (0, 0)'s corner. Tile (32, 32) straddles the
/// origin, so the map runs from +17066 down to -17066.
pub const MAP_ORIGIN: f32 = 32.0 * TILE_SIZE;

/// MCNKs per tile side.
pub const CHUNKS_PER_SIDE: usize = 16;

/// The 128-byte MCNK header that precedes its sub-chunks.
const MCNK_HEADER_SIZE: usize = 128;

/// `MCNK` header flag bits naming which liquids stand on this chunk.
///
/// These bits are the only declaration of a chunk's liquids: `MCLQ` itself does
/// not say what it holds. Its blocks appear in this bit order, one per set bit,
/// so one chunk can carry both a river and the ocean it flows into. The bit
/// values are vmangos' (`ADT_MCNK_flags` in the extractor) and are checked
/// against real tiles by `vale water`: a lava chunk read as water would show
/// byte 0 of its vertices spread flat across 0..255 (a texture coordinate)
/// instead of water's plateau-and-shore shape.
pub mod mcnk_flags {
    /// The chunk carries an `MCSH` shadow map worth reading.
    ///
    /// `MCSH` is the only shadow 1.12 draws on terrain. The client casts no
    /// shadows at runtime: a building's shadow on the ground was baked by
    /// Blizzard's tools into a 64x64 bitmap per chunk, at the same resolution and
    /// in the same layout as an alpha map, and is drawn by darkening the sun term
    /// where the bit is set. See [`Mcnk::shadow`].
    pub const HAS_MCSH: u32 = 0x01;
    pub const LQ_RIVER: u32 = 0x04;
    pub const LQ_OCEAN: u32 = 0x08;
    pub const LQ_MAGMA: u32 = 0x10;
    pub const LQ_SLIME: u32 = 0x20;
    /// Any of the four liquid bits: the chunk has an `MCLQ` to read.
    pub const LQ_ANY: u32 = LQ_RIVER | LQ_OCEAN | LQ_MAGMA | LQ_SLIME;
}

/// One `MCLQ` liquid block: `f32 min, f32 max`, 81 8-byte vertices (the same
/// union as a WMO's `MLIQ` — see [`crate::world::wmo::WmoLiquid::depths`]), 64 tile
/// flag bytes, then `u32 nFlowvs` and two 40-byte flow vectors that are always
/// present whatever `nFlowvs` says. The flow tail is skipped but it is part of
/// the stride to the next block.
const MCLQ_VERTS: usize = (INNER_SIDE + 1) * (INNER_SIDE + 1);
const MCLQ_TILES: usize = INNER_SIDE * INNER_SIDE;
const MCLQ_TILES_OFFSET: usize = 8 + MCLQ_VERTS * LIQUID_VERTEX_SIZE;
const MCLQ_BLOCK_SIZE: usize = MCLQ_TILES_OFFSET + MCLQ_TILES + 4 + 2 * 40;

/// Heights per MCNK: a 9x9 outer grid interleaved with an 8x8 inner grid.
pub const OUTER_SIDE: usize = 9;
pub const INNER_SIDE: usize = 8;
pub const HEIGHTS_PER_CHUNK: usize = OUTER_SIDE * OUTER_SIDE + INNER_SIDE * INNER_SIDE; // 145

/// One doodad (M2) placement from `MDDF`.
#[derive(Debug, Clone)]
pub struct DoodadPlacement {
    /// Index into [`Adt::model_names`].
    pub name_id: u32,
    pub unique_id: u32,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    /// Stored as a fixed-point u16 where 1024 == 1.0; exposed already divided.
    pub scale: f32,
    pub flags: u16,
}

/// One WMO placement from `MODF`.
#[derive(Debug, Clone)]
pub struct WmoPlacement {
    /// Index into [`Adt::wmo_names`].
    pub name_id: u32,
    pub unique_id: u32,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub bounds_lower: [f32; 3],
    pub bounds_upper: [f32; 3],
    pub flags: u16,
    pub doodad_set: u16,
    pub name_set: u16,
}

/// A model placement resolved into world space, ready for a renderer.
#[derive(Debug, Clone)]
pub struct PlacedModel {
    /// Archive path with the extension already fixed up to `.m2` / `.wmo`.
    pub path: String,
    /// `MDDF`/`MODF` unique id — the same object placed in two neighbouring
    /// tiles carries the same id in both, which is how a doodad straddling a
    /// seam is drawn once instead of twice.
    pub unique_id: u32,
    /// World position (+X north, +Y west, +Z up).
    pub position: [f32; 3],
    /// Column-major 4x4 model-to-world matrix, scale included, ready to upload
    /// as a `mat4` uniform.
    pub matrix: [f32; 16],
    pub scale: f32,
    /// Which of a WMO's doodad sets to draw; meaningless for an M2.
    pub doodad_set: u16,
    /// The WMO's name set for this placement: `MODF` 0x3C, the second key into
    /// `WMOAreaTable.dbc`. Meaningless for an M2.
    ///
    /// One inn model is placed in many villages, and each placement is a
    /// different tavern: the same file and the same groups with a different name
    /// set. The name set is the only field that distinguishes "Lion's Pride Inn"
    /// from "Deepwater Tavern". See [`crate::tables::wmoarea`].
    pub name_set: u16,
}

/// `MDDF`/`MODF` position -> world position.
///
/// Placements are not stored in world coordinates. They are measured from
/// the map's corner in the axis order vmangos calls the "internal
/// representation": `(x = westward, y = up, z = northward)`, all offset from
/// `MAP_ORIGIN`.
///
/// Both the axis order and the offset are confirmed two ways:
///
/// * vmangos' vmap extractor reads the same 12 bytes as
///   `pos = fixCoords(v) = (v.z, v.x, v.y)`, and its server converts world to
///   internal with `pos.x = mid - x; pos.y = mid - y; pos.z = z`
///   (`VMapManager2::convertPositionToInternalRep`, `mid = 32 * 533.33`);
/// * measured on `Azeroth_34_51`, whose 1391 doodads then land inside the
///   tile's own `MCNK` bounds (world x -10706..-10069 against chunk origins
///   -10633..-10133, plus the ~50-yard overhang of objects shared with the
///   neighbouring tile) and whose heights, 17.0..171.0, sit exactly on the
///   tile's terrain range of 18.3..170.7.
pub fn placement_to_world(p: [f32; 3]) -> [f32; 3] {
    [MAP_ORIGIN - p[2], MAP_ORIGIN - p[0], p[1]]
}

/// World position -> `MDDF`/`MODF` position: the inverse of
/// [`placement_to_world`], used by anything that writes a placement.
///
/// The axes swap and two of them are subtracted from [`MAP_ORIGIN`], so
/// composing the two functions round trips exactly. It is provided here rather
/// than left to each caller because loading the game's tiles never exercises
/// this direction, and a caller that inverts the swap wrongly writes a
/// placement up to a hundred yards from the intended position.
pub fn placement_from_world(w: [f32; 3]) -> [f32; 3] {
    [MAP_ORIGIN - w[1], w[2], MAP_ORIGIN - w[0]]
}

/// `MDDF`/`MODF`'s three angles, reordered and re-signed as right-handed turns
/// about the world's x, y and z.
///
/// The file stores the triple in an order and with signs that differ from the
/// world's axes. [`placement_matrix`] derives the composition; in short:
///
/// ```text
/// M = Rz(rot[1]) * Ry(-rot[0]) * Rx(-rot[2]) * Rz(180°)
/// ```
///
/// so `rot[1]` is a turn about the world's z, and the other two are turns
/// about y and x with the opposite sign. Code that displays an angle, or sets
/// one from an on-screen handle, uses this function; code that writes the
/// record uses [`placement_euler_from_world`], its exact inverse.
///
/// This function converts the angles, not the axes they turn about. The three
/// turns compose, so once either of the others is non-zero only the first, the
/// one about z, is a turn about a world axis. In that case each angle turns
/// about the axis carried by the rotations outside it, which the caller must
/// account for.
pub fn placement_euler_to_world(rotation_deg: [f32; 3]) -> [f32; 3] {
    [-rotation_deg[2], -rotation_deg[0], rotation_deg[1]]
}

/// World-axis angles -> the `MDDF`/`MODF` triple: the inverse of
/// [`placement_euler_to_world`], used by anything that writes a placement.
pub fn placement_euler_from_world(world_deg: [f32; 3]) -> [f32; 3] {
    [-world_deg[1], world_deg[2], -world_deg[0]]
}

/// The model-to-world matrix for a placement, column-major.
///
/// `rotation_deg` is the raw `MDDF`/`MODF` triple in degrees, `position` is
/// already world-space (see [`placement_to_world`]), and model vertices are the
/// raw M2 ones. M2 vertices are Z-up, as measured: `WestfallTree02` spans
/// -0.12..21.29 on its third axis and roughly ±6 on the other two, i.e. a
/// 21-yard tree standing on z = 0.
///
/// ## Where the matrix comes from
///
/// vmangos places a doodad in internal space as
/// `p = iPos + fromEulerAnglesZYX(rot.y, rot.x, rot.z) * (scale * v)`
/// (`TileAssembler::ModelPosition::init`, `ModelInstance::ModelInstance`), and
/// G3D's `fromEulerAnglesZYX(a, b, c)` is `Rz(a) * Ry(b) * Rx(c)`.
///
/// Internal and world space differ by a 180° turn about Z (`x` and `y` are both
/// negated), call it `T`. Conjugating the rotation by `T` flips the sign of the
/// X and Y angles but leaves Z alone, and the same `T` has to be applied to the
/// model's own vertices, so:
///
/// ```text
/// M = Rz(rot.y) * Ry(-rot.x) * Rx(-rot.z) * Rz(180°) * scale
/// ```
///
/// The trailing 180° is that frame flip. Leaving it out is hard to see on a
/// tree, which is roughly symmetric about Z, and plainly wrong on a building.
///
/// The extractor's own `fixCoordSystem` (`(x, z, -y)`) is not part of this:
/// `Model::ConvertToVMAPModel` immediately writes the vertices back as
/// `(x, -z', y')`, and the two cancel. Applying only `fixCoordSystem` lays every
/// model on its side.
pub fn placement_matrix(position: [f32; 3], rotation_deg: [f32; 3], scale: f32) -> [f32; 16] {
    let [rx, ry, rz] = rotation_deg.map(f32::to_radians);

    // Rz(ry) * Ry(-rx) * Rx(-rz) * Rz(pi), each right-handed, composed as
    // 3x3s and then scaled. Written out rather than assembled from a matrix
    // library because this crate has no linear algebra dependency and the
    // product is fixed.
    let m = mul3(
        &mul3(&rot_z(ry), &rot_y(-rx)),
        &mul3(&rot_x(-rz), &rot_z(std::f32::consts::PI)),
    );

    // Column-major: columns 0..2 are the scaled basis, column 3 the position.
    let mut out = [0.0f32; 16];
    for col in 0..3 {
        for row in 0..3 {
            out[col * 4 + row] = m[row][col] * scale;
        }
    }
    out[12] = position[0];
    out[13] = position[1];
    out[14] = position[2];
    out[15] = 1.0;
    out
}

type Mat3 = [[f32; 3]; 3];

fn mul3(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0f32; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

fn rot_x(t: f32) -> Mat3 {
    let (s, c) = t.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}

fn rot_y(t: f32) -> Mat3 {
    let (s, c) = t.sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn rot_z(t: f32) -> Mat3 {
    let (s, c) = t.sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// One texture layer on a map chunk (`MCLY`).
///
/// ```text
/// u32 textureId     index into MTEX
/// u32 flags
/// u32 offsetInMCAL  byte offset of this layer's alpha map within MCAL
/// u32 effectId
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureLayer {
    /// Index into [`Adt::texture_names`].
    pub texture_id: u32,
    pub flags: u32,
    /// Offset of this layer's alpha map inside the chunk's `MCAL`.
    pub alpha_offset: u32,
    pub effect_id: u32,
}

/// `MCLY` flag bits that change how the layer is read.
pub mod layer_flags {
    /// Scroll direction of the layer's texture, three bits, 0..7; each step is
    /// 45° round the compass. Layers such as lava, waterfalls and cloud
    /// shadows scroll their UVs; every other layer is still.
    ///
    /// A census of every tile by `vale textures` (run with no tile) fixes these
    /// four constants against the two below them: `0x100` is set on exactly the
    /// layers that are not the base, and every bit from `0x01` to `0x40` is
    /// used, so the low byte is three bits of direction, three of speed and an
    /// enable bit.
    pub const ANIMATION_ROTATION: u32 = 0x007;
    /// Scroll speed, three bits, 0..7. See [`ANIMATION_SPEED_SHIFT`].
    pub const ANIMATION_SPEED: u32 = 0x038;
    /// How far to shift [`ANIMATION_SPEED`] down to read it as a number.
    pub const ANIMATION_SPEED_SHIFT: u32 = 3;
    /// Enables scrolling. A layer with a direction and a speed but without this
    /// bit is still.
    pub const ANIMATION_ENABLED: u32 = 0x040;
    /// The layer is drawn at twice brightness.
    pub const OVERBRIGHT: u32 = 0x080;
    /// The layer has an alpha map in `MCAL`. Layer 0 never does — it is the
    /// base and covers the whole chunk opaquely.
    pub const USE_ALPHA_MAP: u32 = 0x100;
    /// The alpha map is RLE-compressed. Introduced after 1.12; the parser
    /// handles it anyway because patched archives are not always what the
    /// client version implies.
    pub const ALPHA_COMPRESSED: u32 = 0x200;

    /// The layer's scroll direction and speed as a UV velocity, in texture
    /// widths per second, as the renderer uses it.
    ///
    /// `None` when [`ANIMATION_ENABLED`] is clear, which is true of all but a
    /// few layers in the game.
    ///
    /// The direction is `rotation * 45°` measured clockwise from +V, so step 0
    /// scrolls up the texture and step 2 scrolls right. The speed is
    /// `2^(speed) / SPEED_DIVISOR`, a geometric series, so that a three-bit
    /// field covers both a slow cloud shadow and a fast waterfall. See
    /// [`ANIMATION_BASE_RATE`].
    pub fn scroll(flags: u32) -> Option<(f32, f32)> {
        if flags & ANIMATION_ENABLED == 0 {
            return None;
        }
        let turn = (flags & ANIMATION_ROTATION) as f32 * std::f32::consts::FRAC_PI_4;
        let rate = ANIMATION_BASE_RATE * (1u32 << ((flags & ANIMATION_SPEED) >> SPEED)) as f32;
        Some((turn.sin() * rate, turn.cos() * rate))
    }

    const SPEED: u32 = ANIMATION_SPEED_SHIFT;

    /// Scroll rate at speed 0, in texture widths per second.
    ///
    /// This is the only value in this module that is not taken from the file,
    /// and it is a separate constant so that the estimate is kept apart from
    /// the file's data. The three bits are a scale with no unit in the record;
    /// the client decides what a step is worth. The value here is chosen so that
    /// a texture at speed 0 takes about half a minute to scroll its own width
    /// and one at speed 7 takes a quarter of a second, the range that scrolling
    /// lava and drifting shadows need between them. Check it against the client
    /// before relying on the absolute rate; the direction and the relative steps
    /// come from the file.
    pub const ANIMATION_BASE_RATE: f32 = 1.0 / 32.0;
}

/// The fallback normal for terrain with no `MCNR`: straight up.
const UP: [f32; 3] = [0.0, 0.0, 1.0];

/// Side length of a chunk's alpha map, in texels.
pub const ALPHA_SIDE: usize = 64;
/// Decoded alpha map size: one byte per texel.
pub const ALPHA_LEN: usize = ALPHA_SIDE * ALPHA_SIDE;

/// Side of the tile-wide alpha atlas, in texels: the 16x16 `MCNK` grid of 64x64
/// alpha maps, laid out as one texture. See [`Adt::alpha_atlas`].
pub const ATLAS_SIDE: usize = CHUNKS_PER_SIDE * ALPHA_SIDE;

/// A single 33.3-yard map chunk.
#[derive(Debug, Clone)]
pub struct Mcnk {
    pub index_x: u32,
    pub index_y: u32,
    pub area_id: u32,
    /// The raw MCNK header flags. The liquid bits are [`mcnk_flags`]; the rest
    /// are kept because they cost nothing and the header is already in hand.
    pub flags: u32,
    /// Bitmask of 4x4 sub-cells that are holes (no terrain drawn).
    pub holes: u16,
    /// Chunk origin in world space, straight from the file.
    pub position: [f32; 3],
    /// 145 absolute heights (chunk `position.z` already added), in file order:
    /// alternating rows of 9 outer and 8 inner samples.
    pub heights: Vec<f32>,
    /// 145 unit normals from `MCNR`, in the same interleaved order as
    /// `heights` and already in world axes.
    ///
    /// Without them every cell is flat-shaded, because averaging normals across
    /// a mesh whose vertices are duplicated per cell cannot smooth anything.
    pub normals: Vec<[f32; 3]>,
    /// `MCCV`: 145 per-vertex shading multipliers, parallel to `heights`, or
    /// empty when the chunk carries none, which is every chunk of every shipped
    /// 1.12 tile. [`decode_colours`] documents the format and its use.
    pub colours: Vec<[f32; 4]>,
    pub layers: Vec<TextureLayer>,
    /// One decoded 64x64 alpha map per layer, always 8-bit whatever the file
    /// stored, and always [`ALPHA_LEN`] bytes.
    ///
    /// Parallel to `layers`, so `alphas[i]` blends `layers[i]` over everything
    /// below it. Layer 0 has no alpha map in the file — it is the opaque base —
    /// and gets an all-255 map here so the renderer can treat every layer the
    /// same way.
    pub alphas: Vec<Vec<u8>>,
    /// `MCSH`: the shadow the world's buildings and trees cast on this chunk,
    /// expanded from its 512 packed bits to one byte per texel, `255` where the
    /// ground is in shadow. [`ALPHA_LEN`] bytes, or empty when the chunk
    /// declares none, which is the case for most chunks in open ground.
    ///
    /// This is the only terrain shadow 1.12 draws. The client casts no shadows
    /// at runtime: there is no shadow map, no projection and nothing that moves
    /// with the sun. The shadows under Stormwind's walls are this bitmap, baked
    /// by Blizzard's tools at the same 64x64 per chunk an alpha map uses and at
    /// the same texel centres, so it is stored in the same atlas (see
    /// [`Adt::alpha_atlas`]) and gets the same [`fix_alpha_edge`] treatment.
    ///
    /// The shadow's darkness is not stored here: it is `Light.dbc`'s band 17,
    /// per map and per hour (see [`crate::tables::light`]), so a baked shadow
    /// fades out at dusk instead of staying on the ground at midnight.
    pub shadow: Vec<u8>,
    /// The chunk's `MCLQ` liquids: the lake, river, ocean or lava standing on
    /// this chunk, one entry per liquid flag bit in [`Self::flags`].
    ///
    /// Stored as the same [`WmoLiquid`] a WMO's `MLIQ` parses into, because it
    /// is the same structure (the same 8-byte vertex union, the same low-nibble
    /// tile rule) in a different frame. The grid is re-ordered at parse so
    /// `base` is the chunk's low corner and the axes run +X/+Y like a WMO's, so
    /// every rule written for `MLIQ` ([`WmoLiquid::has_liquid`],
    /// [`WmoLiquid::opacity`], [`WmoLiquid::vertex_is_wet`]) applies unchanged.
    /// `vertex` here answers in world space, since a map chunk's frame is the
    /// world's.
    pub liquids: Vec<(Liquid, WmoLiquid)>,
    /// The dominant texture layer in each of the 64 detail cells: the MCNK
    /// header's `ReallyLowQualityTextureingMap` at `0x40`, eight `u16`s of
    /// eight 2-bit entries.
    ///
    /// One row per entry, cells running along decreasing world x and y like
    /// every other 8x8 and 64x64 map in a chunk, so cell `(row, col)` is
    /// `detail_layer[row] >> (col * 2) & 3`. Read
    /// [`Mcnk::detail_layer_at`] rather than unpacking it by hand.
    ///
    /// This is the dominance the file stores, precomputed, not one derived from
    /// the alpha maps. [`Adt::ground_effect_at`] answers the same question a
    /// different way (the topmost layer whose alpha reaches half at that
    /// texel), which is what a footstep needs at one point. Ground effects use
    /// this field, per whole cell. On `Azeroth_34_51` all 16,384 cell entries
    /// name a layer the chunk has, which confirms the field's meaning and
    /// packing.
    pub(crate) detail_layer: [u16; 8],
    /// The detail cells that grow no ground effects: `noEffectDoodad` at
    /// `0x50`, one bit per cell, `row * 8 + col`, set where Blizzard's tools
    /// marked the ground bare.
    ///
    /// 6,560 of `Azeroth_34_51`'s 16,384 cells are suppressed, in contiguous
    /// runs: a path, a bare rock face, the ground under a building. Without
    /// this mask, grass grows through the roads.
    pub(crate) no_detail: u64,
}

impl Mcnk {
    /// Height at outer-grid `(row, col)`, both 0..=8.
    ///
    /// The 145 samples alternate: 9 outer, 8 inner, 9 outer, ... so outer row
    /// `r` starts at `r * 17`.
    #[cfg(test)]
    pub(crate) fn outer_height(&self, row: usize, col: usize) -> f32 {
        self.heights
            .get(row * (OUTER_SIDE + INNER_SIDE) + col)
            .copied()
            .unwrap_or(0.0)
    }

    /// Height at inner-grid `(row, col)`, both 0..=7. Inner row `r` starts at
    /// `r * 17 + 9`.
    #[cfg(test)]
    pub(crate) fn inner_height(&self, row: usize, col: usize) -> f32 {
        self.heights
            .get(row * (OUTER_SIDE + INNER_SIDE) + OUTER_SIDE + col)
            .copied()
            .unwrap_or(0.0)
    }

    /// Normal at outer-grid `(row, col)`, indexed exactly like the heights.
    /// Straight up when the file had no `MCNR`.
    #[cfg(test)]
    pub(crate) fn outer_normal(&self, row: usize, col: usize) -> [f32; 3] {
        self.normals.get(outer_index(row, col)).copied().unwrap_or(UP)
    }

    /// Normal at inner-grid `(row, col)`.
    pub(crate) fn inner_normal(&self, row: usize, col: usize) -> [f32; 3] {
        self.normals.get(inner_index(row, col)).copied().unwrap_or(UP)
    }

    /// The `MCLY` layer dominant in detail cell `(row, col)`, both 0..=7, or
    /// `None` where the cell grows nothing, either because the header
    /// suppressed it or because the layer it names is not one this chunk has.
    ///
    /// The second case does not occur on shipped tiles (measured: 0 of 16,384
    /// on `Azeroth_34_51`); it is checked because a 2-bit field can hold 3
    /// while the chunk has only two layers.
    pub fn detail_layer_at(&self, row: usize, col: usize) -> Option<&TextureLayer> {
        if row >= INNER_SIDE || col >= INNER_SIDE {
            return None;
        }
        if self.no_detail >> (row * INNER_SIDE + col) & 1 != 0 {
            return None;
        }
        let layer = (self.detail_layer[row] >> (col * 2) & 3) as usize;
        self.layers.get(layer)
    }

    /// Is the 4x4 sub-cell containing outer cell `(row, col)` a hole?
    #[cfg(test)]
    pub(crate) fn is_hole(&self, row: usize, col: usize) -> bool {
        // The 16-bit mask is a 4x4 grid; each bit covers a 2x2 block of cells.
        let bit = (row / 2) * 4 + (col / 2);
        self.holes & (1 << bit) != 0
    }

    pub(crate) fn min_max_height(&self) -> (f32, f32) {
        self.heights.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &h| {
            (lo.min(h), hi.max(h))
        })
    }

    /// The liquid surface over a world position, as `(liquid type, world z)`,
    /// or `None` where this chunk's grids say the point is dry.
    ///
    /// This is the terrain part of the liquid query the mover makes twenty
    /// times a second. See [`crate::world::terrain::Terrain::liquid_at`], the
    /// cached form, and `vale_protocol::state::movement::Mover`, which acts on
    /// the result.
    ///
    /// The height is interpolated bilinearly over the tile's four corner
    /// vertices, because that is the surface [`Adt::liquid_surface`] draws.
    /// `MCLQ` states a height per vertex and the mesh is one quad per wet tile,
    /// so a river sloping down a valley is drawn as a ramp. Taking the nearest
    /// vertex instead, as vmangos' extracted `.map` does at one sample per
    /// tile, would put the swimming surface up to half a tile away from the
    /// drawn one. The two agree exactly on flat water, which is all of the
    /// ocean and most of every lake.
    ///
    /// Where grids overlap, the highest wet surface is returned. A chunk may
    /// carry up to four liquids, one per flag bit (a river running into the sea
    /// is the case shipped tiles contain), and the file does not order them, so
    /// a character at that spot is in the one whose surface is higher.
    pub fn liquid_at(&self, x: f32, y: f32) -> Option<(Liquid, f32)> {
        let mut best: Option<(Liquid, f32)> = None;
        for (kind, grid) in &self.liquids {
            let fx = (x - grid.base[0]) / crate::world::wmo::LIQUID_TILE_SIZE;
            let fy = (y - grid.base[1]) / crate::world::wmo::LIQUID_TILE_SIZE;
            if fx < 0.0 || fy < 0.0 {
                continue;
            }
            let (tx, ty) = (fx as usize, fy as usize);
            if tx >= grid.x_tiles || ty >= grid.y_tiles || !grid.has_liquid(tx, ty) {
                continue;
            }
            let (u, v) = (fx - tx as f32, fy - ty as f32);
            let h = grid.height(tx, ty) * (1.0 - u) * (1.0 - v)
                + grid.height(tx + 1, ty) * u * (1.0 - v)
                + grid.height(tx, ty + 1) * (1.0 - u) * v
                + grid.height(tx + 1, ty + 1) * u * v;
            if best.is_none_or(|(_, top)| h > top) {
                best = Some((*kind, h));
            }
        }
        best
    }

    /// Cells this chunk draws: 64 less the ones the hole mask covers.
    ///
    /// The chunk contributes five times this many vertices to a tile mesh. See
    /// [`chunk_mesh_vertices`].
    pub fn drawn_cells(&self) -> usize {
        drawn_cells(self.holes)
    }

    /// This chunk's height field alone, for code that places objects on the
    /// ground after the `Adt` has been dropped.
    ///
    /// A tile is parsed once on a loader thread and then dropped; the meshes,
    /// the placements and the atlas are kept. Code that places objects lazily
    /// (see [`crate::world::foliage`], which generates a chunk's grass only
    /// when the camera is near enough to see it) therefore needs the heights to
    /// outlive the parse. 145 floats per chunk is 580 bytes, against the
    /// megabytes of a whole `Adt`.
    ///
    /// It is a copy of three fields rather than a borrow so that it can be sent
    /// across the channel the loader thread hands its work over on.
    pub fn ground(&self) -> ChunkGround {
        ChunkGround {
            position: self.position,
            heights: self.heights.clone(),
            holes: self.holes,
        }
    }
}

/// One chunk's height field on its own: the least data that answers a ground
/// height query the same way the drawn mesh does.
///
/// Produced by [`Mcnk::ground`]; see there for why it exists. The sampling is
/// the same four-triangle rule [`Adt::height_at`] uses, through the same
/// function, because a second implementation that differed would place models
/// slightly above or below the ground the character walks on.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkGround {
    /// Chunk origin in world space, straight from the file.
    pub position: [f32; 3],
    /// [`HEIGHTS_PER_CHUNK`] samples in `MCVT` order.
    pub heights: Vec<f32>,
    /// The hole mask, so a rule plants nothing over a cave mouth.
    pub holes: u16,
}

impl ChunkGround {
    /// Terrain height at a world position, or `None` outside this chunk and
    /// over holes — [`Adt::height_at`] restricted to one chunk.
    pub fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        // The bound `Adt::height_at` gets from `chunk_at`, stated here because
        // there is no tile to scan: `wedge_in` clamps rather than refusing, so
        // without this a position off the chunk's far edge would answer with
        // the last cell's height instead of nothing.
        if !self.contains(x, y) {
            return None;
        }
        let w = wedge_in(self.position, &self.heights, self.holes, x, y)?;
        Some(barycentric(w.at, w.corners[0], w.corners[1], w.corners[2]))
    }

    /// Terrain normal at a world position: [`Adt::normal_at`] restricted to one
    /// chunk, from the same wedge [`Self::height_at`] answers from.
    pub fn normal_at(&self, x: f32, y: f32) -> Option<[f32; 3]> {
        if !self.contains(x, y) {
            return None;
        }
        Some(wedge_normal(&wedge_in(self.position, &self.heights, self.holes, x, y)?))
    }

    /// Whether the position is inside this chunk's 33.3-yard square at all.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let dx = self.position[0] - x;
        let dy = self.position[1] - y;
        (0.0..CHUNK_SIZE).contains(&dx) && (0.0..CHUNK_SIZE).contains(&dy)
    }
}

/// One vertex of a tile mesh, as [`Adt::to_mesh`] emits it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshVertex {
    /// Which of its cell's five vertices this is: 0 to 3 are the outer corners
    /// in winding order and 4 is the inner-grid centre.
    pub corner: usize,
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Chunk-local, 0..1.
    pub uv: [f32; 2],
    /// `MCCV`'s multiplier at this vertex, or [`COLOUR_NONE`] where the chunk
    /// has none — see [`decode_colours`].
    pub colour: [f32; 4],
}

/// Each drawn cell's place among a chunk's drawn cells, in emission order;
/// `None` for a cell under a hole.
fn cell_ordinals(holes: u16) -> [[Option<u32>; INNER_SIDE]; INNER_SIDE] {
    let mut out = [[None; INNER_SIDE]; INNER_SIDE];
    let mut next = 0;
    for (row, cells) in out.iter_mut().enumerate() {
        for (col, cell) in cells.iter_mut().enumerate() {
            if holes & (1 << ((row / 2) * 4 + col / 2)) == 0 {
                *cell = Some(next);
                next += 1;
            }
        }
    }
    out
}

/// Cells a chunk draws: 64 less the ones its hole mask covers.
///
/// Five vertices each, which is what a chunk contributes to a tile mesh. A free
/// function over the mask so that a caller holding a chunk in some other form
/// can count the same cells — see [`chunk_mesh_vertices`], which walks them.
pub fn drawn_cells(holes: u16) -> usize {
    (0..INNER_SIDE)
        .flat_map(|row| (0..INNER_SIDE).map(move |col| (row, col)))
        .filter(|&(row, col)| holes & (1 << ((row / 2) * 4 + col / 2)) == 0)
        .count()
}

/// The 8x8 cell of a chunk that a world position is in, as `(row, col)`, or
/// `None` when the position is outside the chunk whose origin this is.
///
/// This is the single definition of the grid that a map chunk's three
/// per-cell fields share: the hole mask is this grid at half resolution,
/// `MCLQ`'s wet flags use it directly, and `chunk_mesh_vertices` walks it to
/// emit the ground. Rows run along decreasing world x and columns along
/// decreasing world y from the origin, because the origin is the chunk's
/// maximum corner; `wedge_in` and the mesher go in the same direction.
///
/// It is defined once rather than by each caller because an error here
/// produces no failure: a row and column swapped, or counted from the wrong
/// corner, still names a real cell. The ground would be cut, or the water
/// placed, a few yards from the pointer, and the file would still parse.
pub fn cell_at(position: [f32; 3], x: f32, y: f32) -> Option<(usize, usize)> {
    let fr = (position[0] - x) / UNIT_SIZE;
    let fc = (position[1] - y) / UNIT_SIZE;
    if fr < 0.0 || fc < 0.0 {
        return None;
    }
    let (row, col) = (fr.floor() as usize, fc.floor() as usize);
    (row < INNER_SIDE && col < INNER_SIDE).then_some((row, col))
}

/// The world-space square a block of cells starting at `(row, col)` covers, as
/// `(max corner, min corner)` in x and y, the order the chunk's own origin is
/// in.
///
/// `side` is how many cells on a side: 1 for a liquid tile, 2 for a hole.
pub fn cell_square(
    position: [f32; 3],
    row: usize,
    col: usize,
    side: usize,
) -> ([f32; 2], [f32; 2]) {
    let high = [
        position[0] - row as f32 * UNIT_SIZE,
        position[1] - col as f32 * UNIT_SIZE,
    ];
    let span = side as f32 * UNIT_SIZE;
    (high, [high[0] - span, high[1] - span])
}

/// Which of the sixteen hole bits covers a world position.
///
/// The mask is a 4x4 grid over the chunk's 8x8 cells, so this is [`cell_at`] at
/// half resolution and nothing else.
pub fn hole_bit(position: [f32; 3], x: f32, y: f32) -> Option<usize> {
    let (row, col) = cell_at(position, x, y)?;
    Some((row / 2) * 4 + col / 2)
}

/// The world-space square a hole bit covers. Two cells on a side, so a
/// quarter of the chunk in each axis.
pub fn hole_square(position: [f32; 3], bit: usize) -> ([f32; 2], [f32; 2]) {
    cell_square(position, (bit / 4) * 2, (bit % 4) * 2, 2)
}

/// Every vertex one map chunk contributes to a tile mesh, in the order
/// [`Adt::to_mesh`] emits them: five per cell that is not a hole, cells
/// row-major.
///
/// This is the single definition of that order. It is a free function over the
/// inputs it needs rather than a method so that a caller holding a chunk in
/// some other form can produce the same vertices. Code that patches the
/// heights of a mesh already on the GPU needs this: each new position has to
/// go to the index the original went to, which requires both sides to emit in
/// the same order.
///
/// `heights`, `normals` and `colours` are [`HEIGHTS_PER_CHUNK`] long and in
/// `MCVT` order, except that `colours` may also be empty, because most chunks
/// have no `MCCV`.
///
/// A vertex's x and y are taken from the map's vertex lattice
/// ([`lattice_xy`]), not computed from the chunk's own origin. The origins in
/// the files are rounded: on `Azeroth_32_48` every chunk's far edge, computed
/// from its origin, lies 0.65 mm short of where its neighbour's origin puts
/// the same vertices. Two meshes whose shared vertices differ by any amount
/// leave hairline gaps the background shows through, which flicker as the
/// camera moves. On the lattice a vertex shared by two chunks, or by two
/// tiles, is computed from the same integers and is the same `f32` in both.
/// The heights a caller passes should be [`ChunkGrid::stitched`]'s for the
/// same reason.
pub fn chunk_mesh_vertices(
    position: [f32; 3],
    heights: &[f32],
    normals: &[[f32; 3]],
    colours: &[[f32; 4]],
    holes: u16,
    mut emit: impl FnMut(MeshVertex),
) {
    let outer = |row: usize, col: usize| row * (OUTER_SIDE + INNER_SIDE) + col;
    let inner = |row: usize, col: usize| row * (OUTER_SIDE + INNER_SIDE) + OUTER_SIDE + col;
    let base = lattice_index(position);
    for row in 0..INNER_SIDE {
        for col in 0..INNER_SIDE {
            if holes & (1 << ((row / 2) * 4 + col / 2)) != 0 {
                continue;
            }
            // Four cell corners from the outer grid, then the centre vertex from
            // the inner grid. `r` and `c` are in units of the 8x8 cell grid, so
            // dividing by 8 gives the chunk-local 0..1 coordinate the alpha map
            // is addressed by.
            let corners = [
                (row as f32, col as f32),
                (row as f32, col as f32 + 1.0),
                (row as f32 + 1.0, col as f32 + 1.0),
                (row as f32 + 1.0, col as f32),
                (row as f32 + 0.5, col as f32 + 0.5),
            ];
            for (i, (r, c)) in corners.into_iter().enumerate() {
                let at = if i == 4 {
                    inner(row, col)
                } else {
                    outer(r as usize, c as usize)
                };
                let [x, y] = lattice_xy(base, r, c);
                emit(MeshVertex {
                    corner: i,
                    position: [x, y, heights.get(at).copied().unwrap_or(0.0)],
                    normal: normals.get(at).copied().unwrap_or(UP),
                    uv: [c / INNER_SIDE as f32, r / INNER_SIDE as f32],
                    colour: colours.get(at).copied().unwrap_or(COLOUR_NONE),
                });
            }
        }
    }
}

/// The map's vertex lattice: the corner of the 64x64 tile grid, and the
/// spacing of a chunk's outer vertices, in `f64` so that a vertex index maps to
/// the same `f32` whichever chunk computes it. A tile is 1600/3 yards.
const LATTICE_ORIGIN: f64 = 32.0 * 1600.0 / 3.0;
const LATTICE_UNIT: f64 = 1600.0 / 3.0 / 128.0;

/// A chunk origin's place on the vertex lattice, as whole vertex steps from
/// the map's corner along x and y. The file's origins are within a millimetre
/// of a lattice point; rounding takes them onto it.
pub(crate) fn lattice_index(position: [f32; 3]) -> [f64; 2] {
    [
        ((LATTICE_ORIGIN - f64::from(position[0])) / LATTICE_UNIT).round(),
        ((LATTICE_ORIGIN - f64::from(position[1])) / LATTICE_UNIT).round(),
    ]
}

/// The world x and y of the vertex `r` rows and `c` columns (in cell units,
/// halves for a cell's centre) from a chunk whose origin is at lattice
/// `base`.
pub(crate) fn lattice_xy(base: [f64; 2], r: f32, c: f32) -> [f32; 2] {
    [
        (LATTICE_ORIGIN - (base[0] + f64::from(r)) * LATTICE_UNIT) as f32,
        (LATTICE_ORIGIN - (base[1] + f64::from(c)) * LATTICE_UNIT) as f32,
    ]
}

/// A tile's chunks by their place on the map, for finding a chunk's
/// neighbours.
///
/// A vertex on a chunk's edge is stored twice, once in each chunk that shares
/// it (four times at a corner), and the copies need not decode to the same
/// height: `MCVT` stores heights relative to each chunk's own base, so the
/// same world height rounds differently in two chunks. On the `Nephraites`
/// tiles written by this crate's writer, a third of shared edge vertices
/// differ, by up to 6 mm. [`Self::stitched`] gives every copy the same value.
pub struct ChunkGrid {
    cells: std::collections::HashMap<(i64, i64), usize>,
}

impl ChunkGrid {
    /// From the chunks' origins, in the tile's order; a chunk is named by its
    /// index in that order.
    pub fn new(origins: impl IntoIterator<Item = [f32; 3]>) -> ChunkGrid {
        let cells = origins
            .into_iter()
            .enumerate()
            .map(|(index, origin)| (Self::cell(origin), index))
            .collect();
        ChunkGrid { cells }
    }

    fn cell(origin: [f32; 3]) -> (i64, i64) {
        let [x, y] = lattice_index(origin);
        ((x / 8.0).round() as i64, (y / 8.0).round() as i64)
    }

    /// The chunk `rows` and `cols` chunks on from the one at `origin`, toward
    /// decreasing x and y, if the tile has it.
    pub fn neighbour(&self, origin: [f32; 3], rows: i64, cols: i64) -> Option<usize> {
        let (x, y) = Self::cell(origin);
        self.cells.get(&(x + rows, y + cols)).copied()
    }

    /// A chunk's heights with every outer vertex on its edges replaced by the
    /// same vertex's height in the chunk that owns it.
    ///
    /// The owner is chosen among the chunks of this tile that hold the vertex
    /// (two along an edge, four at a corner) as the one furthest on: greatest
    /// row step, then greatest column step. That is a property of the vertex
    /// and of which chunks the tile has, so every chunk holding the vertex
    /// picks the same owner and reads the same value. A vertex on the tile's
    /// own outer edge is held by fewer chunks here than on the map; the next
    /// tile's copy may still differ slightly, and [`Adt::to_mesh`] covers that
    /// seam with a skirt.
    ///
    /// `heights_of` answers a chunk's decoded heights by index.
    pub fn stitched(
        &self,
        origin: [f32; 3],
        heights: &[f32],
        heights_of: impl Fn(usize) -> Option<Vec<f32>>,
    ) -> Vec<f32> {
        let mut out = heights.to_vec();
        let side = OUTER_SIDE + INNER_SIDE;
        let last = INNER_SIDE as i64;
        let mut cache: std::collections::HashMap<(i64, i64), Option<Vec<f32>>> =
            std::collections::HashMap::new();
        for r in 0..=last {
            for c in 0..=last {
                if r != 0 && r != last && c != 0 && c != last {
                    continue;
                }
                // The row steps and column steps of the chunks holding (r, c).
                let rows: &[i64] = if r == 0 { &[-1, 0] } else if r == last { &[0, 1] } else { &[0] };
                let cols: &[i64] = if c == 0 { &[-1, 0] } else if c == last { &[0, 1] } else { &[0] };
                let mut owner = (0, 0);
                for &dr in rows {
                    for &dc in cols {
                        if (dr, dc) > owner && self.neighbour(origin, dr, dc).is_some() {
                            owner = (dr, dc);
                        }
                    }
                }
                if owner == (0, 0) {
                    continue;
                }
                let theirs = cache
                    .entry(owner)
                    .or_insert_with(|| self.neighbour(origin, owner.0, owner.1).and_then(&heights_of));
                let Some(theirs) = theirs else {
                    continue;
                };
                let (tr, tc) = (r - owner.0 * last, c - owner.1 * last);
                let at = |row: i64, col: i64| (row as usize) * side + col as usize;
                if let (Some(slot), Some(&value)) = (out.get_mut(at(r, c)), theirs.get(at(tr, tc))) {
                    *slot = value;
                }
            }
        }
        out
    }
}

/// How far below a tile's outer edge its skirt reaches, in yards. Enough to
/// cover the sub-centimetre height differences between two tiles' copies of
/// their shared edge, which [`ChunkGrid::stitched`] cannot reach because the
/// other tile is not in hand.
pub(crate) const SKIRT_DEPTH: f32 = 0.25;

/// The `MCCV` byte that leaves a vertex's shading unchanged.
///
/// 0x7F and not 0xFF, because the value is a multiplier and the scale puts 1.0
/// at 127: a fully white vertex is 0xFF, which is twice the light rather than
/// all of it. A chunk filled with this byte draws exactly as a chunk with no
/// `MCCV` at all, and [`decode_colours`] depends on that.
pub const COLOUR_NEUTRAL: u8 = 0x7F;

/// The divisor that turns an `MCCV` byte into a multiplier: [`COLOUR_NEUTRAL`]
/// becomes 1.0.
pub(crate) const COLOUR_SCALE: f32 = COLOUR_NEUTRAL as f32;

/// `MCCV`'s four bytes per vertex, as the multipliers the ground is shaded by.
///
/// ## Layout
///
/// One `CImVector` per vertex, 145 of them, in the same interleaved order
/// `MCVT` and `MCNR` use, so vertex *n* of the heights is vertex *n* of the
/// colours. `CImVector` is blue, green, red, alpha in that byte order. A
/// red/blue swap is easy to make and does not show on grey vertices.
///
/// ## The bytes are multipliers
///
/// 0x7F is 1.0 and 0xFF is 2.0 (see [`COLOUR_NEUTRAL`]). The range runs from
/// black through unchanged at the midpoint to twice as bright. A tool that
/// treated the byte as an ordinary 0..255 colour would halve the brightness of
/// every vertex it wrote with an unpainted value.
///
/// ## Shipped 1.12 tiles have no `MCCV`
///
/// No shipped 1.12 tile carries this chunk (measured: 0 occurrences across
/// `Azeroth_32_48`'s 256 chunks), and the three header dwords that hold its
/// offset are zero on every one of them. This project's client reads it
/// because it draws the ground itself, and `MCCV` gives per-vertex shading that
/// the 1.12.1 client cannot express. This is a deliberate deviation from
/// 1.12.1.
///
/// A vertex the region is too short for answers neutral, which is the same
/// answer a chunk with no region at all gives.
pub fn decode_colours(mccv: &[u8]) -> Vec<[f32; 4]> {
    (0..HEIGHTS_PER_CHUNK)
        .map(|i| {
            let at = |k: usize| {
                mccv.get(i * 4 + k)
                    .map(|&b| f32::from(b) / COLOUR_SCALE)
                    .unwrap_or(1.0)
            };
            // Blue, green, red, alpha in the file; red, green, blue, alpha out.
            [at(2), at(1), at(0), at(3)]
        })
        .collect()
}

/// The multiplier for a vertex with no `MCCV`: 1.0 in every channel, which
/// leaves its shading unchanged.
pub(crate) const COLOUR_NONE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// `MCNR`'s three signed bytes per vertex, as unit normals in the world's own
/// axes.
///
/// The single decoder for the encoding, called by the parser and by anything
/// that has to produce the same numbers from an edited chunk. A vertex the
/// region is too short for, and one whose triple is degenerate, answers
/// straight up.
pub fn decode_normals(mcnr: &[u8]) -> Vec<[f32; 3]> {
    (0..HEIGHTS_PER_CHUNK)
        .map(|i| {
            let at = |k: usize| {
                mcnr.get(i * 3 + k)
                    .map(|&b| b as i8 as f32 / 127.0)
                    .unwrap_or(0.0)
            };
            let n = [at(0), at(1), at(2)];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len > 0.1 {
                [n[0] / len, n[1] / len, n[2] / len]
            } else {
                UP
            }
        })
        .collect()
}

/// The *specular* name of a ground texture: `_s` inserted before the
/// extension, so `Tileset\Duskwood\DuskwoodRock.blp` becomes
/// `…\DuskwoodRock_s.blp`.
///
/// This is the 1.12.1 client's rule. When specular terrain is on, the client
/// inserts `_s` before the extension (after the last `.`) of a ground texture
/// path and loads the result. When it is off, the client loads the name as
/// given and nothing else.
///
/// Specular terrain is on only when the `specular` CVar and pixel shaders are
/// both on and `terrainp_s.bls` is loaded. So the `_s` texture and the `_s`
/// fragment program are used together or not at all.
///
/// The two files differ in one respect. `DuskwoodCobblestone.blp` is 256x256
/// DXT with `alphaDepth` 0 and 44,876 bytes; `_s` is the same size, DXT3,
/// `alphaDepth` 8, 88,580 bytes. The extra plane is the *gloss mask* that
/// controls the specular sheen; see `terrain.wgsl`.
///
/// A path with no extension gets the suffix at the end, as the client does
/// with such a path.
pub fn specular_texture(path: &str) -> String {
    match path.rfind('.') {
        Some(dot) => format!("{}_s{}", &path[..dot], &path[dot..]),
        None => format!("{path}_s"),
    }
}

#[derive(Debug, Clone)]
pub struct Adt {
    pub version: u32,
    pub texture_names: Vec<String>,
    pub model_names: Vec<String>,
    pub wmo_names: Vec<String>,
    pub doodads: Vec<DoodadPlacement>,
    pub wmos: Vec<WmoPlacement>,
    /// Up to 256 map chunks, in file order (row-major, y then x).
    pub chunks: Vec<Mcnk>,
}

impl Adt {
    pub fn parse(buf: &[u8]) -> Result<Adt, AssetError> {
        let mut adt = Adt {
            version: 0,
            texture_names: Vec::new(),
            model_names: Vec::new(),
            wmo_names: Vec::new(),
            doodads: Vec::new(),
            wmos: Vec::new(),
            chunks: Vec::new(),
        };

        for c in ChunkReader::new(buf) {
            match &c.magic {
                b"MVER" => adt.version = chunk::u32_at(c.data, 0),
                b"MTEX" => adt.texture_names = chunk::split_strings(c.data),
                b"MMDX" => adt.model_names = chunk::split_strings(c.data),
                b"MWMO" => adt.wmo_names = chunk::split_strings(c.data),
                b"MDDF" => adt.doodads = parse_doodads(c.data),
                b"MODF" => adt.wmos = parse_wmos(c.data),
                b"MCNK" => {
                    if let Some(mcnk) = parse_mcnk(c.data) {
                        adt.chunks.push(mcnk);
                    }
                }
                _ => {}
            }
        }

        if adt.chunks.is_empty() {
            return Err(AssetError::malformed("ADT", "no MCNK chunks"));
        }
        Ok(adt)
    }

    /// Where a chunk's vertices begin in the buffer [`Adt::to_mesh`] builds, and
    /// how many there are.
    ///
    /// The chunks are emitted in order and each contributes five vertices per
    /// cell it draws, so a chunk's vertices are one contiguous run. That is what
    /// lets a caller rewrite one chunk's heights in a mesh already on the GPU
    /// without rebuilding the tile.
    ///
    /// The hole mask decides the count and a height edit does not change it, so
    /// a span taken before an edit is still the span after it.
    pub fn chunk_vertex_span(&self, chunk: usize) -> (usize, usize) {
        let start: usize = self
            .chunks
            .iter()
            .take(chunk)
            .map(|c| c.drawn_cells() * 5)
            .sum();
        let count = self.chunks.get(chunk).map_or(0, |c| c.drawn_cells() * 5);
        (start, count)
    }

    /// Lowest and highest terrain height across the tile.
    pub fn height_range(&self) -> (f32, f32) {
        self.chunks
            .iter()
            .map(Mcnk::min_max_height)
            .fold((f32::MAX, f32::MIN), |(lo, hi), (a, b)| (lo.min(a), hi.max(b)))
    }

    /// The map chunk containing a world position, if this tile has one.
    ///
    /// Found by scanning rather than by arithmetic on the tile's index: an
    /// `Adt` does not know which tile it is, but every chunk carries its own
    /// absolute origin. 256 comparisons cost little next to parsing the file.
    pub fn chunk_at(&self, x: f32, y: f32) -> Option<&Mcnk> {
        self.chunks.iter().find(|c| {
            let dx = c.position[0] - x;
            let dy = c.position[1] - y;
            (0.0..CHUNK_SIZE).contains(&dx) && (0.0..CHUNK_SIZE).contains(&dy)
        })
    }

    /// Terrain height at a world position, or `None` outside the tile and over
    /// holes.
    ///
    /// This has to agree with [`Adt::to_mesh`], and therefore with the server:
    /// a cell is four triangles meeting at the inner-grid vertex in its
    /// centre, not two triangles across a quad. Interpolating across two
    /// triangles gives a different answer on any cell that is not planar,
    /// which on real terrain is most of them, and the difference shows as a
    /// character walking partly buried in, or floating above, the ground.
    pub fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        let w = self.wedge_at(x, y)?;
        Some(barycentric(w.at, w.corners[0], w.corners[1], w.corners[2]))
    }

    /// The slope of the triangle [`Adt::height_at`] samples, as a unit normal
    /// in the world's axes (`+X` north, `+Y` west, `+Z` up).
    ///
    /// This is the geometric normal of the wedge [`Adt::height_at`]
    /// interpolated over, not `MCNR`. `MCNR` holds shading normals, smoothed
    /// across cell boundaries and authored for lighting. Placing a model on the
    /// ground needs the plane the height came from. Reading `MCNR` instead
    /// would tilt a model away from the ground under it by however much the
    /// artist smoothed.
    ///
    /// The normal always points up: a cell's four triangles have a consistent
    /// winding in [`Adt::to_mesh`], but a model placed on one does not depend
    /// on the winding, and a normal that pointed into the ground would turn
    /// the model upside down.
    pub fn normal_at(&self, x: f32, y: f32) -> Option<[f32; 3]> {
        Some(wedge_normal(&self.wedge_at(x, y)?))
    }

    /// The wedge a world position falls in; see [`Wedge`].
    ///
    /// Shared by [`Adt::height_at`] and [`Adt::normal_at`] so that the height a
    /// character stands at and the slope it stands on always come from the
    /// same triangle.
    fn wedge_at(&self, x: f32, y: f32) -> Option<Wedge> {
        let chunk = self.chunk_at(x, y)?;
        wedge_in(chunk.position, &chunk.heights, chunk.holes, x, y)
    }

    /// The liquid surface over a world position. [`Mcnk::liquid_at`] holds the
    /// rule; this function only finds the chunk.
    ///
    /// It does not apply the hole test [`Self::height_at`] makes. A hole is a
    /// gap in the drawn ground, and the water above it is drawn regardless:
    /// Ironforge's forge and every cave mouth in the game have this shape, so
    /// refusing a liquid over a hole would leave a swimmer with neither a
    /// floor nor a surface.
    pub fn liquid_at(&self, x: f32, y: f32) -> Option<(Liquid, f32)> {
        self.chunk_at(x, y)?.liquid_at(x, y)
    }

    /// Is the ground at a world position inside this tile's baked `MCSH`
    /// shadow?
    ///
    /// This is an input to a model's lighting, not the terrain's: the 1.12
    /// client scales each placed M2's sun term by a per-instance intensity
    /// chosen from the shadow bit under its origin (`Model2.bls` ends in a
    /// per-instance `MAD`), so a tree standing in a building's baked shadow is
    /// dimmed with the ground it stands on instead of taking full sun.
    ///
    /// The texel derivation mirrors [`Self::to_mesh`]'s UVs, which is the
    /// mapping `vale textures` round-trips against the atlas: `u` runs along
    /// decreasing world y and `v` along decreasing world x, and the shadow map
    /// is row-major with `u` minor. Outside the tile, or over a chunk with no
    /// `MCSH`, the answer is "not shadowed", because the lit scale is the
    /// common case.
    pub fn shadowed_at(&self, x: f32, y: f32) -> bool {
        let Some(chunk) = self.chunk_at(x, y) else {
            return false;
        };
        let texel = |d: f32| (((d / CHUNK_SIZE) * ALPHA_SIDE as f32) as usize).min(ALPHA_SIDE - 1);
        let row = texel(chunk.position[0] - x);
        let col = texel(chunk.position[1] - y);
        chunk.shadow.get(row * ALPHA_SIDE + col).copied().unwrap_or(0) != 0
    }

    /// The `GroundEffectTexture` id of the texture layer dominant at a world
    /// position: `MCLY`'s `effectId`, which alone decides the footstep sound
    /// ([`crate::tables::sound`]) and which grass tufts grow.
    ///
    /// The dominant layer is the topmost layer whose alpha at the texel reaches
    /// half, else the base layer; the texel derivation is
    /// [`Self::shadowed_at`]'s, because the alpha maps and the shadow map share
    /// `MCAL`'s layout. `None` outside the tile or on a chunk with no layers.
    /// An `effectId` of 0 is a valid result, a texture with no ground effect
    /// row, and the caller decides what it means.
    pub fn ground_effect_at(&self, x: f32, y: f32) -> Option<u32> {
        let chunk = self.chunk_at(x, y)?;
        let texel = |d: f32| (((d / CHUNK_SIZE) * ALPHA_SIDE as f32) as usize).min(ALPHA_SIDE - 1);
        let row = texel(chunk.position[0] - x);
        let col = texel(chunk.position[1] - y);
        for i in (1..chunk.layers.len()).rev() {
            let alpha = chunk
                .alphas
                .get(i)
                .and_then(|a| a.get(row * ALPHA_SIDE + col))
                .copied()
                .unwrap_or(0);
            if alpha >= 128 {
                return Some(chunk.layers[i].effect_id);
            }
        }
        chunk.layers.first().map(|l| l.effect_id)
    }

    /// Every `MCLQ` surface on the tile, merged into one draw per liquid type.
    ///
    /// The mesh is built exactly as [`crate::world::wmo::WmoModel`]'s
    /// `add_liquid` builds it (one quad per wet tile, the vertex colour black
    /// with `MLIQ`/`MCLQ`'s depth byte as the alpha, one texture repeat per
    /// tile), because it feeds the same material. Merging by type keeps a lake
    /// from being 256 draws: a tile has at most four liquids, so it gets at
    /// most four draws.
    ///
    /// Positions are world-space in the file's axes, like [`Self::to_mesh`]'s,
    /// and the winding faces +Z for the same reason a WMO pool's does: the
    /// file-to-grid re-order in `parse_mclq_block` is a 180° turn, which
    /// preserves the winding.
    pub fn liquid_surface(&self) -> Vec<TerrainLiquidDraw> {
        let mut draws: Vec<TerrainLiquidDraw> = Vec::new();
        for chunk in &self.chunks {
            for (kind, grid) in &chunk.liquids {
                let draw = match draws.iter_mut().position(|d| d.kind == *kind) {
                    Some(i) => &mut draws[i],
                    None => {
                        draws.push(TerrainLiquidDraw {
                            kind: *kind,
                            positions: Vec::new(),
                            uvs: Vec::new(),
                            colours: Vec::new(),
                            indices: Vec::new(),
                        });
                        draws.last_mut().expect("just pushed")
                    }
                };
                for ty in 0..grid.y_tiles {
                    for tx in 0..grid.x_tiles {
                        if !grid.has_liquid(tx, ty) {
                            continue;
                        }
                        let base = draw.positions.len() as u32;
                        for ((x, y), uv) in [
                            ((tx, ty), [0.0, 0.0]),
                            ((tx + 1, ty), [1.0, 0.0]),
                            ((tx + 1, ty + 1), [1.0, 1.0]),
                            ((tx, ty + 1), [0.0, 1.0]),
                        ] {
                            draw.positions.push(grid.vertex(x, y));
                            draw.uvs.push(uv);
                            let index = y * (grid.x_tiles + 1) + x;
                            draw.colours.push([0, 0, 0, grid.opacity(index, *kind)]);
                        }
                        draw.indices
                            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                    }
                }
            }
        }
        draws.retain(|d| !d.indices.is_empty());
        draws
    }

    /// Build a renderable triangle mesh for the whole tile.
    ///
    /// WoW terrain is not a plain grid: each of the 8x8 cells in a chunk is
    /// drawn as four triangles meeting at the inner-grid vertex in the cell's
    /// centre. A two-triangle quad visibly differs on steep ground and does not
    /// match server-side height lookups.
    ///
    /// Positions use the file's own axes (+X north, +Y west, +Z up); the
    /// renderer is responsible for any handedness conversion.
    pub fn to_mesh(&self) -> TerrainMesh {
        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut normals: Vec<[f32; 3]> = Vec::new();
        let mut colours: Vec<[f32; 4]> = Vec::new();
        let mut uvs: Vec<[f32; 2]> = Vec::new();
        let mut alpha_uvs: Vec<[f32; 2]> = Vec::new();
        // Per chunk, so that chunks sharing a texture set can have their index
        // ranges concatenated below. A chunk's own indices have to be built
        // together regardless, because they reference vertices it owns.
        let mut per_chunk: Vec<(Vec<u32>, TerrainDraw)> = Vec::new();
        // Where each chunk's vertices start in the buffer, for the skirts.
        let mut starts: Vec<u32> = Vec::with_capacity(self.chunks.len());
        let grid = ChunkGrid::new(self.chunks.iter().map(|c| c.position));

        for (chunk_index, chunk) in self.chunks.iter().enumerate() {
            let mut indices: Vec<u32> = Vec::new();
            let mut base = positions.len() as u32;
            starts.push(base);

            // The emission order is [`chunk_mesh_vertices`]' and not this
            // function's, so that anything patching a chunk's heights can put
            // each new position at the index the original went to. The
            // heights are stitched, so a vertex two chunks share is one value.
            let heights = grid.stitched(chunk.position, &chunk.heights, |i| {
                self.chunks.get(i).map(|c| c.heights.clone())
            });
            chunk_mesh_vertices(
                chunk.position,
                &heights,
                &chunk.normals,
                &chunk.colours,
                chunk.holes,
                |vertex| {
                positions.push(vertex.position);
                normals.push(vertex.normal);
                colours.push(vertex.colour);
                uvs.push(vertex.uv);
                alpha_uvs.push(atlas_uv(chunk_index, vertex.uv[0], vertex.uv[1]));
                // A cell's four triangles meet at its centre vertex, which is
                // the last of the five, so the cell is complete when it
                // arrives.
                if vertex.corner == 4 {
                    let centre = base + 4;
                    for i in 0..4u32 {
                        indices.push(centre);
                        indices.push(base + i);
                        indices.push(base + (i + 1) % 4);
                    }
                    base += 5;
                }
            },
            );

            // The chunk's footprint is known without looking at the mesh (one
            // CHUNK_SIZE square from its origin, running in decreasing x and
            // y), so only the height range has to be measured.
            let (lo, hi) = chunk.min_max_height();
            let half = CHUNK_SIZE / 2.0;
            let centre = [
                chunk.position[0] - half,
                chunk.position[1] - half,
                (lo + hi) / 2.0,
            ];
            let radius = (half * half * 2.0 + ((hi - lo) / 2.0).powi(2)).sqrt();

            per_chunk.push((
                indices,
                TerrainDraw {
                    index_start: 0, // assigned when the groups are laid out
                    index_count: 0,
                    textures: chunk.layers.iter().map(|l| l.texture_id).collect(),
                    scrolls: chunk
                        .layers
                        .iter()
                        .map(|l| layer_flags::scroll(l.flags).unwrap_or((0.0, 0.0)))
                        .map(|(u, v)| [u, v])
                        .collect(),
                    centre,
                    radius,
                    chunks: 1,
                },
            ));
        }

        // The skirts: along each side of a chunk that has no neighbour in this
        // tile, a strip hanging `SKIRT_DEPTH` below the edge, drawn from both
        // sides, in the chunk's own draw group. It covers the gap to the next
        // tile, whose copy of the edge can differ in height by a millimetre
        // or so. Each skirt vertex copies an edge vertex, and the bottom pair
        // drops by the depth. They go after every chunk's vertices so the
        // per-chunk spans stay as `chunk_vertex_span` states them.
        let mut skirt_sources: Vec<u32> = Vec::new();
        for (chunk_index, chunk) in self.chunks.iter().enumerate() {
            let cell_of = cell_ordinals(chunk.holes);
            // (neighbour row step, column step, cell for segment k, its two corners)
            let sides: [(i64, i64, fn(usize) -> (usize, usize), usize, usize); 4] = [
                (-1, 0, |k| (0, k), 0, 1),
                (1, 0, |k| (INNER_SIDE - 1, k), 3, 2),
                (0, -1, |k| (k, 0), 0, 3),
                (0, 1, |k| (k, INNER_SIDE - 1), 1, 2),
            ];
            for (rows, cols, cell, from, to) in sides {
                if grid.neighbour(chunk.position, rows, cols).is_some() {
                    continue;
                }
                for k in 0..INNER_SIDE {
                    let (row, col) = cell(k);
                    let Some(ordinal) = cell_of[row][col] else {
                        continue;
                    };
                    let first = starts[chunk_index] + ordinal * 5;
                    let (a, b) = (first + from as u32, first + to as u32);
                    let at = positions.len() as u32;
                    for (source, drop) in [(a, 0.0), (b, 0.0), (a, SKIRT_DEPTH), (b, SKIRT_DEPTH)] {
                        let i = source as usize;
                        let [x, y, z] = positions[i];
                        positions.push([x, y, z - drop]);
                        normals.push(normals[i]);
                        colours.push(colours[i]);
                        uvs.push(uvs[i]);
                        alpha_uvs.push(alpha_uvs[i]);
                        skirt_sources.push(source);
                    }
                    let (ta, tb, ba, bb) = (at, at + 1, at + 2, at + 3);
                    per_chunk[chunk_index]
                        .0
                        .extend([ta, ba, tb, tb, ba, bb, ta, tb, ba, tb, bb, ba]);
                }
            }
        }

        // Concatenate the chunks that want identical GL state.
        //
        // A chunk's draw state is its up-to-four ground textures and its alpha
        // map. With the alpha map as a texture per chunk, every chunk was its
        // own draw, ~2300 for nine tiles. The alpha map is one cell of a
        // tile-wide atlas addressed per vertex, so the only state left is the
        // texture set, and a tile uses a handful of sets, not 256. Grouping by
        // texture set cuts draws about ten-fold and changes no pixel.
        //
        // A group's draw is one index range only if the group's indices are
        // contiguous, so the indices are laid out here in group order rather
        // than in chunk order.
        let mut indices: Vec<u32> = Vec::with_capacity(per_chunk.iter().map(|(i, _)| i.len()).sum());
        let mut draws: Vec<TerrainDraw> = Vec::new();
        let mut placed = vec![false; per_chunk.len()];
        for i in 0..per_chunk.len() {
            if placed[i] || per_chunk[i].0.is_empty() {
                // An empty chunk is all holes: no state, no draw.
                placed[i] = true;
                continue;
            }
            let index_start = indices.len() as u32;
            let mut group = per_chunk[i].1.clone();
            group.index_start = index_start;
            for j in i..per_chunk.len() {
                if placed[j]
                    || per_chunk[j].1.textures != group.textures
                    || per_chunk[j].1.scrolls != group.scrolls
                {
                    continue;
                }
                indices.extend_from_slice(&per_chunk[j].0);
                if j != i {
                    group = merge_bounds(&group, &per_chunk[j].1);
                    group.index_start = index_start;
                }
                placed[j] = true;
            }
            group.index_count = indices.len() as u32 - index_start;
            draws.push(group);
        }

        let (lo, hi) = self.height_range();
        let bounds = self.chunks.iter().fold(
            ([f32::MAX; 2], [f32::MIN; 2]),
            |(min, max), c| {
                (
                    [min[0].min(c.position[0] - CHUNK_SIZE), min[1].min(c.position[1] - CHUNK_SIZE)],
                    [max[0].max(c.position[0]), max[1].max(c.position[1])],
                )
            },
        );
        let centre = [
            (bounds.0[0] + bounds.1[0]) / 2.0,
            (bounds.0[1] + bounds.1[1]) / 2.0,
            (lo + hi) / 2.0,
        ];
        let radius = {
            let dx = (bounds.1[0] - bounds.0[0]) / 2.0;
            let dy = (bounds.1[1] - bounds.0[1]) / 2.0;
            let dz = (hi - lo) / 2.0;
            (dx * dx + dy * dy + dz * dz).sqrt()
        };

        TerrainMesh {
            positions,
            normals,
            colours,
            uvs,
            alpha_uvs,
            indices,
            draws,
            centre,
            radius,
            skirt_sources,
        }
    }

    /// Every `MDDF` doodad, resolved to an archive path and a world matrix.
    ///
    /// A placement whose `name_id` is not in `MMDX` is dropped: it cannot be
    /// drawn, and dropping it here spares the renderer from handling gaps in
    /// the list.
    pub fn placed_doodads(&self) -> Vec<PlacedModel> {
        self.doodads
            .iter()
            .filter_map(|d| {
                let name = self.model_names.get(d.name_id as usize)?;
                let position = placement_to_world(d.position);
                Some(PlacedModel {
                    path: crate::world::m2::model_path(name),
                    unique_id: d.unique_id,
                    position,
                    matrix: placement_matrix(position, d.rotation, d.scale),
                    scale: d.scale,
                    doodad_set: 0,
                    name_set: 0,
                })
            })
            .collect()
    }

    /// Every `MODF` WMO placement, resolved the same way.
    ///
    /// WMO placements carry a scale field that 1.12 does not use (it arrives in
    /// Legion), so the scale is fixed at 1.
    pub fn placed_wmos(&self) -> Vec<PlacedModel> {
        self.wmos
            .iter()
            .filter_map(|w| {
                let name = self.wmo_names.get(w.name_id as usize)?;
                let position = placement_to_world(w.position);
                Some(PlacedModel {
                    path: name.clone(),
                    unique_id: w.unique_id,
                    position,
                    matrix: placement_matrix(position, w.rotation, 1.0),
                    scale: 1.0,
                    doodad_set: w.doodad_set,
                    name_set: w.name_set,
                })
            })
            .collect()
    }

    /// Whether a point is inside the `MODF` box of a building whose collision
    /// geometry the caller has not loaded yet.
    ///
    /// `MODF` states each placement's world-space box, and the box can be
    /// tested before the `.wmo` has been read, which is when this question
    /// arises. The terrain under a city is not the city's floor: Stormwind's
    /// ground is far below its streets, so a mover that uses the terrain
    /// because the building has not loaded falls through the world.
    ///
    /// The 1.12 client waits in this case: when the point is inside a map
    /// object's box, the client waits for that object to load before it
    /// queries the geometry, and never answers from the terrain instead. This
    /// is why the function answers "not yet" rather than falling back to the
    /// ground. This project's client cannot block its frame, so it reports no
    /// data for the point and the mover holds its altitude, which is
    /// [`crate::world::adt::Adt::height_at`]'s absent case and the same
    /// behaviour a loading screen would produce.
    ///
    /// `pending` is called only for placements whose box contains the point,
    /// about a dozen boxes on a tile, because each call costs the caller a
    /// lock.
    pub fn awaiting_building(&self, point: [f32; 3], pending: impl Fn(u32) -> bool) -> bool {
        self.wmos.iter().any(|w| {
            crate::world::wmo::box_contains(&modf_world_box(w), point) && pending(w.unique_id)
        })
    }

    /// Whether a point is inside any building's `MODF` box, loaded or not.
    ///
    /// [`Self::awaiting_building`] tests only the placements that have not
    /// loaded; this tests all of them, and the two callers use the answer for
    /// different decisions. The first decides whether to wait; this decides
    /// whether the terrain may be stood on when it is above the character's
    /// head.
    ///
    /// Ironforge needs the second rule. The terrain over the city is the
    /// mountain, hundreds of yards up, and a building's hull has gaps (a
    /// doorway, a stair the file states no `MOPY` for), so the mover can ask
    /// for a floor at a point inside the city where no geometry answers. Taking
    /// the terrain there snaps the character to the top of the mountain. Inside
    /// a `MODF` box this client reports no floor for the point, which the mover
    /// reads as "hold the altitude the server last sent".
    pub fn inside_building(&self, point: [f32; 3]) -> bool {
        self.awaiting_building(point, |_| true)
    }

    /// Every chunk's alpha maps in one tile-wide 1024x1024 RGBA atlas: a 16x16
    /// grid of 64x64 cells, chunk `i` of `self.chunks` at cell
    /// `(i % 16, i / 16)`.
    ///
    /// Layers 1, 2 and 3 go in R, G and B; the base layer needs no channel
    /// because every other layer blends over it.
    ///
    /// Alpha holds the chunk's `MCSH` shadow. It is stored here rather than in
    /// a texture of its own because it is the same 64x64 grid, over the same
    /// chunk, sampled at the same texel centres, so it uses the same cell, the
    /// same [`atlas_uv`] and the same mip chain. A second 1024x1024 texture
    /// would add a binding and a sampler to every terrain material for a lookup
    /// at the identical coordinate. A chunk with no shadow leaves alpha at 0,
    /// which means nothing is shadowed.
    ///
    /// The atlas lets chunks share a draw call. With one alpha texture per
    /// chunk the alpha map was part of the draw state, so no two chunks could
    /// be drawn together and nine tiles cost ~2300 calls. With an atlas the
    /// lookup is a per-vertex coordinate ([`atlas_uv`]), so the texture set is
    /// the only state a chunk carries, and a tile uses a handful of sets.
    ///
    /// The cell assignment follows the position in `self.chunks` rather than
    /// the chunk's own `index_x`/`index_y`, so that a tile with a damaged
    /// `MCNK` still agrees with [`atlas_uv`]; the two stay consistent because
    /// both are derived from the same index.
    ///
    /// Bilinear filtering would pull a neighbouring chunk's blend into an edge
    /// texel. The half-texel inset in [`atlas_uv`] prevents that, not this
    /// function.
    ///
    /// [`alpha_atlas_mips`] builds the atlas's mip chain; the cell layout above
    /// is what makes that chain safe to build.
    pub fn alpha_atlas(&self) -> Vec<u8> {
        let mut out = vec![0u8; ATLAS_SIDE * ATLAS_SIDE * 4];
        for (i, chunk) in self.chunks.iter().enumerate() {
            if i >= CHUNKS_PER_SIDE * CHUNKS_PER_SIDE {
                break;
            }
            let cell = alpha_atlas_cell(&chunk.alphas, &chunk.shadow);
            let (cell_x, cell_y) = (i % CHUNKS_PER_SIDE, i / CHUNKS_PER_SIDE);
            for texel in 0..ALPHA_LEN {
                let (tx, ty) = (texel % ALPHA_SIDE, texel / ALPHA_SIDE);
                let x = cell_x * ALPHA_SIDE + tx;
                let y = cell_y * ALPHA_SIDE + ty;
                out[(y * ATLAS_SIDE + x) * 4..][..4].copy_from_slice(&cell[texel * 4..][..4]);
            }
        }
        out
    }
}

/// The 64x64 RGBA one chunk contributes to the atlas: its layers 1, 2 and 3 in
/// R, G and B, and its `MCSH` in A.
///
/// The single definition of the packing, called by [`Adt::alpha_atlas`] when a
/// tile is built and by anything writing a chunk's blend into an atlas that is
/// already on the GPU. `alphas` must already have had [`fix_alpha_edge`]
/// applied: a parsed [`Mcnk`]'s has; a map decoded through
/// [`decode_alpha_layer`] has not, and the caller of that function applies it.
pub fn alpha_atlas_cell(alphas: &[Vec<u8>], shadow: &[u8]) -> Vec<u8> {
    let mut cell = vec![0u8; ALPHA_LEN * 4];
    for texel in 0..ALPHA_LEN {
        for channel in 0..3 {
            // `alphas[0]` is the opaque base layer and is skipped.
            cell[texel * 4 + channel] = alphas
                .get(channel + 1)
                .and_then(|a| a.get(texel))
                .copied()
                .unwrap_or(0);
        }
        cell[texel * 4 + 3] = shadow.get(texel).copied().unwrap_or(0);
    }
    cell
}

/// Where each level of an atlas plus its mip chain begins, in the buffer they
/// are concatenated into, and how wide that level is.
///
/// Level 0 first, then [`ALPHA_ATLAS_MIPS`] levels below it. That layout is the
/// one `Image::data` wants and the one [`write_alpha_atlas_cell`] indexes into.
pub(crate) fn alpha_atlas_levels() -> Vec<(usize, usize)> {
    let mut levels = Vec::with_capacity(1 + ALPHA_ATLAS_MIPS);
    let mut at = 0usize;
    for level in 0..=ALPHA_ATLAS_MIPS {
        let side = ATLAS_SIDE >> level;
        levels.push((at, side));
        at += side * side * 4;
    }
    levels
}

/// Write one chunk's cell into an atlas that is already built, at every level of
/// its mip chain.
///
/// This relies on the property that makes the mip chain safe, stated on
/// [`alpha_atlas_mips`]: a level-`n` texel covers an aligned `2^n` block of
/// level-0 texels, and a cell is 64 wide, so while the chain stops at level 6
/// no block spans two cells. A cell is therefore self-contained at every level
/// and can be minified on its own, so a host that has changed one chunk's
/// blend can write it into an atlas on the GPU rather than rebuild the tile.
///
/// `cell` is [`alpha_atlas_cell`]'s output. The filter is the box filter
/// [`alpha_atlas_mips`] uses, applied within the cell, for the reason that
/// function gives: every channel here is an independent map, and weighting one
/// by another would fade a chunk's blend out wherever its shadow does.
pub fn write_alpha_atlas_cell(data: &mut [u8], chunk_index: usize, cell: &[u8]) {
    let (cell_x, cell_y) = (
        chunk_index % CHUNKS_PER_SIDE,
        chunk_index / CHUNKS_PER_SIDE,
    );
    let mut level_cell = cell.to_vec();
    let mut cell_side = ALPHA_SIDE;
    for (level, &(offset, side)) in alpha_atlas_levels().iter().enumerate() {
        if level > 0 {
            level_cell = halve(&level_cell, cell_side);
            cell_side /= 2;
        }
        let (x0, y0) = (cell_x * cell_side, cell_y * cell_side);
        for row in 0..cell_side {
            let from = row * cell_side * 4;
            let to = offset + ((y0 + row) * side + x0) * 4;
            let Some(slice) = data.get_mut(to..to + cell_side * 4) else {
                return;
            };
            slice.copy_from_slice(&level_cell[from..from + cell_side * 4]);
        }
    }
}

/// A square RGBA block box-filtered to half its side.
fn halve(src: &[u8], side: usize) -> Vec<u8> {
    let half = side / 2;
    let mut out = vec![0u8; half * half * 4];
    for y in 0..half {
        for x in 0..half {
            for channel in 0..4 {
                let tap = |dx: usize, dy: usize| {
                    src[(((y * 2 + dy) * side) + x * 2 + dx) * 4 + channel] as u32
                };
                let sum = tap(0, 0) + tap(1, 0) + tap(0, 1) + tap(1, 1);
                out[(y * half + x) * 4 + channel] = ((sum + 2) / 4) as u8;
            }
        }
    }
    out
}

/// How many levels below the top the alpha atlas may be minified: as many as it
/// takes to reduce a 64x64 cell to a single texel, and no more.
///
/// Below that, a level averages whole chunks together rather than texels
/// within one, and the cell layout no longer holds.
pub(crate) const ALPHA_ATLAS_MIPS: usize = ALPHA_SIDE.trailing_zeros() as usize;

/// The alpha atlas minified, levels 1..=[`ALPHA_ATLAS_MIPS`], top excluded.
///
/// The blend maps must be minified along with the ground textures. A tile's
/// blend maps are 64 texels across a 33-yard chunk, so at most distances one
/// screen pixel covers many of them. Sampling only the top level aliases the
/// blend weights into a regular lattice that shows as a cross-hatch over the
/// whole landscape and changes phase at every chunk boundary, so it looks like
/// a grid of squares rather than like ordinary aliasing. The colours stay
/// correct and only the pattern is wrong, which makes this artefact unlike the
/// one an unmipped colour texture produces.
///
/// A level below the top averages across its cells. Two properties of the
/// layout in [`Adt::alpha_atlas`] make that harmless:
///
/// * A box filter cannot cross a cell boundary at these levels. A level-`n`
///   texel covers an aligned `2^n` block of level-0 texels, and a cell is
///   [`ALPHA_SIDE`] = 64 wide, so while `2^n` divides 64, every block lies
///   inside one cell. Level 6 is where a cell becomes one texel, and the chain
///   stops there.
/// * Bilinear filtering does cross a cell boundary, but the neighbouring value
///   is correct. The inset in [`atlas_uv`] is half a texel of the top level, so
///   at level `n` an edge sample sits `0.5 / 2^n` texels inside the cell and
///   bilinear reaches into the next one. The atlas is a 16x16 grid of cells
///   holding a 16x16 grid of chunks, so a cell's neighbour in the atlas is its
///   neighbour in the world, and the blend is continuous across a chunk
///   boundary, measured at 0.05 of 255 by `vale textures`. The atlas edges
///   are the tile's own edges, where `CLAMP_TO_EDGE` repeats the last value,
///   which is the same convention `fix_alpha_edge` applies within a chunk.
///
/// The filter is a box filter, not the alpha-weighted filter a composed skin
/// gets. [`crate::look::character::generate_mips`] weights each tap's colour by
/// its own alpha, which is right for a texture whose transparent texels hold
/// meaningless colour and wrong for this one, where every channel is an
/// independent map: the alpha is `MCSH` (see [`Adt::alpha_atlas`]), so
/// weighting would make a chunk's blend weights fade out wherever its shadow
/// does.
pub fn alpha_atlas_mips(atlas: &[u8]) -> Vec<Vec<u8>> {
    crate::look::character::box_mips(ATLAS_SIDE as u32, ATLAS_SIDE as u32, atlas)
        .into_iter()
        .take(ALPHA_ATLAS_MIPS)
        .collect()
}

/// Where chunk `chunk_index`'s local `(u, v)` lands in the tile's alpha atlas.
///
/// The half-texel inset is the only cost of the atlas: a lookup at exactly the
/// cell boundary would have bilinear filtering average in the neighbouring
/// chunk's blend, which draws a visible seam grid over the landscape. Mapping
/// 0..1 onto texel centres (0.5 to 63.5 of 64) keeps every sample inside its
/// own cell, and loses the outer half-texel of stretch that a `CLAMP_TO_EDGE`
/// per-chunk texture gives.
pub fn atlas_uv(chunk_index: usize, u: f32, v: f32) -> [f32; 2] {
    let (cell_x, cell_y) = (
        (chunk_index % CHUNKS_PER_SIDE) as f32,
        (chunk_index / CHUNKS_PER_SIDE) as f32,
    );
    let side = ALPHA_SIDE as f32;
    let inset = |cell: f32, t: f32| (cell * side + 0.5 + t * (side - 1.0)) / ATLAS_SIDE as f32;
    [inset(cell_x, u), inset(cell_y, v)]
}

/// One liquid type over one tile, ready for the liquid material.
///
/// The same shape a WMO liquid draw reaches the renderer in: positions in the
/// file's axes, one texture repeat per 4.1667-yard tile in the UVs, and the
/// vertex colour carrying `MCLQ`'s depth byte in its alpha. The depth byte is
/// the input to the opacity, not the opacity itself (see
/// [`crate::tables::light`]). The renderer adds what a mesh cannot state: the
/// blend mode, the flipbook texture and the `Light.dbc` tint, as it does for a
/// canal inside a building.
#[derive(Debug, Clone)]
pub struct TerrainLiquidDraw {
    pub kind: Liquid,
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// `[0, 0, 0, depth]` per vertex: black, so the additive vertex-light term
    /// adds nothing, with the depth byte as alpha.
    pub colours: Vec<[u8; 4]>,
    pub indices: Vec<u32>,
}

impl TerrainLiquidDraw {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// A slice of the tile mesh and the textures to draw it with: one per group of
/// chunks that share a texture set, not one per chunk.
#[derive(Debug, Clone)]
pub struct TerrainDraw {
    pub index_start: u32,
    pub index_count: u32,
    /// Indices into [`Adt::texture_names`], base layer first, at most four.
    /// Identical for every chunk in the group; this is what defines the group.
    pub textures: Vec<u32>,
    /// The scroll velocity of each of those layers, in texture widths per
    /// second, from `MCLY`'s animation bits; see [`layer_flags::scroll`]. Zero
    /// for a still layer, which is all but 164 of Azeroth's 386,031 records.
    ///
    /// Also part of what defines the group, for the same reason as the
    /// textures: the velocity reaches the shader as a per-draw-group uniform,
    /// so a still chunk grouped with a scrolling one would scroll. `vale
    /// textures Azeroth 39 32` shows a tile where this happens.
    pub scrolls: Vec<[f32; 2]>,
    /// Bounding sphere over every chunk in the group, world space. A set used
    /// all over the tile gives a tile-sized sphere and culls nothing, so the
    /// renderer culls per tile and uses this only as an extra test.
    pub centre: [f32; 3],
    pub radius: f32,
    /// How many chunks are in the group. Used only by the report that shows
    /// how much the grouping reduces draws.
    pub chunks: u32,
}

/// A sphere covering both inputs. Not the tightest such sphere: its diameter is
/// the distance between the centres plus both radii, which for chunks on a grid
/// is within a few percent of the tightest and never too small.
fn merge_bounds(a: &TerrainDraw, b: &TerrainDraw) -> TerrainDraw {
    let d = ((b.centre[0] - a.centre[0]).powi(2)
        + (b.centre[1] - a.centre[1]).powi(2)
        + (b.centre[2] - a.centre[2]).powi(2))
    .sqrt();
    if d + b.radius <= a.radius {
        return TerrainDraw { chunks: a.chunks + b.chunks, ..a.clone() };
    }
    if d + a.radius <= b.radius {
        return TerrainDraw {
            chunks: a.chunks + b.chunks,
            index_start: a.index_start,
            index_count: a.index_count,
            textures: a.textures.clone(),
            scrolls: a.scrolls.clone(),
            ..b.clone()
        };
    }
    let radius = (d + a.radius + b.radius) / 2.0;
    // Slide the centre towards b by however much a's side has to give up.
    let t = if d > 0.0 { (radius - a.radius) / d } else { 0.0 };
    TerrainDraw {
        index_start: a.index_start,
        index_count: a.index_count,
        textures: a.textures.clone(),
        scrolls: a.scrolls.clone(),
        centre: [
            a.centre[0] + (b.centre[0] - a.centre[0]) * t,
            a.centre[1] + (b.centre[1] - a.centre[1]) * t,
            a.centre[2] + (b.centre[2] - a.centre[2]) * t,
        ],
        radius,
        chunks: a.chunks + b.chunks,
    }
}

/// One of the four triangles a terrain cell is made of, as
/// [`Adt::height_at`] and [`Adt::normal_at`] both need it.
///
/// A named type rather than a tuple because it holds two different things,
/// where the point is and what the triangle is, and together they are all that
/// links a height to the slope it was taken on.
struct Wedge {
    /// The queried point in cell-local `(u, v)`, both `0..1` and both running
    /// along decreasing world x and y.
    at: (f32, f32),
    /// The triangle's three corners in the same coordinates, each with its
    /// height. The third is always the cell's own centre vertex.
    corners: [((f32, f32), f32); 3],
}

/// The unit normal of a wedge, in the world's axes, pointing up. Used by
/// [`Adt::normal_at`] on the tile and [`ChunkGround::normal_at`] on one
/// chunk.
fn wedge_normal(wedge: &Wedge) -> [f32; 3] {
    let wedge = wedge.corners;
    // Cell-local `(u, v)` runs along decreasing world x and y, so a
    // corner's world offset from the cell origin is `(-u, -v)` yards. The
    // two sign flips cancel in the cross product; the `+Z` check below
    // decides the direction.
    let corner = |((u, v), h): ((f32, f32), f32)| [-u * UNIT_SIZE, -v * UNIT_SIZE, h];
    let (a, b, c) = (corner(wedge[0]), corner(wedge[1]), corner(wedge[2]));
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let mut n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    if n[2] < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    // A degenerate wedge answers straight up rather than `None`: there is
    // ground there, but its slope cannot be computed.
    if len < 1e-6 {
        return UP;
    }
    [n[0] / len, n[1] / len, n[2] / len]
}

/// The three world-axis angles that align a placement with a slope: the turn
/// about up as given, and the two tilts that carry the model's up axis onto
/// `normal`.
///
/// The result is in the frame [`placement_euler_from_world`] takes
/// (`[about x, about y, about z]` in degrees), so a caller writes
/// `placement_euler_from_world(lean_to_normal(n, turn))` into the record.
/// Derived from [`placement_matrix`], which defines what the three angles do:
/// with the world angles `w`, the model's up lands at
/// `Rz(w2) · Ry(w1) · Rx(w0) · ẑ`, and inverting that for a unit `normal`
/// takes two lines once the turn is removed. A normal straight up gives no
/// tilt, whatever the turn; a normal at or below the horizon is treated as
/// straight up, since tilting cannot stand a model on a ceiling.
///
/// `vale_assets::world::adt::tests::a_lean_carries_the_models_up_onto_the_normal`
/// checks it through the matrix itself, on a dozen normals at a dozen turns.
pub fn lean_to_normal(normal: [f32; 3], turn_deg: f32) -> [f32; 3] {
    let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if len < 1e-6 || normal[2] <= 0.0 {
        return [0.0, 0.0, turn_deg];
    }
    let n = [normal[0] / len, normal[1] / len, normal[2] / len];
    // Take the turn back out: `m = Rz(-turn) · n`.
    let (s, c) = turn_deg.to_radians().sin_cos();
    let m = [c * n[0] + s * n[1], -s * n[0] + c * n[1], n[2]];
    // `Ry(w1) · Rx(w0) · ẑ = (sin w1 cos w0, -sin w0, cos w1 cos w0)`.
    let about_x = (-m[1]).clamp(-1.0, 1.0).asin();
    let about_y = m[0].atan2(m[2]);
    [about_x.to_degrees(), about_y.to_degrees(), turn_deg]
}

/// The index of an outer-grid `(row, col)` sample among a chunk's 145.
///
/// The 145 are nine rows of nine outer samples interleaved with eight rows of
/// eight inner ones, and both `MCVT` and `MCNR` are in that order. This and
/// [`inner_index`] are the only definition of that order in this module;
/// anything that writes those regions has its own definition, and tests check
/// that the two agree.
pub fn outer_index(row: usize, col: usize) -> usize {
    row * (OUTER_SIDE + INNER_SIDE) + col
}

/// The index of an inner-grid `(row, col)` sample among a chunk's 145. A
/// detail cell's centre is exactly an inner sample, so the foliage reads its
/// per-cell normal directly rather than interpolating; see
/// [`crate::world::foliage::ChunkFoliage`].
pub fn inner_index(row: usize, col: usize) -> usize {
    row * (OUTER_SIDE + INNER_SIDE) + OUTER_SIDE + col
}

/// Where vertex `index` of a chunk's 145 sits, as a distance from the chunk
/// origin along decreasing world x and decreasing world y. Both are at least 0
/// and at most [`CHUNK_SIZE`]. The inverse of [`outer_index`] and
/// [`inner_index`] combined.
pub fn vertex_offset(index: usize) -> (f32, f32) {
    let stride = OUTER_SIDE + INNER_SIDE;
    let row = index / stride;
    let within = index % stride;
    if within < OUTER_SIDE {
        (row as f32 * UNIT_SIZE, within as f32 * UNIT_SIZE)
    } else {
        (
            (row as f32 + 0.5) * UNIT_SIZE,
            (within - OUTER_SIDE) as f32 * UNIT_SIZE + 0.5 * UNIT_SIZE,
        )
    }
}

/// The `(row, col)` of vertex `index` on the 9x9 outer grid, or `None` for an
/// inner vertex. The inverse of [`outer_index`].
pub fn outer_row_col(index: usize) -> Option<(usize, usize)> {
    let stride = OUTER_SIDE + INNER_SIDE;
    let (row, col) = (index / stride, index % stride);
    (col < OUTER_SIDE && row < OUTER_SIDE).then_some((row, col))
}

/// A map chunk's `(x, y)` on its tile's 16x16 grid from its number in `MCIN`
/// order. `x` is the header's `indexX` and grows along decreasing world y; `y`
/// is `indexY` and grows along decreasing world x.
pub fn chunk_xy(index: usize) -> (usize, usize) {
    (index % CHUNKS_PER_SIDE, index / CHUNKS_PER_SIDE)
}

/// The chunk number at `(x, y)` on a tile's 16x16 grid: the inverse of
/// [`chunk_xy`].
pub fn chunk_index(x: usize, y: usize) -> usize {
    y * CHUNKS_PER_SIDE + x
}

/// The chunk number at `(x, y)`, or `None` when either is off the tile's grid.
/// For walking to a neighbouring chunk, where a step can leave the tile.
pub fn chunk_on_grid(x: i32, y: i32) -> Option<usize> {
    let side = CHUNKS_PER_SIDE as i32;
    ((0..side).contains(&x) && (0..side).contains(&y)).then(|| chunk_index(x as usize, y as usize))
}

/// Which of a chunk's 256 triangles a world position falls in, and where in it.
///
/// The single implementation, taking the three fields rather than a `&Mcnk`
/// so that [`ChunkGround`], which holds only those three fields, answers
/// exactly as the whole tile does. A second copy of this rule that differed
/// would stand doodads slightly off the ground the character walks on: a cell
/// is four triangles meeting at its centre vertex, and interpolating a quad
/// instead is wrong on every cell that is not planar.
fn wedge_in(position: [f32; 3], heights: &[f32], holes: u16, x: f32, y: f32) -> Option<Wedge> {
    let outer = |row: usize, col: usize| heights.get(outer_index(row, col)).copied().unwrap_or(0.0);
    let inner = |row: usize, col: usize| heights.get(inner_index(row, col)).copied().unwrap_or(0.0);

    // Cells run in decreasing world x and y from the chunk origin, the
    // same direction `to_mesh` lays its vertices out.
    let fr = (position[0] - x) / UNIT_SIZE;
    let fc = (position[1] - y) / UNIT_SIZE;
    if fr < 0.0 || fc < 0.0 {
        return None;
    }
    let row = (fr.floor() as usize).min(INNER_SIDE - 1);
    let col = (fc.floor() as usize).min(INNER_SIDE - 1);
    // The 16-bit mask is a 4x4 grid; each bit covers a 2x2 block of cells.
    if holes & (1 << ((row / 2) * 4 + col / 2)) != 0 {
        return None;
    }

    // Position within the cell, both 0..1.
    let u = (fr - row as f32).clamp(0.0, 1.0);
    let v = (fc - col as f32).clamp(0.0, 1.0);

    // Which of the four wedges the point falls in, named by the cell edge
    // it touches. The centre vertex is the third corner of all four.
    let (p1, h1, p2, h2) = if u + v < 1.0 {
        if u < v {
            ((0.0, 0.0), outer(row, col), (0.0, 1.0), outer(row, col + 1))
        } else {
            ((0.0, 0.0), outer(row, col), (1.0, 0.0), outer(row + 1, col))
        }
    } else if u < v {
        (
            (0.0, 1.0),
            outer(row, col + 1),
            (1.0, 1.0),
            outer(row + 1, col + 1),
        )
    } else {
        (
            (1.0, 0.0),
            outer(row + 1, col),
            (1.0, 1.0),
            outer(row + 1, col + 1),
        )
    };

    Some(Wedge {
        at: (u, v),
        corners: [(p1, h1), (p2, h2), ((0.5, 0.5), inner(row, col))],
    })
}

/// Height at `p` on the plane through three `(point, height)` samples.
///
/// A degenerate triangle would divide by zero. That cannot happen for the
/// three corners this is called with, but returning the first sample rather
/// than NaN keeps a future caller from writing NaN into a position.
fn barycentric(
    p: (f32, f32),
    a: ((f32, f32), f32),
    b: ((f32, f32), f32),
    c: ((f32, f32), f32),
) -> f32 {
    let ((ax, ay), ha) = a;
    let ((bx, by), hb) = b;
    let ((cx, cy), hc) = c;

    let det = (by - cy) * (ax - cx) + (cx - bx) * (ay - cy);
    if det.abs() < f32::EPSILON {
        return ha;
    }
    let l1 = ((by - cy) * (p.0 - cx) + (cx - bx) * (p.1 - cy)) / det;
    let l2 = ((cy - ay) * (p.0 - cx) + (ax - cx) * (p.1 - cy)) / det;
    let l3 = 1.0 - l1 - l2;
    l1 * ha + l2 * hb + l3 * hc
}

/// A triangle mesh ready to hand to a renderer.
#[derive(Debug, Clone)]
pub struct TerrainMesh {
    pub positions: Vec<[f32; 3]>,
    /// Per-vertex normals from `MCNR`, parallel to `positions`.
    pub normals: Vec<[f32; 3]>,
    /// Per-vertex shading multipliers from `MCCV`, parallel to `positions`, and
    /// [`COLOUR_NONE`] for every vertex of a chunk that carries none, which is
    /// every chunk of every shipped 1.12 tile. See [`decode_colours`].
    ///
    /// Always the full length, never empty, unlike `Mcnk::colours`: a mesh has
    /// one array per attribute, and a renderer cannot use an array that covers
    /// only some vertices. The cost is 16 bytes a vertex on a tile that has no
    /// colours at all, so `render::terrain` checks whether the tile has any
    /// before it uploads them.
    pub colours: Vec<[f32; 4]>,
    /// Chunk-local 0..1. The diffuse textures repeat eight times across a chunk,
    /// so the shader scales these rather than the mesh carrying a second set.
    pub uvs: Vec<[f32; 2]>,
    /// The same coordinate mapped into the tile's alpha atlas ([`atlas_uv`]).
    ///
    /// A second UV set rather than arithmetic in the shader, because the shader
    /// would need the chunk index, and a per-chunk uniform would bring back the
    /// per-chunk draw call the atlas removes.
    pub alpha_uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub draws: Vec<TerrainDraw>,
    /// Bounding sphere over the whole tile, the renderer's culling unit,
    /// because a draw can span the whole tile.
    pub centre: [f32; 3],
    pub radius: f32,
    /// The skirt vertices at the end of `positions`, each as the edge vertex it
    /// copies. See [`Self::source_of`].
    pub skirt_sources: Vec<u32>,
}

impl TerrainMesh {
    /// The chunk vertex a vertex follows: itself, or for a skirt vertex the
    /// edge vertex it hangs from. A caller that rewrites chunk vertices in
    /// place (a height brush) writes a skirt vertex from this one, which folds
    /// the skirt flat until the tile is built again.
    pub fn source_of(&self, index: u32) -> u32 {
        let first = (self.positions.len() - self.skirt_sources.len()) as u32;
        match index.checked_sub(first) {
            Some(skirt) => self.skirt_sources.get(skirt as usize).copied().unwrap_or(index),
            None => index,
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Flatten positions into a single `[x, y, z, x, y, z, ...]` array.
    ///
    /// A tile is ~80k vertices, and serialising them as nested arrays roughly
    /// triples the JSON size; a flat buffer maps directly onto a WebGL vertex
    /// buffer on the receiving side.
    pub fn flatten(&self) -> FlatMesh {
        let mut positions = Vec::with_capacity(self.positions.len() * 3);
        for p in &self.positions {
            positions.extend_from_slice(p);
        }
        let mut normals = Vec::with_capacity(self.normals.len() * 3);
        for n in &self.normals {
            normals.extend_from_slice(n);
        }
        let flat2 = |src: &[[f32; 2]]| {
            let mut out = Vec::with_capacity(src.len() * 2);
            for uv in src {
                out.extend_from_slice(uv);
            }
            out
        };
        FlatMesh {
            positions,
            normals,
            uvs: flat2(&self.uvs),
            alpha_uvs: flat2(&self.alpha_uvs),
            indices: self.indices.clone(),
            draws: self.draws.clone(),
            centre: self.centre,
            radius: self.radius,
        }
    }
}

/// [`TerrainMesh`] in the layout a GPU buffer wants.
#[derive(Debug, Clone)]
pub struct FlatMesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    pub alpha_uvs: Vec<f32>,
    pub indices: Vec<u32>,
    pub draws: Vec<TerrainDraw>,
    pub centre: [f32; 3],
    pub radius: f32,
}

/// `MDDF`: 36 bytes per doodad placement.
fn parse_doodads(data: &[u8]) -> Vec<DoodadPlacement> {
    const ENTRY: usize = 36;
    data.chunks_exact(ENTRY)
        .map(|e| DoodadPlacement {
            name_id: chunk::u32_at(e, 0),
            unique_id: chunk::u32_at(e, 4),
            position: [
                chunk::f32_at(e, 8),
                chunk::f32_at(e, 12),
                chunk::f32_at(e, 16),
            ],
            rotation: [
                chunk::f32_at(e, 20),
                chunk::f32_at(e, 24),
                chunk::f32_at(e, 28),
            ],
            // Fixed point: 1024 == 1.0.
            scale: u16::from_le_bytes([e[32], e[33]]) as f32 / 1024.0,
            flags: u16::from_le_bytes([e[34], e[35]]),
        })
        .collect()
}

/// One `MODF` placement's own box, in world axes.
///
/// The corners are stated in the same (westward, up, northward) frame as the
/// position, so converting both and re-sorting per axis gives the world box.
/// `vale wmos`' placement check compares the built geometry against this box
/// and measures 0.00 yards of horizontal error over every building on a tile.
/// Both use this one conversion.
fn modf_world_box(w: &WmoPlacement) -> [[f32; 3]; 2] {
    let a = placement_to_world(w.bounds_lower);
    let b = placement_to_world(w.bounds_upper);
    [
        [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
        [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
    ]
}

/// `MODF`: 64 bytes per WMO placement.
pub(crate) fn parse_wmos(data: &[u8]) -> Vec<WmoPlacement> {
    const ENTRY: usize = 64;
    data.chunks_exact(ENTRY)
        .map(|e| WmoPlacement {
            name_id: chunk::u32_at(e, 0),
            unique_id: chunk::u32_at(e, 4),
            position: [
                chunk::f32_at(e, 8),
                chunk::f32_at(e, 12),
                chunk::f32_at(e, 16),
            ],
            rotation: [
                chunk::f32_at(e, 20),
                chunk::f32_at(e, 24),
                chunk::f32_at(e, 28),
            ],
            bounds_lower: [
                chunk::f32_at(e, 32),
                chunk::f32_at(e, 36),
                chunk::f32_at(e, 40),
            ],
            bounds_upper: [
                chunk::f32_at(e, 44),
                chunk::f32_at(e, 48),
                chunk::f32_at(e, 52),
            ],
            flags: u16::from_le_bytes([e[56], e[57]]),
            doodad_set: u16::from_le_bytes([e[58], e[59]]),
            name_set: u16::from_le_bytes([e[60], e[61]]),
        })
        .collect()
}

/// Locate an MCNK sub-chunk by magic.
///
/// A scan rather than a jump via the header's `ofsHeight` / `ofsNormal`
/// fields: those offsets are relative to different bases in different
/// documentation, and `MCNR` declares 435 bytes while occupying 448. Scanning
/// avoids both problems, and an MCNK payload is only a few kilobytes.
fn find_subchunk<'a>(payload: &'a [u8], magic: &[u8; 4]) -> Option<&'a [u8]> {
    let i = subchunk_position(payload, magic)?;
    let size = chunk::u32_at(payload, i + 4) as usize;
    let start = i + 8;
    let end = start.saturating_add(size).min(payload.len());
    Some(&payload[start..end])
}

/// Where a sub-chunk's magic sits in the payload, for the one sub-chunk whose
/// declared size is wrong: `MCLQ` is written with a size of zero, so its
/// extent has to come from the MCNK header's `sizeLiquid` instead.
fn subchunk_position(payload: &[u8], magic: &[u8; 4]) -> Option<usize> {
    let needle = [magic[3], magic[2], magic[1], magic[0]];
    let mut i = MCNK_HEADER_SIZE.min(payload.len());
    while i + 8 <= payload.len() {
        if payload[i..i + 4] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_mcnk(payload: &[u8]) -> Option<Mcnk> {
    if payload.len() < MCNK_HEADER_SIZE {
        return None;
    }

    // Field offsets within the 128-byte MCNK header.
    let flags = chunk::u32_at(payload, 0x00);
    let index_x = chunk::u32_at(payload, 0x04);
    let index_y = chunk::u32_at(payload, 0x08);
    let area_id = chunk::u32_at(payload, 0x34);
    // `holes` is a u16 followed by 2 padding bytes.
    let holes = (chunk::u32_at(payload, 0x3C) & 0xFFFF) as u16;
    // `sizeLiquid` is the only correct statement of MCLQ's extent (the
    // sub-chunk's own size field is zero), and it is 8 (the bare header) when
    // the chunk is dry.
    let size_liquid = chunk::u32_at(payload, 0x64) as usize;
    // Two fields only the ground effects read: `0x40` is eight `u16`s naming
    // the dominant texture layer of each of the 64 detail cells, and `0x50` is
    // eight bytes of per-cell "grow nothing" bits. See `Mcnk::detail_layer`
    // and `Mcnk::no_detail`.
    let mut detail_layer = [0u16; 8];
    for (row, entry) in detail_layer.iter_mut().enumerate() {
        *entry = (chunk::u32_at(payload, 0x40 + (row / 2) * 4) >> ((row % 2) * 16)) as u16;
    }
    let no_detail = u64::from(chunk::u32_at(payload, 0x50))
        | u64::from(chunk::u32_at(payload, 0x54)) << 32;
    let position = [
        chunk::f32_at(payload, 0x68),
        chunk::f32_at(payload, 0x6C),
        chunk::f32_at(payload, 0x70),
    ];

    // MCVT: 145 f32 heights, stored relative to the chunk's z position.
    let mut heights = Vec::with_capacity(HEIGHTS_PER_CHUNK);
    if let Some(mcvt) = find_subchunk(payload, b"MCVT") {
        for i in 0..HEIGHTS_PER_CHUNK {
            heights.push(position[2] + chunk::f32_at(mcvt, i * 4));
        }
    } else {
        heights.resize(HEIGHTS_PER_CHUNK, position[2]);
    }

    // MCNR: 145 normals, three signed bytes each, in the same interleaved order
    // as MCVT, scaled by 1/127.
    //
    // The components are already in world axes, `(north, west, up)`, with no
    // swap. Sources disagree about this (a `(x, z, y)` ordering is widely
    // repeated), so it was measured against the normals implied by the MCVT
    // grid, which is known to be correct because the character walks on it.
    // On `Azeroth_34_51` chunk 0 the two agree sample for sample:
    //
    // ```text
    //   outer (0,0)  grid (-0.23, -0.01, 0.97)   MCNR (-27,  0, 123)/127
    //   outer (2,3)  grid (-0.17,  0.12, 0.98)   MCNR (-22, 11, 124)/127
    //   outer (4,4)  grid (-0.04,  0.18, 0.98)   MCNR ( -1, 19, 124)/127
    // ```
    //
    // Swapping the last two bytes leaves the up component near zero, which
    // lights the whole tile as if the sun were underground.
    //
    // MCNR declares 435 bytes while occupying 448 (13 bytes of padding).
    // `find_subchunk` scans rather than trusting offsets, so the padding does
    // not affect parsing.
    let mut normals = Vec::with_capacity(HEIGHTS_PER_CHUNK);
    if let Some(mcnr) = find_subchunk(payload, b"MCNR") {
        normals = decode_normals(mcnr);
    } else {
        normals.resize(HEIGHTS_PER_CHUNK, UP);
    }

    // MCCV: 145 per-vertex shading multipliers; see `decode_colours`, which
    // documents the format and why no shipped 1.12 tile has one. Left empty
    // when the chunk has none rather than filled with the neutral value: a
    // chunk with no colours and a chunk painted entirely neutral draw the
    // same, and only the empty one costs nothing to carry.
    let colours = find_subchunk(payload, b"MCCV")
        .map(decode_colours)
        .unwrap_or_default();

    // MCLY: 16 bytes per texture layer.
    let mut layers = Vec::new();
    if let Some(mcly) = find_subchunk(payload, b"MCLY") {
        for e in mcly.chunks_exact(16) {
            layers.push(TextureLayer {
                texture_id: chunk::u32_at(e, 0),
                flags: chunk::u32_at(e, 4),
                alpha_offset: chunk::u32_at(e, 8),
                effect_id: chunk::u32_at(e, 12),
            });
        }
    }

    let mcal = find_subchunk(payload, b"MCAL").unwrap_or(&[]);
    let alphas = decode_alpha_maps(&layers, mcal);

    // MCSH: 512 bits of baked shadow, read only when `HAS_MCSH` is set. The
    // sub-chunk is present on chunks that declare no shadow at all (the tools
    // write it whether or not any shadow falls on the ground there), so
    // reading it on the sub-chunk's presence alone draws whatever stale bits
    // it holds.
    let shadow = match find_subchunk(payload, b"MCSH") {
        Some(mcsh) if flags & mcnk_flags::HAS_MCSH != 0 => decode_shadow_map(mcsh),
        _ => Vec::new(),
    };

    let liquids = parse_mclq(payload, size_liquid, flags, position);

    Some(Mcnk {
        index_x,
        index_y,
        area_id,
        flags,
        holes,
        position,
        heights,
        normals,
        colours,
        layers,
        alphas,
        shadow,
        liquids,
        detail_layer,
        no_detail,
    })
}

/// `MCSH`'s 512 packed bits as one byte per texel, 255 where the ground is in
/// shadow.
///
/// The bits run LSB first within each byte, the same order `MCAL`'s 4-bit
/// alpha nibbles run in. This is the only detail here that the file's layout
/// does not make evident. Reversing it mirrors every group of eight texels
/// within its own byte, which shows not as an obvious error but as a shadow
/// with a comb pattern through it, at a period of 8 of 64 texels, along one
/// axis only. `vale textures` measures this: it reports the horizontal and the
/// vertical edge density of the decoded map side by side, and the two are
/// within a few per cent of each other when the order is right (a baked shadow
/// has no reason to prefer an axis) and several times apart when it is not.
///
/// The last row and column get [`fix_alpha_edge`] for the same reason an alpha
/// map does (the pre-Cataclysm client stretches a 64x64 map so that texels 0
/// and 62 sit on the chunk's edges, and never reads 63); without it every
/// chunk's far edge draws an unwritten strip of shadow.
pub fn decode_shadow_map(src: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; ALPHA_LEN];
    for i in 0..ALPHA_LEN {
        let Some(&byte) = src.get(i / 8) else { break };
        out[i] = if byte >> (i % 8) & 1 != 0 { 255 } else { 0 };
    }
    fix_alpha_edge(&mut out);
    out
}

/// Bytes an `MCSH` occupies: one bit per texel of a 64x64 map.
pub const SHADOW_MAP_BYTES: usize = ALPHA_LEN / 8;

/// Pack a 64x64 shadow map, one byte per texel with any non-zero value in
/// shadow, into the [`SHADOW_MAP_BYTES`] of an `MCSH`: the inverse of
/// [`decode_shadow_map`]. Texel `i` is bit `i % 8` of byte `i / 8`, LSB first.
/// Texels past [`ALPHA_LEN`] are ignored and missing ones are lit.
pub fn encode_shadow_map(texels: &[u8]) -> Vec<u8> {
    let mut bits = vec![0u8; SHADOW_MAP_BYTES];
    for (i, _) in texels.iter().take(ALPHA_LEN).enumerate().filter(|(_, &t)| t != 0) {
        bits[i / 8] |= 1 << (i % 8);
    }
    bits
}

/// Parse a chunk's `MCLQ` into one grid per declared liquid.
///
/// The sub-chunk is located by scanning for its magic, like every other MCNK
/// sub-chunk here, but its extent comes from the header's `sizeLiquid`,
/// because the tools that made these files wrote `MCLQ`'s own size field as
/// zero. Its contents are declared outside it: one block per liquid flag bit
/// in the MCNK header, in bit order.
fn parse_mclq(
    payload: &[u8],
    size_liquid: usize,
    flags: u32,
    position: [f32; 3],
) -> Vec<(Liquid, WmoLiquid)> {
    // 8 is the bare sub-chunk header: a dry chunk.
    if size_liquid <= 8 || flags & mcnk_flags::LQ_ANY == 0 {
        return Vec::new();
    }
    let Some(start) = subchunk_position(payload, b"MCLQ") else {
        return Vec::new();
    };
    let end = start.saturating_add(size_liquid).min(payload.len());
    let mut data = payload.get(start + 8..end).unwrap_or(&[]);

    let kinds = [
        (mcnk_flags::LQ_RIVER, Liquid::Water),
        (mcnk_flags::LQ_OCEAN, Liquid::Ocean),
        (mcnk_flags::LQ_MAGMA, Liquid::Magma),
        (mcnk_flags::LQ_SLIME, Liquid::Slime),
    ];
    let mut out = Vec::new();
    for (bit, kind) in kinds {
        if flags & bit == 0 {
            continue;
        }
        // A truncated tail drops the remaining blocks, not the whole chunk.
        let Some(grid) = parse_mclq_block(data, position) else {
            break;
        };
        out.push((kind, grid));
        data = data.get(MCLQ_BLOCK_SIZE..).unwrap_or(&[]);
    }
    out
}

/// One `MCLQ` block as a [`WmoLiquid`], re-ordered into its frame.
///
/// The file's 9x9 vertices run like `MCVT`'s outer grid: row-major, rows
/// along decreasing world x, columns along decreasing world y from the chunk
/// origin. `WmoLiquid` runs +X/+Y from a low corner. The map between the two
/// is `(row, col) = (8 - gx, 8 - gy)`, a 180° turn in the plane, which
/// preserves triangle winding, so the quad emission that faces a WMO pool
/// upward also faces a lake upward.
///
/// Heights are absolute world z, not offsets from the chunk's. `vale water`
/// confirms this by checking that the surface stands above the terrain
/// beneath it; reading the heights as offsets would put the surface ~90 yards
/// wrong on any city tile.
fn parse_mclq_block(data: &[u8], position: [f32; 3]) -> Option<WmoLiquid> {
    if data.len() < MCLQ_TILES_OFFSET + MCLQ_TILES {
        return None;
    }
    let side = INNER_SIDE + 1;
    let mut heights = vec![0f32; MCLQ_VERTS];
    let mut depths = vec![0u8; MCLQ_VERTS];
    for row in 0..side {
        for col in 0..side {
            let v = &data[8 + (row * side + col) * LIQUID_VERTEX_SIZE..][..LIQUID_VERTEX_SIZE];
            let (gx, gy) = (INNER_SIDE - row, INNER_SIDE - col);
            heights[gy * side + gx] = chunk::f32_at(v, 4);
            depths[gy * side + gx] = v[0];
        }
    }
    let mut tile_flags = vec![0x0Fu8; MCLQ_TILES];
    for row in 0..INNER_SIDE {
        for col in 0..INNER_SIDE {
            let (tx, ty) = (INNER_SIDE - 1 - row, INNER_SIDE - 1 - col);
            let flag = data[MCLQ_TILES_OFFSET + row * INNER_SIDE + col];
            // The low nibble is the cell's liquid type, and the 1.12.1 client
            // draws nothing on a cell of type 0, so such a cell is read as dry.
            // Every wet cell of a shipped tile carries its block's type: water
            // 4, ocean 1, magma 6.
            tile_flags[ty * INNER_SIDE + tx] = match flag & 0x0F {
                0 => flag | 0x0F,
                _ => flag,
            };
        }
    }
    Some(WmoLiquid {
        x_tiles: INNER_SIDE,
        y_tiles: INNER_SIDE,
        base: [
            position[0] - CHUNK_SIZE,
            position[1] - CHUNK_SIZE,
            // The block's own stated minimum, which `WmoLiquid::height` falls
            // back to for an index outside the grid.
            chunk::f32_at(data, 0),
        ],
        // MCLQ states no type of its own; the MCNK flag bit paired with this
        // grid is the only declaration.
        liquid_type: 0,
        heights,
        depths,
        tile_flags,
    })
}

/// Decode every layer's alpha map out of one chunk's `MCAL`.
///
/// Three encodings share this chunk and nothing in it says which is in use:
///
/// * 4-bit, 2048 bytes: two texels per byte, low nibble first. This is what
///   1.12 maps use.
/// * 8-bit, 4096 bytes: one byte per texel ("big alpha", flagged by `MPHD`
///   bit 0x4 in the WDT).
/// * RLE, variable: flagged per layer by [`layer_flags::ALPHA_COMPRESSED`].
///
/// The width is chosen by measuring rather than by trusting the WDT: an `Adt`
/// does not have the WDT to hand, and a patched archive can disagree with what
/// the client build implies. The layer offsets give the measurement:
/// consecutive alpha maps sit back to back, so the gap between them is the
/// stride.
fn decode_alpha_maps(layers: &[TextureLayer], mcal: &[u8]) -> Vec<Vec<u8>> {
    let stride = alpha_stride(layers, mcal.len());
    layers
        .iter()
        .enumerate()
        .map(|(i, layer)| {
            // Layer 0 is the base: opaque everywhere, no map in the file.
            if i == 0 || layer.flags & layer_flags::USE_ALPHA_MAP == 0 {
                return vec![255u8; ALPHA_LEN];
            }
            let start = layer.alpha_offset as usize;
            if start >= mcal.len() {
                return vec![0u8; ALPHA_LEN];
            }
            let rest = &mcal[start..];
            let mut map = if layer.flags & layer_flags::ALPHA_COMPRESSED != 0 {
                decompress_alpha_rle(rest)
            } else if stride == ALPHA_LEN {
                let mut a = vec![0u8; ALPHA_LEN];
                let n = rest.len().min(ALPHA_LEN);
                a[..n].copy_from_slice(&rest[..n]);
                a
            } else {
                expand_4bit_alpha(rest)
            };
            fix_alpha_edge(&mut map);
            map
        })
        .collect()
}

/// Replace the last row and column with copies of the second-to-last.
///
/// A 64x64 alpha map holds 63x63 texels of blend. The pre-Cataclysm client
/// stretches the map over the chunk so that the centres of texels 0 and 62 sit
/// on the chunk's two edges, and never reads row or column 63, so Blizzard's
/// tools never wrote meaningful values there. Noggit does the same duplication
/// (`MapChunk`, gated on the WotLK-era `do_not_fix_alpha_map` flag, which no
/// 1.12 WDT sets).
///
/// Rendering the unwritten edge instead draws a grid of seams on every chunk
/// boundary, 256 per tile. It looks like a checkerboard pattern rather than an
/// error, because the artefact appears only on a chunk's two far edges and
/// alternates with its neighbours' clean near edges.
///
/// `vale textures` measures this: against the neighbouring chunk's first
/// column, which the ground has to be continuous with, column 62 differs by
/// 12.9–27.6 (of 255) across three tilesets, and column 63 by 42.1–53.6.
/// Column 62 continues into the neighbour; column 63 holds no blend data.
///
/// Copying rather than dropping keeps the map [`ALPHA_LEN`] bytes, so the atlas,
/// the UV inset and every existing test see the shape they expect; the last
/// texel holds the value the client would use for it.
pub fn fix_alpha_edge(map: &mut [u8]) {
    if map.len() != ALPHA_LEN {
        return;
    }
    let last = ALPHA_SIDE - 1;
    for t in 0..ALPHA_SIDE {
        map[t * ALPHA_SIDE + last] = map[t * ALPHA_SIDE + last - 1];
    }
    for t in 0..ALPHA_SIDE {
        map[last * ALPHA_SIDE + t] = map[(last - 1) * ALPHA_SIDE + t];
    }
}

/// Bytes one uncompressed alpha map occupies: [`ALPHA_LEN`] for 8-bit, half
/// that for 4-bit.
///
/// The gap between two consecutive layer offsets gives the stride directly.
/// With fewer than two alpha layers to compare there is no gap, so it falls
/// back to the total size: a lone 4096-byte `MCAL` holding one map is 8-bit.
pub fn alpha_stride(layers: &[TextureLayer], mcal_len: usize) -> usize {
    let mut offsets: Vec<usize> = layers
        .iter()
        .skip(1)
        .filter(|l| {
            l.flags & layer_flags::USE_ALPHA_MAP != 0
                && l.flags & layer_flags::ALPHA_COMPRESSED == 0
        })
        .map(|l| l.alpha_offset as usize)
        .collect();
    offsets.sort_unstable();

    if let Some(gap) = offsets.windows(2).map(|w| w[1] - w[0]).find(|g| *g > 0) {
        return if gap >= ALPHA_LEN { ALPHA_LEN } else { ALPHA_LEN / 2 };
    }
    match offsets.first() {
        Some(&first) if mcal_len.saturating_sub(first) >= ALPHA_LEN => ALPHA_LEN,
        _ => ALPHA_LEN / 2,
    }
}

/// 2048 bytes of nibbles -> 4096 bytes, low nibble first.
///
/// The nibble is scaled by 17 rather than by 16 so that 0xF becomes 255 and not
/// 240. A layer meant to be fully opaque must reach 255, or every blend lets
/// the layer beneath it show through faintly.
pub(crate) fn expand_4bit_alpha(src: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; ALPHA_LEN];
    for i in 0..ALPHA_LEN {
        let Some(byte) = src.get(i / 2) else { break };
        let nibble = if i % 2 == 0 { byte & 0x0F } else { byte >> 4 };
        out[i] = nibble * 17;
    }
    out
}

/// One layer's alpha map out of `MCAL`, without the edge fix.
///
/// [`decode_alpha_maps`] is for a renderer and applies [`fix_alpha_edge`] to
/// its output; this function is for a caller that will write the bytes back,
/// and returns the map as the file holds it. The two differ in the last row
/// and column, so decoding through the renderer's version, changing nothing
/// and encoding again writes a file whose far edges have been overwritten by
/// their neighbours.
///
/// The base layer has no map, because every other layer blends over it, so a
/// caller asking for layer 0 gets an all-255 (opaque) map.
pub fn decode_alpha_layer(index: usize, layer: &TextureLayer, mcal: &[u8], stride: usize) -> Vec<u8> {
    if index == 0 || layer.flags & layer_flags::USE_ALPHA_MAP == 0 {
        return vec![255u8; ALPHA_LEN];
    }
    let start = layer.alpha_offset as usize;
    if start >= mcal.len() {
        return vec![0u8; ALPHA_LEN];
    }
    let rest = &mcal[start..];
    if layer.flags & layer_flags::ALPHA_COMPRESSED != 0 {
        return decompress_alpha_rle(rest);
    }
    if stride == ALPHA_LEN {
        let mut map = vec![0u8; ALPHA_LEN];
        let n = rest.len().min(ALPHA_LEN);
        map[..n].copy_from_slice(&rest[..n]);
        return map;
    }
    expand_4bit_alpha(rest)
}

/// 4096 bytes -> 2048 of nibbles, the inverse of [`expand_4bit_alpha`].
///
/// Rounded rather than truncated. The decode multiplies by 17, so 255 must
/// come back as 0xF and 254 must not come back as 0xE: `(a * 15 + 127) / 255`
/// is the nearest nibble, and every value the decode can produce encodes back
/// to the nibble it came from.
pub(crate) fn pack_4bit_alpha(map: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; ALPHA_LEN / 2];
    for i in 0..ALPHA_LEN {
        let value = map.get(i).copied().unwrap_or(0) as u32;
        let nibble = ((value * 15 + 127) / 255) as u8;
        match i % 2 {
            0 => out[i / 2] |= nibble,
            _ => out[i / 2] |= nibble << 4,
        }
    }
    out
}

/// Lay a chunk's alpha maps out as one `MCAL` payload, and say where each layer
/// landed.
///
/// The inverse of [`decode_alpha_maps`], used by anything that writes a tile.
/// The output is uncompressed: the RLE form is a post-1.12 addition that the
/// decoder handles because patched archives are not always what the client
/// version implies, and writing it would produce a file the 1.12.1 (build
/// 5875) client cannot read.
///
/// `maps[0]` is the base layer and is not written; the returned offset for it is
/// zero, which is what a `MCLY` record with no [`layer_flags::USE_ALPHA_MAP`]
/// carries. Every other layer is written at `stride` bytes, in layer order, so
/// [`alpha_stride`]'s gap rule reads back the stride it was given.
pub fn encode_alpha_maps(maps: &[Vec<u8>], stride: usize) -> (Vec<u8>, Vec<u32>) {
    let mut out: Vec<u8> = Vec::with_capacity(maps.len().saturating_sub(1) * stride);
    let mut offsets = Vec::with_capacity(maps.len());
    for (index, map) in maps.iter().enumerate() {
        if index == 0 {
            offsets.push(0);
            continue;
        }
        offsets.push(out.len() as u32);
        match stride == ALPHA_LEN {
            true => {
                let mut plane = map.clone();
                plane.resize(ALPHA_LEN, 0);
                out.extend_from_slice(&plane);
            }
            false => out.extend_from_slice(&pack_4bit_alpha(map)),
        }
    }
    (out, offsets)
}

/// RLE alpha: a control byte whose top bit selects fill or copy, and whose low
/// seven bits are the run length.
fn decompress_alpha_rle(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ALPHA_LEN);
    let mut i = 0usize;
    while out.len() < ALPHA_LEN && i < src.len() {
        let control = src[i];
        i += 1;
        let count = (control & 0x7F) as usize;
        let remaining = ALPHA_LEN - out.len();
        let count = count.min(remaining);

        if control & 0x80 != 0 {
            let Some(&value) = src.get(i) else { break };
            i += 1;
            out.extend(std::iter::repeat_n(value, count));
        } else {
            let end = (i + count).min(src.len());
            out.extend_from_slice(&src[i..end]);
            i = end;
        }
    }
    out.resize(ALPHA_LEN, 0);
    out
}

#[cfg(test)]
mod tests;
