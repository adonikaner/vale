//! What a project changes about what creatures sell and teach, as SQL,
//! applied when a person asks, and live on a reload.
//!
//! ## Four tables, written as keyed rows
//!
//! `npc_vendor`, `npc_vendor_template`, `npc_trainer` and
//! `npc_trainer_template` are keyed rows in the project's store, on
//! [`super::loot`]'s terms: a created row is a `DELETE` and an `INSERT`
//! naming every column, an edited one an `UPDATE`, a removed one a `DELETE`,
//! and the statements that put each row back are read from the database
//! immediately before anything runs. The keys are `vale_mangos::vendor::key`
//! and `vale_mangos::trainer::key`.
//!
//! A created row is refused, with the loader's own reason, when the server
//! would skip it at load for a reason the row alone shows:
//! `vale_mangos::vendor::Ware::check` and `vale_mangos::trainer::Lesson::check`.
//! The checks that need the item table, the spell tables or the rest of the
//! list are the window's, which refuses to add such a row; see
//! `crate::subjects::services`.
//!
//! ## Live on a reload, removals included
//!
//! `.reload npc_vendor` re-reads both vendor tables and `.reload npc_trainer`
//! both trainer tables, and each loader clears its lists first. An apply asks a
//! running playtest for both, since the put-back before it may have written
//! any of the four tables. This is the one creature subject that needs no
//! restart.
//!
//! ```text
//! sql\services.sql          what this project does to the four tables
//! sql\services-revert.sql   what puts those rows back
//! server\rows.txt           the store both are written from, shared
//! ```

use super::stack::Subject;
use super::subject::RowSubject;
use vale_mangos::row;
use vale_mangos::{trainer, vendor};

/// What this project does to the four tables, as SQL.
pub const SQL_VPATH: &str = "sql\\services.sql";

/// What puts it back.
pub const REVERT_VPATH: &str = "sql\\services-revert.sql";

/// The four tables, in the order a plan writes them.
pub const TABLES: &[row::Table] = &[
    vendor::TEMPLATE_TABLE,
    vendor::VENDOR_TABLE,
    trainer::TEMPLATE_TABLE,
    trainer::TRAINER_TABLE,
];

/// The two reload commands that re-read them, in the order they are sent.
/// Each re-reads its template table first.
pub const RELOADS: [&str; 2] = [vendor::VENDOR, trainer::TRAINER];

/// What creatures sell and teach.
///
/// A created row is refused when its key is already there, which is the
/// module's default: the vendor tables' primary key is `(entry, item)`, so a
/// second row of one item in one list is refused by MySQL too.
pub struct Services;

impl RowSubject for Services {
    fn subject(&self) -> Subject {
        Subject::Services
    }

    fn name(&self) -> &'static str {
        "vendors and trainers"
    }

    fn label(&self) -> &'static str {
        "Vendors and trainers"
    }

    fn word(&self) -> &'static str {
        "services"
    }

    fn noun(&self) -> &'static str {
        "vendor and trainer"
    }

    fn tables(&self) -> &'static [row::Table] {
        TABLES
    }

    fn table_list(&self) -> &'static str {
        "npc_vendor, npc_vendor_template, npc_trainer, npc_trainer_template"
    }

    fn sql_vpath(&self) -> &'static str {
        SQL_VPATH
    }

    fn revert_vpath(&self) -> &'static str {
        REVERT_VPATH
    }

    fn sql_about(&self) -> &'static str {
        "what creatures sell and teach"
    }

    fn sql_note(&self) -> String {
        "-- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then reload both lists:\n\
         --   .reload npc_vendor     (re-reads npc_vendor_template, then npc_vendor)\n\
         --   .reload npc_trainer    (re-reads npc_trainer_template, then npc_trainer)\n\
         -- Both loaders clear their lists first, so removals are live too."
            .to_string()
    }

    fn going_live(&self) -> &'static str {
        "Live on `.reload npc_vendor` and `.reload npc_trainer`, which an apply sends \
         when it is made with the panels open over a playtest \u{2014} removals \
         included, since each re-reads its template table and its own and clears \
         both lists first. Applied at any other time, it is live after a restart. \
         The client asks for a \
         list each time the window opens, so no relog is needed. A vendor's current \
         count of a limited item is not reset."
    }

    /// Both, since the put-back before an Apply may have written any of the
    /// four tables.
    fn reloads(&self) -> Vec<&'static str> {
        RELOADS.to_vec()
    }

    /// The loader's own checks of a created ware or lesson.
    fn faults(&self, table: &'static row::Table, row: &vale_mangos::schema::Row) -> Vec<String> {
        match row::table_named(vendor::TABLES, table.name) {
            Some(_) => vendor::Ware::from_row(row).map(|ware| ware.check()).unwrap_or_default(),
            None => trainer::Lesson::from_row(row).map(|lesson| lesson.check()).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::subject::{self, Plan};
    use vale_mangos::row::{Assignment, Edits, Key, Life, RowEdit};

    fn plan_from(edits: &Edits) -> Plan {
        subject::plan_of(&Services, edits)
    }

    fn owns(table: &str) -> bool {
        Services.owns(table)
    }

    fn created(table: &str, key: &Key, changes: Vec<Assignment>, edits: &mut Edits) {
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in changes {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(table, key, Some(&row.to_line()));
    }

    /// A created ware and lesson are each a `DELETE` and an `INSERT`, an edit
    /// one `UPDATE`, and the tables are written in [`TABLES`]' order.
    #[test]
    fn each_table_is_written_as_keyed_rows() {
        let mut edits = Edits::default();
        let lesson = trainer::Lesson::new(328, 1173);
        created(trainer::TRAINER, &lesson.key(), lesson.assignments(), &mut edits);
        let ware = vendor::Ware::new(54, 2488, 9);
        created(vendor::VENDOR, &ware.key(), ware.assignments(), &mut edits);
        edits.set(vendor::TEMPLATE, &vendor::key(1279501, 5565), "slot", Some("3".into()));
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (2, 1, 0));
        let sql = plan.statements();
        assert_eq!(sql.len(), 5);
        assert!(sql[0].starts_with("UPDATE `npc_vendor_template`"), "{}", sql[0]);
        assert!(sql[2].starts_with("INSERT INTO `npc_vendor`"), "{}", sql[2]);
        assert!(sql[4].starts_with("INSERT INTO `npc_trainer`"), "{}", sql[4]);
    }

    /// A created row the loader would skip is refused with its reason, one
    /// missing a column by name, and a key column among the columns by name.
    #[test]
    fn a_row_the_server_would_skip_is_refused() {
        let mut edits = Edits::default();
        let ware = vendor::Ware { maxcount: 5, ..vendor::Ware::new(54, 2488, 9) };
        created(vendor::VENDOR, &ware.key(), ware.assignments(), &mut edits);
        let plan = plan_from(&edits);
        assert!(plan.rows.is_empty());
        assert!(plan.refused[0].contains("incrtime is 0"), "{:?}", plan.refused);

        let mut edits = Edits::default();
        let lesson = trainer::Lesson::new(328, 1173);
        let mut changes = lesson.assignments();
        changes.retain(|change| change.column != "reqlevel");
        created(trainer::TRAINER, &lesson.key(), changes, &mut edits);
        let plan = plan_from(&edits);
        assert!(plan.refused[0].contains("without reqlevel"), "{:?}", plan.refused);

        let mut edits = Edits::default();
        edits.set(vendor::VENDOR, &vendor::key(54, 2488), "item", Some("2489".into()));
        assert!(plan_from(&edits).refused[0].contains("part of the key"));
    }

    /// The other writers ignore these rows, and this one ignores theirs.
    #[test]
    fn each_writer_ignores_the_other_subjects_rows() {
        let mut edits = Edits::default();
        edits.set(vendor::VENDOR, &vendor::key(54, 2488), "maxcount", Some("0".into()));
        edits.set(
            vale_mangos::loot::CREATURE,
            &vale_mangos::loot::Entry::item(68, 2589).key(),
            "maxcount",
            Some("3".into()),
        );
        let services = plan_from(&edits);
        assert_eq!(services.rows.len(), 1);
        assert!(services.refused.is_empty(), "{:?}", services.refused);
        let loot = subject::plan_of(&super::super::loot::Loot, &edits);
        assert_eq!(loot.rows.len(), 1);
        assert!(loot.refused.is_empty(), "{:?}", loot.refused);
        let creatures = subject::plan_of(&super::super::creatures::Creatures, &edits);
        assert!(creatures.refused.is_empty(), "{:?}", creatures.refused);
    }

    #[test]
    fn the_subject_owns_the_four_tables() {
        for table in TABLES {
            assert!(owns(table.name));
        }
        assert!(!owns(vale_mangos::creature::TEMPLATE));
    }
}
