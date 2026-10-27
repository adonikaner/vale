//! What a project changes about which spawns act together, as SQL, applied
//! when a person asks: a formation live on a reload, a link after a restart.
//!
//! ## Three tables, written as keyed rows
//!
//! `creature_groups`, `creature_linking` and `creature_linking_template` are
//! keyed rows in the project's store, on [`super::addons`]' terms. The keys
//! are `vale_mangos::formation::member_key`, `link_key` and
//! `template_link_key`. A created row is refused with the loader's reason
//! when the row alone shows the server would skip it
//! (`vale_mangos::formation::Link::check` and the others); whether both
//! spawns exist and share a map is the inspector's to mark.
//!
//! ## Live on a reload, and after a restart
//!
//! `.reload creature_groups` re-reads the formations. The two linking tables
//! are read at startup only, so an apply that writes them says to restart.
//!
//! ```text
//! sql\formations.sql          what this project does to the three tables
//! sql\formations-revert.sql   what puts those rows back
//! server\rows.txt             the store all three are written from, shared
//! ```

use super::stack::Subject;
use super::subject::{Plan, RowSubject};
use vale_mangos::formation;
use vale_mangos::row;

/// What this project does to the three tables, as SQL.
pub(crate) const SQL_VPATH: &str = "sql\\formations.sql";

/// What puts it back.
pub(crate) const REVERT_VPATH: &str = "sql\\formations-revert.sql";

/// The three tables.
pub(crate) const TABLES: &[row::Table] = formation::TABLES;

/// Which spawns walk, fight and respawn together.
pub(crate) struct Formations;

impl RowSubject for Formations {
    fn subject(&self) -> Subject {
        Subject::Formations
    }

    fn name(&self) -> &'static str {
        "creature formations and links"
    }

    fn label(&self) -> &'static str {
        "Formations and links"
    }

    fn word(&self) -> &'static str {
        "formations"
    }

    fn noun(&self) -> &'static str {
        "formation or link"
    }

    fn tables(&self) -> &'static [row::Table] {
        TABLES
    }

    fn table_list(&self) -> &'static str {
        "creature_groups, creature_linking, creature_linking_template"
    }

    fn sql_vpath(&self) -> &'static str {
        SQL_VPATH
    }

    fn revert_vpath(&self) -> &'static str {
        REVERT_VPATH
    }

    fn sql_about(&self) -> &'static str {
        "which of its spawns walk, fight and respawn together"
    }

    fn sql_note(&self) -> String {
        "-- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand: .reload creature_groups re-reads the formations, and\n\
         -- creature_linking and creature_linking_template are read at startup only."
            .to_string()
    }

    fn going_live(&self) -> &'static str {
        "Formations are live on .reload creature_groups. Links are read at startup and \
         have no .reload, so an applied link changes nothing until the server starts again."
    }

    fn reloads(&self) -> Vec<&'static str> {
        vec![formation::GROUPS]
    }

    /// The reload only when the plan writes a formation: a plan of links
    /// alone needs a restart, which [`Self::restart_note`] says.
    fn reloads_for(&self, plan: &Plan) -> Vec<&'static str> {
        match plan.rows.iter().any(|row| row.table.name == formation::GROUPS) {
            true => self.reloads(),
            false => Vec::new(),
        }
    }

    fn restart_note(&self) -> &'static str {
        "restart the server to load the links; creature_linking has no reload"
    }

    /// The loader's own checks of a created row.
    fn faults(&self, table: &'static row::Table, row: &vale_mangos::schema::Row) -> Vec<String> {
        match table.name {
            formation::GROUPS => formation::Member::from_row(row).map(|member| member.check()).unwrap_or_default(),
            formation::LINKING => formation::Link::from_row(row).map(|link| link.check()).unwrap_or_default(),
            _ => formation::TemplateLink::from_row(row).map(|link| link.check()).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::subject;
    use vale_mangos::formation::{Link, Member};
    use vale_mangos::row::{Assignment, Edits, Key, Life, RowEdit};

    fn plan_from(edits: &Edits) -> Plan {
        subject::plan_of(&Formations, edits)
    }

    fn created(table: &str, key: &Key, changes: Vec<Assignment>, edits: &mut Edits) {
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in changes {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(table, key, Some(&row.to_line()));
    }

    /// A created member is a `DELETE` and an `INSERT`, an edited link an
    /// `UPDATE`, and only a plan that writes a formation asks for its reload.
    #[test]
    fn each_table_is_written_as_keyed_rows() {
        let mut edits = Edits::default();
        let member = Member::new(51427, 51430, 3.0, 1.5);
        created(formation::GROUPS, &member.key(), member.assignments(), &mut edits);
        edits.set(formation::LINKING, &formation::link_key(80001), "flag", Some("1025".into()));
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        let sql = plan.statements();
        assert!(sql[0].starts_with("DELETE FROM `creature_groups` WHERE `member_guid` = 51430"), "{}", sql[0]);
        assert!(sql[1].starts_with("INSERT INTO `creature_groups`"), "{}", sql[1]);
        assert!(sql[2].starts_with("UPDATE `creature_linking` SET `flag` = 1025"), "{}", sql[2]);
        assert_eq!(Formations.reloads_for(&plan), vec!["creature_groups"]);

        let mut links = Edits::default();
        edits_link(&mut links);
        assert!(Formations.reloads_for(&plan_from(&links)).is_empty());
    }

    fn edits_link(edits: &mut Edits) {
        let link = Link::new(80001, 80000);
        created(formation::LINKING, &link.key(), link.assignments(), edits);
    }

    /// A created link the loader would skip is refused with its reason.
    #[test]
    fn a_link_the_server_skips_is_refused() {
        let mut edits = Edits::default();
        let link = Link { flag: 0, ..Link::new(80001, 80000) };
        created(formation::LINKING, &link.key(), link.assignments(), &mut edits);
        let plan = plan_from(&edits);
        assert!(plan.rows.is_empty());
        assert!(plan.refused[0].contains("flag is 0"), "{:?}", plan.refused);
        for table in TABLES {
            assert!(Formations.owns(table.name));
        }
        assert!(!Formations.owns(vale_mangos::addon::ADDON));
    }
}
