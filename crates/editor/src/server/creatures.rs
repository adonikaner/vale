//! What a project changes about the server's creatures, as SQL, applied when a
//! person asks.
//!
//! ## Why creature edits are applied by hand, in a batch
//!
//! [`super::rows`] — the spell half — applies itself as part of every save and
//! then tells a running playtest to `.reload`. That works for spells because a
//! spell reaches the client through `Spell.dbc` and the server through one
//! table with a documented reload, and because the edit is a diff of two files
//! that always says the same thing.
//!
//! Creature edits do not work that way. A change to `creature_template` is not
//! visible in a running server in any reliable way: `.reload creature_template`
//! replaces the templates but does not restat creatures that are already
//! spawned, so a display id, a scale or a faction changes for nothing standing
//! in the world. An automatic apply therefore wrote rows to a database and
//! reported a success that the game did not show.
//!
//! So edits accumulate in the project, a save writes them as SQL, and nothing
//! reaches the database until Apply is pressed. The person then restarts the
//! server, which is the only thing that makes every kind of creature change
//! live at once. Revert runs the file beside it.
//!
//! ## The files the creature half writes
//!
//! ```text
//! sql\creatures.sql         what this project does to the server's creatures
//! sql\creatures-revert.sql  …and what puts those rows back
//! server\rows.txt           …and the store both are written from
//! ```
//!
//! They are separate from `sql\world.sql` and `sql\revert.sql`, which are the
//! spell half's. One file holding two subjects with two different application
//! rules could not be reasoned about: applying it applied both, and reverting
//! it reverted both.
//!
//! Each is rewritten whole on every save, because the store describes the
//! whole of what the project changes rather than an increment of it. An edit
//! taken back removes its own statement.
//!
//! ## How the revert file is written
//!
//! One `UPDATE` per row in `creatures.sql`, naming the same columns, with the
//! values read out of the database immediately before the apply runs. So the
//! pair is symmetrical: run one and the rows are the project's, run the other
//! and they are what they were.
//!
//! It is written before any statement runs. A crash between the two leaves an
//! undo for a change that did not happen, which restores what is already
//! there; the other order would leave a change with no undo.
//!
//! A row already in the revert file is not snapshotted again. Applying twice
//! without reverting would otherwise capture the state after the first apply,
//! and Revert would return the row to a half-edited state rather than to what
//! the project found.

use super::stack::Subject;
use super::subject::{self, Row, RowSubject, Stores, Whole};
use crate::session::EditSession;
use vale_mangos::conn::Db;
use vale_mangos::creature;
use vale_mangos::row::{self, Edits, Life};

/// Where the project keeps the waypoint paths it changes. A store of their own,
/// because a path's edit is a set of rows rather than a column. See
/// [`vale_mangos::path`].
pub const PATHS_VPATH: &str = "server\\paths.txt";

/// The SQL the row store's creature rows and the paths come to.
pub const SQL_VPATH: &str = "sql\\creatures.sql";

/// The SQL that puts those rows back.
pub const REVERT_VPATH: &str = "sql\\creatures-revert.sql";

/// The creature subject: `creature_template`, `creature`, and the waypoint
/// paths written whole into `creature_movement`.
pub struct Creatures;

impl RowSubject for Creatures {
    fn subject(&self) -> Subject {
        Subject::Creatures
    }

    fn name(&self) -> &'static str {
        "creatures"
    }

    fn label(&self) -> &'static str {
        "Creatures"
    }

    fn word(&self) -> &'static str {
        "creatures"
    }

    fn noun(&self) -> &'static str {
        "creature"
    }

    fn tables(&self) -> &'static [row::Table] {
        creature::TABLES
    }

    fn table_list(&self) -> &'static str {
        "creature_template, creature, creature_movement"
    }

    fn sql_vpath(&self) -> &'static str {
        SQL_VPATH
    }

    fn revert_vpath(&self) -> &'static str {
        REVERT_VPATH
    }

    fn sql_about(&self) -> &'static str {
        "the server's creatures"
    }

    fn sql_note(&self) -> String {
        "-- Nothing applies this by itself. Apply it from the editor's Server\n\
         -- panel, or run it by hand, and then RESTART the server: vmangos reads\n\
         -- creature_template once at startup, and .reload does not restat\n\
         -- creatures that are already spawned."
            .to_string()
    }

    fn going_live(&self) -> &'static str {
        "The server has to be restarted. `.reload creature_template` does not \
         restat creatures already spawned, and `.reload creature` never erases a \
         spawn it has read."
    }

    /// None: no reload makes every kind of creature change live, so an Apply
    /// says to restart the server. See the module comment.
    fn reloads(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// A table no subject writes is refused here, once. Every subject in
    /// [`Subject::ORDER`] is asked whether it owns the table, so a subject
    /// added there is skipped here without a second list to keep.
    fn refuses_unowned(&self) -> bool {
        true
    }

    /// The waypoint paths, which need no column check: every column of a
    /// movement row is `vale_mangos`' own constant and none of them is typed.
    ///
    /// A path on a spawn this same project removes is refused. It would write
    /// `creature_movement` rows keyed to a guid whose creature the six
    /// `DELETE`s have just taken away: an orphan the server reports at every
    /// start and nothing points at. Only a spawn's own path can be orphaned
    /// this way: a template path is keyed by a creature entry, which no
    /// removal here touches.
    fn wholes(&self, stores: &Stores<'_>, rows: &[Row], refused: &mut Vec<String>) -> Vec<Whole> {
        let mut out = Vec::new();
        for path in stores.paths.iter() {
            let removed = path.which == vale_mangos::path::Which::Spawn
                && rows.iter().any(|row| {
                    row.life == Life::Delete && row.table.name == creature::SPAWN && row.key.first() == Some(path.owner)
                });
            if removed {
                refused.push(format!(
                    "the path on {} {} is not written \u{2014} this project removes that spawn",
                    path.which.table(),
                    path.owner
                ));
                continue;
            }
            out.push(Whole::Path(path));
        }
        out
    }

    fn refuse_arrival(&self, db: &mut Db, row: &Row, own: &Edits) -> Result<(), String> {
        let limit = match row.table.name {
            creature::SPAWN => creature::MAX_GUID,
            _ => u64::from(creature::MAX_ENTRY),
        };
        subject::refuse_a_taken_id(db, row, own, Some(limit))
    }

    /// A spawn this project creates is undone by removing it with its five
    /// dependent tables, a template on its own. See
    /// `creature::insert_undo_statements`.
    fn undo_of_a_creation(&self, row: &Row) -> Vec<String> {
        creature::insert_undo_statements(row.table.name, &row.key)
    }

    /// Everything a removed spawn takes with it: the `creature` row and then
    /// each of the five tables keyed by its guid, in [`creature::DEPENDENTS`]'
    /// own order, so running them restores the creature before the rows that
    /// point at it. `None` when the spawn is not there, which is a removal
    /// with nothing to undo.
    fn undo_of_a_removal(&self, db: &mut Db, row: &Row) -> Result<Option<Vec<String>>, String> {
        subject::snapshot_with_dependents(db, row, creature::delete_statements(&row.key), &creature::DEPENDENTS)
    }
}

/// Whether this project has applied that particular path to the database.
///
/// [`subject::apply::OnTheServer::covers`] for the one caller that wants a
/// single answer and has no reason to read the rest: the waypoint window is
/// about one path, and "this project has applied something" is not an answer
/// about the one being looked at.
pub fn path_applied(session: &EditSession, path: &vale_mangos::path::Path) -> bool {
    subject::apply::OnTheServer::read(session, &Creatures).covers(path.which.table(), &path.key())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::subject::Plan;
    use vale_mangos::row::{Assignment, Key};

    /// The plan the two stores come to, with no scripts.
    fn plan_from(edits: &Edits, paths: &vale_mangos::path::Paths) -> Plan {
        let scripts = vale_mangos::scripts::Scripts::default();
        subject::plan_from(&Creatures, &Stores { edits, paths, scripts: &scripts })
    }

    fn table(name: &str) -> &'static row::Table {
        row::table_named(creature::TABLES, name).expect("a creature table")
    }

    /// A whole new spawn, as the tool writes one.
    fn a_new_spawn(guid: u64) -> (Key, vale_mangos::row::RowEdit) {
        let mut row = vale_mangos::row::RowEdit {
            life: Life::Insert,
            ..vale_mangos::row::RowEdit::default()
        };
        for change in creature::new_spawn(68, 0, (-8817.5, 809.0, 98.7), 1.5) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        (Key::one("guid", guid), row)
    }

    fn store(rows: Vec<(Key, vale_mangos::row::RowEdit)>) -> vale_mangos::row::Edits {
        let mut edits = vale_mangos::row::Edits::default();
        for (key, row) in rows {
            edits.set_row_line(creature::SPAWN, &key, Some(&row.to_line()));
        }
        edits
    }

    /// A created spawn is a `DELETE` of its key and an `INSERT` naming every
    /// column, and the plan counts it apart from the rows it edits.
    ///
    /// The pair rather than the `INSERT` alone is what makes applying twice
    /// mean the same as applying once. It is `Creature::SaveToDB`'s own, and
    /// the apply is what establishes that the key is this project's before
    /// running it.
    #[test]
    fn a_created_spawn_is_a_delete_and_an_insert() {
        let (key, row) = a_new_spawn(10_000_001);
        let plan = plan_from(&store(vec![(key, row)]), &Default::default());
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (1, 0, 0));
        let sql = plan.statements();
        assert_eq!(sql.len(), 2);
        assert_eq!(sql[0], "DELETE FROM `creature` WHERE `guid` = 10000001;");
        assert!(
            sql[1].starts_with("INSERT INTO `creature` (`guid`, "),
            "{}",
            sql[1]
        );
        assert!(sql[1].contains("10000001"), "{}", sql[1]);
    }

    /// A row of another subject's table is skipped without a refusal, and a
    /// table no subject writes is refused by name. The behaviour tables are
    /// the case that was refused: an event or a spell list made for a new
    /// creature put "creature_ai_events is not a table this editor writes" on
    /// the creature block of the Server panel.
    #[test]
    fn another_subjects_row_is_skipped_and_an_unknown_table_refused() {
        let row = vale_mangos::row::RowEdit {
            life: Life::Update,
            columns: [("comment".to_string(), "'a note'".to_string())].into(),
            ..vale_mangos::row::RowEdit::default()
        };
        let mut edits = vale_mangos::row::Edits::default();
        for table in [
            vale_mangos::eventai::TABLE,
            vale_mangos::creaturespells::TABLE,
            "no_such_table",
        ] {
            edits.set_row_line(table, &Key::one("entry", 680), Some(&row.to_line()));
        }
        let plan = plan_from(&edits, &Default::default());
        assert_eq!(plan.refused.len(), 1, "{:?}", plan.refused);
        assert!(plan.refused[0].contains("no_such_table"), "{:?}", plan.refused);
        assert!(plan.rows.is_empty());
    }

    /// A creation that does not name every column is not written. A column an
    /// `INSERT` leaves out takes the table's own default, and three of this
    /// table's defaults are wrong for a spawn placed by hand, so the answer is
    /// a refusal naming the columns rather than a row nobody chose.
    #[test]
    fn a_created_spawn_missing_a_column_is_refused() {
        let (key, mut row) = a_new_spawn(10_000_001);
        row.columns.remove("position_z");
        let plan = plan_from(&store(vec![(key, row)]), &Default::default());
        assert!(plan.rows.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(plan.refused[0].contains("position_z"), "{:?}", plan.refused);
    }

    /// A removed spawn is the row and its five dependent tables, and it needs
    /// no columns.
    #[test]
    fn a_removed_spawn_is_six_deletes() {
        let mut edits = vale_mangos::row::Edits::default();
        let key = Key::one("guid", 19272);
        edits.set_life(creature::SPAWN, &key, Life::Delete);
        let plan = plan_from(&edits, &Default::default());
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (0, 0, 1));
        assert_eq!(plan.statements().len(), 6);
    }

    /// A template cannot be removed, and the plan says which row it would
    /// have been rather than dropping it.
    #[test]
    fn a_template_removal_is_refused_by_name() {
        let mut edits = vale_mangos::row::Edits::default();
        let key = Key::two(("entry", 68), ("patch", 0));
        edits.set_life(creature::TEMPLATE, &key, Life::Delete);
        let plan = plan_from(&edits, &Default::default());
        assert!(plan.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(
            plan.refused[0].contains("creature_template"),
            "{:?}",
            plan.refused
        );
    }

    /// A template this project creates is planned as a `DELETE` of its key
    /// and an `INSERT` naming every column, and a creation missing a column
    /// is refused for the reason a spawn's is.
    #[test]
    fn a_created_template_is_a_delete_and_an_insert() {
        let mut edits = vale_mangos::row::Edits::default();
        let key = creature::template_key(2_000_000, 10);
        let mut row = vale_mangos::row::RowEdit {
            life: Life::Insert,
            ..Default::default()
        };
        for change in creature::new_template("Test Subject") {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(creature::TEMPLATE, &key, Some(&row.to_line()));
        let plan = plan_from(&edits, &Default::default());
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (1, 0, 0));
        let sql = plan.statements();
        assert_eq!(sql.len(), 2);
        assert_eq!(
            sql[0],
            "DELETE FROM `creature_template` WHERE `entry` = 2000000 AND `patch` = 10;"
        );
        assert!(sql[1].starts_with("INSERT INTO `creature_template` (`entry`, `patch`, "));
        assert!(!plan.rows[0].moves());

        let mut short = vale_mangos::row::Edits::default();
        short.set_life(creature::TEMPLATE, &key, Life::Insert);
        short.set(creature::TEMPLATE, &key, "name", Some("'Half'".into()));
        let plan = plan_from(&short, &Default::default());
        assert!(plan.is_empty());
        assert!(plan.refused[0].contains("is created without"), "{:?}", plan.refused);
    }

    /// A path on a spawn the same project removes is not written. The six
    /// `DELETE`s take the creature's `creature_movement` rows with it, and
    /// `INSERT`ing a path afterwards leaves points keyed to a guid with no
    /// creature: a row the server reports at every start and nothing points
    /// at.
    #[test]
    fn a_path_on_a_removed_spawn_is_refused() {
        let mut edits = vale_mangos::row::Edits::default();
        edits.set_life(creature::SPAWN, &Key::one("guid", 19272), Life::Delete);
        let mut paths = vale_mangos::path::Paths::default();
        paths.set(&vale_mangos::path::Path {
            which: vale_mangos::path::Which::Spawn,
            owner: 19272,
            nodes: vec![vale_mangos::path::Node::at(1.0, 2.0, 3.0)],
        });
        let plan = plan_from(&edits, &paths);
        assert!(plan.wholes.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(
            plan.refused[0].contains("removes that spawn"),
            "{:?}",
            plan.refused
        );
        // The same path on a spawn the project is not removing is written.
        let plan = plan_from(&Default::default(), &paths);
        assert_eq!(plan.wholes.len(), 1);
        assert!(plan.refused.is_empty());
    }

    /// A created spawn's `INSERT` comes before its path's rows. The
    /// statements are one file run top to bottom, and `creature_movement` rows
    /// for a creature that is not there yet are what the other order writes.
    #[test]
    fn a_new_spawn_is_written_before_the_path_it_is_given() {
        let (key, row) = a_new_spawn(10_000_001);
        let mut paths = vale_mangos::path::Paths::default();
        paths.set(&vale_mangos::path::Path {
            which: vale_mangos::path::Which::Spawn,
            owner: 10_000_001,
            nodes: vec![vale_mangos::path::Node::at(1.0, 2.0, 3.0)],
        });
        let sql = plan_from(&store(vec![(key, row)]), &paths).statements();
        let creature_at = sql
            .iter()
            .position(|s| s.contains("INTO `creature`"))
            .expect("the spawn");
        let path_at = sql
            .iter()
            .position(|s| s.contains("INTO `creature_movement`"))
            .expect("the path");
        assert!(creature_at < path_at, "{sql:?}");
    }

    /// One row is one statement, naming every column it changes and the whole
    /// key.
    #[test]
    fn a_row_is_one_update_naming_the_whole_key() {
        let row = Row {
            table: table(creature::TEMPLATE),
            key: Key::two(("entry", 68), ("patch", 0)),
            life: Life::Update,
            changes: vec![
                Assignment {
                    column: "level_min",
                    value: "56".into(),
                },
                Assignment {
                    column: "name",
                    value: "'Guard'".into(),
                },
            ],
            at: Key::two(("entry", 68), ("patch", 0)),
        };
        let sql = row.statements();
        assert_eq!(sql.len(), 1);
        assert!(
            sql[0].starts_with("UPDATE `creature_template` SET `level_min` = 56, `name` = 'Guard'")
        );
        assert!(
            sql[0].ends_with("WHERE `entry` = 68 AND `patch` = 0;"),
            "{}",
            sql[0]
        );
        assert_eq!(row.names(), "creature_template entry=68;patch=0");
    }

    /// A row with no changes is no statement at all.
    #[test]
    fn a_row_with_nothing_in_it_is_no_statement() {
        let row = Row {
            table: table(creature::SPAWN),
            key: Key::one("guid", 1),
            life: Life::Update,
            changes: Vec::new(),
            at: Key::one("guid", 1),
        };
        assert!(row.statements().is_empty());
    }

    /// A removed spawn is six statements, and the row itself is the last of
    /// them. See `vale_mangos::creature::delete_statements`.
    #[test]
    fn a_removed_spawn_is_the_row_and_its_five_dependents() {
        let row = Row {
            table: table(creature::SPAWN),
            key: Key::one("guid", 19272),
            life: Life::Delete,
            changes: Vec::new(),
            at: Key::one("guid", 19272),
        };
        let sql = row.statements();
        assert_eq!(sql.len(), 6);
        assert!(
            sql.iter()
                .all(|statement| statement.starts_with("DELETE FROM ")),
            "{sql:?}"
        );
        assert_eq!(
            sql.last().map(String::as_str),
            Some("DELETE FROM `creature` WHERE `guid` = 19272;")
        );
    }
}
