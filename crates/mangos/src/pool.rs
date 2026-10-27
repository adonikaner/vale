//! The pool tables: sets of spawns of which the server keeps a few in the
//! world at a time.
//!
//! ## Six tables
//!
//! ```text
//! pool_template             one pool: how many of its members are spawned at
//!                           once, its description and flags, by entry and
//!                           patch range
//! pool_creature             a creature spawn in a pool, with its chance
//! pool_gameobject           a game object spawn in a pool, with its chance
//! pool_pool                 a pool in a mother pool, with its chance
//! pool_creature_template    every spawn of a creature entry in a pool
//! pool_gameobject_template  every spawn of a game object entry in a pool
//! ```
//!
//! On the reference database there are 7,440 pools, 140 creature members,
//! 29,466 game object members (the gathering nodes), 5,583 pools in mother
//! pools, 56 creature entries and 100 game object entries.
//!
//! ## What the server reads
//!
//! `PoolManager::LoadFromDB` (`PoolManager.cpp:668`) runs at startup only.
//! A pool row is read when the server's `WowPatch` is between its
//! `patch_min` and `patch_max`; `max_limit` 0 means no limit and an empty
//! description is logged, and neither is refused. A member row outside its
//! patch range is skipped; so is one whose spawn does not exist, whose pool
//! is not read, or whose chance is below 0 or above 100
//! (`CheckPoolAndChance`, line 653). The members of one pool must stand on
//! one map. A pool may not be its own mother, and a chain of mothers that
//! comes back to where it started has its last link removed.
//!
//! A member's `chance` 0 means an equal share of what the explicit chances
//! leave. A pool whose members all have an explicit chance must have chances
//! summing to 100 (`PoolGroup::CheckPool`, line 171), or the server does not
//! spawn it at all; [`chances_fault`] states that rule.
//!
//! ## A row is a keyed row of the project's store
//!
//! On `crate::addon`'s terms. A pool is keyed by `(entry, patch_min,
//! patch_max)`, its primary key; a member by its spawn's `guid`, so a spawn
//! is in one pool at most; a pool in a mother pool by `pool_id`; an entry
//! member by `id`.

use crate::row::{Assignment, Key, Table};

pub use crate::schema::{Bit, Column, Group, Kind, Row, RowValue, Value};

pub const TEMPLATE: &str = "pool_template";
pub const CREATURE: &str = "pool_creature";
pub const GAMEOBJECT: &str = "pool_gameobject";
pub const POOL: &str = "pool_pool";
pub const CREATURE_TEMPLATE: &str = "pool_creature_template";
pub const GAMEOBJECT_TEMPLATE: &str = "pool_gameobject_template";

/// The six tables, in the order a plan writes them: a pool before the rows
/// that name it, since a member of a pool the server does not read is
/// skipped.
pub const TABLES: &[Table] = &[
    TEMPLATE_TABLE,
    CREATURE_TABLE,
    GAMEOBJECT_TABLE,
    POOL_TABLE,
    CREATURE_TEMPLATE_TABLE,
    GAMEOBJECT_TEMPLATE_TABLE,
];

pub const TEMPLATE_TABLE: Table = Table::new(TEMPLATE, &TEMPLATE_COLUMNS);
pub const CREATURE_TABLE: Table = Table::new(CREATURE, &CREATURE_COLUMNS);
pub const GAMEOBJECT_TABLE: Table = Table::new(GAMEOBJECT, &GAMEOBJECT_COLUMNS);
pub const POOL_TABLE: Table = Table::new(POOL, &POOL_COLUMNS);
pub const CREATURE_TEMPLATE_TABLE: Table = Table::new(CREATURE_TEMPLATE, &CREATURE_TEMPLATE_COLUMNS);
pub const GAMEOBJECT_TEMPLATE_TABLE: Table = Table::new(GAMEOBJECT_TEMPLATE, &GAMEOBJECT_TEMPLATE_COLUMNS);

/// `PoolFlags`, from `PoolManager.h:34`. The server sets `AUTO_SPAWN` on
/// every pool it reads and clears it on a pool in a mother pool and on one
/// whose chances do not add up.
pub const POOL_FLAGS: [Bit; 2] = [
    Bit { bit: 0x1, name: "AUTO_SPAWN", about: "set by the server on every pool it spawns by itself" },
    Bit { bit: 0x2, name: "MAXLIMIT_SCALING_LINEAR", about: "max_limit grows with the realm's population above a Blizzlike one" },
];

/// `PoolObject::ObjectFlags`, from `PoolManager.h:65`.
pub const MEMBER_FLAGS: [Bit; 1] = [Bit {
    bit: 0x1,
    name: "SPAWN_ENABLE_IF_WORLD_POP_OVER_BLIZZLIKE",
    about: "spawned only while the realm has more players than a Blizzlike one",
}];

/// The pool's columns, in the table's order.
pub const TEMPLATE_COLUMNS: [Column; 7] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the pool" },
    Column { name: "max_limit", kind: Kind::Unsigned, group: Group::Respawn, about: "how many members are spawned at once; 0 is no limit" },
    Column { name: "description", kind: Kind::Text, group: Group::Identity, about: "what the pool is, for a reader; the server logs an empty one" },
    Column { name: "flags", kind: Kind::Flags(&POOL_FLAGS), group: Group::Respawn, about: "how the pool spawns" },
    Column { name: "instance", kind: Kind::Signed, group: Group::Respawn, about: "the continent instance the pool spawns in, when continents are instanced; 0 for the server's choice" },
    Column { name: "patch_min", kind: Kind::Key, group: Group::Respawn, about: "lowest content patch the pool is read in" },
    Column { name: "patch_max", kind: Kind::Key, group: Group::Respawn, about: "highest content patch the pool is read in" },
];

/// A spawn member's columns: `pool_creature` and `pool_gameobject` have the
/// same seven.
const fn spawn_member_columns(about: &'static str) -> [Column; 7] {
    [
        Column { name: "guid", kind: Kind::Key, group: Group::Identity, about },
        Column { name: "pool_entry", kind: Kind::Ref(TEMPLATE), group: Group::Identity, about: "the pool the spawn is in" },
        Column { name: "chance", kind: Kind::Float, group: Group::Respawn, about: "percent chance the spawn is the one chosen; 0 for an equal share of what the explicit chances leave" },
        Column { name: "description", kind: Kind::Text, group: Group::Identity, about: "what the member is, for a reader" },
        Column { name: "flags", kind: Kind::Flags(&MEMBER_FLAGS), group: Group::Respawn, about: "when the member may spawn" },
        Column { name: "patch_min", kind: Kind::Choice(&crate::creature::PATCHES), group: Group::Respawn, about: "lowest content patch the member is read in" },
        Column { name: "patch_max", kind: Kind::Choice(&crate::creature::PATCHES), group: Group::Respawn, about: "highest content patch the member is read in" },
    ]
}

pub const CREATURE_COLUMNS: [Column; 7] = spawn_member_columns("the creature spawn in the pool");
pub const GAMEOBJECT_COLUMNS: [Column; 7] = spawn_member_columns("the game object spawn in the pool");

/// `pool_pool`'s columns.
pub const POOL_COLUMNS: [Column; 5] = [
    Column { name: "pool_id", kind: Kind::Key, group: Group::Identity, about: "the pool that is a member" },
    Column { name: "mother_pool", kind: Kind::Ref(TEMPLATE), group: Group::Identity, about: "the pool it is a member of" },
    Column { name: "chance", kind: Kind::Float, group: Group::Respawn, about: "percent chance the pool is the one chosen; 0 for an equal share" },
    Column { name: "description", kind: Kind::Text, group: Group::Identity, about: "what the member is, for a reader" },
    Column { name: "flags", kind: Kind::Flags(&MEMBER_FLAGS), group: Group::Respawn, about: "when the member may spawn" },
];

/// An entry member's columns: every spawn of the entry is a member.
const fn entry_member_columns(about: &'static str) -> [Column; 7] {
    [
        Column { name: "id", kind: Kind::Key, group: Group::Identity, about },
        Column { name: "pool_entry", kind: Kind::Ref(TEMPLATE), group: Group::Identity, about: "the pool every spawn of the entry is in" },
        Column { name: "chance", kind: Kind::Float, group: Group::Respawn, about: "percent chance a spawn of the entry is the one chosen; 0 for an equal share" },
        Column { name: "description", kind: Kind::Text, group: Group::Identity, about: "what the member is, for a reader" },
        Column { name: "flags", kind: Kind::Flags(&MEMBER_FLAGS), group: Group::Respawn, about: "when the member may spawn" },
        Column { name: "patch_min", kind: Kind::Choice(&crate::creature::PATCHES), group: Group::Respawn, about: "lowest content patch the member is read in" },
        Column { name: "patch_max", kind: Kind::Choice(&crate::creature::PATCHES), group: Group::Respawn, about: "highest content patch the member is read in" },
    ]
}

pub const CREATURE_TEMPLATE_COLUMNS: [Column; 7] = entry_member_columns("the creature_template entry whose spawns are in the pool");
pub const GAMEOBJECT_TEMPLATE_COLUMNS: [Column; 7] = entry_member_columns("the gameobject_template entry whose spawns are in the pool");

/// The key of a pool.
pub fn pool_key(entry: u32, patch_min: u32, patch_max: u32) -> Key {
    Key(vec![
        ("entry".to_string(), entry.to_string()),
        ("patch_min".to_string(), patch_min.to_string()),
        ("patch_max".to_string(), patch_max.to_string()),
    ])
}

/// The key of a spawn member, in either spawn table.
pub fn member_key(guid: u64) -> Key {
    Key::one("guid", guid)
}

/// The key of a pool in a mother pool.
pub fn child_key(pool: u32) -> Key {
    Key::one("pool_id", u64::from(pool))
}

/// The key of an entry member, in either entry table.
pub fn entry_member_key(entry: u32) -> Key {
    Key::one("id", u64::from(entry))
}

/// Which kind of thing a member row places in a pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Of {
    Creature,
    GameObject,
    /// A pool in a mother pool.
    Pool,
    /// Every spawn of a creature entry.
    CreatureEntry,
    /// Every spawn of a game object entry.
    GameObjectEntry,
}

impl Of {
    /// The member table.
    pub fn table(self) -> &'static str {
        match self {
            Of::Creature => CREATURE,
            Of::GameObject => GAMEOBJECT,
            Of::Pool => POOL,
            Of::CreatureEntry => CREATURE_TEMPLATE,
            Of::GameObjectEntry => GAMEOBJECT_TEMPLATE,
        }
    }

    /// The member table's column that names the pool.
    pub fn pool_column(self) -> &'static str {
        match self {
            Of::Pool => "mother_pool",
            _ => "pool_entry",
        }
    }

    /// The member table's key column.
    pub fn id_column(self) -> &'static str {
        match self {
            Of::Creature | Of::GameObject => "guid",
            Of::Pool => "pool_id",
            Of::CreatureEntry | Of::GameObjectEntry => "id",
        }
    }

    /// The key of the member with this id.
    pub fn key(self, id: u64) -> Key {
        Key::one(self.id_column(), id)
    }

    /// Whether the table has a patch range; `pool_pool` has none.
    pub fn has_patches(self) -> bool {
        self != Of::Pool
    }

    /// Every member table, in [`TABLES`]' order.
    pub const ALL: [Of; 5] = [Of::Creature, Of::GameObject, Of::Pool, Of::CreatureEntry, Of::GameObjectEntry];

    /// The kind whose table is `table`.
    pub fn of_table(table: &str) -> Option<Of> {
        Self::ALL.into_iter().find(|of| of.table() == table)
    }
}

/// One `pool_template` row.
#[derive(Debug, Clone, PartialEq)]
pub struct Pool {
    pub entry: u32,
    pub max_limit: u32,
    pub description: String,
    pub flags: u32,
    pub instance: i32,
    pub patch_min: u32,
    pub patch_max: u32,
}

impl Pool {
    /// A pool that spawns one member at a time, read at every patch.
    pub fn new(entry: u32, description: &str) -> Pool {
        Pool {
            entry,
            max_limit: 1,
            description: description.to_string(),
            flags: 0,
            instance: 0,
            patch_min: 0,
            patch_max: crate::conn::DEFAULT_WOW_PATCH,
        }
    }

    pub fn key(&self) -> Key {
        pool_key(self.entry, self.patch_min, self.patch_max)
    }

    /// Whether the server reads the row at `wow_patch`.
    pub fn read_at(&self, wow_patch: u32) -> bool {
        (self.patch_min..=self.patch_max).contains(&wow_patch)
    }

    /// Every column that is not the key, as SQL literals.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "max_limit", value: self.max_limit.to_string() },
            Assignment { column: "description", value: crate::sql::text(&self.description) },
            Assignment { column: "flags", value: self.flags.to_string() },
            Assignment { column: "instance", value: self.instance.to_string() },
        ]
    }

    pub fn from_row(row: &Row) -> Option<Pool> {
        let unsigned = |column: &str| row.integer(column).unwrap_or(0).max(0) as u32;
        Some(Pool {
            entry: row.integer("entry")?.max(0) as u32,
            max_limit: unsigned("max_limit"),
            description: row.text("description").unwrap_or("").to_string(),
            flags: unsigned("flags"),
            instance: row.integer("instance").unwrap_or(0) as i32,
            patch_min: unsigned("patch_min"),
            patch_max: unsigned("patch_max"),
        })
    }

    /// What the server would do with the row that the row alone shows.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.patch_min > self.patch_max {
            out.push(format!(
                "patch_min {} is above patch_max {}, so no patch reads the pool",
                self.patch_min, self.patch_max
            ));
        }
        if self.description.trim().is_empty() {
            out.push("description is empty; the server logs it".to_string());
        }
        out
    }
}

/// One member row of any of the five member tables.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub of: Of,
    /// The spawn's guid, the child pool's id, or the entry.
    pub id: u64,
    pub pool: u32,
    pub chance: f32,
    pub description: String,
    pub flags: u32,
    /// `pool_pool` has no patch range; it reads as every patch.
    pub patch_min: u32,
    pub patch_max: u32,
}

impl Member {
    /// A member at an equal share, read at every patch.
    pub fn new(of: Of, id: u64, pool: u32, description: &str) -> Member {
        Member {
            of,
            id,
            pool,
            chance: 0.0,
            description: description.to_string(),
            flags: 0,
            patch_min: 0,
            patch_max: crate::conn::DEFAULT_WOW_PATCH,
        }
    }

    pub fn key(&self) -> Key {
        self.of.key(self.id)
    }

    /// Whether the server reads the row at `wow_patch`.
    pub fn read_at(&self, wow_patch: u32) -> bool {
        (self.patch_min..=self.patch_max).contains(&wow_patch)
    }

    /// Every column that is not the key, as SQL literals.
    pub fn assignments(&self) -> Vec<Assignment> {
        let mut out = vec![
            Assignment { column: self.of.pool_column(), value: self.pool.to_string() },
            Assignment { column: "chance", value: crate::sql::float(self.chance) },
            Assignment { column: "description", value: crate::sql::text(&self.description) },
            Assignment { column: "flags", value: self.flags.to_string() },
        ];
        if self.of.has_patches() {
            out.push(Assignment { column: "patch_min", value: self.patch_min.to_string() });
            out.push(Assignment { column: "patch_max", value: self.patch_max.to_string() });
        }
        out
    }

    /// One row of `of`'s table as the database answered it.
    pub fn from_row(of: Of, row: &Row) -> Option<Member> {
        let unsigned = |column: &str| row.integer(column).unwrap_or(0).max(0) as u32;
        Some(Member {
            of,
            id: row.integer(of.id_column())?.max(0) as u64,
            pool: unsigned(of.pool_column()),
            chance: row.number("chance").unwrap_or(0.0) as f32,
            description: row.text("description").unwrap_or("").to_string(),
            flags: unsigned("flags"),
            patch_min: match of.has_patches() {
                true => unsigned("patch_min"),
                false => 0,
            },
            patch_max: match of.has_patches() {
                true => unsigned("patch_max"),
                false => crate::conn::DEFAULT_WOW_PATCH,
            },
        })
    }

    /// Why the server would skip the row, from the row alone: `CheckPoolAndChance`
    /// and a pool that is its own mother. Whether the pool and the spawn
    /// exist, and whether the members share a map, are the caller's.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !(0.0..=100.0).contains(&self.chance) {
            out.push(format!("chance {} is outside 0 to 100; the server skips the row", self.chance));
        }
        if self.of == Of::Pool && self.id == u64::from(self.pool) {
            out.push(format!("pool {} is its own mother; the server skips the row", self.pool));
        }
        if self.patch_min > self.patch_max {
            out.push(format!(
                "patch_min {} is above patch_max {}, so no patch reads the row",
                self.patch_min, self.patch_max
            ));
        }
        out
    }
}

/// The fault in a pool's chances, or `None`: when every member read has an
/// explicit chance, the chances must sum to 100, or the server does not spawn
/// the pool (`PoolGroup::CheckPool`). The server checks each kind of member,
/// creatures, game objects and pools, on its own.
pub fn chances_fault(chances: &[f32]) -> Option<String> {
    if chances.is_empty() || chances.contains(&0.0) {
        return None;
    }
    let sum: f32 = chances.iter().sum();
    match (sum - 100.0).abs() < 0.001 {
        true => None,
        false => Some(format!(
            "every member has an explicit chance and they sum to {sum}, not 100; the server does not spawn the pool"
        )),
    }
}

// ---------------------------------------------------------------------------
// The queries
// ---------------------------------------------------------------------------

/// Every pool row, at every patch. 7,440 rows on the reference database, so
/// an editor reads the table whole and finds a pool in it.
pub fn pools_query() -> String {
    format!("SELECT * FROM {} ORDER BY `entry`, `patch_min`", crate::sql::name(TEMPLATE))
}

/// Every row of one member table, at every patch, on [`pools_query`]'s
/// terms. `pool_gameobject` is the largest, at 29,466 rows.
pub fn members_query(of: Of) -> String {
    format!(
        "SELECT * FROM {} ORDER BY {}, {}",
        crate::sql::name(of.table()),
        crate::sql::name(of.pool_column()),
        crate::sql::name(of.id_column())
    )
}

/// The largest pool entry, which a new pool is numbered above.
pub fn max_entry_query() -> String {
    format!("SELECT MAX(`entry`) AS `n` FROM {}", crate::sql::name(TEMPLATE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(values: &[(&str, &str)]) -> Row {
        values
            .iter()
            .map(|(column, value)| (column.to_string(), Some(value.to_string())))
            .collect()
    }

    /// Each member table names its pool and its id by its own columns, and
    /// `pool_pool` has no patch range.
    #[test]
    fn each_member_table_is_read_by_its_own_columns() {
        let node = Member::from_row(
            Of::GameObject,
            &row(&[("guid", "5000"), ("pool_entry", "1024"), ("chance", "0"), ("description", "Copper Vein"), ("flags", "0"), ("patch_min", "0"), ("patch_max", "10")]),
        )
        .unwrap();
        assert_eq!((node.id, node.pool), (5000, 1024));
        assert_eq!(node.key(), member_key(5000));
        let child = Member::from_row(Of::Pool, &row(&[("pool_id", "7"), ("mother_pool", "3"), ("chance", "50"), ("description", ""), ("flags", "0")])).unwrap();
        assert_eq!((child.id, child.pool, child.patch_max), (7, 3, crate::conn::DEFAULT_WOW_PATCH));
        assert!(child.assignments().iter().all(|a| a.column != "patch_min"));
        assert_eq!(child.assignments()[0].column, "mother_pool");
        assert_eq!(Of::of_table("pool_creature_template"), Some(Of::CreatureEntry));
    }

    /// The loader's checks: chance, a pool its own mother, and the sum of
    /// explicit chances.
    #[test]
    fn a_member_or_pool_the_server_skips_says_why() {
        assert!(Member { chance: 120.0, ..Member::new(Of::Creature, 1, 2, "x") }.check()[0].contains("120"));
        assert!(Member::new(Of::Pool, 4, 4, "x").check()[0].contains("its own mother"));
        assert_eq!(chances_fault(&[50.0, 50.0]), None);
        assert_eq!(chances_fault(&[50.0, 0.0]), None);
        assert!(chances_fault(&[30.0, 30.0]).unwrap().contains("sum to 60"));
        assert!(Pool::new(9, "").check()[0].contains("description is empty"));
        assert!(Pool::new(9, "Copper").read_at(10));
        assert_eq!(pool_key(9, 0, 10).text(), "entry=9;patch_min=0;patch_max=10");
    }

    /// The queries read the tables whole.
    #[test]
    fn the_queries_read_each_table() {
        assert_eq!(pools_query(), "SELECT * FROM `pool_template` ORDER BY `entry`, `patch_min`");
        assert!(members_query(Of::Pool).ends_with("ORDER BY `mother_pool`, `pool_id`"));
        assert!(members_query(Of::GameObject).starts_with("SELECT * FROM `pool_gameobject`"));
        for (of, table) in Of::ALL.into_iter().zip(&TABLES[1..]) {
            assert_eq!(of.table(), table.name);
        }
    }
}
