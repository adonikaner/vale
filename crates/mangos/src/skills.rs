//! `skill_line_ability`: the server's copy of `SkillLineAbility.dbc`, and how
//! an edit to a row reaches it.
//!
//! ## What the server reads, and from where
//!
//! vmangos reads the skill tables from two places:
//!
//! ```text
//! SkillLineAbility    the SQL table skill_line_ability, one row per
//!                     (id, build); ObjectMgr::LoadSkillLineAbility
//!                     (ObjectMgr.cpp:9767) reads the rows whose build is
//!                     5875 and no others
//! SkillLine           DataDir\5875\dbc\SkillLine.dbc
//! SkillRaceClassInfo  DataDir\5875\dbc\SkillRaceClassInfo.dbc
//! SkillTiers          DataDir\5875\dbc\SkillTiers.dbc
//! ```
//!
//! So an edited ability reaches the server as a row of `skill_line_ability`,
//! written here, and an edited skill line reaches it as the copied file. The
//! table is read once, at startup (`World.cpp:1381`), and
//! `SpellMgr::LoadSkillLineAbilityMaps` builds its lookups from it after
//! that. There is no `.reload` for it: a change needs a restart.
//!
//! ## The row
//!
//! ```text
//! id                   smallint unsigned  SkillLineAbility field 0
//! build                smallint unsigned  5875; see BUILD
//! skill_id             int unsigned       field 1
//! spell_id             smallint unsigned  field 2
//! race_mask            int unsigned       field 3
//! class_mask           int unsigned       field 4
//! req_skill_value      int unsigned       field 7
//! superseded_by_spell  smallint unsigned  field 8
//! learn_on_get_skill   int unsigned       field 9
//! max_value            int unsigned       field 10
//! min_value            int unsigned       field 11
//! req_train_points     int unsigned       field 14
//! ```
//!
//! Fields 5, 6, 12 and 13 have no column. They are zero on all 5,072 shipped
//! rows, and an edit to one changes the client's file and nothing on the
//! server.
//!
//! ## The statements
//!
//! The loader takes the 5875 rows and nothing else, so there is no lower
//! build to fall back to and no row to copy forward, which is where this
//! differs from `crate::spell` and `crate::taxi`. On the reference install the
//! table holds 5,072 rows at build 5875, one per row of the client's file. An
//! edit is two statements: an `INSERT IGNORE` of the whole row, which does
//! nothing for an id the table already has at 5875, and an `UPDATE` of the
//! columns that changed. The undo is a `DELETE` for a row this project
//! created and an `UPDATE` back to the values the row holds now for one that
//! was already there.

use crate::row::{self, Assignment, Key};
use std::collections::HashMap;
use vale_edit::dbc::DbcFile;

/// The table.
pub const TABLE: &str = "skill_line_ability";

/// The build an edit is written at, and the only one the loader reads.
pub const BUILD: u32 = 5875;

const ID: &str = "id";
const BUILD_COLUMN: &str = "build";

/// The largest id the table holds, and the largest spell either of its spell
/// columns holds: all three are `smallint unsigned`. MySQL clamps a larger
/// value instead of refusing it, which would write another row's key.
pub const MAX_SMALLINT: u32 = u16::MAX as u32;

/// The columns the client's file feeds, in table order, each with the
/// `SkillLineAbility.dbc` field it takes its value from.
pub const COLUMNS: [(&str, usize); 10] = [
    ("skill_id", 1),
    ("spell_id", 2),
    ("race_mask", 3),
    ("class_mask", 4),
    ("req_skill_value", 7),
    ("superseded_by_spell", 8),
    ("learn_on_get_skill", 9),
    ("max_value", 10),
    ("min_value", 11),
    ("req_train_points", 14),
];

/// The two spell columns, which are `smallint unsigned` where the file's
/// fields are 32 bits.
const SPELL_FIELDS: [usize; 2] = [2, 8];

/// Whether the server's table can hold the row with this id as the file has
/// it: the id and both spells fit a `smallint unsigned`. A row the file does
/// not have does not fit.
pub fn fits(dbc: &DbcFile, id: u32) -> bool {
    let Some(record) = dbc.row_of(id) else {
        return false;
    };
    (1..=MAX_SMALLINT).contains(&id)
        && SPELL_FIELDS
            .iter()
            .all(|&field| dbc.u32_at(record, field).is_some_and(|spell| spell <= MAX_SMALLINT))
}

/// The row's key.
pub fn key(id: u32) -> Key {
    Key::two((ID, u64::from(id)), (BUILD_COLUMN, u64::from(BUILD)))
}

/// Each column's value on one record of the file.
fn values(dbc: &DbcFile, record: usize) -> Vec<(&'static str, String)> {
    COLUMNS
        .iter()
        .map(|&(column, field)| (column, dbc.u32_at(record, field).unwrap_or(0).to_string()))
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

/// The whole row at build 5875, from the edited file. `INSERT IGNORE`, so it
/// does nothing for an id the table already holds at that build.
pub fn insert_new(edited: &DbcFile, id: u32) -> Option<String> {
    let record = edited.row_of(id)?;
    let mut names = vec![crate::sql::name(ID), crate::sql::name(BUILD_COLUMN)];
    let mut literals = vec![id.to_string(), BUILD.to_string()];
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

/// The 5875 row of one id, for [`undo`] to read before an apply.
pub fn dev_row_query(id: u32) -> String {
    format!(
        "SELECT * FROM {} WHERE {};",
        crate::sql::name(TABLE),
        key(id).where_clause()
    )
}

/// The statement that puts one row back to what the table holds now: a
/// `DELETE` when it holds no 5875 row for the id, which is a row this
/// project is about to create, and an `UPDATE` of the changed columns when it
/// holds one.
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

    /// A `SkillLineAbility.dbc` of 15 fields holding the rows given, each as
    /// `(id, skill, spell)` with a required skill value of 1.
    fn table(rows: &[(u32, u32, u32)]) -> DbcFile {
        let fields = 15u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"WDBC");
        for word in [rows.len() as u32, fields, fields * 4, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        for &(id, skill, spell) in rows {
            let mut record = [0u32; 15];
            record[0] = id;
            record[1] = skill;
            record[2] = spell;
            record[7] = 1;
            for word in record {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        }
        bytes.push(0);
        DbcFile::parse(&bytes).expect("a table")
    }

    #[test]
    fn an_unchanged_row_writes_nothing() {
        let shipped = table(&[(69, 6, 116)]);
        assert!(changes(&shipped, &shipped, 69).is_empty());
        assert!(statements(&shipped, &shipped, 69).is_empty());
    }

    #[test]
    fn a_changed_class_mask_is_one_column_of_the_5875_row() {
        let shipped = table(&[(69, 6, 116)]);
        let mut edited = table(&[(69, 6, 116)]);
        edited.set_u32(0, 4, 128);
        let changed: Vec<&str> = changes(&shipped, &edited, 69)
            .iter()
            .map(|change| change.column)
            .collect();
        assert_eq!(changed, ["class_mask"]);
        let sql = statements(&shipped, &edited, 69);
        assert_eq!(sql.len(), 2);
        assert_eq!(
            sql[0],
            "INSERT IGNORE INTO `skill_line_ability` (`id`, `build`, `skill_id`, `spell_id`, \
             `race_mask`, `class_mask`, `req_skill_value`, `superseded_by_spell`, \
             `learn_on_get_skill`, `max_value`, `min_value`, `req_train_points`) VALUES \
             (69, 5875, 6, 116, 0, 128, 1, 0, 0, 0, 0, 0);"
        );
        assert_eq!(
            sql[1],
            "UPDATE `skill_line_ability` SET `class_mask` = 128 WHERE `id` = 69 AND `build` = 5875;"
        );
    }

    /// A field the server has no column for changes the file and writes no
    /// statement.
    #[test]
    fn a_field_with_no_column_writes_nothing() {
        let shipped = table(&[(69, 6, 116)]);
        let mut edited = table(&[(69, 6, 116)]);
        edited.set_u32(0, 5, 2);
        edited.set_u32(0, 12, 7);
        assert!(statements(&shipped, &edited, 69).is_empty());
    }

    #[test]
    fn a_new_row_names_every_column() {
        let shipped = table(&[(69, 6, 116)]);
        let edited = table(&[(69, 6, 116), (15031, 6, 133)]);
        assert_eq!(changes(&shipped, &edited, 15031).len(), COLUMNS.len());
        let sql = statements(&shipped, &edited, 15031);
        assert_eq!(sql.len(), 2);
        for (column, _) in COLUMNS {
            assert!(sql[0].contains(&format!("`{column}`")), "{column} missing: {}", sql[0]);
        }
    }

    /// An id or a spell past a `smallint` is refused before a statement
    /// exists, since MySQL would clamp it onto another row.
    #[test]
    fn a_row_past_a_smallint_is_not_written() {
        let shipped = table(&[]);
        let big_id = table(&[(70_000, 6, 116)]);
        assert!(!fits(&big_id, 70_000));
        assert!(statements(&shipped, &big_id, 70_000).is_empty());
        let big_spell = table(&[(69, 6, 90_210)]);
        assert!(!fits(&big_spell, 69));
        assert!(fits(&table(&[(69, 6, 65_535)]), 69));
        assert!(!fits(&shipped, 69), "a row the file does not have");
    }

    #[test]
    fn the_undo_deletes_a_row_this_project_made_and_restores_one_it_did_not() {
        let shipped = table(&[(69, 6, 116)]);
        let mut edited = table(&[(69, 6, 116)]);
        edited.set_u32(0, 4, 128);
        let changes = changes(&shipped, &edited, 69);
        assert_eq!(
            undo(69, &changes, None).unwrap(),
            "DELETE FROM `skill_line_ability` WHERE `id` = 69 AND `build` = 5875;"
        );
        let now = HashMap::from([("class_mask".to_string(), Some("0".to_string()))]);
        assert_eq!(
            undo(69, &changes, Some(&now)).unwrap(),
            "UPDATE `skill_line_ability` SET `class_mask` = 0 WHERE `id` = 69 AND `build` = 5875;"
        );
        assert!(undo(69, &[], None).is_none());
    }
}
