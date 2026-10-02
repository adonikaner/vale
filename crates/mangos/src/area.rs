//! `area_template`: the server's copy of `AreaTable.dbc`, and how an edit to a
//! row reaches it.
//!
//! ## What the server reads, and from where
//!
//! vmangos does not open `AreaTable.dbc`. `ObjectMgr::LoadAreaTemplate`
//! (`ObjectMgr.cpp:12050`) loads the SQL table `area_template` through
//! `sAreaStorage` (`SQLStorages.cpp:45`, format `iiiiiisii`), once, at startup
//! (`World.cpp:1409`). There is no `.reload` for it: a change needs a restart.
//!
//! The server uses the row for what the client cannot be trusted with: which
//! zone a position is in, whether a duel may start there, whether a character
//! is resting, the experience for discovering an area, and which bit of the
//! explored-zones mask the discovery sets. `Player::CheckAreaExploreAndOutdoor`
//! refuses an `explore_flag` whose word is past the mask's 64.
//!
//! ## The row
//!
//! ```text
//! entry         mediumint unsigned  AreaTable field 0
//! map_id        mediumint unsigned  field 1
//! zone_id       mediumint unsigned  field 2, the parent
//! explore_flag  mediumint unsigned  field 3
//! flags         mediumint unsigned  field 4
//! area_level    mediumint           field 10, signed
//! name          varchar(100)        field 11, the enUS name
//! team          tinyint unsigned    field 20
//! liquid_type   tinyint unsigned    field 24
//! ```
//!
//! The key is `entry` alone: the table has no build column, so there is one
//! row per area and nothing to copy forward. On the reference install it holds
//! 1,081 rows, one per row of the client's file, with the same values.
//!
//! Fields 5 to 9 (reverb, ambience and music) and the seven other locale
//! names have no column. An edit to one changes the client's file and nothing
//! on the server. The server's own translations are in `locales_area`, which
//! this module does not write.
//!
//! ## The statements
//!
//! As for `crate::skills`: an `INSERT IGNORE` of the whole row, which does
//! nothing for an entry the table already has, then an `UPDATE` of the columns
//! that changed. The undo is a `DELETE` for a row this project created and an
//! `UPDATE` back to the values the row holds now for one that was already
//! there.

use crate::row::{self, Assignment, Key};
use std::collections::HashMap;
use vale_edit::dbc::DbcFile;

/// The table.
pub const TABLE: &str = "area_template";

const ENTRY: &str = "entry";

/// The largest value a `mediumint unsigned` column holds: the entry, the map
/// and the parent. MySQL clamps a larger value instead of refusing it, which
/// would write another row's key.
pub const MAX_MEDIUMINT: u32 = 0x00FF_FFFF;

/// The largest value the two `tinyint unsigned` columns hold.
pub const MAX_TINYINT: u32 = u8::MAX as u32;

/// How many bits the character's explored-zones mask has: 64 words of 32.
/// The server refuses to set a bit at or past this.
pub const EXPLORE_BITS: u32 = 64 * 32;

/// The longest name the column holds, in bytes.
pub const MAX_NAME: usize = 100;

/// How a column's value is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Unsigned,
    /// The field's four bytes as an `i32`.
    Signed,
    /// The field is an offset into the string block.
    Text,
}

/// The columns the client's file feeds, in table order, each with the
/// `AreaTable.dbc` field it takes its value from.
const COLUMNS: [(&str, usize, Shape); 8] = [
    ("map_id", 1, Shape::Unsigned),
    ("zone_id", 2, Shape::Unsigned),
    ("explore_flag", 3, Shape::Unsigned),
    ("flags", 4, Shape::Unsigned),
    ("area_level", 10, Shape::Signed),
    ("name", 11, Shape::Text),
    ("team", 20, Shape::Unsigned),
    ("liquid_type", 24, Shape::Unsigned),
];

/// The `AreaTable.dbc` fields the server has a column for, the id included.
/// An edit to any other field does not reach the server.
pub fn mirrored_fields() -> impl Iterator<Item = usize> {
    std::iter::once(0).chain(COLUMNS.iter().map(|&(_, field, _)| field))
}

/// Why the server's table cannot hold the row with this id as the file has
/// it, or `None` when it can. A row the file does not have cannot be held.
pub fn misfit(dbc: &DbcFile, id: u32) -> Option<String> {
    let Some(record) = dbc.row_of(id) else {
        return Some(format!("AreaTable has no row {id}"));
    };
    let field = |at: usize| dbc.u32_at(record, at).unwrap_or(0);
    if !(1..=MAX_MEDIUMINT).contains(&id) {
        return Some(format!("an entry must be between 1 and {MAX_MEDIUMINT}"));
    }
    if field(1) > MAX_MEDIUMINT || field(2) > MAX_MEDIUMINT {
        return Some(format!("its map and its parent must each be at most {MAX_MEDIUMINT}"));
    }
    if field(3) >= EXPLORE_BITS {
        return Some(format!(
            "its explore bit is {}, and the explored-zones mask has {EXPLORE_BITS} bits",
            field(3)
        ));
    }
    if field(4) > MAX_MEDIUMINT {
        return Some(format!("its flags must be at most {MAX_MEDIUMINT}"));
    }
    if field(20) > MAX_TINYINT || field(24) > MAX_TINYINT {
        return Some(format!("its team and its liquid type must each be at most {MAX_TINYINT}"));
    }
    let name = dbc.string_at(record, 11).unwrap_or_default();
    if name.len() > MAX_NAME {
        return Some(format!("its name is {} bytes, and the column holds {MAX_NAME}", name.len()));
    }
    None
}

/// Whether the server's table can hold the row; see [`misfit`].
pub fn fits(dbc: &DbcFile, id: u32) -> bool {
    misfit(dbc, id).is_none()
}

/// The row's key.
pub fn key(id: u32) -> Key {
    Key::one(ENTRY, u64::from(id))
}

/// Each column's value on one record of the file, as a SQL literal.
fn values(dbc: &DbcFile, record: usize) -> Vec<(&'static str, String)> {
    COLUMNS
        .iter()
        .map(|&(column, field, shape)| {
            let raw = dbc.u32_at(record, field).unwrap_or(0);
            let literal = match shape {
                Shape::Unsigned => raw.to_string(),
                Shape::Signed => (raw as i32).to_string(),
                Shape::Text => crate::sql::text(&dbc.string_at(record, field).unwrap_or_default()),
            };
            (column, literal)
        })
        .collect()
}

/// What changed between the row the archives ship and the row this project
/// edited: one assignment per column whose value differs. A row the shipped
/// file does not have is new, and every column is a change. Empty for an id
/// the edited file does not have.
pub fn changes(shipped: &DbcFile, edited: &DbcFile, id: u32) -> Vec<Assignment> {
    let Some(record) = edited.row_of(id) else {
        return Vec::new();
    };
    let before = shipped.row_of(id).map(|record| values(shipped, record));
    values(edited, record)
        .into_iter()
        .enumerate()
        .filter(|(n, (_, after))| before.as_ref().map(|b| &b[*n].1) != Some(after))
        .map(|(_, (column, value))| Assignment { column, value })
        .collect()
}

/// The whole row, from the edited file. `INSERT IGNORE`, so it does nothing
/// for an entry the table already holds.
pub fn insert_new(edited: &DbcFile, id: u32) -> Option<String> {
    let record = edited.row_of(id)?;
    let mut names = vec![crate::sql::name(ENTRY)];
    let mut literals = vec![id.to_string()];
    for (column, value) in values(edited, record) {
        names.push(crate::sql::name(column));
        literals.push(value);
    }
    Some(format!(
        "INSERT IGNORE INTO {} ({}) VALUES ({});",
        crate::sql::name(TABLE),
        names.join(", "),
        literals.join(", ")
    ))
}

/// Everything a save emits for one edited row, in order. Empty when no
/// column the server reads changed or the row cannot be stored.
pub fn statements(shipped: &DbcFile, edited: &DbcFile, id: u32) -> Vec<String> {
    if !fits(edited, id) {
        return Vec::new();
    }
    let changes = changes(shipped, edited, id);
    let (Some(insert), Some(update)) = (insert_new(edited, id), row::update(TABLE, &key(id), &changes))
    else {
        return Vec::new();
    };
    vec![insert, update]
}

/// The row of one entry, for [`undo`] to read before an apply.
pub fn dev_row_query(id: u32) -> String {
    format!(
        "SELECT * FROM {} WHERE {};",
        crate::sql::name(TABLE),
        key(id).where_clause()
    )
}

/// The statement that puts one row back to what the table holds now: a
/// `DELETE` when it holds no row for the entry, which is a row this project
/// is about to create, and an `UPDATE` of the changed columns when it holds
/// one.
pub fn undo(
    id: u32,
    changes: &[Assignment],
    now: Option<&HashMap<String, Option<String>>>,
) -> Option<String> {
    if changes.is_empty() {
        return None;
    }
    match now {
        None => Some(row::delete(TABLE, &key(id))),
        Some(now) => row::undo(TABLE, &key(id), changes, Some(now)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `AreaTable.dbc` of 25 fields holding the rows given, each as
    /// `(id, map, parent, explore bit, name)` with the duel flag set.
    fn table(rows: &[(u32, u32, u32, u32, &str)]) -> DbcFile {
        let fields = 25u32;
        let mut strings = vec![0u8];
        let mut records = Vec::new();
        for &(id, map, parent, bit, name) in rows {
            let mut record = [0u32; 25];
            record[0] = id;
            record[1] = map;
            record[2] = parent;
            record[3] = bit;
            record[4] = 0x40;
            record[11] = strings.len() as u32;
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
            records.push(record);
        }
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"WDBC");
        for word in [rows.len() as u32, fields, fields * 4, strings.len() as u32] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        for record in records {
            for word in record {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&strings);
        DbcFile::parse(&bytes).expect("a table")
    }

    #[test]
    fn an_unchanged_row_writes_nothing() {
        let shipped = table(&[(12, 0, 0, 126, "Elwynn Forest")]);
        assert!(changes(&shipped, &shipped, 12).is_empty());
        assert!(statements(&shipped, &shipped, 12).is_empty());
    }

    #[test]
    fn a_changed_level_is_one_signed_column() {
        let shipped = table(&[(12, 0, 0, 126, "Elwynn Forest")]);
        let mut edited = table(&[(12, 0, 0, 126, "Elwynn Forest")]);
        edited.set_u32(0, 10, (-1i32) as u32);
        let sql = statements(&shipped, &edited, 12);
        assert_eq!(sql.len(), 2);
        assert_eq!(
            sql[1],
            "UPDATE `area_template` SET `area_level` = -1 WHERE `entry` = 12;"
        );
    }

    /// A field the server has no column for changes the file and writes no
    /// statement.
    #[test]
    fn a_field_with_no_column_writes_nothing() {
        let shipped = table(&[(12, 0, 0, 126, "Elwynn Forest")]);
        let mut edited = table(&[(12, 0, 0, 126, "Elwynn Forest")]);
        edited.set_u32(0, 8, 42);
        edited.set_u32(0, 7, 35);
        assert!(statements(&shipped, &edited, 12).is_empty());
        assert!(!mirrored_fields().any(|field| field == 8));
        assert!(mirrored_fields().any(|field| field == 11));
    }

    #[test]
    fn a_new_area_names_every_column_and_quotes_its_name() {
        let shipped = table(&[(12, 0, 0, 126, "Elwynn Forest")]);
        let edited = table(&[
            (12, 0, 0, 126, "Elwynn Forest"),
            (3487, 0, 12, 1077, "Miller's Rest"),
        ]);
        assert_eq!(changes(&shipped, &edited, 3487).len(), COLUMNS.len());
        let sql = statements(&shipped, &edited, 3487);
        assert_eq!(
            sql[0],
            "INSERT IGNORE INTO `area_template` (`entry`, `map_id`, `zone_id`, `explore_flag`, \
             `flags`, `area_level`, `name`, `team`, `liquid_type`) VALUES \
             (3487, 0, 12, 1077, 64, 0, 'Miller\\'s Rest', 0, 0);"
        );
    }

    /// A row the table cannot hold is refused before a statement exists, with
    /// the reason.
    #[test]
    fn a_row_the_table_cannot_hold_is_refused_with_the_reason() {
        let shipped = table(&[]);
        let far_bit = table(&[(3487, 0, 12, EXPLORE_BITS, "Past the mask")]);
        assert!(misfit(&far_bit, 3487).is_some_and(|why| why.contains("explore bit")));
        assert!(statements(&shipped, &far_bit, 3487).is_empty());
        let long = "x".repeat(MAX_NAME + 1);
        let long_name = table(&[(3487, 0, 12, 1077, &long)]);
        assert!(misfit(&long_name, 3487).is_some_and(|why| why.contains("name")));
        assert!(fits(&table(&[(3487, 0, 12, EXPLORE_BITS - 1, "Last bit")]), 3487));
        assert!(!fits(&shipped, 3487), "a row the file does not have");
    }

    #[test]
    fn the_undo_deletes_a_row_this_project_made_and_restores_one_it_did_not() {
        let shipped = table(&[(12, 0, 0, 126, "Elwynn Forest")]);
        let mut edited = table(&[(12, 0, 0, 126, "Elwynn Forest")]);
        edited.set_u32(0, 20, 2);
        let changes = changes(&shipped, &edited, 12);
        assert_eq!(
            undo(12, &changes, None).unwrap(),
            "DELETE FROM `area_template` WHERE `entry` = 12;"
        );
        let now = HashMap::from([("team".to_string(), Some("0".to_string()))]);
        assert_eq!(
            undo(12, &changes, Some(&now)).unwrap(),
            "UPDATE `area_template` SET `team` = 0 WHERE `entry` = 12;"
        );
        assert!(undo(12, &[], None).is_none());
    }
}
