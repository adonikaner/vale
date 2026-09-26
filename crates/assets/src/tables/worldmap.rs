//! **The world map**: which parchment is showing, and where on it a world
//! position lands.
//!
//! Two tables and one page of arithmetic, all of it the client's own rule,
//! because none of it crosses the wire and neither file states it.
//!
//! ```text
//! WorldMapArea.dbc        51 rows x 8 fields
//!   [0] id  [1] mapId  [2] areaId  [3] areaName — the art directory
//!   [4] locLeft (y1)   [5] locRight (y2)   [6] locTop (x1)   [7] locBottom (x2)
//!
//! WorldMapContinent.dbc    2 rows x 13 fields
//!   [0] id  [1] mapId  [2..5] left/right/top/bottom tile
//!   [6] offsetX  [7] offsetY  [8] scale  [9..12] unread (see below)
//! ```
//!
//! …and **one file that is not a table**: `Interface\WorldMap\Kalimdor.zmp` and
//! `Azeroth.zmp`, 65,536 bytes each, which are 128x128 little-endian `AreaTable`
//! ids and are the *only* thing that says which zone a point on a continent map
//! is over. See [`WorldMap::load_zone_grids`].
//!
//! ## The three views, and the state behind them
//!
//! The client keeps two integers, continent and zone, and every map
//! function in the interface is a read of them:
//!
//! ```text
//! continent  zone     what is showing        GetMapInfo answers
//! -1         -        the cosmic world map   nothing — WorldMapFrame_Update falls back to "World"
//!  i         -1       a continent            the continent's own WorldMapArea directory
//!  i          j       a zone                 that zone's directory
//! ```
//!
//! `GetCurrentMapContinent` and `GetCurrentMapZone` are those two **plus one**,
//! which is why `WorldMapFrame_Update` compares the continent against **0** to decide whether the zoom-out button is
//! enabled. [`MapView`] is that state.
//!
//! ## The continent table is built from `WorldMapArea`, not from a list
//!
//! The client's builder walks `WorldMapArea.dbc` twice:
//!
//! * a row with **`areaId == 0` is a continent** — its `mapId` names the
//!   continent and its own id is the map drawn when no zone is selected;
//! * a row with the **same `mapId` and a non-zero `areaId` is one of its
//!   zones**, and the zone list is sorted before anything reads
//!   it, so `GetMapZones` order is not file order.
//!
//! The continent's *name* is `Map.dbc`'s localised name for its `mapId`
//! (`+0x10 + locale*4`, which is field 4); a zone's name is
//! `AreaTable.dbc`'s (`+0x2c` = field 11). Neither is in
//! `WorldMapArea` — field 3 there is the **art directory**, which is what
//! `GetMapInfo` answers and what `Interface\WorldMap\<dir>\<dir>N` is built
//! from.
//!
//! ## Where a position lands, both branches
//!
//! On a **continent or zone map** it is one lerp per axis over the row's own
//! rectangle, with the axes crossed the way this game's coordinates are (world
//! `y` runs west, world `x` runs north):
//!
//! ```text
//! u = 1 - (pos.y - locRight)  / (locLeft - locRight)
//! v = 1 - (pos.x - locBottom) / (locTop  - locBottom)
//! ```
//!
//! and **(0, 0) if either denominator is zero, if the numerator is zero, or if
//! the result leaves 0..1** — which is what `GetPlayerMapPosition` answering
//! `0, 0` means to `WorldMapButton_OnUpdate`, where it hides the arrow. The row
//! is also required to be on the unit's own map.
//!
//! On the **cosmic map** there is no rectangle, because there is no
//! `WorldMapArea` row for it; the continent is placed on the parchment by its
//! own tile offset and scale:
//!
//! ```text
//! u = 0.5 + offsetX * (16/1002) - (pos.y / 533.33333) * scale * (16/1002)
//! v = 0.5 + offsetY * (16/668)  - (pos.x / 533.33333) * scale * (16/668)
//! ```
//!
//! The two constants are 0.015968064 and
//! 0.023952097, which are 16/1002 and 16/668, the world-map parchment's own
//! pixel dimensions over a 16-pixel unit. **The four floats at fields 9..12 are
//! not read by any of this**: the same builder derives the continent's rectangle
//! *on the cosmic map* from fields 2..8 instead ([`Continent::rect_on_cosmic`]),
//! and that derivation is what puts Kalimdor at u 0.089..0.400 and Eastern
//! Kingdoms at 0.624..0.923 — which is where they are in a screenshot.
//!
//! ## The highlight under the pointer, and what it is *not*
//!
//! `UpdateMapHighlight(x, y)` answers eight values: a name, the art directory,
//! two texture fractions, a size and an offset. **Its three branches are the
//! three views**, and the first thing it does is read the zone integer: a
//! zone below 0 skips the overlays.
//!
//! * On a **zone map** it walks `WorldMapOverlay.dbc`'s hit rectangles (fields
//!   13..16, `hitRectTop/Left/Bottom/Right`, scaled by 1/668 and 1/1002)
//!   and, on a hit, answers **the sub-area's name and
//!   nothing else**: the name and then six literal zeroes.
//!   There is no sub-area highlight art in the archives to go with it.
//! * On a **continent map** the zone under the pointer is highlighted, and on
//!   the **cosmic map** the continent is. Both take the directory from
//!   `WorldMapArea` field 3 — `Interface\WorldMap\Elwynn\
//!   ElwynnHighlight.blp` and `…\Azeroth\AzerothHighlight.blp`, which is where
//!   the art actually is.
//!
//! So **`WorldMapOverlay` is the label on a zone map and not the highlight**,
//! and the highlight needs no table this file was not already reading. See
//! [`WorldMap::highlight`] for the geometry.
//!
//! ## The overlays — the explored half of a zone map
//!
//! A zone's parchment ships **blank**, and everything drawn on it is a
//! `WorldMapOverlay` row the character has explored: the roads, the villages,
//! the lakes and the named corners. 526 rows, 17 fields, and the layout below
//! follows `GetMapOverlayInfo` rather than being inferred —
//! the function reads exactly `+0x20`, `+0x24`, `+0x28`, `+0x2c`, `+0x30`,
//! `+0x18`, `+0x1c` and returns them in that order, which pins seven of the
//! seventeen outright:
//!
//! ```text
//! WorldMapOverlay.dbc     526 rows x 17 fields, 0x44 bytes each
//!   [ 0] id
//!   [ 1] mapAreaId       the WorldMapArea row this belongs to
//!   [ 2..5] areaId[4]    AreaTable ids — **any** one explored shows it
//!   [ 6] mapPointX  [ 7] mapPointY
//!   [ 8] textureName     Interface\WorldMap\<directory>\<textureName>N
//!   [ 9] textureWidth   [10] textureHeight
//!   [11] offsetX        [12] offsetY
//!   [13..16] hitRect Top, Left, Bottom, Right
//! ```
//!
//! **Which rows are shown is the client's loop and it is copied rather than
//! guessed at.** For every row whose `mapAreaId` is the row being drawn, it
//! walks the four `areaId`s and takes the row the moment one of them is
//! explored — the bit being `AreaTable`'s own `areaBit`
//! ([`crate::tables::area::Area::explore_bit`]) tested against
//! `PLAYER_EXPLORED_ZONES`. An `areaId` of 0 or one the table does not have is
//! skipped rather than accepted, and a row all four of whose areas are unknown
//! is not shown at all.
//!
//! …and the **label** on a zone map is the same list: `UpdateMapHighlight`'s
//! `zone >= 0` branch walks the *explored* overlays' hit rectangles
//! and answers the sub-area's name with six literal zeroes after it, because no
//! sub-area highlight art is shipped. That is [`WorldMap::overlay_label`].
//!
//! ## What is still deliberately not here
//!
//! **Landmarks.** `GetNumMapLandmarks` is the POI pin layer and it is server
//! state of a different kind — `SMSG_WORLD_MAP_POI`-shaped, which vmangos does
//! not send in 1.12 — so it stays a stub answering 0.

use crate::tables::area::Areas;
use crate::tables::dbc::Dbc;
use std::collections::HashMap;

mod area_fields {
    pub const MAP_ID: usize = 1;
    pub const AREA_ID: usize = 2;
    /// The **art directory**, not a name to show — see the module comment.
    pub const DIRECTORY: usize = 3;
    pub const LOC_LEFT: usize = 4;
    pub const LOC_RIGHT: usize = 5;
    pub const LOC_TOP: usize = 6;
    pub const LOC_BOTTOM: usize = 7;
}

mod overlay_fields {
    pub const MAP_AREA_ID: usize = 1;
    /// `areaId[4]` — fields 2..5, and **any** one explored shows the row.
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

mod continent_fields {
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

/// **The parchment, in the pixels every overlay is authored against** —
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

/// **Which parchment is showing.** The client's own two integers, named.
///
/// `Cosmic` is `continent == -1`; a `Zone`'s indices are both **zero-based**
/// here and one-based across the Lua boundary, exactly as the client keeps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapView {
    /// Both continents at once — no `WorldMapArea` row, art directory "World".
    #[default]
    Cosmic,
    Continent(usize),
    Zone(usize, usize),
}

impl MapView {
    /// `GetCurrentMapContinent()` — the client's integer plus one.
    pub fn continent_index(self) -> usize {
        match self {
            MapView::Cosmic => 0,
            MapView::Continent(c) | MapView::Zone(c, _) => c + 1,
        }
    }

    /// `GetCurrentMapZone()` — likewise.
    pub fn zone_index(self) -> usize {
        match self {
            MapView::Cosmic | MapView::Continent(_) => 0,
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

/// **One `WorldMapOverlay` row** — a picture of an explored corner of a zone.
///
/// Every measurement here is in parchment pixels ([`PARCHMENT_WIDTH`] by
/// [`PARCHMENT_HEIGHT`]), which is the space `WorldMapFrame_Update` lays them
/// out in and the space the hit rectangles are compared in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapOverlay {
    pub id: u32,
    /// The [`MapArea`] row this belongs to — the zone whose parchment it is on.
    pub map_area: u32,
    /// **The four areas that reveal it**, zeroes and all: any one explored shows
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
/// that a **check** can ask the archives whether every piece is there. See
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
    /// **`Interface\WorldMap\<directory>.zmp`, resolved to zone ids** — see
    /// [`WorldMap::load_zone_grids`]. `None` until it is loaded, and for a
    /// continent that ships none.
    zone_grid: Option<Vec<u32>>,
}

impl Continent {
    /// **Where this continent sits on the cosmic parchment**, as
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

    /// **The zone standing at a world position**, out of the `.zmp` grid — an
    /// `AreaTable` zone id, or `None` for a cell that names nothing and for a
    /// continent with no grid.
    ///
    /// The client's rule, with one term negated:
    ///
    /// ```text
    /// row = ftol(-(0.5 - x/34133.332) * -128)      ; north to south
    /// col = ftol( (0.5 - y/34133.332) *  128)      ; west to east
    /// cell = grid[row * 128 + col]
    /// ```
    ///
    /// The double negative is the client's own and cancels; it is written out
    /// here because
    /// getting it backwards flips the map north to south and still answers a
    /// zone.
    pub fn zone_at_position(&self, x: f32, y: f32) -> Option<u32> {
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

    /// Where a world position on this continent falls on the **cosmic** map.
    ///
    /// Unlike [`MapArea::position`] this is not clamped to the parchment: the
    /// client's cosmic branch has no range test at all, because the whole world
    /// is on it by construction.
    pub fn position_on_cosmic(&self, x: f32, y: f32) -> (f32, f32) {
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
    /// **In file order**, which is the order the client walks them and therefore
    /// the order `GetMapOverlayInfo`'s index means. Nothing sorts it.
    overlays: Vec<MapOverlay>,
}

impl WorldMap {
    /// Parse and build. `areas` decides the **order** of each continent's zone
    /// list, because the client sorts it by the `AreaTable` name — so a build
    /// with no `AreaTable` keeps file order and says so rather than inventing
    /// one.
    ///
    /// `None` when either table is missing, on `Skills::parse`'s argument: a
    /// `WorldMap` with areas and no continents is indistinguishable from a
    /// correctly-read table for a game with one map.
    /// `world_map_overlay` is the third and is **optional on its own terms**: a
    /// build without it is a map whose zone parchments are blank, which is
    /// exactly what an unexplored one looks like, so it degrades into a real
    /// state of the game rather than into a wrong one.
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

        // **A continent is a `WorldMapArea` row with no area** — the client
        // skips every row whose field 2 is non-zero before it takes one.
        let mut continents: Vec<Continent> = Vec::new();
        let mut whole_rows: Vec<&MapArea> =
            areas_by_id.values().filter(|row| row.area == 0).collect();
        // **By id, where the builder walks the file.** The two agree in the
        // shipped table — Kalimdor is record 3 with id 13 and Azeroth record 4
        // with id 14 — so `GetMapContinents()` answers "Kalimdor", "Eastern
        // Kingdoms" either way, which is the order the drop-down shows. Sorting
        // is what makes this build reproducible from a `HashMap`; it is not a
        // claim that the client sorts.
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
            // **A continent with no `WorldMapContinent` row is still a
            // continent**: the builder writes its map, its row and its zones
            // before it looks, and only the cosmic geometry is missing. A zero
            // scale then makes every cosmic position collapse to the centre,
            // which is visible rather than wrong.
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

        Some(WorldMap {
            areas: areas_by_id,
            continents,
            overlays: parse_overlays(world_map_overlay),
        })
    }

    /// **Load the two `.zmp` grids** — the file that actually answers "which
    /// zone is the pointer over", and the one thing on this panel that is not a
    /// DBC.
    ///
    /// The client builds `Interface\WorldMap\%s.zmp` from the continent's own
    /// art directory and reads **`0x10000` bytes** of it: 128x128
    /// little-endian `AreaTable` ids covering the whole 64x64 tile map. The
    /// client then walks all 16,384 of them once at load, replacing
    /// each with the enclosing **zone** — `if (parentAreaId) id = parentAreaId`
    /// — and then with the `WorldMapArea` row that has it. This
    /// keeps the zone id rather than the row, because the row's *index* depends
    /// on a sort this file also owns.
    ///
    /// `read` answers by **archive path**, which is why this is a second step
    /// rather than an argument to [`WorldMap::parse`]: every other input here is
    /// a table asked for by name.
    ///
    /// **A continent with no grid keeps the rectangle rule** in
    /// [`WorldMap::zone_at`], which is a documented degradation and not a
    /// silence: it is right for 32 of the 46 zones and names a neighbour for the
    /// rest, since map rectangles overlap heavily and nothing in the two tables
    /// breaks the tie.
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
                        // The sub-area's enclosing zone, which is the whole of
                        // what the continent map names. With no `AreaTable` the
                        // id stands as it is — a sub-area then matches no row
                        // and reads as nothing, which is visible rather than
                        // wrong.
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

    /// The `WorldMapArea` row a view is drawn from — `None` for the cosmic map,
    /// which has none. That `None` is the whole reason `WorldMapFrame_Update`
    /// carries `if ( not mapFileName ) then mapFileName = "World"; end`.
    pub fn area_for(&self, view: MapView) -> Option<&MapArea> {
        let id = match view {
            MapView::Cosmic => return None,
            MapView::Continent(c) => self.continent(c)?.whole,
            MapView::Zone(c, z) => *self.continent(c)?.zones.get(z)?,
        };
        self.areas.get(&id)
    }

    pub fn area(&self, id: u32) -> Option<&MapArea> {
        self.areas.get(&id)
    }

    /// `GetMapInfo`'s first answer: the art directory, or `None` for the cosmic
    /// map.
    pub fn directory(&self, view: MapView) -> Option<&str> {
        self.area_for(view).map(|area| area.directory.as_str())
    }

    /// **Where a world position lands on the view that is showing.**
    ///
    /// `None` when the position is not on it — off the rectangle, on a different
    /// map, or on a cosmic view whose continent this position is not on. Every
    /// one of those is `GetPlayerMapPosition` answering `0, 0`.
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

    /// **Which zone of `continent` a point on its map is over**, as a zone index
    /// — what `UpdateMapHighlight` names and what a click zooms into.
    ///
    /// The point is taken *back* into world yards through the
    /// continent's own rectangle, and the answer is read out of the `.zmp` grid
    /// at half-tile resolution. Nothing is projected and nothing overlaps —
    /// which matters, because the map rectangles overlap so heavily that a
    /// rectangle test names Elwynn Forest over Stormwind and Dun Morogh over
    /// half of Loch Modan.
    ///
    /// That rectangle test is the fallback for a continent with no grid, and it
    /// agrees with the grid for 32 of the game's 46 zones. See
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

        // **A grid that says nothing is an answer** — open ocean — so the
        // fallback is chosen on the grid's presence and never on its cell.
        if continent_row.zone_grid.is_some() {
            let zone = continent_row.zone_at_position(x, y)?;
            return continent_row
                .zones
                .iter()
                .position(|id| self.areas.get(id).is_some_and(|row| row.area == zone));
        }
        // No grid: ask each zone's rectangle whether it contains the point, and
        // take the first in the sorted list — an arbitrary tie-break, stated.
        continent_row.zones.iter().position(|id| {
            self.areas.get(id).is_some_and(|zone| {
                (zone.right..=zone.left).contains(&y) && (zone.bottom..=zone.top).contains(&x)
            })
        })
    }

    /// **Which continent a point on the cosmic parchment is over.**
    ///
    /// The client's `continent == -1` branch, which tests the cursor against
    /// each continent's cached [`Continent::rect_on_cosmic`] and takes the first
    /// that contains it. Shared by the highlight and by `ProcessMapClick`, which
    /// are the same question asked by two verbs.
    pub fn continent_at(&self, u: f32, v: f32) -> Option<usize> {
        self.continents.iter().position(|continent| {
            let (left, top, right, bottom) = continent.rect_on_cosmic();
            (left..=right).contains(&u) && (top..=bottom).contains(&v)
        })
    }

    /// **`UpdateMapHighlight`'s other seven answers** — the art to lay over the
    /// thing the pointer is on, or `None` for a pointer over nothing.
    ///
    /// The three views are three answers:
    ///
    /// * a **zone map** highlights nothing. Its label is `WorldMapOverlay`'s and
    ///   the branch that answers it pushes six literal zeroes after the name,
    ///   because no sub-area highlight art is shipped.
    /// * a **continent map** highlights the zone under the pointer, sized and
    ///   placed by that zone's own rectangle inside the continent's — the same
    ///   lerp [`MapArea::position`] makes, taken over the corners rather than
    ///   over a point.
    /// * the **cosmic map** highlights the continent under the pointer, at
    ///   exactly [`Continent::rect_on_cosmic`].
    ///
    /// [`MapHighlight::tex_percentage`] is the one part that is not geometry;
    /// see its own note.
    pub fn highlight(&self, view: MapView, u: f32, v: f32) -> Option<MapHighlight> {
        match view {
            // The label here is the overlays' and there is no art — see above.
            MapView::Zone(_, _) => None,
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
                    // **1.0 across and a computed fraction down**, which is the
                    // asymmetry the client itself has: it uses the
                    // literal 1.0 and only the second is measured.
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
                // **The continent's fit is its tile grid's aspect**, not the
                // fraction rule above: the client compares the two spans and
                // gives the longer one the whole sheet. The tile counts are
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

    /// **Every overlay the table has**, in file order — for the checks, which
    /// ask about all 526 rather than about one zone's.
    pub fn all_overlays(&self) -> &[MapOverlay] {
        &self.overlays
    }

    /// **The overlays to draw on the view that is showing** — the client's loop,
    /// in the order `GetMapOverlayInfo`'s one-based index means.
    ///
    /// Empty for the cosmic and continent parchments, which is the client's own
    /// first branch: it compares the zone integer against the continent's
    /// and leaves the list at zero when no zone is showing. So `GetNumMapOverlays`
    /// is 0 anywhere but a zone map, which is what makes the continent view draw
    /// its own single picture and nothing over it.
    ///
    /// `explored` is the character's mask and `areas` is `AreaTable` — the two
    /// halves of the bit test. With **either** missing nothing is shown, because
    /// the honest answer for a client that does not know what has been explored
    /// is the fresh character's map rather than everybody's.
    pub fn overlays(
        &self,
        view: MapView,
        areas: Option<&Areas>,
        explored: Option<&ExplorationMask>,
    ) -> Vec<&MapOverlay> {
        let (MapView::Zone(_, _), Some(areas), Some(explored)) = (view, areas, explored) else {
            return Vec::new();
        };
        let Some(row) = self.area_for(view) else {
            return Vec::new();
        };
        self.overlays
            .iter()
            .filter(|overlay| overlay.map_area == row.id)
            .filter(|overlay| {
                // **Any of the four, and an id the table does not have is
                // skipped rather than accepted** — the client goes on to the
                // next `areaId`, not to the accepting branch.
                overlay.areas.iter().any(|id| {
                    areas
                        .get(*id)
                        .is_some_and(|area| explored(area.explore_bit))
                })
            })
            .collect()
    }

    /// **What `GetMapOverlayInfo` answers for one of them** — the seven values,
    /// with the texture as the whole path bar its piece number.
    ///
    /// Here rather than at the Lua boundary because the directory is
    /// `WorldMapArea`'s and only this type holds both tables — the same argument
    /// [`WorldMap::area_for`] is here for.
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

    /// **The sub-area under the pointer on a zone map**, out of the same list —
    /// `UpdateMapHighlight`'s zone branch, which answers a name and six
    /// literal zeroes.
    ///
    /// The name is the overlay's *first* area that has one, because the label is
    /// `AreaTable`'s and an overlay names up to four.
    pub fn overlay_label(
        &self,
        view: MapView,
        u: f32,
        v: f32,
        areas: Option<&Areas>,
        explored: Option<&ExplorationMask>,
    ) -> Option<String> {
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

/// **What this crate needs from the character's exploration mask**, and nothing
/// more: "has this `areaBit` been seen?".
///
/// A borrowed closure rather than the type itself, because the mask is
/// `vale_protocol::play::explored::Explored` — update-field state, in a crate
/// neither of these two depends on. A trait here could not be implemented for it
/// anywhere (the orphan rule), and a copy of the bitset in this crate would be
/// the same 256 bytes with two owners.
pub type ExplorationMask<'a> = dyn Fn(u32) -> bool + 'a;

/// **The path `GetMapOverlayInfo` answers with**, without the piece number that
/// `WorldMapFrame_Update` appends.
///
/// The client's own format string, `Interface\WorldMap\%s\%s`, and its
/// fallback for an overlay whose `WorldMapArea` row does not resolve —
/// `Interface\WorldMap\World\%s`. The fallback is unreachable in the shipped
/// table (`vale zones` reports 526 of 526 naming a row that exists) and is
/// copied because it is the function's other branch.
fn path_in(directory: &str, texture: &str) -> String {
    let directory = if directory.is_empty() {
        "World"
    } else {
        directory
    };
    format!("Interface\\WorldMap\\{directory}\\{texture}")
}

/// `WorldMapOverlay.dbc`, in file order — see the module comment for the layout
/// and where each field index came from.
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

/// **What fraction of a zone's square highlight sheet the art fills**, down.
///
/// The client's own arithmetic, which is stranger than a ratio and has to be
/// copied rather than reasoned about: the shape's height is expressed against a
/// **128-wide** canvas, truncated to an integer by `_ftol`, and
/// then divided by [`sheet_canvas`] of that integer. For the 3:2 rectangle every
/// `WorldMapArea` row is authored at, that is `85 / 128` — and the shipped art
/// agrees: over the game's 46 zone sheets, decoded and measured, the lowest
/// texel with anything in it is **row 83 of 128** (the Barrens), which is under
/// that line and against none of the alternatives. The one exception is
/// Silithus, whose sheet fills 88x127 of its canvas; it is an AQ-era addition
/// authored to another convention, and since this is the client's own
/// arithmetic the reference presumably crops it too. That last clause is an
/// inference and the rest is measurement.
fn sheet_fraction(height: f32, width: f32) -> f32 {
    if width == 0.0 {
        return 1.0;
    }
    // `_ftol` chops, and so does `as i32`.
    let scaled = (height / width * ZONE_SHEET_UNIT) as i32;
    if scaled <= 0 {
        return 1.0;
    }
    scaled as f32 / sheet_canvas(scaled) as f32
}

/// `128.0` — the width the sheet fraction is measured against.
const ZONE_SHEET_UNIT: f32 = 128.0;

/// **The canvas [`sheet_fraction`] divides by**.
///
/// It is *nearly* "round up to a power of two" and deliberately is not: the
/// value is rounded up on its **low byte only**, and whatever was above 256 is
/// added back rounded up to the next whole 256. A low byte of zero skips both
/// and answers the value itself. Nothing in the shipped table reaches the second
/// case — every zone lands on 85 — but it is copied rather than simplified,
/// because a rule this shaped is not one to re-derive from the cases that occur.
///
/// (The client also has a `canvas * 8 >= low` loop between the two, which can
/// never run: the power-of-two round-up already satisfies it. Left out.)
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

/// **What the pointer is over**, for the caller to name: a zone's name is
/// `AreaTable`'s and a continent's is `Map.dbc`'s, and neither table is here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightTarget {
    Zone { continent: usize, zone: usize },
    Continent(usize),
}

/// **The art `WorldMapButton_OnUpdate` lays over the hovered thing**, in the
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

    /// **Shared with [`crate::tables::areapoi`]'s tests**, which need a real
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

    /// **A row with no area is the continent, and every other row on its map is
    /// one of its zones** — the client's builder and the count loop after it.
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

    /// **The lerp, against a position whose answer is known by eye**: Orgrimmar
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

    /// **The cosmic branch places a continent by its tiles and scale**, which is
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
    /// the cross-check the two branches owe each other.
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

    /// **Which zone the pointer is over**, which is the whole of what
    /// `WorldMapFrameAreaLabel` shows — and the reason it said "BLAH!".
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

    /// **The `.zmp` grid decides, and it decides differently from the
    /// rectangles** — which is the whole reason it is read.
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

    /// **A sub-area in the grid resolves to its zone**, which is the walk
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

    /// **The highlight over a zone is that zone's own rectangle**, in the
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
    /// [`sheet_fraction`], which is where the strangeness is.
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

    /// **The cosmic map highlights a continent**, at the rectangle
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

    /// **A zone map highlights nothing**, which is the branch that answers
    /// six zeroes: its label is `WorldMapOverlay`'s and no sub-area
    /// highlight art is shipped.
    #[test]
    fn a_zone_map_highlights_nothing() {
        let map = built();
        assert_eq!(map.highlight(MapView::Zone(0, 0), 0.5, 0.5), None);
    }

    /// Either table absent is no table — [`WorldMap::parse`]'s own argument.
    /// **The overlays are the exception**: a build with none is a map whose
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

    /// **An overlay is shown when any one of its four areas is explored** —
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

    /// **An `areaId` the table does not have is skipped, not accepted** —
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

    /// **The label on a zone map is an explored overlay's hit rectangle**, and
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
