//! Graveyards on the server: which safe place serves which zone, and which way
//! a spirit faces there.
//!
//! The places themselves are `WorldSafeLocs.dbc`, which the server reads as a
//! file from `DataDir\5875\dbc\`. Two tables say how they are used:
//!
//! ```text
//! game_graveyard_zone     a link from a safe place to a zone, for one side or
//!                         both. ObjectMgr::LoadGraveyardZones (ObjectMgr.cpp:7453)
//!                         reads the rows whose patch_min..patch_max holds the
//!                         server's WowPatch. Live on `.reload game_graveyard_zone`;
//!                         the loader clears its map first
//! world_safe_locs_facing  the facing a released spirit takes at a safe place.
//!                         Read at startup (World.cpp:1390); no reload
//! ```
//!
//! When a character releases, `GetClosestGraveYard` finds the zone and area
//! the corpse is in and takes the nearest safe place linked to that area for
//! the character's side, then to its zone. A zone with no link sends the
//! spirit to no graveyard at all.
//!
//! ## What the loader skips
//!
//! A link naming a safe place `WorldSafeLocs.dbc` does not hold, a zone
//! `area_template` does not hold, or a faction other than 0, 67 (Horde) or
//! 469 (Alliance). [`Link::check`] makes the last check; the first two need
//! the tables and are the form's.

use crate::row::{Assignment, Key};
use crate::schema::{Column, Group, Kind, Value};

pub const ZONE: &str = "game_graveyard_zone";
pub const FACING: &str = "world_safe_locs_facing";

/// Both tables, in the order a plan writes them.
pub const TABLES: [&str; 2] = [ZONE, FACING];

/// The static name for one of [`TABLES`] read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// The three values the loader accepts in `faction`: vmangos' `TEAM_NONE`,
/// `HORDE` and `ALLIANCE`.
pub const FACTIONS: [Value; 3] = [
    Value { value: 0, name: "Both" },
    Value { value: 67, name: "Horde" },
    Value { value: 469, name: "Alliance" },
];

/// The patch range a new link covers: every content patch.
pub const PATCH_MIN: u32 = 0;
pub const PATCH_MAX: u32 = 10;

/// `game_graveyard_zone`, in table order.
pub const ZONE_COLUMNS: [Column; 5] = [
    Column { name: "id", kind: Kind::Key, group: Group::Identity, about: "the WorldSafeLocs.dbc id of the safe place" },
    Column { name: "ghost_zone", kind: Kind::Key, group: Group::Identity, about: "the AreaTable id of the zone or area it serves" },
    Column { name: "faction", kind: Kind::Choice(&FACTIONS), group: Group::Place, about: "which side it serves: 0 both, 67 Horde, 469 Alliance" },
    Column { name: "patch_min", kind: Kind::Unsigned, group: Group::Identity, about: "the first content patch the link exists in" },
    Column { name: "patch_max", kind: Kind::Key, group: Group::Identity, about: "the last content patch the link exists in" },
];

/// `world_safe_locs_facing`, in table order.
pub const FACING_COLUMNS: [Column; 2] = [
    Column { name: "id", kind: Kind::Key, group: Group::Identity, about: "the WorldSafeLocs.dbc id" },
    Column { name: "orientation", kind: Kind::Float, group: Group::Place, about: "the facing a released spirit takes, in radians" },
];

/// Every column of one of [`TABLES`].
pub fn columns_of(table: &str) -> &'static [Column] {
    match table {
        ZONE => &ZONE_COLUMNS,
        FACING => &FACING_COLUMNS,
        _ => &[],
    }
}

/// One column, by name, of one of [`TABLES`].
pub fn column(table: &str, name: &str) -> Option<&'static Column> {
    columns_of(table).iter().find(|column| column.name == name)
}

/// The key of a link: the table's primary key.
pub fn link_key(safe_loc: u32, zone: u32, patch_max: u32) -> Key {
    Key(vec![
        ("id".to_string(), safe_loc.to_string()),
        ("ghost_zone".to_string(), zone.to_string()),
        ("patch_max".to_string(), patch_max.to_string()),
    ])
}

/// The key of a facing.
pub fn facing_key(safe_loc: u32) -> Key {
    Key::one("id", u64::from(safe_loc))
}

/// One link, as the server uses it.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub safe_loc: u32,
    pub zone: u32,
    pub faction: u32,
    pub patch_min: u32,
    pub patch_max: u32,
}

impl Link {
    /// A new link for both sides over every content patch.
    pub fn new(safe_loc: u32, zone: u32) -> Link {
        Link {
            safe_loc,
            zone,
            faction: 0,
            patch_min: PATCH_MIN,
            patch_max: PATCH_MAX,
        }
    }

    pub fn key(&self) -> Key {
        link_key(self.safe_loc, self.zone, self.patch_max)
    }

    /// Every column but the key, as a created row names them.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "faction", value: self.faction.to_string() },
            Assignment { column: "patch_min", value: self.patch_min.to_string() },
        ]
    }

    /// A row as the database returned it.
    pub fn from_row(row: &crate::schema::Row) -> Option<Link> {
        use crate::schema::RowValue;
        let int = |column: &str| row.text(column)?.parse::<u32>().ok();
        Some(Link {
            safe_loc: int("id")?,
            zone: int("ghost_zone")?,
            faction: int("faction").unwrap_or(0),
            patch_min: int("patch_min").unwrap_or(PATCH_MIN),
            patch_max: int("patch_max").unwrap_or(PATCH_MAX),
        })
    }

    /// Why the server would skip this row, from the row alone.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !FACTIONS.iter().any(|known| known.value == self.faction) {
            out.push(format!("faction {} is not 0, 67 or 469", self.faction));
        }
        if self.patch_min > self.patch_max {
            out.push(format!("patch_min {} is above patch_max {}", self.patch_min, self.patch_max));
        }
        out
    }
}

/// Why the server would skip a created row of one of [`TABLES`], from its
/// columns alone.
pub fn check_created(table: &str, row: &crate::schema::Row) -> Vec<String> {
    match table {
        ZONE => Link::from_row(row)
            .map(|link| link.check())
            .unwrap_or_else(|| vec!["the row does not name a safe place and a zone".to_string()]),
        _ => Vec::new(),
    }
}

/// The links the server uses at `wow_patch`: `LoadGraveyardZones`' own
/// selection.
pub fn links_query(wow_patch: u32) -> String {
    format!("SELECT * FROM `{ZONE}` WHERE {wow_patch} BETWEEN `patch_min` AND `patch_max` ORDER BY `id`, `ghost_zone`")
}

/// Every facing.
pub fn facings_query() -> String {
    format!("SELECT * FROM `{FACING}` ORDER BY `id`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_keyed_by_place_zone_and_last_patch() {
        let link = Link::new(100, 132);
        assert_eq!(link.key().where_clause(), "`id` = 100 AND `ghost_zone` = 132 AND `patch_max` = 10");
        assert!(link.check().is_empty());
        assert_eq!(Link { faction: 1, ..link.clone() }.check().len(), 1);
        assert!(links_query(10).contains("10 BETWEEN `patch_min` AND `patch_max`"));
    }

    #[test]
    fn a_row_reads_back_as_the_link_it_was() {
        let mut row = crate::schema::Row::new();
        for (column, value) in [("id", "100"), ("ghost_zone", "132"), ("faction", "469"), ("patch_min", "0"), ("patch_max", "10")] {
            row.insert(column.into(), Some(value.into()));
        }
        let link = Link::from_row(&row).unwrap();
        assert_eq!(link.faction, 469);
        assert!(check_created(ZONE, &row).is_empty());
    }
}
