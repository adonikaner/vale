//! `AreaTable.dbc`: the name of the place a character is standing in.
//!
//! One table, 1,081 rows, and it answers the four questions the interface asks
//! about a place: `GetZoneText`, `GetSubZoneText`, `GetRealZoneText` and
//! `GetMinimapZoneText`. It is also the name half of the world map — a zone tab
//! on the continent drop-down is an `AreaTable` name reached through
//! [`crate::tables::worldmap`].
//!
//! Nothing in the protocol carries a zone name. The server sends a *map* in
//! `SMSG_LOGIN_VERIFY_WORLD` and nothing else about where the character is;
//! everything below that is the client reading the terrain under its own feet.
//! `MCNK`'s header carries an `areaId` per 33-yard chunk ([`crate::world::adt`]), and
//! that id indexes this table.
//!
//! ## The layout
//!
//! ```text
//! AreaTable.dbc      1,081 rows x 25 fields
//!   [ 0] id
//!   [ 1] mapId
//!   [ 2] parentAreaId   0 for a zone; the zone's id for a sub-area
//!   [ 3] areaBit        the exploration bit — see [`Area::explore_bit`]
//!   [ 4] flags
//!   [11] name           enUS, first of the eight locale columns
//! ```
//!
//! vmangos's `AreaTableEntryfmt` is `"niiiixxxxxissssssssxixxxi"` — id, three
//! ints, five skipped, one int, eight strings, which puts the name block at
//! 11. Corroborated against the file: row 12 reads mapId 0, parent 0, and its
//! field 11 resolves to "Elwynn Forest", which is what area 12 is.
//!
//! ## A zone is an area with no parent
//!
//! The table is two levels and only two: "Northshire Valley" (parent 12) sits
//! inside "Elwynn Forest" (parent 0). That is the whole of the difference
//! between `GetZoneText` and `GetSubZoneText` — the interface shows the zone on
//! the minimap's title and the sub-area under it, and an area with no parent
//! draws an empty sub-area rather than repeating itself.
//!
//! [`Areas::zone_of`] walks up rather than assuming one level, with a step bound
//! for the same reason [`crate::tables::dbc`]'s fallback walk has one: a table this
//! client did not author can contain a cycle, and a client that hangs on one is
//! worse than one that answers the area itself.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

/// The field indices this module reads, and the rest of the row's layout.
/// `tables::schema::AREA_TABLE` names the same fields and a test there holds
/// the two together.
pub mod fields {
    pub const MAP_ID: usize = 1;
    pub const PARENT: usize = 2;
    /// The exploration bit — see [`super::Area::explore_bit`].
    pub const AREA_BIT: usize = 3;
    /// The flags word — see [`super::Area::flags`].
    pub const FLAGS: usize = 4;
    /// The music played in the area: a `ZoneMusic` row. Every one of the 447
    /// shipped rows that names one resolves.
    pub const ZONE_MUSIC: usize = 8;
    /// The exploration level, field 10 — read as a *gate* rather than as a
    /// level: the client skips the area check for a landmark whose row has a
    /// negative one, which is 200 of the 339 rows in `AreaPOI.dbc`. See
    /// [`crate::tables::areapoi`].
    pub const EXPLORE_LEVEL: usize = 10;
    pub const NAME: usize = 11;
    /// The word after the eight name columns.
    pub const NAME_FLAGS: usize = 19;
    /// Which side the area belongs to: 2 for the Alliance, 4 for the Horde,
    /// 0 for neither. 1,019 of the 1,081 shipped rows hold 0.
    pub const TEAM: usize = 20;
    /// A `LiquidType` row that replaces the liquid drawn in the area. One
    /// shipped row holds one: Naxxramas, 21.
    pub const LIQUID_TYPE: usize = 24;
    /// How many fields a row has.
    pub const COUNT: usize = 25;
}

/// How far [`Areas::zone_of`] will walk before giving up. Three is already one
/// more level than the shipped table has.
const MAX_PARENT_DEPTH: usize = 8;

/// One row, reduced to what anything here asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Area {
    pub id: u32,
    pub map: u32,
    /// 0 for a zone; the enclosing zone's id for a sub-area.
    pub parent: u32,
    pub name: String,
    /// The flags word, field 4 — vmangos' `AreaFlags` (`DBCEnums.h`), read
    /// off the same column. Two of them are read here: [`AREA_FLAG_SLAVE_CAPITAL`]
    /// and [`AREA_FLAG_CITY`]. Stormwind City and Orgrimmar read `0x138`, the
    /// Trade District `0x38`, Elwynn Forest `0x40` (duels allowed).
    pub flags: u32,
    /// Which bit of `PLAYER_EXPLORED_ZONES` says this place has been seen.
    ///
    /// The one column in this table that is neither a name nor a hierarchy, and
    /// the only thing that joins the character's exploration mask to a place.
    /// The server writes it the same way (`Player::CheckAreaExploreAndOutdoor`
    /// takes `areaFlag / 32` and `1 << (areaFlag % 32)`), and the client reads
    /// it as a byte array — `areaBit / 8` and `1 << (areaBit % 8)` — which is the same bit string either way.
    ///
    /// It is not unique and it is not the row id: several rows share a bit, and
    /// 0 is a real value (the first bit) rather than "no bit", so nothing
    /// here filters on it. See [`crate::tables::worldmap::WorldMap::overlays`], which is
    /// the only reader.
    pub explore_bit: u32,
    /// Field 10, and it is a gate rather than a level here. A negative one
    /// means a landmark in this area is shown without asking whether it has
    /// been explored — see [`crate::tables::areapoi`], which is the only reader.
    pub explore_level: i32,
    /// Which side the area belongs to, field 20: a mask over
    /// `FactionGroup.MaskID`, so 2 for the Alliance and 4 for the Horde.
    /// See [`crate::tables::territory`], which is the only reader.
    pub team: u32,
}

/// `AREA_FLAG_SLAVE_CAPITAL`, `0x8` — "allow trade channel" in vmangos' own
/// comment: the sub-areas of a capital carry it, and it is what decides
/// whether the trade channel exists where a character stands.
pub const AREA_FLAG_SLAVE_CAPITAL: u32 = 0x8;
/// `AREA_FLAG_CITY`, `0x200` — "highest areaid with this flag will be name
/// used on chat channels": the row the city channels are named after, which
/// in 1.12 is the one called `City`. See [`Areas::city_name`].
pub const AREA_FLAG_CITY: u32 = 0x200;

impl Area {
    /// Whether the trade channel exists here — see [`AREA_FLAG_SLAVE_CAPITAL`].
    pub fn allows_trade(&self) -> bool {
        self.flags & AREA_FLAG_SLAVE_CAPITAL != 0
    }

    /// A zone is an area nothing encloses — see the module comment.
    pub fn is_zone(&self) -> bool {
        self.parent == 0
    }
}

/// `AreaTable.dbc`, parsed.
#[derive(Debug, Default, Clone)]
pub struct Areas {
    by_id: HashMap<u32, Area>,
}

impl Areas {
    /// Parse the table. `None` for bytes that are not a DBC at all, which costs
    /// every zone name in the client and nothing else — see [`crate::tables::dbc`]'s own
    /// note on documented degradations.
    pub fn parse(area_table: &[u8]) -> Option<Areas> {
        let dbc = Dbc::parse(area_table).ok()?;
        let mut by_id = HashMap::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            let Some(id) = dbc.u32_at(record, 0) else {
                continue;
            };
            by_id.insert(
                id,
                Area {
                    id,
                    map: dbc.u32_at(record, fields::MAP_ID).unwrap_or(0),
                    parent: dbc.u32_at(record, fields::PARENT).unwrap_or(0),
                    name: dbc.string_at(record, fields::NAME).unwrap_or_default(),
                    explore_bit: dbc.u32_at(record, fields::AREA_BIT).unwrap_or(0),
                    explore_level: dbc
                        .u32_at(record, fields::EXPLORE_LEVEL)
                        .unwrap_or(0) as i32,
                    flags: dbc.u32_at(record, fields::FLAGS).unwrap_or(0),
                    team: dbc.u32_at(record, fields::TEAM).unwrap_or(0),
                },
            );
        }
        Some(Areas { by_id })
    }

    /// The table built by hand — the tests', for a machine with no archives.
    pub fn from_rows(rows: Vec<Area>) -> Areas {
        Areas {
            by_id: rows.into_iter().map(|area| (area.id, area)).collect(),
        }
    }

    pub fn get(&self, id: u32) -> Option<&Area> {
        self.by_id.get(&id)
    }

    /// Every row, in no order.
    pub fn iter(&self) -> impl Iterator<Item = &Area> {
        self.by_id.values()
    }

    /// The name the city channels take — the highest area id carrying
    /// [`AREA_FLAG_CITY`], which is the rule vmangos' comment on the flag
    /// states and which the 1.12 table answers with the row named `City`:
    /// `Trade - City`, one channel for every capital. `None` for a table with
    /// no such row.
    pub fn city_name(&self) -> Option<&str> {
        self.by_id
            .values()
            .filter(|area| area.flags & AREA_FLAG_CITY != 0)
            .max_by_key(|area| area.id)
            .map(|area| area.name.as_str())
    }

    /// The enclosing zone, or the area itself when it is one.
    ///
    /// `None` only for an id the table does not have, which a damaged or patched
    /// `MCNK` can carry.
    pub fn zone_of(&self, id: u32) -> Option<&Area> {
        let mut at = self.get(id)?;
        for _ in 0..MAX_PARENT_DEPTH {
            if at.parent == 0 {
                return Some(at);
            }
            match self.get(at.parent) {
                Some(parent) => at = parent,
                // A parent the table does not carry: the area itself is the
                // best answer there is, and it is a real place name.
                None => return Some(at),
            }
        }
        Some(at)
    }

    /// The sub-area's own name, empty when the area is a zone — which is
    /// what `GetSubZoneText` answers standing in open country, and what the
    /// minimap's second line is written against.
    pub fn sub_zone_name(&self, id: u32) -> String {
        match self.get(id) {
            Some(area) if !area.is_zone() => area.name.clone(),
            _ => String::new(),
        }
    }

    /// The zone's name, empty for an id the table does not have.
    pub fn zone_name(&self, id: u32) -> String {
        self.zone_of(id).map_or_else(String::new, |a| a.name.clone())
    }

    pub fn count(&self) -> usize {
        self.by_id.len()
    }

    /// How many of them are zones rather than sub-areas — the check that the
    /// parent column is being read as a column and not as a flag.
    pub fn zone_count(&self) -> usize {
        self.zones().count()
    }

    /// Every zone, in no particular order — the rows a `/who z-` may name.
    ///
    /// Zones rather than every area, because that is what the reference walks:
    /// it takes only rows whose parent is zero. A sub-area in this list
    /// would make `z-"Northshire Valley"` a filter the server can satisfy, which
    /// it cannot — `SMSG_WHO` matches against the player's cached zone.
    pub fn zones(&self) -> impl Iterator<Item = &Area> {
        self.by_id.values().filter(|a| a.is_zone())
    }

    /// The one zone a map is, for a map that has no ground to ask.
    ///
    /// Twenty of the game's forty-three maps carry no ADT at all, so the
    /// `MCNK` `areaId` this table is normally reached through does not exist on
    /// them and the building's `WMOAreaTable` row is the whole answer. Where
    /// that row states nothing — and 42 of Ironforge's 104 groups are the shape
    /// that states nothing, so it is not rare — the `mapId` column here is the
    /// only thing left in any shipped file that names the place.
    ///
    /// It refuses where the file is ambiguous rather than picking, which is
    /// the difference between reading the data and guessing at it: one zone
    /// naming a map is an answer, and several is the file declining to say. See
    /// `vale zones`, whose last section is the census, and
    /// `game::place::worldmap`, which is the caller.
    ///
    /// Never call this for a map that *has* terrain: map 0 has forty zone rows
    /// naming it and every one of them is right somewhere on the continent.
    pub fn zone_of_map(&self, map: u32) -> Option<&Area> {
        let mut found = self.by_id.values().filter(|a| a.is_zone() && a.map == map);
        let only = found.next()?;
        found.next().is_none().then_some(only)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// `\0Elwynn Forest\0Northshire Valley\0` — offsets 1 and 15.
    const NAMES: &[u8] = b"\0Elwynn Forest\0Northshire Valley\0";

    fn row(id: u32, map: u32, parent: u32, name: u32) -> Vec<u32> {
        let mut row = vec![0u32; 25];
        row[0] = id;
        row[fields::MAP_ID] = map;
        row[fields::PARENT] = parent;
        row[fields::NAME] = name;
        row
    }

    fn areas() -> Areas {
        Areas::parse(&dbc(
            &[row(12, 0, 0, 1), row(9, 0, 12, 15), row(88, 0, 77, 1)],
            25,
            NAMES,
        ))
        .expect("the table parses")
    }

    /// A map with one zone naming it is that zone; a map with several is not
    /// answered at all.
    ///
    /// This is the last thing that names a place on the twenty maps with no
    /// ground — `vale zones` measures that none of their buildings states an
    /// `AreaTable` area, so there is nothing else — and the refusal is the half
    /// that keeps it a reading of the file rather than a guess at it. Map 0 has
    /// forty-eight zone rows and every one of them is right *somewhere* on the
    /// continent; picking one would be this client inventing a rule, and map 0
    /// has ground to ask anyway.
    ///
    /// A sub-area naming the map does not count, which is the third case: only
    /// zones are candidates, or Northshire Valley would be a continent's answer.
    #[test]
    fn a_map_is_named_by_its_one_zone_and_never_by_a_choice_between_two() {
        // 34 is the Stockade's shape: one zone row, nothing else on the map.
        // 55 is the ambiguous shape, and 0 the sub-area-only one.
        let areas = Areas::parse(&dbc(
            &[
                row(717, 34, 0, 1),
                row(2366, 55, 0, 1),
                row(2367, 55, 0, 15),
                row(9, 66, 12, 15),
            ],
            25,
            NAMES,
        ))
        .expect("the table parses");

        assert_eq!(areas.zone_of_map(34).map(|z| z.id), Some(717));
        assert_eq!(areas.zone_of_map(55), None, "two rows is the file declining");
        assert_eq!(areas.zone_of_map(66), None, "a sub-area is not a map's zone");
        assert_eq!(areas.zone_of_map(99), None, "a map with no row at all");
    }

    #[test]
    fn a_sub_area_names_itself_and_reports_its_zone() {
        let areas = areas();
        assert_eq!(areas.get(9).expect("the row").name, "Northshire Valley");
        assert_eq!(areas.zone_name(9), "Elwynn Forest");
        assert_eq!(areas.sub_zone_name(9), "Northshire Valley");
    }

    /// A zone's sub-area is empty, not its own name — which is the whole of
    /// what the minimap's second line shows in open country.
    #[test]
    fn a_zone_is_its_own_zone_and_has_no_sub_area() {
        let areas = areas();
        assert!(areas.get(12).expect("the row").is_zone());
        assert_eq!(areas.zone_name(12), "Elwynn Forest");
        assert_eq!(areas.sub_zone_name(12), "");
    }

    /// An id nothing carries answers empty rather than panicking: a patched
    /// `MCNK` can name an area this table does not have.
    #[test]
    fn an_unknown_area_answers_empty() {
        let areas = areas();
        assert_eq!(areas.zone_name(9999), "");
        assert_eq!(areas.sub_zone_name(9999), "");
        assert!(areas.zone_of(9999).is_none());
    }

    /// A parent the table does not carry stops the walk at the area itself,
    /// which is a real place name rather than nothing.
    #[test]
    fn a_dangling_parent_stops_at_the_area_itself() {
        let areas = areas();
        assert_eq!(areas.zone_of(88).expect("the row").id, 88);
    }

    #[test]
    fn the_counts_split_zones_from_sub_areas() {
        let areas = areas();
        assert_eq!(areas.count(), 3);
        assert_eq!(areas.zone_count(), 1);
    }
}
