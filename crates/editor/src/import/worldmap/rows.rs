//! The rows behind a map, as table edits undone with Ctrl+Z: a new zone map,
//! continent or single map; a row's rectangle and directory; a continent's
//! place on the whole world's picture; removing a map.
//!
//! ## What a new map adds
//!
//! ```text
//! zone map     a WorldMapArea row for a zone (an AreaTable row with no
//!              parent) on a continent's map
//! continent    a WorldMapArea row with area 0 for a map, and the map's
//!              WorldMapContinent row: the one a flight map made, or a new one
//!              over the map's tiles (`vale_edit::flightmap`), which also gives
//!              the map a flight map. A row still at the flight map's
//!              placement, scale 0, is placed in the middle of the whole
//!              world's picture at a scale that shows it
//! single map   a WorldMapArea row with an area on a map with no continent;
//!              the area is one of the map's zones, because a row with area 0
//!              is a continent
//! ```
//!
//! Each starts with a rectangle over the map's tiles; the panel then fits it
//! to what the map or the zone covers once the tiles are read (see
//! [`super::grid`]).
//!
//! ## A continent on the whole world's picture
//!
//! `WorldMapContinent`'s tile range, offsets and scale place the continent:
//! one tile is `16 * scale` of the picture's 1002 by 668 pixels, and the
//! offsets move it by 16 pixels a unit. `vale_assets::tables::worldmap`'s
//! `Continent::rect_on_cosmic` is the rule; [`default_placement`] centres a
//! new continent at a scale that makes it a third of the picture across or
//! down, whichever is reached first.

use super::{grid, map_directory, Entry, Kind, AREAS, CONTINENTS, OVERLAYS};
use crate::session::EditSession;
use crate::tables::zones::area;
use vale_assets::tables::taxi::continent_fields as cf;
use vale_assets::tables::worldmap::area_fields as af;
use vale_client::assets::GameAssets;
use vale_edit::dbc::{cell, Cell, Row};
use vale_edit::worldmap as pieces;

/// The New map form.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NewMap {
    /// Zone, Continent or Single.
    pub(crate) kind: Kind,
    /// The map, for a continent or a single map.
    pub(crate) map: Option<u32>,
    /// The zone, for a zone map or a single map.
    pub(crate) area: Option<u32>,
    pub(crate) directory: String,
}

impl Default for NewMap {
    fn default() -> NewMap {
        NewMap { kind: Kind::Zone, map: None, area: None, directory: String::new() }
    }
}

/// Write `words` to one row of `table` as one undo entry, or as part of the
/// entry for one drag when `gesture` names the drag and the time. Answers
/// whether anything changed.
pub(crate) fn set_words(
    session: &mut EditSession,
    table: &str,
    record: usize,
    words: &[(usize, u32)],
    label: &str,
    gesture: Option<(&str, f64)>,
) -> bool {
    let Some(file) = session.tables.get(table) else { return false };
    let cells: Vec<Cell> = words
        .iter()
        .filter_map(|&(field, value)| Cell::new(file, record, field, value))
        .filter(Cell::moves)
        .collect();
    if cells.is_empty() {
        return false;
    }
    match gesture {
        Some((subject, now)) => session.history.begin_gesture(label, subject, now),
        None => session.history.begin(label),
    }
    for cell in cells {
        if let Some(file) = session.tables.get_mut(table) {
            cell.apply(file);
        }
        session.history.record_cell(table, cell);
    }
    session.history.end();
    session.table_edited(table);
    true
}

/// Set a row's text field, as one undo entry.
fn set_text(session: &mut EditSession, table: &str, record: usize, field: usize, text: &str, label: &str) -> bool {
    let Some(file) = session.tables.get_mut(table) else { return false };
    let Some(edit) = cell::set_text(file, record, field, text) else { return false };
    session.history.begin(label);
    session.history.record_cell(table, edit);
    session.history.end();
    session.table_edited(table);
    true
}

/// The record of a `WorldMapArea` row by id.
pub(crate) fn record_of(session: &EditSession, id: u32) -> Option<usize> {
    session.table(AREAS)?.row_of(id)
}

/// Set a row's rectangle: left, right, top, bottom, as one undo entry for
/// one drag.
pub(crate) fn set_rect(session: &mut EditSession, id: u32, rect: [f32; 4], now: Option<f64>) -> bool {
    let Some(record) = record_of(session, id) else { return false };
    let words = [
        (af::LOC_LEFT, rect[0].to_bits()),
        (af::LOC_RIGHT, rect[1].to_bits()),
        (af::LOC_TOP, rect[2].to_bits()),
        (af::LOC_BOTTOM, rect[3].to_bits()),
    ];
    let subject = format!("map rectangle {id}");
    set_words(session, AREAS, record, &words, "Map rectangle", now.map(|now| (subject.as_str(), now)))
}

/// Set a row's directory, which its pieces are found by.
pub(crate) fn set_directory(session: &mut EditSession, id: u32, directory: &str) -> Result<(), String> {
    check_directory(session, directory, Some(id))?;
    let Some(record) = record_of(session, id) else { return Err("the row is no longer in the table".to_string()) };
    set_text(session, AREAS, record, af::DIRECTORY, directory, "Map directory");
    Ok(())
}

/// Whether `directory` can be a map's: letters and digits, and no other row's.
fn check_directory(session: &EditSession, directory: &str, own: Option<u32>) -> Result<(), String> {
    if directory.is_empty() || !directory.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("a directory is letters and digits only, as the shipped ones are".to_string());
    }
    if directory.eq_ignore_ascii_case(pieces::WORLD) {
        return Err("World is the whole world's directory".to_string());
    }
    let table = session.table(AREAS).ok_or("WorldMapArea is not open")?;
    let taken = (0..table.record_count()).any(|record| {
        table.u32_at(record, 0) != own
            && table.string_at(record, af::DIRECTORY).is_some_and(|other| other.eq_ignore_ascii_case(directory))
    });
    match taken {
        true => Err(format!("another map already uses {directory}")),
        false => Ok(()),
    }
}

/// The continent's `WorldMapContinent` record for `map`.
pub(crate) fn continent_record(session: &EditSession, map: u32) -> Option<usize> {
    vale_edit::flightmap::row_of(session.table(CONTINENTS)?, map)
}

/// A continent's place on the whole world's picture: offset x, offset y,
/// scale, and the tile range first column, last column, first row, last row.
pub(crate) fn placement(session: &EditSession, map: u32) -> Option<([f32; 3], [u32; 4])> {
    let table = session.table(CONTINENTS)?;
    let record = continent_record(session, map)?;
    let float = |field| table.f32_at(record, field).unwrap_or(0.0);
    let word = |field| table.u32_at(record, field).unwrap_or(0);
    Some((
        [float(cf::OFFSET_X), float(cf::OFFSET_Y), float(cf::SCALE)],
        [word(cf::LEFT_TILE), word(cf::RIGHT_TILE), word(cf::TOP_TILE), word(cf::BOTTOM_TILE)],
    ))
}

/// Set a continent's offsets and scale, as one undo entry for one drag.
pub(crate) fn set_placement(session: &mut EditSession, map: u32, place: [f32; 3], now: f64) -> bool {
    let Some(record) = continent_record(session, map) else { return false };
    let words = [(cf::OFFSET_X, place[0].to_bits()), (cf::OFFSET_Y, place[1].to_bits()), (cf::SCALE, place[2].to_bits())];
    let subject = format!("continent placement {map}");
    set_words(session, CONTINENTS, record, &words, "Continent placement", Some((&subject, now)))
}

/// Offsets and a scale that put a continent over `range` (first column, last
/// column, first row, last row) in the middle of the whole world's picture,
/// a third of it across or down.
pub(crate) fn default_placement(range: [u32; 4]) -> [f32; 3] {
    // One tile is 16 * scale pixels of the 1002 by 668 picture.
    let across = (range[1].saturating_sub(range[0]) + 1) as f32;
    let down = (range[3].saturating_sub(range[2]) + 1) as f32;
    let scale = (pieces::WIDTH as f32 / 3.0 / 16.0 / across).min(pieces::HEIGHT as f32 / 3.0 / 16.0 / down).min(1.0);
    // The middle tile lands on the picture's middle when the offset is
    // (32 - middle) * scale: see `rect_on_cosmic`.
    let middle = |first: u32, last: u32| (first + last + 1) as f32 / 2.0;
    [(32.0 - middle(range[0], range[1])) * scale, (32.0 - middle(range[2], range[3])) * scale, scale]
}

/// The zones that can have a zone map: `AreaTable` rows with no parent on a
/// map that has a continent, and no `WorldMapArea` row of their own, as
/// `(id, name, map)` in name order.
pub(crate) fn zones_without_map(session: &EditSession) -> Vec<(u32, String, u32)> {
    let (Some(areas), Some(maps)) = (session.table(area::TABLE), session.table(AREAS)) else { return Vec::new() };
    let rows: Vec<(u32, u32)> = (0..maps.record_count())
        .filter_map(|record| Some((maps.u32_at(record, af::MAP_ID)?, maps.u32_at(record, af::AREA_ID)?)))
        .collect();
    let continent = |map: u32| rows.iter().any(|(m, a)| *m == map && *a == 0);
    let mapped = |id: u32| rows.iter().any(|(_, a)| *a == id);
    let mut out: Vec<(u32, String, u32)> = (0..areas.record_count())
        .filter_map(|record| {
            let id = areas.u32_at(record, 0)?;
            let map = areas.u32_at(record, area::MAP)?;
            (areas.u32_at(record, area::PARENT)? == 0 && continent(map) && !mapped(id))
                .then(|| (id, areas.string_at(record, area::NAME).unwrap_or_default(), map))
        })
        .collect();
    out.sort_by_key(|(_, name, _)| name.to_lowercase());
    out
}

/// The maps with no continent row, as `(id, directory)`, and for each
/// whether it has a single map already.
pub(crate) fn maps_without_continent(session: &EditSession) -> Vec<(u32, String, bool)> {
    let (Some(maps), Some(rows)) = (session.table(vale_edit::dbc::places::MAPS), session.table(AREAS)) else {
        return Vec::new();
    };
    let has = |map: u32, continent: bool| {
        (0..rows.record_count()).any(|record| {
            rows.u32_at(record, af::MAP_ID) == Some(map) && (rows.u32_at(record, af::AREA_ID) == Some(0)) == continent
        })
    };
    vale_edit::dbc::places::map_directories(maps)
        .into_iter()
        .filter(|(id, _)| !has(*id, true))
        .map(|(id, directory)| (id, directory, has(id, false)))
        .collect()
}

/// The zones of `map`, as `(id, name)` in name order.
pub(crate) fn zones_of_map(session: &EditSession, map: u32) -> Vec<(u32, String)> {
    let Some(areas) = session.table(area::TABLE) else { return Vec::new() };
    let mut out: Vec<(u32, String)> = (0..areas.record_count())
        .filter_map(|record| {
            (areas.u32_at(record, area::MAP)? == map && areas.u32_at(record, area::PARENT)? == 0)
                .then(|| Some((areas.u32_at(record, 0)?, areas.string_at(record, area::NAME).unwrap_or_default())))
                .flatten()
        })
        .collect();
    out.sort_by_key(|(_, name)| name.to_lowercase());
    out
}

/// A directory from a name: its letters and digits.
pub(crate) fn directory_from(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphanumeric).collect()
}

/// Make the map the form describes, as one undo entry. Answers the new
/// `WorldMapArea` row's id and what was done.
pub(crate) fn create(form: &NewMap, session: &mut EditSession, assets: &GameAssets) -> Result<(u32, String), String> {
    check_directory(session, &form.directory, None)?;
    let (map, area) = match form.kind {
        Kind::Zone => {
            let area = form.area.ok_or("choose a zone")?;
            let map = zones_without_map(session)
                .into_iter()
                .find(|(id, _, _)| *id == area)
                .map(|(_, _, map)| map)
                .ok_or("that zone already has a map, or is not on a continent's map")?;
            (map, area)
        }
        Kind::Continent => (form.map.ok_or("choose a map")?, 0),
        Kind::Single => {
            let map = form.map.ok_or("choose a map")?;
            // A row with area 0 is a continent, so a single map takes the
            // chosen zone or the map's first.
            let area = form
                .area
                .or_else(|| zones_of_map(session, map).first().map(|(id, _)| *id))
                .ok_or("the map has no zone; make one in Zones first, since a single map's row names one")?;
            (map, area)
        }
        Kind::World => return Err("there is one whole world".to_string()),
    };
    let tiles = grid::map_tiles(session, assets, map);
    if tiles.is_empty() {
        let directory = map_directory(session, assets, map).unwrap_or_default();
        return Err(format!("map {map} ({directory}) has no tiles in its WDT"));
    }
    let rect = pieces::fit_rect(grid::tile_bounds(&tiles));
    session.history.begin("New world map");
    if form.kind == Kind::Continent {
        add_continent_row(session, map, &tiles)?;
    }
    let record = crate::tables::rows::push_row(
        session,
        AREAS,
        &[
            (af::MAP_ID, map),
            (af::AREA_ID, area),
            (af::LOC_LEFT, rect[0].to_bits()),
            (af::LOC_RIGHT, rect[1].to_bits()),
            (af::LOC_TOP, rect[2].to_bits()),
            (af::LOC_BOTTOM, rect[3].to_bits()),
        ],
        &[(af::DIRECTORY, form.directory.clone())],
    );
    session.history.end();
    let id = record.and_then(|record| session.table(AREAS)?.u32_at(record, 0)).ok_or("the row could not be added")?;
    let said = match form.kind {
        Kind::Continent => format!(
            "added continent {} for map {map}; it is a new entry of the continent drop-down. Draw or import its \
             picture, then write its zone grid once its zones have maps",
            form.directory
        ),
        Kind::Single => format!("added single map {}; on map {map} the world map opens on it", form.directory),
        _ => format!("added zone map {}", form.directory),
    };
    Ok((id, said))
}

/// Give `map` a `WorldMapContinent` row placed on the whole world's picture,
/// or place the one a flight map made, in the history entry that is open.
fn add_continent_row(session: &mut EditSession, map: u32, tiles: &[(u32, u32)]) -> Result<(), String> {
    use vale_edit::flightmap;
    if continent_record(session, map).is_none() {
        let plan = flightmap::plan(map, tiles, &[]).ok_or("the map has no tiles")?;
        let table = session.tables.get_mut(CONTINENTS).ok_or("WorldMapContinent is not open")?;
        match flightmap::write(table, &mut session.numbers, &plan).map_err(|why| why.to_string())? {
            flightmap::Written::Added(row) => session.history.record_row(CONTINENTS, row),
            flightmap::Written::Changed(cells) => {
                for cell in cells {
                    session.history.record_cell(CONTINENTS, cell);
                }
            }
        }
        session.table_edited(CONTINENTS);
    }
    let Some((place, range)) = placement(session, map) else { return Ok(()) };
    if place[2] == 0.0 {
        let record = continent_record(session, map).expect("found above");
        let place = default_placement(range);
        let table = session.tables.get_mut(CONTINENTS).ok_or("WorldMapContinent is not open")?;
        for (field, value) in [(cf::OFFSET_X, place[0]), (cf::OFFSET_Y, place[1]), (cf::SCALE, place[2])] {
            if let Some(cell) = Cell::new(table, record, field, value.to_bits()) {
                cell.apply(table);
                session.history.record_cell(CONTINENTS, cell);
            }
        }
        session.table_edited(CONTINENTS);
    }
    Ok(())
}

/// Remove a map's `WorldMapArea` row and its overlays' rows, as one undo
/// entry. A continent's `WorldMapContinent` row stays, because its flight map
/// reads it. Answers what was done.
pub(crate) fn remove(session: &mut EditSession, entry: &Entry) -> String {
    let Some(area) = &entry.area else { return "the whole world is not a row".to_string() };
    let overlays: Vec<u32> = super::overlays::Overlays::rows(session, area.id).iter().map(|row| row.id).collect();
    session.history.begin("Remove world map");
    let mut removed = 0;
    for id in overlays {
        if let Some(record) = session.table(OVERLAYS).and_then(|table| table.row_of(id)) {
            if remove_record(session, OVERLAYS, record) {
                removed += 1;
            }
        }
    }
    if let Some(record) = record_of(session, area.id) {
        remove_record(session, AREAS, record);
    }
    session.history.end();
    format!("removed {} and {removed} of its overlays; its files stay in the project", entry.name)
}

/// Remove one record in the history entry that is open.
fn remove_record(session: &mut EditSession, table: &str, record: usize) -> bool {
    let Some(file) = session.tables.get_mut(table) else { return false };
    let Some(row) = Row::removed(file, record) else { return false };
    file.remove_record(record);
    session.history.record_row(table, row);
    session.table_edited(table);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new continent's placement centres its tiles on the whole world's
    /// picture, at a third of it on the longer side, by the client's rule.
    #[test]
    fn a_new_continent_is_placed_in_the_middle_of_the_world() {
        let range = [10, 59, 20, 39];
        let [ox, oy, scale] = default_placement(range);
        // `rect_on_cosmic`, written out: u(tile) = (tile * s + 31.3125 - 32 s + ox) * 16 / 1002.
        let u = |tile: f32| (tile * scale + 31.3125 - 32.0 * scale + ox) * 16.0 / 1002.0;
        let v = |tile: f32| (tile * scale + 20.875 - 32.0 * scale + oy) * 16.0 / 668.0;
        let (left, right) = (u(10.0), u(60.0));
        let (top, bottom) = (v(20.0), v(40.0));
        assert!(((left + right) / 2.0 - 0.5).abs() < 1e-4, "{left} {right}");
        assert!(((top + bottom) / 2.0 - 0.5).abs() < 1e-4, "{top} {bottom}");
        assert!(((right - left) - 1.0 / 3.0).abs() < 1e-3 || ((bottom - top) - 1.0 / 3.0).abs() < 1e-3);
        assert_eq!(directory_from("Stranglethorn Vale"), "StranglethornVale");
    }
}
