//! Places: what adding and removing a row does to `AreaTrigger.dbc`,
//! `WorldSafeLocs.dbc` and `Map.dbc`.
//!
//! ```text
//! AreaTrigger    a sphere or a box on a map that the client reports standing in
//! WorldSafeLocs  a place a dead character's spirit appears
//! Map            a map: its id, its folder, its name and what kind of place it is
//! ```
//!
//! The field indices are `vale_assets::tables::{areatrigger, safeloc, map}`'s.
//! This module only states what an edit to the rows is. Each operation writes
//! the table as it goes and returns what it wrote as [`Edits`], the same type
//! the flight path operations return, for the caller to put on the undo stack
//! as one entry.
//!
//! ## A trigger goes at the end of its map's block
//!
//! The 1.12.1 client finds the first row of the current map and walks forward
//! until the map changes, so it reads only one contiguous block of rows per
//! map; see `vale_assets::tables::areatrigger`. A row appended to the end of
//! the file is outside that block for every map but the last one, and the
//! client never tests it. [`new_trigger`] therefore inserts after the last row
//! of the trigger's map, or where the map would sort when the file has none.
//! [`trigger_blocks`] reports a file whose maps are not each one block.
//!
//! ## Ids
//!
//! A new row takes one past the largest id in its table. A trigger's id must
//! fit `areatrigger_template.id`, a `smallint unsigned`, and a map's must stay
//! under [`vale_assets::tables::map::MAX_NEW_ID`].

use super::taxi::Edits;
use super::{DbcFile, Row};
use std::collections::HashMap;
use vale_assets::tables::areatrigger::fields as tf;
use vale_assets::tables::map::fields as mf;
use vale_assets::tables::safeloc::fields as sf;

/// The three tables, by the names the caller's table map uses.
pub const TRIGGERS: &str = "AreaTrigger";
pub const SAFE_LOCS: &str = "WorldSafeLocs";
pub const MAPS: &str = "Map";

/// The largest trigger id the server's `areatrigger_template.id` holds.
pub const MAX_TRIGGER_ID: u32 = u16::MAX as u32;

/// One `AreaTrigger` row, as read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trigger {
    pub id: u32,
    pub record: usize,
    pub map: u32,
    /// The centre: x, y, z in the server's axes, north, west, up.
    pub at: [f32; 3],
    /// Above 0 for a sphere.
    pub radius: f32,
    /// A box's whole length, width and height.
    pub extent: [f32; 3],
    /// A box's turn about up, in radians.
    pub yaw: f32,
}

impl Trigger {
    /// Whether the volume is a sphere: the rule `vale_assets` reads by.
    pub fn is_sphere(&self) -> bool {
        self.radius > 0.0
    }
}

/// One `WorldSafeLocs` row, as read.
#[derive(Debug, Clone, PartialEq)]
pub struct SafeLoc {
    pub id: u32,
    pub record: usize,
    pub map: u32,
    pub at: [f32; 3],
    pub name: String,
}

/// What a new map is made of.
#[derive(Debug, Clone, PartialEq)]
pub struct MapSpec {
    /// The folder its files are in; see `vale_assets::tables::map::directory_problem`.
    pub directory: String,
    pub name: String,
    /// Field 2: 0 world, 1 dungeon, 2 raid, 3 battleground.
    pub instance_type: u32,
    pub max_players: u32,
    /// An `AreaTable` row, or 0.
    pub area: u32,
    /// A `LoadingScreens` row, or 0.
    pub loading_screen: u32,
    /// Field 3.
    pub pvp: bool,
    /// Fields 13 and 14; 0 for none.
    pub min_level: u32,
    pub max_level: u32,
    /// Fields 20 and 29, in the client's own locale; empty for none.
    pub descriptions: [String; 2],
}

/// Why an operation wrote nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum Refused {
    /// The table is not in the map the caller passed.
    NotOpen(&'static str),
    NoSuchTrigger(u32),
    NoSuchSafeLoc(u32),
    /// The next trigger id is past [`MAX_TRIGGER_ID`].
    TriggerIdsExhausted,
    /// The next map id is past `vale_assets::tables::map::MAX_NEW_ID`.
    MapIdsExhausted,
    /// The directory cannot be a map's folder, with the reason.
    BadDirectory(String),
    /// The record could not be written; the table is not the width 1.12 ships.
    WrongWidth(&'static str),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::NotOpen(table) => write!(f, "{table}.dbc is not open"),
            Refused::NoSuchTrigger(id) => write!(f, "AreaTrigger has no row {id}"),
            Refused::NoSuchSafeLoc(id) => write!(f, "WorldSafeLocs has no row {id}"),
            Refused::TriggerIdsExhausted => write!(
                f,
                "trigger ids stop at {MAX_TRIGGER_ID}: areatrigger_template.id is a smallint"
            ),
            Refused::MapIdsExhausted => write!(
                f,
                "map ids stop at {}: the server's tile files carry the id in three digits",
                vale_assets::tables::map::MAX_NEW_ID
            ),
            Refused::BadDirectory(why) => write!(f, "{why}"),
            Refused::WrongWidth(table) => write!(f, "{table}.dbc does not have the 1.12 record size"),
        }
    }
}

fn open<'a>(tables: &'a mut HashMap<String, DbcFile>, name: &'static str) -> Result<&'a mut DbcFile, Refused> {
    tables.get_mut(name).ok_or(Refused::NotOpen(name))
}

/// Write `words` into a record, taking the record back out if one does not fit.
fn write_words(table: &mut DbcFile, name: &'static str, record: usize, words: &[(usize, u32)]) -> Result<(), Refused> {
    for &(field, value) in words {
        if !table.set_u32(record, field, value) {
            table.remove_record(record);
            return Err(Refused::WrongWidth(name));
        }
    }
    Ok(())
}

/// Every trigger in the table, in file order.
pub fn triggers(table: &DbcFile) -> Vec<Trigger> {
    (0..table.record_count())
        .filter_map(|record| trigger_at(table, record))
        .collect()
}

/// The trigger at a record.
pub fn trigger_at(table: &DbcFile, record: usize) -> Option<Trigger> {
    let float = |field: usize| table.f32_at(record, field);
    Some(Trigger {
        id: table.u32_at(record, tf::ID)?,
        record,
        map: table.u32_at(record, tf::MAP)?,
        at: [float(tf::X)?, float(tf::Y)?, float(tf::Z)?],
        radius: float(tf::RADIUS)?,
        extent: [float(tf::BOX_LENGTH)?, float(tf::BOX_WIDTH)?, float(tf::BOX_HEIGHT)?],
        yaw: float(tf::BOX_YAW)?,
    })
}

/// Where a new trigger on `map` goes: after the last row of that map, or
/// before the first row of a higher map when the file has none of it.
pub fn trigger_slot(table: &DbcFile, map: u32) -> usize {
    let maps: Vec<u32> = (0..table.record_count())
        .map(|record| table.u32_at(record, tf::MAP).unwrap_or(0))
        .collect();
    if let Some(last) = maps.iter().rposition(|&had| had == map) {
        return last + 1;
    }
    maps.iter().position(|&had| had > map).unwrap_or(maps.len())
}

/// The maps whose triggers are split into more than one block, which the
/// 1.12.1 client reads only the first of. Empty for the shipped file.
pub fn trigger_blocks(table: &DbcFile) -> Vec<u32> {
    let mut seen: Vec<u32> = Vec::new();
    let mut split: Vec<u32> = Vec::new();
    let mut last: Option<u32> = None;
    for record in 0..table.record_count() {
        let map = table.u32_at(record, tf::MAP).unwrap_or(0);
        if last != Some(map) {
            if seen.contains(&map) && !split.contains(&map) {
                split.push(map);
            }
            seen.push(map);
        }
        last = Some(map);
    }
    split
}

/// Make a sphere trigger of `radius` yards centred at `at`. Returns its id in
/// [`Edits::made`].
pub fn new_trigger(
    tables: &mut HashMap<String, DbcFile>,
    map: u32,
    at: [f32; 3],
    radius: f32,
) -> Result<Edits, Refused> {
    let table = open(tables, TRIGGERS)?;
    let id = table.max_id() + 1;
    if id > MAX_TRIGGER_ID {
        return Err(Refused::TriggerIdsExhausted);
    }
    let slot = trigger_slot(table, map);
    let bytes = table.blank_record(id);
    if !table.insert_record(slot, &bytes) {
        return Err(Refused::WrongWidth(TRIGGERS));
    }
    write_words(
        table,
        TRIGGERS,
        slot,
        &[
            (tf::MAP, map),
            (tf::X, at[0].to_bits()),
            (tf::Y, at[1].to_bits()),
            (tf::Z, at[2].to_bits()),
            (tf::RADIUS, radius.to_bits()),
        ],
    )?;
    let mut out = Edits::default();
    out.rows.push((TRIGGERS.to_string(), Row::added(table, slot).ok_or(Refused::WrongWidth(TRIGGERS))?));
    out.made = Some(id);
    Ok(out)
}

/// Remove a trigger.
pub fn remove_trigger(tables: &mut HashMap<String, DbcFile>, id: u32) -> Result<Edits, Refused> {
    let table = open(tables, TRIGGERS)?;
    let record = table.row_of(id).ok_or(Refused::NoSuchTrigger(id))?;
    let row = Row::removed(table, record).ok_or(Refused::WrongWidth(TRIGGERS))?;
    table.remove_record(record);
    let mut out = Edits::default();
    out.rows.push((TRIGGERS.to_string(), row));
    Ok(out)
}

/// Every safe place in the table, in file order.
pub fn safe_locs(table: &DbcFile) -> Vec<SafeLoc> {
    (0..table.record_count())
        .filter_map(|record| safe_loc_at(table, record))
        .collect()
}

/// The safe place at a record.
pub fn safe_loc_at(table: &DbcFile, record: usize) -> Option<SafeLoc> {
    let float = |field: usize| table.f32_at(record, field);
    Some(SafeLoc {
        id: table.u32_at(record, sf::ID)?,
        record,
        map: table.u32_at(record, sf::MAP)?,
        at: [float(sf::X)?, float(sf::Y)?, float(sf::Z)?],
        name: table.string_at(record, sf::NAME).unwrap_or_default(),
    })
}

/// Make a safe place. Returns its id in [`Edits::made`].
pub fn new_safe_loc(
    tables: &mut HashMap<String, DbcFile>,
    map: u32,
    at: [f32; 3],
    name: &str,
) -> Result<Edits, Refused> {
    let table = open(tables, SAFE_LOCS)?;
    let id = table.max_id() + 1;
    let bytes = table.blank_record(id);
    let record = table.push_record(&bytes).ok_or(Refused::WrongWidth(SAFE_LOCS))?;
    write_words(
        table,
        SAFE_LOCS,
        record,
        &[
            (sf::MAP, map),
            (sf::X, at[0].to_bits()),
            (sf::Y, at[1].to_bits()),
            (sf::Z, at[2].to_bits()),
        ],
    )?;
    // The name is appended to the string block before the record is noted, so
    // the noted bytes carry its offset.
    table.set_string(record, sf::NAME, name);
    let mut out = Edits::default();
    out.rows.push((SAFE_LOCS.to_string(), Row::added(table, record).ok_or(Refused::WrongWidth(SAFE_LOCS))?));
    out.made = Some(id);
    Ok(out)
}

/// Remove a safe place.
pub fn remove_safe_loc(tables: &mut HashMap<String, DbcFile>, id: u32) -> Result<Edits, Refused> {
    let table = open(tables, SAFE_LOCS)?;
    let record = table.row_of(id).ok_or(Refused::NoSuchSafeLoc(id))?;
    let row = Row::removed(table, record).ok_or(Refused::WrongWidth(SAFE_LOCS))?;
    table.remove_record(record);
    let mut out = Edits::default();
    out.rows.push((SAFE_LOCS.to_string(), row));
    Ok(out)
}

/// Every map's id and directory, in file order.
pub fn map_directories(table: &DbcFile) -> Vec<(u32, String)> {
    (0..table.record_count())
        .filter_map(|record| {
            Some((
                table.u32_at(record, mf::ID)?,
                table.string_at(record, mf::DIRECTORY)?,
            ))
        })
        .collect()
}

/// Make a map row. Fields the spec does not name take the values most
/// shipped rows hold: the shipped name flags, the flags of an empty
/// description, -1 in field 16, 1 in field 40 and 1.0 in field 41.
/// Returns the new map's id in [`Edits::made`].
pub fn new_map(tables: &mut HashMap<String, DbcFile>, spec: &MapSpec) -> Result<Edits, Refused> {
    let table = open(tables, MAPS)?;
    let taken = map_directories(table);
    if let Some(why) =
        vale_assets::tables::map::directory_problem(&spec.directory, taken.iter().map(|(_, dir)| dir.as_str()))
    {
        return Err(Refused::BadDirectory(why));
    }
    let id = vale_assets::tables::map::next_id(taken.iter().map(|(id, _)| *id)).ok_or(Refused::MapIdsExhausted)?;
    let bytes = table.blank_record(id);
    let record = table.push_record(&bytes).ok_or(Refused::WrongWidth(MAPS))?;
    write_words(
        table,
        MAPS,
        record,
        &[
            (mf::INSTANCE_TYPE, spec.instance_type),
            (mf::PVP, u32::from(spec.pvp)),
            (mf::NAME_FLAGS, vale_assets::tables::map::SHIPPED_NAME_FLAGS),
            (mf::MIN_LEVEL, spec.min_level),
            (mf::MAX_LEVEL, spec.max_level),
            (mf::MAX_PLAYERS, spec.max_players),
            (mf::FIELD_16, u32::MAX),
            (mf::AREA, spec.area),
            (mf::DESCRIPTION_0_FLAGS, vale_assets::tables::map::EMPTY_DESCRIPTION_FLAGS),
            (mf::DESCRIPTION_1_FLAGS, vale_assets::tables::map::EMPTY_DESCRIPTION_FLAGS),
            (mf::LOADING_SCREEN, spec.loading_screen),
            (mf::FIELD_40, 1),
            (mf::FIELD_41, 1.0f32.to_bits()),
        ],
    )?;
    table.set_string(record, mf::DIRECTORY, &spec.directory);
    table.set_string(record, mf::NAME, &spec.name);
    for (field, text) in [(mf::DESCRIPTION_0, &spec.descriptions[0]), (mf::DESCRIPTION_1, &spec.descriptions[1])] {
        if !text.is_empty() {
            table.set_string(record, field, text);
        }
    }
    let mut out = Edits::default();
    out.rows.push((MAPS.to_string(), Row::added(table, record).ok_or(Refused::WrongWidth(MAPS))?));
    out.made = Some(id);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of `fields` fields with one record per id, each record's field 1
    /// set to its map.
    fn table(fields: usize, rows: &[(u32, u32)]) -> DbcFile {
        let size = fields * 4;
        let mut buf = Vec::new();
        buf.extend_from_slice(b"WDBC");
        for word in [rows.len() as u32, fields as u32, size as u32, 1] {
            buf.extend_from_slice(&word.to_le_bytes());
        }
        for &(id, map) in rows {
            let mut record = vec![0u8; size];
            record[0..4].copy_from_slice(&id.to_le_bytes());
            record[4..8].copy_from_slice(&map.to_le_bytes());
            buf.extend_from_slice(&record);
        }
        buf.push(0);
        DbcFile::parse(&buf).unwrap()
    }

    fn open_with(name: &str, file: DbcFile) -> HashMap<String, DbcFile> {
        HashMap::from([(name.to_string(), file)])
    }

    /// A new trigger lands at the end of its map's block, so the client's walk
    /// over that block reaches it; a map with no rows yet sorts into place.
    #[test]
    fn a_new_trigger_goes_at_the_end_of_its_maps_block() {
        let mut tables = open_with(TRIGGERS, table(tf::COUNT, &[(1, 0), (2, 0), (3, 1), (4, 36)]));
        let done = new_trigger(&mut tables, 0, [10.0, 20.0, 30.0], 5.0).unwrap();
        assert_eq!(done.made, Some(5));
        let file = &tables[TRIGGERS];
        let order: Vec<(u32, u32)> = triggers(file).iter().map(|t| (t.id, t.map)).collect();
        assert_eq!(order, [(1, 0), (2, 0), (5, 0), (3, 1), (4, 36)]);
        let made = triggers(file).into_iter().find(|t| t.id == 5).unwrap();
        assert_eq!((made.at, made.radius), ([10.0, 20.0, 30.0], 5.0));
        assert!(made.is_sphere());
        assert!(trigger_blocks(file).is_empty());

        new_trigger(&mut tables, 30, [0.0; 3], 3.0).unwrap();
        let order: Vec<u32> = triggers(&tables[TRIGGERS]).iter().map(|t| t.map).collect();
        assert_eq!(order, [0, 0, 0, 1, 30, 36]);

        let done = remove_trigger(&mut tables, 5).unwrap();
        assert_eq!(done.rows.len(), 1);
        assert!(tables[TRIGGERS].row_of(5).is_none());
    }

    /// A map whose rows are split is reported once.
    #[test]
    fn a_split_map_block_is_reported() {
        let file = table(tf::COUNT, &[(1, 0), (2, 1), (3, 0), (4, 0)]);
        assert_eq!(trigger_blocks(&file), [0]);
    }

    #[test]
    fn a_safe_place_is_made_with_its_name_and_removed() {
        let mut tables = open_with(SAFE_LOCS, table(sf::COUNT, &[(2, 0), (4, 1)]));
        let done = new_safe_loc(&mut tables, 0, [1.0, 2.0, 3.0], "Hilltop").unwrap();
        assert_eq!(done.made, Some(5));
        let made = safe_locs(&tables[SAFE_LOCS]).into_iter().find(|loc| loc.id == 5).unwrap();
        assert_eq!((made.map, made.at, made.name.as_str()), (0, [1.0, 2.0, 3.0], "Hilltop"));
        remove_safe_loc(&mut tables, 5).unwrap();
        assert!(safe_locs(&tables[SAFE_LOCS]).iter().all(|loc| loc.id != 5));
        assert_eq!(remove_safe_loc(&mut tables, 99), Err(Refused::NoSuchSafeLoc(99)));
    }

    #[test]
    fn a_new_map_takes_the_next_id_and_refuses_a_taken_directory() {
        let mut file = table(mf::COUNT, &[(0, 0), (36, 0)]);
        file.set_string(0, mf::DIRECTORY, "Azeroth");
        let mut tables = open_with(MAPS, file);
        let spec = MapSpec {
            directory: "Islands".into(),
            name: "The Islands".into(),
            instance_type: 1,
            max_players: 5,
            area: 0,
            loading_screen: 0,
            pvp: true,
            min_level: 20,
            max_level: 30,
            descriptions: ["Held by no one.".into(), String::new()],
        };
        let done = new_map(&mut tables, &spec).unwrap();
        assert_eq!(done.made, Some(37));
        let maps = &tables[MAPS];
        let record = maps.row_of(37).unwrap();
        assert_eq!(maps.string_at(record, mf::DIRECTORY).as_deref(), Some("Islands"));
        assert_eq!(maps.string_at(record, mf::NAME).as_deref(), Some("The Islands"));
        assert_eq!(maps.u32_at(record, mf::INSTANCE_TYPE), Some(1));
        assert_eq!(maps.u32_at(record, mf::FIELD_16), Some(u32::MAX));
        assert_eq!(maps.u32_at(record, mf::PVP), Some(1));
        assert_eq!((maps.u32_at(record, mf::MIN_LEVEL), maps.u32_at(record, mf::MAX_LEVEL)), (Some(20), Some(30)));
        assert_eq!(maps.string_at(record, mf::DESCRIPTION_0).as_deref(), Some("Held by no one."));

        let again = MapSpec { directory: "AZEROTH".into(), ..spec };
        assert!(matches!(new_map(&mut tables, &again), Err(Refused::BadDirectory(_))));
    }
}
