//! **The little round map**: which picture covers which tile, how far it sees,
//! and where a world position lands inside it.
//!
//! Three rules, none of them on the wire and none of them stated by any file
//! this project already reads — so all three are the client's own.
//!
//! ## The pictures are keyed by an MD5, not by a name
//!
//! The world's minimap art is one 256x256 BLP per ADT tile, and it does **not**
//! live under a readable path. `textures\Minimap\md5translate.trs` is a plain
//! text index shipped beside it:
//!
//! ```text
//! dir: Azeroth
//! Azeroth\map32_48.blp  <TAB>  7fc0d0b4d9b3f6e9c1a0…blp
//! ```
//!
//! …and the file that actually exists is `textures\Minimap\<that md5>.blp`.
//! `MINIMAPCHUNKNOTFOUND|%d|%d` and `No minimap texture: "%s"` are the client's
//! own two complaints about a miss, which is what says the lookup can
//! legitimately fail: **the
//! index is sparse**, and a tile with no picture is black rather than an error.
//!
//! The `dir:` name is the `Map.dbc` directory — the same string
//! `MapTerrain::directory` answers with — and `map<col>_<row>` is the ADT's own
//! numbering, so [`crate::world::terrain::tile_for_position`] is the join.
//!
//! ## How far it sees is a table of chunks, and the indoor one is yards
//!
//! The whole rule:
//!
//! ```text
//!   is the character indoors?
//!     yes: the indoor zoom level indexes a table of floats — the radius, in yards
//!     no:  the outdoor zoom level indexes a table of ints — a count of chunks
//!          * 0.5      — a diameter into a radius
//!          * 33.3333  — chunks into yards
//! ```
//!
//! ```text
//! zoom          0      1      2      3      4      5
//! outdoor  ×½×33⅓  14     12     10      8      6      4   chunks
//!             →   233.3  200.0  166.7  133.3  100.0   66.7  yards
//! indoor          150    120     90     60     40     25    yards
//! ```
//!
//! Both tables are the client's own; the two
//! multipliers are `0.5` and `33.33333`, which is [`crate::world::adt::CHUNK_SIZE`].
//! There are **six** zoom levels, which is what
//! `Minimap:GetZoomLevels()` answers — and the client clamps a `SetZoom` to 5.
//! The `minimapZoom` CVar's default is the string `"3"`, so a
//! session opens at a 133-yard radius outdoors.
//!
//! ## …and the shape it is cut to is a hard disc
//!
//! `Textures\MinimapMask` is 64x64 DXT3, and its alpha bucketed by
//! radius from the centre reads
//!
//! ```text
//! r <= 29   255       every texel
//! r 30..31  34..255   the one transitional ring
//! r >= 32   0
//! ```
//!
//! — a circle inscribed in the square, one texel of rim, no feathering. The
//! same measurement `Interface\CharacterFrame\TempPortraitAlphaMask` gives, and
//! the same conclusion: cutting the quad to a disc is not an approximation of
//! the mask, it *is* the mask, and it costs no second texture.
//!
//! ## Inside a building the map is the building's own pictures
//!
//! While the character stands in a group of a WMO, the 1.12.1 client draws
//! none of the terrain. It draws that WMO's pictures over a black disc, and
//! the radius comes from the indoor table and the indoor zoom level
//! (`minimapInsideZoom`). The pictures are also in `md5translate.trs`, one per
//! cell of a grid over each group:
//!
//! ```text
//! WMO\Dungeon\MD_Goldmine\MD_Goldmine_003_01_00.blp   <TAB>  <md5>.blp
//!     the WMO path without `World\` and `.wmo`, then _<group:03>_<x:02>_<y:02>
//! ```
//!
//! [`group_tiles`] is the grid, [`interior_query`] the box the client searches
//! with, and [`interior_pictures`] the group walk that chooses what is drawn.
//! [`draw_order`] is the order they are drawn in. A picture lies flat in the
//! WMO's own frame: texel column 0 is the cell's least `x`, row 0 its greatest
//! `y`.

use std::collections::HashMap;

use crate::world::adt::{MAP_ORIGIN, TILE_SIZE};
use crate::world::wmo::{group_flags, WmoGroupHeader, WmoPortals};

/// The index that says which picture covers which tile.
pub const MD5_TRANSLATE: &str = r"textures\Minimap\md5translate.trs";

/// …and the directory the pictures themselves are in.
pub const TEXTURE_DIR: &str = r"textures\Minimap";

/// How many zoom levels the minimap has.
pub const ZOOM_LEVELS: usize = 6;

/// …and which one a session starts on: the `minimapZoom` CVar's shipped default
/// (the string `"3"`).
pub const DEFAULT_ZOOM: usize = 3;

/// The **outdoor** radii, as the client's table states them: a count of
/// `MCNK` chunks across the *diameter*.
const OUTDOOR_CHUNKS: [f32; ZOOM_LEVELS] = [14.0, 12.0, 10.0, 8.0, 6.0, 4.0];

/// …and the **indoor** ones, which the client's table states in yards
/// outright — already a radius, with no arithmetic on top.
const INDOOR_YARDS: [f32; ZOOM_LEVELS] = [150.0, 120.0, 90.0, 60.0, 40.0, 25.0];

/// **How far the minimap sees, in yards from the character**.
///
/// A zoom past the last level is clamped rather than refused, which is the
/// client's own handling (it writes `5` over anything larger before it ever
/// reaches this table).
pub fn radius_yards(zoom: usize, indoors: bool) -> f32 {
    let zoom = zoom.min(ZOOM_LEVELS - 1);
    if indoors {
        INDOOR_YARDS[zoom]
    } else {
        // `* 0.5` then `* 33.3333` — a diameter in chunks into a radius in yards.
        OUTDOOR_CHUNKS[zoom] * 0.5 * crate::world::adt::CHUNK_SIZE
    }
}

/// One tile's picture, and where it lands in the widget.
///
/// The rectangle is in the widget's own `0..1` space, **y down** — the space a
/// painter draws in — and it is deliberately allowed to fall outside `0..1` on
/// every side: a tile is 533 yards and the view is at most 466 across, so the
/// common case is two or four tiles each showing a corner. Whoever draws it
/// clips; whoever tests it does not have to reason about the clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileView {
    /// The ADT column and row — [`crate::world::terrain::tile_for_position`]'s pair.
    pub tile: (u32, u32),
    /// Where this tile's whole picture sits: `(left, top, right, bottom)`.
    pub rect: [f32; 4],
}

/// **Which tiles a view covers, and where each one goes** — the geometry half of
/// the minimap, with no archive and no window.
///
/// `x` and `y` are the character's world position and `radius` is
/// [`radius_yards`]'s. The view is the square that circumscribes the circle,
/// because that is what has to be sampled; the disc is cut out of it afterwards.
///
/// **North is up and the view does not turn.** 1.12 registers no `rotateMinimap`
/// CVar, where `minimapZoom` and `minimapInsideZoom` are both registered —
/// so the only rotation
/// on this widget is the arrow standing in the middle of it.
///
/// **And a picture's own `(0, 0)` texel is its tile's north-west corner,
/// measured** rather than assumed. The same tile states its water twice — once
/// as `MCLQ`'s grid of wet chunks and once as the picture's own blue — so
/// `vale minimap <Map>` correlates the two under all four readings of the
/// file. Voted over 25 coastline tiles it is north-west first **on Azeroth by
/// 18.8 points of mean agreement and 18 of 25 tiles, and on Kalimdor by 9.7 and
/// 12 of 25**, against east-west mirrored, north-south mirrored and transposed.
/// It is worth a check of its own because a mirrored minimap draws, moves and
/// reads as a map: a picture cannot settle it.
pub fn tiles_in_view(x: f32, y: f32, radius: f32) -> Vec<TileView> {
    if radius.is_nan() || radius <= 0.0 || !x.is_finite() || !y.is_finite() {
        return Vec::new();
    }
    let span = radius * 2.0;
    // The two world extremes of the square, in each axis. North (+x) and west
    // (+y) are the top and left of a north-up map.
    let (north, south) = (x + radius, x - radius);
    let (west, east) = (y + radius, y - radius);
    let (left_col, top_row) = crate::world::terrain::tile_for_position(north, west);
    let (right_col, bottom_row) = crate::world::terrain::tile_for_position(south, east);

    let mut out = Vec::new();
    for row in top_row..=bottom_row {
        for col in left_col..=right_col {
            // This tile's own north-west corner, which is where its picture's
            // (0, 0) texel sits — see the module comment.
            let tile_north = MAP_ORIGIN - row as f32 * TILE_SIZE;
            let tile_west = MAP_ORIGIN - col as f32 * TILE_SIZE;
            let left = (west - tile_west) / span;
            let top = (north - tile_north) / span;
            out.push(TileView {
                tile: (col, row),
                rect: [
                    left,
                    top,
                    left + TILE_SIZE / span,
                    top + TILE_SIZE / span,
                ],
            });
        }
    }
    out
}

/// `textures\Minimap\md5translate.trs`, parsed.
#[derive(Debug, Default)]
pub struct MinimapTiles {
    /// Keyed by `(lowercased map directory, column, row)`, because the `.trs`
    /// spells `Azeroth` and a caller may hold `AZEROTH` — the archives are
    /// case-insensitive and this index has to be too.
    tiles: HashMap<(String, u32, u32), String>,
    /// The WMO pictures, keyed by `(lowercased stem, group, x, y)`; the stem is
    /// [`interior_stem`]'s.
    interiors: HashMap<(String, u32, u32, u32), String>,
}

impl MinimapTiles {
    /// Parse the index. **Never fails**: a chain without the file, or with a
    /// damaged one, yields an empty index, and an empty index is a black
    /// minimap rather than a dead one — which is the client's own answer to
    /// `MINIMAPCHUNKNOTFOUND` and not a new degradation.
    ///
    /// The file is `dir: <name>` lines interleaved with tab-separated pairs, CRLF
    /// throughout. A pair's left half already carries the directory, so the
    /// `dir:` lines are not needed to read it — but they are what says the file
    /// is the shape this expects, and a line that is neither is skipped rather
    /// than guessed at.
    pub fn parse(raw: &[u8]) -> MinimapTiles {
        let text = String::from_utf8_lossy(raw);
        let mut tiles = HashMap::new();
        let mut interiors = HashMap::new();
        for line in text.lines() {
            let line = line.trim_end_matches(['\r', '\n']);
            let Some((path, file)) = line.split_once('\t') else {
                continue;
            };
            let file = file.trim();
            if file.is_empty() {
                continue;
            }
            let Some((dir, name)) = path.rsplit_once('\\') else {
                continue;
            };
            // A terrain picture first; a name that is not `map<col>_<row>` may
            // be a WMO's `<stem>_<group>_<x>_<y>`.
            if let Some((col, row)) = parse_tile_name(name) {
                tiles.insert((dir.to_ascii_lowercase(), col, row), file.to_string());
            } else if let Some((stem, group, x, y)) = parse_interior_name(path) {
                interiors.insert((stem, group, x, y), file.to_string());
            }
        }
        MinimapTiles { tiles, interiors }
    }

    /// The archive path of one WMO picture: the cell `(x, y)` of group `group`
    /// of the WMO whose [`interior_stem`] is `stem`. `None` for a cell the index
    /// does not carry, which the client logs and leaves black.
    pub fn interior_texture(&self, stem: &str, group: u32, x: u32, y: u32) -> Option<String> {
        let key = (normalise_stem(stem), group, x, y);
        self.interiors
            .get(&key)
            .map(|file| format!("{TEXTURE_DIR}\\{file}"))
    }

    /// How many WMO pictures the index carries.
    pub fn interior_len(&self) -> usize {
        self.interiors.len()
    }

    /// Every WMO stem the index carries pictures for, sorted.
    pub fn interior_stems(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .interiors
            .keys()
            .map(|(stem, ..)| stem.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        names.sort();
        names
    }

    /// Every `(group, x, y)` the index carries for one stem, sorted.
    pub fn interior_cells(&self, stem: &str) -> Vec<(u32, u32, u32)> {
        let stem = normalise_stem(stem);
        let mut out: Vec<(u32, u32, u32)> = self
            .interiors
            .keys()
            .filter(|(s, ..)| *s == stem)
            .map(|(_, group, x, y)| (*group, *x, *y))
            .collect();
        out.sort_unstable();
        out
    }

    /// **The archive path of one tile's picture**, or `None` for a tile the
    /// index does not carry — which is most of the world, since only tiles a map
    /// actually ships get one.
    pub fn texture(&self, directory: &str, col: u32, row: u32) -> Option<String> {
        let key = (directory.to_ascii_lowercase(), col, row);
        self.tiles
            .get(&key)
            .map(|file| format!("{TEXTURE_DIR}\\{file}"))
    }

    /// How many tiles the index carries, for the checks.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The map directories it names, sorted — the census `vale minimap`
    /// prints, and the one thing that says whether the `dir:` names really are
    /// `Map.dbc`'s.
    pub fn directories(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .tiles
            .keys()
            .map(|(dir, _, _)| dir.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        names.sort();
        names
    }

    /// Every tile one directory carries, sorted — for the same check.
    pub fn tiles_of(&self, directory: &str) -> Vec<(u32, u32)> {
        let directory = directory.to_ascii_lowercase();
        let mut out: Vec<(u32, u32)> = self
            .tiles
            .keys()
            .filter(|(dir, _, _)| *dir == directory)
            .map(|(_, col, row)| (*col, *row))
            .collect();
        out.sort_unstable();
        out
    }
}

/// `map32_48.blp` -> `(32, 48)`, and `None` for anything else in the file.
fn parse_tile_name(name: &str) -> Option<(u32, u32)> {
    let name = name.strip_suffix(".blp").unwrap_or(name);
    let rest = name.strip_prefix("map").or_else(|| name.strip_prefix("Map"))?;
    let (col, row) = rest.split_once('_')?;
    Some((col.parse().ok()?, row.parse().ok()?))
}

/// `WMO\…\GoldshireInn_003_01_00.blp` -> `("wmo\…\goldshireinn", 3, 1, 0)`, and
/// `None` for a name without three numeric fields after the stem.
fn parse_interior_name(path: &str) -> Option<(String, u32, u32, u32)> {
    let lower = path.to_ascii_lowercase();
    let name = lower.strip_suffix(".blp").unwrap_or(&lower);
    let mut fields = name.rsplitn(4, '_');
    let y = fields.next()?;
    let x = fields.next()?;
    let group = fields.next()?;
    let stem = fields.next()?;
    let number = |field: &str| {
        (!field.is_empty() && field.bytes().all(|b| b.is_ascii_digit()))
            .then(|| field.parse::<u32>().ok())
            .flatten()
    };
    Some((normalise_stem(stem), number(group)?, number(x)?, number(y)?))
}

/// A stem as the index stores it: lower case, backslashes.
fn normalise_stem(stem: &str) -> String {
    stem.replace('/', "\\").to_ascii_lowercase()
}

/// **The `.trs` stem of a WMO's pictures**, from the WMO's archive path: the
/// path with its first six characters (`World\`) and everything from the first
/// `.` after them removed, so `World\wmo\Dungeon\MD_Goldmine\MD_Goldmine.wmo`
/// is `wmo\dungeon\md_goldmine\md_goldmine`. The 1.12.1 client removes the six
/// characters without checking what they are; every `MODF` path starts with
/// `World\`.
pub fn interior_stem(wmo_path: &str) -> String {
    let rest = wmo_path.get(6..).unwrap_or("");
    let stem = rest.split('.').next().unwrap_or(rest);
    normalise_stem(stem)
}

/// Yards one texel of a WMO picture covers.
pub const INTERIOR_YARDS_PER_TEXEL: f32 = 0.5;

/// The widest a WMO picture is, in texels, and so the cell a large group is cut
/// into: 256 texels at half a yard is 128 yards.
const INTERIOR_MAX_TEXELS: u32 = 256;

/// …and the narrowest.
const INTERIOR_MIN_TEXELS: u32 = 32;

/// How far a group's outermost cells are widened past the group's box on its
/// outer edges, in yards.
const INTERIOR_EDGE: f32 = 2.0 * INTERIOR_YARDS_PER_TEXEL;

/// The most pictures one search returns; the client's list holds 512.
pub const INTERIOR_MOST: usize = 512;

/// One cell of a group's picture grid, in the WMO's own frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InteriorTile {
    /// The cell's column (along the WMO's `x`) and row (along its `y`), the
    /// `<x>` and `<y>` of its picture's name.
    pub x: u32,
    pub y: u32,
    /// The rectangle the picture is stretched over, `[[x, y] min, [x, y] max]`.
    /// Cells on the group's outer edges are widened by [`INTERIOR_EDGE`] on
    /// those edges.
    pub rect: [[f32; 2]; 2],
    /// The height the cell sits at: the middle of the group's box.
    pub z: f32,
}

/// One axis of the grid: the picture's edge in yards and the cell count.
///
/// The picture is the smallest power of two of texels at
/// [`INTERIOR_YARDS_PER_TEXEL`] that covers the extent, clamped to 32..256, and
/// the count is the extent over 128 yards rounded up. Both roundings go up. An
/// extent of under half a yard asks for a negative power, which the 1.12.1
/// client turns into the widest picture; it is reproduced rather than tidied.
fn grid_axis(extent: f32) -> (f32, u32) {
    let texels = extent / (INTERIOR_MAX_TEXELS as f32 * INTERIOR_YARDS_PER_TEXEL)
        * INTERIOR_MAX_TEXELS as f32;
    let power = f64::from(texels).log2().ceil();
    // The shift uses the power's low five bits, as an x86 shift count does,
    // and a non-finite power (an extent of zero) converts to `i32::MIN`.
    let power = if power.is_finite() { power as i32 } else { i32::MIN };
    let picture = 1u32 << (power as u32 & 31);
    let texels = if picture >= INTERIOR_MAX_TEXELS {
        INTERIOR_MAX_TEXELS
    } else {
        picture.max(INTERIOR_MIN_TEXELS)
    };
    let tiles = (extent / (INTERIOR_MAX_TEXELS as f32 * INTERIOR_YARDS_PER_TEXEL)).ceil();
    let count = if tiles.is_finite() && tiles > 0.0 { (tiles as u32) & 0xFF } else { 0 };
    (texels as f32 * INTERIOR_YARDS_PER_TEXEL, count)
}

/// **The picture grid over one group's box**, rows outer and columns inner,
/// which is the order the 1.12.1 client lists them in.
pub fn group_tiles(bounds: &[[f32; 3]; 2]) -> Vec<InteriorTile> {
    let [min, max] = *bounds;
    let (width, columns) = grid_axis(max[0] - min[0]);
    let (height, rows) = grid_axis(max[1] - min[1]);
    let z = (min[2] + max[2]) * 0.5;
    let mut out = Vec::with_capacity((columns * rows) as usize);
    for y in 0..rows {
        for x in 0..columns {
            let mut lo = [min[0] + x as f32 * width, min[1] + y as f32 * height];
            let mut hi = [lo[0] + width, lo[1] + height];
            if x == 0 {
                lo[0] -= INTERIOR_EDGE;
            }
            if y == 0 {
                lo[1] -= INTERIOR_EDGE;
            }
            if x + 1 == columns {
                hi[0] += INTERIOR_EDGE;
            }
            if y + 1 == rows {
                hi[1] += INTERIOR_EDGE;
            }
            out.push(InteriorTile { x, y, rect: [lo, hi], z });
        }
    }
    out
}

/// **The world box the client searches for pictures**, from the character's
/// world position and the indoor radius `radius`.
///
/// The position is snapped to a grid of `radius`-sized cells, so the set of
/// pictures changes only when the character crosses a cell edge or the zoom
/// changes. Around the cell `[x0, x0 + radius]` the box reaches one radius
/// west and south of the cell and one past its far side, which covers the
/// view from anywhere in the cell. Vertically it reaches one and a half radii
/// below the feet and one above, so a floor below shows through a stairwell
/// and a floor far above does not.
pub fn interior_query(position: [f32; 3], radius: f32) -> [[f32; 3]; 2] {
    let snap = |v: f32| {
        let k = 1.0 / radius;
        ((v * k) - 0.5).round_ties_even() * radius
    };
    let (x0, y0) = (snap(position[0]), snap(position[1]));
    let z = position[2];
    [
        [x0 - radius, y0 - radius, z - 1.5 * radius],
        [x0 + 2.0 * radius, y0 + 2.0 * radius, z + radius],
    ]
}

/// A world box carried into a WMO's frame: the box around the eight corners,
/// each through `world_to_local` (column-major, the inverse of the placement).
pub fn box_to_local(world: &[[f32; 3]; 2], world_to_local: &[f32; 16]) -> [[f32; 3]; 2] {
    let m = world_to_local;
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for corner in 0..8 {
        let p = [
            world[corner & 1][0],
            world[(corner >> 1) & 1][1],
            world[(corner >> 2) & 1][2],
        ];
        for axis in 0..3 {
            let v = m[axis] * p[0] + m[4 + axis] * p[1] + m[8 + axis] * p[2] + m[12 + axis];
            lo[axis] = lo[axis].min(v);
            hi[axis] = hi[axis].max(v);
        }
    }
    [lo, hi]
}

/// What one WMO's interior map needs: its stem, every group's header (`None`
/// for a group file that could not be read) and the root's portal graph.
#[derive(Debug, Clone, Default)]
pub struct InteriorModel {
    pub stem: String,
    pub groups: Vec<Option<WmoGroupHeader>>,
    pub portals: WmoPortals,
}

impl InteriorModel {
    /// Read the root and every group file's header. `None` when the root is
    /// not in the archives or is not a WMO.
    pub fn load(path: &str, mut read: impl FnMut(&str) -> Option<Vec<u8>>) -> Option<InteriorModel> {
        let raw = read(path)?;
        let root = crate::world::wmo::WmoRoot::parse(&raw).ok()?;
        let portals = WmoPortals::parse(&raw);
        let groups = (0..root.group_count)
            .map(|index| {
                read(&crate::world::wmo::group_path(path, index))
                    .and_then(|raw| WmoGroupHeader::parse(&raw))
            })
            .collect();
        Some(InteriorModel { stem: interior_stem(path), groups, portals })
    }

    /// **The group a point stands in**, given the flags of the floor under it:
    /// among the groups whose box holds the point (WMO-local) and whose flags
    /// are the floor's, the one with the smallest box. Boxes overlap where
    /// rooms sit inside a district, and the flags are what tie the answer to
    /// the floor the character is on.
    pub fn group_at(&self, local: [f32; 3], floor_flags: u32) -> Option<u32> {
        let volume = |b: &[[f32; 3]; 2]| (0..3).map(|a| (b[1][a] - b[0][a]).max(0.0)).product::<f32>();
        self.groups
            .iter()
            .enumerate()
            .filter_map(|(index, group)| Some((index as u32, group.as_ref()?)))
            .filter(|(_, group)| group.flags == floor_flags)
            .filter(|(_, group)| (0..3).all(|a| local[a] >= group.bounds[0][a] && local[a] <= group.bounds[1][a]))
            .min_by(|a, b| volume(&a.1.bounds).total_cmp(&volume(&b.1.bounds)))
            .map(|(index, _)| index)
    }
}

/// One picture to draw: which cell of which group, and where, WMO-local.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InteriorPicture {
    pub group: u32,
    pub tile: InteriorTile,
}

/// **Which pictures are drawn**: a walk through the WMO's portals from the
/// group the character stands in, inside the WMO-local search box `query`
/// ([`interior_query`] through [`box_to_local`]).
///
/// From each group reached:
///
/// * an `EXTERIOR` (0x8) group, or one already visited, ends the walk there;
/// * a group whose box misses the search box in `x` and `y` ends it too, and
///   gives no pictures;
/// * otherwise its cells ([`group_tiles`]) that meet the search box in `x` and
///   `y` are listed, unless the group is `UNREACHABLE` (0x80);
/// * then each portal of the group is crossed, in `MOPR` order, when its other
///   side is a group other than the one the walk came from and its polygon is
///   not wholly outside any one face of the search box. That test is in three
///   dimensions, and it is the one place the height of the box matters.
///
/// The list is in walk order and stops at [`INTERIOR_MOST`].
pub fn interior_pictures(model: &InteriorModel, seed: u32, query: &[[f32; 3]; 2]) -> Vec<InteriorPicture> {
    let mut out = Vec::new();
    let mut visited = vec![false; model.groups.len()];
    walk(model, seed, seed, query, &mut visited, &mut out);
    out
}

fn walk(
    model: &InteriorModel,
    group: u32,
    from: u32,
    query: &[[f32; 3]; 2],
    visited: &mut [bool],
    out: &mut Vec<InteriorPicture>,
) {
    let Some(Some(header)) = model.groups.get(group as usize) else {
        return;
    };
    if header.flags & group_flags::EXTERIOR != 0 || visited[group as usize] {
        return;
    }
    visited[group as usize] = true;
    let [qmin, qmax] = *query;
    let [gmin, gmax] = header.bounds;
    if !(qmin[0] <= gmax[0] && qmin[1] <= gmax[1] && qmax[0] >= gmin[0] && qmax[1] >= gmin[1]) {
        return;
    }
    if header.flags & (group_flags::EXTERIOR | group_flags::UNREACHABLE) == 0 {
        for tile in group_tiles(&header.bounds) {
            let [lo, hi] = tile.rect;
            let meets = qmin[0] <= hi[0] && qmin[1] <= hi[1] && qmax[0] >= lo[0] && qmax[1] >= lo[1];
            if meets && out.len() < INTERIOR_MOST {
                out.push(InteriorPicture { group, tile });
            }
        }
    }
    let start = usize::from(header.portal_start);
    let end = start + usize::from(header.portal_count);
    for reference in model.portals.refs.get(start..end).unwrap_or(&[]) {
        let next = u32::from(reference.group);
        if reference.group == 0xFFFF || next == from {
            continue;
        }
        let Some(polygon) = model.portals.polygon(reference.portal) else {
            continue;
        };
        if polygon.is_empty() {
            continue;
        }
        // One bit per face of the box the vertex is outside of; a portal all
        // of whose vertices share a bit is wholly outside that face.
        let outside = polygon.iter().fold(0x3Fu8, |all, v| {
            let mut bits = 0u8;
            for axis in 0..3 {
                if v[axis] < qmin[axis] {
                    bits |= 1 << axis;
                }
                if v[axis] > qmax[axis] {
                    bits |= 8 << axis;
                }
            }
            all & bits
        });
        if outside == 0 {
            walk(model, next, group, query, visited, out);
        }
    }
}

/// **The order the pictures are drawn in**, bottom first: by the height of a
/// cell's group above the character (`local_z`, WMO-local), lowest first, with
/// every cell of the first listed picture's group — the group the walk began
/// in, when it gave any — drawn last. A floor below shows only through the
/// gaps in the floor the character is on, and a floor above never covers it.
pub fn draw_order(pictures: &mut [InteriorPicture], local_z: f32) {
    let Some(top) = pictures.first().map(|p| p.group) else {
        return;
    };
    let key = |p: &InteriorPicture| if p.group == top { f32::MAX } else { p.tile.z - local_z };
    pictures.sort_by(|a, b| key(a).total_cmp(&key(b)));
}

/// The texture coordinate at each of [`PlacedPicture::corners`]: texel column
/// 0 at the cell's least `x` and row 0 at its greatest `y`.
///
/// Measured as well as stated: `vale minimap <path>.wmo` scores each picture's
/// opaque texels against its group's floor under four readings, and this one
/// wins 101 of 107 pictures of Ironforge (97.4% agreement against 82.6% for
/// the best other), 268 of 310 of Stormwind and all ten of the Goldshire inn.
pub const PICTURE_UV: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

/// One picture, placed on the widget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedPicture {
    pub group: u32,
    pub x: u32,
    pub y: u32,
    /// The cell's corners in the widget's `0..1` space, y down, in the order
    /// least `x` least `y`, greatest `x` least `y`, greatest both, least `x`
    /// greatest `y` (WMO-local). The WMO's placement turns the cell, so these
    /// are a parallelogram rather than an upright rectangle.
    pub corners: [[f32; 2]; 4],
}

/// A point through a column-major matrix.
fn transform(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| m[axis] * p[0] + m[4 + axis] * p[1] + m[8 + axis] * p[2] + m[12 + axis])
}

/// **The pictures of the building a character stands in, in draw order and
/// placed on the widget**: [`interior_query`], [`box_to_local`],
/// [`interior_pictures`] and [`draw_order`] in turn, then each cell carried to
/// the world by the placement and onto the north-up widget.
///
/// `position` is the character's world position, `radius` the indoor radius,
/// and the two matrices are the placement (`local_to_world`, column-major) and
/// its inverse.
pub fn place_pictures(
    model: &InteriorModel,
    seed: u32,
    local_to_world: &[f32; 16],
    world_to_local: &[f32; 16],
    position: [f32; 3],
    radius: f32,
) -> Vec<PlacedPicture> {
    if !(radius > 0.0) {
        return Vec::new();
    }
    let query = box_to_local(&interior_query(position, radius), world_to_local);
    let mut pictures = interior_pictures(model, seed, &query);
    draw_order(&mut pictures, transform(world_to_local, position)[2]);
    let span = radius * 2.0;
    pictures
        .iter()
        .map(|picture| {
            let [lo, hi] = picture.tile.rect;
            let z = picture.tile.z;
            let local = [[lo[0], lo[1]], [hi[0], lo[1]], [hi[0], hi[1]], [lo[0], hi[1]]];
            let corners = local.map(|[x, y]| {
                let world = transform(local_to_world, [x, y, z]);
                let (north, west) = (world[0] - position[0], world[1] - position[1]);
                [0.5 - west / span, 0.5 - north / span]
            });
            PlacedPicture { group: picture.group, x: picture.tile.x, y: picture.tile.y, corners }
        })
        .collect()
}

/// **The part of a picture inside the minimap's disc**, as a convex polygon of
/// `(point, uv)` pairs; empty when nothing of it is inside.
///
/// `corners` and `uvs` are the picture's four corners and their texture
/// coordinates (any 2-D space; the painters pass pixels). The disc is the
/// `segments`-sided polygon of radius `radius` about `centre` that the
/// painters draw the terrain through, so the two meet along the same edge.
pub fn clip_to_disc(
    corners: [[f32; 2]; 4],
    uvs: [[f32; 2]; 4],
    centre: [f32; 2],
    radius: f32,
    segments: usize,
) -> Vec<([f32; 2], [f32; 2])> {
    let mut polygon: Vec<([f32; 2], [f32; 2])> = corners.into_iter().zip(uvs).collect();
    let segments = segments.max(3);
    let apothem = radius * (std::f32::consts::PI / segments as f32).cos();
    for step in 0..segments {
        if polygon.is_empty() {
            break;
        }
        let angle = std::f32::consts::TAU * (step as f32 + 0.5) / segments as f32;
        let (sin, cos) = angle.sin_cos();
        let inside = |p: [f32; 2]| apothem - ((p[0] - centre[0]) * cos + (p[1] - centre[1]) * sin);
        let mut next = Vec::with_capacity(polygon.len() + 1);
        for i in 0..polygon.len() {
            let a = polygon[i];
            let b = polygon[(i + 1) % polygon.len()];
            let (da, db) = (inside(a.0), inside(b.0));
            if da >= 0.0 {
                next.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                let mix = |p: [f32; 2], q: [f32; 2]| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
                next.push((mix(a.0, b.0), mix(a.1, b.1)));
            }
        }
        polygon = next;
    }
    if polygon.len() < 3 {
        polygon.clear();
    }
    polygon
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file's own shape, including the `dir:` header it interleaves and the
    /// CRLF it is written with.
    #[test]
    fn the_index_reads_the_shipped_shape() {
        let raw = b"dir: Azeroth\r\nAzeroth\\map32_48.blp\t1fcd95d6.blp\r\ndir: Kalimdor\r\nKalimdor\\map01_02.blp\tdeadbeef.blp\r\n";
        let tiles = MinimapTiles::parse(raw);
        assert_eq!(tiles.len(), 2);
        assert_eq!(
            tiles.texture("Azeroth", 32, 48).as_deref(),
            Some(r"textures\Minimap\1fcd95d6.blp")
        );
        // The archives are case-insensitive and so is this.
        assert_eq!(
            tiles.texture("AZEROTH", 32, 48).as_deref(),
            Some(r"textures\Minimap\1fcd95d6.blp")
        );
        assert_eq!(tiles.texture("Azeroth", 32, 49), None, "a tile with no picture");
        assert_eq!(tiles.directories(), vec!["azeroth", "kalimdor"]);
        assert_eq!(tiles.tiles_of("kalimdor"), vec![(1, 2)]);
    }

    /// A chain without the file is an empty index and not a failure — see
    /// [`MinimapTiles::parse`].
    #[test]
    fn a_missing_index_is_empty_rather_than_an_error() {
        let tiles = MinimapTiles::parse(b"");
        assert!(tiles.is_empty());
        assert_eq!(tiles.texture("Azeroth", 32, 48), None);
    }

    /// **The two zoom tables, as the client states them** — the outdoor one in
    /// chunks and the indoor one in yards, with the default landing on 133.
    #[test]
    fn the_zoom_radii_are_the_clients_two_tables() {
        let outdoor: Vec<f32> = (0..ZOOM_LEVELS).map(|z| radius_yards(z, false)).collect();
        let indoor: Vec<f32> = (0..ZOOM_LEVELS).map(|z| radius_yards(z, true)).collect();
        for (got, want) in outdoor.iter().zip([233.33, 200.0, 166.67, 133.33, 100.0, 66.67]) {
            assert!((got - want).abs() < 0.01, "{got} vs {want}");
        }
        assert_eq!(indoor, vec![150.0, 120.0, 90.0, 60.0, 40.0, 25.0]);
        assert!((radius_yards(DEFAULT_ZOOM, false) - 133.33).abs() < 0.01);
        // Past the last level clamps, which is what the client already did.
        assert_eq!(radius_yards(99, false), radius_yards(5, false));
    }

    /// **A character standing on a tile centre sees that tile and only it, dead
    /// centre** — the case that pins every sign in [`tiles_in_view`] at once.
    #[test]
    fn a_tile_centre_puts_its_own_picture_in_the_middle() {
        let centre = crate::world::terrain::tile_centre(32, 48);
        // 66 yards is zoom 5, comfortably inside one 533-yard tile.
        let view = tiles_in_view(centre[0], centre[1], 66.0);
        assert_eq!(view.len(), 1, "{view:?}");
        assert_eq!(view[0].tile, (32, 48));
        let [left, top, right, bottom] = view[0].rect;
        // The tile is four times the 133-yard view across, centred on it.
        assert!((left + right - 1.0).abs() < 1e-3, "not centred: {left}..{right}");
        assert!((top + bottom - 1.0).abs() < 1e-3, "not centred: {top}..{bottom}");
        assert!((right - left - TILE_SIZE / 132.0).abs() < 0.01);
    }

    /// **North is up and west is left**, which is the one thing about this that
    /// is easy to get backwards and impossible to see in a still picture.
    ///
    /// Standing a little north-west of a tile corner must put the tile *south
    /// and east* of the middle — so its picture's top-left goes to the right of
    /// and below the centre.
    #[test]
    fn north_is_up_and_west_is_left() {
        // The north-west corner of tile (32, 48), stepped 10 yards into the
        // tile beyond it in both axes.
        let north = MAP_ORIGIN - 48.0 * TILE_SIZE + 10.0;
        let west = MAP_ORIGIN - 32.0 * TILE_SIZE + 10.0;
        let view = tiles_in_view(north, west, 60.0);
        let target = view
            .iter()
            .find(|t| t.tile == (32, 48))
            .expect("the tile to the south east is in view");
        let [left, top, ..] = target.rect;
        assert!(left > 0.5, "west of the character is left, so the tile starts right: {left}");
        assert!(top > 0.5, "north of the character is up, so the tile starts low: {top}");
        // …and the tile that owns the character is up and to the left of it.
        assert!(view.iter().any(|t| t.tile == (31, 47)), "{view:?}");
        assert_eq!(view.len(), 4, "a corner sees four tiles: {view:?}");
    }

    /// **The character stands in the middle of their own map**, walked all the
    /// way back through the texture's own uv — which is the one property that
    /// ties [`tiles_in_view`]'s placement to what is *in* the pictures, and the
    /// whole of what the painter does with a `TileView`.
    ///
    /// The round trip is: take the widget's centre, find where it falls inside
    /// the placement of the tile the character is standing on, read that as a
    /// texel coordinate on that tile's picture, and turn the texel back into a
    /// world position. It has to come back to the character. Any sign, any
    /// transposition and any half-tile offset anywhere in the chain breaks it.
    #[test]
    fn the_character_lands_in_the_middle_of_their_own_picture() {
        let goldshire = crate::world::terrain::tile_centre(32, 48);
        for (x, y) in [
            // The origin, a tile centre, somewhere well off it, and a point a
            // few yards inside a tile corner — which is the case that puts four
            // pictures on the widget at once.
            (0.0, 0.0),
            (goldshire[0], goldshire[1]),
            (-8000.0, 1500.0),
            (MAP_ORIGIN - 40.5 * TILE_SIZE + 7.0, MAP_ORIGIN - 12.25 * TILE_SIZE - 3.0),
        ] {
            for zoom in 0..ZOOM_LEVELS {
                let radius = radius_yards(zoom, false);
                let view = tiles_in_view(x, y, radius);
                let here = crate::world::terrain::tile_for_position(x, y);
                let placed = view
                    .iter()
                    .find(|tile| tile.tile == here)
                    .unwrap_or_else(|| panic!("the tile under the character at {x},{y}: {view:?}"));
                let [left, top, right, bottom] = placed.rect;
                // Where the widget's centre falls in this picture, 0..1 from its
                // north-west corner — exactly the uv the painter hands egui.
                let u = (0.5 - left) / (right - left);
                let v = (0.5 - top) / (bottom - top);
                // The slack is a float epsilon and not a tolerance: the origin
                // sits exactly on tile (32, 32)'s north-west seam, where the
                // placement puts the character at `v == 0` and `MAP_ORIGIN -
                // 32 * TILE_SIZE` cancels to a few ten-millionths rather than
                // to zero. `tile_for_position` is in `f64` for the same reason.
                const SEAM: f32 = 1e-5;
                assert!((-SEAM..=1.0 + SEAM).contains(&u), "u {u}");
                assert!((-SEAM..=1.0 + SEAM).contains(&v), "v {v}");
                // …and that texel read back as a world position. `u` runs west
                // to east across the picture and `v` north to south.
                let back_y = MAP_ORIGIN - (here.0 as f32 + u) * TILE_SIZE;
                let back_x = MAP_ORIGIN - (here.1 as f32 + v) * TILE_SIZE;
                assert!((back_x - x).abs() < 0.05, "north at zoom {zoom}: {back_x} vs {x}");
                assert!((back_y - y).abs() < 0.05, "west at zoom {zoom}: {back_y} vs {y}");
            }
        }
    }

    /// A WMO picture line is indexed by stem, group and cell, beside the
    /// terrain lines, and the stem a WMO path gives finds it.
    #[test]
    fn the_index_reads_wmo_pictures_by_stem_group_and_cell() {
        let raw = b"dir: WMO\\Dungeon\\MD_Goldmine\r\nWMO\\Dungeon\\MD_Goldmine\\MD_Goldmine_003_01_00.blp\tabc.blp\r\nAzeroth\\map32_48.blp\tdef.blp\r\n";
        let tiles = MinimapTiles::parse(raw);
        assert_eq!(tiles.len(), 1, "the terrain line");
        assert_eq!(tiles.interior_len(), 1);
        let stem = interior_stem(r"World\wmo\Dungeon\MD_Goldmine\MD_Goldmine.wmo");
        assert_eq!(stem, r"wmo\dungeon\md_goldmine\md_goldmine");
        assert_eq!(
            tiles.interior_texture(&stem, 3, 1, 0).as_deref(),
            Some(r"textures\Minimap\abc.blp")
        );
        assert_eq!(tiles.interior_texture(&stem, 3, 0, 1), None, "x and y are not interchangeable");
        assert_eq!(tiles.interior_cells(&stem), vec![(3, 1, 0)]);
        assert_eq!(parse_interior_name(r"Azeroth\map32_48.blp"), None);
    }

    /// **The grid over a group**: a power-of-two picture at half a yard a texel,
    /// 32..256 texels, as many cells as 128-yard steps cover the extent, and the
    /// outer edges widened by a yard.
    #[test]
    fn a_group_is_cut_into_power_of_two_pictures() {
        // 40 yards is 80 texels: a 128-texel picture, 64 yards, one cell. 300
        // yards is 600: the 256 cap, 128 yards, three cells.
        let tiles = group_tiles(&[[10.0, -20.0, 0.0], [50.0, 280.0, 6.0]]);
        assert_eq!(tiles.len(), 3);
        assert_eq!((tiles[0].x, tiles[0].y), (0, 0));
        assert_eq!((tiles[2].x, tiles[2].y), (0, 2), "rows outer, columns inner");
        assert_eq!(tiles[0].rect, [[9.0, -21.0], [75.0, 108.0]]);
        assert_eq!(tiles[1].rect, [[9.0, 108.0], [75.0, 236.0]], "an inner row is not widened");
        assert_eq!(tiles[2].rect, [[9.0, 236.0], [75.0, 365.0]]);
        assert!(tiles.iter().all(|t| t.z == 3.0));
        // Ten yards is twenty texels, under the 32-texel floor: 16 yards.
        let small = group_tiles(&[[0.0; 3], [10.0, 10.0, 1.0]]);
        assert_eq!(small.len(), 1);
        assert_eq!(small[0].rect, [[-1.0, -1.0], [17.0, 17.0]]);
        // A flat box has no cells along its empty axis.
        assert!(group_tiles(&[[0.0; 3], [0.0, 10.0, 1.0]]).is_empty());
    }

    /// The search box is built around the radius-sized cell the character is
    /// in: one radius before the cell, two past its start, and from one and a
    /// half radii below the feet to one above.
    #[test]
    fn the_search_box_is_snapped_to_the_radius() {
        let query = interior_query([130.0, -70.0, 10.0], 60.0);
        // 130 is in the cell starting at 120; -70 in the one starting at -120.
        assert_eq!(query, [[60.0, -180.0, -80.0], [240.0, 0.0, 70.0]]);
        // Moving within the cell does not move the box in x or y.
        let same = interior_query([175.0, -65.0, 10.0], 60.0);
        assert_eq!(same[0][..2], query[0][..2]);
    }

    /// A WMO whose four groups test each clause of the walk.
    ///
    /// ```text
    /// g0  seed, 0..20, portal to g1 (inside the box) and g2
    /// g1  a room beside it, UNREACHABLE: no pictures, but the walk goes on to g4
    /// g2  EXTERIOR: the walk stops there
    /// g3  reached only through a portal far below the box: never reached
    /// g4  behind g1
    /// ```
    fn test_model() -> InteriorModel {
        use crate::world::wmo::WmoPortalRef;
        let header = |flags: u32, x: f32, z: f32, start: u16, count: u16| {
            Some(WmoGroupHeader {
                flags,
                bounds: [[x, 0.0, z], [x + 20.0, 20.0, z + 5.0]],
                portal_start: start,
                portal_count: count,
            })
        };
        let square = |x: f32, z: f32| {
            vec![[x, 5.0, z], [x, 10.0, z], [x, 10.0, z + 3.0], [x, 5.0, z + 3.0]]
        };
        let mut vertices = Vec::new();
        for (x, z) in [(20.0, 0.0), (0.0, 0.0), (10.0, -500.0), (40.0, 0.0)] {
            vertices.extend(square(x, z));
        }
        let portals = vec![(0, 4), (4, 4), (8, 4), (12, 4)];
        let r = |portal: u16, group: u16| WmoPortalRef { portal, group, side: 1 };
        InteriorModel {
            stem: "test".into(),
            groups: vec![
                header(0x2000, 0.0, 0.0, 0, 3),
                header(0x2000 | group_flags::UNREACHABLE, 20.0, 0.0, 3, 2),
                header(group_flags::EXTERIOR, -20.0, 0.0, 5, 1),
                header(0x2000, 0.0, -500.0, 6, 1),
                header(0x2000, 40.0, 0.0, 7, 1),
            ],
            portals: WmoPortals {
                vertices,
                portals,
                refs: vec![
                    r(0, 1), r(1, 2), r(2, 3), // g0
                    r(0, 0), r(3, 4),          // g1
                    r(1, 0),                   // g2
                    r(2, 0),                   // g3
                    r(3, 1),                   // g4
                ],
            },
        }
    }

    #[test]
    fn the_walk_follows_portals_inside_the_box() {
        let model = test_model();
        let query = [[-100.0, -100.0, -10.0], [100.0, 100.0, 10.0]];
        let pictures = interior_pictures(&model, 0, &query);
        let groups: Vec<u32> = pictures.iter().map(|p| p.group).collect();
        assert_eq!(groups, vec![0, 4], "g1 gives nothing, g2 stops, g3 is out of reach");
        // A box that misses g4 in x and y stops the walk there.
        let narrow = [[-100.0, -100.0, -10.0], [30.0, 100.0, 10.0]];
        let groups: Vec<u32> = interior_pictures(&model, 0, &narrow).iter().map(|p| p.group).collect();
        assert_eq!(groups, vec![0]);
        // An exterior seed gives nothing at all.
        assert!(interior_pictures(&model, 2, &query).is_empty());
    }

    /// The seed group is drawn last whatever its height, and the others from
    /// the lowest up.
    #[test]
    fn the_seed_group_is_drawn_over_the_rest() {
        let tile = |z: f32| InteriorTile { x: 0, y: 0, rect: [[0.0; 2], [1.0; 2]], z };
        let mut pictures = vec![
            InteriorPicture { group: 5, tile: tile(0.0) },
            InteriorPicture { group: 7, tile: tile(9.0) },
            InteriorPicture { group: 6, tile: tile(-9.0) },
        ];
        draw_order(&mut pictures, 1.0);
        let order: Vec<u32> = pictures.iter().map(|p| p.group).collect();
        assert_eq!(order, vec![6, 7, 5]);
    }

    /// The group a point stands in is the smallest box that holds it among the
    /// groups whose flags are the floor's.
    #[test]
    fn the_group_under_a_point_matches_the_floor_flags() {
        let mut model = test_model();
        model.groups.push(Some(WmoGroupHeader {
            flags: 0x2000,
            bounds: [[2.0, 2.0, 0.0], [8.0, 8.0, 5.0]],
            portal_start: 0,
            portal_count: 0,
        }));
        assert_eq!(model.group_at([5.0, 5.0, 1.0], 0x2000), Some(5), "the smaller box");
        assert_eq!(model.group_at([15.0, 15.0, 1.0], 0x2000), Some(0));
        assert_eq!(model.group_at([15.0, 15.0, 1.0], 0x8000), None, "no group with the floor's flags");
    }

    /// With the identity placement, a picture's least-x corner is south of its
    /// greatest-x one, so it is lower on the widget, and its least-y corner is
    /// east, so it is further right.
    #[test]
    fn a_picture_is_placed_north_up() {
        let model = test_model();
        let identity: [f32; 16] = std::array::from_fn(|i| if i % 5 == 0 { 1.0 } else { 0.0 });
        let placed = place_pictures(&model, 0, &identity, &identity, [10.0, 10.0, 1.0], 40.0);
        assert_eq!(placed.last().map(|p| p.group), Some(0), "the seed group is drawn last");
        let seed = placed.last().expect("the seed group's picture");
        let [low, along_x, _, along_y] = seed.corners;
        assert!(along_x[1] < low[1], "greater x is north, so higher: {placed:?}");
        assert!(along_y[0] < low[0], "greater y is west, so further left");
        // The character at (10, 10) is the widget's centre: the cell runs from
        // -1 to 33 in both axes, 80 yards across the widget.
        assert!((low[0] - (0.5 + 11.0 / 80.0)).abs() < 1e-5);
        assert!((low[1] - (0.5 + 11.0 / 80.0)).abs() < 1e-5);
    }

    /// A picture wholly inside the disc is returned whole, one wholly outside
    /// is empty, and one across the rim is cut at it with its uv carried along.
    #[test]
    fn a_picture_is_cut_to_the_disc() {
        let uv = PICTURE_UV;
        let inside = clip_to_disc([[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]], uv, [0.0; 2], 10.0, 48);
        assert_eq!(inside.len(), 4);
        let outside = clip_to_disc([[20.0, 20.0], [21.0, 20.0], [21.0, 21.0], [20.0, 21.0]], uv, [0.0; 2], 10.0, 48);
        assert!(outside.is_empty());
        let across = clip_to_disc([[0.0, -1.0], [20.0, -1.0], [20.0, 1.0], [0.0, 1.0]], uv, [0.0; 2], 10.0, 48);
        let reach = across.iter().map(|(p, _)| p[0]).fold(0.0f32, f32::max);
        assert!(reach <= 10.0 + 1e-4 && reach > 9.9, "cut at the rim: {reach}");
        let (p, t) = across.iter().find(|(p, _)| p[0] == reach).expect("the rim point");
        assert!((t[0] - p[0] / 20.0).abs() < 1e-4, "uv follows the cut");
    }

    /// The widest zoom is 466 yards across and a tile is 533, so it can never
    /// need more than a 2x2 — which is what bounds the draw at four quads.
    #[test]
    fn the_widest_view_never_needs_more_than_four_tiles() {
        let radius = radius_yards(0, false);
        for step in 0..16 {
            let along = step as f32 * TILE_SIZE / 16.0;
            let view = tiles_in_view(along, along, radius);
            assert!(view.len() <= 4, "{} tiles at {along}: {view:?}", view.len());
        }
    }
}
