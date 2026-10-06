//! A flight map for a map that has none: the `WorldMapContinent.dbc` row the
//! 1.12.1 client needs before it opens the flight master's window, and the
//! picture it draws behind the nodes.
//!
//! ## Why a map needs a row of a world-map table to fly from
//!
//! The client lays the flight map out over a box in world yards, read from the
//! `WorldMapContinent` row whose map is the current node's map
//! (`vale_assets::tables::taxi`, `TaxiBox`). With no such row it opens no
//! window at all, and the shipped table has rows for maps 0 and 1 only. A row
//! for another map is read by the flight map and by the world map's
//! zoomed-out placement of the player (fields 6..8), and by nothing else unless
//! the map also has a `WorldMapArea` continent row.
//!
//! ```text
//! field  what [`continent`] writes
//! 2..5   the map's tile range, first index then second; unread for a map
//!        with no WorldMapArea row
//! 6, 7   -31.3125 and -20.875, which with scale 0 put the player at (0, 0)
//!        in the world map's zoomed-out view, where a map with no row puts it
//! 8      0
//! 9..12  the box: min x, min y, max x, max y
//! ```
//!
//! ## The box is square
//!
//! The client divides each axis by the other axis's span (see
//! `TaxiBox::project`), so only a square box draws the nodes where they are.
//! [`taxi_box`] covers every tile of the map and every node, adds a margin,
//! and widens the shorter side.
//!
//! ## The picture behind the nodes
//!
//! `Interface\TaxiFrame\TAXIMAP<map>.blp`, the name the client builds from the
//! current node's map id. The frame stretches the whole picture over its
//! 316x352 map area and places each node at the same fractions of it, so a
//! picture of exactly the box lines up with the nodes at any size.
//! [`picture`] draws it from the map's minimap tiles, north at the top and
//! west on the left, each pixel the mean of 4x4 samples.

use crate::dbc::{Cell, DbcFile, Row};
use std::collections::HashMap;
use vale_assets::tables::taxi::continent_fields as cf;
use vale_assets::world::adt::{MAP_ORIGIN, TILE_SIZE};

/// The table the row is in, by the name the caller's table map uses.
pub const TABLE: &str = "WorldMapContinent";

/// The side of the picture, in pixels: the shipped `TAXIMAP0.blp` and
/// `TAXIMAP1.blp` are 512x512.
pub const SIDE: u32 = 512;

/// The side of one minimap tile's picture, in pixels.
pub const TILE_SIDE: u32 = 256;

/// The share of the box's side left round what it covers.
const MARGIN: f32 = 0.05;

/// The least side a box is given, in yards, so a map of one node still shows
/// the ground round it.
const LEAST_SIDE: f32 = 600.0;

/// Fields 6 and 7 for a map with no world-map placement. With field 8 at 0,
/// the world map's zoomed-out view places the player at (0, 0) for these two
/// values, which is where it places the player on a map with no row.
const OFFSET_X: f32 = -31.3125;
const OFFSET_Y: f32 = -20.875;

/// The colour drawn where no minimap tile is.
const BACKGROUND: [u8; 3] = [40, 34, 26];

/// What a flight map row says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlightMap {
    pub map: u32,
    /// The tile range: first index min and max, then second index min and max.
    pub tiles: [u32; 4],
    /// min x, min y, max x, max y, in world yards.
    pub taxi_box: [f32; 4],
}

impl FlightMap {
    /// Whether a node at world `x`, `y` is inside the box, as the client tests
    /// it before drawing the node.
    pub fn holds(&self, x: f32, y: f32) -> bool {
        (self.taxi_box[0]..=self.taxi_box[2]).contains(&x) && (self.taxi_box[1]..=self.taxi_box[3]).contains(&y)
    }
}

/// The square box over `tiles` (column, row) and `nodes` (world x, y), with
/// the margin. `None` when both are empty.
pub fn taxi_box(tiles: &[(u32, u32)], nodes: &[[f32; 2]]) -> Option<[f32; 4]> {
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    let mut take = |x: f32, y: f32| {
        low = [low[0].min(x), low[1].min(y)];
        high = [high[0].max(x), high[1].max(y)];
    };
    for &(col, row) in tiles {
        // A tile's north edge is at the origin less `row` tiles, and its west
        // edge at the origin less `col` tiles.
        let north = MAP_ORIGIN - row as f32 * TILE_SIZE;
        let west = MAP_ORIGIN - col as f32 * TILE_SIZE;
        take(north, west);
        take(north - TILE_SIZE, west - TILE_SIZE);
    }
    for node in nodes {
        take(node[0], node[1]);
    }
    if !low[0].is_finite() {
        return None;
    }
    let centre = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0];
    let span = (high[0] - low[0]).max(high[1] - low[1]);
    let half = (span * (1.0 + 2.0 * MARGIN)).max(LEAST_SIDE) / 2.0;
    Some([centre[0] - half, centre[1] - half, centre[0] + half, centre[1] + half])
}

/// The flight map for `map`: the box over its tiles and nodes, and the tile
/// range. `None` when there are neither tiles nor nodes.
pub fn plan(map: u32, tiles: &[(u32, u32)], nodes: &[[f32; 2]]) -> Option<FlightMap> {
    let taxi_box = taxi_box(tiles, nodes)?;
    let range = |pick: fn(&(u32, u32)) -> u32| {
        let values = tiles.iter().map(pick);
        (values.clone().min().unwrap_or(0), values.max().unwrap_or(0))
    };
    let (first_low, first_high) = range(|tile| tile.0);
    let (second_low, second_high) = range(|tile| tile.1);
    Some(FlightMap { map, tiles: [first_low, first_high, second_low, second_high], taxi_box })
}

/// The row `map` has in the table, if it has one.
pub fn row_of(table: &DbcFile, map: u32) -> Option<usize> {
    (0..table.record_count()).find(|record| table.u32_at(*record, cf::MAP) == Some(map))
}

/// The flight map a row says.
pub fn read(table: &DbcFile, record: usize) -> Option<FlightMap> {
    let word = |field| table.u32_at(record, field);
    let float = |field| table.f32_at(record, field);
    Some(FlightMap {
        map: word(cf::MAP)?,
        tiles: [word(cf::LEFT_TILE)?, word(cf::RIGHT_TILE)?, word(cf::TOP_TILE)?, word(cf::BOTTOM_TILE)?],
        taxi_box: [float(cf::TAXI_MIN_X)?, float(cf::TAXI_MIN_X + 1)?, float(cf::TAXI_MIN_X + 2)?, float(cf::TAXI_MIN_X + 3)?],
    })
}

/// What [`write`] did, for the undo stack: the row it added, or the fields it
/// changed in the row that was there.
#[derive(Debug, Clone, PartialEq)]
pub enum Written {
    Added(Row),
    Changed(Vec<Cell>),
}

/// Why [`write`] wrote nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum Refused {
    /// The table is not the width 1.12 ships.
    WrongWidth,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::WrongWidth => write!(f, "{TABLE}.dbc is not the width 1.12 ships"),
        }
    }
}

/// The 11 fields after the id and the map, as words.
fn words(flight: &FlightMap) -> [(usize, u32); 11] {
    [
        (cf::LEFT_TILE, flight.tiles[0]),
        (cf::RIGHT_TILE, flight.tiles[1]),
        (cf::TOP_TILE, flight.tiles[2]),
        (cf::BOTTOM_TILE, flight.tiles[3]),
        (cf::OFFSET_X, OFFSET_X.to_bits()),
        (cf::OFFSET_Y, OFFSET_Y.to_bits()),
        (cf::SCALE, 0f32.to_bits()),
        (cf::TAXI_MIN_X, flight.taxi_box[0].to_bits()),
        (cf::TAXI_MIN_X + 1, flight.taxi_box[1].to_bits()),
        (cf::TAXI_MIN_X + 2, flight.taxi_box[2].to_bits()),
        (cf::TAXI_MIN_X + 3, flight.taxi_box[3].to_bits()),
    ]
}

/// Write the map's row: add one, or rewrite the one it has. The table is
/// written as it goes; the answer is what the undo stack needs.
pub fn write(table: &mut DbcFile, flight: &FlightMap) -> Result<Written, Refused> {
    if let Some(record) = row_of(table, flight.map) {
        let cells: Vec<Cell> = words(flight)
            .iter()
            .filter_map(|&(field, value)| Cell::new(table, record, field, value))
            .filter(Cell::moves)
            .collect();
        for cell in &cells {
            cell.apply(table);
        }
        return Ok(Written::Changed(cells));
    }
    let id = table.max_id() + 1;
    let bytes = table.blank_record(id);
    let record = table.push_record(&bytes).ok_or(Refused::WrongWidth)?;
    for (field, value) in [(cf::MAP, flight.map)].into_iter().chain(words(flight)) {
        if !table.set_u32(record, field, value) {
            table.remove_record(record);
            return Err(Refused::WrongWidth);
        }
    }
    Ok(Written::Added(Row::added(table, record).ok_or(Refused::WrongWidth)?))
}

/// The picture of the box, `SIDE` pixels square, as RGBA: north at the top
/// and west on the left. `tile` answers one minimap tile's RGBA pixels,
/// `TILE_SIDE` square, or `None` where there is none; it is asked once per
/// tile.
pub fn picture(taxi_box: [f32; 4], mut tile: impl FnMut(u32, u32) -> Option<Vec<u8>>) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let side = SIDE as f32;
    let span = [taxi_box[2] - taxi_box[0], taxi_box[3] - taxi_box[1]];
    let mut tiles: HashMap<(u32, u32), Option<Vec<u8>>> = HashMap::new();
    let mut out = vec![0u8; (SIDE * SIDE * 4) as usize];
    for v in 0..SIDE {
        for u in 0..SIDE {
            let mut sum = [0u32; 3];
            for sv in 0..SAMPLES {
                for su in 0..SAMPLES {
                    let fu = (u as f32 + (su as f32 + 0.5) / SAMPLES as f32) / side;
                    let fv = (v as f32 + (sv as f32 + 0.5) / SAMPLES as f32) / side;
                    // Down the picture is south (falling x), across it is
                    // east (falling y).
                    let x = taxi_box[2] - fv * span[0];
                    let y = taxi_box[3] - fu * span[1];
                    let across = (MAP_ORIGIN - y) / TILE_SIZE;
                    let down = (MAP_ORIGIN - x) / TILE_SIZE;
                    let colour = match (across >= 0.0 && across < 64.0, down >= 0.0 && down < 64.0) {
                        (true, true) => {
                            let at = (across as u32, down as u32);
                            let pixels = tiles.entry(at).or_insert_with(|| tile(at.0, at.1));
                            match pixels {
                                Some(pixels) if pixels.len() == (TILE_SIDE * TILE_SIDE * 4) as usize => {
                                    let tu = ((across.fract() * TILE_SIDE as f32) as u32).min(TILE_SIDE - 1);
                                    let tv = ((down.fract() * TILE_SIDE as f32) as u32).min(TILE_SIDE - 1);
                                    let i = ((tv * TILE_SIDE + tu) * 4) as usize;
                                    [pixels[i], pixels[i + 1], pixels[i + 2]]
                                }
                                _ => BACKGROUND,
                            }
                        }
                        _ => BACKGROUND,
                    };
                    for (total, value) in sum.iter_mut().zip(colour) {
                        *total += u32::from(value);
                    }
                }
            }
            let i = ((v * SIDE + u) * 4) as usize;
            let count = SAMPLES * SAMPLES;
            for c in 0..3 {
                out[i + c] = (sum[c] / count) as u8;
            }
            out[i + 3] = 255;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::tables::taxi::continent_fields::COUNT;

    /// A table of 13 fields with the two shipped rows' ids and maps.
    fn table() -> DbcFile {
        let size = COUNT * 4;
        let mut buf = Vec::new();
        buf.extend_from_slice(b"WDBC");
        for word in [2, COUNT as u32, size as u32, 1] {
            buf.extend_from_slice(&word.to_le_bytes());
        }
        for (id, map) in [(1u32, 0u32), (2, 1)] {
            let mut record = vec![0u8; size];
            record[0..4].copy_from_slice(&id.to_le_bytes());
            record[4..8].copy_from_slice(&map.to_le_bytes());
            buf.extend_from_slice(&record);
        }
        buf.push(0);
        DbcFile::parse(&buf).unwrap()
    }

    /// The box covers every tile and node, is square, and keeps a margin.
    #[test]
    fn the_box_is_square_and_covers_tiles_and_nodes() {
        let tiles = [(30, 30), (31, 30)];
        let node = [MAP_ORIGIN - 30.5 * TILE_SIZE, MAP_ORIGIN - 33.0 * TILE_SIZE];
        let b = taxi_box(&tiles, &[node]).unwrap();
        assert!((b[2] - b[0] - (b[3] - b[1])).abs() < 0.01, "square: {b:?}");
        // The tiles' corners and the node are inside.
        for (x, y) in [
            (MAP_ORIGIN - 30.0 * TILE_SIZE, MAP_ORIGIN - 30.0 * TILE_SIZE),
            (MAP_ORIGIN - 31.0 * TILE_SIZE, MAP_ORIGIN - 32.0 * TILE_SIZE),
            (node[0], node[1]),
        ] {
            assert!((b[0]..=b[2]).contains(&x) && (b[1]..=b[3]).contains(&y), "{x}, {y} outside {b:?}");
        }
        assert!(b[3] - b[1] > 3.0 * TILE_SIZE, "a margin round three tiles' width");
        assert_eq!(taxi_box(&[], &[]), None);
        let one = taxi_box(&[], &[[100.0, 200.0]]).unwrap();
        assert_eq!(one[2] - one[0], LEAST_SIDE);
    }

    /// A new map's row is added after the shipped two, and written again it
    /// changes only the fields that differ.
    #[test]
    fn a_row_is_added_and_then_rewritten() {
        let mut file = table();
        let flight = plan(534, &[(29, 29), (30, 31)], &[]).unwrap();
        assert_eq!(flight.tiles, [29, 30, 29, 31]);
        let Ok(Written::Added(_)) = write(&mut file, &flight) else { panic!("not added") };
        let record = row_of(&file, 534).unwrap();
        assert_eq!(file.u32_at(record, cf::ID), Some(3));
        assert_eq!(read(&file, record), Some(flight));
        assert_eq!(file.f32_at(record, cf::SCALE), Some(0.0));
        let moved = FlightMap { taxi_box: [0.0, 0.0, 10.0, 10.0], ..flight };
        let Ok(Written::Changed(cells)) = write(&mut file, &moved) else { panic!("not changed") };
        assert_eq!(cells.len(), 4, "the four box fields");
        assert_eq!(file.record_count(), 3);
        assert!(moved.holds(5.0, 5.0) && !moved.holds(11.0, 5.0));
    }

    /// North is at the top and west on the left: a box over one tile, whose
    /// picture is dark in its north-west quarter, is drawn dark at the top
    /// left.
    #[test]
    fn the_picture_has_north_at_the_top_and_west_on_the_left() {
        let north = MAP_ORIGIN - 30.0 * TILE_SIZE;
        let west = MAP_ORIGIN - 30.0 * TILE_SIZE;
        let taxi_box = [north - TILE_SIZE, west - TILE_SIZE, north, west];
        let mut pixels = vec![255u8; (TILE_SIDE * TILE_SIDE * 4) as usize];
        for v in 0..TILE_SIDE / 2 {
            for u in 0..TILE_SIDE / 2 {
                let i = ((v * TILE_SIDE + u) * 4) as usize;
                pixels[i..i + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
        let mut asked = Vec::new();
        let out = picture(taxi_box, |col, row| {
            asked.push((col, row));
            Some(pixels.clone())
        });
        let at = |u: u32, v: u32| out[((v * SIDE + u) * 4) as usize];
        assert_eq!(at(10, 10), 0, "north-west is dark");
        assert_eq!(at(SIDE - 10, 10), 255, "north-east is light");
        assert_eq!(at(10, SIDE - 10), 255, "south-west is light");
        assert_eq!(asked, vec![(30, 30)], "one tile, asked once");
    }
}
