//! What a project changes about the server's loot, as SQL. It is applied when
//! a person asks, and takes effect on a reload of each table written.
//!
//! ## Edits work as in [`super::quests`], over nine tables of one schema
//!
//! An edit is stored, batched, written and undone as in that module: the
//! project's store accumulates typed values, a save writes the SQL, nothing
//! reaches the database until **Apply**, and the statements that put each row
//! back are written from the database immediately before anything runs.
//!
//! A row here is created, edited or removed, and all three are live on
//! `.reload <table>`: `LootStore::LoadLootTable` clears its store before it
//! reads (see `vale_mangos::loot`'s module comment). So a
//! [`vale_mangos::row::Life::Delete`] row is written, and its undo is the row as it stood.
//!
//! ## Removals before creations, inside a table
//!
//! A row moved between groups is a removal and a creation of the same
//! `(entry, item)` under two keys. On the five tables whose primary key is
//! `(entry, item)` alone, the `INSERT` fails while the old row stands, so
//! [`super::subject::Plan`] writes every table's removals before its creations. Put
//! back runs the file newest first, which undoes them in the other order.
//!
//! ```text
//! sql\loot.sql          what this project does to the loot tables
//! sql\loot-revert.sql   the statements that put those rows back
//! server\rows.txt       the store both files are written from, shared
//! ```

use super::stack::Subject;
use super::subject::RowSubject;
use vale_mangos::loot;
use vale_mangos::row;

/// What this project does to the server's loot, as SQL.
pub(crate) const SQL_VPATH: &str = "sql\\loot.sql";

/// The SQL that puts back the loot rows this project changed.
pub(crate) const REVERT_VPATH: &str = "sql\\loot-revert.sql";

/// The loot subject: the nine `*_loot_template` tables.
///
/// A created row is refused when the whole key is already there, which is the
/// module's default. A loot row's key is five columns and none of them is an
/// id of its own. A table whose primary key is `(entry, item)` alone refuses a
/// second group of the same item itself, and MySQL's error message is
/// reported unchanged.
pub(crate) struct Loot;

impl RowSubject for Loot {
    fn subject(&self) -> Subject {
        Subject::Loot
    }

    fn name(&self) -> &'static str {
        "loot"
    }

    fn label(&self) -> &'static str {
        "Loot"
    }

    fn word(&self) -> &'static str {
        "loot"
    }

    fn noun(&self) -> &'static str {
        "loot"
    }

    fn tables(&self) -> &'static [row::Table] {
        loot::TABLES
    }

    fn table_list(&self) -> &'static str {
        "the nine *_loot_template tables"
    }

    fn sql_vpath(&self) -> &'static str {
        SQL_VPATH
    }

    fn revert_vpath(&self) -> &'static str {
        REVERT_VPATH
    }

    fn sql_about(&self) -> &'static str {
        "the server's loot"
    }

    fn sql_note(&self) -> String {
        "-- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then reload each table it names:\n\
         --   .reload creature_loot_template       (and likewise the others)\n\
         -- Every loot loader clears its store first, so removals are live too."
            .to_string()
    }

    fn going_live(&self) -> &'static str {
        "Live on `.reload <table>` for each loot table written, which an apply sends \
         when it is made with the panels open over a playtest \u{2014} removals \
         included, since every loot loader clears its store before it reads. \
         Applied at any other time, it is live after a restart. Loot already rolled onto a corpse or into \
         an opened chest keeps the list it was given."
    }

    /// All nine tables, not only the tables a plan writes, because the
    /// put-back before an Apply may have written any of them.
    fn reloads(&self) -> Vec<&'static str> {
        loot::TABLES.iter().map(|table| table.name).collect()
    }

    /// The loader's own checks of a created row.
    fn faults(&self, _table: &'static row::Table, row: &vale_mangos::schema::Row) -> Vec<String> {
        loot::Entry::from_row(row).map(|entry| entry.check()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::subject::{self, Plan};
    use vale_mangos::row::{Edits, Key, Life, RowEdit};

    fn plan_from(edits: &Edits) -> Plan {
        subject::plan_of(&Loot, edits)
    }

    fn created(entry: u32, item: u32) -> (Key, RowEdit) {
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in loot::new_item(entry, item) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        (loot::Entry::item(entry, item).key(), row)
    }

    /// An edit is one `UPDATE` naming all five key columns.
    #[test]
    fn an_edit_is_one_update_naming_the_whole_key() {
        let mut edits = Edits::default();
        let key = loot::Entry::item(68, 2589).key();
        edits.set(loot::CREATURE, &key, "ChanceOrQuestChance", Some("35".into()));
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (0, 1, 0));
        assert_eq!(
            plan.statements(),
            vec![
                "UPDATE `creature_loot_template` SET `ChanceOrQuestChance` = 35 WHERE `entry` = 68 \
                 AND `item` = 2589 AND `groupid` = 0 AND `patch_min` = 0 AND `patch_max` = 10;"
            ]
        );
        assert_eq!(plan.tables(), vec![loot::CREATURE]);
    }

    /// A removal is written before a creation in the same table, whatever
    /// order the store holds them in. A regroup on a table keyed by
    /// `(entry, item)` needs that order.
    #[test]
    fn a_removal_is_written_before_a_creation() {
        let mut edits = Edits::default();
        // The creation first: it sorts first by key, since its group is lower.
        let (key, row) = created(1502, 2770);
        edits.set_row_line(loot::GAMEOBJECT, &key, Some(&row.to_line()));
        edits.set_life(loot::GAMEOBJECT, &loot::key(1502, 2770, 3, 0, 10), Life::Delete);
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (1, 0, 1));
        let sql = plan.statements();
        assert_eq!(sql.len(), 3);
        assert!(sql[0].starts_with("DELETE FROM `gameobject_loot_template`"), "{}", sql[0]);
        assert!(sql[0].contains("`groupid` = 3"), "{}", sql[0]);
        assert!(sql[2].starts_with("INSERT INTO `gameobject_loot_template`"), "{}", sql[2]);
    }

    /// A creation the server would skip at load is refused with the loader's
    /// own reason, and one missing a column is refused by name.
    #[test]
    fn a_created_row_the_server_would_skip_is_refused() {
        let (key, mut row) = created(68, 2589);
        row.columns.insert("mincountOrRef".to_string(), "0".to_string());
        let mut edits = Edits::default();
        edits.set_row_line(loot::CREATURE, &key, Some(&row.to_line()));
        let plan = plan_from(&edits);
        assert!(plan.rows.is_empty());
        assert!(plan.refused[0].contains("mincountOrRef is 0"), "{:?}", plan.refused);

        let (key, mut row) = created(68, 2589);
        row.columns.remove("maxcount");
        let mut edits = Edits::default();
        edits.set_row_line(loot::CREATURE, &key, Some(&row.to_line()));
        let plan = plan_from(&edits);
        assert!(plan.rows.is_empty());
        assert!(plan.refused[0].contains("maxcount"), "{:?}", plan.refused);
    }

    /// The other writers ignore a loot row, and this one ignores theirs.
    #[test]
    fn each_writer_ignores_the_other_subjects_rows() {
        let mut edits = Edits::default();
        edits.set(
            loot::SKINNING,
            &loot::Entry::item(299, 2934).key(),
            "maxcount",
            Some("3".into()),
        );
        edits.set(
            vale_mangos::quest::TEMPLATE,
            &vale_mangos::quest::template_key(783, 0),
            "QuestLevel",
            Some("5".into()),
        );
        let loot = plan_from(&edits);
        assert_eq!(loot.rows.len(), 1);
        assert!(loot.refused.is_empty(), "{:?}", loot.refused);
        let quests = subject::plan_of(&super::super::quests::Quests, &edits);
        assert_eq!(quests.rows.len(), 1);
        assert!(quests.refused.is_empty(), "{:?}", quests.refused);
        let items = subject::plan_of(&super::super::items::Items, &edits);
        assert!(items.rows.is_empty());
        assert!(items.refused.is_empty(), "{:?}", items.refused);
    }

    /// A key column among a row's columns is refused by name.
    #[test]
    fn a_key_column_is_not_written() {
        let mut edits = Edits::default();
        edits.set(
            loot::CREATURE,
            &loot::Entry::item(68, 2589).key(),
            "groupid",
            Some("2".into()),
        );
        let plan = plan_from(&edits);
        assert!(plan.is_empty());
        assert!(plan.refused[0].contains("part of the key"), "{:?}", plan.refused);
    }
}
