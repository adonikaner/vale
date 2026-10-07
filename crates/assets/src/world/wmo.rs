//! WMO — World Map Object: every building, bridge, cave mouth and dungeon in
//! the game. The `MODF` placements that put them on a tile were already parsed
//! (`adt::placed_wmos`); this is the format they point at.
//!
//! A WMO is **two or more files**. The root (`Stormwind.wmo`) holds the
//! materials, the texture names, the doodad sets and one info record per group;
//! each group (`Stormwind_000.wmo`, `_001`, ...) holds one room's or one
//! wall's worth of geometry. The split is the game's own visibility unit: a
//! group is what the client culls, lights and portals independently.
//!
//! ```text
//! root                                   group
//!   MVER  u32 version (17)                 MVER
//!   MOHD  64-byte header, group count      MOGP  68-byte header, then nested:
//!   MOTX  \0-separated texture names               MOPY  2 bytes per triangle
//!   MOMT  64 bytes per material                    MOVI  u16 indices
//!   MOGN  \0-separated group names                 MOVT  3 f32 positions
//!   MOGI  32 bytes per group                       MONR  3 f32 normals
//!   MOSB  skybox model name                        MOTV  2 f32 texture coords
//!   MOLT/MOPV/MOPT/MOPR  lights, portals           MOBA  24 bytes per batch
//!   MODS  32 bytes per doodad set                  MOLR/MODR/MOBN/MOBR
//!   MODN  \0-separated doodad model names          MOCV  vertex colours
//!   MODD  40 bytes per doodad spawn                MLIQ  water
//!   MFOG  fog
//! ```
//!
//! ## The group file's sub-chunks are nested inside `MOGP`
//!
//! `MOGP` is a normal chunk whose payload is a 68-byte header followed by more
//! chunks. Both reference implementations sidestep the nesting — vmangos'
//! extractor overwrites `MOGP`'s size with 68 and keeps reading linearly, and
//! Noggit reads the header and then carries on through the file — because the
//! two are equivalent in practice: `MOGP` is the last top-level chunk. This
//! parser does the same, walking from `MOGP`'s payload + 68 to end of file, so a
//! file whose `MOGP` size field is wrong (they exist) still reads.
//!
//! ## Sources
//!
//! Geometry, placement and the coordinate frame come from vmangos'
//! `contrib/vmap_extractor/vmapextract/wmo.cpp`, which is the authority this
//! project uses for anything the server also has an opinion about. The
//! *rendering* side — materials, `MOBA` batch layout, which blend mode means
//! what — is not in vmangos at all (it only wants collision), and is taken from
//! Noggit's `src/noggit/WMO.cpp`, which renders these files correctly.
//!
//! **`MOVT` vertices are in the same model space as an M2's**, so
//! [`crate::world::adt::placement_matrix`] applies unchanged. That is not obvious and
//! it is worth knowing why: vmangos writes M2 vertices through
//! `fixCoordSystem` and then immediately back through the inverse swap in
//! `Model::ConvertToVMAPModel` (a no-op overall), and writes `MOVT` **raw**.
//! Both therefore reach `ModelInstance` — which is where the placement rotation
//! lives — in the file's own axes.

use crate::world::chunk::{self, Chunk, ChunkReader};
use crate::AssetError;

/// WMO version in 1.12. Later expansions bump this and change `MOMT`.
pub const WMO_VERSION: u32 = 17;

/// Bytes in the `MOGP` header that precedes a group's sub-chunks.
const MOGP_HEADER_SIZE: usize = 68;

/// `MOMT` stride.
const MATERIAL_SIZE: usize = 0x40;
/// `MOGI` stride.
const GROUP_INFO_SIZE: usize = 32;
/// `MODS` stride.
const DOODAD_SET_SIZE: usize = 32;
/// `MODD` stride.
const DOODAD_DEF_SIZE: usize = 40;
/// `MOBA` stride.
const BATCH_SIZE: usize = 24;

/// `MOMT` flag bits this renderer acts on.
pub mod material_flags {
    /// Ignore lighting — the material is drawn at full brightness.
    pub const UNLIT: u32 = 0x01;
    /// Do not cull back faces. Set on fences, railings and window frames.
    pub const UNCULLED: u32 = 0x04;
}

/// `MOHD` flag bits. Only the two the vertex-colour fixup consults are acted
/// on; the others are named so a reader knows the field is not a mystery.
pub mod root_flags {
    pub const DO_NOT_ATTENUATE_VERTICES: u32 = 0x01;
    /// The root's ambient colour is applied by the renderer rather than baked
    /// out of the vertex colours — so [`WmoGroup::shaded_colours`] must not
    /// subtract it.
    pub const UNIFIED_RENDER_PATH: u32 = 0x02;
    pub const USE_LIQUID_TYPE_DBC_ID: u32 = 0x04;
    /// The `MOCV` colours are already final; only their alpha needs forcing.
    pub const DO_NOT_FIX_VERTEX_COLOUR_ALPHA: u32 = 0x08;
}

/// `MOPY` flag bits — one byte per *triangle*, saying what the surface is for.
///
/// Transcribed from vmangos' `MopyFlags` (`vmap_extractor/wmo.h`), which is the
/// authority here because collision is exactly what its extractor reads them
/// for. Only the three [`collides`] consults are acted on.
pub mod mopy_flags {
    /// Decoration laid over another surface — grass on a floor, trim on a wall.
    /// Rendered, but not solid on its own.
    pub const DETAIL: u8 = 0x04;
    /// Solid regardless of whether it is drawn. This is how an invisible
    /// collision hull is marked.
    pub const COLLISION: u8 = 0x08;
    pub const RENDER: u8 = 0x20;
}

/// Is a triangle with these `MOPY` flags solid?
///
/// vmangos' rule, verbatim (`WMOGroup::ConvertToVMAPGroupWmo`):
///
/// ```text
/// isRenderFace = (flags & RENDER) && !(flags & DETAIL);
/// isCollision  = (flags & COLLISION) || isRenderFace;
/// ```
///
/// The `DETAIL` term is the part that is not guessable: a drawn triangle is
/// normally solid, *except* the decorative ones laid over a surface that is
/// already solid. Dropping the term costs nothing visible and doubles the
/// triangle count in exactly the places (floors, walls) that are queried most.
pub fn collides(flags: u8) -> bool {
    let render_face = flags & mopy_flags::RENDER != 0 && flags & mopy_flags::DETAIL == 0;
    flags & mopy_flags::COLLISION != 0 || render_face
}

/// `MOGP` flag bits worth naming.
pub mod group_flags {
    pub const HAS_VERTEX_COLOUR: u32 = 0x0004;
    pub const EXTERIOR: u32 = 0x0008;
    /// **The second exterior bit, and the one this client used to miss.**
    ///
    /// The client never tests `EXTERIOR` on its own: it tests the *pair*,
    /// `flags & 0x48`, everywhere it asks — including right before the
    /// visible-group doodad walk. A group carrying
    /// only this bit is outdoor geometry — a city street, a courtyard, a bridge
    /// deck — and reading it as indoor draws it flat at its baked colour with no
    /// sun on it at all, which is most of "certain objects are obnoxiously dark".
    ///
    /// See [`WmoGroup::is_exterior`], which is the one place the mask is asked.
    pub const EXTERIOR_LIT: u32 = 0x0040;
    /// The group is unreachable — vmangos drops these from collision.
    pub const UNREACHABLE: u32 = 0x0080;
    pub const HAS_LIGHTS: u32 = 0x0200;
    pub const HAS_DOODADS: u32 = 0x0800;
    pub const HAS_WATER: u32 = 0x1000;
    pub const INDOOR: u32 = 0x2000;
    /// **The group is open sky for the rules that care** — a mount may be
    /// summoned on it and an outdoor-only spell cast.
    ///
    /// It is the one bit both halves read for that question and neither reads
    /// `INDOOR` or `EXTERIOR` instead. The client's outdoors test
    /// takes the group the unit's model is linked
    /// into and answers `(flags >> 15) & 1`; vmangos' `IsOutdoorWMO` is
    /// `(mogpFlags & 0x8000) != 0`. Stormwind's streets carry it and its
    /// buildings do not, which is the whole of "you can mount in the city but
    /// not in the bank". See [`outdoors_at`].
    pub const OUTDOOR: u32 = 0x8000;
    /// The group's `MLIQ` is sea water rather than fresh. **This is what
    /// separates ocean from water**, not the liquid type — see
    /// [`WmoGroup::liquid_kind`].
    pub const LIQUID_IS_OCEAN: u32 = 0x0008_0000;
    /// Invisible geometry that only blocks visibility. Never drawn.
    pub const ANTIPORTAL: u32 = 0x0400_0000;
}

/// **How far below the probe the ground may be and still count as covering a
/// building's floor**, in yards — vmangos' own `z + 2.0f`, which its comment
/// says it took from `GetHeightStatic()`.
const TERRAIN_OVER_FLOOR: f32 = 2.0;

/// **Is a character whose probe is at `probe_z` under open sky?** — the rule
/// the server applies to a mount and to every outdoor-only spell, as
/// `TerrainInfo::IsOutdoors`:
///
/// * no building floor under the probe: **outdoors**;
/// * the terrain lies between the probe (plus [`TERRAIN_OVER_FLOOR`]) and the
///   building floor — a cellar under a hill, a floor the ground has buried —
///   then the building does not count: **outdoors**;
/// * otherwise the floor's group decides, by [`group_flags::OUTDOOR`].
///
/// `building` is `(floor height, MOGP flags)` from
/// [`crate::world::collision::CollisionWorld::building_floor`], asked with the
/// probe as its ceiling. The probe is the feet plus a yard, which is vmangos'
/// `z += 1.0f` and keeps a character standing on a floor from reading the
/// floor as above them.
pub fn outdoors_at(building: Option<(f32, u32)>, terrain: Option<f32>, probe_z: f32) -> bool {
    building_under(building, terrain, probe_z).is_none_or(|flags| flags & group_flags::OUTDOOR != 0)
}

/// **The `MOGP` flags of the building group a character counts as standing
/// in**, or `None` where the building does not count: the first two clauses
/// of [`outdoors_at`], before the flag is read. The minimap asks this to
/// decide whether it shows the building's own pictures.
pub fn building_under(building: Option<(f32, u32)>, terrain: Option<f32>, probe_z: f32) -> Option<u32> {
    let (floor, flags) = building?;
    if let Some(ground) = terrain {
        if probe_z + TERRAIN_OVER_FLOOR > ground && ground > floor {
            return None;
        }
    }
    Some(flags)
}

/// One `MOMT` material.
#[derive(Debug, Clone)]
pub struct WmoMaterial {
    pub flags: u32,
    pub shader: u32,
    /// 0 opaque, 1 alpha-key, 2 alpha blend, 3 additive, 4 add-alpha,
    /// 5 modulate, 6 modulate2x — the same table M2 materials use.
    pub blend_mode: u32,
    /// Index into [`WmoRoot::textures`], or `None` when the material names no
    /// texture (an offset past the end of `MOTX`, which does happen).
    pub texture: Option<u32>,
    /// The second texture, used only by the multi-texture shaders. Parsed so a
    /// later pass can use it; the renderer currently samples `texture` only.
    pub texture2: Option<u32>,
    pub ground_type: u32,
}

impl WmoMaterial {
    pub fn unlit(&self) -> bool {
        self.flags & material_flags::UNLIT != 0
    }

    pub fn two_sided(&self) -> bool {
        self.flags & material_flags::UNCULLED != 0
    }
}

/// One `MOGI` record: what the root says about a group before its file is read.
#[derive(Debug, Clone)]
pub struct WmoGroupInfo {
    pub flags: u32,
    pub bounds: [[f32; 3]; 2],
    /// Byte offset into `MOGN`, or negative for an unnamed group.
    pub name_offset: i32,
}

/// One `MODS` doodad set: a contiguous run of [`WmoRoot::doodads`].
///
/// A `MODF` placement picks exactly one set by index, which is how the same
/// building appears furnished in one spot and empty in another.
#[derive(Debug, Clone)]
pub struct WmoDoodadSet {
    pub name: String,
    pub start: u32,
    pub count: u32,
}

/// One `MODD` doodad spawn, in the WMO's own local space.
#[derive(Debug, Clone)]
pub struct WmoDoodad {
    /// Archive path with `.mdx` already fixed up to `.m2`.
    pub path: String,
    pub position: [f32; 3],
    /// `(x, y, z, w)`, straight from the file and normalised.
    pub rotation: [f32; 4],
    pub scale: f32,
    /// BGRA, and it is **the light this spawn is lit by** — see [`Self::light`].
    pub colour: u32,
    /// Column-major model-to-**WMO-local** matrix. The renderer multiplies the
    /// placement's own matrix by this one; see the doc comment on
    /// [`doodad_matrix`].
    pub matrix: [f32; 16],
    /// **This spawn stands outside and the sun lights it**, not its baked
    /// colour. Not a field of `MODD` — it is resolved by [`WmoModel::assemble`]
    /// from `MODR`, the per-group doodad reference list: a spawn referenced by
    /// an `EXTERIOR` group and by no `INDOOR` one is street furniture — a
    /// lamp-post, a market stall — and its baked colour is the room light of a
    /// room it is not in. Lighting those by the bake is what drew Stormwind's
    /// street lamps at dusk in the middle of the day. A spawn no group
    /// references keeps its bake, which is the conservative side: the colour is
    /// at least the building's own measurement.
    pub exterior_lit: bool,
}

impl WmoDoodad {
    /// **What a chair standing inside a room is lit by**, normalised RGB and
    /// sRGB as the file states it, or `None` for a spawn that names no light.
    ///
    /// A `MODD` spawn is the one thing in the world with no lighting of its own
    /// and no business taking the sun's: it is an ordinary M2, shared with the
    /// trees outside, so it carries no `MOCV`, and it is standing under a roof,
    /// so the sun reaches it through the ceiling — which at midnight is the
    /// difference between a lit tavern and a lit tavern full of black furniture.
    /// This field is the answer the file gives: one baked colour per *spawn*,
    /// sampled where the tools put it, so a crate in a cellar is darker than the
    /// same crate by a window.
    ///
    /// **It is measured before it is believed**, by `vale wmos`: 6,158 of
    /// Stormwind's 6,158 spawns carry one and 2,104 of Darkshire's 2,104, at a
    /// mean peak channel of 133 and 164. A field of zeroes would have meant
    /// falling back to the root's `MOHD` ambient for everything — which
    /// Stormwind states as `11/11/11`, i.e. black, so the fallback is a fallback
    /// and not an alternative.
    ///
    /// The high byte is left alone. Every one of those 8,262 spawns has a
    /// non-zero one and no reading of it produces a difference anyone can point
    /// at, so it is not treated as an opacity or as a "use me" flag; the
    /// *colour* being zero is what says a spawn names no light.
    pub fn light(&self) -> Option<[f32; 3]> {
        if self.colour & 0x00FF_FFFF == 0 {
            return None;
        }
        // `CImVector` again — B, G, R, A — which is the same byte order `MOCV`
        // uses and the reverse of the ambient beside it. See `read_bgra`.
        let channel = |shift: u32| ((self.colour >> shift) & 0xFF) as f32 / 255.0;
        Some([channel(16), channel(8), channel(0)])
    }

    /// Whether there is anything to draw for this spawn.
    ///
    /// A `MODN` entry can be empty in a patched archive. Such a spawn used to be
    /// filtered out of the list at parse time, which silently shifted the
    /// `MODS`/`MODR` indexing of every spawn after it; it stays in the list now
    /// and is skipped here, by the consumers that would load a model for it.
    pub fn drawable(&self) -> bool {
        !self.path.is_empty() && self.path != ".m2"
    }
}

/// The root file: everything about a WMO except its geometry.
#[derive(Debug, Clone)]
pub struct WmoRoot {
    pub version: u32,
    /// How many `_NNN.wmo` group files this WMO has.
    pub group_count: u32,
    pub wmo_id: u32,
    pub flags: u32,
    /// `MOHD`'s bounding box, WMO-local.
    pub bounds: [[f32; 3]; 2],
    /// `MOHD`'s ambient light colour, normalised, RGB.
    ///
    /// This is the light an *interior* group is lit by: there is no sun in a
    /// room, so a batch's colour is this plus its `MOCV` vertex colour. It is
    /// also subtracted from those vertex colours when they are read (see
    /// [`WmoGroup::shaded_colours`]), so the two halves cancel and the file's
    /// own numbers come back out.
    pub ambient: [f32; 3],
    /// Unique texture paths, in the order the materials first mention them.
    pub textures: Vec<String>,
    pub materials: Vec<WmoMaterial>,
    pub group_info: Vec<WmoGroupInfo>,
    pub doodad_sets: Vec<WmoDoodadSet>,
    /// **The lights the building carries** — `MOLT`. Empty for a barn and ten
    /// for the Goldshire inn; see [`WmoLight`], which carries the layout and
    /// the measurement that pinned it. 1.12 reads these for nothing at all — a
    /// WMO is lit by its baked `MOCV` — so drawing anything by them is
    /// `crate::render::lamps`' deviation rather than the game's.
    pub lights: Vec<WmoLight>,
    pub doodads: Vec<WmoDoodad>,
    /// Raw `MOGN` blob; group headers index into it by byte offset.
    group_names: Vec<u8>,
}

/// **The most group files a root may claim.**
///
/// `MOHD`'s group count is a `u32` read straight out of the file, and it is the
/// one count in this module that drives real work rather than a slice: [`load`]
/// both sizes a `Vec` from it and reads that many archive files, one per index.
/// An unbounded one is therefore an unbounded allocation and an unbounded run of
/// archive lookups — which is a window that stops answering, then grows to
/// several gigabytes, then falls over. That is what a damaged read produces:
/// `wow_mpq` recovers an unreadable sector by filling it with zeros and
/// returning success, so a root whose header sector is damaged parses with
/// whatever the surrounding bytes happen to say.
///
/// 1,024 is the bound, and the number under it is measured: over all 816 `.wmo`
/// roots in a 1.12 install the largest is **306**, `stormwind.wmo`. Undercity is
/// 206 and Orgrimmar 144; nothing else reaches a hundred and fifty. So the bound
/// is more than three times the largest thing the game ships and still small
/// enough that the worst case is a thousand archive misses rather than four
/// billion.
///
/// It refuses the whole root rather than clamping, because a header that states
/// an impossible group count is a header whose other fields — the bounds, the
/// ambient colour, the material offsets — came out of the same bytes and are
/// not worth trusting either. A refused root is a building that is not drawn and
/// is reported once, which is this module's documented way to fail.
pub const MAX_GROUPS: u32 = 1024;

impl WmoRoot {
    pub fn parse(buf: &[u8]) -> Result<WmoRoot, AssetError> {
        let mut root = WmoRoot {
            version: 0,
            group_count: 0,
            wmo_id: 0,
            flags: 0,
            bounds: [[0.0; 3]; 2],
            ambient: [0.0; 3],
            textures: Vec::new(),
            materials: Vec::new(),
            lights: Vec::new(),
            group_info: Vec::new(),
            doodad_sets: Vec::new(),
            doodads: Vec::new(),
            group_names: Vec::new(),
        };

        // Materials name their textures by byte offset into MOTX, and MOTX can
        // arrive before or after MOMT depending on nothing in particular. Keep
        // both raw and resolve once the whole file has been walked.
        let mut motx: &[u8] = &[];
        let mut momt: &[u8] = &[];
        let mut modn: &[u8] = &[];
        let mut modd: &[u8] = &[];
        let mut saw_header = false;

        for c in ChunkReader::new(buf) {
            match &c.magic {
                b"MVER" => root.version = chunk::u32_at(c.data, 0),
                b"MOHD" => {
                    saw_header = true;
                    root.group_count = chunk::u32_at(c.data, 0x04);
                    root.ambient = read_argb(c.data, 0x1C);
                    root.wmo_id = chunk::u32_at(c.data, 0x20);
                    root.bounds = [read_vec3(c.data, 0x24), read_vec3(c.data, 0x30)];
                    // Only the low 16 bits are flags; the high half is `numLod`,
                    // which is zero in 1.12.
                    root.flags = chunk::u32_at(c.data, 0x3C) & 0xFFFF;
                }
                b"MOTX" => motx = c.data,
                b"MOMT" => momt = c.data,
                b"MOGN" => root.group_names = c.data.to_vec(),
                b"MOGI" => root.group_info = parse_group_info(c.data),
                b"MOLT" => root.lights = parse_lights(c.data),
                b"MODS" => root.doodad_sets = parse_doodad_sets(c.data),
                b"MODN" => modn = c.data,
                b"MODD" => modd = c.data,
                _ => {}
            }
        }

        if !saw_header {
            return Err(AssetError::malformed("WMO", "no MOHD chunk"));
        }
        if root.version != WMO_VERSION {
            return Err(AssetError::malformed(
                "WMO",
                format!("version {} is not a 1.12 WMO", root.version),
            ));
        }
        // See [`MAX_GROUPS`]. This is the one field of `MOHD` a caller acts on
        // rather than reads, so it is the one that has to be believable.
        if root.group_count > MAX_GROUPS {
            return Err(AssetError::malformed(
                "WMO",
                format!(
                    "{} groups is more than any 1.12 building has ({MAX_GROUPS} is the limit)",
                    root.group_count
                ),
            ));
        }

        let (textures, materials) = parse_materials(momt, motx);
        root.textures = textures;
        root.materials = materials;
        root.doodads = parse_doodad_defs(modd, modn);
        Ok(root)
    }

    /// A group's name from `MOGN`, empty when it has none.
    pub fn group_name(&self, name_offset: i32) -> String {
        if name_offset < 0 {
            return String::new();
        }
        let start = name_offset as usize;
        let Some(rest) = self.group_names.get(start..) else {
            return String::new();
        };
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        String::from_utf8_lossy(&rest[..end]).into_owned()
    }

    /// The doodad spawns belonging to one `MODF` doodad set.
    pub fn doodads_in_set(&self, set: u16) -> &[WmoDoodad] {
        doodads_in_set(&self.doodad_sets, &self.doodads, set)
    }
}

/// One `MOBA` render batch: a slice of the group's index buffer and the root
/// material to draw it with.
#[derive(Debug, Clone)]
pub struct WmoBatch {
    pub index_start: u32,
    pub index_count: u32,
    pub vertex_start: u16,
    pub vertex_end: u16,
    /// Index into [`WmoRoot::materials`]. 0xFF means "none" and the batch is
    /// dropped — despite the field being called `texture` in most references,
    /// it is a *material* index (Noggit: `materials.at(batch.texture)`).
    pub material: Option<u8>,
    /// Which of the three `MOBA` sections this batch is in — and therefore
    /// which lighting law it takes inside an interior group.
    ///
    /// **Resolved at parse time, before the retain below**, for the same reason
    /// [`WmoGroup::interior_start`] is: the class is stated as three *counts*
    /// over the batch list, so dropping a batch first shifts every batch after
    /// it into its neighbour's law — which draws, and draws plausibly.
    pub section: BatchSection,
}

/// Which `MOBA` section a batch belongs to: the list is laid out
/// `transBatchCount` transition batches, then `intBatchCount` interior ones,
/// then the rest exterior, and the three counts are `MOGP` `+0x28`, `+0x2A` and
/// `+0x2C`.
///
/// **This is the whole of how a building's inside meets its outside**, and
/// without it every batch of an indoor group takes one flat law — which is the
/// hard step of darkness at a doorway. Only the group's own geometry states it;
/// nothing on the wire and no other file says a word about it.
///
/// What each section means is under [`WmoDraw::light`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum BatchSection {
    /// The seam: geometry that is neither wholly in the room nor wholly out of
    /// it — the doorway arch, the porch, the window reveal.
    Trans = 0,
    /// Inside. Lit by its own baked `MOCV` and nothing else.
    Int = 1,
    /// Outside, even where the group around it is not. The default, so a file
    /// with no counts at all reads as wholly exterior — the conservative side,
    /// since an exterior law on interior geometry is a lit room and an interior
    /// law on exterior geometry is a black one.
    #[default]
    Ext = 2,
}

/// `MLIQ`'s header, which is 30 bytes and not 32.
///
/// `sizeof` it in C and a compiler pads the trailing `u16` out to 32, which
/// reads the first two liquid vertices as part of the header and shifts the
/// whole grid. vmangos writes the constant out term by term
/// (`WMOLiquidHeaderSize`) rather than using `sizeof` for exactly that reason.
const LIQUID_HEADER_SIZE: usize = 4 * 4 + 3 * 4 + 2;

/// Bytes per `MLIQ` vertex. Two `u16`s this client has no use for, then the
/// height — which is the only field vmangos' extractor keeps either.
/// `pub(crate)` because an `MCLQ` vertex is the same union — see `adt.rs`.
pub(crate) const LIQUID_VERTEX_SIZE: usize = 8;

/// The side of one liquid tile, in yards.
///
/// **Not stated anywhere in the file** — the grid gives a corner and a count and
/// nothing else. It is the game's `CHUNKSIZE / 8`: a map chunk is
/// `533.333/16 = 33.333` yards across and carries an 8x8 liquid grid, and a
/// WMO's `MLIQ` uses the same spacing. Measured rather than assumed — see
/// `vale water`, which reports every liquid grid's overhang past the bounding
/// box of the group that owns it.
pub const LIQUID_TILE_SIZE: f32 = crate::world::adt::TILE_SIZE / 16.0 / 8.0;

/// The four liquids the 1.12 client draws, after [`WmoGroup::liquid_kind`]'s
/// fixup. The numbering is vmangos' and it is *not* `LiquidType.dbc`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liquid {
    Water,
    Ocean,
    Magma,
    Slime,
}

impl Liquid {
    /// All four, in the order the client's own texture table holds them —
    /// `lake_a`, `ocean_h`, `lava`, `slime`, indexed by the same
    /// number [`Self::index`] answers.
    pub const ALL: [Liquid; 4] = [Liquid::Water, Liquid::Ocean, Liquid::Magma, Liquid::Slime];

    /// This liquid's place in [`Self::ALL`], so a caller can key a fixed array
    /// by it rather than a map.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The texture family the client draws this liquid with.
    ///
    /// **From the client**, which is the only authority that has an
    /// opinion: `LiquidType.dbc` in 1.12 is five rows of id, name, type and
    /// spell id and names no texture at all, so the mapping is a table in the
    /// client. The five `XTextures\` format strings it holds are
    /// `river\lake_a.%d.blp`, `river\fast_a.%d.blp`, `ocean\ocean_h.%d.blp`,
    /// `lava\lava.%d.blp` and `slime\slime.%d.blp`; `fast_a` is the rapids
    /// variant, which nothing in 1.12's `MLIQ` selects.
    pub fn texture_pattern(self) -> &'static str {
        match self {
            Liquid::Water => r"XTextures\river\lake_a.%d.blp",
            Liquid::Ocean => r"XTextures\ocean\ocean_h.%d.blp",
            Liquid::Magma => r"XTextures\lava\lava.%d.blp",
            Liquid::Slime => r"XTextures\slime\slime.%d.blp",
        }
    }

    /// One frame of the flipbook, numbered from 1 as the client's `%d` is.
    pub fn texture(self, frame: u32) -> String {
        self.texture_pattern().replace("%d", &frame.to_string())
    }

    /// Every frame of the flipbook, in order, from 1.
    ///
    /// The whole set, because the client loads the whole set: frames 1 to 30,
    /// once per liquid type, latched by a per-type "already loaded" flag.
    pub fn textures(self) -> impl Iterator<Item = String> {
        (1..=LIQUID_FRAMES).map(move |frame| self.texture(frame))
    }

    /// **Which frame of the flipbook is showing**, `elapsed` seconds into a
    /// session — 0-based, so it indexes [`Self::textures`] directly.
    ///
    /// The client's whole rule is four constants long:
    ///
    /// ```text
    /// the type's own period, seconds
    /// × 1000                     -> milliseconds
    /// the millisecond clock mod the period
    /// …as a fraction of the period
    /// × 30                       -> a frame number
    /// − 0.5, then rounded        -> the frame, floored
    /// ```
    ///
    /// The period table holds **1.25** for every one of the four
    /// kinds, so all of them run at the same 24 frames a second; it is read per
    /// type anyway, which is why this is a method rather than a constant.
    ///
    /// **It is the wall clock and not the world's**, which is what makes the
    /// water go on moving while the hour stands still — the game's own hour is
    /// half-minutes and would advance this frame once every thirty seconds.
    pub fn frame_at(self, elapsed: f32) -> u32 {
        let period = self.flipbook_period();
        if period <= 0.0 {
            return 0;
        }
        let phase = (elapsed.rem_euclid(period)) / period;
        // Floored, which is what the `− 0.5` before the round-to-nearest
        // amounts to. Clamped because a phase of exactly 1.0 is
        // reachable through `f32` rounding and the client's own integer
        // modulo cannot produce it.
        ((phase * LIQUID_FRAMES as f32) as u32).min(LIQUID_FRAMES - 1)
    }

    /// How long one lap of the flipbook takes, in seconds.
    pub fn flipbook_period(self) -> f32 {
        match self {
            Liquid::Water | Liquid::Ocean | Liquid::Magma | Liquid::Slime => 1.25,
        }
    }
}

/// How many frames every liquid flipbook has.
///
/// **Thirty**, and both halves of that are the client's: the load loop stops
/// at 30, and the float the phase is multiplied by to turn it into a frame
/// number is `30.0`. Checked
/// against the archives, where `lake_a.30.blp` exists and `lake_a.31.blp` does
/// not, for all four families.
pub const LIQUID_FRAMES: u32 = 30;

/// One group's `MLIQ`: a rectangular grid of liquid over part of the room.
///
/// The grid is `xtiles` by `ytiles` *tiles* and therefore one more than that in
/// vertices each way, laid out from [`Self::base`] along the group's own +X and
/// +Y. Each tile carries a flag byte whose **low nibble of 15 means no liquid
/// there** — which is how a pool is a shape rather than a rectangle, and how a
/// grid that covers the whole room can be almost entirely empty.
#[derive(Debug, Clone)]
pub struct WmoLiquid {
    pub x_tiles: usize,
    pub y_tiles: usize,
    /// The grid's corner, in the group's own model space.
    pub base: [f32; 3],
    /// Raw `MLIQ` type. Resolved by [`WmoGroup::liquid_kind`], which needs the
    /// root and the group flags as well.
    pub liquid_type: u16,
    /// One height per *vertex*, so `(x_tiles + 1) * (y_tiles + 1)` of them, in
    /// row-major order along +X first.
    pub heights: Vec<f32>,
    /// The first byte of each liquid vertex, parallel to [`Self::heights`].
    ///
    /// **For water this is the depth**, and it is the field the surface's
    /// opacity comes from — see [`Self::opacity`]. The 8-byte vertex is a union
    /// the liquid type selects between: water is
    /// `{ u8 depth, u8 flow0Pct, u8 flow1Pct, u8 filler, f32 height }`, magma
    /// and slime are `{ i16 s, i16 t, f32 height }` and carry texture
    /// coordinates here instead. vmangos reads the height and nothing else,
    /// because a server does not draw.
    pub depths: Vec<u8>,
    /// One flag byte per *tile*. See [`Self::has_liquid`].
    pub tile_flags: Vec<u8>,
}

impl WmoLiquid {
    fn parse(data: &[u8]) -> Option<WmoLiquid> {
        if data.len() < LIQUID_HEADER_SIZE {
            return None;
        }
        let count = |offset| chunk::u32_at(data, offset) as usize;
        let (x_verts, y_verts) = (count(0), count(4));
        let (x_tiles, y_tiles) = (count(8), count(12));
        // A grid whose vertex count does not agree with its tile count is not a
        // grid, and the two products below are what index into the payload.
        if x_verts != x_tiles + 1 || y_verts != y_tiles + 1 {
            return None;
        }
        let vertices = x_verts.checked_mul(y_verts)?;
        let tiles = x_tiles.checked_mul(y_tiles)?;
        let heights_end = LIQUID_HEADER_SIZE + vertices * LIQUID_VERTEX_SIZE;
        if data.len() < heights_end + tiles {
            return None;
        }
        Some(WmoLiquid {
            x_tiles,
            y_tiles,
            base: read_vec3(data, 16),
            liquid_type: u16::from_le_bytes([data[28], data[29]]),
            heights: data[LIQUID_HEADER_SIZE..heights_end]
                .chunks_exact(LIQUID_VERTEX_SIZE)
                .map(|v| chunk::f32_at(v, 4))
                .collect(),
            depths: data[LIQUID_HEADER_SIZE..heights_end]
                .chunks_exact(LIQUID_VERTEX_SIZE)
                .map(|v| v[0])
                .collect(),
            tile_flags: data[heights_end..heights_end + tiles].to_vec(),
        })
    }

    /// Is there liquid on this tile? The low nibble is the per-tile type and
    /// **15 means none** — the flag that turns a rectangle into a pool.
    pub fn has_liquid(&self, x: usize, y: usize) -> bool {
        self.tile_flags
            .get(y * self.x_tiles + x)
            .is_some_and(|f| f & 0x0F != 15)
    }

    /// The surface's opacity at one grid vertex, 0..255.
    ///
    /// **Only water and ocean have one.** The 8-byte liquid vertex is a union
    /// the liquid type picks between, and byte 0 is a depth for water but the
    /// low half of an `s` texture coordinate for magma and slime — which is
    /// measured, not assumed, and is the sharpest evidence in the file that the
    /// union is real: over Stormwind, which is entirely water, byte 0 is **0 at
    /// every one of the 1,014 dry vertices**, 86 across 3,183 of the pool
    /// interior, and a scatter of 1..76 around the banks. Over Ironforge, which
    /// is mostly magma, the same byte is spread flat across all 256 values with
    /// no plateau at all — a coordinate, not a depth. Reading it as depth there
    /// would hand a lava pool an opacity that walks with its texture.
    ///
    /// So lava and slime are opaque, which is what their textures already say
    /// (`alphaDepth=0`), and water fades out as it shallows.
    ///
    /// The byte is used **as an 8-bit alpha directly**, which is the reading the
    /// data supports and costs no invented constant: it is exactly zero where
    /// there is no water, so a pool's edge fades to nothing of its own accord,
    /// and it rises to a plateau over the middle. What the plateau *means* — how
    /// opaque deep water should be — is the one part the file does not settle,
    /// and it is a single multiply here if the canal reads too faint.
    pub fn opacity(&self, index: usize, kind: Liquid) -> u8 {
        match kind {
            Liquid::Water | Liquid::Ocean => self.depths.get(index).copied().unwrap_or(u8::MAX),
            Liquid::Magma | Liquid::Slime => u8::MAX,
        }
    }

    /// Does any of the up-to-four tiles meeting at this vertex carry liquid?
    ///
    /// A vertex belongs to no tile of its own — the grid is one larger than the
    /// tile grid in each axis — so a per-vertex quantity is only meaningful
    /// where at least one adjacent tile is wet. The dry ones hold whatever the
    /// authoring tool left behind and must not be averaged in.
    pub fn vertex_is_wet(&self, index: usize) -> bool {
        let stride = self.x_tiles + 1;
        let (vx, vy) = (index % stride, index / stride);
        [
            (vx.wrapping_sub(1), vy.wrapping_sub(1)),
            (vx, vy.wrapping_sub(1)),
            (vx.wrapping_sub(1), vy),
            (vx, vy),
        ]
        .into_iter()
        .any(|(tx, ty)| tx < self.x_tiles && ty < self.y_tiles && self.has_liquid(tx, ty))
    }

    /// Height at a grid *vertex*, model-space.
    pub fn height(&self, x: usize, y: usize) -> f32 {
        self.heights
            .get(y * (self.x_tiles + 1) + x)
            .copied()
            .unwrap_or(self.base[2])
    }

    /// Model-space position of a grid vertex.
    pub fn vertex(&self, x: usize, y: usize) -> [f32; 3] {
        [
            self.base[0] + x as f32 * LIQUID_TILE_SIZE,
            self.base[1] + y as f32 * LIQUID_TILE_SIZE,
            self.height(x, y),
        ]
    }

    /// How many tiles actually carry liquid.
    pub fn wet_tiles(&self) -> usize {
        (0..self.y_tiles)
            .flat_map(|y| (0..self.x_tiles).map(move |x| (x, y)))
            .filter(|&(x, y)| self.has_liquid(x, y))
            .count()
    }
}

/// One group file's geometry.
#[derive(Debug, Clone)]
pub struct WmoGroup {
    pub name: String,
    pub flags: u32,
    /// `MOGP`'s bounding box, WMO-local.
    pub bounds: [[f32; 3]; 2],
    /// Byte offset of the group's name in the root's `MOGN`.
    pub name_offset: i32,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u16>,
    /// `MOPY`'s flag byte, one per *triangle* — so parallel to `indices` in
    /// threes, not to `positions`. Empty when the group has no `MOPY`.
    ///
    /// This is what says whether a triangle is solid; see [`collides`]. It is
    /// kept beside the render geometry rather than resolved here because the
    /// two answers differ: `WmoModel::assemble` drops batches whose material
    /// will not resolve and whole groups that are antiportals, and collision
    /// must not follow it — an invisible collision hull carries no batch at all.
    pub triangle_flags: Vec<u8>,
    pub batches: Vec<WmoBatch>,
    /// `MODR` — which of the root's doodad spawns stand in this group.
    pub doodad_refs: Vec<u16>,
    /// `MOCV` vertex colours, normalised RGBA and parallel to `positions`, or
    /// empty when the group has none.
    ///
    /// **Raw, as they appear in the file.** The client rewrites them before use;
    /// [`Self::shaded_colours`] is that rewrite, and it needs the root.
    pub colours: Vec<[f32; 4]>,
    /// First vertex belonging to a batch that is *not* a transparency batch.
    ///
    /// The colour fixup treats the two halves of the group differently, and the
    /// boundary is stated only as a batch count in the `MOGP` header — so it has
    /// to be resolved against the batch list before any batch is dropped, which
    /// is why it is a field rather than something the caller recomputes.
    pub interior_start: u32,
    /// `MOGP` 0x34 — which liquid the group's `MLIQ` holds, *before* the
    /// client's fixup. Meaningless without [`Self::liquid`].
    pub liquid_type: u32,
    /// **`MOGP` 0x38 — this group's own id, and the third key into
    /// `WMOAreaTable.dbc`.** See [`crate::tables::wmoarea`], which is what turns it into
    /// a place name.
    ///
    /// Measured rather than assumed: `ironforge_000.wmo` reads 3558 and its
    /// neighbours 3559, 3609 and 3698, which land inside the 3549..3652 block
    /// `WMOAreaTable` carries for root 208 — the `wmo_id` `ironforge.wmo`'s own
    /// `MOHD` states. Reading it at 0x34 instead gets `liquid_type` (15 in all
    /// four), which resolves to nothing and would look like a WMO with no areas.
    pub group_id: u32,
    /// `MLIQ` — the water, lava or slime standing in this group, if any.
    pub liquid: Option<WmoLiquid>,
}

impl WmoGroup {
    pub fn parse(buf: &[u8]) -> Result<WmoGroup, AssetError> {
        let mogp = ChunkReader::new(buf)
            .find(|c: &Chunk| c.is(b"MOGP"))
            .ok_or_else(|| AssetError::malformed("WMO group", "no MOGP chunk"))?;
        if mogp.data.len() < MOGP_HEADER_SIZE {
            return Err(AssetError::malformed("WMO group", "MOGP header is short"));
        }

        let mut group = WmoGroup {
            name: String::new(),
            name_offset: chunk::u32_at(mogp.data, 0x00) as i32,
            flags: chunk::u32_at(mogp.data, 0x08),
            bounds: [read_vec3(mogp.data, 0x0C), read_vec3(mogp.data, 0x18)],
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            triangle_flags: Vec::new(),
            batches: Vec::new(),
            doodad_refs: Vec::new(),
            colours: Vec::new(),
            interior_start: 0,
            liquid_type: chunk::u32_at(mogp.data, 0x34),
            group_id: chunk::u32_at(mogp.data, 0x38),
            liquid: None,
        };
        // MOGP 0x28 and 0x2A: how many of this group's batches are *transition*
        // batches and how many are *interior* ones. They come first in the batch
        // list in that order, everything after them is exterior, and the split
        // is the whole of how a room's light meets the daylight — see
        // [`BatchSection`]. The colour fixup below also treats the two halves'
        // vertices differently, which is what `interior_start` resolves.
        let trans_batches = u16::from_le_bytes([mogp.data[0x28], mogp.data[0x29]]) as usize;
        let int_batches = u16::from_le_bytes([mogp.data[0x2A], mogp.data[0x2B]]) as usize;

        // The sub-chunks live inside MOGP's payload, but a file whose MOGP size
        // field is wrong still has them immediately after the header — so read
        // from the header's end to EOF rather than trusting the size. This is
        // what both reference implementations do, for the same reason.
        let start = mogp.payload_offset + MOGP_HEADER_SIZE;
        if start > buf.len() {
            return Err(AssetError::malformed("WMO group", "MOGP runs past EOF"));
        }

        for c in ChunkReader::at(buf, start) {
            match &c.magic {
                b"MOVI" => {
                    group.indices = c
                        .data
                        .chunks_exact(2)
                        .map(|b| u16::from_le_bytes([b[0], b[1]]))
                        .collect();
                }
                b"MOVT" => {
                    group.positions =
                        c.data.chunks_exact(12).map(|v| read_vec3(v, 0)).collect();
                }
                b"MONR" => {
                    group.normals =
                        c.data.chunks_exact(12).map(|v| read_vec3(v, 0)).collect();
                }
                // Only the first UV set is read. `MOTV` appears twice on groups
                // flagged 0x2000000, which is a later-expansion flag; the second
                // set is a lightmap coordinate this renderer has no use for.
                b"MOTV" if group.uvs.is_empty() => {
                    group.uvs = c
                        .data
                        .chunks_exact(8)
                        .map(|v| [chunk::f32_at(v, 0), chunk::f32_at(v, 4)])
                        .collect();
                }
                // Two bytes per triangle — a flag byte and a material index —
                // and only the flags are wanted. The material a *collision*
                // triangle names is the one its batch already carries.
                b"MOPY" => {
                    group.triangle_flags = c.data.chunks_exact(2).map(|b| b[0]).collect();
                }
                b"MOBA" => group.batches = parse_batches(c.data),
                // `MOCV` appears twice on groups that use the second set for
                // texture blending — a later-expansion feature. The first is the
                // lighting one, so keep it and ignore any repeat.
                b"MOCV" if group.colours.is_empty() => {
                    group.colours = c.data.chunks_exact(4).map(read_bgra).collect();
                }
                b"MLIQ" => group.liquid = WmoLiquid::parse(c.data),
                b"MODR" => {
                    group.doodad_refs = c
                        .data
                        .chunks_exact(2)
                        .map(|b| u16::from_le_bytes([b[0], b[1]]))
                        .collect();
                }
                _ => {}
            }
        }

        if group.positions.is_empty() {
            return Err(AssetError::malformed("WMO group", "no MOVT vertices"));
        }
        // Normals and UVs are parallel to the positions; a group missing either
        // still has usable geometry, so pad rather than refuse. A zero normal
        // would normalise to NaN and light the surface black, so pad with up.
        group.normals.resize(group.positions.len(), [0.0, 0.0, 1.0]);
        group.uvs.resize(group.positions.len(), [0.0, 0.0]);
        // A short MOCV is padded rather than discarded, for the same reason: an
        // unlit vertex is better than no room. Transparent black is the
        // identity here — the colours are an additive light term whose alpha is
        // an emissive mask, so a pad vertex must add no light and emit none.
        if !group.colours.is_empty() {
            group.colours.resize(group.positions.len(), [0.0, 0.0, 0.0, 0.0]);
        }

        // Where the transparency batches end. Resolved here, against the batch
        // list as the file wrote it, because the retain below can drop the very
        // batch this reads — and then every interior vertex would take the wrong
        // branch of the fixup, which is a plausible-looking result rather than
        // an error.
        group.interior_start = trans_batches
            .checked_sub(1)
            .and_then(|last| group.batches.get(last))
            .map_or(0, |b| b.vertex_end as u32 + 1);

        // …and each batch's own section, stamped for the same reason and at the
        // same moment: the counts index the list the file wrote, so this has to
        // happen before anything is dropped from it.
        for (i, batch) in group.batches.iter_mut().enumerate() {
            batch.section = if i < trans_batches {
                BatchSection::Trans
            } else if i < trans_batches + int_batches {
                BatchSection::Int
            } else {
                BatchSection::Ext
            };
        }

        // A batch indexing past the index buffer means the offsets are wrong,
        // and drawing it reads whatever geometry happens to follow. Drop the
        // batch, keep the group — the same rule `m2.rs` applies.
        let len = group.indices.len() as u32;
        group
            .batches
            .retain(|b| b.index_count > 0 && b.index_start.saturating_add(b.index_count) <= len);

        Ok(group)
    }

    /// Is this group interior geometry the outdoor sun should not light?
    pub fn is_indoor(&self) -> bool {
        self.flags & group_flags::INDOOR != 0
    }

    /// Is this group lit by the world's sun rather than by its own `MOCV`?
    ///
    /// **The mask is `0x48` and not `0x08`**, which is the client's own — see
    /// [`group_flags::EXTERIOR_LIT`], where the four instructions are. Reading
    /// only `EXTERIOR` puts every group carrying the second bit alone under the
    /// interior law, and those are streets and courtyards: drawn at a bake
    /// authored to be *multiplied* by daylight, they read as permanent dusk in
    /// the middle of the day.
    ///
    /// Note it is still not the absence of `INDOOR` (0x2000): the bits are
    /// independent and a group can carry none of the three.
    pub fn is_exterior(&self) -> bool {
        self.flags & (group_flags::EXTERIOR | group_flags::EXTERIOR_LIT) != 0
    }

    /// `MOCV` vertex colours with the client's fixup applied, ready to light a
    /// room with. Empty when the group has no `MOCV`.
    ///
    /// This is the client's own fixup, and it is the one piece of the
    /// WMO format that cannot be derived from the file alone — it needs the
    /// root's ambient colour and two of its flags. The transcription is Noggit's
    /// (`WMOGroup::fix_vertex_color_alpha`), which is prior art known to render
    /// these files, with two deliberate departures:
    ///
    /// * Noggit's line 1093 reads `r += ((b * a / 64.f) - ...)` where every
    ///   neighbouring line assigns to its own channel. That is a copy-paste
    ///   slip: it drops blue entirely and adds it to red twice. Fixed here.
    /// * the arithmetic runs on **normalised** components, as Noggit's does.
    ///   Blizzard's original works on bytes, where `+ rgb * a / 64` is a gain of
    ///   up to ×5 rather than the ×1.016 it is here — but the original also
    ///   halves the result and doubles it again in the shader, and Noggit drops
    ///   both. Following the renderer that works end to end beats reassembling
    ///   the original from two halves; `vale wmos` prints the resulting
    ///   range so the choice is checkable against real files rather than taken
    ///   on trust.
    ///
    /// **The alpha that comes out is the interior emissive mask, and only
    /// that.** The game's own `MapObjOverbright.bls` — the standard WMO diffuse
    /// pixel shader whenever the `mapObjOverbright` cvar is on — computes `tex * MOCV * (1 + 4 * MOCV.a)`, so an
    /// interior vertex's alpha is an authored self-illumination worth up to
    /// five times the base: hearths, forges, the whitened texels at a portal
    /// seam. Measured on real tiles the channel has exactly that shape —
    /// Stormwind's 486,309 coloured verts are 90% zero / 6% low / 3% mid / 1%
    /// high, Darkshire's 89/7/3/1 (`vale wmos`) — so an interior vertex
    /// keeps its authored alpha here.
    ///
    /// **And so, as of this round, does a transition vertex's** — a different
    /// payload read under a different [`BatchSection`], and the one this
    /// function used to destroy. It is the weight the seam is faded toward the
    /// daylight by ([`BatchLight::Blend`]); Noggit folds `(1 - a)` into the
    /// colour instead, which is that fade collapsed against a *black* lit pass,
    /// and it draws every fully-outdoor vertex of every doorway in the game at
    /// zero. Both halves are in the same channel and neither is a guess about
    /// the other: an interior batch never reads it as a weight and a transition
    /// batch never reads it as an emissive.
    pub fn shaded_colours(&self, root: &WmoRoot) -> Vec<[f32; 4]> {
        let mut colours = self.colours.clone();
        if colours.is_empty() {
            return colours;
        }
        let interior_start = self.interior_start as usize;

        // The flag says the colours are already final. Their alpha still is
        // forced, to the client's own values: 1.0 on an exterior group (whose
        // colours a sun-lit batch never reads anyway) and 0.0 on an interior
        // one — so a DO_NOT_FIX building deliberately gets no emissive term,
        // which is what the client's flagged branch leaves for
        // `MapObjOverbright` to multiply by.
        if root.flags & root_flags::DO_NOT_FIX_VERTEX_COLOUR_ALPHA != 0 {
            let alpha = if self.is_exterior() { 1.0 } else { 0.0 };
            for c in colours.iter_mut().skip(interior_start) {
                c[3] = alpha;
            }
            return colours;
        }

        // Under the unified render path the renderer applies ambient itself, so
        // taking it out here as well would subtract it twice.
        let ambient = if root.flags & root_flags::UNIFIED_RENDER_PATH != 0 {
            [0.0; 3]
        } else {
            root.ambient
        };

        for (i, c) in colours.iter_mut().enumerate() {
            let a = c[3];
            for k in 0..3 {
                c[k] = if i >= interior_start {
                    // Interior: alpha brightens the vertex towards its own lamp.
                    c[k] + c[k] * a / 64.0 - ambient[k]
                } else {
                    // A transition vertex's colour is the same bake, ambient
                    // taken out to be added back in the shader. **Its alpha is
                    // not consumed here**, which is the change this round: it is
                    // the weight the seam is faded toward daylight by, and it
                    // has a batch section of its own to be read under. Folding
                    // `(1 - a)` into the colour — Noggit's collapse, which
                    // assumes the lit half of the two-pass draw is black —
                    // makes a vertex the file marks *fully outdoors* draw as
                    // fully black, and every doorway in the game has one.
                    c[k] - ambient[k]
                }
                .clamp(0.0, 1.0);
            }
        }
        colours
    }

    /// Geometry that exists only to block visibility, never to be drawn.
    pub fn is_antiportal(&self) -> bool {
        self.flags & group_flags::ANTIPORTAL != 0
    }

    /// Should this group be left out of the collision hull entirely?
    ///
    /// The two groups vmangos' `WMOGroup::ShouldSkip` drops before writing a
    /// vmap: an antiportal (invisible, and solid would seal the building shut)
    /// and one flagged unreachable (0x0080 — scenery the player is never meant
    /// to get to, and making it solid turns decoration into an obstacle).
    pub fn is_uncollidable(&self) -> bool {
        self.is_antiportal() || self.flags & group_flags::UNREACHABLE != 0
    }

    /// Which liquid this group's `MLIQ` actually holds.
    ///
    /// **The raw `MOGP` type is not it**, and this is the fixup vmangos'
    /// extractor applies with "according to https://wowdev.wiki" written over
    /// it — a chain of four steps, each of which changes the answer:
    ///
    /// 1. the root's `USE_LIQUID_TYPE_DBC_ID` flag decides whether the group's
    ///    number is a `LiquidType.dbc` id or is one *less* than one;
    /// 2. 15 means "not stated here", and the answer is then taken from the
    ///    first tile in the grid whose own low nibble is not 15 — so a group can
    ///    declare nothing and still be full of water;
    /// 3. the id is folded modulo 4, which is what makes the whole table only
    ///    four liquids wide however many rows the DBC grows to;
    /// 4. and the water/ocean pair is separated by a *group* flag (0x80000), not
    ///    by the type at all — so an ocean that is read as water is a bug no
    ///    number in `MLIQ` can reveal.
    ///
    /// vmangos additionally special-cases `Stratholme_raid` onto Naxxramas'
    /// slime; that is a server-side liquid id with no bearing on what is drawn,
    /// and is not reproduced.
    pub fn liquid_kind(&self, root: &WmoRoot) -> Option<Liquid> {
        let liquid = self.liquid.as_ref()?;
        let mut entry = if root.flags & root_flags::USE_LIQUID_TYPE_DBC_ID != 0 {
            self.liquid_type
        } else if self.liquid_type == 15 {
            0
        } else {
            self.liquid_type + 1
        };
        if entry == 0 {
            // Nothing declared: take the first tile that states one.
            entry = liquid
                .tile_flags
                .iter()
                .map(|f| u32::from(f & 0x0F))
                .find(|&t| t != 15)
                .map_or(0, |t| t + 1);
        }
        if entry == 0 || entry >= 21 {
            return None;
        }
        Some(match (entry - 1) & 3 {
            0 if self.flags & group_flags::LIQUID_IS_OCEAN != 0 => Liquid::Ocean,
            0 => Liquid::Water,
            1 => Liquid::Ocean,
            2 => Liquid::Magma,
            _ => Liquid::Slime,
        })
    }

    /// Is triangle `index` solid? `false` for a group with no `MOPY`, and for
    /// an index past its end — a short `MOPY` is a damaged tail, and inventing
    /// solid geometry from one is worse than leaving a hole.
    pub fn triangle_collides(&self, index: usize) -> bool {
        self.triangle_flags.get(index).is_some_and(|&f| collides(f))
    }

    pub fn triangle_count(&self) -> usize {
        self.batches.iter().map(|b| b.index_count as usize).sum::<usize>() / 3
    }
}

/// The bounding box over two sets of vertices, either of which may be empty.
///
/// Two rather than one because a group's box has to cover the water standing in
/// it as well as its walls: the box becomes a culling sphere, and a canal culled
/// with the room's masonry pops in and out at the room's edge.
fn bounds_of_two(a: &[[f32; 3]], b: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    let mut any = false;
    for v in a.iter().chain(b) {
        any = true;
        for axis in 0..3 {
            lo[axis] = lo[axis].min(v[axis]);
            hi[axis] = hi[axis].max(v[axis]);
        }
    }
    if any {
        (lo, hi)
    } else {
        ([0.0; 3], [0.0; 3])
    }
}

/// One drawable slice of an assembled WMO — the same shape as an [`M2Batch`],
/// so the renderer draws a building with the model shader and no special case.
///
/// [`M2Batch`]: crate::world::m2::M2Batch
#[derive(Debug, Clone)]
pub struct WmoDraw {
    /// Index into [`WmoModel::groups`], so the renderer can cull a room without
    /// walking its batches.
    pub group: u32,
    pub index_start: u32,
    pub index_count: u32,
    /// Index into [`WmoModel::textures`].
    pub texture: Option<u32>,
    pub blend: u32,
    pub unlit: bool,
    pub two_sided: bool,
    /// **Which of the three lighting laws this batch takes**, resolved from the
    /// group's own flags and the batch's [`BatchSection`].
    ///
    /// Decided per *draw* and not per group, which is the whole of this round:
    /// a room and the daylight outside it meet on the batches the file calls
    /// transition, and a group-wide answer has nowhere to put that.
    pub light: BatchLight,
    /// The raw section the law above was derived from, carried beside it so
    /// `vale wmos` can cross the file's own claim against the group flags —
    /// the two are different statements and only one of them is measurable.
    pub section: BatchSection,
    /// Which liquid this is, when it is an `MLIQ` surface rather than masonry.
    ///
    /// **The kind and not a flag, because the colour is looked up by it.** Every
    /// other draw in a WMO takes both its colour and its alpha from the texel,
    /// which is right for masonry and glass and wrong for water twice over:
    /// `lake_a`'s alpha channel is a foam and highlight mask (mean 54 of 255,
    /// three quarters of the texels under 64), and its *colour* channel is not a
    /// colour at all — the brightest texel in the whole image reaches 41 of 255,
    /// greyscale. So the alpha comes off `MLIQ`'s own per-vertex depth (see
    /// [`WmoLiquid::depths`]) and the colour comes out of `Light.dbc`, which is
    /// keyed by this. See [`crate::tables::light`].
    pub liquid: Option<Liquid>,
}

/// **What lights one WMO batch** — three laws, and which one a batch takes is
/// its group's flags crossed with its [`BatchSection`].
///
/// The two ends were always here; the middle one is what a doorway is made of.
/// Without it an indoor group is drawn wholly at its bake and an outdoor one
/// wholly at the sun, and the geometry between them — the arch, the porch, the
/// window reveal — has to pick a side. It picked the bake, which is why walking
/// out of a building crossed a hard step of darkness instead of a gradient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum BatchLight {
    /// The world's daylight. Everything outdoors, and every batch of an indoor
    /// group that the file itself calls exterior.
    #[default]
    Sun,
    /// The room's own baked `MOCV`, plus the root's ambient, and no sun at all —
    /// a ceiling faces down, so a room lit by the sun is a black box.
    Bake,
    /// **The seam**: the same bake, faded per vertex toward the sun-lit surface
    /// by the vertex's own `MOCV` alpha. `MOCV × mix(1, sun, alpha)`.
    ///
    /// The reference draws these batches *twice* — once lit, blended
    /// `SRC_ALPHA`, and once at the bake blended `ONE_MINUS_SRC_ALPHA` — and the
    /// two passes collapse to that one product exactly. Which is why a
    /// transition vertex's alpha must survive [`WmoGroup::shaded_colours`]
    /// rather than being folded into its colour: folded, alpha 1 means *black*
    /// where the file means *full daylight*.
    ///
    /// Two-pass identity aside, the reading of the section table comes from a
    /// D3D capture of the Goldshire inn. What is checked here is the header
    /// layout and the correlation `vale wmos` prints; see that command.
    Blend,
}

/// One group's place in the assembled buffers.
#[derive(Debug, Clone)]
pub struct WmoGroupDraw {
    pub name: String,
    pub flags: u32,
    pub indoor: bool,
    /// The group carries `MOCV` and is not exterior — so it is lit by its own
    /// vertex colours. See [`WmoDraw::vertex_lit`].
    pub vertex_lit: bool,
    /// Range of [`WmoModel::draws`] belonging to this group.
    pub draw_start: u32,
    pub draw_count: u32,
    /// Bounding sphere in WMO-local space, for per-group frustum culling.
    pub centre: [f32; 3],
    pub radius: f32,
    /// The same volume as a box, which is what the sphere was derived from.
    ///
    /// Carried because **a sphere cannot answer "is this character in this
    /// room"** — a room's sphere reaches well outside its walls, and the
    /// question the renderer asks of an *indoor* group is a containment test
    /// rather than a cull. See [`WmoModel::interior_bounds`].
    pub bounds: [[f32; 3]; 2],
    /// `MOGP` 0x38 — see [`WmoGroup::group_id`] and [`WmoModel::area_bounds`].
    pub group_id: u32,
}

impl WmoGroupDraw {
    /// **Is this group a room for *lighting* purposes** — the client's own
    /// `0x48` test, and not [`Self::indoor`].
    ///
    /// The two disagree on most of a city and the disagreement is the whole of
    /// the "randomly dark objects" report: `INDOOR` is what the portal system
    /// culls by, `0x48` is what the renderer shades by, and Stormwind's 115
    /// building shells carry both bits. See [`group_flags::EXTERIOR_LIT`] for
    /// the mask and [`WmoModel::interior_bounds`] for
    /// what asks.
    pub fn is_room(&self) -> bool {
        self.flags & (group_flags::EXTERIOR | group_flags::EXTERIOR_LIT) == 0
    }
}

/// A whole WMO — root plus every group — flattened into one vertex buffer, one
/// index buffer and a list of draws.
///
/// One buffer rather than one per group because that is what the terrain does
/// for the same reason: a group is a *draw range*, not a separate binding, and
/// nine tiles of buildings would otherwise be a few thousand buffer binds a
/// frame. Indices are `u32` because the groups are concatenated — each group's
/// own `MOVI` is `u16` and could not address the whole model.
#[derive(Debug, Clone)]
pub struct WmoModel {
    pub path: String,
    /// `MOHD` 0x20 — the building's own id, and the first key into
    /// `WMOAreaTable.dbc`. See [`crate::tables::wmoarea`].
    pub wmo_id: u32,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Shaded `MOCV` colours as RGBA8, one per vertex — the light inside the
    /// building in RGB, and the authored emissive mask in alpha (see
    /// [`WmoGroup::shaded_colours`]). Groups without `MOCV` contribute
    /// transparent black, the identity for both.
    pub colours: Vec<[u8; 4]>,
    pub indices: Vec<u32>,
    pub textures: Vec<String>,
    /// The root's ambient light, carried through so the renderer can add back
    /// what [`WmoGroup::shaded_colours`] took out.
    pub ambient: [f32; 3],
    pub draws: Vec<WmoDraw>,
    pub groups: Vec<WmoGroupDraw>,
    /// **The lights it states** — `MOLT`, copied from the root. See
    /// [`WmoLight`].
    pub lights: Vec<WmoLight>,
    pub doodad_sets: Vec<WmoDoodadSet>,
    pub doodads: Vec<WmoDoodad>,
    /// Bounding box over the geometry actually assembled, WMO-local. Measured
    /// rather than taken from `MOHD`, because `MOHD`'s box includes groups this
    /// build dropped and the renderer culls against what it will really draw.
    pub bounds: [[f32; 3]; 2],
    /// The building's *solid* triangles, in the same model space as
    /// `positions`.
    ///
    /// Carried on the assembled model rather than left to the caller because
    /// the groups are dropped at the end of [`load`] and a hull cannot be
    /// rebuilt from what survives: it is a different subset of a different set
    /// of chunks. See [`crate::world::collision`] for why the two disagree.
    pub collision: crate::world::collision::CollisionMesh,
    /// `MOHD`'s own box, unmodified.
    ///
    /// Kept beside the measured one because it is the box the game's tools used
    /// when they wrote each `MODF` placement's world bounds, and comparing the
    /// two is how the placement matrix is checked against data this project did
    /// not produce (`vale wmos`).
    pub local_bounds: [[f32; 3]; 2],
    pub radius: f32,
}

impl WmoModel {
    /// Flatten a root and its groups into one renderable model.
    ///
    /// Antiportal groups are skipped: they are invisible visibility blockers,
    /// and a solid black box the size of a cathedral is what drawing them looks
    /// like. Batches whose material does not resolve are skipped too — an
    /// untextured wall is worse than a missing one here, because a WMO's
    /// materials are the only thing that says whether a surface is a window.
    pub fn assemble(path: &str, root: &WmoRoot, groups: &[WmoGroup]) -> WmoModel {
        let mut model = WmoModel {
            path: path.to_string(),
            wmo_id: root.wmo_id,
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            colours: Vec::new(),
            indices: Vec::new(),
            textures: root.textures.clone(),
            ambient: root.ambient,
            draws: Vec::new(),
            groups: Vec::new(),
            doodad_sets: root.doodad_sets.clone(),
            lights: root.lights.clone(),
            doodads: root.doodads.clone(),
            // Built from every group, including the ones the loop below skips:
            // an antiportal is not solid, but a group whose every batch has an
            // unresolvable material still has walls.
            collision: crate::world::collision::CollisionMesh::build(groups),
            bounds: [[f32::MAX; 3], [f32::MIN; 3]],
            local_bounds: root.bounds,
            radius: 0.0,
        };

        // **Which spawns the sun lights, off `MODR`.** Each group lists the
        // spawns standing in it, and the group's own flags say which light it is
        // under — so a spawn referenced only by outdoor groups is street
        // furniture and takes the sun, while one any *room* claims keeps its
        // baked colour (a rug in a doorway is referenced by both, and the room's
        // answer is the one its colour was baked for). Walked over *all* groups,
        // including ones the draw loop below skips: whether a room's batches
        // resolve has no bearing on where its furniture stands.
        //
        // **The question is [`WmoGroup::is_exterior`] and never `INDOOR`.**
        // Those are two different bits answering two different questions —
        // `0x2000` is the *portal* system's, and `0x48` is the light's — and a
        // group carrying both is common enough to be most of a city:
        // Stormwind's 115 building shells (`BH01`, `DW02`, `ClockTower`,
        // `NightElf01` — they name themselves in `MOGN`) are `INDOOR` for
        // culling and exterior-lit for shading, and they carry no `MOCV` at all,
        // which is the file saying the sun does their walls. Keyed on `INDOOR`,
        // every crate, barrel, lamp and cart those groups list was drawn at a
        // flat baked colour in the open air — dark at noon and bright at
        // midnight, because a bake does not move with the sun. That is the
        // "randomly dark objects" report, and it is the same wrong bit the batch
        // law was fixed for one round earlier.
        let mut indoor_ref = vec![false; model.doodads.len()];
        for group in groups {
            for &spawn in &group.doodad_refs {
                let Some(doodad) = model.doodads.get_mut(spawn as usize) else {
                    continue;
                };
                if group.is_exterior() {
                    if !indoor_ref[spawn as usize] {
                        doodad.exterior_lit = true;
                    }
                } else {
                    indoor_ref[spawn as usize] = true;
                    doodad.exterior_lit = false;
                }
            }
        }

        for group in groups {
            // A group with no batches used to be skipped outright. It cannot be
            // any more: `MLIQ` is a sub-chunk of the *group*, and a room whose
            // only content is the water standing in it draws nothing else.
            if group.is_antiportal() || (group.batches.is_empty() && group.liquid.is_none()) {
                continue;
            }
            let group_index = model.groups.len() as u32;
            let vertex_base = model.positions.len() as u32;
            let index_base = model.indices.len() as u32;
            let draw_start = model.draws.len() as u32;

            model.positions.extend_from_slice(&group.positions);
            model.normals.extend_from_slice(&group.normals);
            model.uvs.extend_from_slice(&group.uvs);
            model
                .indices
                .extend(group.indices.iter().map(|&i| vertex_base + i as u32));

            // One colour per vertex whether or not this group has any, so the
            // buffer stays parallel to the positions across the concatenation.
            // A group that skipped its share would shift every later group's
            // colours by its own vertex count. Transparent black, not opaque:
            // the RGB is an additive light term and the alpha an emissive
            // mask, and the identity for both is zero.
            let colours = group.shaded_colours(root);
            let vertex_lit = !colours.is_empty() && !group.is_exterior();
            if colours.is_empty() {
                model.colours.resize(model.positions.len(), [0, 0, 0, 0]);
            } else {
                model.colours.extend(colours.iter().map(to_rgba8));
            }

            for batch in &group.batches {
                let Some(material) = batch
                    .material
                    .and_then(|m| root.materials.get(m as usize))
                else {
                    continue;
                };
                // The group answers whether there is a room light to be had at
                // all; the batch's own section answers which law it takes
                // inside one. An exterior group is the sun whatever its
                // sections say — the counts are still written on one, and they
                // describe the drawing order rather than the light.
                let light = if !vertex_lit {
                    BatchLight::Sun
                } else {
                    match batch.section {
                        BatchSection::Int => BatchLight::Bake,
                        BatchSection::Trans => BatchLight::Blend,
                        BatchSection::Ext => BatchLight::Sun,
                    }
                };
                model.draws.push(WmoDraw {
                    group: group_index,
                    index_start: index_base + batch.index_start,
                    index_count: batch.index_count,
                    texture: material.texture,
                    blend: material.blend_mode,
                    unlit: material.unlit(),
                    two_sided: material.two_sided(),
                    light,
                    section: batch.section,
                    liquid: None,
                });
            }

            // The water, lava or slime standing in the room, appended to the
            // same buffers and emitted as one more draw. It goes through the
            // model material like everything else here — see `add_liquid`.
            let liquid_start = model.positions.len();
            model.add_liquid(group, root, group_index);

            // Measured from the vertices rather than taken from MOGP's box: the
            // declared box is generous on some groups, and a sphere derived from
            // a generous box culls nothing. The liquid counts: a canal that is
            // culled with the room's walls pops in and out at the room's edge.
            let (lo, hi) = bounds_of_two(&group.positions, &model.positions[liquid_start..]);
            let centre = [
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            ];
            let radius = distance(centre, hi);
            for axis in 0..3 {
                model.bounds[0][axis] = model.bounds[0][axis].min(lo[axis]);
                model.bounds[1][axis] = model.bounds[1][axis].max(hi[axis]);
            }

            model.groups.push(WmoGroupDraw {
                name: group.name.clone(),
                flags: group.flags,
                indoor: group.is_indoor(),
                vertex_lit,
                draw_start,
                draw_count: model.draws.len() as u32 - draw_start,
                centre,
                radius,
                bounds: [lo, hi],
                group_id: group.group_id,
            });
        }

        if model.positions.is_empty() {
            model.bounds = [[0.0; 3]; 2];
        }
        let centre = [
            (model.bounds[0][0] + model.bounds[1][0]) / 2.0,
            (model.bounds[0][1] + model.bounds[1][1]) / 2.0,
            (model.bounds[0][2] + model.bounds[1][2]) / 2.0,
        ];
        model.radius = distance(centre, model.bounds[1]);
        model
    }

    /// **Where the inside of this building is**, one box per indoor group, in
    /// WMO-local space.
    ///
    /// This is the whole of what an entity's interiority test needs. A `MODD`
    /// spawn says where it is standing and so needs no test at all — its light
    /// comes with it — but a *player* walks in and out of the same room, and
    /// nothing in any file or any packet says which side of the door they are
    /// on. The building's own idea of what a room is, is `MOGP`'s exterior pair
    /// being clear ([`WmoGroupDraw::is_room`]) and the geometry that group
    /// covers.
    ///
    /// **`INDOOR` is the wrong bit here and used to be the one asked**, which
    /// put every entity standing against a Stormwind building shell under the
    /// room light — those 115 groups carry `INDOOR` for the portal system and
    /// `0x40` for the sun, and their boxes are the outsides of houses. See the
    /// `MODR` walk in [`WmoModel::assemble`], which had the same wrong bit for
    /// the same reason.
    ///
    /// **A box and not the culling sphere**, because those are answering
    /// different questions: a sphere over a long room reaches half way across
    /// the street, and a test that says "indoors" outside the wall lights a
    /// character standing in the sun by a tavern's lamps.
    ///
    /// Loose in one direction on purpose: an L-shaped room's box covers the
    /// corner it does not occupy, and a group's box includes its own walls. Both
    /// err toward calling a doorway indoors, which is the side to err on — the
    /// cost is a stride of wrong light at a threshold, against a whole room of
    /// it.
    pub fn interior_bounds(&self) -> Vec<[[f32; 3]; 2]> {
        self.groups
            .iter()
            .filter(|group| group.is_room())
            .map(|group| group.bounds)
            .collect()
    }

    /// **…and where each of its *areas* is** — the same boxes, every group and
    /// not only the indoor ones, each with the `MOGP` id
    /// [`crate::tables::wmoarea::WmoAreas`] looks a place name up by.
    ///
    /// **Not a filter on `indoor`, and that is the whole point of it being a
    /// second method.** Stormwind's districts — Trade District, Old Town, the
    /// Mage Quarter — are `EXTERIOR` groups of one open-air WMO, so the
    /// interiority test cannot answer where you are standing in a city. Ironforge
    /// is the other way round and both are the same lookup.
    ///
    /// It inherits [`Self::interior_bounds`]' looseness and one more of its own:
    /// a city's district boxes **overlap**, and the first containing box wins.
    /// The alternative is the portal graph (`MOPT`/`MOPR`), which this client
    /// does not read.
    pub fn area_bounds(&self) -> Vec<([[f32; 3]; 2], u32)> {
        self.groups
            .iter()
            .map(|group| (group.bounds, group.group_id))
            .collect()
    }

    /// **What the rooms in this building light the things standing in them
    /// by**, as one colour: the mean of the `MODD` lights of one doodad set.
    ///
    /// The furniture gets a light *per spawn* and is right to
    /// ([`WmoDoodad::light`]); an entity cannot, because it moves and the file
    /// samples nothing where it is standing. The nearest honest answer is what
    /// the building's own tools measured for the things that do not move, and
    /// the mean of those is the room's general brightness.
    ///
    /// **Not `MOHD`'s ambient, which is the obvious candidate and is black.**
    /// Stormwind states `11/11/11` — see [`WmoRoot::ambient`] — so lighting a
    /// player by it would put them in a darker room than the chair beside them,
    /// which is the bug this is fixing pointing the other way. The ambient is
    /// still the fallback for a set whose spawns name no light at all, and for a
    /// building with no furniture in it.
    pub fn room_light(&self, set: u16) -> [f32; 3] {
        let spawns = match self.doodad_sets.get(set as usize) {
            Some(s) => {
                let start = (s.start as usize).min(self.doodads.len());
                let end = start
                    .saturating_add(s.count as usize)
                    .min(self.doodads.len());
                &self.doodads[start..end]
            }
            None => &[][..],
        };
        mean_light(spawns, self.ambient)
    }
}

/// The mean of a run of `MODD` spawns' own lights, or `fallback` where none of
/// them names one. See [`WmoModel::room_light`], which is this over one doodad
/// set; the renderer calls it directly, because by the time a placement is
/// spawned the model has been reduced to its GPU assets and only the spawns
/// survive.
pub fn mean_light(spawns: &[WmoDoodad], fallback: [f32; 3]) -> [f32; 3] {
    let mut total = [0.0f32; 3];
    let mut count = 0.0f32;
    // The sun-lit spawns sit outside the rooms this mean stands for — a
    // lamp-post in the street should not brighten the tavern behind it.
    for light in spawns
        .iter()
        .filter(|s| !s.exterior_lit)
        .filter_map(WmoDoodad::light)
    {
        for axis in 0..3 {
            total[axis] += light[axis];
        }
        count += 1.0;
    }
    if count == 0.0 {
        return fallback;
    }
    total.map(|c| c / count)
}

/// Whether a point is inside an axis-aligned box, both in the same space.
///
/// A free function because the two callers hold the box and the point in
/// different types — the renderer transforms the entity into the building's own
/// frame rather than transforming eight corners the other way, since a `MODF`
/// matrix is a rotation and a uniform scale and its inverse is exact.
pub fn box_contains(bounds: &[[f32; 3]; 2], point: [f32; 3]) -> bool {
    (0..3).all(|axis| point[axis] >= bounds[0][axis] && point[axis] <= bounds[1][axis])
}

impl WmoModel {

    /// Append one group's `MLIQ` surface to the model's buffers, as one draw.
    ///
    /// **Water goes through the model material like a wall does**, which is the
    /// same decision the buildings themselves are drawn under and it buys the
    /// same thing: no second shader, no second pass, no second cache. Four
    /// parameters carry the whole of what makes it water rather than masonry,
    /// and each is read off the file rather than chosen:
    ///
    /// * the **texture** is one frame of the flipbook the 1.12 client names for
    ///   this liquid ([`Liquid::texture`]) — appended to the model's own texture
    ///   list, so the renderer loads it with the building's;
    /// * the **blend mode** follows the texture's alpha: `lake_a` and `ocean_h`
    ///   are `alphaDepth=8` and blend, `lava` and `slime` are `alphaDepth=0` and
    ///   are opaque, which is exactly right and is the file's decision and not
    ///   this client's;
    /// * **unlit** for lava and slime, which glow, and sun-lit for water;
    /// * **two-sided**, because a water surface is seen from underneath as soon
    ///   as anything swims, and because that removes any question about which way
    ///   round a generated quad is wound.
    ///
    /// Only the *wet* tiles are emitted. The grid is a rectangle and the pool is
    /// not: Stormwind's canals are 2,885 wet tiles of 3,905, and emitting the
    /// dry ones would floor every room the grid overlaps with water.
    ///
    /// **Static, for now.** The client cycles thirty frames; this draws frame 1.
    /// A flipbook is a material swap per frame across every liquid draw in the
    /// world, which is a batching question rather than a parsing one.
    fn add_liquid(&mut self, group: &WmoGroup, root: &WmoRoot, group_index: u32) {
        let Some(kind) = group.liquid_kind(root) else {
            return;
        };
        let liquid = group.liquid.as_ref().expect("liquid_kind implies MLIQ");

        let texture = kind.texture(1);
        let texture_index = match self.textures.iter().position(|t| *t == texture) {
            Some(index) => index as u32,
            None => {
                self.textures.push(texture);
                self.textures.len() as u32 - 1
            }
        };

        let start = self.indices.len() as u32;
        for y in 0..liquid.y_tiles {
            for x in 0..liquid.x_tiles {
                if !liquid.has_liquid(x, y) {
                    continue;
                }
                let base = self.positions.len() as u32;
                // One texture repeat per tile, which is what makes the surface
                // read at the size the art was drawn for; the corners are whole
                // numbers either way, so this is the same as UVs taken from the
                // grid indices.
                for (corner, uv) in [
                    ((x, y), [0.0, 0.0]),
                    ((x + 1, y), [1.0, 0.0]),
                    ((x + 1, y + 1), [1.0, 1.0]),
                    ((x, y + 1), [0.0, 1.0]),
                ] {
                    self.positions.push(liquid.vertex(corner.0, corner.1));
                    self.normals.push([0.0, 0.0, 1.0]);
                    self.uvs.push(uv);
                    // Black, so the additive light term the vertex colours
                    // otherwise carry is the identity — this surface is not lit
                    // by `MOCV`. **The alpha is the payload**: it is the
                    // surface's opacity, off `MLIQ`'s own per-vertex depth, and
                    // the reason a liquid draw carries vertex colours at all.
                    let index = corner.1 * (liquid.x_tiles + 1) + corner.0;
                    self.colours.push([0, 0, 0, liquid.opacity(index, kind)]);
                }
                self.indices
                    .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }

        let count = self.indices.len() as u32 - start;
        if count == 0 {
            return;
        }
        let translucent = matches!(kind, Liquid::Water | Liquid::Ocean);
        self.draws.push(WmoDraw {
            group: group_index,
            // Already absolute: a `MOBA` offset is group-local and has to have
            // the group's base added, but these indices were pushed straight
            // into the model's own buffer.
            index_start: start,
            index_count: count,
            texture: Some(texture_index),
            blend: if translucent { 2 } else { 0 },
            unlit: !translucent,
            two_sided: true,
            // Not `MOCV`-lit — the sun lights water — but the vertex colours are
            // kept all the same, because their *alpha* is the opacity. See
            // [`WmoDraw::liquid`].
            light: BatchLight::Sun,
            section: BatchSection::Ext,
            liquid: Some(kind),
        });
    }

    pub fn triangle_count(&self) -> usize {
        self.draws.iter().map(|d| d.index_count as usize).sum::<usize>() / 3
    }

    /// The doodad spawns belonging to one `MODF` doodad set. See
    /// [`WmoRoot::doodads_in_set`].
    pub fn doodads_in_set(&self, set: u16) -> &[WmoDoodad] {
        doodads_in_set(&self.doodad_sets, &self.doodads, set)
    }
}

/// Slice the spawn list by a set's range, clamped to what is actually there.
///
/// Out-of-range sets and runs that overhang the list are empty rather than a
/// panic: a `MODF` in a patched archive can name a set the root no longer has.
fn doodads_in_set<'a>(
    sets: &[WmoDoodadSet],
    doodads: &'a [WmoDoodad],
    set: u16,
) -> &'a [WmoDoodad] {
    let Some(s) = sets.get(set as usize) else {
        return &[];
    };
    let start = (s.start as usize).min(doodads.len());
    let end = start.saturating_add(s.count as usize).min(doodads.len());
    &doodads[start..end]
}

/// Read a WMO and all its group files, and flatten them into one model.
///
/// A group that will not read is **skipped, not fatal**: a WMO is dozens of
/// files, and one damaged room should cost that room rather than the whole
/// cathedral. The names of the groups that failed come back alongside the model
/// so a caller can report them once instead of per frame.
pub fn load(assets: &mut crate::Assets, path: &str) -> Result<(WmoModel, Vec<String>), AssetError> {
    let root = WmoRoot::parse(&assets.read(path)?)?;

    // **Not `with_capacity(group_count)`.** Sizing an allocation from a number a
    // file states is the shape of the fault [`MAX_GROUPS`] is about, and it buys
    // nothing here: the groups are pushed one at a time behind an archive read
    // each, so the growth is free beside the read. The bound above is what makes
    // the *loop* finite; this is what stops the allocation preceding it.
    let mut groups = Vec::new();
    let mut failed = Vec::new();
    for index in 0..root.group_count {
        let group_file = group_path(path, index);
        let parsed = assets
            .read(&group_file)
            .and_then(|raw| WmoGroup::parse(&raw));
        match parsed {
            Ok(mut group) => {
                // The name lives in the *root*'s MOGN, so it can only be filled
                // in here, where both halves are in hand.
                group.name = root.group_name(group.name_offset);
                groups.push(group);
            }
            Err(e) => failed.push(format!("{group_file}: {e}")),
        }
    }

    Ok((WmoModel::assemble(path, &root, &groups), failed))
}

/// Path of one group file: `Building.wmo` + index 3 -> `Building_003.wmo`.
///
/// The suffix is inserted before the extension, not appended — `Building.wmo_003`
/// is in no archive.
pub fn group_path(root_path: &str, index: u32) -> String {
    let suffix = format!("_{index:03}.wmo");
    match root_path.rfind('.') {
        Some(dot) if root_path[dot..].eq_ignore_ascii_case(".wmo") => {
            format!("{}{suffix}", &root_path[..dot])
        }
        _ => format!("{root_path}{suffix}"),
    }
}

/// One `MOPR` record: a group's reference to a portal and to the group on the
/// portal's other side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WmoPortalRef {
    /// Index into [`WmoPortals::portals`].
    pub portal: u16,
    /// The group on the other side; `0xFFFF` names none.
    pub group: u16,
    /// Which side of the portal's plane the owning group is on.
    pub side: i16,
}

/// A root's portal graph, WMO-local: `MOPV` vertices, `MOPT` portals and `MOPR`
/// references. A group's own references are the slice
/// [`WmoGroupHeader::portal_start`]..+[`WmoGroupHeader::portal_count`] of
/// [`Self::refs`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WmoPortals {
    pub vertices: Vec<[f32; 3]>,
    /// `MOPT`, 20 bytes each: the first vertex and the vertex count. The plane
    /// that follows them is not read.
    pub portals: Vec<(u16, u16)>,
    /// `MOPR`, 8 bytes each.
    pub refs: Vec<WmoPortalRef>,
}

impl WmoPortals {
    /// Read the three chunks out of a root file. A root without them (most
    /// small buildings have no portals) gives an empty graph.
    pub fn parse(root: &[u8]) -> WmoPortals {
        let mut out = WmoPortals::default();
        let u16_at = |b: &[u8], at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
        for c in ChunkReader::new(root) {
            match &c.magic {
                b"MOPV" => out.vertices = c.data.chunks_exact(12).map(|v| read_vec3(v, 0)).collect(),
                b"MOPT" => {
                    out.portals = c.data.chunks_exact(20).map(|p| (u16_at(p, 0), u16_at(p, 2))).collect();
                }
                b"MOPR" => {
                    out.refs = c
                        .data
                        .chunks_exact(8)
                        .map(|r| WmoPortalRef {
                            portal: u16_at(r, 0),
                            group: u16_at(r, 2),
                            side: u16_at(r, 4) as i16,
                        })
                        .collect();
                }
                _ => {}
            }
        }
        out
    }

    /// One portal's polygon, or `None` for an index or a vertex range the file
    /// does not hold.
    pub fn polygon(&self, portal: u16) -> Option<&[[f32; 3]]> {
        let &(start, count) = self.portals.get(usize::from(portal))?;
        let start = usize::from(start);
        self.vertices.get(start..start + usize::from(count))
    }
}

/// The part of a group file's `MOGP` header that does not need the geometry:
/// the flags, the box and the group's slice of the root's portal references.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WmoGroupHeader {
    pub flags: u32,
    /// WMO-local, `[min, max]`.
    pub bounds: [[f32; 3]; 2],
    /// `MOGP` 0x24: the first of this group's `MOPR` records.
    pub portal_start: u16,
    /// `MOGP` 0x26: how many there are.
    pub portal_count: u16,
}

impl WmoGroupHeader {
    /// `None` for a file with no `MOGP` or a short header.
    pub fn parse(buf: &[u8]) -> Option<WmoGroupHeader> {
        let mogp = ChunkReader::new(buf).find(|c: &Chunk| c.is(b"MOGP"))?;
        let data = mogp.data;
        if data.len() < MOGP_HEADER_SIZE {
            return None;
        }
        Some(WmoGroupHeader {
            flags: chunk::u32_at(data, 0x08),
            bounds: [read_vec3(data, 0x0C), read_vec3(data, 0x18)],
            portal_start: u16::from_le_bytes([data[0x24], data[0x25]]),
            portal_count: u16::from_le_bytes([data[0x26], data[0x27]]),
        })
    }
}

/// The model-to-WMO-local matrix for one `MODD` spawn, column-major.
///
/// `translate(position) * rotate(quaternion) * scale`, with the quaternion used
/// exactly as it appears in the file. That last part is worth stating because
/// Noggit appears to disagree: it builds `inverse(quat(w, -x, -z, y))`. Its
/// world space is the file's under the permutation `(x, y, z) -> (x, z, -y)`,
/// and conjugating a quaternion by that permutation is precisely
/// `(w, x, y, z) -> (w, x, z, -y)` — which is what its `inverse` of the negated
/// components comes out as. In the file's own axes, which is where this crate
/// works, the two cancel and the raw quaternion is the answer.
pub fn doodad_matrix(position: [f32; 3], rotation: [f32; 4], scale: f32) -> [f32; 16] {
    let [x, y, z, w] = rotation;
    let len = (x * x + y * y + z * z + w * w).sqrt();
    // A zero quaternion is not a rotation; identity is the only safe reading.
    let (x, y, z, w) = if len > 1e-6 {
        (x / len, y / len, z / len, w / len)
    } else {
        (0.0, 0.0, 0.0, 1.0)
    };

    let r = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ];

    let mut m = [0.0f32; 16];
    for col in 0..3 {
        for row in 0..3 {
            m[col * 4 + row] = r[row][col] * scale;
        }
    }
    m[12] = position[0];
    m[13] = position[1];
    m[14] = position[2];
    m[15] = 1.0;
    m
}

/// `a * b`, both column-major 4x4. Used to fold a doodad's local matrix into
/// its building's placement.
pub fn mul4(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
        }
    }
    out
}

/// A `CImVector`: four bytes in **B, G, R, A** order, normalised to RGBA.
///
/// Note the asymmetry with [`read_argb`] four bytes away in the same header.
/// The format uses two different colour structs — `CImVector` here and `CArgb`
/// for `MOHD`'s ambient — and they disagree about byte order. Reading one with
/// the other's layout swaps red and blue, which on a lighting term looks like a
/// cold room rather than like an error.
fn read_bgra(b: &[u8]) -> [f32; 4] {
    [
        b[2] as f32 / 255.0,
        b[1] as f32 / 255.0,
        b[0] as f32 / 255.0,
        b[3] as f32 / 255.0,
    ]
}

/// A `CArgb`: four bytes in **R, G, B, A** order. Alpha is dropped — nothing
/// reads the ambient light's alpha, and the client explicitly zeroes it before
/// use so it cannot disturb a vertex colour's own.
fn read_argb(buf: &[u8], at: usize) -> [f32; 3] {
    let byte = |i: usize| buf.get(i).copied().unwrap_or(0) as f32 / 255.0;
    [byte(at), byte(at + 1), byte(at + 2)]
}

fn to_rgba8(c: &[f32; 4]) -> [u8; 4] {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [byte(c[0]), byte(c[1]), byte(c[2]), byte(c[3])]
}

fn read_vec3(buf: &[u8], at: usize) -> [f32; 3] {
    [
        chunk::f32_at(buf, at),
        chunk::f32_at(buf, at + 4),
        chunk::f32_at(buf, at + 8),
    ]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// `MOMT` + `MOTX`: 64 bytes per material, each naming its textures by **byte
/// offset** into the `MOTX` string blob rather than by index.
///
/// The offsets are collapsed to a deduplicated path list here, because the
/// frontend wants an index it can use as an array subscript and several
/// materials routinely share one texture.
fn parse_materials(momt: &[u8], motx: &[u8]) -> (Vec<String>, Vec<WmoMaterial>) {
    let mut textures: Vec<String> = Vec::new();
    let mut by_offset: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();

    let mut resolve = |offset: u32| -> Option<u32> {
        let start = offset as usize;
        let rest = motx.get(start..)?;
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        Some(*by_offset.entry(offset).or_insert_with(|| {
            textures.push(String::from_utf8_lossy(&rest[..end]).into_owned());
            textures.len() as u32 - 1
        }))
    };

    let materials = momt
        .chunks_exact(MATERIAL_SIZE)
        .map(|m| WmoMaterial {
            flags: chunk::u32_at(m, 0x00),
            shader: chunk::u32_at(m, 0x04),
            blend_mode: chunk::u32_at(m, 0x08),
            texture: resolve(chunk::u32_at(m, 0x0C)),
            texture2: resolve(chunk::u32_at(m, 0x18)),
            ground_type: chunk::u32_at(m, 0x20),
        })
        .collect();

    (textures, materials)
}

fn parse_group_info(data: &[u8]) -> Vec<WmoGroupInfo> {
    data.chunks_exact(GROUP_INFO_SIZE)
        .map(|g| WmoGroupInfo {
            flags: chunk::u32_at(g, 0x00),
            bounds: [read_vec3(g, 0x04), read_vec3(g, 0x10)],
            name_offset: chunk::u32_at(g, 0x1C) as i32,
        })
        .collect()
}

/// **One light a building carries** — `MOLT`, and the game's own answer to
/// "why is that window lit".
///
/// ## The layout
///
/// `World\\wmo\\Azeroth\\Buildings\\GoldshireInn\\GoldshireInn.wmo` declares
/// `nLights = 10` in `MOHD` and a `MOLT` of **480 bytes**, so the record is 48.
/// Reading it as
///
/// ```text
/// 0x00 u8      kind          0 omni, 1 spot, 2 direct, 3 ambient
/// 0x01 u8      use_atten
/// 0x02 u8[2]   pad
/// 0x04 BGRA    colour
/// 0x08 f32[3]  position, in the building's own space
/// 0x14 f32     intensity
/// 0x18 f32[4]  unread
/// 0x28 f32     attenuation start, yards
/// 0x2C f32     attenuation end, yards
/// ```
///
/// gives all ten as `kind = 0`, `use_atten = 1`, `intensity = 1.0`, and
/// **`start` 5.0..5.9 against `end` 7.0..9.5** — ten records in a row where the
/// near distance is under the far one and both are plausible yards for a lamp
/// in a room. The four unread floats are the same `(-0, 0, -1, -0.5)` in every
/// one of them, which is what a field nothing writes looks like. Getting the
/// offsets wrong by one float breaks the start/end ordering immediately, which
/// is why that is the check rather than the count.
///
/// The colours read warm, which is the other confirmation that this is what it
/// looks like: `255/255/160` for the ceiling lamps and `255/193/96` for the two
/// over the hearth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WmoLight {
    /// 0 omni, 1 spot, 2 direct, 3 ambient. Every light in the buildings
    /// measured so far is 0, and the renderer draws nothing else.
    pub kind: u8,
    /// Whether the two distances below are read at all.
    pub use_atten: bool,
    /// Its colour, 0..1, **sRGB as the file states it** — the space every other
    /// band in this project is summed in. `MOLT` stores BGRA.
    pub colour: [f32; 3],
    /// Where it hangs, in the building's own space and axes.
    pub position: [f32; 3],
    pub intensity: f32,
    /// Where the falloff begins and ends, in yards.
    pub atten_start: f32,
    pub atten_end: f32,
}

impl WmoLight {
    /// **Is this a lamp?** — an omni light with a falloff, which is what every
    /// one measured so far is. A spot, a directional or an ambient light is a
    /// different fixture and the renderer has nothing to do with it yet.
    pub fn is_point(&self) -> bool {
        self.kind == 0 && self.use_atten && self.atten_end > self.atten_start
    }
}

/// `MOLT`: one 48-byte record per light. See [`WmoLight`] for the layout and
/// the measurement behind it.
fn parse_lights(data: &[u8]) -> Vec<WmoLight> {
    data.chunks_exact(0x30)
        .map(|r| WmoLight {
            kind: r[0],
            use_atten: r[1] != 0,
            // BGRA on disk, and the alpha is not an opacity — no light in the
            // measured set has anything but 255 in it.
            colour: [
                f32::from(r[6]) / 255.0,
                f32::from(r[5]) / 255.0,
                f32::from(r[4]) / 255.0,
            ],
            position: read_vec3(r, 0x08),
            intensity: chunk::f32_at(r, 0x14),
            atten_start: chunk::f32_at(r, 0x28),
            atten_end: chunk::f32_at(r, 0x2C),
        })
        .collect()
}

#[cfg(test)]
mod light_tests {
    use super::*;

    /// **The record's shape, from the ten in the Goldshire inn.**
    ///
    /// The bytes are the first of them verbatim. What this pins is the thing
    /// that would otherwise be a plausible guess: the two attenuation distances
    /// sit at 0x28 and 0x2C, *after* four floats nothing reads — not
    /// immediately after `intensity`, which is where the obvious layout puts
    /// them and where reading them yields `-0.0` and `0.0`.
    #[test]
    fn a_molt_record_is_forty_eight_bytes_and_its_falloff_is_at_the_end() {
        let mut r = vec![0u8; 0x30];
        r[0] = 0; // omni
        r[1] = 1; // attenuated
        r[4..8].copy_from_slice(&[160, 255, 255, 255]); // BGRA
        r[0x08..0x14].copy_from_slice(&[
            0x00, 0x00, 0x12, 0xC1, // -9.125
            0x00, 0x00, 0x90, 0xBF, // -1.125
            0x00, 0x00, 0x8F, 0x40, // 4.469
        ]);
        r[0x14..0x18].copy_from_slice(&1.0f32.to_le_bytes());
        r[0x18..0x28].copy_from_slice(&[0u8; 16]);
        r[0x28..0x2C].copy_from_slice(&5.0f32.to_le_bytes());
        r[0x2C..0x30].copy_from_slice(&6.972f32.to_le_bytes());

        let lights = parse_lights(&r);
        assert_eq!(lights.len(), 1, "one 48-byte record");
        let l = lights[0];
        assert_eq!(l.kind, 0);
        assert!(l.use_atten);
        // BGRA on disk, so the warm channel is the *last* byte of the four.
        assert_eq!(l.colour, [1.0, 1.0, 160.0 / 255.0]);
        assert_eq!(l.intensity, 1.0);
        assert_eq!(l.atten_start, 5.0);
        assert!((l.atten_end - 6.972).abs() < 1e-4);
        // …and the ordering that is the whole check on the offsets: read one
        // float early and this fails.
        assert!(l.atten_start < l.atten_end);
        assert!(l.is_point());
    }

    /// A record that is not a lamp is not treated as one — the guard the
    /// renderer takes, since only `kind` 0 is a point light.
    #[test]
    fn only_an_attenuated_omni_light_is_a_lamp() {
        let lamp = WmoLight {
            kind: 0,
            use_atten: true,
            colour: [1.0, 1.0, 1.0],
            position: [0.0; 3],
            intensity: 1.0,
            atten_start: 5.0,
            atten_end: 7.0,
        };
        assert!(lamp.is_point());
        assert!(!WmoLight { kind: 1, ..lamp }.is_point(), "a spot is not");
        assert!(!WmoLight { kind: 3, ..lamp }.is_point(), "nor an ambient");
        assert!(!WmoLight { use_atten: false, ..lamp }.is_point());
        // A falloff that ends where it starts lights nothing.
        assert!(!WmoLight { atten_end: 5.0, ..lamp }.is_point());
    }

    /// A truncated tail yields the whole records and drops the fragment, which
    /// is `chunks_exact`'s own behaviour and the rule this parser follows
    /// everywhere — see `ChunkReader`.
    #[test]
    fn a_damaged_tail_costs_the_last_light_and_not_the_building() {
        let mut r = vec![0u8; 0x30 + 7];
        r[1] = 1;
        r[0x2C..0x30].copy_from_slice(&7.0f32.to_le_bytes());
        assert_eq!(parse_lights(&r).len(), 1);
    }
}

fn parse_doodad_sets(data: &[u8]) -> Vec<WmoDoodadSet> {
    data.chunks_exact(DOODAD_SET_SIZE)
        .map(|s| {
            let end = s[..20].iter().position(|&b| b == 0).unwrap_or(20);
            WmoDoodadSet {
                name: String::from_utf8_lossy(&s[..end]).into_owned(),
                start: chunk::u32_at(s, 20),
                count: chunk::u32_at(s, 24),
            }
        })
        .collect()
}

/// `MODD`: 40 bytes per spawn. The first word packs a 24-bit byte offset into
/// `MODN` with 8 bits of flags — not an index, an offset, which is why `MODN`
/// has to be kept whole rather than split into a name list.
fn parse_doodad_defs(modd: &[u8], modn: &[u8]) -> Vec<WmoDoodad> {
    modd.chunks_exact(DOODAD_DEF_SIZE)
        .map(|d| {
            let name_offset = (chunk::u32_at(d, 0) & 0x00FF_FFFF) as usize;
            let name = modn
                .get(name_offset..)
                .map(|rest| {
                    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
                    String::from_utf8_lossy(&rest[..end]).into_owned()
                })
                .unwrap_or_default();
            let position = read_vec3(d, 0x04);
            let rotation = [
                chunk::f32_at(d, 0x10),
                chunk::f32_at(d, 0x14),
                chunk::f32_at(d, 0x18),
                chunk::f32_at(d, 0x1C),
            ];
            let scale = chunk::f32_at(d, 0x20);
            WmoDoodad {
                // `MODN` names models with the pre-1.0 extension, exactly as the
                // creature DBCs do.
                path: crate::world::m2::model_path(&name),
                position,
                rotation,
                scale,
                colour: chunk::u32_at(d, 0x24),
                matrix: doodad_matrix(position, rotation, scale),
                // Resolved from `MODR` once the groups are read; see
                // `WmoModel::assemble`.
                exterior_lit: false,
            }
        })
        // **A spawn with no name is kept, not filtered.** Every index into this
        // list is the file's own — `MODS` slices it by start/count and `MODR`
        // references it by position — so dropping a row here would shift every
        // spawn after it onto its neighbour's slot. A pathless spawn is skipped
        // where it would be *drawn* instead (see [`WmoDoodad::drawable`]).
        .collect()
}

/// `MOBA`: 24 bytes per batch — a 12-byte integer bounding box nobody needs,
/// then the index range, the vertex range, a flag byte and the material.
fn parse_batches(data: &[u8]) -> Vec<WmoBatch> {
    data.chunks_exact(BATCH_SIZE)
        .map(|b| WmoBatch {
            index_start: chunk::u32_at(b, 0x0C),
            index_count: u16::from_le_bytes([b[0x10], b[0x11]]) as u32,
            vertex_start: u16::from_le_bytes([b[0x12], b[0x13]]),
            vertex_end: u16::from_le_bytes([b[0x14], b[0x15]]),
            material: (b[0x17] != 0xFF).then_some(b[0x17]),
            section: BatchSection::default(),
        })
        .collect()
}

#[cfg(test)]
mod tests;
