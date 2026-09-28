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
//! [`Life::Delete`] row is written, and its undo is the row as it stood.
//!
//! ## Removals before creations, inside a table
//!
//! A row moved between groups is a removal and a creation of the same
//! `(entry, item)` under two keys. On the five tables whose primary key is
//! `(entry, item)` alone, the `INSERT` fails while the old row stands, so
//! [`Plan::ordered`] writes every table's removals before its creations. Put
//! back runs the file newest first, which undoes them in the other order.
//!
//! ```text
//! sql\loot.sql          what this project does to the loot tables
//! sql\loot-revert.sql   the statements that put those rows back
//! server\rows.txt       the store both files are written from, shared
//! ```

use crate::session::EditSession;
use vale_mangos::conn::Db;
use vale_mangos::loot;
use vale_mangos::row::{self, Assignment, Key, Life};
use bevy::prelude::*;

pub use super::creatures::Undo;

/// What this project does to the server's loot, as SQL.
pub const SQL_VPATH: &str = "sql\\loot.sql";

/// The SQL that puts back the loot rows this project changed.
pub const REVERT_VPATH: &str = "sql\\loot-revert.sql";

/// One row's worth of change: what is to become of it, and the columns it sets.
#[derive(Debug, Clone)]
pub struct Row {
    pub table: &'static str,
    pub key: Key,
    pub life: Life,
    pub changes: Vec<Assignment>,
}

impl Row {
    /// The statements it becomes.
    pub fn statements(&self) -> Vec<String> {
        loot::statements(self.table, &self.key, self.life, &self.changes)
    }

    /// How it is named in a file's marker and in a status line.
    pub fn names(&self) -> String {
        format!("{} {}", self.table, self.key.text())
    }
}

/// Everything a project changes about the server's loot.
#[derive(Debug, Default)]
pub struct Plan {
    pub rows: Vec<Row>,
    /// What the store asked for and this writer would not write, each as a
    /// sentence — see [`super::creatures::Plan::refused`].
    pub refused: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// How many rows it creates, edits and removes.
    pub fn counts(&self) -> (usize, usize, usize) {
        let of = |life: Life| self.rows.iter().filter(|row| row.life == life).count();
        (of(Life::Insert), of(Life::Update), of(Life::Delete))
    }

    /// Every statement this plan comes to, in [`Self::ordered`]'s order.
    pub fn statements(&self) -> Vec<String> {
        self.ordered().flat_map(Row::statements).collect()
    }

    /// The rows in the order they are written: table by table in
    /// [`loot::TABLES`]' order, removals first inside a table — see the module
    /// comment — and by key after that.
    pub fn ordered(&self) -> impl Iterator<Item = &Row> {
        let mut rows: Vec<&Row> = self.rows.iter().collect();
        rows.sort_by_key(|row| {
            (
                loot::TABLES.iter().position(|table| *table == row.table),
                row.life != Life::Delete,
                row.key.clone(),
            )
        });
        rows.into_iter()
    }

    /// The tables it writes, in [`loot::TABLES`]' order.
    pub fn tables(&self) -> Vec<&'static str> {
        loot::TABLES
            .into_iter()
            .filter(|table| self.rows.iter().any(|row| row.table == *table))
            .collect()
    }

    /// What this plan would do to the database, as one number — see
    /// [`super::creatures::Plan::signature`].
    pub fn signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for statement in self.statements() {
            statement.hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// The loot rows of the project's store, as a plan of statements.
pub fn plan(session: &EditSession) -> Plan {
    plan_from(&session.server_edits)
}

/// [`plan`] over the store alone, so it can be tested without a session.
pub fn plan_from(edits: &vale_mangos::row::Edits) -> Plan {
    let mut out = Plan::default();
    for (table, key, row) in edits.rows() {
        // Another subject's row is skipped without a message, as every writer
        // here skips the others' rows. See `super::items::plan_from`.
        let Some(table) = loot::table_named(table) else {
            continue;
        };
        let mut changes = Vec::new();
        for (column, value) in &row.columns {
            let Some(known) = loot::column(table, column) else {
                out.refused.push(format!(
                    "{table}.{column} is not a column this editor writes"
                ));
                continue;
            };
            if !known.editable() {
                out.refused.push(format!(
                    "{table}.{column} is part of the key and is not written"
                ));
                continue;
            }
            changes.push(Assignment {
                column: known.name,
                value: value.clone(),
            });
        }
        match row.life {
            // A creation names every column or it is not written, on
            // `super::creatures`' own rule — and it is checked as the server
            // will check it, so a row the loader would skip is refused here
            // with the loader's own reason.
            Life::Insert => {
                let missing: Vec<&str> = loot::columns_of(table)
                    .iter()
                    .filter(|column| column.editable())
                    .map(|column| column.name)
                    .filter(|name| !changes.iter().any(|change| change.column == *name))
                    .collect();
                if !missing.is_empty() {
                    out.refused.push(format!(
                        "{table} {} is created without {} — it is not written",
                        key.text(),
                        missing.join(", ")
                    ));
                    continue;
                }
                let faults = entry_of(key, &changes).map(|entry| entry.check()).unwrap_or_default();
                if !faults.is_empty() {
                    out.refused.push(format!(
                        "{table} {} would be skipped by the server: {} — it is not written",
                        key.text(),
                        faults.join("; ")
                    ));
                    continue;
                }
            }
            Life::Delete => changes.clear(),
            Life::Update => {
                if changes.is_empty() {
                    continue;
                }
            }
        }
        out.rows.push(Row {
            table,
            key: key.clone(),
            life: row.life,
            changes,
        });
    }
    out
}

/// A created row, read back out of its key and columns for the check.
fn entry_of(key: &Key, changes: &[Assignment]) -> Option<loot::Entry> {
    let mut row = loot::Row::new();
    for (column, value) in &key.0 {
        row.insert(column.clone(), Some(value.clone()));
    }
    for change in changes {
        row.insert(change.column.to_string(), Some(change.value.clone()));
    }
    loot::Entry::from_row(&row)
}

/// Writes `sql\loot.sql`, or removes it when the project changes nothing.
pub fn write_sql(session: &mut EditSession) -> Result<usize, String> {
    let plan = plan(session);
    for refused in &plan.refused {
        warn!("{}: {refused}", super::creatures::EDITS_VPATH);
    }
    if plan.is_empty() {
        remove(&session.project, SQL_VPATH);
        return Ok(0);
    }
    let count = plan.rows.len();
    let mut body = format!(
        "-- {} — what this project changes about the server's loot.\n\
         -- Rewritten on every save from the project's own edits, so it is the\n\
         -- whole of what the project does rather than an increment of it.\n\
         --\n\
         -- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then reload each table it names:\n\
         --   .reload creature_loot_template       (and likewise the others)\n\
         -- Every loot loader clears its store first, so removals are live too.\n\n",
        session.project.name
    );
    for row in plan.ordered() {
        body.push_str(&format!("-- {}\n", row.names()));
        for statement in row.statements() {
            body.push_str(&statement);
            body.push('\n');
        }
        body.push('\n');
    }
    session
        .project
        .write(SQL_VPATH, body.as_bytes())
        .map_err(|e| format!("{SQL_VPATH}: {e}"))?;
    Ok(count)
}

/// The loot part of a save: writes the SQL for the store's loot rows. The
/// store itself is written by [`super::creatures::save`], on the same save.
pub fn save(session: &mut EditSession) {
    match write_sql(session) {
        Ok(0) => {}
        Ok(rows) => session.status = format!("{rows} loot change(s) written to {SQL_VPATH}"),
        Err(e) => {
            warn!("loot: {e}");
            session.status = format!("saved, but {SQL_VPATH} did not: {e}");
        }
    }
}

/// What an apply did.
#[derive(Debug, Default)]
pub struct Applied {
    pub rows: usize,
    pub affected: u64,
    pub newly_undoable: usize,
    /// How many rows the project had applied and no longer claims, which were
    /// put back and not written again — see [`super::reconcile`].
    pub taken_back: usize,
    /// The tables a reload was asked for, in order.
    pub tables: Vec<&'static str>,
}

impl Applied {
    pub fn line(&self) -> String {
        format!(
            "{} loot row(s) applied, {} affected — reloading {}",
            self.rows,
            self.affected,
            self.tables.join(", ")
        )
    }
}

/// An Apply, with everything it needs to run off the main thread. It is split
/// into the same three parts as [`super::items::ApplyJob`].
pub struct ApplyJob {
    plan: Plan,
    project: vale_edit::project::Project,
    at: vale_mangos::conn::Where,
}

/// The result of running an [`ApplyJob`].
pub struct ApplyDone {
    signature: u64,
    pub result: Result<Applied, String>,
}

impl ApplyDone {
    /// What the status line says about it.
    pub fn line(&self) -> String {
        match &self.result {
            Ok(done) if done.rows == 0 && done.taken_back == 0 => "nothing to apply".to_string(),
            Ok(done) => done.line(),
            Err(e) => format!("loot: {e}"),
        }
    }
}

/// The main thread's first half of an Apply. `None` when the project claims no
/// loot row and has applied none.
pub fn prepare_apply(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<ApplyJob>, String> {
    let plan = plan(session);
    if plan.is_empty() && !super::reconcile::has_applied(session, REVERT_VPATH) {
        return Ok(None);
    }
    let (at, _source) = server
        .resolve()
        .ok_or_else(vale_mangos::conn::Where::absent)?;
    Ok(Some(ApplyJob {
        plan,
        project: session.project.clone(),
        at,
    }))
}

impl ApplyJob {
    /// The worker's half, in [`super::reconcile`]'s order. What the project
    /// applied before is put back. Then each row is read as it stands, its
    /// undo is written, and its own statements run.
    pub fn run(self) -> ApplyDone {
        let signature = self.plan.signature();
        let result = (|| {
            let mut db = Db::open(&self.at)?;
            let rows: Vec<&Row> = self.plan.ordered().collect();
            let steps: Vec<super::reconcile::Step> = rows
                .iter()
                .map(|row| super::reconcile::Step {
                    table: row.table.to_string(),
                    key: row.key.clone(),
                    statements: row.statements(),
                })
                .collect();
            let done = super::reconcile::apply(
                &self.project,
                &mut db,
                REVERT_VPATH,
                "the server's loot",
                &steps,
                |db, index| undo_of_a_row(db, rows[index]),
            )?;
            Ok(Applied {
                rows: self.plan.rows.len(),
                affected: done.affected,
                newly_undoable: done.undoable,
                taken_back: done.taken_back,
                // All nine tables, not only the tables the plan writes, because
                // the put-back before it may have written any of them.
                tables: loot::TABLES.to_vec(),
            })
        })();
        ApplyDone { signature, result }
    }
}

/// The main thread's second half. The tables may have changed whether or not
/// the run finished, so every window's read is stale, and so is the server's
/// copy. A deferred reload of each table is requested.
pub fn finish_apply(
    session: &mut EditSession,
    reloads: &mut super::reload::Reloads,
    done: &ApplyDone,
) {
    session.wrote_the_database();
    session.applied_loot = done.result.as_ref().ok().map(|_| done.signature);
    for table in loot::TABLES {
        reloads.when_there_is_a_session(table);
    }
}

/// An Apply as one step of [`super::stack`], which is the only way an Apply
/// runs. `None` when there is nothing to apply.
pub fn apply_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_apply(session, server)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new("applying loot", move || {
        let done = job.run();
        let ok = done.result.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            finish_apply(session, reloads, &done);
            session.status = done.line();
        });
        (ok, finish)
    })))
}

/// One row's undo, read immediately before the row is written. This is the
/// loot part of [`super::reconcile`]'s step 3.
///
/// A created row is refused when the whole key is already there. A loot row's
/// key is five columns and none of them is an id of its own, so the check is
/// on the whole key, as it is for a quest relation. A table whose primary key
/// is `(entry, item)` alone refuses a second group of the same item itself,
/// and MySQL's error message is reported unchanged.
fn undo_of_a_row(db: &mut Db, row: &Row) -> Result<Option<Vec<String>>, String> {
    if row.life == Life::Insert && db.row(&loot::exists_query(row.table, &row.key))?.is_some() {
        return Err(format!(
            "{} {} is already in the database and is not this project's.",
            row.table,
            row.key.text()
        ));
    }
    Ok(match row.life {
        Life::Insert => Some(vec![row::delete(row.table, &row.key)]),
        // `None` when it is not in the database: nothing to put back. The
        // `DELETE` first, so the entry can be run twice — see
        // `super::reconcile`.
        Life::Delete => db.row(&loot::row_query(row.table, &row.key))?.map(|held| {
            let mut out = vec![row::delete(row.table, &row.key)];
            out.extend(row::insert_from_row(row.table, &held));
            out
        }),
        Life::Update => {
            let now = db.row(&loot::row_query(row.table, &row.key))?;
            row::undo(row.table, &row.key, &row.changes, now.as_ref()).map(|one| vec![one])
        }
    })
}

/// A Put back, with everything it needs to run off the main thread.
pub struct RevertJob {
    project: vale_edit::project::Project,
    at: vale_mangos::conn::Where,
}

/// The main thread's first half of a Put back: `None` when this project has
/// applied nothing.
pub fn prepare_revert(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<RevertJob>, String> {
    if !super::reconcile::has_applied(session, REVERT_VPATH) {
        return Ok(None);
    }
    let (at, _source) = server
        .resolve()
        .ok_or_else(vale_mangos::conn::Where::absent)?;
    Ok(Some(RevertJob {
        project: session.project.clone(),
        at,
    }))
}

impl RevertJob {
    /// The worker's half: run the revert file, and forget it.
    pub fn run(self) -> Result<usize, String> {
        let mut db = Db::open(&self.at)?;
        super::reconcile::put_back(&self.project, &mut db, REVERT_VPATH)
    }
}

/// The main thread's second half: all nine tables reloaded, since the revert
/// file does not say which it touches.
pub fn finish_revert(session: &mut EditSession, reloads: &mut super::reload::Reloads) {
    session.applied_loot = None;
    session.wrote_the_database();
    for table in loot::TABLES {
        reloads.when_there_is_a_session(table);
    }
}

/// A Put back as one step of [`super::stack`]. `None` when this project has
/// applied nothing.
pub fn revert_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_revert(session, server)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new("putting back loot", move || {
        let done = job.run();
        let ok = done.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            session.status = match done {
                Ok(rows) => {
                    finish_revert(session, reloads);
                    format!("{rows} loot row(s) put back")
                }
                Err(e) => format!("loot: {e}"),
            };
        });
        (ok, finish)
    })))
}

/// Which of this project's loot rows are in the database. The loot
/// counterpart of [`super::items::OnTheServer`].
pub struct OnTheServer {
    undo: Undo,
    current: bool,
}

impl OnTheServer {
    pub fn read_with(session: &EditSession, plan: &Plan) -> OnTheServer {
        OnTheServer {
            undo: Undo::open_at(&session.project, REVERT_VPATH).unwrap_or(Undo {
                entries: Vec::new(),
            }),
            current: session
                .applied_loot
                .is_some_and(|had| had == plan.signature()),
        }
    }

    /// How many rows this project has applied and can take back.
    pub fn rows(&self) -> usize {
        self.undo.entries.len()
    }

    pub fn covers(&self, table: &str, key: &Key) -> bool {
        self.undo.covers(table, key)
    }

    /// Whether what the project says now is what it applied.
    pub fn current(&self) -> bool {
        self.current
    }
}

/// Handles `--apply-loot` and `--revert-loot`, which do what the Server
/// panel's two buttons do, for a scripted run. Fires once, after any scripted
/// edit has landed.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    server: Res<super::settings::ServerSettings>,
    mut reloads: ResMut<super::reload::Reloads>,
    loot: Res<crate::tools::loot::Loot>,
    mut done: Local<bool>,
) {
    if *done || !(args.apply_loot || args.revert_loot) {
        return;
    }
    if !loot.scripted_done {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    super::creatures::save(&mut session);
    save(&mut session);
    // Runs through [`super::stack`], as the Server panel's buttons do, so a
    // flag puts back and applies the later subjects with this one.
    let wanted = [
        (args.revert_loot, "--revert-loot", super::stack::Wanted::PutBack(super::stack::Subject::Loot)),
        (args.apply_loot, "--apply-loot", super::stack::Wanted::Apply(super::stack::Subject::Loot)),
    ];
    for (asked, flag, wanted) in wanted {
        if !asked {
            continue;
        }
        match super::stack::now(wanted, &mut session, &server, &mut reloads) {
            Ok(line) => info!("{flag}: {line}"),
            Err(e) => warn!("{flag}: {e}"),
        }
    }
}

/// Take a file out of the project, if it is there.
fn remove(project: &vale_edit::project::Project, vpath: &str) {
    if let Some(disk) = project.path_for(vpath) {
        let _ = std::fs::remove_file(disk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_mangos::row::{Edits, RowEdit};

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
        let quests = super::super::quests::plan_from(&edits);
        assert_eq!(quests.rows.len(), 1);
        assert!(quests.refused.is_empty(), "{:?}", quests.refused);
        let items = super::super::items::plan_from(&edits);
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
