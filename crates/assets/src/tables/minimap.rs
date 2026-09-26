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

use std::collections::HashMap;

use crate::world::adt::{MAP_ORIGIN, TILE_SIZE};

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
        for line in text.lines() {
            let line = line.trim_end_matches(['\r', '\n']);
            let Some((path, file)) = line.split_once('\t') else {
                continue;
            };
            let Some((dir, name)) = path.rsplit_once('\\') else {
                continue;
            };
            let Some((col, row)) = parse_tile_name(name) else {
                continue;
            };
            let file = file.trim();
            if file.is_empty() {
                continue;
            }
            tiles.insert((dir.to_ascii_lowercase(), col, row), file.to_string());
        }
        MinimapTiles { tiles }
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
