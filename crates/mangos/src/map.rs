//! `map_template`: the server's maps.
//!
//! vmangos does not read `Map.dbc`. `ObjectMgr::LoadMapTemplate`
//! (`ObjectMgr.cpp:6755`) loads this table progressively by patch: per entry,
//! the row with the highest `patch` at or below the server's `WowPatch`. It is
//! live on `.reload map_template`, but a map whose row arrives after startup
//! has no grids loaded for it until the server restarts.
//!
//! ```text
//! entry, patch          the key; a new map's row is written at patch 0
//! parent                for a dungeon, the map it stands in, or 0
//! map_type              0 world, 1 dungeon, 2 raid, 3 battleground
//! linked_zone           an AreaTable id, the same value as Map.dbc field 19
//! player_limit          how many characters an instance holds
//! reset_delay           days between a raid's resets; 0 for none
//! ghost_entrance_map    where a dead character's ghost is sent to walk in:
//! ghost_entrance_x, _y  a map and a point on it, or -1 for none
//! map_name              the server's own copy of the name
//! script_name           the instance script, or empty
//! ```
//!
//! ## What the loader skips
//!
//! A dungeon whose `parent` names no map, or names a continent; a ghost
//! entrance whose point is outside its map's grid, or whose map has no row.
//! [`Template::check`] makes the checks the row alone can show.

use crate::row::{Assignment, Key};
use crate::schema::{Column, Group, Kind, Value};

pub const TEMPLATE: &str = "map_template";

/// The one table, as the row subjects list their tables.
pub const TABLES: [&str; 1] = [TEMPLATE];

/// The static name for [`TEMPLATE`] read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// `map_type`'s values, vmangos' `MAP_COMMON` to `MAP_BATTLEGROUND`.
pub const MAP_TYPES: [Value; 4] = [
    Value { value: 0, name: "World" },
    Value { value: 1, name: "Dungeon" },
    Value { value: 2, name: "Raid" },
    Value { value: 3, name: "Battleground" },
];

/// `map_template`, in table order.
pub const COLUMNS: [Column; 12] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the Map.dbc id" },
    Column { name: "patch", kind: Kind::Key, group: Group::Identity, about: "the content patch the row belongs to" },
    Column { name: "parent", kind: Kind::Ref("Map"), group: Group::Place, about: "for a dungeon, the map it stands in, or 0" },
    Column { name: "map_type", kind: Kind::Choice(&MAP_TYPES), group: Group::Identity, about: "what kind of place the map is" },
    Column { name: "linked_zone", kind: Kind::Ref("AreaTable"), group: Group::Place, about: "the area the map belongs to, as Map.dbc field 19 holds it" },
    Column { name: "player_limit", kind: Kind::Unsigned, group: Group::Requirements, about: "how many characters one instance holds" },
    Column { name: "reset_delay", kind: Kind::Unsigned, group: Group::Requirements, about: "days between a raid's resets, or 0" },
    Column { name: "ghost_entrance_map", kind: Kind::Signed, group: Group::Place, about: "the map a dead character's ghost walks in from, or -1" },
    Column { name: "ghost_entrance_x", kind: Kind::Float, group: Group::Place, about: "north, in yards, on the ghost entrance's map" },
    Column { name: "ghost_entrance_y", kind: Kind::Float, group: Group::Place, about: "west, in yards" },
    Column { name: "map_name", kind: Kind::Text, group: Group::Identity, about: "the server's own copy of the name" },
    Column { name: "script_name", kind: Kind::Text, group: Group::Behaviour, about: "the instance script the server runs, or empty" },
];

/// Every column of [`TEMPLATE`].
pub fn columns_of(table: &str) -> &'static [Column] {
    match table {
        TEMPLATE => &COLUMNS,
        _ => &[],
    }
}

/// One column, by name.
pub fn column(table: &str, name: &str) -> Option<&'static Column> {
    columns_of(table).iter().find(|column| column.name == name)
}

/// The key of a row.
pub fn key(entry: u32, patch: u32) -> Key {
    Key::two(("entry", u64::from(entry)), ("patch", u64::from(patch)))
}

/// One row, as a new map's is written.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub entry: u32,
    pub parent: u32,
    pub map_type: u32,
    pub linked_zone: u32,
    pub player_limit: u32,
    pub reset_delay: u32,
    /// A map id, or -1.
    pub ghost_map: i32,
    pub ghost_at: [f32; 2],
    pub name: String,
}

impl Template {
    /// A new map's row at patch 0, with no parent and no ghost entrance.
    pub fn new(entry: u32, map_type: u32, name: &str) -> Template {
        Template {
            entry,
            parent: 0,
            map_type,
            linked_zone: 0,
            player_limit: 0,
            reset_delay: 0,
            ghost_map: -1,
            ghost_at: [0.0, 0.0],
            name: name.to_string(),
        }
    }

    pub fn key(&self) -> Key {
        key(self.entry, 0)
    }

    /// Every column but the key, as a created row names them.
    pub fn assignments(&self) -> Vec<Assignment> {
        let f = crate::sql::float;
        vec![
            Assignment { column: "parent", value: self.parent.to_string() },
            Assignment { column: "map_type", value: self.map_type.to_string() },
            Assignment { column: "linked_zone", value: self.linked_zone.to_string() },
            Assignment { column: "player_limit", value: self.player_limit.to_string() },
            Assignment { column: "reset_delay", value: self.reset_delay.to_string() },
            Assignment { column: "ghost_entrance_map", value: self.ghost_map.to_string() },
            Assignment { column: "ghost_entrance_x", value: f(self.ghost_at[0]) },
            Assignment { column: "ghost_entrance_y", value: f(self.ghost_at[1]) },
            Assignment { column: "map_name", value: crate::sql::text(&self.name) },
            Assignment { column: "script_name", value: crate::sql::text("") },
        ]
    }

    /// Why the server would skip this row, from the row alone.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !MAP_TYPES.iter().any(|known| known.value == self.map_type) {
            out.push(format!("map_type {} is not 0 to 3", self.map_type));
        }
        // A parent of 0 means none, so of the two continents only Kalimdor (1)
        // can be named; the loader skips a dungeon whose parent is either.
        if matches!(self.map_type, 1 | 2) && self.parent == 1 {
            out.push("a dungeon's parent must not be a continent".to_string());
        }
        out
    }
}

/// Why the server would skip a created row, from its columns alone.
pub fn check_created(table: &str, row: &crate::schema::Row) -> Vec<String> {
    use crate::schema::RowValue;
    if table != TEMPLATE {
        return Vec::new();
    }
    let int = |column: &str| row.text(column).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
    let template = Template {
        entry: int("entry") as u32,
        parent: int("parent") as u32,
        map_type: int("map_type") as u32,
        linked_zone: int("linked_zone") as u32,
        player_limit: int("player_limit") as u32,
        reset_delay: int("reset_delay") as u32,
        ghost_map: int("ghost_entrance_map") as i32,
        ghost_at: [0.0, 0.0],
        name: String::new(),
    };
    template.check()
}

/// The row the server uses for one map: the highest patch at or below
/// `wow_patch`.
pub fn winning_row_query(entry: u32, wow_patch: u32) -> String {
    format!(
        "SELECT * FROM `{TEMPLATE}` WHERE `entry` = {entry} AND `patch` <= {wow_patch} \
         ORDER BY `patch` DESC LIMIT 1"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_map_names_every_column_but_its_key() {
        let template = Template::new(534, 1, "The Islands");
        let names: Vec<&str> = template.assignments().iter().map(|a| a.column).collect();
        let expected: Vec<&str> = COLUMNS.iter().filter(|c| c.editable()).map(|c| c.name).collect();
        assert_eq!(names, expected);
        assert_eq!(template.key().where_clause(), "`entry` = 534 AND `patch` = 0");
        assert!(template.check().is_empty());
        assert_eq!(Template { map_type: 7, ..template.clone() }.check().len(), 1);
        assert_eq!(Template { parent: 1, ..template }.check().len(), 1);
    }
}
