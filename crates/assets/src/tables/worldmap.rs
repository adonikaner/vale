//! The world map: which picture is showing, and where on it a world position
//! lands.
//!
//! Two tables, one grid file per continent, and the arithmetic below. None of
//! it crosses the wire and neither table states it, so every rule here is the
//! 1.12.1 client's.
//!
//! ```text
//! WorldMapArea.dbc        51 rows x 8 fields
//!   [0] id  [1] mapId  [2] areaId  [3] areaName, the art directory
//!   [4] locLeft (y1)   [5] locRight (y2)   [6] locTop (x1)   [7] locBottom (x2)
//!
//! WorldMapContinent.dbc    2 rows x 13 fields
//!   [0] id  [1] mapId  [2..5] left/right/top/bottom tile
//!   [6] offsetX  [7] offsetY  [8] scale  [9..12] not read by the world map
//! ```
//!
//! The grid files are `Interface\WorldMap\Kalimdor.zmp` and `Azeroth.zmp`,
//! 65,536 bytes each: 128x128 little-endian `AreaTable` ids. They are the only
//! data that says which zone a point on a continent map is over. See
//! [`WorldMap::load_zone_grids`].
//!
//! ## The four views
//!
//! The client keeps two integers, continent and zone, and every map function
//! in the interface reads them:
//!
//! ```text
//! continent  zone  what is showing         GetMapInfo answers
//! -1         -1    the cosmic map          nothing; WorldMapFrame_Update falls back to "World"
//!  i         -1    a continent             the continent's WorldMapArea directory
//!  i          j    a zone                  that zone's directory
//! -2         -1    one row on its own      that row's directory
//! ```
//!
//! `GetCurrentMapContinent` and `GetCurrentMapZone` answer the two integers
//! plus one. `WorldMapFrame_Update` disables the zoom-out button when the
//! continent answer is 0. [`MapView`] is this state.
//!
//! The fourth view is for a map that is no continent's map. The client lists,
//! at load, every row in file order that has an area and whose map has no
//! continent, and shows the first such row for the character's map on its own:
//! that is how Alterac Valley, Warsong Gulch and Arathi Basin have a map. See
//! [`WorldMap::single_for`]. A single map has no zones to choose
//! (`GetMapZones(-1)` answers none), zooms out to the cosmic map, ignores a
//! click, highlights nothing and labels nothing, and draws its overlays.
//!
//! ## The continents are built from `WorldMapArea`
//!
//! The client's builder walks `WorldMapArea.dbc` twice:
//!
//! * a row with `areaId == 0` is a continent: its `mapId` names the continent
//!   and its own id is the map drawn when no zone is selected;
//! * a row with the same `mapId` and a non-zero `areaId` is one of its zones.
//!   The zone list is sorted by name before anything reads it, so the order
//!   `GetMapZones` answers in is not file order.
//!
//! A continent's name is `Map.dbc`'s localised name for its `mapId` (field
//! 4); a zone's name is `AreaTable.dbc`'s (field 11). `WorldMapArea` holds
//! neither: its field 3 is the art directory, which `GetMapInfo` answers and
//! `Interface\WorldMap\<dir>\<dir>N` is built from.
//!
//! ## Where a position lands
//!
//! On a continent, zone or single map it is one interpolation per axis over
//! the row's rectangle. The axes are crossed as this game's coordinates are
//! (world `y` runs west, world `x` runs north):
//!
//! ```text
//! u = 1 - (pos.y - locRight)  / (locLeft - locRight)
//! v = 1 - (pos.x - locBottom) / (locTop  - locBottom)
//! ```
//!
//! The answer is (0, 0) if either denominator is zero, if the numerator is
//! zero, or if the result leaves 0..1. `GetPlayerMapPosition` answering
//! `0, 0` makes `WorldMapButton_OnUpdate` hide the arrow. The row must also be
//! on the unit's own map.
//!
//! The cosmic map has no `WorldMapArea` row and so no rectangle. A continent is
//! placed on it by its own tile offset and scale:
//!
//! ```text
//! u = 0.5 + offsetX * (16/1002) - (pos.y / 533.33333) * scale * (16/1002)
//! v = 0.5 + offsetY * (16/668)  - (pos.x / 533.33333) * scale * (16/668)
//! ```
//!
//! The two constants are 0.015968064 and 0.023952097: 16/1002 and 16/668, the
//! picture's pixel size over a 16-pixel unit. The four floats at fields 9..12
//! are not read. The same builder derives the continent's rectangle on the
//! cosmic map from fields 2..8 instead ([`Continent::rect_on_cosmic`]), which
//! puts Kalimdor at u 0.089..0.400 and Eastern Kingdoms at 0.624..0.923, where
//! a screenshot of the 1.12.1 client has them.
//!
//! ## The highlight under the pointer
//!
//! `UpdateMapHighlight(x, y)` answers eight values: a name, the art
//! directory, two texture fractions, a size and an offset. It reads the zone
//! integer first, and a zone below 0 skips the overlays.
//!
//! * On a zone map it walks `WorldMapOverlay.dbc`'s hit rectangles (fields
//!   13..16, `hitRectTop/Left/Bottom/Right`, scaled by 1/668 and 1/1002). On a
//!   hit it answers the sub-area's name followed by six zeroes, because the
//!   archives have no sub-area highlight art.
//! * On a continent map the zone under the pointer is highlighted, and on the
//!   cosmic map the continent. Both take the directory from `WorldMapArea`
//!   field 3: `Interface\WorldMap\Elwynn\ElwynnHighlight.blp` and
//!   `…\Azeroth\AzerothHighlight.blp`.
//! * On a single map it answers nothing.
//!
//! So `WorldMapOverlay` gives the label on a zone map and not the highlight.
//! See [`WorldMap::highlight`] for the geometry.
//!
//! ## The overlays: the explored parts of a zone map
//!
//! A zone's picture ships blank, and everything drawn on it is a
//! `WorldMapOverlay` row the character has explored: the roads, the villages,
//! the lakes and the named places. The table has 526 rows of 17 fields. The
//! layout below follows the seven values `GetMapOverlayInfo` returns, which
//! fix seven of the seventeen fields:
//!
//! ```text
//! WorldMapOverlay.dbc     526 rows x 17 fields, 0x44 bytes each
//!   [ 0] id
//!   [ 1] mapAreaId       the WorldMapArea row this belongs to
//!   [ 2..5] areaId[4]    AreaTable ids; any one explored shows it
//!   [ 6] mapPointX  [ 7] mapPointY
//!   [ 8] textureName     Interface\WorldMap\<directory>\<textureName>N
//!   [ 9] textureWidth   [10] textureHeight
//!   [11] offsetX        [12] offsetY
//!   [13..16] hitRect Top, Left, Bottom, Right
//! ```
//!
//! For every row whose `mapAreaId` is the row being drawn, the client walks
//! the four `areaId`s and takes the row as soon as one of them is explored:
//! `AreaTable`'s `areaBit` ([`crate::tables::area::Area::explore_bit`]) tested
//! against `PLAYER_EXPLORED_ZONES`. An `areaId` of 0, or one the table does not
//! have, is skipped, so a row none of whose areas is known is not shown.
//!
//! The label on a zone map comes from the same list: `UpdateMapHighlight`'s
//! zone branch walks the explored overlays' hit rectangles. That is
//! [`WorldMap::overlay_label`].
//!
//! ## Landmarks
//!
//! The landmark flags (`GetNumMapLandmarks`) are `AreaPOI.dbc` rows projected
//! through [`WorldMap::position`]; see [`crate::tables::areapoi`].

use crate::tables::area::Areas;
use crate::tables::dbc::Dbc;
use std::collections::HashMap;

/// `WorldMapArea.dbc`'s fields. See the module comment for the layout.
pub mod area_fields {
    pub const ID: usize = 0;
    pub const MAP_ID: usize = 1;
    /// The `AreaTable` id, or 0 for the row that is a continent.
    pub const AREA_ID: usize = 2;
    /// The art directory, not a name to show; see the module comment.
    pub const DIRECTORY: usize = 3;
    pub const LOC_LEFT: usize = 4;
    pub const LOC_RIGHT: usize = 5;
    pub const LOC_TOP: usize = 6;
    pub const LOC_BOTTOM: usize = 7;
}

/// `WorldMapOverlay.dbc`'s fields. See the module comment for the layout.
pub mod overlay_fields {
    pub const ID: usize = 0;
    pub const MAP_AREA_ID: usize = 1;
    /// `areaId[4]`, fields 2..5. Any one of them explored shows the row.
    pub const AREA_IDS: std::ops::Range<usize> = 2..6;
    pub const MAP_POINT_X: usize = 6;
    pub const MAP_POINT_Y: usize = 7;
    pub const TEXTURE: usize = 8;
    pub const TEXTURE_WIDTH: usize = 9;
    pub const TEXTURE_HEIGHT: usize = 10;
    pub const OFFSET_X: usize = 11;
    pub const OFFSET_Y: usize = 12;
    pub const HIT_TOP: usize = 13;
    pub const HIT_LEFT: usize = 14;
    pub const HIT_BOTTOM: usize = 15;
    pub const HIT_RIGHT: usize = 16;
}

/// `WorldMapContinent.dbc`'s fields that this file reads. See the module
/// comment for the layout.
pub mod continent_fields {
    pub const ID: usize = 0;
    pub const MAP_ID: usize = 1;
    pub const LEFT_TILE: usize = 2;
    pub const RIGHT_TILE: usize = 3;
    pub const TOP_TILE: usize = 4;
    pub const BOTTOM_TILE: usize = 5;
    pub const OFFSET_X: usize = 6;
    pub const OFFSET_Y: usize = 7;
    pub const SCALE: usize = 8;
}

/// One ADT tile, in yards. The same constant [`crate::world::terrain`] uses, restated
/// here because the cosmic mapping divides by it.
const TILE_YARDS: f32 = 533.333_33;

/// The whole map, in yards — the client uses its reciprocal, `1 / 34133.332`.
const MAP_YARDS: f32 = TILE_YARDS * 64.0;

/// A `.zmp` is `128 x 128` cells, which is the 64-tile map at half-tile
/// resolution.
const GRID_SIDE: usize = 128;

/// The parchment, in the pixels every overlay is authored against —
/// `WorldMapDetailFrame`'s own size in `WorldMapFrame.xml`, and the two divisors
/// `UpdateMapHighlight` scales a hit rectangle by.
///
/// An overlay's `offsetX`/`textureWidth`/`hitRect*` are all in this space, so a
/// fraction of the map is a coordinate over one of these.
pub const PARCHMENT_WIDTH: f32 = 1002.0;
pub const PARCHMENT_HEIGHT: f32 = 668.0;

/// `16 / 1002` and `16 / 668` — the cosmic
/// parchment's pixel size over the 16-pixel unit the tile offsets are in.
const COSMIC_U_PER_UNIT: f32 = 16.0 / PARCHMENT_WIDTH;
const COSMIC_V_PER_UNIT: f32 = 16.0 / PARCHMENT_HEIGHT;

/// The builder's `31.3125 - scale*32` and
/// `20.875 - scale*32`, which centre a continent's tile grid on the parchment.
const COSMIC_TILE_SPAN: f32 = 32.0;
const COSMIC_CENTRE_X: f32 = 31.3125;
const COSMIC_CENTRE_Y: f32 = 20.875;

/// Which parchment is showing. The client's own two integers, named.
///
/// `Cosmic` is `continent == -1`; a `Zone`'s indices are both zero-based
/// here and one-based across the Lua boundary, exactly as the client keeps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapView {
    /// Both continents at once — no `WorldMapArea` row, art directory "World".
    #[default]
    Cosmic,
    Continent(usize),
    Zone(usize, usize),
    /// A `WorldMapArea` row, by id, on a map that is no continent's map,
    /// shown on its own. See [`WorldMap::single_for`].
    Single(u32),
}

impl MapView {
    /// `GetCurrentMapContinent()`: the client's continent integer plus one.
    /// The client holds -2 for a [`MapView::Single`] view, so it answers -1.
    pub fn continent_index(self) -> i32 {
        match self {
            MapView::Cosmic => 0,
            MapView::Single(_) => -1,
            MapView::Continent(c) | MapView::Zone(c, _) => c as i32 + 1,
        }
    }

    /// `GetCurrentMapZone()`: the client's zone integer plus one. A single
    /// map has no zone, so it answers 0.
    pub fn zone_index(self) -> usize {
        match self {
            MapView::Cosmic | MapView::Continent(_) | MapView::Single(_) => 0,
            MapView::Zone(_, z) => z + 1,
        }
    }

    /// The pair as the interface passes them to `SetMapZoom`, one-based with 0
    /// meaning "all of it".
    pub fn from_indices(continent: usize, zone: usize) -> MapView {
        match (continent.checked_sub(1), zone.checked_sub(1)) {
            (None, _) => MapView::Cosmic,
            (Some(c), None) => MapView::Continent(c),
            (Some(c), Some(z)) => MapView::Zone(c, z),
        }
    }
}

/// One `WorldMapArea` row.
#[derive(Debug, Clone, PartialEq)]
pub struct MapArea {
    pub id: u32,
    pub map: u32,
    /// `AreaTable` id, or 0 for the row that *is* a continent.
    pub area: u32,
    /// `Interface\WorldMap\<directory>\<directory>1..12`.
    pub directory: String,
    /// The rectangle in world yards: left/right are `y`, top/bottom are `x`, and
    /// each pair runs from the larger value to the smaller.
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl MapArea {
    /// Where a world position falls on this map, `0..1` from the top left — or
    /// `None` for a position off it. See the module comment for the rule and the
    /// three ways it declines.
    pub fn position(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let axis = |value: f32, near: f32, far: f32| {
            let span = near - far;
            let offset = value - far;
            if span == 0.0 || offset == 0.0 {
                return None;
            }
            Some(1.0 - offset / span)
        };
        let u = axis(y, self.left, self.right)?;
        let v = axis(x, self.top, self.bottom)?;
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return None;
        }
        Some((u, v))
    }
}

/// One `WorldMapOverlay` row — a picture of an explored corner of a zone.
///
/// Every measurement here is in parchment pixels ([`PARCHMENT_WIDTH`] by
/// [`PARCHMENT_HEIGHT`]), which is the space `WorldMapFrame_Update` lays them
/// out in and the space the hit rectangles are compared in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapOverlay {
    pub id: u32,
    /// The [`MapArea`] row this belongs to — the zone whose parchment it is on.
    pub map_area: u32,
    /// The four areas that reveal it, zeroes and all: any one explored shows
    /// the row. Kept as the raw four rather than a filtered list because
    /// the client's loop is a fixed four and a zero is skipped inside it.
    pub areas: [u32; 4],
    /// `Interface\WorldMap\<directory>\<texture>N`, `N` from 1 — see
    /// [`MapOverlay::tiles`]. The shipped names are upper case and the archives
    /// are case-insensitive.
    pub texture: String,
    pub width: u32,
    pub height: u32,
    pub offset_x: u32,
    pub offset_y: u32,
    /// `GetMapOverlayInfo`'s last two returns. Nothing in 5875's FrameXML reads
    /// them — `WorldMapFrame_Update` assigns them and never uses them — and
    /// every shipped row has 0 in both, but they are the function's answer and
    /// so they are carried.
    pub map_point: (u32, u32),
    /// The label rectangle: top, left, bottom, right, in parchment pixels.
    pub hit: (u32, u32, u32, u32),
}

/// The tile grid an overlay's art is cut into — 256 pixels a side, the last
/// column and row short.
///
/// `WorldMapFrame_Update` does this arithmetic itself and this exists only so
/// that a check can ask the archives whether every piece is there. See
/// `vale zones`.
impl MapOverlay {
    /// How many 256-pixel pieces across and down the art is.
    pub fn tiles(&self) -> (u32, u32) {
        let ceil = |value: u32| value.div_ceil(256).max(1);
        (ceil(self.width), ceil(self.height))
    }

    /// The `n`th piece's file name, `n` from 1, in the file's own row-major
    /// order — `((j - 1) * numTexturesWide) + k`.
    pub fn piece(&self, directory: &str, n: u32) -> String {
        format!("{}{n}", path_in(directory, &self.texture))
    }

    /// Whether a point on the parchment, in `0..1`, is inside this overlay's
    /// label rectangle.
    fn covers(&self, u: f32, v: f32) -> bool {
        let (top, left, bottom, right) = self.hit;
        let x = u * PARCHMENT_WIDTH;
        let y = v * PARCHMENT_HEIGHT;
        (left as f32..=right as f32).contains(&x) && (top as f32..=bottom as f32).contains(&y)
    }
}

/// One continent, as the client builds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Continent {
    pub map: u32,
    /// The `WorldMapArea` row drawn when no zone is selected — the row whose
    /// `areaId` is 0.
    pub whole: u32,
    /// …and its zones' rows, in the order the client sorts them.
    pub zones: Vec<u32>,
    offset_x: f32,
    offset_y: f32,
    scale: f32,
    left_tile: f32,
    right_tile: f32,
    top_tile: f32,
    bottom_tile: f32,
    /// `Interface\WorldMap\<directory>.zmp`, resolved to zone ids — see
    /// [`WorldMap::load_zone_grids`]. `None` until it is loaded, and for a
    /// continent that ships none.
    zone_grid: Option<Vec<u32>>,
}

impl Continent {
    /// Where this continent sits on the cosmic parchment, as
    /// `(left, top, right, bottom)` fractions.
    ///
    /// The right and bottom tiles are the *inclusive* last tile in the file, so
    /// each is used as `tile + 1`, as the client does.
    pub fn rect_on_cosmic(&self) -> (f32, f32, f32, f32) {
        let centred_x = COSMIC_CENTRE_X - self.scale * COSMIC_TILE_SPAN + self.offset_x;
        let centred_y = COSMIC_CENTRE_Y - self.scale * COSMIC_TILE_SPAN + self.offset_y;
        let u = |tile: f32| (tile * self.scale + centred_x) * COSMIC_U_PER_UNIT;
        let v = |tile: f32| (tile * self.scale + centred_y) * COSMIC_V_PER_UNIT;
        (
            u(self.left_tile),
            v(self.top_tile),
            u(self.right_tile + 1.0),
            v(self.bottom_tile + 1.0),
        )
    }

    /// Whether this continent's `.zmp` grid was loaded — the difference between
    /// the game's own answer and the rectangle fallback, for the checks.
    pub fn has_zone_grid(&self) -> bool {
        self.zone_grid.is_some()
    }

    /// The zone standing at a world position, out of the `.zmp` grid — an
    /// `AreaTable` zone id, or `None` for a cell that names nothing and for a
    /// continent with no grid.
    ///
    /// The 1.12.1 client's rule, each value truncated towards zero:
    ///
    /// ```text
    /// row = trunc((0.5 - x/34133.332) * 128)      ; north to south
    /// col = trunc((0.5 - y/34133.332) * 128)      ; west to east
    /// cell = grid[row * 128 + col]
    /// ```
    ///
    /// The row is written out because the reverse sign flips the map north to
    /// south and still answers a zone.
    pub(crate) fn zone_at_position(&self, x: f32, y: f32) -> Option<u32> {
        let grid = self.zone_grid.as_ref()?;
        let axis = |value: f32| {
            let normalised = 0.5 - value / MAP_YARDS;
            // The client refuses anything off the map rather than
            // clamping it, which is one of the two ways this answers nothing.
            (0.0..=1.0)
                .contains(&normalised)
                .then(|| (normalised * GRID_SIDE as f32) as usize)
                .filter(|cell| *cell < GRID_SIDE)
        };
        let cell = grid.get(axis(x)? * GRID_SIDE + axis(y)?)?;
        (*cell != 0).then_some(*cell)
    }

    /// Where a world position on this continent falls on the cosmic map.
    ///
    /// Unlike [`MapArea::position`] this is not clamped to the parchment: the
    /// client's cosmic branch has no range test at all, because the whole world
    /// is on it by construction.
    pub(crate) fn position_on_cosmic(&self, x: f32, y: f32) -> (f32, f32) {
        let u = 0.5 + self.offset_x * COSMIC_U_PER_UNIT
            - (y / TILE_YARDS) * self.scale * COSMIC_U_PER_UNIT;
        let v = 0.5 + self.offset_y * COSMIC_V_PER_UNIT
            - (x / TILE_YARDS) * self.scale * COSMIC_V_PER_UNIT;
        (u, v)
    }
}

/// The two tables, joined the way the client joins them.
#[derive(Debug, Default)]
pub struct WorldMap {
    areas: HashMap<u32, MapArea>,
    continents: Vec<Continent>,
    /// The overlays in file order, which is the order the client walks them
    /// and therefore the order `GetMapOverlayInfo`'s index means. Nothing
    /// sorts it.
    overlays: Vec<MapOverlay>,
    /// The ids of the rows that have an area and whose map has no continent,
    /// in file order. See [`WorldMap::single_for`].
    singles: Vec<u32>,
}

impl WorldMap {
    /// Parse and build. `areas` decides the order of each continent's zone
    /// list, because the client sorts it by the `AreaTable` name; with no
    /// `AreaTable` the zones stay in id order.
    ///
    /// `None` when either of the first two tables is missing, because a
    /// `WorldMap` with areas and no continents cannot be told apart from a
    /// correctly read table for a game with one map. `world_map_overlay` may
    /// be empty: without it every zone picture is blank, which is what an
    /// unexplored zone looks like.
    pub fn parse(
        world_map_area: &[u8],
        world_map_continent: &[u8],
        world_map_overlay: &[u8],
        areas: Option<&Areas>,
    ) -> Option<WorldMap> {
        let area_dbc = Dbc::parse(world_map_area).ok()?;
        let continent_dbc = Dbc::parse(world_map_continent).ok()?;

        let mut areas_by_id: HashMap<u32, MapArea> = HashMap::new();
        for record in 0..area_dbc.record_count {
            let Some(id) = area_dbc.u32_at(record, 0) else {
                continue;
            };
            let float = |f: usize| f32::from_bits(area_dbc.u32_at(record, f).unwrap_or(0));
            areas_by_id.insert(
                id,
                MapArea {
                    id,
                    map: area_dbc.u32_at(record, area_fields::MAP_ID).unwrap_or(0),
                    area: area_dbc.u32_at(record, area_fields::AREA_ID).unwrap_or(0),
                    directory: area_dbc
                        .string_at(record, area_fields::DIRECTORY)
                        .unwrap_or_default(),
                    left: float(area_fields::LOC_LEFT),
                    right: float(area_fields::LOC_RIGHT),
                    top: float(area_fields::LOC_TOP),
                    bottom: float(area_fields::LOC_BOTTOM),
                },
            );
        }

        // A continent is a `WorldMapArea` row with no area: the client skips
        // every row whose field 2 is non-zero.
        let mut continents: Vec<Continent> = Vec::new();
        let mut whole_rows: Vec<&MapArea> =
            areas_by_id.values().filter(|row| row.area == 0).collect();
        // Sorted by id, where the client walks the file in record order. The
        // two orders agree in the shipped table (Kalimdor is record 3 with id
        // 13, Azeroth record 4 with id 14), so `GetMapContinents()` answers
        // "Kalimdor", "Eastern Kingdoms" either way, the drop-down's order.
        // The sort makes the build independent of the `HashMap`'s order; the
        // client does not sort.
        whole_rows.sort_by_key(|row| row.id);

        for whole in whole_rows {
            // Its zones: same map, and an area of their own.
            let mut zones: Vec<u32> = areas_by_id
                .values()
                .filter(|row| row.map == whole.map && row.area != 0)
                .map(|row| row.id)
                .collect();
            zones.sort_unstable();
            if let Some(areas) = areas {
                // The client's sort, whose comparator resolves each row's
                // `areaId` through `AreaTable` and compares the names. The id
                // tie-break is this client's, for a reproducible build.
                zones.sort_by(|a, b| {
                    let name = |id: &u32| {
                        areas_by_id
                            .get(id)
                            .and_then(|row| areas.get(row.area))
                            .map_or(String::new(), |area| area.name.to_lowercase())
                    };
                    name(a).cmp(&name(b)).then(a.cmp(b))
                });
            }

            // …and the row in `WorldMapContinent` that carries its geometry.
            let mut found = None;
            for record in 0..continent_dbc.record_count {
                if continent_dbc.u32_at(record, continent_fields::MAP_ID) != Some(whole.map) {
                    continue;
                }
                let int = |f: usize| continent_dbc.u32_at(record, f).unwrap_or(0) as i32 as f32;
                let float = |f: usize| f32::from_bits(continent_dbc.u32_at(record, f).unwrap_or(0));
                found = Some((
                    float(continent_fields::OFFSET_X),
                    float(continent_fields::OFFSET_Y),
                    float(continent_fields::SCALE),
                    int(continent_fields::LEFT_TILE),
                    int(continent_fields::RIGHT_TILE),
                    int(continent_fields::TOP_TILE),
                    int(continent_fields::BOTTOM_TILE),
                ));
                break;
            }
            // A continent with no `WorldMapContinent` row is still a
            // continent: the client records its map, its row and its zones
            // before it looks for the row, and only the placement on the
            // cosmic map is missing. With a scale of 0 every cosmic position
            // falls at one point.
            let (offset_x, offset_y, scale, left_tile, right_tile, top_tile, bottom_tile) =
                found.unwrap_or((0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0));
            continents.push(Continent {
                map: whole.map,
                whole: whole.id,
                zones,
                offset_x,
                offset_y,
                scale,
                left_tile,
                right_tile,
                top_tile,
                bottom_tile,
                zone_grid: None,
            });
        }

        // The rows on a map that is no continent's map, in file order: the
        // client walks the file in order and keeps each row with an area whose
        // map no continent has.
        let singles = (0..area_dbc.record_count)
            .filter_map(|record| {
                let id = area_dbc.u32_at(record, 0)?;
                let map = area_dbc.u32_at(record, area_fields::MAP_ID)?;
                let area = area_dbc.u32_at(record, area_fields::AREA_ID)?;
                (area != 0 && !continents.iter().any(|continent| continent.map == map)).then_some(id)
            })
            .collect();

        Some(WorldMap {
            areas: areas_by_id,
            continents,
            overlays: parse_overlays(world_map_overlay),
            singles,
        })
    }

    /// Load the continents' `.zmp` grids, which answer which zone a point on a
    /// continent map is over.
    ///
    /// The client reads `0x10000` bytes of `Interface\WorldMap\<dir>.zmp`,
    /// `<dir>` being the continent's art directory: 128x128 little-endian
    /// `AreaTable` ids covering the whole 64x64 tile map. At load it replaces
    /// each of the 16,384 ids with its enclosing zone (`if (parentAreaId) id =
    /// parentAreaId`) and then with the `WorldMapArea` row that has it. This
    /// keeps the zone id rather than the row, because the row's index depends
    /// on the zone sort in [`WorldMap::parse`].
    ///
    /// `read` answers by archive path, which is why this is a second step
    /// rather than an argument to [`WorldMap::parse`], whose inputs are tables.
    ///
    /// A continent with no grid uses the rectangle rule in
    /// [`WorldMap::zone_at`]. That rule is right for 32 of the 46 zones and
    /// names a neighbour for the rest, because the rectangles overlap and the
    /// two tables have no tie-break.
    pub fn load_zone_grids(
        &mut self,
        mut read: impl FnMut(&str) -> Option<Vec<u8>>,
        areas: Option<&Areas>,
    ) {
        let WorldMap { areas: rows, continents, .. } = self;
        for continent in continents.iter_mut() {
            let Some(directory) = rows.get(&continent.whole).map(|row| row.directory.as_str())
            else {
                continue;
            };
            let Some(raw) = read(&format!("Interface\\WorldMap\\{directory}.zmp")) else {
                continue;
            };
            if raw.len() < GRID_SIDE * GRID_SIDE * 4 {
                continue;
            }
            continent.zone_grid = Some(
                raw.chunks_exact(4)
                    .take(GRID_SIDE * GRID_SIDE)
                    .map(|cell| {
                        let id = u32::from_le_bytes([cell[0], cell[1], cell[2], cell[3]]);
                        // The sub-area's enclosing zone, which is all the
                        // continent map names. With no `AreaTable` the id is
                        // kept as it is, and a sub-area then matches no row
                        // and names nothing.
                        areas
                            .and_then(|areas| areas.get(id))
                            .filter(|area| area.parent != 0)
                            .map_or(id, |area| area.parent)
                    })
                    .collect(),
            );
        }
    }

    pub fn continents(&self) -> &[Continent] {
        &self.continents
    }

    pub fn continent(&self, index: usize) -> Option<&Continent> {
        self.continents.get(index)
    }

    /// The `WorldMapArea` row a view is drawn from, or `None` for the cosmic
    /// map, which has none. For that case `WorldMapFrame_Update` has
    /// `if ( not mapFileName ) then mapFileName = "World"; end`.
    pub(crate) fn area_for(&self, view: MapView) -> Option<&MapArea> {
        let id = match view {
            MapView::Cosmic => return None,
            MapView::Continent(c) => self.continent(c)?.whole,
            MapView::Zone(c, z) => *self.continent(c)?.zones.get(z)?,
            MapView::Single(id) => id,
        };
        self.areas.get(&id)
    }

    /// The row the world map shows on its own for a character on `map`, a map
    /// that is no continent's: the first row, in file order, with an area and
    /// that map. `None` for a map with no such row, where the client shows
    /// the cosmic map.
    ///
    /// The shipped table has three: Alterac Valley (map 30), Warsong Gulch
    /// (489) and Arathi Basin (529).
    pub fn single_for(&self, map: u32) -> Option<u32> {
        self.singles
            .iter()
            .copied()
            .find(|id| self.areas.get(id).is_some_and(|row| row.map == map))
    }

    pub fn area(&self, id: u32) -> Option<&MapArea> {
        self.areas.get(&id)
    }

    /// `GetMapInfo`'s first answer: the art directory, or `None` for the cosmic
    /// map.
    pub fn directory(&self, view: MapView) -> Option<&str> {
        self.area_for(view).map(|area| area.directory.as_str())
    }

    /// Where a world position lands on the view that is showing.
    ///
    /// `None` when the position is not on it: off the rectangle, on another
    /// map, or on the cosmic map for a map with no continent. In each case
    /// `GetPlayerMapPosition` answers `0, 0`.
    pub fn position(&self, view: MapView, map: u32, x: f32, y: f32) -> Option<(f32, f32)> {
        match view {
            MapView::Cosmic => {
                let continent = self.continents.iter().find(|c| c.map == map)?;
                Some(continent.position_on_cosmic(x, y))
            }
            _ => {
                let area = self.area_for(view)?;
                // The row has to be on the unit's own map.
                if area.map != map {
                    return None;
                }
                area.position(x, y)
            }
        }
    }

    /// Which zone of `continent` a point on its map is over, as a zone index:
    /// what `UpdateMapHighlight` names and what a click zooms into.
    ///
    /// The point is taken back into world yards through the continent's
    /// rectangle, and the zone is read from the `.zmp` grid at half-tile
    /// resolution. The grid has no overlaps; the zone rectangles do, so a
    /// rectangle test names Elwynn Forest over Stormwind and Dun Morogh over
    /// half of Loch Modan.
    ///
    /// The rectangle test is the rule for a continent with no grid. It agrees
    /// with the grid for 32 of the game's 46 zones. See
    /// [`WorldMap::load_zone_grids`].
    pub fn zone_at(&self, continent: usize, u: f32, v: f32) -> Option<usize> {
        let continent_row = self.continent(continent)?;
        let whole = self.areas.get(&continent_row.whole)?;
        let span_u = whole.left - whole.right;
        let span_v = whole.top - whole.bottom;
        if span_u == 0.0 || span_v == 0.0 {
            return None;
        }
        // Back out of the continent map's fractions into world yards.
        let y = whole.left - u * span_u;
        let x = whole.top - v * span_v;

        // A cell that names nothing (open sea) is an answer, so the rectangle
        // rule is used only when there is no grid, never for an empty cell.
        if continent_row.zone_grid.is_some() {
            let zone = continent_row.zone_at_position(x, y)?;
            return continent_row
                .zones
                .iter()
                .position(|id| self.areas.get(id).is_some_and(|row| row.area == zone));
        }
        // No grid: the first zone, in the sorted list, whose rectangle holds
        // the point. The tie-break is this client's.
        continent_row.zones.iter().position(|id| {
            self.areas.get(id).is_some_and(|zone| {
                (zone.right..=zone.left).contains(&y) && (zone.bottom..=zone.top).contains(&x)
            })
        })
    }

    /// Which continent a point on the cosmic parchment is over.
    ///
    /// On the cosmic map the client tests the pointer against each continent's
    /// [`Continent::rect_on_cosmic`] and takes the first that contains it. The
    /// highlight and `ProcessMapClick` both use this.
    pub fn continent_at(&self, u: f32, v: f32) -> Option<usize> {
        self.continents.iter().position(|continent| {
            let (left, top, right, bottom) = continent.rect_on_cosmic();
            (left..=right).contains(&u) && (top..=bottom).contains(&v)
        })
    }

    /// `UpdateMapHighlight`'s other seven answers: the art to lay over what the
    /// pointer is on, or `None` for nothing.
    ///
    /// * A zone map highlights nothing. Its label comes from
    ///   `WorldMapOverlay`, followed by six zeroes, because no sub-area
    ///   highlight art is shipped.
    /// * A single map highlights nothing and has no label.
    /// * A continent map highlights the zone under the pointer, sized and
    ///   placed by that zone's rectangle inside the continent's: the
    ///   interpolation [`MapArea::position`] makes, taken at the corners.
    /// * The cosmic map highlights the continent under the pointer, at
    ///   [`Continent::rect_on_cosmic`].
    ///
    /// [`MapHighlight::tex_percentage`] is the part that is not geometry; see
    /// its own note.
    pub fn highlight(&self, view: MapView, u: f32, v: f32) -> Option<MapHighlight> {
        match view {
            MapView::Zone(_, _) | MapView::Single(_) => None,
            MapView::Continent(index) => {
                let continent = self.continent(index)?;
                let whole = self.areas.get(&continent.whole)?;
                let hit = self.zone_at(index, u, v)?;
                let zone = self.areas.get(continent.zones.get(hit)?)?;
                let (span_u, span_v) = (whole.left - whole.right, whole.top - whole.bottom);
                if span_u == 0.0 || span_v == 0.0 {
                    return None;
                }
                let (width, height) = (zone.left - zone.right, zone.top - zone.bottom);
                Some(MapHighlight {
                    target: HighlightTarget::Zone {
                        continent: index,
                        zone: hit,
                    },
                    directory: zone.directory.clone(),
                    // 1.0 across and a computed fraction down: the client
                    // uses a constant 1.0 for the first and computes only the
                    // second.
                    tex_percentage: (1.0, sheet_fraction(height, width)),
                    size: (width / span_u, height / span_v),
                    offset: (
                        (whole.left - zone.left) / span_u,
                        (whole.top - zone.top) / span_v,
                    ),
                })
            }
            MapView::Cosmic => {
                let index = self.continent_at(u, v)?;
                let continent = self.continent(index)?;
                let (left, top, right, bottom) = continent.rect_on_cosmic();
                // A continent's sheet is filled by its tile grid's aspect, not
                // by the fraction rule above: the client compares the two spans
                // and gives the longer one the whole sheet. The tile counts are
                // inclusive, so each span is `last - first + 1`.
                let across = continent.right_tile - continent.left_tile + 1.0;
                let down = continent.bottom_tile - continent.top_tile + 1.0;
                Some(MapHighlight {
                    target: HighlightTarget::Continent(index),
                    directory: self.areas.get(&continent.whole)?.directory.clone(),
                    tex_percentage: if across > down {
                        (1.0, down / across)
                    } else {
                        (across / down, 1.0)
                    },
                    size: (right - left, bottom - top),
                    offset: (left, top),
                })
            }
        }
    }

    /// Every overlay the table has, in file order, for the checks, which ask
    /// about all 526 rather than one zone's.
    pub fn all_overlays(&self) -> &[MapOverlay] {
        &self.overlays
    }

    /// The overlays to draw on the view that is showing, in the order
    /// `GetMapOverlayInfo`'s one-based index means.
    ///
    /// The client builds the list for a zone map and for a single map, and
    /// leaves it empty on a continent and on the cosmic map, so
    /// `GetNumMapOverlays` is 0 there and a continent shows its own picture
    /// with nothing over it.
    ///
    /// `explored` is the character's mask and `areas` is `AreaTable`, the two
    /// halves of the bit test. With either missing nothing is shown: a client
    /// that does not know what has been explored shows a new character's map.
    pub fn overlays(
        &self,
        view: MapView,
        areas: Option<&Areas>,
        explored: Option<&ExplorationMask>,
    ) -> Vec<&MapOverlay> {
        let (MapView::Zone(_, _) | MapView::Single(_), Some(areas), Some(explored)) = (view, areas, explored) else {
            return Vec::new();
        };
        let Some(row) = self.area_for(view) else {
            return Vec::new();
        };
        self.overlays
            .iter()
            .filter(|overlay| overlay.map_area == row.id)
            .filter(|overlay| {
                // Any of the four. An id the table does not have is skipped:
                // the client goes on to the next `areaId`.
                overlay.areas.iter().any(|id| {
                    areas
                        .get(*id)
                        .is_some_and(|area| explored(area.explore_bit))
                })
            })
            .collect()
    }

    /// What `GetMapOverlayInfo` answers for one overlay: the seven values, the
    /// texture as the whole path without its piece number.
    ///
    /// It is here rather than at the Lua boundary because the directory is
    /// `WorldMapArea`'s, and only this type holds both tables.
    pub fn overlay_info(&self, overlay: &MapOverlay) -> (String, u32, u32, u32, u32, u32, u32) {
        let directory = self
            .areas
            .get(&overlay.map_area)
            .map_or("", |row| row.directory.as_str());
        (
            path_in(directory, &overlay.texture),
            overlay.width,
            overlay.height,
            overlay.offset_x,
            overlay.offset_y,
            overlay.map_point.0,
            overlay.map_point.1,
        )
    }

    /// The sub-area under the pointer on a zone map, from the same list:
    /// `UpdateMapHighlight`'s zone branch, which answers a name and six
    /// zeroes.
    ///
    /// The name is that of the overlay's first area that has one, because the
    /// label is `AreaTable`'s and an overlay names up to four areas.
    pub fn overlay_label(
        &self,
        view: MapView,
        u: f32,
        v: f32,
        areas: Option<&Areas>,
        explored: Option<&ExplorationMask>,
    ) -> Option<String> {
        // The client reads the labels on a zone map only. On a single map the
        // overlays are drawn and name nothing.
        if !matches!(view, MapView::Zone(_, _)) {
            return None;
        }
        let areas = areas?;
        let overlay = self
            .overlays(view, Some(areas), explored)
            .into_iter()
            .find(|overlay| overlay.covers(u, v))?;
        overlay
            .areas
            .iter()
            .filter_map(|id| areas.get(*id))
            .map(|area| area.name.clone())
            .find(|name| !name.is_empty())
    }

    /// How many rows, how many continents and how many overlays, for the checks.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.areas.len(), self.continents.len(), self.overlays.len())
    }
}

/// What this crate needs from the character's exploration mask, and nothing
/// more: "has this `areaBit` been seen?".
///
/// A borrowed closure rather than the type itself, because the mask is
/// `vale_protocol::play::explored::Explored`, update-field state in a crate
/// this one does not depend on. A trait here could not be implemented for it
/// (the orphan rule), and a copy of the bitset here would give the same 256
/// bytes two owners.
pub(crate) type ExplorationMask<'a> = dyn Fn(u32) -> bool + 'a;

/// The path `GetMapOverlayInfo` answers with, without the piece number that
/// `WorldMapFrame_Update` appends.
///
/// `Interface\WorldMap\<directory>\<texture>`, and `Interface\WorldMap\World\
/// <texture>` for an overlay whose `WorldMapArea` row does not resolve, as the
/// 1.12.1 client builds it. No shipped overlay reaches the second form (`vale
/// zones` reports 526 of 526 naming a row that exists).
fn path_in(directory: &str, texture: &str) -> String {
    let directory = if directory.is_empty() {
        "World"
    } else {
        directory
    };
    format!("Interface\\WorldMap\\{directory}\\{texture}")
}

/// `WorldMapOverlay.dbc`, in file order. See the module comment for the
/// layout.
fn parse_overlays(raw: &[u8]) -> Vec<MapOverlay> {
    let Ok(dbc) = Dbc::parse(raw) else {
        return Vec::new();
    };
    (0..dbc.record_count)
        .filter_map(|record| {
            let at = |field: usize| dbc.u32_at(record, field).unwrap_or(0);
            let mut areas = [0u32; 4];
            for (slot, field) in areas.iter_mut().zip(overlay_fields::AREA_IDS) {
                *slot = at(field);
            }
            Some(MapOverlay {
                id: dbc.u32_at(record, 0)?,
                map_area: at(overlay_fields::MAP_AREA_ID),
                areas,
                texture: dbc
                    .string_at(record, overlay_fields::TEXTURE)
                    .unwrap_or_default(),
                width: at(overlay_fields::TEXTURE_WIDTH),
                height: at(overlay_fields::TEXTURE_HEIGHT),
                offset_x: at(overlay_fields::OFFSET_X),
                offset_y: at(overlay_fields::OFFSET_Y),
                map_point: (at(overlay_fields::MAP_POINT_X), at(overlay_fields::MAP_POINT_Y)),
                hit: (
                    at(overlay_fields::HIT_TOP),
                    at(overlay_fields::HIT_LEFT),
                    at(overlay_fields::HIT_BOTTOM),
                    at(overlay_fields::HIT_RIGHT),
                ),
            })
        })
        .collect()
}

/// What fraction of a zone's square highlight sheet the art fills, down.
///
/// The 1.12.1 client's arithmetic, which is not a plain ratio: the height is
/// expressed against a 128-wide canvas, truncated to an integer, and divided
/// by [`sheet_canvas`] of that integer. For the 3:2 rectangle every
/// `WorldMapArea` row is authored at, that is `85 / 128`. The shipped art
/// agrees: over the game's 46 zone sheets, decoded and measured, the lowest
/// texel with anything in it is row 83 of 128 (the Barrens). The exception is
/// Silithus, whose sheet fills 88x127 of its canvas; it was added later to
/// another convention. That the client crops Silithus the same way is
/// inferred from the arithmetic, not measured.
pub fn sheet_fraction(height: f32, width: f32) -> f32 {
    if width == 0.0 {
        return 1.0;
    }
    // The client truncates towards zero, as `as i32` does.
    let scaled = (height / width * ZONE_SHEET_UNIT) as i32;
    if scaled <= 0 {
        return 1.0;
    }
    scaled as f32 / sheet_canvas(scaled) as f32
}

/// `128.0` — the width the sheet fraction is measured against.
const ZONE_SHEET_UNIT: f32 = 128.0;

/// The canvas [`sheet_fraction`] divides by.
///
/// It is not a plain round-up to a power of two: the low byte alone is rounded
/// up to a power of two, and whatever was above 256 is added back rounded up
/// to the next whole 256. A low byte of zero answers the value itself. Nothing
/// in the shipped table reaches the second case (every zone lands on 85); the
/// rule is kept whole so that other data gets the client's answer.
///
/// The client also has a `canvas * 8 >= low` loop between the two steps,
/// which never runs because the power-of-two round-up already satisfies it,
/// so it is left out.
fn sheet_canvas(value: i32) -> i32 {
    let low = value & 0xff;
    if low == 0 {
        return value;
    }
    let mut canvas = 1;
    while canvas < low {
        canvas <<= 1;
    }
    if value > 256 {
        canvas += ((value >> 8) + 1) << 8;
    }
    canvas
}

/// What the pointer is over, for the caller to name: a zone's name is
/// `AreaTable`'s and a continent's is `Map.dbc`'s, and neither table is here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightTarget {
    Zone { continent: usize, zone: usize },
    Continent(usize),
}

/// The art `WorldMapButton_OnUpdate` lays over the hovered thing, in the
/// units it lays it out in: everything but [`Self::tex_percentage`] is a
/// fraction of the parchment, which is what the file multiplies by the map's own
/// width and height.
#[derive(Debug, Clone, PartialEq)]
pub struct MapHighlight {
    pub target: HighlightTarget,
    /// `Interface\WorldMap\<directory>\<directory>Highlight`.
    pub directory: String,
    /// `SetTexCoord(0, x, 0, y)` — how much of the square sheet the art fills.
    pub tex_percentage: (f32, f32),
    /// `textureX`, `textureY` — the quad's size.
    pub size: (f32, f32),
    /// `scrollChildX`, `scrollChildY` — its top-left corner, from the map's.
    pub offset: (f32, f32),
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// `\0Kalimdor\0Durotar\0Mulgore\0`
    const DIRS: &[u8] = b"\0Kalimdor\0Durotar\0Mulgore\0";

    fn area_row(id: u32, map: u32, area: u32, dir: u32, rect: [f32; 4]) -> Vec<u32> {
        let mut row = vec![0u32; 8];
        row[0] = id;
        row[area_fields::MAP_ID] = map;
        row[area_fields::AREA_ID] = area;
        row[area_fields::DIRECTORY] = dir;
        row[area_fields::LOC_LEFT] = rect[0].to_bits();
        row[area_fields::LOC_RIGHT] = rect[1].to_bits();
        row[area_fields::LOC_TOP] = rect[2].to_bits();
        row[area_fields::LOC_BOTTOM] = rect[3].to_bits();
        row
    }

    fn continent_row(id: u32, map: u32, tiles: [u32; 4], offsets: [f32; 3]) -> Vec<u32> {
        let mut row = vec![0u32; 13];
        row[0] = id;
        row[continent_fields::MAP_ID] = map;
        row[continent_fields::LEFT_TILE] = tiles[0];
        row[continent_fields::RIGHT_TILE] = tiles[1];
        row[continent_fields::TOP_TILE] = tiles[2];
        row[continent_fields::BOTTOM_TILE] = tiles[3];
        row[continent_fields::OFFSET_X] = offsets[0].to_bits();
        row[continent_fields::OFFSET_Y] = offsets[1].to_bits();
        row[continent_fields::SCALE] = offsets[2].to_bits();
        row
    }

    /// One `WorldMapOverlay` row, in the layout the module comment pins.
    fn overlay_row(id: u32, map_area: u32, areas: [u32; 4], texture: u32) -> Vec<u32> {
        let mut row = vec![0u32; 17];
        row[0] = id;
        row[overlay_fields::MAP_AREA_ID] = map_area;
        for (slot, area) in overlay_fields::AREA_IDS.zip(areas) {
            row[slot] = area;
        }
        row[overlay_fields::TEXTURE] = texture;
        row[overlay_fields::TEXTURE_WIDTH] = 300;
        row[overlay_fields::TEXTURE_HEIGHT] = 260;
        row[overlay_fields::OFFSET_X] = 100;
        row[overlay_fields::OFFSET_Y] = 50;
        row[overlay_fields::HIT_TOP] = 60;
        row[overlay_fields::HIT_LEFT] = 110;
        row[overlay_fields::HIT_BOTTOM] = 300;
        row[overlay_fields::HIT_RIGHT] = 390;
        row
    }

    /// Shared with [`crate::tables::areapoi`]'s tests, which need a real
    /// projection to pin a landmark's units against the corpse's — the one
    /// sibling that asks the same table the same question.
    pub(crate) fn built() -> WorldMap {
        built_with(&[])
    }

    fn built_with(overlays: &[Vec<u32>]) -> WorldMap {
        // Kalimdor's own row (area 0), plus Durotar and Mulgore. Mulgore is
        // listed first by id so that the *name* sort has something to do.
        let areas = [
            area_row(13, 1, 0, 1, [16000.0, -16000.0, 16000.0, -16000.0]),
            area_row(9, 1, 215, 18, [2047.917, -3089.583, -272.917, -3697.917]),
            area_row(4, 1, 14, 10, [-1962.5, -7250.0, 1808.333, -1716.667]),
        ];
        let continents = [continent_row(2, 1, [23, 48, 9, 52], [-19.0, -0.322, 0.75])];
        WorldMap::parse(
            &dbc(&areas, 8, DIRS),
            &dbc(&continents, 13, b"\0"),
            &dbc(overlays, 17, DIRS),
            Some(&crate::tables::area::Areas::default()),
        )
        .expect("both tables parse")
    }

    /// A row with no area is the continent, and every other row on its map is
    /// one of its zones — the client's builder and the count loop after it.
    #[test]
    fn the_continent_is_the_row_with_no_area() {
        let map = built();
        assert_eq!(map.counts(), (3, 1, 0));
        let kalimdor = map.continent(0).expect("the continent");
        assert_eq!(kalimdor.map, 1);
        assert_eq!(kalimdor.whole, 13);
        assert_eq!(kalimdor.zones, vec![4, 9]);
        assert_eq!(map.directory(MapView::Continent(0)), Some("Kalimdor"));
        assert_eq!(map.directory(MapView::Zone(0, 0)), Some("Durotar"));
        // …and the cosmic map has no row at all, which is what makes
        // `WorldMapFrame_Update` fall back to "World".
        assert_eq!(map.directory(MapView::Cosmic), None);
    }

    /// The interpolation, at a position whose answer is known: Orgrimmar
    /// sits in the upper middle of the Durotar parchment.
    #[test]
    fn a_position_lands_where_the_rectangle_says() {
        let map = built();
        let (u, v) = map
            .position(MapView::Zone(0, 0), 1, 1500.0, -4400.0)
            .expect("on the map");
        assert!((u - 0.461).abs() < 0.001, "u = {u}");
        assert!((v - 0.087).abs() < 0.001, "v = {v}");
    }

    /// …and every way it declines, each of which is `GetPlayerMapPosition`
    /// answering `0, 0`.
    #[test]
    fn a_position_off_the_map_has_no_answer() {
        let map = built();
        // Outside the rectangle.
        assert_eq!(map.position(MapView::Zone(0, 0), 1, 9000.0, -4400.0), None);
        // On a different map — the client's own test.
        assert_eq!(map.position(MapView::Zone(0, 0), 0, 1500.0, -4400.0), None);
        // A zone index the continent does not have.
        assert_eq!(map.position(MapView::Zone(0, 9), 1, 1500.0, -4400.0), None);
    }

    /// The cosmic branch places a continent by its tiles and scale, which is
    /// the client's derivation rather than the four floats at fields
    /// 9..12. Kalimdor's real numbers, and the answer is where it is in a
    /// screenshot: the left third of the parchment.
    #[test]
    fn the_cosmic_rectangle_is_derived_from_the_tile_bounds() {
        let map = built();
        let (left, top, right, bottom) = map.continent(0).expect("it").rect_on_cosmic();
        assert!((left - 0.0888).abs() < 0.001, "left = {left}");
        assert!((right - 0.4002).abs() < 0.001, "right = {right}");
        assert!((top - 0.0791).abs() < 0.001, "top = {top}");
        assert!((bottom - 0.8695).abs() < 0.001, "bottom = {bottom}");
    }

    /// …and a position on the cosmic map lands inside that rectangle, which is
    /// which checks the two projections against each other.
    #[test]
    fn a_cosmic_position_lands_inside_its_own_continents_rectangle() {
        let map = built();
        let (left, top, right, bottom) = map.continent(0).expect("it").rect_on_cosmic();
        // Orgrimmar again.
        let (u, v) = map
            .position(MapView::Cosmic, 1, 1500.0, -4400.0)
            .expect("the continent is on the cosmic map");
        assert!(u > left && u < right, "u {u} not in {left}..{right}");
        assert!(v > top && v < bottom, "v {v} not in {top}..{bottom}");
        // …and a map with no continent row of its own is not on it.
        assert_eq!(map.position(MapView::Cosmic, 42, 0.0, 0.0), None);
    }

    /// The indices cross the Lua boundary one-based, with 0 meaning "all of it"
    /// — the client's plus one and the compare in `WorldMapFrame_Update`.
    #[test]
    fn the_view_indices_are_one_based_across_the_boundary() {
        assert_eq!(MapView::Cosmic.continent_index(), 0);
        assert_eq!(MapView::Cosmic.zone_index(), 0);
        assert_eq!(MapView::Continent(0).continent_index(), 1);
        assert_eq!(MapView::Continent(0).zone_index(), 0);
        assert_eq!(MapView::Zone(1, 4).continent_index(), 2);
        assert_eq!(MapView::Zone(1, 4).zone_index(), 5);
        assert_eq!(MapView::from_indices(0, 0), MapView::Cosmic);
        assert_eq!(MapView::from_indices(0, 3), MapView::Cosmic);
        assert_eq!(MapView::from_indices(2, 0), MapView::Continent(1));
        assert_eq!(MapView::from_indices(2, 5), MapView::Zone(1, 4));
    }

    /// Which zone the pointer is over: what `WorldMapFrameAreaLabel` shows.
    #[test]
    fn a_point_on_the_continent_map_names_the_zone_under_it() {
        let map = built();
        // Orgrimmar's own position, taken to the continent map and back.
        let (u, v) = map
            .position(MapView::Continent(0), 1, 1500.0, -4400.0)
            .expect("on the continent");
        assert_eq!(map.zone_at(0, u, v), Some(0), "Durotar");
        // …and open ocean is nowhere.
        let (u, v) = map
            .position(MapView::Continent(0), 1, 15000.0, 15000.0)
            .expect("on the continent");
        assert_eq!(map.zone_at(0, u, v), None);
    }

    /// The `.zmp` grid decides, and it decides differently from the
    /// rectangles.
    ///
    /// The fixture puts a one-cell "city" inside Durotar's rectangle, which is
    /// what Stormwind is inside Elwynn's: the rectangle rule names the zone that
    /// sorts first and the grid names the cell.
    #[test]
    fn the_zone_grid_outranks_the_rectangles() {
        let mut map = built();
        // One cell of Durotar, in the grid's own coordinates.
        let (x, y) = (1500.0f32, -4400.0f32);
        let cell = |value: f32| ((0.5 - value / MAP_YARDS) * GRID_SIDE as f32) as usize;
        let mut grid = vec![0u8; GRID_SIDE * GRID_SIDE * 4];
        let at = |grid: &mut Vec<u8>, x: f32, y: f32, id: u32| {
            let index = (cell(x) * GRID_SIDE + cell(y)) * 4;
            grid[index..index + 4].copy_from_slice(&id.to_le_bytes());
        };
        // 215 is Mulgore's area, deliberately written under a point that is
        // inside *Durotar's* rectangle — so the two rules cannot agree.
        at(&mut grid, x, y, 215);
        map.load_zone_grids(
            |path| (path == r"Interface\WorldMap\Kalimdor.zmp").then(|| grid.clone()),
            None,
        );

        let (u, v) = map
            .position(MapView::Continent(0), 1, x, y)
            .expect("on the continent");
        assert_eq!(map.zone_at(0, u, v), Some(1), "the grid says Mulgore");
        // …and a cell that names nothing is an answer rather than a fallback:
        // the rectangle rule would have said Durotar here too.
        let (u, v) = map
            .position(MapView::Continent(0), 1, x + 600.0, y)
            .expect("on the continent");
        assert_eq!(map.zone_at(0, u, v), None);
        // A grid too short to be one is refused outright, leaving the rules
        // where they were — the parsers-tolerate-damage rule.
        let mut short = built();
        short.load_zone_grids(|_| Some(vec![0u8; 16]), None);
        let (u, v) = short
            .position(MapView::Continent(0), 1, x, y)
            .expect("on the continent");
        assert_eq!(short.zone_at(0, u, v), Some(0), "back to the rectangles");
    }

    /// A sub-area in the grid resolves to its zone, which is the walk
    /// the client makes once at load: the file names Northshire Valley and the
    /// continent map says Elwynn Forest.
    #[test]
    fn a_sub_area_in_the_grid_answers_its_zone() {
        let mut map = built();
        let (x, y) = (1500.0f32, -4400.0f32);
        let cell = |value: f32| ((0.5 - value / MAP_YARDS) * GRID_SIDE as f32) as usize;
        let mut grid = vec![0u8; GRID_SIDE * GRID_SIDE * 4];
        let index = (cell(x) * GRID_SIDE + cell(y)) * 4;
        // 362 is a made-up sub-area whose parent is Durotar (14).
        grid[index..index + 4].copy_from_slice(&362u32.to_le_bytes());
        // `AreaTable` is 25 fields with the parent at 2 and the name at 11 —
        // see [`crate::tables::area`], whose own fixture this mirrors.
        let area_row = |id: u32, parent: u32| {
            let mut row = vec![0u32; 25];
            row[0] = id;
            row[1] = 1;
            row[2] = parent;
            row
        };
        let areas = crate::tables::area::Areas::parse(&dbc(&[area_row(362, 14), area_row(14, 0)], 25, b"\0"))
            .expect("the table parses");
        map.load_zone_grids(|_| Some(grid.clone()), Some(&areas));
        let (u, v) = map
            .position(MapView::Continent(0), 1, x, y)
            .expect("on the continent");
        assert_eq!(map.zone_at(0, u, v), Some(0), "Durotar, not the valley");
    }

    /// The highlight over a zone is that zone's own rectangle, in the
    /// fractions `WorldMapButton_OnUpdate` multiplies by the map's size — so the
    /// quad it lays out has to land exactly where the zone is drawn.
    #[test]
    fn a_zone_highlight_covers_the_zone_it_is_over() {
        let map = built();
        let (u, v) = map
            .position(MapView::Continent(0), 1, 1500.0, -4400.0)
            .expect("on the continent");
        let hit = map.highlight(MapView::Continent(0), u, v).expect("Durotar");
        assert_eq!(
            hit.target,
            HighlightTarget::Zone {
                continent: 0,
                zone: 0
            }
        );
        assert_eq!(hit.directory, "Durotar");
        // The corners of the quad against the corners of the rectangle: the
        // point that was hit is inside it, and the two lerps agree.
        let (left, top) = hit.offset;
        let (right, bottom) = (left + hit.size.0, top + hit.size.1);
        assert!(left < u && u < right, "u {u} not in {left}..{right}");
        assert!(top < v && v < bottom, "v {v} not in {top}..{bottom}");
        let corner = |x: f32, y: f32| {
            map.position(MapView::Continent(0), 1, x, y)
                .expect("a corner of Durotar")
        };
        let zone = map.area(4).expect("Durotar's row");
        let (cu, cv) = corner(zone.top - 1.0, zone.left - 1.0);
        assert!((cu - left).abs() < 0.001 && (cv - top).abs() < 0.001);
    }

    /// …and the fraction of the sheet it uses is `85 / 128` for every zone in
    /// the game, because every `WorldMapArea` rectangle is authored 3:2. See
    /// [`sheet_fraction`] for the rule.
    #[test]
    fn a_zone_sheet_is_used_to_eighty_five_of_a_hundred_and_twenty_eight() {
        let map = built();
        // One point inside each of the fixture's two zones.
        for (name, x, y) in [("Durotar", 1500.0, -4400.0), ("Mulgore", -2000.0, 0.0)] {
            let (u, v) = map
                .position(MapView::Continent(0), 1, x, y)
                .expect("on the continent");
            let hit = map.highlight(MapView::Continent(0), u, v).expect(name);
            assert_eq!(hit.directory, name);
            assert_eq!(hit.tex_percentage.0, 1.0, "{name} across");
            assert!(
                (hit.tex_percentage.1 - 85.0 / 128.0).abs() < 1e-6,
                "{name} down = {}",
                hit.tex_percentage.1
            );
        }
        // The canvas rule itself, on the three shapes it can take.
        assert_eq!(sheet_canvas(85), 128);
        assert_eq!(sheet_canvas(64), 64, "an exact power of two is itself");
        assert_eq!(sheet_canvas(256), 256, "a zero low byte is the value");
        assert_eq!(sheet_canvas(668), 1024, "the low byte, then the rest");
    }

    /// The cosmic map highlights a continent, at the rectangle
    /// [`Continent::rect_on_cosmic`] already derives — and its sheet fraction is
    /// the tile grid's aspect rather than the zone rule.
    #[test]
    fn a_continent_highlight_is_its_cosmic_rectangle() {
        let map = built();
        let (left, top, right, bottom) = map.continent(0).expect("it").rect_on_cosmic();
        let middle = ((left + right) / 2.0, (top + bottom) / 2.0);
        let hit = map
            .highlight(MapView::Cosmic, middle.0, middle.1)
            .expect("Kalimdor");
        assert_eq!(hit.target, HighlightTarget::Continent(0));
        assert_eq!(hit.directory, "Kalimdor");
        assert!((hit.offset.0 - left).abs() < 1e-6 && (hit.offset.1 - top).abs() < 1e-6);
        assert!((hit.size.0 - (right - left)).abs() < 1e-6);
        assert!((hit.size.1 - (bottom - top)).abs() < 1e-6);
        // 26 tiles across against 44 down, so the sheet is filled downwards.
        assert_eq!(hit.tex_percentage.1, 1.0);
        assert!((hit.tex_percentage.0 - 26.0 / 44.0).abs() < 1e-6);
        // …and open space on the parchment highlights nothing.
        assert_eq!(map.highlight(MapView::Cosmic, 0.99, 0.99), None);
    }

    /// A zone map highlights nothing, which is the branch that answers
    /// six zeroes: its label is `WorldMapOverlay`'s and no sub-area
    /// highlight art is shipped.
    #[test]
    fn a_zone_map_highlights_nothing() {
        let map = built();
        assert_eq!(map.highlight(MapView::Zone(0, 0), 0.5, 0.5), None);
    }

    /// Either table absent is no table — [`WorldMap::parse`]'s own argument.
    /// The overlays are the exception: a build with none is a map whose
    /// zones are blank, which is a real state of the game.
    #[test]
    fn a_half_built_table_is_no_table() {
        assert!(WorldMap::parse(&[], &dbc(&[], 13, b"\0"), &[], None).is_none());
        assert!(WorldMap::parse(&dbc(&[], 8, DIRS), &[], &[], None).is_none());
        assert_eq!(built().counts().2, 0, "no overlay table is no overlays");
    }

    // --- the overlays ---

    /// `AreaTable` for the overlay tests: three areas with distinct explore
    /// bits, mirroring [`crate::tables::area`]'s own fixture (25 fields, parent at 2,
    /// bit at 3, name at 11).
    fn overlay_areas() -> Areas {
        let row = |id: u32, bit: u32, name: u32| {
            let mut row = vec![0u32; 25];
            row[0] = id;
            row[1] = 1;
            row[2] = 215;
            row[3] = bit;
            row[11] = name;
            row
        };
        // `\0Kalimdor\0Durotar\0Mulgore\0` — the names are reused as labels.
        Areas::parse(&dbc(
            &[row(404, 7, 10), row(405, 8, 18), row(406, 2047, 1)],
            25,
            DIRS,
        ))
        .expect("the table parses")
    }

    /// An overlay is shown when any one of its four areas is explored —
    /// the client's loop — and it is shown on the zone map and nowhere else.
    #[test]
    fn an_overlay_needs_one_of_its_four_areas_explored() {
        // Two rows on Durotar's parchment (`WorldMapArea` id 4): one revealed by
        // area 404, one by 405 — and one on Mulgore's, which must never appear.
        let map = built_with(&[
            overlay_row(1, 4, [0, 404, 0, 0], 10),
            overlay_row(2, 4, [405, 0, 0, 0], 18),
            overlay_row(3, 9, [404, 0, 0, 0], 1),
        ]);
        assert_eq!(map.counts().2, 3);
        let areas = overlay_areas();
        let view = MapView::Zone(0, 0); // Durotar
        let seen = |bits: &[u32]| {
            let bits = bits.to_vec();
            let mask = move |bit: u32| bits.contains(&bit);
            map.overlays(view, Some(&areas), Some(&mask))
                .iter()
                .map(|overlay| overlay.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(seen(&[]), Vec::<u32>::new(), "a fresh character sees none");
        assert_eq!(seen(&[7]), vec![1], "area 404's bit");
        assert_eq!(seen(&[8]), vec![2], "area 405's bit");
        assert_eq!(seen(&[7, 8]), vec![1, 2], "in file order");
        // …and Mulgore's row is on Mulgore's parchment even though its area is
        // explored — the client's `mapAreaId` test.
        assert!(!seen(&[7]).contains(&3));

        // An unknown mask or an unknown table is the fresh character's map,
        // rather than everybody's — see [`WorldMap::overlays`].
        let all = |_: u32| true;
        assert!(map.overlays(view, None, Some(&all)).is_empty());
        assert!(map.overlays(view, Some(&areas), None).is_empty());
        // …and every other view has none at all, which is what makes
        // `GetNumMapOverlays` zero on a continent.
        assert!(map
            .overlays(MapView::Continent(0), Some(&areas), Some(&all))
            .is_empty());
        assert!(map
            .overlays(MapView::Cosmic, Some(&areas), Some(&all))
            .is_empty());
    }

    /// An `areaId` the table does not have is skipped, not accepted —
    /// the client goes on to the next of the four.
    #[test]
    fn an_area_the_table_does_not_have_reveals_nothing() {
        let map = built_with(&[overlay_row(1, 4, [9999, 0, 0, 0], 10)]);
        let areas = overlay_areas();
        let all = |_: u32| true;
        assert!(map
            .overlays(MapView::Zone(0, 0), Some(&areas), Some(&all))
            .is_empty());
    }

    /// The label on a zone map is an explored overlay's hit rectangle, and
    /// an unexplored one names nothing even where its rectangle is.
    ///
    /// The fixture's rectangle is x 110..390 of 1002 and y 60..300 of 668.
    #[test]
    fn the_zone_label_is_the_explored_overlay_under_the_pointer() {
        let map = built_with(&[overlay_row(1, 4, [404, 0, 0, 0], 10)]);
        let areas = overlay_areas();
        let view = MapView::Zone(0, 0);
        let seen = |bit: bool| move |_: u32| bit;
        let inside = (250.0 / PARCHMENT_WIDTH, 180.0 / PARCHMENT_HEIGHT);
        let label = |u: f32, v: f32, explored: bool| {
            let mask = seen(explored);
            map.overlay_label(view, u, v, Some(&areas), Some(&mask))
        };
        assert_eq!(label(inside.0, inside.1, true).as_deref(), Some("Durotar"));
        // Just outside each edge.
        assert_eq!(label(100.0 / PARCHMENT_WIDTH, inside.1, true), None);
        assert_eq!(label(400.0 / PARCHMENT_WIDTH, inside.1, true), None);
        assert_eq!(label(inside.0, 50.0 / PARCHMENT_HEIGHT, true), None);
        assert_eq!(label(inside.0, 320.0 / PARCHMENT_HEIGHT, true), None);
        // …and an unexplored overlay has no rectangle at all.
        assert_eq!(label(inside.0, inside.1, false), None);
    }

    /// A row on a map that is no continent's map is shown on its own: the
    /// first such row in file order is the map's, its directory and rectangle
    /// are the view's, its overlays are drawn, and it has no labels, no
    /// highlight and the indices -1 and 0.
    #[test]
    fn a_map_with_no_continent_shows_its_own_row() {
        // `\0Kalimdor\0Durotar\0AlteracValley\0Second\0`
        let dirs: &[u8] = b"\0Kalimdor\0Durotar\0AlteracValley\0Second\0";
        let areas = [
            area_row(13, 1, 0, 1, [16000.0, -16000.0, 16000.0, -16000.0]),
            area_row(4, 1, 14, 10, [-1962.5, -7250.0, 1808.333, -1716.667]),
            // Two rows on map 30, which has no continent; the first in the
            // file is the map's.
            area_row(401, 30, 2597, 18, [1781.25, -2456.25, 1085.417, -1739.583]),
            area_row(400, 30, 2598, 32, [100.0, -100.0, 100.0, -100.0]),
        ];
        let continents = [continent_row(2, 1, [23, 48, 9, 52], [-19.0, -0.322, 0.75])];
        let overlays = [overlay_row(1, 401, [404, 0, 0, 0], 18)];
        let map = WorldMap::parse(
            &dbc(&areas, 8, dirs),
            &dbc(&continents, 13, b"\0"),
            &dbc(&overlays, 17, dirs),
            None,
        )
        .expect("the tables parse");
        assert_eq!(map.single_for(30), Some(401), "file order, not id order");
        assert_eq!(map.single_for(1), None, "a continent's map");
        assert_eq!(map.single_for(42), None, "a map with no row");
        let view = MapView::Single(401);
        assert_eq!((view.continent_index(), view.zone_index()), (-1, 0));
        assert_eq!(map.directory(view), Some("AlteracValley"));
        // The middle of the rectangle is the middle of the map, on map 30
        // only.
        let (x, y) = ((1085.417 - 1739.583) / 2.0, (1781.25 - 2456.25) / 2.0);
        let (u, v) = map.position(view, 30, x, y).expect("on the map");
        assert!((u - 0.5).abs() < 1e-3 && (v - 0.5).abs() < 1e-3, "{u}, {v}");
        assert_eq!(map.position(view, 1, x, y), None);
        // The overlays are drawn and name nothing; nothing is highlighted.
        let areas = overlay_areas();
        let all = |_: u32| true;
        assert_eq!(map.overlays(view, Some(&areas), Some(&all)).len(), 1);
        let inside = (250.0 / PARCHMENT_WIDTH, 180.0 / PARCHMENT_HEIGHT);
        assert_eq!(map.overlay_label(view, inside.0, inside.1, Some(&areas), Some(&all)), None);
        assert_eq!(map.highlight(view, inside.0, inside.1), None);
    }

    /// The art is cut into 256-pixel pieces, and the count is what
    /// `WorldMapFrame_Update`'s two `ceil`s come to — including the one row and
    /// column a zero-sized overlay would otherwise have none of.
    #[test]
    fn an_overlays_art_is_cut_into_two_hundred_and_fifty_six_pixel_pieces() {
        let map = built_with(&[overlay_row(1, 4, [404, 0, 0, 0], 10)]);
        let overlay = &map.all_overlays()[0];
        assert_eq!(overlay.tiles(), (2, 2), "300 x 260");
        assert_eq!(
            overlay.piece("Durotar", 3),
            r"Interface\WorldMap\Durotar\Durotar3"
        );
        let mut empty = overlay.clone();
        empty.width = 0;
        empty.height = 0;
        assert_eq!(empty.tiles(), (1, 1), "never zero pieces");
    }
}
