//! What a project changes about which spawns the server keeps a few of at a
//! time, as SQL, applied when a person asks, and live after a restart.
//!
//! ## Six tables, written as keyed rows
//!
//! The pool tables are keyed rows in the project's store, on
//! [`super::addons`]' terms. The keys are `vale_mangos::pool::pool_key`,
//! `member_key`, `child_key` and `entry_member_key`. A created row is
//! refused with the loader's reason when the row alone shows the server
//! would skip it (`vale_mangos::pool::Member::check`, `Pool::check`). The
//! rules that need the whole pool, the sum of its chances and its members
//! sharing a map, are the Pool window's to mark.
//!
//! ## Live after a restart
//!
//! `PoolManager::LoadFromDB` runs at startup only, and there is no
//! `.reload` for any of the six tables.
//!
//! ```text
//! sql\pools.sql          what this project does to the six tables
//! sql\pools-revert.sql   what puts those rows back
//! server\rows.txt        the store all six are written from, shared
//! ```

use super::stack::Subject;
use super::subject::RowSubject;
use vale_mangos::pool;
use vale_mangos::row;

/// What this project does to the six tables, as SQL.
pub(crate) const SQL_VPATH: &str = "sql\\pools.sql";

/// What puts it back.
pub(crate) const REVERT_VPATH: &str = "sql\\pools-revert.sql";

/// The six tables, a pool before its members.
pub(crate) const TABLES: &[row::Table] = pool::TABLES;

/// Which spawns the server keeps a few of at a time.
pub(crate) struct Pools;

impl RowSubject for Pools {
    fn subject(&self) -> Subject {
        Subject::Pools
    }

    fn name(&self) -> &'static str {
        "spawn pools"
    }

    fn label(&self) -> &'static str {
        "Pools"
    }

    fn word(&self) -> &'static str {
        "pools"
    }

    fn noun(&self) -> &'static str {
        "pool row"
    }

    fn tables(&self) -> &'static [row::Table] {
        TABLES
    }

    fn table_list(&self) -> &'static str {
        "pool_template, pool_creature, pool_gameobject, pool_pool, pool_creature_template, \
         pool_gameobject_template"
    }

    fn sql_vpath(&self) -> &'static str {
        SQL_VPATH
    }

    fn revert_vpath(&self) -> &'static str {
        REVERT_VPATH
    }

    fn sql_about(&self) -> &'static str {
        "which of its spawns are kept a few at a time"
    }

    fn sql_note(&self) -> String {
        "-- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then restart the server: the pool tables are read\n\
         -- at startup and have no .reload."
            .to_string()
    }

    fn going_live(&self) -> &'static str {
        "Live after a restart. The server reads the pool tables at startup and none has a \
         .reload, so an applied pool changes nothing until the server starts again."
    }

    /// None: the pools are read at startup only.
    fn reloads(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn restart_note(&self) -> &'static str {
        "restart the server to load them; the pool tables have no reload"
    }

    /// The loader's own checks of a created pool or member.
    fn faults(&self, table: &'static row::Table, row: &vale_mangos::schema::Row) -> Vec<String> {
        match pool::Of::of_table(table.name) {
            Some(of) => pool::Member::from_row(of, row).map(|member| member.check()).unwrap_or_default(),
            None => pool::Pool::from_row(row).map(|pool| pool.check()).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::subject::{self, Plan};
    use vale_mangos::pool::{Member, Of, Pool};
    use vale_mangos::row::{Assignment, Edits, Key, Life, RowEdit};

    fn plan_from(edits: &Edits) -> Plan {
        subject::plan_of(&Pools, edits)
    }

    fn created(table: &str, key: &Key, changes: Vec<Assignment>, edits: &mut Edits) {
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in changes {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(table, key, Some(&row.to_line()));
    }

    /// A created pool is written before the member that names it, and a pool
    /// in a mother pool writes no patch columns.
    #[test]
    fn a_pool_is_written_before_its_members() {
        let mut edits = Edits::default();
        let member = Member::new(Of::GameObject, 5000, 9000, "Copper Vein");
        created(pool::GAMEOBJECT, &member.key(), member.assignments(), &mut edits);
        let new = Pool::new(9000, "Copper Veins - New Mine");
        created(pool::TEMPLATE, &new.key(), new.assignments(), &mut edits);
        let child = Member::new(Of::Pool, 9000, 9001, "the mine's veins");
        created(pool::POOL, &child.key(), child.assignments(), &mut edits);
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        let sql = plan.statements();
        assert!(sql[1].starts_with("INSERT INTO `pool_template`"), "{}", sql[1]);
        assert!(sql[3].starts_with("INSERT INTO `pool_gameobject`"), "{}", sql[3]);
        assert!(sql[5].starts_with("INSERT INTO `pool_pool`") && !sql[5].contains("patch_min"), "{}", sql[5]);
        assert!(Pools.reloads().is_empty());
    }

    /// A member the loader would skip is refused with its reason.
    #[test]
    fn a_member_the_server_skips_is_refused() {
        let mut edits = Edits::default();
        let member = Member { chance: 150.0, ..Member::new(Of::Creature, 80001, 9000, "x") };
        created(pool::CREATURE, &member.key(), member.assignments(), &mut edits);
        let plan = plan_from(&edits);
        assert!(plan.rows.is_empty());
        assert!(plan.refused[0].contains("chance 150"), "{:?}", plan.refused);
        for table in TABLES {
            assert!(Pools.owns(table.name));
        }
    }
}
