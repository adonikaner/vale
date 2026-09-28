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

use crate::session::EditSession;
use vale_edit::project::Project;
use vale_mangos::conn::Db;
use vale_mangos::creature;
use vale_mangos::row::{self, Assignment, Key, Life};
use bevy::prelude::*;

/// Where the project keeps what it changes.
pub const EDITS_VPATH: &str = "server\\rows.txt";

/// Where the project keeps the waypoint paths it changes. A store of their own,
/// because a path's edit is a set of rows rather than a column. See
/// [`vale_mangos::path`].
pub const PATHS_VPATH: &str = "server\\paths.txt";

/// The SQL the two stores come to.
pub const SQL_VPATH: &str = "sql\\creatures.sql";

/// The SQL that puts those rows back.
pub const REVERT_VPATH: &str = "sql\\creatures-revert.sql";

/// One row's worth of change: what is to become of it, and the columns it
/// sets.
#[derive(Debug, Clone)]
pub struct Row {
    pub table: &'static str,
    pub key: Key,
    /// Whether this row is edited, created or removed — see
    /// [`vale_mangos::row::Life`].
    pub life: Life,
    pub changes: Vec<Assignment>,
    /// Where the database has the row, which is [`Self::key`] unless the
    /// project changes the row's id. See `vale_mangos::row::RowEdit::from`.
    pub at: Key,
}

impl Row {
    /// The statements it becomes: one `UPDATE`, one `INSERT`, the six
    /// `DELETE`s a spawn and its five dependent tables come to, or the move of
    /// a row whose id changes with everything that names it.
    pub fn statements(&self) -> Vec<String> {
        if self.life == Life::Update && self.at != self.key {
            return row::move_statements(
                self.table,
                &self.at,
                &self.key,
                &self.changes,
                creature::references(self.table),
            );
        }
        creature::statements(self.table, &self.key, self.life, &self.changes)
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

/// Everything a project changes about the server's creatures.
#[derive(Debug, Default)]
pub struct Plan {
    pub rows: Vec<Row>,
    /// Every waypoint path the project replaces.
    ///
    /// Separate from [`Self::rows`] because one path is several statements — a
    /// `DELETE` and an `INSERT` a node — where a row is one `UPDATE`.
    pub paths: Vec<vale_mangos::path::Path>,
    /// What the store asked for and this writer would not write, each as a
    /// sentence.
    ///
    /// A table or a column this editor has no schema for, a key column an edit
    /// tried to move, a row a table cannot create or remove, a creation that
    /// does not name every column, a path on a spawn the same project is
    /// removing. Reported rather than skipped silently: the store is a text
    /// file somebody may have edited, and an edit that reaches no statement
    /// never happens.
    pub refused: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.paths.is_empty()
    }

    /// The tables it touches, in the order they were walked.
    pub fn tables(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for row in &self.rows {
            if !out.contains(&row.table) {
                out.push(row.table);
            }
        }
        out
    }

    /// How many rows it creates, edits and removes.
    pub fn counts(&self) -> (usize, usize, usize) {
        let of = |life: Life| self.rows.iter().filter(|row| row.life == life).count();
        (of(Life::Insert), of(Life::Update), of(Life::Delete))
    }

    /// One line for a panel.
    pub fn line(&self) -> String {
        let (new, edited, gone) = self.counts();
        let mut parts: Vec<String> = Vec::new();
        if new > 0 {
            parts.push(format!("{new} new"));
        }
        if edited > 0 {
            parts.push(format!("{edited} edited"));
        }
        if gone > 0 {
            parts.push(format!("{gone} removed"));
        }
        let rows = match parts.is_empty() {
            true => String::new(),
            false => format!("{} row(s): {}", self.rows.len(), parts.join(", ")),
        };
        let paths = match self.paths.len() {
            0 => String::new(),
            1 => "1 path replaced".to_string(),
            n => format!("{n} paths replaced"),
        };
        match (rows.is_empty(), paths.is_empty()) {
            (true, true) => "no creature edits".to_string(),
            (false, true) => rows,
            (true, false) => paths,
            (false, false) => format!("{rows}, {paths}"),
        }
    }

    /// What this plan would do to the database, as one number.
    ///
    /// `super::rows::Plan::signature`'s counterpart, with the same purpose:
    /// two plans with the same signature apply the same statements, so a
    /// project whose signature is the one the last apply used is a project
    /// whose edits are in the database. The revert file cannot answer that: it
    /// records what a row held before this project touched it, which does not
    /// change when the edit does.
    ///
    /// The hash is over the statements rather than the row count: two edits
    /// to one row are one row and two different databases.
    pub fn signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for statement in self.statements() {
            statement.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Every statement this plan comes to, rows first.
    ///
    /// Rows come before paths, which matters for two cases. A spawn whose
    /// `movement_type` is being set to 2 in the same save that gives it a path
    /// reads top to bottom as being made to walk before it is told where;
    /// neither order is wrong for the database, since they are different
    /// tables. And a spawn this project creates and gives a path to has to
    /// exist before its `creature_movement` rows are written.
    pub fn statements(&self) -> Vec<String> {
        let mut out: Vec<String> = self.rows.iter().flat_map(Row::statements).collect();
        for path in &self.paths {
            out.extend(vale_mangos::path::statements(path));
        }
        out
    }
}

/// What the project's store comes to, as statements.
///
/// A table, a column or a life the schema does not know is left out and named
/// in [`Plan::refused`]. The column's name in a statement is always one of this
/// crate's own constants rather than text read off a disk. See
/// `vale_mangos::creature::column`.
pub fn plan(session: &EditSession) -> Plan {
    plan_from(&session.server_edits, &session.server_paths)
}

/// The same over the two stores alone, so what a store comes to can be
/// checked without a session, a project folder or a database.
pub fn plan_from(edits: &vale_mangos::row::Edits, paths: &vale_mangos::path::Paths) -> Plan {
    let mut out = Plan::default();
    for (table, key, row) in edits.rows() {
        let Some(table) = creature::table_named(table) else {
            // A row belonging to another subject is skipped, not refused.
            // The store is one file holding every server row a project
            // changes, and each writer walks the whole of it. An item's row is
            // `super::items`' to write, and a refusal here would put an error
            // on the creature panel for an edit that the item panel applies.
            // Every subject in `super::stack::Subject::ORDER` is asked, so a
            // subject added there is skipped here without a second list to
            // keep; a hand-kept list here missed the behaviour tables.
            if super::stack::Subject::ORDER.iter().any(|subject| subject.owns(table)) {
                continue;
            }
            out.refused
                .push(format!("{table} is not a table this editor writes"));
            continue;
        };
        if !creature::can_live(table, row.life) {
            // Only a spawn can be created or removed. See
            // `vale_mangos::creature::can_live` for the reason.
            out.refused.push(format!(
                "{table} {} cannot be {}d, only edited",
                key.text(),
                row.life.word()
            ));
            continue;
        }
        let mut changes = Vec::new();
        for (column, value) in &row.columns {
            let Some(known) = creature::column(table, column) else {
                out.refused.push(format!(
                    "{table}.{column} is not a column this editor writes"
                ));
                continue;
            };
            // A key column is what the `WHERE` names; writing one would move
            // the row the edit is about. An `INSERT` names it from the key.
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
            // A creation names every column or it is not written. A column an
            // `INSERT` leaves out takes the table's own default, and three of
            // this table's defaults are wrong for a spawn placed by hand, so a
            // missing column would be a value nobody chose, written silently.
            Life::Insert => {
                let missing: Vec<&str> = creature::columns_of(table)
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
            // A removal carries no columns and needs none: the statement is
            // the key.
            Life::Delete => changes.clear(),
            Life::Update => {
                if changes.is_empty() && row.from.is_none() {
                    continue;
                }
            }
        }
        let at = row.from.clone().unwrap_or_else(|| key.clone());
        // A removal is of the row where the database has it. A row the
        // project moved and then removed is at its old id, and a `DELETE`
        // naming the new one would match nothing and report no error.
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
    // The paths, which need no column check: every column of a movement row is
    // this crate's own constant and none of them is typed.
    //
    // One check remains. A path on a spawn this same project removes would
    // write `creature_movement` rows keyed to a guid whose creature the six
    // `DELETE`s above have just taken away: an orphan the server reports at
    // every start and nothing points at.
    for path in paths.iter() {
        if removes_the_spawn(&out, path.which, path.owner) {
            out.refused.push(format!(
                "the path on {} {} is not written — this project removes that spawn",
                path.which.table(),
                path.owner
            ));
            continue;
        }
        out.paths.push(path);
    }
    out
}

/// Whether the plan removes the spawn a path belongs to.
///
/// Only a spawn's own path can be orphaned this way: a template path is keyed
/// by a creature entry, which no removal here touches.
fn removes_the_spawn(plan: &Plan, which: vale_mangos::path::Which, owner: u64) -> bool {
    if which != vale_mangos::path::Which::Spawn {
        return false;
    }
    plan.rows.iter().any(|row| {
        row.life == Life::Delete && row.table == creature::SPAWN && row.key.first() == Some(owner)
    })
}
/// Write `sql\creatures.sql`, or remove it when the project changes
/// nothing.
///
/// Called by every save. It writes a file and touches no database.
pub fn write_sql(session: &mut EditSession) -> Result<usize, String> {
    let plan = plan(session);
    for refused in &plan.refused {
        warn!("{EDITS_VPATH}: {refused}");
    }
    if plan.is_empty() {
        remove(&session.project, SQL_VPATH);
        return Ok(0);
    }
    let count = plan.rows.len() + plan.paths.len();
    let mut body = format!(
        "-- {} — what this project changes about the server's creatures.\n\
         -- Rewritten on every save from the project's own edits, so it is the\n\
         -- whole of what the project does rather than an increment of it.\n\
         --\n\
         -- Nothing applies this by itself. Apply it from the editor's creature\n\
         -- panel, or run it by hand, and then RESTART the server: vmangos reads\n\
         -- creature_template once at startup, and .reload does not restat\n\
         -- creatures that are already spawned.\n\n",
        session.project.name
    );
    for table in plan.tables() {
        body.push_str(&format!("-- {table}\n"));
        for row in plan.rows.iter().filter(|row| row.table == table) {
            for statement in row.statements() {
                body.push_str(&statement);
                body.push('\n');
            }
        }
        body.push('\n');
    }
    // The paths, one block each, because a path is several statements and a
    // reader has to be able to see where one ends. The heading names the
    // creature and the node count, which is what somebody reviewing this file
    // is checking.
    for path in &plan.paths {
        body.push_str(&format!(
            "-- {} {} = {} — {} node(s)\n",
            path.which.table(),
            path.which.key_column(),
            path.owner,
            path.nodes.len()
        ));
        for statement in vale_mangos::path::statements(path) {
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

/// The creature half of a save: the project's store, and the SQL it comes
/// to.
///
/// Two files and no database. Everything here is written to the project folder;
/// reaching the server is [`apply`], which is a button. See the module comment
/// for why the two are separate steps.
pub fn save(session: &mut EditSession) {
    if !session.save_server_edits() {
        // `save_server_edits` has already put the reason on the status line.
        return;
    }
    if !session.save_server_paths() {
        return;
    }
    match write_sql(session) {
        Ok(0) => {}
        Ok(rows) => session.status = format!("{rows} creature change(s) written to {SQL_VPATH}"),
        Err(e) => {
            warn!("creatures: {e}");
            session.status = format!("saved, but {SQL_VPATH} did not: {e}");
        }
    }
}

/// What an apply did.
#[derive(Debug, Default)]
pub struct Applied {
    pub rows: usize,
    pub affected: u64,
    /// How many rows the revert file covers after this apply.
    pub newly_undoable: usize,
    /// How many rows the project had applied and no longer claims, which were
    /// put back and not written again. See [`super::reconcile`].
    pub taken_back: usize,
}

impl Applied {
    pub fn line(&self) -> String {
        let taken_back = match self.taken_back {
            0 => String::new(),
            n => format!(", {n} no longer claimed put back"),
        };
        format!(
            "{} row(s) applied, {} affected{taken_back} — restart the server to see them",
            self.rows, self.affected
        )
    }
}

/// An Apply, with everything it needs to run off the main thread: the plan,
/// the project handle its revert file is written through, and where the
/// database is. See [`super::queue`].
pub struct ApplyJob {
    plan: Plan,
    project: vale_edit::project::Project,
    at: vale_mangos::conn::Where,
    /// The project's store as it stood when the Apply was asked for. See
    /// [`super::reconcile::refuse_a_taken_id`], which leaves the project's own
    /// rows out of its count.
    own: vale_mangos::row::Edits,
}

/// What running an [`ApplyJob`] answered, for the main thread's half.
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
            Err(e) => format!("creatures: {e}"),
        }
    }
}

/// The main thread's first half of an Apply: the plan, and whether there is
/// anything to do. `None` when the project claims no creature row or path and
/// has applied none.
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
    /// The worker's half. The order is what makes it safe: every row not
    /// already covered by the revert file is read as it stands, the statement
    /// that puts it back is appended, the file is written, and only then does
    /// anything run. See [`super::reconcile`].
    pub fn run(self) -> ApplyDone {
        let signature = self.plan.signature();
        let plan = &self.plan;
        let result = (|| {
            let mut db = Db::open(&self.at)?;
            // Rows before paths, which is [`Plan::statements`]' own order and
            // for its reason: a spawn this project creates has to exist before
            // its path.
            let mut steps: Vec<super::reconcile::Step> = plan
                .rows
                .iter()
                .map(|row| super::reconcile::Step {
                    table: row.table.to_string(),
                    key: row.key.clone(),
                    statements: row.statements(),
                })
                .collect();
            steps.extend(plan.paths.iter().map(|path| super::reconcile::Step {
                table: path.which.table().to_string(),
                key: path.key(),
                statements: vale_mangos::path::statements(path),
            }));
            let rows = plan.rows.len();
            let done = super::reconcile::apply(
                &self.project,
                &mut db,
                REVERT_VPATH,
                "the server's creatures",
                &steps,
                |db, index| match index < rows {
                    true => undo_of_a_row(db, &plan.rows[index], &self.own),
                    false => undo_of_a_path(db, &plan.paths[index - rows]).map(Some),
                },
            )?;
            Ok(Applied {
                rows: plan.rows.len() + plan.paths.len(),
                affected: done.affected,
                newly_undoable: done.undoable,
                taken_back: done.taken_back,
            })
        })();
        ApplyDone { signature, result }
    }
}

/// The main thread's second half of an Apply.
///
/// The database has changed whether or not the run finished, so what the tools
/// read out of it is stale either way. The signature of what is now in the
/// database is kept only for a run that finished, so the panel can tell
/// "applied" from "applied, and changed since" (see [`Plan::signature`]), and a
/// failure leaves the project saying it has applied nothing rather than
/// claiming a change that did not land.
///
/// Sends no reload and asks for none. See the module comment: a reload cannot
/// make every kind of creature change live, so the instruction is to restart
/// the server, and the status line says so.
pub fn finish_apply(session: &mut EditSession, done: &ApplyDone) {
    session.wrote_the_database();
    session.applied_creatures = done.result.as_ref().ok().map(|_| done.signature);
}

/// An Apply as one step of [`super::stack`], which is the only way it runs:
/// `None` when there is nothing to apply.
pub fn apply_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_apply(session, server)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new("applying creatures", move || {
        let done = job.run();
        let ok = done.result.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, _: &mut super::reload::Reloads| {
            finish_apply(session, &done);
            session.status = done.line();
        });
        (ok, finish)
    })))
}

/// What the database holds for a path right now, as the statements that put it
/// back.
///
/// A path with no rows still gets an undo, and it is the `DELETE` alone. That
/// is the correct inverse of creating a path where there was none, and it is
/// the case a row's undo cannot have: `row::undo` returns `None` for a missing
/// row.
fn undo_of_a_path(db: &mut Db, path: &vale_mangos::path::Path) -> Result<Vec<String>, String> {
    let rows = db.rows(&vale_mangos::path::whole_rows_query(path.which, path.owner))?;
    Ok(vale_mangos::path::undo_from_rows(path.which, path.owner, &rows))
}

/// One row's undo, read immediately before the row is written: the subject's
/// half of [`super::reconcile`]'s step 3.
fn undo_of_a_row(
    db: &mut Db,
    row: &Row,
    own: &vale_mangos::row::Edits,
) -> Result<Option<Vec<String>>, String> {
    let references = creature::references(row.table);
    if row.life == Life::Insert || row.moves() {
        let limit = match row.table {
            creature::SPAWN => creature::MAX_GUID,
            _ => u64::from(creature::MAX_ENTRY),
        };
        super::reconcile::refuse_a_guid_past_the_limit(row.table, &row.key, limit)?;
        super::reconcile::refuse_a_taken_id(db, row.table, &row.key, references, own)?;
    }
    Ok(match row.life {
        // A row this project creates is undone by removing it: a spawn with
        // its five dependent tables, a template on its own. See
        // `creature::insert_undo_statements`.
        Life::Insert => Some(creature::insert_undo_statements(row.table, &row.key)),
        // A row this project removes is undone by putting back every row the
        // six `DELETE`s take. `None` when the row is not there: there is
        // nothing to put back, and a marker with no statement under it would
        // read as an applied removal this project could take back and could
        // not.
        Life::Delete => snapshot_the_spawn(db, &row.key)?,
        // `None` when the row is not there. A key that matches nothing names a
        // row somebody else removed, and inventing an `INSERT` for it would
        // write a row this project never saw.
        Life::Update => {
            let now = db.row(&creature::row_query(row.table, &row.at))?;
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

/// Everything a removed spawn takes with it, as the statements that put it
/// back.
///
/// The `creature` row first and then each of the five tables keyed by its guid,
/// in [`creature::DEPENDENTS`]' own order, so running them restores the
/// creature before the rows that point at it. That is the order a person
/// reading the file expects, and no foreign key here enforces it.
///
/// `None` when the spawn is not in the database at all, which is a removal with
/// nothing to undo.
fn snapshot_the_spawn(db: &mut Db, key: &Key) -> Result<Option<Vec<String>>, String> {
    let Some(spawn) = db.row(&creature::row_query(creature::SPAWN, key))? else {
        return Ok(None);
    };
    let Some(guid) = key.first() else {
        return Ok(None);
    };
    // The `DELETE`s first, so the entry can be run against a spawn it has
    // already restored. See `super::reconcile` for the rule.
    let mut out = creature::delete_statements(key);
    out.extend(row::insert_from_row(creature::SPAWN, &spawn));
    for (table, column) in creature::DEPENDENTS {
        let sql = format!(
            "SELECT * FROM {} WHERE {} = {guid}",
            vale_mangos::sql::name(table),
            vale_mangos::sql::name(column)
        );
        for held in db.rows(&sql)? {
            out.extend(row::insert_from_row(table, &held));
        }
    }
    Ok(Some(out))
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
    /// The worker's half: run the revert file, and forget it. The file is
    /// removed only after the statements have run, so a revert that fails half
    /// way keeps its file, to be run again or read by hand.
    pub fn run(self) -> Result<usize, String> {
        let mut db = Db::open(&self.at)?;
        super::reconcile::put_back(&self.project, &mut db, REVERT_VPATH)
    }
}

/// The main thread's second half: the database is what it was, so nothing this
/// project says is in it.
pub fn finish_revert(session: &mut EditSession) {
    session.applied_creatures = None;
    session.wrote_the_database();
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
    Ok(Some(super::stack::Step::new("putting back creatures", move || {
        let done = job.run();
        let ok = done.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, _: &mut super::reload::Reloads| {
            session.status = match done {
                Ok(rows) => {
                    finish_revert(session);
                    format!(
                        "put back {rows} row(s) \u{2014} restart the server. This project still \
                         changes them; Discard gives them up"
                    )
                }
                Err(e) => format!("creatures: {e}"),
            };
        });
        (ok, finish)
    })))
}

/// Whether this project has applied that particular path to the database.
///
/// [`OnTheServer::covers`] for the one caller that wants a single answer and
/// has no reason to read the rest: the waypoint window is about one path, and
/// "this project has applied something" is not an answer about the one being
/// looked at.
pub fn path_applied(session: &EditSession, path: &vale_mangos::path::Path) -> bool {
    OnTheServer::read(session).covers(path.which.table(), &path.key())
}

/// What of this project is in the database, as the two questions a panel
/// asks.
///
/// Named for the state rather than the act: [`Applied`] above is what one
/// press of the button did, and this is what the project's rows are.
///
/// Per row, because a project can carry a spawn it has applied beside one it
/// has not and the sentence for each is different. The revert file is the
/// record (a key in it is a row this project has written), and it is the
/// project's own, so it outlives the session that wrote it.
///
/// [`OnTheServer::current`] is the second half and the one the file cannot answer:
/// whether what the project says now is what was applied. See
/// [`Plan::signature`].
pub struct OnTheServer {
    undo: Undo,
    /// Whether the plan's signature is the one the last apply used. `false`
    /// when nothing has been applied this session, which is also what it is
    /// after a relaunch. See `EditSession::applied_creatures`.
    current: bool,
}

impl OnTheServer {
    /// The revert file alone, which answers [`Self::covers`].
    ///
    /// For a caller that asks about one row and nothing else; see
    /// [`path_applied`]. [`Self::current`] is `false`, which is the safe
    /// reading: it only ever adds a caveat.
    pub fn read(session: &EditSession) -> OnTheServer {
        OnTheServer {
            undo: Undo::open(session).unwrap_or_else(|_| Undo {
                entries: Vec::new(),
            }),
            current: false,
        }
    }

    /// The same with [`Self::current`] answered, which needs the plan.
    ///
    /// The plan is the caller's because building one walks the store and
    /// formats every statement it comes to (a path of 227 nodes is 228
    /// strings), and the panel that wants this has already built one.
    pub fn read_with(session: &EditSession, plan: &Plan) -> OnTheServer {
        OnTheServer {
            current: session
                .applied_creatures
                .is_some_and(|had| had == plan.signature()),
            ..OnTheServer::read(session)
        }
    }

    /// How many rows this project has applied and can take back.
    pub fn rows(&self) -> usize {
        self.undo.entries.len()
    }

    /// Whether this project has applied that row.
    pub fn covers(&self, table: &str, key: &Key) -> bool {
        self.undo.covers(table, key)
    }

    /// Whether what the project says now is what it applied.
    ///
    /// `false` after a relaunch even for a project that applied everything
    /// last session, so a caller uses it to add a caveat and never to take
    /// one away: "applied" is what [`Self::covers`] says, and this only
    /// decides whether "and changed since" goes after it.
    pub fn current(&self) -> bool {
        self.current
    }
}

/// Take a file out of the project, if it is there.
fn remove(project: &Project, vpath: &str) {
    if let Some(disk) = project.path_for(vpath) {
        let _ = std::fs::remove_file(disk);
    }
}

/// `--apply-creatures` and `--revert-creatures`, which are the panel's two
/// buttons with nobody at the keyboard. See [`crate::Args::apply_creatures`].
///
/// Fires once, on the first frame there is a session to read the project out
/// of. A flag that fired repeatedly would re-apply after every save.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    server: Res<super::settings::ServerSettings>,
    mut reloads: ResMut<super::reload::Reloads>,
    waypoints: Res<crate::tools::waypoints::Waypoints>,
    creatures: Res<crate::tools::creatures::Creatures>,
    mut done: Local<bool>,
) {
    if *done || !(args.apply_creatures || args.revert_creatures) {
        return;
    }
    // Wait for a scripted edit to land. This fires on the first frame there
    // is a session; `--waypoint-add` fires when two queries have come back,
    // `--spawn-add` when three have and `--template-new` when the map read
    // has, several seconds later. Without this the apply ran against an empty
    // store and said so. See `tools::waypoints::Waypoints::scripted_done` and
    // the two counterparts on the creature tool.
    if !waypoints.scripted_done
        || !creatures.scripted_done
        || !creatures.scripted_template_done
    {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    // The store is written first, so the flag applies what the project holds
    // rather than what its last save happened to have written.
    save(&mut session);
    // Through the stack, as the Server panel's buttons are (see
    // [`super::stack`]), so a flag puts back and applies the later subjects
    // with this one.
    let wanted = [
        (args.revert_creatures, "--revert-creatures", super::stack::Wanted::PutBack(super::stack::Subject::Creatures)),
        (args.apply_creatures, "--apply-creatures", super::stack::Wanted::Apply(super::stack::Subject::Creatures)),
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

/// `sql\creatures-revert.sql`: how to put the rows back.
///
/// Plain SQL with a marker line before each statement naming the table and the
/// row. The marker is read, not decorative: it is how an apply knows which rows
/// it has already snapshotted. The alternative, a side file in some
/// serialisation, would be a second format and would stop the undo being a
/// file a person can read and run by hand, which is the property that makes an
/// automatic write to a database checkable.
///
/// ```text
/// -- row creature_template entry=68;patch=0
/// UPDATE `creature_template` SET `name` = 'Stormwind City Guard' WHERE …;
///
/// -- row creature_movement id=12345
/// DELETE FROM `creature_movement` WHERE `id` = 12345;
/// INSERT INTO `creature_movement` (...) VALUES (12345, 1, ...);
/// INSERT INTO `creature_movement` (...) VALUES (12345, 2, ...);
/// ```
///
/// An entry runs to the next marker, which is what lets a path's undo be
/// several statements. Reading one line per marker restored a path as its
/// `DELETE` and none of its nodes, so the undo emptied the path it was meant
/// to put back.
pub struct Undo {
    /// `(table, key, statements)`, oldest first.
    ///
    /// Several statements per entry, because a waypoint path's undo is a
    /// `DELETE` and an `INSERT` a node rather than one `UPDATE`. A row's entry
    /// holds exactly one and reads back as one.
    pub entries: Vec<(String, Key, Vec<String>)>,
}

impl Undo {
    const MARKER: &'static str = "-- row ";
    /// The line a finished file carries to say its entries are newest first.
    /// A file without it is oldest first, which is every file written before
    /// the order mattered and any file an apply was stopped in the middle of.
    const NEWEST_FIRST: &'static str = "-- order: newest first";
    /// The line that closes an entry. See [`Self::from_text`].
    const END: &'static str = "-- end";

    pub fn open(session: &EditSession) -> Result<Undo, String> {
        Undo::open_at(&session.project, REVERT_VPATH)
    }

    /// The same from any of the project's revert files.
    ///
    /// The marker format is one format, which is why the path is a parameter
    /// rather than a second parser: `super::items` keeps a revert file of its
    /// own for the reason this half keeps its own SQL (two subjects applied by
    /// two gestures), and a second reading of `-- row <table> <key>` would be
    /// a place for the two to drift apart silently.
    pub fn open_at(project: &Project, vpath: &str) -> Result<Undo, String> {
        let Some(text) = project.read(vpath) else {
            return Ok(Undo {
                entries: Vec::new(),
            });
        };
        let text = String::from_utf8(text).map_err(|e| format!("{vpath}: {e}"))?;
        Ok(Undo::from_text(&text))
    }

    /// The parse on its own, so it can be tested without a project on disk.
    ///
    /// An entry without its [`Self::END`] line is not read, when the file has
    /// any. An entry is appended before its row's statements run, so one cut
    /// short by a failed or interrupted append is a row that was never
    /// written; running what survived of it would be half an undo (the
    /// `DELETE`s of a removed spawn's snapshot without the `INSERT`s that
    /// restore it) against a row that is still there. A file with no `END`
    /// line at all predates them and is read as it always was.
    pub fn from_text(text: &str) -> Undo {
        let mut entries: Vec<(String, Key, Vec<String>)> = Vec::new();
        let mut ended: Vec<bool> = Vec::new();
        let mut any_end = false;
        let mut at: Option<(String, Key)> = None;
        let mut newest_first = false;
        for line in text.lines() {
            let line = line.trim();
            if line == Self::NEWEST_FIRST {
                newest_first = true;
                continue;
            }
            if line == Self::END {
                any_end = true;
                if let Some(last) = ended.last_mut() {
                    *last = true;
                }
                at = None;
                continue;
            }
            if let Some(rest) = line.strip_prefix(Self::MARKER) {
                at = rest
                    .split_once(' ')
                    .and_then(|(table, key)| Some((table.trim().to_string(), Key::parse(key)?)));
                continue;
            }
            // A comment that is not a marker clears nothing and carries
            // nothing, so the file's own header cannot become an entry.
            if line.is_empty() || line.starts_with("--") {
                continue;
            }
            // The marker is not taken, so every statement until the next one
            // joins this entry. See the type's own doc.
            let Some((table, key)) = at.as_ref() else {
                continue;
            };
            match (entries.last_mut(), ended.last()) {
                (Some((had_table, had_key, statements)), Some(false))
                    if had_table == table && had_key == key =>
                {
                    statements.push(line.to_string());
                }
                _ => {
                    entries.push((table.clone(), key.clone(), vec![line.to_string()]));
                    ended.push(false);
                }
            }
        }
        if any_end {
            let mut ended = ended.into_iter();
            entries.retain(|_| ended.next().unwrap_or(false));
        }
        if newest_first {
            entries.reverse();
        }
        Undo { entries }
    }

    /// Whether this row has already been snapshotted.
    ///
    /// By table and key together. `creature_template` entry 68 and
    /// `creature` guid 68 are different rows, and `(68, patch 0)` and
    /// `(68, patch 3)` are two more; a check on the id alone would give the
    /// second of any pair the first's statement.
    pub fn covers(&self, table: &str, key: &Key) -> bool {
        self.entries
            .iter()
            .any(|(had_table, had_key, _)| had_table == table && had_key == key)
    }

    pub fn add(&mut self, table: &str, key: &Key, statement: &str) {
        self.add_many(table, key, &[statement.to_string()]);
    }

    /// The same for the several statements a path's undo is.
    pub fn add_many(&mut self, table: &str, key: &Key, statements: &[String]) {
        self.entries
            .push((table.to_string(), key.clone(), statements.to_vec()));
    }

    /// Every statement, in the order that puts the database back: the newest
    /// entry first, and each entry's own statements in the order it holds
    /// them.
    ///
    /// Newest first because entries are written in the order the plan ran and
    /// each is the inverse of its own step given the steps before it; see
    /// [`super::reconcile`]. A row created at an id another row was moved away
    /// from has to go before that row can come back.
    pub fn put_back(&self) -> Vec<String> {
        self.entries
            .iter()
            .rev()
            .flat_map(|(_, _, statements)| statements.iter().cloned())
            .collect()
    }

    pub fn write(&self, session: &EditSession) -> Result<(), String> {
        self.write_at(&session.project, REVERT_VPATH, "the server's creatures")
    }

    /// Add the newest entry to the file, which is [`Self::write_at`] for a
    /// caller that adds one entry per row of a plan: rewriting the whole file
    /// each time is quadratic in the plan, and a project that moves two
    /// thousand spawns is four hundred megabytes of it.
    ///
    /// The file is the same text either way. The first entry writes the whole
    /// file, header included.
    pub fn append_newest_at(
        &self,
        project: &Project,
        vpath: &str,
        subject: &str,
    ) -> Result<(), String> {
        let (Some((table, key, statements)), true) = (self.entries.last(), self.entries.len() > 1)
        else {
            return self.write_at(project, vpath, subject);
        };
        let Some(disk) = project.path_for(vpath) else {
            return self.write_at(project, vpath, subject);
        };
        let body = Self::entry_text(&(table.clone(), key.clone(), statements.clone()));
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&disk)
            .and_then(|mut file| file.write_all(body.as_bytes()))
            .map_err(|e| format!("{vpath}: {e}"))
    }

    /// Write to any of the project's revert files. See [`Undo::open_at`].
    ///
    /// Oldest entry first, which is the order an apply adds them in and the
    /// order [`Self::append_newest_at`] continues. It is the file as it stands
    /// while an apply is running; [`Self::write_finished_at`] is what replaces
    /// it when the apply is done.
    pub fn write_at(
        &self,
        project: &Project,
        vpath: &str,
        subject: &str,
    ) -> Result<(), String> {
        if self.entries.is_empty() {
            return Ok(());
        }
        let mut body = format!(
            "-- {} — how to put {subject} back.\n\
             -- An apply is writing this file: one entry per row, each added\n\
             -- immediately before that row's own statements run. Run the entries\n\
             -- from the LAST to the first. Every statement can be run twice.\n\n",
            project.name
        );
        for entry in &self.entries {
            body.push_str(&Self::entry_text(entry));
        }
        project
            .write(vpath, body.as_bytes())
            .map_err(|e| format!("{vpath}: {e}"))?;
        Ok(())
    }

    /// The file as a finished apply leaves it: newest entry first, so that
    /// running it top to bottom by hand is [`Self::put_back`]. The
    /// [`Self::NEWEST_FIRST`] line is what tells [`Self::from_text`] which way
    /// round it is.
    pub fn write_finished_at(
        &self,
        project: &Project,
        vpath: &str,
        subject: &str,
    ) -> Result<(), String> {
        if self.entries.is_empty() {
            return Ok(());
        }
        let mut body = format!(
            "-- {} — how to put {subject} back.\n\
             -- One entry per row this project applied, each read from the database\n\
             -- immediately before that row was written. Run top to bottom, it\n\
             -- returns the database to what it was before this project. Every\n\
             -- statement can be run twice. Plain SQL: it can be run by hand.\n\
             {}\n\n",
            project.name,
            Self::NEWEST_FIRST
        );
        for entry in self.entries.iter().rev() {
            body.push_str(&Self::entry_text(entry));
        }
        project
            .write(vpath, body.as_bytes())
            .map_err(|e| format!("{vpath}: {e}"))?;
        Ok(())
    }

    fn entry_text((table, key, statements): &(String, Key, Vec<String>)) -> String {
        let mut body = format!("{}{table} {}\n", Self::MARKER, key.text());
        for statement in statements {
            body.push_str(statement);
            body.push('\n');
        }
        body.push_str(Self::END);
        body.push_str("\n\n");
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The revert file round-trips through its own marker.
    ///
    /// That property is what stops a second apply from overwriting the first
    /// snapshot. A marker that stopped being recognised would read as "this
    /// row has no undo yet", re-snapshot the already-edited row, and leave
    /// Revert restoring the edit instead of removing it, with no error
    /// reported.
    #[test]
    fn the_revert_file_reads_back_what_it_wrote() {
        let written = "-- a project — how to put the server's creatures back.\n\
             -- a second comment line that is not a marker\n\n\
             -- row creature_template entry=68;patch=0\n\
             UPDATE `creature_template` SET `level_min` = 55 WHERE `entry` = 68;\n\n\
             -- row creature guid=19272\n\
             UPDATE `creature` SET `position_z` = 98.7 WHERE `guid` = 19272;\n";
        let undo = Undo::from_text(written);
        assert_eq!(undo.entries.len(), 2);
        assert_eq!(undo.entries[0].0, creature::TEMPLATE);
        assert_eq!(undo.entries[1].0, creature::SPAWN);
        assert!(undo.covers(creature::TEMPLATE, &Key::two(("entry", 68), ("patch", 0))));
        assert!(undo.covers(creature::SPAWN, &Key::one("guid", 19272)));
        assert!(!undo.covers(creature::SPAWN, &Key::one("guid", 1)));
    }

    /// An entry cut short by an interrupted append is not run. Its row's
    /// statements never ran either, and half a removal's snapshot (the
    /// `DELETE`s without the `INSERT`s) would delete a spawn that is still
    /// there.
    #[test]
    fn an_entry_without_its_end_is_not_read() {
        let mut undo = Undo {
            entries: Vec::new(),
        };
        undo.add_many(
            creature::SPAWN,
            &Key::one("guid", 1),
            &["DELETE FROM `creature` WHERE `guid` = 1;".into(), "INSERT INTO `creature` …;".into()],
        );
        undo.add_many(
            creature::SPAWN,
            &Key::one("guid", 2),
            &["DELETE FROM `creature` WHERE `guid` = 2;".into(), "INSERT INTO `creature` …;".into()],
        );
        let whole: String = undo.entries.iter().map(Undo::entry_text).collect();
        assert_eq!(Undo::from_text(&whole).entries, undo.entries);
        // The second entry torn after its `DELETE`.
        let cut = whole.find("INSERT INTO `creature` …;\n-- end\n\n-- row creature guid=2").unwrap();
        let torn = format!(
            "{}-- row creature guid=2\nDELETE FROM `creature` WHERE `guid` = 2;\n",
            &whole[..cut + "INSERT INTO `creature` …;\n-- end\n\n".len()]
        );
        let read = Undo::from_text(&torn);
        assert_eq!(read.entries.len(), 1);
        assert!(read.covers(creature::SPAWN, &Key::one("guid", 1)));
        assert!(!read.covers(creature::SPAWN, &Key::one("guid", 2)));
    }

    /// The same id in two tables is two rows, and so is the same entry at
    /// two patches.
    #[test]
    fn the_table_and_the_whole_key_are_what_an_undo_covers() {
        let mut undo = Undo {
            entries: Vec::new(),
        };
        let key = Key::two(("entry", 68), ("patch", 0));
        undo.add(creature::TEMPLATE, &key, "UPDATE `creature_template` …;");
        assert!(undo.covers(creature::TEMPLATE, &key));
        assert!(!undo.covers(creature::SPAWN, &key));
        assert!(!undo.covers(creature::TEMPLATE, &Key::two(("entry", 68), ("patch", 3))));
    }

    /// A comment that is not a marker carries no statement with it, so the
    /// file's own header cannot become an entry.
    #[test]
    fn a_comment_is_not_an_entry() {
        assert!(Undo::from_text("-- just a note\nUPDATE x;\n")
            .entries
            .is_empty());
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
        assert!(plan.paths.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(
            plan.refused[0].contains("removes that spawn"),
            "{:?}",
            plan.refused
        );
        // The same path on a spawn the project is not removing is written.
        let plan = plan_from(&Default::default(), &paths);
        assert_eq!(plan.paths.len(), 1);
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
            table: creature::TEMPLATE,
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
            table: creature::SPAWN,
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
            table: creature::SPAWN,
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
