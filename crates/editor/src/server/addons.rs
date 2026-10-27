//! What a project changes about how a spawn stands and what a creature
//! holds, as SQL, applied when a person asks, and live after a restart.
//!
//! ## Two tables, written as keyed rows
//!
//! `creature_addon` and `creature_equip_template` are keyed rows in the
//! project's store, on [`super::services`]' terms: a created row is a
//! `DELETE` and an `INSERT` naming every column, an edited one an `UPDATE`,
//! a removed one a `DELETE`, and the statements that put each row back are
//! read from the database immediately before anything runs. The keys are
//! `vale_mangos::addon::addon_key` and `vale_mangos::addon::equipment_key`.
//!
//! A created row is refused, with the loader's own reason, when the server
//! would put it back to a default for a reason the row alone shows:
//! `vale_mangos::addon::Addon::check` and `Equipment::check`. The checks that
//! need the item rows or the display tables are the window's and the
//! section's; see `crate::subjects::addons`.
//!
//! ## Live after a restart
//!
//! Neither table has a `.reload`: `LoadCreatureAddons` and
//! `LoadEquipmentTemplates` run at startup only. An apply says so on the
//! status line, as the creature subject's does.
//!
//! ```text
//! sql\addons.sql          what this project does to the two tables
//! sql\addons-revert.sql   what puts those rows back
//! server\rows.txt         the store both are written from, shared
//! ```

use super::stack::Subject;
use super::subject::RowSubject;
use vale_mangos::addon;
use vale_mangos::row;

/// What this project does to the two tables, as SQL.
pub(crate) const SQL_VPATH: &str = "sql\\addons.sql";

/// What puts it back.
pub(crate) const REVERT_VPATH: &str = "sql\\addons-revert.sql";

/// The two tables, in the order a plan writes them: a set before the addon
/// rows that name it.
pub(crate) const TABLES: &[row::Table] = addon::TABLES;

/// How a spawn stands and what a creature holds.
///
/// A created row is refused when its key is already there, which is the
/// module's default: both tables' keys are their primary keys.
pub(crate) struct Addons;

impl RowSubject for Addons {
    fn subject(&self) -> Subject {
        Subject::Addons
    }

    fn name(&self) -> &'static str {
        "creature addons and equipment"
    }

    fn label(&self) -> &'static str {
        "Addons and equipment"
    }

    fn word(&self) -> &'static str {
        "addons"
    }

    fn noun(&self) -> &'static str {
        "addon and equipment"
    }

    fn tables(&self) -> &'static [row::Table] {
        TABLES
    }

    fn table_list(&self) -> &'static str {
        "creature_addon, creature_equip_template"
    }

    fn sql_vpath(&self) -> &'static str {
        SQL_VPATH
    }

    fn revert_vpath(&self) -> &'static str {
        REVERT_VPATH
    }

    fn sql_about(&self) -> &'static str {
        "how its spawns stand and what its creatures hold"
    }

    fn sql_note(&self) -> String {
        "-- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then restart the server: creature_addon and\n\
         -- creature_equip_template are read at startup and have no .reload."
            .to_string()
    }

    fn going_live(&self) -> &'static str {
        "Live after a restart. The server reads creature_addon and creature_equip_template \
         at startup and neither has a .reload, so an applied row changes nothing until the \
         server starts again. A spawn that respawns before then keeps its old stance and \
         weapons."
    }

    /// None: both tables are read at startup only.
    fn reloads(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn restart_note(&self) -> &'static str {
        "restart the server to load them; neither table has a reload"
    }

    /// The loader's own checks of a created addon or equipment row.
    fn faults(&self, table: &'static row::Table, row: &vale_mangos::schema::Row) -> Vec<String> {
        match table.name {
            addon::ADDON => addon::Addon::from_row(row).map(|addon| addon.check()).unwrap_or_default(),
            _ => addon::Equipment::from_row(row).map(|set| set.check()).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::subject::{self, Plan};
    use vale_mangos::addon::{Addon, Equipment};
    use vale_mangos::row::{Assignment, Edits, Key, Life, RowEdit};

    fn plan_from(edits: &Edits) -> Plan {
        subject::plan_of(&Addons, edits)
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

    /// A created set row and addon are each a `DELETE` and an `INSERT`, an
    /// edit one `UPDATE`, and the set is written before the addon that names
    /// it.
    #[test]
    fn each_table_is_written_as_keyed_rows() {
        let mut edits = Edits::default();
        let addon = Addon { equipment_id: 2000123, ..Addon::new(4805, 0) };
        created(addon::ADDON, &addon.key(), addon.assignments(), &mut edits);
        let set = Equipment::new(2000123, [1904, 0, 0]);
        created(addon::EQUIPMENT, &set.key(), set.assignments(), &mut edits);
        edits.set(addon::ADDON, &addon::addon_key(4806, 0), "stand_state", Some("1".into()));
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (2, 1, 0));
        let sql = plan.statements();
        assert_eq!(sql.len(), 5);
        assert!(sql[0].starts_with("DELETE FROM `creature_equip_template`"), "{}", sql[0]);
        assert!(sql[1].starts_with("INSERT INTO `creature_equip_template`"), "{}", sql[1]);
        assert!(sql[3].starts_with("INSERT INTO `creature_addon`"), "{}", sql[3]);
        assert!(sql[4].starts_with("UPDATE `creature_addon` SET `stand_state` = 1"), "{}", sql[4]);
        assert!(Addons.reloads().is_empty());
    }

    /// A created row the loader would put back to a default is refused with
    /// its reason, and a key column among the columns by name.
    #[test]
    fn a_row_the_server_would_reset_is_refused() {
        let mut edits = Edits::default();
        let addon = Addon { stand_state: 12, ..Addon::new(4805, 0) };
        created(addon::ADDON, &addon.key(), addon.assignments(), &mut edits);
        let plan = plan_from(&edits);
        assert!(plan.rows.is_empty());
        assert!(plan.refused[0].contains("stand_state 12"), "{:?}", plan.refused);

        let mut edits = Edits::default();
        let set = Equipment { patch_min: 9, patch_max: 2, ..Equipment::new(62, [1904, 0, 0]) };
        created(addon::EQUIPMENT, &set.key(), set.assignments(), &mut edits);
        assert!(plan_from(&edits).refused[0].contains("patch_min 9"));

        let mut edits = Edits::default();
        edits.set(addon::EQUIPMENT, &addon::equipment_key(62, [1904, 0, 0]), "item1", Some("1905".into()));
        assert!(plan_from(&edits).refused[0].contains("part of the key"));
    }

    /// The other writers ignore these rows, and this one ignores theirs.
    #[test]
    fn each_writer_ignores_the_other_subjects_rows() {
        let mut edits = Edits::default();
        edits.set(addon::ADDON, &addon::addon_key(4805, 0), "emote_state", Some("234".into()));
        edits.set(
            vale_mangos::vendor::VENDOR,
            &vale_mangos::vendor::key(54, 2488),
            "maxcount",
            Some("0".into()),
        );
        let addons = plan_from(&edits);
        assert_eq!(addons.rows.len(), 1);
        assert!(addons.refused.is_empty(), "{:?}", addons.refused);
        let services = subject::plan_of(&super::super::services::Services, &edits);
        assert_eq!(services.rows.len(), 1);
        assert!(services.refused.is_empty(), "{:?}", services.refused);
        for table in TABLES {
            assert!(Addons.owns(table.name));
        }
        assert!(!Addons.owns(vale_mangos::creature::SPAWN));
    }
}
