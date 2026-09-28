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
//! module comment records where this was read. A [`Life::Delete`] row is
//! therefore written rather than refused, and its undo is
//! [`snapshot_the_quest`]: every version of the template and every row of the
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

use crate::session::EditSession;
use vale_mangos::conn::Db;
use vale_mangos::quest;
use vale_mangos::row::{self, Assignment, Key, Life};
use bevy::prelude::*;

pub use super::creatures::Undo;

/// What this project does to the server's quests, as SQL.
pub const SQL_VPATH: &str = "sql\\quests.sql";

/// The statements that put the changed rows back, as SQL.
pub const REVERT_VPATH: &str = "sql\\quests-revert.sql";

/// One row's worth of change: what is to become of it, and the columns it sets.
#[derive(Debug, Clone)]
pub struct Row {
    pub table: &'static str,
    pub key: Key,
    pub life: Life,
    pub changes: Vec<Assignment>,
    /// Where the database has the row. This is [`Self::key`] unless the
    /// project changes the quest's entry; see
    /// `vale_mangos::row::RowEdit::from`.
    pub at: Key,
}

impl Row {
    /// The statements it becomes. A quest whose entry changes is the move of
    /// the row, of every other content-patch version of it, and of every
    /// column that names it — see `vale_mangos::quest::REFERENCES`.
    pub fn statements(&self) -> Vec<String> {
        if self.moves() {
            return row::move_statements(
                self.table,
                &self.at,
                &self.key,
                &self.changes,
                quest::references(self.table),
            );
        }
        quest::statements(self.table, &self.key, self.life, &self.changes)
    }

    /// Whether the project changes this row's id.
    pub fn moves(&self) -> bool {
        self.life == Life::Update && self.at != self.key
    }

    /// How it is named in a file's marker and in a status line.
    pub fn names(&self) -> String {
        format!("{} {}", self.table, self.key.text())
    }
}

/// Everything a project changes about the server's quests.
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

    /// Every statement this plan comes to, the templates' before the relations'
    /// so that a file run by hand creates a quest before it names one.
    pub fn statements(&self) -> Vec<String> {
        self.ordered().flat_map(Row::statements).collect()
    }

    /// The rows in the order they are written: table by table in
    /// [`quest::RELOAD_ORDER`], and by key inside a table.
    pub fn ordered(&self) -> impl Iterator<Item = &Row> {
        let mut rows: Vec<&Row> = self.rows.iter().collect();
        rows.sort_by_key(|row| {
            (
                quest::RELOAD_ORDER
                    .iter()
                    .position(|table| *table == row.table),
                row.key.clone(),
            )
        });
        rows.into_iter()
    }

    /// The tables it writes, in the order they have to be reloaded.
    pub fn tables(&self) -> Vec<&'static str> {
        quest::RELOAD_ORDER
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

/// The plan the project's store comes to.
pub fn plan(session: &EditSession) -> Plan {
    plan_from(&session.server_edits)
}

/// The plan for a store on its own, so it can be checked with no session.
pub fn plan_from(edits: &vale_mangos::row::Edits) -> Plan {
    let mut out = Plan::default();
    for (table, key, row) in edits.rows() {
        // Another subject's row: skipped without a message, as every writer
        // here skips the others'. See `super::items::plan_from`.
        let Some(table) = quest::table_named(table) else {
            continue;
        };
        let mut changes = Vec::new();
        for (column, value) in &row.columns {
            let Some(known) = quest::column(table, column) else {
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
            // A creation must name every column or it is not written, the
            // same rule as in `super::creatures`.
            Life::Insert => {
                let missing: Vec<&str> = quest::columns_of(table)
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
            }
            Life::Delete => changes.clear(),
            Life::Update => {
                if changes.is_empty() && row.from.is_none() {
                    continue;
                }
            }
        }
        let at = row.from.clone().unwrap_or_else(|| key.clone());
        // A removal deletes the row where the database has it. A row the
        // project moved and then removed is at its old id, and a `DELETE`
        // naming the new id would match nothing without reporting it.
        let key = match row.life {
            Life::Delete => at.clone(),
            _ => key.clone(),
        };
        out.rows.push(Row {
            table,
            key,
            life: row.life,
            changes,
            at,
        });
    }
    out
}

/// Writes `sql\quests.sql`, or removes it when the project changes nothing.
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
        "-- {} — what this project changes about the server's quests.\n\
         -- Rewritten on every save from the project's own edits, so it is the\n\
         -- whole of what the project does rather than an increment of it.\n\
         --\n\
         -- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then reload, the templates first:\n\
         --   .reload quest_template\n\
         --   .reload creature_questrelation       (and the three beside it)\n\
         -- The relation loader drops a row whose quest it has not loaded.\n\n",
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

/// The quest part of a save: the SQL the store comes to. The store itself
/// is written by [`super::creatures::save`], on the same save.
pub fn save(session: &mut EditSession) {
    match write_sql(session) {
        Ok(0) => {}
        Ok(rows) => session.status = format!("{rows} quest change(s) written to {SQL_VPATH}"),
        Err(e) => {
            warn!("quests: {e}");
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
            "{} quest row(s) applied, {} affected — reloading {}",
            self.rows,
            self.affected,
            self.tables.join(", ")
        )
    }
}

/// An Apply with everything it needs to run off the main thread. It has the
/// same three parts as [`super::items::ApplyJob`].
pub struct ApplyJob {
    plan: Plan,
    project: vale_edit::project::Project,
    at: vale_mangos::conn::Where,
    /// The project's store as it stood when the Apply was asked for — see
    /// [`super::reconcile::refuse_a_taken_id`].
    own: vale_mangos::row::Edits,
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
            Err(e) => format!("quests: {e}"),
        }
    }
}

/// The main thread's first half of an Apply. `None` when the project claims no
/// quest row and has applied none.
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
        own: session.server_edits.clone(),
    }))
}

impl ApplyJob {
    /// The worker thread's half, in [`super::reconcile`]'s order: what the
    /// project applied before is put back, then each row is read as it stands,
    /// its undo is written, and its own statements run. Templates run before
    /// relations, so a relation of a quest whose entry changes is found where
    /// the move left it.
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
                "the server's quests",
                &steps,
                |db, index| undo_of_a_row(db, rows[index], &self.own),
            )?;
            Ok(Applied {
                rows: self.plan.rows.len(),
                affected: done.affected,
                newly_undoable: done.undoable,
                taken_back: done.taken_back,
                // All five tables in order, not only the tables the plan
                // writes: the put-back before it may have written any of them,
                // and a move writes the relation tables through its references.
                tables: quest::RELOAD_ORDER.to_vec(),
            })
        })();
        ApplyDone { signature, result }
    }
}

/// The main thread's second half. The tables have changed whether or not the
/// run finished, so the list the tool read is stale and so is the server's
/// copy. All five reloads are asked for, deferred, in order.
pub fn finish_apply(
    session: &mut EditSession,
    reloads: &mut super::reload::Reloads,
    done: &ApplyDone,
) {
    session.wrote_the_database();
    session.applied_quests = done.result.as_ref().ok().map(|_| done.signature);
    for table in quest::RELOAD_ORDER {
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
    Ok(Some(super::stack::Step::new("applying quests", move || {
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
/// quest module's part of [`super::reconcile`]'s step 3.
fn undo_of_a_row(
    db: &mut Db,
    row: &Row,
    own: &vale_mangos::row::Edits,
) -> Result<Option<Vec<String>>, String> {
    let references = quest::references(row.table);
    if row.life == Life::Insert || row.moves() {
        match row.table == quest::TEMPLATE {
            true => super::reconcile::refuse_a_taken_id(db, row.table, &row.key, references, own)?,
            // A relation's key is two ids and neither is its own, so the
            // check is on the whole key.
            false => {
                if db.row(&quest::exists_query(row.table, &row.key))?.is_some() {
                    return Err(format!(
                        "{} {} is already in the database and is not this project's.",
                        row.table,
                        row.key.text()
                    ));
                }
            }
        }
    }
    Ok(match (row.life, row.table == quest::TEMPLATE) {
        // A quest this project creates is undone by removing it with its
        // dependents, and a relation by removing the one row.
        (Life::Insert, true) => row
            .key
            .first()
            .map(|entry| quest::delete_statements(entry as u32)),
        (Life::Insert, false) => Some(vec![row::delete(row.table, &row.key)]),
        // `None` when it is not in the database: nothing to put back.
        (Life::Delete, true) => snapshot_the_quest(db, &row.key)?,
        (Life::Delete, false) => db.row(&quest::row_query(row.table, &row.key))?.map(|held| {
            // The `DELETE` first, so the entry can be run twice — see
            // `super::reconcile`.
            let mut out = vec![row::delete(row.table, &row.key)];
            out.extend(row::insert_from_row(row.table, &held));
            out
        }),
        (Life::Update, _) => {
            let now = db.row(&quest::row_query(row.table, &row.at))?;
            super::reconcile::undo_of_an_update(
                row.table,
                &row.at,
                &row.key,
                &row.changes,
                references,
                now.as_ref(),
            )?
        }
    })
}

/// Everything a removed quest takes with it, as the statements that put it
/// back: every content-patch version of the template, then each of
/// [`quest::DEPENDENTS`]. `None` when the quest is not in the database.
fn snapshot_the_quest(db: &mut Db, key: &Key) -> Result<Option<Vec<String>>, String> {
    let Some(entry) = key.first() else {
        return Ok(None);
    };
    let versions = db.rows(&format!(
        "SELECT * FROM {} WHERE `entry` = {entry}",
        vale_mangos::sql::name(quest::TEMPLATE)
    ))?;
    if versions.is_empty() {
        return Ok(None);
    }
    // The `DELETE`s first, so the entry can be run against a quest it has
    // already restored — see `super::reconcile`.
    let mut out = quest::delete_statements(entry as u32);
    for held in &versions {
        out.extend(row::insert_from_row(quest::TEMPLATE, held));
    }
    for (table, column) in quest::DEPENDENTS {
        let sql = format!(
            "SELECT * FROM {} WHERE {} = {entry}",
            vale_mangos::sql::name(table),
            vale_mangos::sql::name(column)
        );
        for held in db.rows(&sql)? {
            out.extend(row::insert_from_row(table, &held));
        }
    }
    Ok(Some(out))
}

/// A Put back with everything it needs to run off the main thread.
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

/// The main thread's second half: all five tables reloaded, since the revert
/// file does not say which it touches.
pub fn finish_revert(session: &mut EditSession, reloads: &mut super::reload::Reloads) {
    session.applied_quests = None;
    session.wrote_the_database();
    for table in quest::RELOAD_ORDER {
        reloads.when_there_is_a_session(table);
    }
}

/// A Put back as one step of [`super::stack`]: `None` when this project has
/// applied nothing.
pub fn revert_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_revert(session, server)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new("putting back quests", move || {
        let done = job.run();
        let ok = done.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            session.status = match done {
                Ok(rows) => {
                    finish_revert(session, reloads);
                    format!("{rows} quest row(s) put back")
                }
                Err(e) => format!("quests: {e}"),
            };
        });
        (ok, finish)
    })))
}

/// Which of this project's quest rows are in the database. The quest
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
                .applied_quests
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

/// Runs `--apply-quests` and `--revert-quests`, the command-line forms of the
/// Server panel's two buttons. Fires once, after any scripted edit has landed.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    server: Res<super::settings::ServerSettings>,
    mut reloads: ResMut<super::reload::Reloads>,
    quests: Res<crate::tools::quests::Quests>,
    mut done: Local<bool>,
) {
    if *done || !(args.apply_quests || args.revert_quests) {
        return;
    }
    if !quests.scripted_done {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    super::creatures::save(&mut session);
    save(&mut session);
    // Run through [`super::stack`], as the Server panel's buttons are, so a
    // flag also puts back and applies the later subjects with this one.
    let wanted = [
        (args.revert_quests, "--revert-quests", super::stack::Wanted::PutBack(super::stack::Subject::Quests)),
        (args.apply_quests, "--apply-quests", super::stack::Wanted::Apply(super::stack::Subject::Quests)),
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
        let items = super::super::items::plan_from(&edits);
        assert_eq!(items.rows.len(), 1);
        assert!(items.refused.is_empty(), "{:?}", items.refused);
        let creatures = super::super::creatures::plan_from(&edits, &Default::default());
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
