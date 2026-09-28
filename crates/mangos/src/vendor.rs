//! `npc_vendor` and `npc_vendor_template`: what a creature sells.
//!
//! ## Two tables, one schema
//!
//! `ObjectMgr::LoadVendors` (`ObjectMgr.cpp:10780`) reads both with one
//! `SELECT`. What differs is what an `entry` is:
//!
//! ```text
//! npc_vendor           creature_template.entry: the creature's own list
//! npc_vendor_template  creature_template.vendor_id: a list shared by every
//!                      creature that names it
//! ```
//!
//! The client is sent the creature's own list and then its template list, in
//! that order (`WorldSession::SendListInventory`, `ItemHandler.cpp:751`). 41
//! creatures on the reference database name a `vendor_id`.
//!
//! ## A row is keyed by `(entry, item)`
//!
//! That is the primary key of both tables, so an item is sold at most once per
//! list. `slot` is an ordinary column: the loader reads `ORDER BY entry, slot`,
//! and the list the client is sent is in that order. The shipped rows number
//! their slots from 1; some lists repeat a number, and two rows with one slot
//! are sent in whatever order the database returns them.
//!
//! ## Limited stock
//!
//! `maxcount` is how many the vendor holds, 0 for no limit; `incrtime` is the
//! seconds before one more is restocked. The server refuses a row with one and
//! not the other (`ObjectMgr::IsVendorItemValid`, `ObjectMgr.cpp:11260`).
//! `itemflags` changes the restock delay: [`FLAGS`].
//!
//! ## What the loader skips
//!
//! `IsVendorItemValid` skips a row, with one log line each, when:
//!
//! * the creature has no `creature_template` row (the own list only);
//! * the item is not in `item_template`;
//! * `maxcount` and `incrtime` disagree, as above;
//! * `condition_id` names no `conditions` row;
//! * the item is already in the list, or, for an own list, in the creature's
//!   template list;
//! * the two lists already hold 255 items together.
//!
//! The `SELECT` itself leaves out every row whose item is in `forbidden_items`
//! at the server's `WowPatch`. [`Ware::check`] makes the checks that need no
//! database; [`rows_query`] reads the forbidden test beside each row.
//!
//! ## Live on a reload
//!
//! `.reload npc_vendor` re-reads `npc_vendor_template` and then `npc_vendor`
//! (`ServerCommands.cpp:1292`), and `LoadVendors` clears each list first, so a
//! removed row is gone from the server. The client asks for the list each time
//! the window is opened, so no relog is needed. A vendor's current count of a
//! limited item is kept on the creature and is not reset by the reload.

use crate::row::{Assignment, Key, Life};

pub use crate::schema::{mask_words, seconds_words, Bit, Column, Group, Kind, Row, RowValue};

pub const VENDOR: &str = "npc_vendor";
pub const TEMPLATE: &str = "npc_vendor_template";

/// Both tables, in the order the server reloads them.
pub const TABLES: [&str; 2] = [TEMPLATE, VENDOR];

/// The static name for a table read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// What an `entry` of each table is, for a window's heading.
pub fn keyed_by(table: &str) -> &'static str {
    match table {
        TEMPLATE => "creature_template.vendor_id",
        _ => "creature_template.entry",
    }
}

/// `VendorItemFlags`, from `CreatureDefines.h:585`.
pub const FLAGS: [Bit; 2] = [
    Bit { bit: 0x1, name: "RANDOM_RESTOCK", about: "the restock delay varies from 80% to 120% of incrtime" },
    Bit { bit: 0x2, name: "DYNAMIC_RESTOCK", about: "the restock delay shortens when more players are online than a Blizzlike realm had" },
];

/// The columns of both tables: `LoadVendors`' `SELECT`, with `slot` after
/// `entry`, where the table has it.
pub const COLUMNS: [Column; 7] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the list: a creature entry, or a vendor_id for the template table" },
    Column { name: "slot", kind: Kind::Unsigned, group: Group::Services, about: "the place in the list the client is sent, lowest first" },
    Column { name: "item", kind: Kind::Key, group: Group::Identity, about: "the item_template entry sold" },
    Column { name: "maxcount", kind: Kind::Unsigned, group: Group::Services, about: "how many the vendor holds, 0 for no limit; up to 255" },
    Column { name: "incrtime", kind: Kind::Seconds, group: Group::Services, about: "seconds before one more is restocked; set exactly when maxcount is" },
    Column { name: "itemflags", kind: Kind::Flags(&FLAGS), group: Group::Services, about: "how the restock delay varies" },
    Column { name: "condition_id", kind: Kind::Unsigned, group: Group::Services, about: "a row of `conditions` the player must meet to see the item, or 0" },
];

/// One column, by name, of either table.
pub fn column(table: &str, name: &str) -> Option<&'static Column> {
    table_named(table)?;
    COLUMNS.iter().find(|column| column.name == name)
}

/// The most `maxcount` holds: the column is a `tinyint unsigned`.
pub const MAX_COUNT: u32 = 255;

/// The number of items a creature's two lists together may reach. The loader
/// skips a row when the lists already hold this many (`countItems >=
/// UINT8_MAX`), so a creature sells at most 254.
pub const MAX_ITEMS: usize = 255;

/// The key of one row.
pub fn key(entry: u32, item: u32) -> Key {
    Key::two(("entry", entry as u64), ("item", item as u64))
}

/// One row of either table, as the server reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Ware {
    pub entry: u32,
    pub slot: u32,
    pub item: u32,
    pub maxcount: u32,
    pub incrtime: u32,
    pub flags: u32,
    pub condition: u32,
}

impl Ware {
    /// A row as a new one is written: in `slot`, with no limit.
    pub fn new(entry: u32, item: u32, slot: u32) -> Ware {
        Ware {
            entry,
            slot,
            item,
            maxcount: 0,
            incrtime: 0,
            flags: 0,
            condition: 0,
        }
    }

    pub fn key(&self) -> Key {
        key(self.entry, self.item)
    }

    /// Every column that is not the key, as SQL literals, which is what a row
    /// this project creates carries — see [`crate::row::Life::Insert`].
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "slot", value: self.slot.to_string() },
            Assignment { column: "maxcount", value: self.maxcount.to_string() },
            Assignment { column: "incrtime", value: self.incrtime.to_string() },
            Assignment { column: "itemflags", value: self.flags.to_string() },
            Assignment { column: "condition_id", value: self.condition.to_string() },
        ]
    }

    /// One row as the database answered it, or `None` for a row missing a key
    /// column.
    pub fn from_row(row: &Row) -> Option<Ware> {
        let number = |column: &str| row.integer(column).unwrap_or(0).max(0) as u32;
        Some(Ware {
            entry: row.integer("entry")? as u32,
            item: row.integer("item")? as u32,
            slot: number("slot"),
            maxcount: number("maxcount"),
            incrtime: number("incrtime"),
            flags: number("itemflags"),
            condition: number("condition_id"),
        })
    }

    /// The stock in words: `unlimited`, or `5, one more every 10m`.
    pub fn stock_words(&self) -> String {
        match self.maxcount {
            0 => "unlimited".to_string(),
            count => format!("{count}, one more every {}", seconds_words(self.incrtime as i64, 0)),
        }
    }

    /// Why the server would skip this row at load, each as a sentence, or
    /// nothing. The checks of `IsVendorItemValid` that need no database; the
    /// item, the condition, a duplicate and the count are checked by the
    /// caller, which has the list.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.maxcount > MAX_COUNT {
            out.push(format!("maxcount {} is over {MAX_COUNT}", self.maxcount));
        }
        match (self.maxcount, self.incrtime) {
            (0, 0) => {}
            (0, _) => out.push("incrtime is set but maxcount is 0: set both or neither".to_string()),
            (_, 0) => out.push("maxcount is set but incrtime is 0: set both or neither".to_string()),
            _ => {}
        }
        out
    }
}

/// The statements a save emits for one row this project claims: an `UPDATE`
/// for a row it edits, a `DELETE`/`INSERT` pair for one it creates, so that
/// applying twice means the same as applying once, and a `DELETE` for one it
/// removes.
pub fn statements(table: &str, key: &Key, life: Life, changes: &[Assignment]) -> Vec<String> {
    match life {
        Life::Update => crate::row::update(table, key, changes).into_iter().collect(),
        Life::Insert => match crate::row::insert(table, key, changes) {
            Some(statement) => vec![crate::row::delete(table, key), statement],
            None => Vec::new(),
        },
        Life::Delete => vec![crate::row::delete(table, key)],
    }
}

// ---------------------------------------------------------------------------
// The queries
// ---------------------------------------------------------------------------

/// The `forbidden_items` test of `LoadVendors`' `SELECT`, for an item column
/// named `item`: true when the server leaves the row out at `wow_patch`.
fn forbidden_test(wow_patch: u32) -> String {
    format!(
        "`item` IN (SELECT `entry` FROM `forbidden_items` WHERE \
         (`after_or_before` = 0 AND `patch` <= {wow_patch}) OR \
         (`after_or_before` = 1 AND `patch` >= {wow_patch}))"
    )
}

/// Every row of one list, in the order the server sends them, each with a
/// `forbidden` column that is 1 when `forbidden_items` leaves it out at
/// `wow_patch`. The rows the server leaves out are read too, so a window can
/// say why an item is not sold.
pub fn rows_query(table: &str, entry: u32, wow_patch: u32) -> String {
    format!(
        "SELECT *, ({}) AS `forbidden` FROM {} WHERE `entry` = {entry} ORDER BY `slot`, `item`",
        forbidden_test(wow_patch),
        crate::sql::name(table)
    )
}

/// The row a key names — what an undo is taken from.
pub fn row_query(table: &str, key: &Key) -> String {
    format!("SELECT * FROM {} WHERE {} LIMIT 1", crate::sql::name(table), key.where_clause())
}

/// Whether a row is already there, which an apply asks before it creates one.
pub fn exists_query(table: &str, key: &Key) -> String {
    format!("SELECT 1 FROM {} WHERE {} LIMIT 1", crate::sql::name(table), key.where_clause())
}

/// The creatures that name a template list, one line per entry: every one of
/// them sells what the list holds.
pub fn users_query(vendor_id: u32) -> String {
    format!(
        "SELECT `entry`, MAX(`name`) AS `name` FROM `creature_template` \
         WHERE `vendor_id` = {vendor_id} GROUP BY `entry` ORDER BY `entry`"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The columns are `LoadVendors`' `SELECT`, with `slot`, which it orders
    /// by, after `entry` where the table's DDL has it.
    #[test]
    fn every_column_is_the_servers_own_in_the_tables_order() {
        let ours: Vec<&str> = COLUMNS.iter().map(|column| column.name).collect();
        assert_eq!(ours, ["entry", "slot", "item", "maxcount", "incrtime", "itemflags", "condition_id"]);
        let keyed: Vec<&str> = COLUMNS.iter().filter(|c| !c.editable()).map(|c| c.name).collect();
        assert_eq!(keyed, ["entry", "item"], "the primary key of both tables");
        assert!(column(TEMPLATE, "slot").is_some());
        assert!(column("npc_trainer", "slot").is_none());
    }

    /// `IsVendorItemValid`'s stock check: both or neither.
    #[test]
    fn a_limit_needs_a_restock_time_and_the_reverse() {
        assert!(Ware::new(54, 2488, 1).check().is_empty());
        let limited = Ware { maxcount: 3, incrtime: 600, ..Ware::new(54, 2488, 1) };
        assert!(limited.check().is_empty());
        assert_eq!(limited.stock_words(), "3, one more every 10m");
        let no_time = Ware { maxcount: 3, ..Ware::new(54, 2488, 1) };
        assert!(no_time.check()[0].contains("incrtime is 0"), "{:?}", no_time.check());
        let no_count = Ware { incrtime: 600, ..Ware::new(54, 2488, 1) };
        assert!(no_count.check()[0].contains("maxcount is 0"), "{:?}", no_count.check());
        let big = Ware { maxcount: 300, incrtime: 1, ..Ware::new(54, 2488, 1) };
        assert!(big.check()[0].contains("over 255"));
    }

    /// A created row is a `DELETE` and an `INSERT` naming every column, an
    /// edit one `UPDATE`, a removal one `DELETE`, each on `(entry, item)`.
    #[test]
    fn a_row_becomes_the_statements_it_means() {
        let ware = Ware::new(54, 2488, 9);
        let created = statements(VENDOR, &ware.key(), Life::Insert, &ware.assignments());
        assert_eq!(
            created,
            vec![
                "DELETE FROM `npc_vendor` WHERE `entry` = 54 AND `item` = 2488;".to_string(),
                "INSERT INTO `npc_vendor` (`entry`, `item`, `slot`, `maxcount`, `incrtime`, \
                 `itemflags`, `condition_id`) VALUES (54, 2488, 9, 0, 0, 0, 0);"
                    .to_string(),
            ]
        );
        let edited = statements(
            TEMPLATE,
            &key(1279501, 5565),
            Life::Update,
            &[Assignment { column: "slot", value: "4".into() }],
        );
        assert_eq!(
            edited,
            vec!["UPDATE `npc_vendor_template` SET `slot` = 4 WHERE `entry` = 1279501 AND `item` = 5565;"]
        );
    }

    /// A row read back carries every column, and the read asks the forbidden
    /// test at the server's patch.
    #[test]
    fn a_row_is_read_as_the_server_reads_it() {
        let mut row = Row::new();
        for (column, value) in [
            ("entry", "66"),
            ("slot", "7"),
            ("item", "2320"),
            ("maxcount", "5"),
            ("incrtime", "3600"),
            ("itemflags", "1"),
            ("condition_id", "0"),
        ] {
            row.insert(column.to_string(), Some(value.to_string()));
        }
        let ware = Ware::from_row(&row).expect("a whole row");
        assert_eq!((ware.entry, ware.slot, ware.item, ware.maxcount), (66, 7, 2320, 5));
        assert_eq!(ware.stock_words(), "5, one more every 1h");
        assert_eq!(mask_words(&FLAGS, ware.flags), "RANDOM_RESTOCK");
        let sql = rows_query(VENDOR, 66, 10);
        assert!(sql.contains("`patch` <= 10"), "{sql}");
        assert!(sql.ends_with("ORDER BY `slot`, `item`"), "{sql}");
    }
}
