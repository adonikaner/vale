//! `faction` and `faction_template`: the server's copies of `Faction.dbc`
//! and `FactionTemplate.dbc`, and how an edit to a row of either reaches
//! them.
//!
//! ## What the server reads, and from where
//!
//! vmangos does not open either file. `ObjectMgr::LoadFactions`
//! (`ObjectMgr.cpp:8670`) reads the SQL tables `faction` and
//! `faction_template`, each keyed by `(id, build)`, taking for each id the
//! row with the highest build at or below 5875. It runs once, at startup
//! (`World.cpp:1369`), and there is no `.reload` for either table: a change
//! needs a restart. `locales_faction` holds the other locales' names and is
//! not written here.
//!
//! A `faction` row is what a character's reputation is measured against:
//! which of its four race-and-class alternatives fits the character, the
//! standing that alternative starts at, the flags the reputation panel
//! starts with, and the heading the faction is listed under. A
//! `faction_template` row is how two units stand to each other: the groups
//! the template is in, is friendly to and is hostile to, the factions it
//! names as enemies and friends, and the flags that make a creature call for
//! help or attack a player who has been fighting.
//!
//! ## The rows
//!
//! ```text
//! faction
//! id                    smallint unsigned   Faction field 0
//! build                 smallint unsigned   5875; see BUILD
//! reputation_list_id    mediumint           field 1, signed: -1 for no slot
//! base_rep_race_mask1-4 mediumint unsigned  fields 2 to 5
//! base_rep_class_mask1-4 mediumint unsigned fields 6 to 9
//! base_rep_value1-4     mediumint           fields 10 to 13, signed
//! reputation_flags1-4   mediumint unsigned  fields 14 to 17
//! team                  mediumint unsigned  field 18, the parent faction
//! name                  varchar(256)        field 19, the enUS name
//! description           varchar(512)        field 28, the enUS description
//!
//! faction_template
//! id                    smallint unsigned   FactionTemplate field 0
//! build                 smallint unsigned   5875
//! faction_id            mediumint unsigned  field 1
//! faction_flags         mediumint unsigned  field 2
//! our_mask              mediumint unsigned  field 3
//! friendly_mask         mediumint unsigned  field 4
//! hostile_mask          mediumint unsigned  field 5
//! enemy_faction1-4      mediumint unsigned  fields 6 to 9
//! friend_faction1-4     mediumint unsigned  fields 10 to 13
//! ```
//!
//! The seven other locale names and descriptions and the two string flag
//! words of `Faction.dbc` have no column. An edit to one changes the client's
//! file and nothing on the server.
//!
//! ## The statements
//!
//! As for `crate::skills`: an `INSERT IGNORE` of the whole row at build 5875,
//! which does nothing for an id the table already holds at that build, then
//! an `UPDATE` of the columns that changed. A row at a lower build stays
//! where it is and stops being read, since the loader takes the highest build.
//! The undo is a `DELETE` for a 5875 row this project created and an `UPDATE`
//! back to the values the row holds now for one that was already there.

use crate::row::{self, Assignment, Key};
use std::collections::HashMap;
use vale_edit::dbc::DbcFile;

/// The server's copy of `Faction.dbc`.
pub const TABLE: &str = "faction";

/// The server's copy of `FactionTemplate.dbc`.
pub const TEMPLATE: &str = "faction_template";

/// The build an edit is written at. The loader takes the highest build at or
/// below it, so a row written here wins over every shipped one.
pub const BUILD: u32 = crate::CLIENT_BUILD;

const ID: &str = "id";
const BUILD_COLUMN: &str = "build";

/// The largest id either table holds: `id` is a `smallint unsigned`. MySQL
/// clamps a larger value instead of refusing it, which would write another
/// row's key.
pub const MAX_ID: u32 = u16::MAX as u32;

/// The largest value a `mediumint unsigned` column holds: every mask, flag
/// and faction column of both tables.
pub const MAX_MEDIUMINT: u32 = 0x00FF_FFFF;

/// The range a signed `mediumint` column holds: the reputation slot and the
/// four base standings.
pub const MEDIUMINT_RANGE: std::ops::RangeInclusive<i32> = -0x0080_0000..=0x007F_FFFF;

/// The longest name the `name` column holds, in bytes.
pub const MAX_NAME: usize = 256;

/// The longest description the `description` column holds, in bytes.
pub const MAX_DESCRIPTION: usize = 512;

/// How a column's value is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// The field's four bytes as a `u32`, at most [`MAX_MEDIUMINT`].
    Unsigned,
    /// The field's four bytes as an `i32`, within [`MEDIUMINT_RANGE`].
    Signed,
    /// The field is an offset into the string block; the column holds at
    /// most this many bytes.
    Text(usize),
}

/// One server table and the client file it mirrors: which column each field
/// feeds, and how.
#[derive(Debug)]
pub struct Mirror {
    /// The server table.
    pub table: &'static str,
    /// The client table, by bare name.
    pub dbc: &'static str,
    /// The columns the client's file feeds, in table order, each with the
    /// field it takes its value from.
    columns: &'static [(&'static str, usize, Shape)],
}

/// `faction`, fed by `Faction.dbc`.
pub const FACTION: Mirror = Mirror {
    table: TABLE,
    dbc: "Faction",
    columns: &[
        ("reputation_list_id", 1, Shape::Signed),
        ("base_rep_race_mask1", 2, Shape::Unsigned),
        ("base_rep_race_mask2", 3, Shape::Unsigned),
        ("base_rep_race_mask3", 4, Shape::Unsigned),
        ("base_rep_race_mask4", 5, Shape::Unsigned),
        ("base_rep_class_mask1", 6, Shape::Unsigned),
        ("base_rep_class_mask2", 7, Shape::Unsigned),
        ("base_rep_class_mask3", 8, Shape::Unsigned),
        ("base_rep_class_mask4", 9, Shape::Unsigned),
        ("base_rep_value1", 10, Shape::Signed),
        ("base_rep_value2", 11, Shape::Signed),
        ("base_rep_value3", 12, Shape::Signed),
        ("base_rep_value4", 13, Shape::Signed),
        ("reputation_flags1", 14, Shape::Unsigned),
        ("reputation_flags2", 15, Shape::Unsigned),
        ("reputation_flags3", 16, Shape::Unsigned),
        ("reputation_flags4", 17, Shape::Unsigned),
        ("team", 18, Shape::Unsigned),
        ("name", 19, Shape::Text(MAX_NAME)),
        ("description", 28, Shape::Text(MAX_DESCRIPTION)),
    ],
};

/// `faction_template`, fed by `FactionTemplate.dbc`.
pub const FACTION_TEMPLATE: Mirror = Mirror {
    table: TEMPLATE,
    dbc: "FactionTemplate",
    columns: &[
        ("faction_id", 1, Shape::Unsigned),
        ("faction_flags", 2, Shape::Unsigned),
        ("our_mask", 3, Shape::Unsigned),
        ("friendly_mask", 4, Shape::Unsigned),
        ("hostile_mask", 5, Shape::Unsigned),
        ("enemy_faction1", 6, Shape::Unsigned),
        ("enemy_faction2", 7, Shape::Unsigned),
        ("enemy_faction3", 8, Shape::Unsigned),
        ("enemy_faction4", 9, Shape::Unsigned),
        ("friend_faction1", 10, Shape::Unsigned),
        ("friend_faction2", 11, Shape::Unsigned),
        ("friend_faction3", 12, Shape::Unsigned),
        ("friend_faction4", 13, Shape::Unsigned),
    ],
};

/// The two mirrors.
pub const MIRRORS: [&Mirror; 2] = [&FACTION, &FACTION_TEMPLATE];

/// The mirror whose server table is `table`, or `None`.
pub fn for_table(table: &str) -> Option<&'static Mirror> {
    MIRRORS.into_iter().find(|mirror| mirror.table == table)
}

/// The mirror whose client table is `dbc`, or `None`.
pub fn for_dbc(dbc: &str) -> Option<&'static Mirror> {
    MIRRORS.into_iter().find(|mirror| mirror.dbc == dbc)
}

/// The row's key, the same for both tables.
pub fn key(id: u32) -> Key {
    Key::two((ID, u64::from(id)), (BUILD_COLUMN, u64::from(BUILD)))
}

impl Mirror {
    /// The fields the server has a column for, the id included. An edit to
    /// any other field does not reach the server.
    #[cfg(test)]
    pub(crate) fn mirrored_fields(&self) -> impl Iterator<Item = usize> + '_ {
        std::iter::once(0).chain(self.columns.iter().map(|&(_, field, _)| field))
    }

    /// Why the server's table cannot hold the row with this id as the file
    /// has it, or `None` when it can. A row the file does not have cannot be
    /// held.
    pub fn misfit(&self, dbc: &DbcFile, id: u32) -> Option<String> {
        let Some(record) = dbc.row_of(id) else {
            return Some(format!("{} has no row {id}", self.dbc));
        };
        if !(1..=MAX_ID).contains(&id) {
            return Some(format!("an id must be between 1 and {MAX_ID}"));
        }
        for &(column, field, shape) in self.columns {
            let raw = dbc.u32_at(record, field).unwrap_or(0);
            match shape {
                Shape::Unsigned if raw > MAX_MEDIUMINT => {
                    return Some(format!("its {column} must be at most {MAX_MEDIUMINT}"));
                }
                Shape::Signed if !MEDIUMINT_RANGE.contains(&(raw as i32)) => {
                    return Some(format!(
                        "its {column} must be between {} and {}",
                        MEDIUMINT_RANGE.start(),
                        MEDIUMINT_RANGE.end()
                    ));
                }
                Shape::Text(most) => {
                    let text = dbc.string_at(record, field).unwrap_or_default();
                    if text.len() > most {
                        return Some(format!(
                            "its {column} is {} bytes, and the column holds {most}",
                            text.len()
                        ));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Whether the server's table can hold the row; see [`Mirror::misfit`].
    pub fn fits(&self, dbc: &DbcFile, id: u32) -> bool {
        self.misfit(dbc, id).is_none()
    }

    /// Each column's value on one record of the file, as a SQL literal.
    fn values(&self, dbc: &DbcFile, record: usize) -> Vec<(&'static str, String)> {
        self.columns
            .iter()
            .map(|&(column, field, shape)| {
                let raw = dbc.u32_at(record, field).unwrap_or(0);
                let literal = match shape {
                    Shape::Unsigned => raw.to_string(),
                    Shape::Signed => (raw as i32).to_string(),
                    Shape::Text(_) => crate::sql::text(&dbc.string_at(record, field).unwrap_or_default()),
                };
                (column, literal)
            })
            .collect()
    }

    /// What changed between the row the archives ship and the row this
    /// project edited: one assignment per column whose value differs. A row
    /// the shipped file does not have is new, and every column is a change.
    /// Empty for an id the edited file does not have.
    pub fn changes(&self, shipped: &DbcFile, edited: &DbcFile, id: u32) -> Vec<Assignment> {
        let Some(record) = edited.row_of(id) else {
            return Vec::new();
        };
        let before = shipped.row_of(id).map(|record| self.values(shipped, record));
        self.values(edited, record)
            .into_iter()
            .enumerate()
            .filter(|(n, (_, after))| before.as_ref().map(|b| &b[*n].1) != Some(after))
            .map(|(_, (column, value))| Assignment { column, value })
            .collect()
    }

    /// The whole row at build 5875, from the edited file. `INSERT IGNORE`,
    /// so it does nothing for an id the table already holds at that build.
    pub fn insert_new(&self, edited: &DbcFile, id: u32) -> Option<String> {
        let record = edited.row_of(id)?;
        let mut names = vec![crate::sql::name(ID), crate::sql::name(BUILD_COLUMN)];
        let mut literals = vec![id.to_string(), BUILD.to_string()];
        for (column, value) in self.values(edited, record) {
            names.push(crate::sql::name(column));
            literals.push(value);
        }
        Some(format!(
            "INSERT IGNORE INTO {} ({}) VALUES ({});",
            crate::sql::name(self.table),
            names.join(", "),
            literals.join(", ")
        ))
    }

    /// Everything a save emits for one edited row, in order. Empty when no
    /// column the server reads changed or the row cannot be stored.
    pub fn statements(&self, shipped: &DbcFile, edited: &DbcFile, id: u32) -> Vec<String> {
        if !self.fits(edited, id) {
            return Vec::new();
        }
        let changes = self.changes(shipped, edited, id);
        let (Some(insert), Some(update)) =
            (self.insert_new(edited, id), row::update(self.table, &key(id), &changes))
        else {
            return Vec::new();
        };
        vec![insert, update]
    }

    /// The 5875 row of one id, for [`Mirror::undo`] to read before an apply.
    pub fn dev_row_query(&self, id: u32) -> String {
        format!(
            "SELECT * FROM {} WHERE {};",
            crate::sql::name(self.table),
            key(id).where_clause()
        )
    }

    /// The statement that puts one row back to what the table holds now: a
    /// `DELETE` when it holds no 5875 row for the id, which is a row this
    /// project is about to create, and an `UPDATE` of the changed columns
    /// when it holds one.
    pub fn undo(
        &self,
        id: u32,
        changes: &[Assignment],
        now: Option<&HashMap<String, Option<String>>>,
    ) -> Option<String> {
        if changes.is_empty() {
            return None;
        }
        match now {
            None => Some(row::delete(self.table, &key(id))),
            Some(now) => row::undo(self.table, &key(id), changes, Some(now)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of `fields` fields holding the rows given, each as its id,
    /// `(field, value)` pairs and `(field, text)` pairs.
    fn table(fields: u32, rows: &[(u32, &[(usize, u32)], &[(usize, &str)])]) -> DbcFile {
        let mut strings = vec![0u8];
        let mut records = Vec::new();
        for &(id, numbers, texts) in rows {
            let mut record = vec![0u32; fields as usize];
            record[0] = id;
            for &(field, value) in numbers {
                record[field] = value;
            }
            for &(field, text) in texts {
                record[field] = strings.len() as u32;
                strings.extend_from_slice(text.as_bytes());
                strings.push(0);
            }
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

    /// A `Faction.dbc` of 37 fields: Ironforge, slot 5, every race, base 0,
    /// under the Alliance heading.
    fn factions(rows: &[(u32, &[(usize, u32)], &[(usize, &str)])]) -> DbcFile {
        table(37, rows)
    }

    const IRONFORGE: (u32, &[(usize, u32)], &[(usize, &str)]) =
        (47, &[(1, 5), (2, 255), (14, 1), (18, 469)], &[(19, "Ironforge")]);

    #[test]
    fn an_unchanged_row_writes_nothing() {
        let shipped = factions(&[IRONFORGE]);
        assert!(FACTION.changes(&shipped, &shipped, 47).is_empty());
        assert!(FACTION.statements(&shipped, &shipped, 47).is_empty());
    }

    #[test]
    fn a_changed_base_is_one_signed_column_of_the_5875_row() {
        let shipped = factions(&[IRONFORGE]);
        let mut edited = factions(&[IRONFORGE]);
        edited.set_u32(0, 10, (-3000i32) as u32);
        let changed: Vec<&str> = FACTION
            .changes(&shipped, &edited, 47)
            .iter()
            .map(|change| change.column)
            .collect();
        assert_eq!(changed, ["base_rep_value1"]);
        let sql = FACTION.statements(&shipped, &edited, 47);
        assert_eq!(sql.len(), 2);
        assert_eq!(
            sql[0],
            "INSERT IGNORE INTO `faction` (`id`, `build`, `reputation_list_id`, \
             `base_rep_race_mask1`, `base_rep_race_mask2`, `base_rep_race_mask3`, \
             `base_rep_race_mask4`, `base_rep_class_mask1`, `base_rep_class_mask2`, \
             `base_rep_class_mask3`, `base_rep_class_mask4`, `base_rep_value1`, \
             `base_rep_value2`, `base_rep_value3`, `base_rep_value4`, `reputation_flags1`, \
             `reputation_flags2`, `reputation_flags3`, `reputation_flags4`, `team`, `name`, \
             `description`) VALUES (47, 5875, 5, 255, 0, 0, 0, 0, 0, 0, 0, -3000, 0, 0, 0, 1, \
             0, 0, 0, 469, 'Ironforge', '');"
        );
        assert_eq!(
            sql[1],
            "UPDATE `faction` SET `base_rep_value1` = -3000 WHERE `id` = 47 AND `build` = 5875;"
        );
    }

    /// A field the server has no column for, such as a locale name, changes
    /// the file and writes no statement.
    #[test]
    fn a_field_with_no_column_writes_nothing() {
        let shipped = factions(&[IRONFORGE]);
        let mut edited = factions(&[IRONFORGE]);
        edited.set_u32(0, 27, 0x00ff_ffff);
        assert!(FACTION.statements(&shipped, &edited, 47).is_empty());
        assert!(!FACTION.mirrored_fields().any(|field| field == 27));
        assert!(FACTION.mirrored_fields().any(|field| field == 28));
        assert_eq!(FACTION_TEMPLATE.mirrored_fields().count(), 14);
    }

    #[test]
    fn a_new_faction_names_every_column_and_quotes_its_text() {
        let shipped = factions(&[IRONFORGE]);
        let edited = factions(&[
            IRONFORGE,
            (
                955,
                &[(1, 55), (2, 255), (14, 1), (18, 469)],
                &[(19, "Miller's Rest"), (28, "The mill's own folk.")],
            ),
        ]);
        assert_eq!(FACTION.changes(&shipped, &edited, 955).len(), FACTION.columns.len());
        let sql = FACTION.statements(&shipped, &edited, 955);
        assert!(sql[0].ends_with("469, 'Miller\\'s Rest', 'The mill\\'s own folk.');"), "{}", sql[0]);
        assert!(sql[1].contains("`name` = 'Miller\\'s Rest'"), "{}", sql[1]);
    }

    /// A reputation slot of -1 is written as -1 and not as the unsigned
    /// value the file holds.
    #[test]
    fn no_reputation_slot_is_minus_one() {
        let shipped = factions(&[]);
        let edited = factions(&[(14, &[(1, u32::MAX)], &[(19, "Monster")])]);
        let changes = FACTION.changes(&shipped, &edited, 14);
        assert_eq!(changes[0].column, "reputation_list_id");
        assert_eq!(changes[0].value, "-1");
    }

    #[test]
    fn a_template_is_thirteen_unsigned_columns() {
        let shipped = table(14, &[(11, &[(1, 11), (2, 2081), (3, 2), (5, 8), (10, 469)], &[])]);
        let mut edited = table(14, &[(11, &[(1, 11), (2, 2081), (3, 2), (5, 8), (10, 469)], &[])]);
        edited.set_u32(0, 6, 67);
        let sql = FACTION_TEMPLATE.statements(&shipped, &edited, 11);
        assert_eq!(
            sql[0],
            "INSERT IGNORE INTO `faction_template` (`id`, `build`, `faction_id`, `faction_flags`, \
             `our_mask`, `friendly_mask`, `hostile_mask`, `enemy_faction1`, `enemy_faction2`, \
             `enemy_faction3`, `enemy_faction4`, `friend_faction1`, `friend_faction2`, \
             `friend_faction3`, `friend_faction4`) VALUES (11, 5875, 11, 2081, 2, 0, 8, 67, 0, 0, \
             0, 469, 0, 0, 0);"
        );
        assert_eq!(
            sql[1],
            "UPDATE `faction_template` SET `enemy_faction1` = 67 WHERE `id` = 11 AND `build` = 5875;"
        );
    }

    /// A row the table cannot hold is refused before a statement exists,
    /// with the reason.
    #[test]
    fn a_row_the_table_cannot_hold_is_refused_with_the_reason() {
        let shipped = factions(&[]);
        let far = factions(&[(70_000, &[], &[(19, "Past the key")])]);
        assert!(FACTION.misfit(&far, 70_000).is_some_and(|why| why.contains("between 1 and")));
        assert!(FACTION.statements(&shipped, &far, 70_000).is_empty());
        let wide = factions(&[(955, &[(2, 0x0100_0000)], &[(19, "Wide mask")])]);
        assert!(FACTION.misfit(&wide, 955).is_some_and(|why| why.contains("base_rep_race_mask1")));
        let low = factions(&[(955, &[(10, (-9_000_000i32) as u32)], &[(19, "Low base")])]);
        assert!(FACTION.misfit(&low, 955).is_some_and(|why| why.contains("base_rep_value1")));
        let long = "x".repeat(MAX_NAME + 1);
        let long_name = factions(&[(955, &[], &[(19, &long)])]);
        assert!(FACTION.misfit(&long_name, 955).is_some_and(|why| why.contains("name")));
        assert!(FACTION.fits(&factions(&[(955, &[(10, (-42_000i32) as u32)], &[(19, "Hated")])]), 955));
        assert!(!FACTION.fits(&shipped, 955), "a row the file does not have");
        let template = table(14, &[(1678, &[(1, 0x0100_0000)], &[])]);
        assert!(FACTION_TEMPLATE.misfit(&template, 1678).is_some_and(|why| why.contains("faction_id")));
    }

    #[test]
    fn the_undo_deletes_a_row_this_project_made_and_restores_one_it_did_not() {
        let shipped = factions(&[IRONFORGE]);
        let mut edited = factions(&[IRONFORGE]);
        edited.set_u32(0, 18, 67);
        let changes = FACTION.changes(&shipped, &edited, 47);
        assert_eq!(
            FACTION.undo(47, &changes, None).unwrap(),
            "DELETE FROM `faction` WHERE `id` = 47 AND `build` = 5875;"
        );
        let now = HashMap::from([("team".to_string(), Some("469".to_string()))]);
        assert_eq!(
            FACTION.undo(47, &changes, Some(&now)).unwrap(),
            "UPDATE `faction` SET `team` = 469 WHERE `id` = 47 AND `build` = 5875;"
        );
        assert!(FACTION.undo(47, &[], None).is_none());
        assert_eq!(
            FACTION_TEMPLATE.dev_row_query(11),
            "SELECT * FROM `faction_template` WHERE `id` = 11 AND `build` = 5875;"
        );
    }

    #[test]
    fn a_mirror_is_found_by_either_table() {
        assert_eq!(for_table(TABLE).map(|m| m.dbc), Some("Faction"));
        assert_eq!(for_table(TEMPLATE).map(|m| m.dbc), Some("FactionTemplate"));
        assert_eq!(for_dbc("FactionTemplate").map(|m| m.table), Some(TEMPLATE));
        assert!(for_table("faction_group").is_none() && for_dbc("FactionGroup").is_none());
    }
}
