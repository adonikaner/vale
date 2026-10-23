//! What a project changes about the server's game objects, as SQL, applied when
//! a person asks.
//!
//! ## The order an apply runs in, shared with [`super::creatures`]
//!
//! The project's store accumulates typed values, a save writes the SQL, nothing
//! reaches the database until Apply is pressed, and the statements that put
//! each row back are read from the database immediately before that row is
//! written. [`super::reconcile`] holds that order, and every subject here
//! applies in it.
//!
//! A game object walks no path, so the plan is rows alone.
//!
//! ## Why an apply asks for a restart rather than a reload
//!
//! `.reload gameobject` calls `LoadGameobjects(true)`, which adds a spawn it has
//! not seen to its grid and never erases one. `.reload gameobject_template`
//! rewrites each `GameObjectInfo` in place, but an object already in the world
//! took its display id, faction, flags and size from the template when it was
//! created. So an apply sends no reload and its line says to restart the
//! server. See `vale_mangos::gameobject`, where the source lines are.
//!
//! ## The three files this subject reads and writes
//!
//! ```text
//! sql\gameobjects.sql         what this project does to the server's game objects
//! sql\gameobjects-revert.sql  the statements that put those rows back
//! server\rows.txt             the store both are written from, shared
//! ```
//!
//! The store is the one file every server subject keeps its rows in, keyed by
//! table; each writer skips the rows the others own. The SQL files are this
//! subject's own because it is applied by its own gesture.

use super::stack::Subject;
use super::subject::{self, Row, RowSubject};
use vale_mangos::conn::Db;
use vale_mangos::gameobject;
use vale_mangos::row::{self, Edits};

/// The project path of the SQL file a save writes.
pub(crate) const SQL_VPATH: &str = "sql\\gameobjects.sql";

/// The project path of the revert file an apply writes.
pub(crate) const REVERT_VPATH: &str = "sql\\gameobjects-revert.sql";

/// The game object subject: `gameobject_template` and `gameobject`.
pub(crate) struct GameObjects;

impl RowSubject for GameObjects {
    fn subject(&self) -> Subject {
        Subject::GameObjects
    }

    fn name(&self) -> &'static str {
        "game objects"
    }

    fn label(&self) -> &'static str {
        "Objects"
    }

    fn word(&self) -> &'static str {
        "gameobjects"
    }

    fn noun(&self) -> &'static str {
        "game object"
    }

    fn tables(&self) -> &'static [row::Table] {
        gameobject::TABLES
    }

    fn table_list(&self) -> &'static str {
        "gameobject_template, gameobject"
    }

    fn sql_vpath(&self) -> &'static str {
        SQL_VPATH
    }

    fn revert_vpath(&self) -> &'static str {
        REVERT_VPATH
    }

    fn sql_about(&self) -> &'static str {
        "the server's game objects"
    }

    fn sql_note(&self) -> String {
        "-- Nothing applies this by itself. Apply it from the editor's Server\n\
         -- panel, or run it by hand, and then RESTART the server: .reload\n\
         -- gameobject never erases a spawn it has read, and an object already in\n\
         -- the world keeps the display id, faction, flags and size it was\n\
         -- created with."
            .to_string()
    }

    fn going_live(&self) -> &'static str {
        "The server has to be restarted. `.reload gameobject` never erases a spawn \
         it has read, and an object already in the world keeps the display id, \
         faction, flags and size it was created with."
    }

    /// None; the module comment says why.
    fn reloads(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn refuse_arrival(&self, db: &mut Db, row: &Row, own: &Edits) -> Result<(), String> {
        let limit = match row.table.name {
            gameobject::SPAWN => gameobject::MAX_GUID,
            _ => u64::from(gameobject::MAX_ENTRY),
        };
        subject::refuse_a_taken_id(db, row, own, Some(limit))
    }

    /// A spawn this project creates is undone by removing it with its two
    /// dependent tables, a template on its own.
    fn undo_of_a_creation(&self, row: &Row) -> Vec<String> {
        gameobject::insert_undo_statements(row.table.name, &row.key)
    }

    /// Everything a removed spawn takes with it: the `gameobject` row, then
    /// each dependent table's rows in `gameobject::DEPENDENTS`' order. `None`
    /// when the spawn is not there.
    fn undo_of_a_removal(&self, db: &mut Db, row: &Row) -> Result<Option<Vec<String>>, String> {
        subject::snapshot_with_dependents(db, row, gameobject::delete_statements(&row.key), &gameobject::DEPENDENTS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::subject::Plan;
    use vale_mangos::row::{Key, Life, RowEdit};

    fn plan_from(edits: &Edits) -> Plan {
        subject::plan_of(&GameObjects, edits)
    }

    /// A whole new spawn, as the tool writes one.
    fn a_new_spawn(guid: u64) -> (Key, RowEdit) {
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in gameobject::new_spawn(1731, 0, (-9400.0, -100.0, 60.0), 1.5) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        (Key::one("guid", guid), row)
    }

    fn store(rows: Vec<(Key, RowEdit)>) -> Edits {
        let mut edits = Edits::default();
        for (key, row) in rows {
            edits.set_row_line(gameobject::SPAWN, &key, Some(&row.to_line()));
        }
        edits
    }

    /// A created spawn is a `DELETE` of its key and an `INSERT` naming every
    /// column, which is `GameObject::SaveToDB`'s own pair.
    #[test]
    fn a_created_spawn_is_a_delete_and_an_insert() {
        let (key, row) = a_new_spawn(10_000_001);
        let plan = plan_from(&store(vec![(key, row)]));
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (1, 0, 0));
        let sql = plan.statements();
        assert_eq!(sql.len(), 2);
        assert_eq!(sql[0], "DELETE FROM `gameobject` WHERE `guid` = 10000001;");
        assert!(sql[1].starts_with("INSERT INTO `gameobject` (`guid`, "), "{}", sql[1]);
    }

    /// A creation that does not name every column is not written, and the
    /// refusal names the column.
    #[test]
    fn a_created_spawn_missing_a_column_is_refused() {
        let (key, mut row) = a_new_spawn(10_000_001);
        row.columns.remove("animprogress");
        let plan = plan_from(&store(vec![(key, row)]));
        assert!(plan.rows.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(plan.refused[0].contains("animprogress"), "{:?}", plan.refused);
    }

    /// A removed spawn is the row and its two dependent tables.
    #[test]
    fn a_removed_spawn_is_three_deletes() {
        let mut edits = Edits::default();
        edits.set_life(gameobject::SPAWN, &Key::one("guid", 29993), Life::Delete);
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (0, 0, 1));
        let sql = plan.statements();
        assert_eq!(sql.len(), 3);
        assert_eq!(
            sql.last().map(String::as_str),
            Some("DELETE FROM `gameobject` WHERE `guid` = 29993;")
        );
    }

    /// A template cannot be removed, and the plan says which row it would
    /// have been.
    #[test]
    fn a_template_removal_is_refused_by_name() {
        let mut edits = Edits::default();
        edits.set_life(
            gameobject::TEMPLATE,
            &Key::two(("entry", 1731), ("patch", 0)),
            Life::Delete,
        );
        let plan = plan_from(&edits);
        assert!(plan.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(plan.refused[0].contains("gameobject_template"), "{:?}", plan.refused);
    }

    /// A template this project creates is planned as a `DELETE` of its key
    /// and an `INSERT` naming every column, and one missing a column is
    /// refused.
    #[test]
    fn a_created_template_is_a_delete_and_an_insert() {
        let mut edits = Edits::default();
        let key = gameobject::template_key(2_000_000, 10);
        let mut row = vale_mangos::row::RowEdit { life: Life::Insert, ..Default::default() };
        for change in gameobject::new_template("Test Object") {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(gameobject::TEMPLATE, &key, Some(&row.to_line()));
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (1, 0, 0));
        let sql = plan.statements();
        assert_eq!(sql.len(), 2);
        assert_eq!(
            sql[0],
            "DELETE FROM `gameobject_template` WHERE `entry` = 2000000 AND `patch` = 10;"
        );
        assert!(sql[1].starts_with("INSERT INTO `gameobject_template` (`entry`, `patch`, "));

        let mut short = Edits::default();
        short.set_life(gameobject::TEMPLATE, &key, Life::Insert);
        short.set(gameobject::TEMPLATE, &key, "name", Some("'Half'".into()));
        let plan = plan_from(&short);
        assert!(plan.is_empty());
        assert!(plan.refused[0].contains("is created without"), "{:?}", plan.refused);
    }

    /// Another subject's rows are neither written nor refused here, and a
    /// game object's are neither in the creature writer.
    #[test]
    fn each_writer_skips_the_other_subjects_rows() {
        let mut edits = Edits::default();
        edits.set(
            vale_mangos::creature::SPAWN,
            &Key::one("guid", 19272),
            "position_z",
            Some("98.7".into()),
        );
        edits.set(
            gameobject::TEMPLATE,
            &Key::two(("entry", 1731), ("patch", 0)),
            "size",
            Some("2".into()),
        );
        let plan = plan_from(&edits);
        assert_eq!(plan.rows.len(), 1);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(
            plan.statements(),
            ["UPDATE `gameobject_template` SET `size` = 2 WHERE `entry` = 1731 AND `patch` = 0;"]
        );

        let creatures = subject::plan_of(&super::super::creatures::Creatures, &edits);
        assert_eq!(creatures.rows.len(), 1);
        assert!(creatures.refused.is_empty(), "{:?}", creatures.refused);
    }

    /// A data column is written under its own name, whatever the type calls it.
    #[test]
    fn a_data_column_is_written_as_the_column_it_is() {
        let mut edits = Edits::default();
        edits.set(
            gameobject::TEMPLATE,
            &Key::two(("entry", 1731), ("patch", 0)),
            "data0",
            Some("39".into()),
        );
        assert_eq!(
            plan_from(&edits).statements(),
            ["UPDATE `gameobject_template` SET `data0` = 39 WHERE `entry` = 1731 AND `patch` = 0;"]
        );
    }
}
