//! The server quest rows a project changes, written as SQL and applied when
//! the user asks. Every change, including a removal, takes effect on a reload.
//!
//! ## Storage and apply follow [`super::items`]; a removal is live on a reload
//!
//! An edit is stored, batched, written and undone as in that module: the
//! project's store accumulates typed values, a save writes the SQL, nothing
//! reaches the database until **Apply**, and the statements that put each row
//! back are written from the database immediately before anything runs.
//!
//! The difference is that a quest row can be removed without a restart.
//! `LoadQuests` clears its map before it reads and every lookup of a quest
//! tests the pointer it is given, so the server stops offering a quest that has
//! gone from the table rather than crashing on it. `vale_mangos::quest`'s
//! module comment records where this was read. A [`vale_mangos::row::Life::Delete`] row is
//! therefore written rather than refused, and its undo is
//! `Quests::undo_of_a_removal`: every version of the template and every row of the
//! six dependent tables, as the `INSERT`s that restore them.
//!
//! ## Files, tables and reload order
//!
//! ```text
//! sql\quests.sql         what this project does to quest_template and the four
//!                        relation tables
//! sql\quests-revert.sql  the statements that put those rows back
//! server\rows.txt        the store both are written from, shared
//! ```
//!
//! An apply asks a running playtest for one `.reload` per table it wrote, in
//! `vale_mangos::quest::RELOAD_ORDER`: the templates first, because the
//! relation loader drops any row whose quest is not in the template map. A
//! relation to a quest created in the same apply would be dropped by a reload
//! in the other order and reported as loaded.
//!
//! The cost of a reload is stated on the Server panel and not prevented: it
//! frees every `Quest`, and two escort script bases hold a pointer to one. See
//! `vale_mangos::quest`.

use super::stack::Subject;
use super::subject::{self, Row, RowSubject};
use vale_mangos::conn::Db;
use vale_mangos::quest;
use vale_mangos::row::{self, Edits};

/// What this project does to the server's quests, as SQL.
pub const SQL_VPATH: &str = "sql\\quests.sql";

/// The statements that put the changed rows back, as SQL.
pub const REVERT_VPATH: &str = "sql\\quests-revert.sql";

/// The quest subject: `quest_template` and the four relation tables, in
/// [`quest::RELOAD_ORDER`], which is [`quest::TABLES`]' order.
pub struct Quests;

impl RowSubject for Quests {
    fn subject(&self) -> Subject {
        Subject::Quests
    }

    fn name(&self) -> &'static str {
        "quests"
    }

    fn label(&self) -> &'static str {
        "Quests"
    }

    fn word(&self) -> &'static str {
        "quests"
    }

    fn noun(&self) -> &'static str {
        "quest"
    }

    fn tables(&self) -> &'static [row::Table] {
        quest::TABLES
    }

    fn table_list(&self) -> &'static str {
        "quest_template, and the four quest relation tables"
    }

    fn sql_vpath(&self) -> &'static str {
        SQL_VPATH
    }

    fn revert_vpath(&self) -> &'static str {
        REVERT_VPATH
    }

    fn sql_about(&self) -> &'static str {
        "the server's quests"
    }

    fn sql_note(&self) -> String {
        "-- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then reload, the templates first:\n\
         --   .reload quest_template\n\
         --   .reload creature_questrelation       (and the three beside it)\n\
         -- The relation loader drops a row whose quest it has not loaded."
            .to_string()
    }

    fn going_live(&self) -> &'static str {
        "Live on `.reload quest_template` and then one reload per relation table \
         written, which an apply sends in that order when it is made with the \
         panels open over a playtest \u{2014} removals included. Applied at any \
         other time, it is live after a restart. The reload frees every quest the server holds, so do \
         not apply while an escort quest is in progress on the server."
    }

    /// All five tables in order, not only the tables a plan writes: the
    /// put-back before an Apply may have written any of them, and a move writes
    /// the relation tables through its references.
    fn reloads(&self) -> Vec<&'static str> {
        quest::RELOAD_ORDER.to_vec()
    }

    /// A template arriving at an entry is refused when the entry is taken or
    /// named. A relation's key is two ids and neither is its own, so the check
    /// is on the whole key.
    fn refuse_arrival(&self, db: &mut Db, row: &Row, own: &Edits) -> Result<(), String> {
        match row.table.name == quest::TEMPLATE {
            true => subject::refuse_a_taken_id(db, row, own, None),
            false => subject::refuse_a_present_key(db, row),
        }
    }

    /// A quest this project creates is undone by removing it with its
    /// dependents, and a relation by removing the one row.
    fn undo_of_a_creation(&self, row: &Row) -> Vec<String> {
        match (row.table.name == quest::TEMPLATE, row.key.first()) {
            (true, Some(entry)) => quest::delete_statements(entry as u32),
            (true, None) => Vec::new(),
            (false, _) => vec![row::delete(row.table.name, &row.key)],
        }
    }

    /// A removed quest takes every content-patch version of the template and
    /// each of [`quest::DEPENDENTS`]; a removed relation is the one row.
    /// `None` when it is not in the database.
    fn undo_of_a_removal(&self, db: &mut Db, row: &Row) -> Result<Option<Vec<String>>, String> {
        if row.table.name != quest::TEMPLATE {
            return subject::snapshot_one_row(db, row);
        }
        let Some(entry) = row.key.first() else {
            return Ok(None);
        };
        let versions = db.rows(&row::rows_where(quest::TEMPLATE, "entry", entry))?;
        if versions.is_empty() {
            return Ok(None);
        }
        // The `DELETE`s first, so the entry can be run against a quest it has
        // already restored; see `super::reconcile`.
        let mut out = quest::delete_statements(entry as u32);
        for held in &versions {
            out.extend(row::insert_from_row(quest::TEMPLATE, held));
        }
        for (table, column) in quest::DEPENDENTS {
            for held in db.rows(&row::rows_where(table, column, entry))? {
                out.extend(row::insert_from_row(table, &held));
            }
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::subject::Plan;
    use vale_mangos::row::{Key, Life, RowEdit};

    fn plan_from(edits: &Edits) -> Plan {
        subject::plan_of(&Quests, edits)
    }

    fn a_new_quest(entry: u32) -> (Key, RowEdit) {
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in quest::new_quest("A Test") {
            row.columns.insert(change.column.to_string(), change.value);
        }
        (quest::template_key(entry, 10), row)
    }

    fn a_new_relation() -> RowEdit {
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in quest::new_relation() {
            row.columns.insert(change.column.to_string(), change.value);
        }
        row
    }

    /// An edit is one `UPDATE` naming the entry and the patch.
    #[test]
    fn an_edit_is_one_update_naming_the_whole_key() {
        let mut edits = Edits::default();
        edits.set(
            quest::TEMPLATE,
            &quest::template_key(783, 0),
            "QuestLevel",
            Some("5".into()),
        );
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(
            plan.statements(),
            vec![
                "UPDATE `quest_template` SET `QuestLevel` = 5 WHERE `entry` = 783 AND `patch` = 0;"
            ]
        );
    }

    /// A quest is written before the relation that names it, whatever order
    /// the store holds them in, and the reloads are asked for in that order.
    #[test]
    fn a_quest_is_written_and_reloaded_before_its_relations() {
        let mut edits = Edits::default();
        // The relation first, which is the order a key sort of the store gives.
        edits.set_row_line(
            quest::CREATURE_GIVES,
            &quest::relation_key(197, 2_000_000),
            Some(&a_new_relation().to_line()),
        );
        let (key, row) = a_new_quest(2_000_000);
        edits.set_row_line(quest::TEMPLATE, &key, Some(&row.to_line()));
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (2, 0, 0));
        let sql = plan.statements();
        assert_eq!(sql.len(), 4);
        assert!(
            sql[1].starts_with("INSERT INTO `quest_template`"),
            "{}",
            sql[1]
        );
        assert!(
            sql[3].starts_with("INSERT INTO `creature_questrelation`"),
            "{}",
            sql[3]
        );
        assert_eq!(plan.tables(), vec![quest::TEMPLATE, quest::CREATURE_GIVES]);
    }

    /// A removed quest is written as a `DELETE` of the entry alone, and of the
    /// six dependent tables. Unlike an item removal, it allows the reload.
    #[test]
    fn a_removed_quest_is_written_with_its_dependents() {
        let mut edits = Edits::default();
        edits.set_life(quest::TEMPLATE, &quest::template_key(783, 0), Life::Delete);
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (0, 0, 1));
        let sql = plan.statements();
        assert_eq!(sql.len(), 1 + quest::DEPENDENTS.len());
        assert_eq!(sql[0], "DELETE FROM `quest_template` WHERE `entry` = 783;");
    }

    /// A removed relation is one `DELETE` naming both key columns.
    #[test]
    fn a_removed_relation_is_one_delete() {
        let mut edits = Edits::default();
        edits.set_life(
            quest::CREATURE_TAKES,
            &quest::relation_key(197, 783),
            Life::Delete,
        );
        assert_eq!(
            plan_from(&edits).statements(),
            vec!["DELETE FROM `creature_involvedrelation` WHERE `id` = 197 AND `quest` = 783;"]
        );
    }

    /// A creation that does not name every column is not written.
    #[test]
    fn a_created_quest_missing_a_column_is_refused() {
        let (key, mut row) = a_new_quest(2_000_000);
        row.columns.remove("RewXP");
        let mut edits = Edits::default();
        edits.set_row_line(quest::TEMPLATE, &key, Some(&row.to_line()));
        let plan = plan_from(&edits);
        assert!(plan.rows.is_empty());
        assert!(plan.refused[0].contains("RewXP"), "{:?}", plan.refused);
    }

    /// The item, quest and creature writers each skip the other subjects' rows
    /// in the shared store without a refusal.
    #[test]
    fn each_writer_ignores_the_other_subjects_rows() {
        let mut edits = Edits::default();
        edits.set(
            vale_mangos::item::TEMPLATE,
            &vale_mangos::item::template_key(2589, 0),
            "quality",
            Some("3".to_string()),
        );
        edits.set(
            quest::TEMPLATE,
            &quest::template_key(783, 0),
            "QuestLevel",
            Some("5".into()),
        );
        let quests = plan_from(&edits);
        assert_eq!(quests.rows.len(), 1);
        assert!(quests.refused.is_empty(), "{:?}", quests.refused);
        let items = subject::plan_of(&super::super::items::Items, &edits);
        assert_eq!(items.rows.len(), 1);
        assert!(items.refused.is_empty(), "{:?}", items.refused);
        let creatures = subject::plan_of(&super::super::creatures::Creatures, &edits);
        assert!(creatures.rows.is_empty());
        assert!(creatures.refused.is_empty(), "{:?}", creatures.refused);
    }

    /// A key column among a row's columns is refused by name.
    #[test]
    fn a_key_column_is_not_written() {
        let mut edits = Edits::default();
        edits.set(
            quest::TEMPLATE,
            &quest::template_key(783, 0),
            "entry",
            Some("9".into()),
        );
        let plan = plan_from(&edits);
        assert!(plan.is_empty());
        assert!(
            plan.refused[0].contains("part of the key"),
            "{:?}",
            plan.refused
        );
    }
}
