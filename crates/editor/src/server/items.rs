//! The server item rows a project changes, written as SQL and applied when the
//! user asks. Unlike a creature change, an item change takes effect on a
//! reload.
//!
//! ## Storage and apply follow [`super::creatures`]; the reload differs
//!
//! An edit is stored, batched, written and undone as in that module: the
//! project's own store accumulates typed values, a save writes the SQL, nothing
//! reaches the database until Apply is pressed, and the statement that puts
//! each row back is written from the database immediately before anything
//! runs.
//!
//! The difference is what happens after the apply. A creature change needs
//! the server restarted, because `.reload creature_template` does not restat
//! creatures that are already spawned and `.reload creature` never erases a
//! spawn it has read. An item change needs nothing:
//! `HandleReloadItemTemplate` calls `ObjectMgr::LoadItemPrototypes`, which
//! clears the prototype map before it reads (`ObjectMgr.cpp:3792`), and
//! `Item::GetProto` is a lookup by entry on every call (`Item.cpp:560`) rather
//! than a pointer taken when the item was made. So an edited row is live for
//! every copy of that item in the world, in a bag and on the auction house,
//! with no restart and no relog.
//!
//! Items are therefore the first subject in this editor that can be edited
//! while a playtest is running and seen without leaving it. Spells already
//! could, through a different route (a DBC the client re-reads). Creatures
//! cannot.
//!
//! An item change is applied from the Server panel, or by a save when the
//! switch is on, through [`crate::server::save`] and [`crate::server::sync`]. Every
//! other subject here uses the same two entry points.
//!
//! [`apply`] asks for that reload itself. The request is kept when the apply
//! was asked for with the panels open over a playtest, and dropped otherwise.
//! Every other apply in this directory does the same. See
//! [`super::reload::Reloads::when_there_is_a_session`].
//!
//! ## Files this module writes
//!
//! ```text
//! sql\items.sql         what this project does to the server's items
//! sql\items-revert.sql  the statements that put those rows back
//! server\rows.txt       the store both are written from, shared
//! ```
//!
//! The store is shared with the creature half because it is one file of every
//! server row a project changes, keyed by table. The two SQL files are separate
//! because the two subjects are applied by two separate actions, and a single
//! file would make the Apply for one subject apply the other as well. Each
//! writer skips the rows the other owns rather than refusing them; see
//! [`super::subject::plan_from`].
//!
//! ## A removal is applied without the reload
//!
//! An item can be removed, and the removal takes every content-patch version
//! and the rows that hand the item out — `vale_mangos::item::DEPENDENTS`.
//! What it cannot have is the reload: an `Item` already loaded in the world
//! whose prototype has gone answers `nullptr` from `GetProto()`, and its
//! callers dereference the result without a null check. A restart is safe,
//! because each loader of a character's items deletes one with no prototype.
//! So an apply whose plan
//! removes anything asks for no reload and says the server has to be
//! restarted — see `vale_mangos::item::reload_is_safe`. That holds for every
//! apply while the removal is in the plan, since any reload after it is the
//! same reload.

use super::stack::Subject;
use super::subject::{self, Plan, Row, RowSubject};
use vale_mangos::conn::Db;
use vale_mangos::item;
use vale_mangos::row::{self, Edits};

/// The project file holding the project's item changes as SQL.
pub const SQL_VPATH: &str = "sql\\items.sql";

/// The project file holding the statements that put the changed rows back.
pub const REVERT_VPATH: &str = "sql\\items-revert.sql";

/// The table `.reload` is asked for after an apply.
pub const RELOAD_TABLE: &str = item::TEMPLATE;

/// The item subject: `item_template`.
pub struct Items;

impl RowSubject for Items {
    fn subject(&self) -> Subject {
        Subject::Items
    }

    fn name(&self) -> &'static str {
        "items"
    }

    fn label(&self) -> &'static str {
        "Items"
    }

    fn word(&self) -> &'static str {
        "items"
    }

    fn noun(&self) -> &'static str {
        "item"
    }

    fn tables(&self) -> &'static [row::Table] {
        item::TABLES
    }

    fn table_list(&self) -> &'static str {
        "item_template"
    }

    fn sql_vpath(&self) -> &'static str {
        SQL_VPATH
    }

    fn revert_vpath(&self) -> &'static str {
        REVERT_VPATH
    }

    fn sql_about(&self) -> &'static str {
        "the server's items"
    }

    fn sql_note(&self) -> String {
        "-- Nothing applies this by itself. Apply it from the editor's item\n\
         -- workspace, or run it by hand and then say `.reload item_template`:\n\
         -- LoadItemPrototypes clears its map before it reads, and Item::GetProto\n\
         -- is a lookup per call, so an item change is live with no restart."
            .to_string()
    }

    fn going_live(&self) -> &'static str {
        "Live on `.reload item_template`, which an apply sends when it is made \
         with the panels open over a playtest \u{2014} for every copy of the item \
         already in a bag, in the world and on the auction house, with no relog. \
         Applied at any other time, it is live after a restart. A removed item \
         needs a restart, and the apply sends no reload while one is in the \
         plan: a reload would leave every copy already loaded with no \
         prototype, which the server does not check for."
    }

    fn reloads(&self) -> Vec<&'static str> {
        vec![RELOAD_TABLE]
    }

    /// No reload while the plan removes an item; see the module comment and
    /// `vale_mangos::item::reload_is_safe`.
    fn reloads_for(&self, plan: &Plan) -> Vec<&'static str> {
        match item::reload_is_safe(plan.rows.iter().map(|row| &row.life)) {
            true => self.reloads(),
            false => Vec::new(),
        }
    }

    fn restart_note(&self) -> &'static str {
        "restart the server: .reload item_template does not apply an item removal"
    }

    fn refuse_arrival(&self, db: &mut Db, row: &Row, own: &Edits) -> Result<(), String> {
        subject::refuse_a_taken_id(db, row, own, None)
    }

    /// Everything a removed item takes with it, as the statements that put it
    /// back: every content-patch version of the template, then the rows of
    /// each of `item::DEPENDENTS`. `None` when the item is not in the
    /// database. The `DELETE`s come first, so the entry can be run against an
    /// item it has already restored; see [`super::reconcile`].
    fn undo_of_a_removal(&self, db: &mut Db, row: &Row) -> Result<Option<Vec<String>>, String> {
        let Some(entry) = row.key.first() else {
            return Ok(None);
        };
        let versions = db.rows(&row::rows_where(item::TEMPLATE, "entry", entry))?;
        if versions.is_empty() {
            return Ok(None);
        }
        let mut out = item::delete_statements(entry);
        for held in &versions {
            out.extend(row::insert_from_row(item::TEMPLATE, held));
        }
        for dependent in &item::DEPENDENTS {
            for held in db.rows(&dependent.select(entry))? {
                out.extend(row::insert_from_row(dependent.table, &held));
            }
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_mangos::row::{Key, Life, RowEdit};

    fn plan_from(edits: &Edits) -> Plan {
        subject::plan_of(&Items, edits)
    }

    fn store_one(key: &Key, row: &RowEdit) -> Edits {
        let mut edits = Edits::default();
        edits.set_row_line(item::TEMPLATE, key, Some(&row.to_line()));
        edits
    }

    fn a_new_item(entry: u32) -> (Key, RowEdit) {
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in item::new_item("Test Item") {
            row.columns.insert(change.column.to_string(), change.value);
        }
        (item::template_key(entry, 0), row)
    }

    /// One row is one statement, naming every column it changes and the whole
    /// key: the entry and the patch. Naming the patch stops an edit from
    /// changing every content-patch version of the item.
    #[test]
    fn an_edit_is_one_update_naming_the_whole_key() {
        let mut edits = Edits::default();
        let key = item::template_key(2589, 0);
        edits.set(item::TEMPLATE, &key, "quality", Some("3".to_string()));
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (0, 1, 0));
        assert_eq!(
            plan.statements(),
            vec!["UPDATE `item_template` SET `quality` = 3 WHERE `entry` = 2589 AND `patch` = 0;"]
        );
    }

    /// A created item is a `DELETE` of its key and an `INSERT` naming every
    /// column, so applying twice has the same effect as applying once.
    #[test]
    fn a_created_item_is_a_delete_and_an_insert() {
        let (key, row) = a_new_item(2_000_000);
        let plan = plan_from(&store_one(&key, &row));
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (1, 0, 0));
        let sql = plan.statements();
        assert_eq!(sql.len(), 2);
        assert!(
            sql[0].starts_with("DELETE FROM `item_template` WHERE `entry` = 2000000"),
            "{}",
            sql[0]
        );
        assert!(
            sql[1].starts_with("INSERT INTO `item_template` (`entry`, `patch`,"),
            "{}",
            sql[1]
        );
    }

    /// A creation that does not name every column is not written, and the
    /// refusal names the column rather than leaving a row nobody chose.
    #[test]
    fn a_created_item_missing_a_column_is_refused() {
        let (key, mut row) = a_new_item(2_000_000);
        row.columns.remove("stackable");
        let plan = plan_from(&store_one(&key, &row));
        assert!(plan.rows.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(plan.refused[0].contains("stackable"), "{:?}", plan.refused);
    }

    /// A removal is a row of the plan and forbids the reload. See the module
    /// comment and `Item::GetProto`.
    #[test]
    fn a_removal_is_written_and_asks_for_a_restart() {
        let mut edits = Edits::default();
        let key = item::template_key(2589, 0);
        edits.set_life(item::TEMPLATE, &key, Life::Delete);
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (0, 0, 1));
        assert!(Items.reloads_for(&plan).is_empty());
        let sql = plan.statements();
        assert_eq!(sql[0], "DELETE FROM `item_template` WHERE `entry` = 2589;");
        assert_eq!(sql.len(), 1 + item::DEPENDENTS.len());
        assert_eq!(plan.line(), "1 row(s): 1 removed");
    }

    /// An item whose entry changes is one claim at the new entry, and it
    /// becomes the move of the row, of every other content-patch version of it,
    /// and of every column that names it.
    #[test]
    fn moving_an_entry_takes_every_version_and_every_reference() {
        let mut edits = Edits::default();
        let key = item::template_key(2589, 10);
        edits.set(item::TEMPLATE, &key, "quality", Some("3".to_string()));
        assert!(edits.rekey(
            item::TEMPLATE,
            &key,
            &item::template_key(2_000_001, 10),
            None
        ));
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.rows.len(), 1);
        assert!(plan.rows[0].moves());
        let sql = plan.statements();
        assert_eq!(
            sql[0],
            "UPDATE `item_template` SET `entry` = 2000001, `quality` = 3 WHERE `entry` = 2589 \
             AND `patch` = 10;"
        );
        assert_eq!(
            sql[1],
            "UPDATE `item_template` SET `entry` = 2000001 WHERE `entry` = 2589;"
        );
        assert_eq!(sql.len(), 2 + item::REFERENCES.len());
        assert!(sql.contains(
            &"UPDATE `npc_vendor` SET `item` = 2000001 WHERE `item` = 2589;".to_string()
        ));
    }

    /// A move with no column edit beside it is still a row of the plan: the
    /// claim says only where the row goes.
    #[test]
    fn a_move_alone_is_a_row_of_the_plan() {
        let mut edits = Edits::default();
        let key = item::template_key(2589, 10);
        assert!(edits.rekey(
            item::TEMPLATE,
            &key,
            &item::template_key(2_000_001, 10),
            None
        ));
        assert_eq!(plan_from(&edits).counts(), (0, 1, 0));
    }

    /// A created row that is re-keyed is created at the new entry and nowhere
    /// else. Before this was fixed, the old entry stayed in the database
    /// because the store no longer named it and nothing took it back. This
    /// test covers the store. `super::reconcile` takes the old row back,
    /// because it puts back everything before it applies.
    #[test]
    fn a_created_row_re_keyed_is_created_once() {
        let (key, row) = a_new_item(2_000_000);
        let mut edits = store_one(&key, &row);
        assert!(edits.rekey(item::TEMPLATE, &key, &item::template_key(505_056, 0), None));
        let plan = plan_from(&edits);
        assert_eq!(plan.counts(), (1, 0, 0));
        let sql = plan.statements();
        assert_eq!(sql.len(), 2);
        assert!(sql[1].contains("(505056, 0,"), "{}", sql[1]);
        assert!(
            sql.iter().all(|statement| !statement.contains("2000000")),
            "{sql:?}"
        );
    }

    /// A key column typed into the store by hand is refused by name.
    #[test]
    fn a_key_column_is_never_a_column_of_an_edit() {
        let (key, mut row) = a_new_item(2_000_000);
        row.columns
            .insert("entry".to_string(), "2000001".to_string());
        let plan = plan_from(&store_one(&key, &row));
        assert_eq!(plan.counts(), (1, 0, 0));
        assert_eq!(plan.refused.len(), 1);
        assert!(
            plan.refused[0].contains("part of the key"),
            "{:?}",
            plan.refused
        );
    }

    /// The two halves of the store do not complain about each other. One
    /// file holds every server row a project changes and each writer walks all
    /// of it; a creature row here and an item row there must each be skipped in
    /// silence, or every project that edits both reports a refusal it cannot
    /// act on.
    #[test]
    fn each_writer_ignores_the_other_subjects_rows() {
        let mut edits = Edits::default();
        let creature = vale_mangos::creature::spawn_key(19272);
        edits.set(
            vale_mangos::creature::SPAWN,
            &creature,
            "position_z",
            Some("98.7".to_string()),
        );
        let item_key = item::template_key(2589, 0);
        edits.set(item::TEMPLATE, &item_key, "quality", Some("3".to_string()));

        let items = plan_from(&edits);
        assert_eq!(items.rows.len(), 1, "only the item");
        assert!(items.refused.is_empty(), "{:?}", items.refused);

        let creatures = subject::plan_of(&super::super::creatures::Creatures, &edits);
        assert_eq!(creatures.rows.len(), 1, "only the creature");
        assert!(creatures.refused.is_empty(), "{:?}", creatures.refused);
    }

    /// A column this editor has no schema for is refused by name: the store is
    /// a text file somebody may have edited, and an edit that reaches no
    /// statement is one that will never happen.
    #[test]
    fn an_unknown_column_is_refused_by_name() {
        let mut edits = Edits::default();
        let key = item::template_key(2589, 0);
        edits.set(item::TEMPLATE, &key, "not_a_column", Some("1".to_string()));
        let plan = plan_from(&edits);
        assert!(plan.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(
            plan.refused[0].contains("not_a_column"),
            "{:?}",
            plan.refused
        );
    }

    /// Two plans that say the same thing have the same signature, and one more
    /// edit is a different database.
    #[test]
    fn the_signature_is_of_the_statements() {
        let mut edits = Edits::default();
        let key = item::template_key(2589, 0);
        edits.set(item::TEMPLATE, &key, "quality", Some("3".to_string()));
        let first = plan_from(&edits).signature();
        assert_eq!(first, plan_from(&edits).signature());
        edits.set(item::TEMPLATE, &key, "item_level", Some("60".to_string()));
        assert_ne!(first, plan_from(&edits).signature());
    }
}
