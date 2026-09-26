//! What a project changes about the server's items, as SQL, applied when a
//! person asks — and, unlike the creature half, **live on a reload**.
//!
//! ## The same shape as [`super::creatures`], and one real difference
//!
//! Everything about how an edit is stored, batched, written and undone is that
//! module's and is unchanged here: the project's own store accumulates typed
//! values, a save writes the SQL, nothing reaches the database until **Apply**
//! is pressed, and the statement that puts each row back is written from the
//! database immediately before anything runs.
//!
//! What is different is what happens after the apply. A creature change needs
//! the server restarted, because `.reload creature_template` does not restat
//! creatures that are already spawned and `.reload creature` never erases a
//! spawn it has read. An item change needs nothing:
//! `HandleReloadItemTemplate` calls `ObjectMgr::LoadItemPrototypes`, which
//! **clears the prototype map before it reads** (`ObjectMgr.cpp:3792`), and
//! `Item::GetProto` is a lookup by entry on every call (`Item.cpp:560`) rather
//! than a pointer taken when the item was made. So an edited row is live for
//! every copy of that item in the world, in a bag and on the auction house,
//! with no restart and no relog.
//!
//! This is therefore the first subject in this editor that can be **edited
//! while a playtest is running and seen without leaving it**, which is what the
//! spell half already had through a different route (a DBC the client re-reads)
//! and what the creature half cannot have at all.
//!
//! **Applied from the Server panel, or by a save when the switch is on** —
//! [`crate::server::save`] and [`crate::ui::sync`], which are the same two
//! doors every other subject here goes through.
//!
//! [`apply`] asks for that reload itself, deferred: sent on the next frame when
//! a playtest is running, and dropped with a line in the log when nothing is —
//! which is what every other apply in this directory does. See
//! [`super::reload::Reloads::when_there_is_a_session`].
//!
//! ## Three files, and they are this half's own
//!
//! ```text
//! sql\items.sql         what this project does to the server's items
//! sql\items-revert.sql  …and what puts those rows back
//! server\rows.txt       …and the store both are written from, shared
//! ```
//!
//! The **store** is shared with the creature half because it is one file of
//! every server row a project changes, keyed by table; the two **SQL** files
//! are separate because the two subjects are applied by two gestures and one
//! file holding both would be a file whose Apply applied the other one too.
//! Each writer skips the rows the other owns rather than refusing them — see
//! [`plan_from`].
//!
//! ## A removal is applied without the reload
//!
//! An item can be removed, and the removal takes every content-patch version
//! and the rows that hand the item out — `vale_mangos::item::DEPENDENTS`.
//! What it cannot have is the reload: an `Item` already loaded in the world
//! whose prototype has gone answers `nullptr` from `GetProto()`, and the callers
//! of it dereference without asking. A restart is safe, because each loader of
//! a character's items deletes one with no prototype. So an apply whose plan
//! removes anything asks for no reload and says the server has to be
//! restarted — see `vale_mangos::item::reload_is_safe`. That holds for every
//! apply while the removal is in the plan, since any reload after it is the
//! same reload.

use crate::session::EditSession;
use vale_mangos::conn::Db;
use vale_mangos::item;
use vale_mangos::row::{self, Assignment, Key, Life};
use bevy::prelude::*;

pub use super::creatures::Undo;

/// …as SQL.
pub const SQL_VPATH: &str = "sql\\items.sql";

/// …and what puts it back.
pub const REVERT_VPATH: &str = "sql\\items-revert.sql";

/// The table `.reload` is asked for after an apply.
pub const RELOAD_TABLE: &str = item::TEMPLATE;

/// One row's worth of change: what is to become of it, and the columns it sets.
#[derive(Debug, Clone)]
pub struct Row {
    pub table: &'static str,
    pub key: Key,
    pub life: Life,
    pub changes: Vec<Assignment>,
    /// **Where the database has the row**, which is [`Self::key`] unless the
    /// project changes the item's entry — see
    /// `vale_mangos::row::RowEdit::from`.
    pub at: Key,
}

impl Row {
    /// The statements it becomes: one `UPDATE`, the `DELETE`/`INSERT` pair a
    /// creation is, or the move of an item whose entry changes with every
    /// column that names it — see `vale_mangos::item::REFERENCES`.
    pub fn statements(&self) -> Vec<String> {
        if self.moves() {
            return row::move_statements(
                self.table,
                &self.at,
                &self.key,
                &self.changes,
                &item::REFERENCES,
            );
        }
        item::statements(self.table, &self.key, self.life, &self.changes)
    }

    /// Whether the project changes this item's entry.
    pub fn moves(&self) -> bool {
        self.life == Life::Update && self.at != self.key
    }

    /// How it is named in a file's marker and in a status line.
    pub fn names(&self) -> String {
        format!("{} {}", self.table, self.key.text())
    }
}

/// Everything a project changes about the server's items.
#[derive(Debug, Default)]
pub struct Plan {
    pub rows: Vec<Row>,
    /// **What the store asked for and this writer would not write**, each as a
    /// sentence — see [`super::creatures::Plan::refused`], where the reason is.
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

    /// Whether `.reload item_template` may follow an apply of this plan — see
    /// the module comment.
    pub fn reload_is_safe(&self) -> bool {
        item::reload_is_safe(self.rows.iter().map(|row| &row.life))
    }

    /// One line for a panel.
    pub fn line(&self) -> String {
        let (new, edited, removed) = self.counts();
        let mut parts: Vec<String> = Vec::new();
        if new > 0 {
            parts.push(format!("{new} new"));
        }
        if edited > 0 {
            parts.push(format!("{edited} edited"));
        }
        if removed > 0 {
            parts.push(format!("{removed} removed"));
        }
        match parts.is_empty() {
            true => "no item edits".to_string(),
            false => format!("{} item(s): {}", self.rows.len(), parts.join(", ")),
        }
    }

    /// **What this plan would do to the database, as one number** — see
    /// [`super::creatures::Plan::signature`], whose purpose this is one table
    /// along.
    pub fn signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for statement in self.statements() {
            statement.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Every statement this plan comes to.
    pub fn statements(&self) -> Vec<String> {
        self.rows.iter().flat_map(Row::statements).collect()
    }
}

/// **What the project's store comes to**, as statements.
pub fn plan(session: &EditSession) -> Plan {
    plan_from(&session.server_edits)
}

/// …and the same over the store alone, so what it comes to can be checked
/// without a session, a project folder or a database.
pub fn plan_from(edits: &vale_mangos::row::Edits) -> Plan {
    let mut out = Plan::default();
    for (table, key, row) in edits.rows() {
        let Some(table) = item::table_named(table) else {
            // Another subject's row — the creature half's, or a table nothing
            // here writes. Skipped in silence either way: this writer is not
            // the one that knows whether `creature` is a table somebody can
            // edit, and saying so twice would put the same complaint on two
            // panels. See `super::creatures::plan_from`, which does the same
            // in the other direction.
            continue;
        };
        if !item::can_live(table, row.life) {
            out.refused.push(format!(
                "{table} {} cannot be {}d — this editor writes that only for \
                 item_template",
                key.text(),
                row.life.word()
            ));
            continue;
        }
        let mut changes = Vec::new();
        for (column, value) in &row.columns {
            let Some(known) = item::column(table, column) else {
                out.refused.push(format!(
                    "{table}.{column} is not a column this editor writes"
                ));
                continue;
            };
            // A key column is what the `WHERE` names and is never a column of
            // an edit. An item's entry is changed by re-keying its claim — see
            // `crate::tools::items::Items::rekey` — and a store written before
            // that held the move as a column is read back as a re-key, so a key
            // column here is a line somebody typed.
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
            // **A creation names every column or it is not written**, on
            // `super::creatures`' own rule: a column an `INSERT` leaves out
            // takes the table's default, which for this table is a row the
            // server loads and the client draws as nothing.
            Life::Insert => {
                let missing: Vec<&str> = item::columns_of(table)
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
            // A removal carries no columns, and one that had any would be
            // columns of a row that is not there afterwards.
            Life::Delete => changes.clear(),
            Life::Update => {
                if changes.is_empty() && row.from.is_none() {
                    continue;
                }
            }
        }
        let at = row.from.clone().unwrap_or_else(|| key.clone());
        out.rows.push(Row {
            table,
            key: key.clone(),
            life: row.life,
            changes,
            at,
        });
    }
    out
}

/// **Write `sql\items.sql`**, or remove it when the project changes nothing.
///
/// Called by every save. It writes a file and touches no database.
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
        "-- {} — what this project changes about the server's items.\n\
         -- Rewritten on every save from the project's own edits, so it is the\n\
         -- whole of what the project does rather than an increment of it.\n\
         --\n\
         -- Nothing applies this by itself. Apply it from the editor's item\n\
         -- workspace, or run it by hand and then say `.reload item_template`:\n\
         -- LoadItemPrototypes clears its map before it reads, and Item::GetProto\n\
         -- is a lookup per call, so an item change is live with no restart.\n\n",
        session.project.name
    );
    for row in &plan.rows {
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

/// **The item half of a save**: the project's store, and the SQL it comes to.
///
/// The store itself is written by [`super::creatures::save`], which is called
/// on the same save and writes the one file both halves keep their rows in.
/// This writes only the SQL.
pub fn save(session: &mut EditSession) {
    match write_sql(session) {
        Ok(0) => {}
        Ok(rows) => session.status = format!("{rows} item change(s) written to {SQL_VPATH}"),
        Err(e) => {
            warn!("items: {e}");
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
    /// put back and not written again — see [`super::reconcile`].
    pub taken_back: usize,
    /// **Whether the reload was asked for.** Not when the plan removes an item,
    /// which is live only once the server has been restarted.
    pub reloaded: bool,
}

impl Applied {
    pub fn line(&self) -> String {
        let taken_back = match self.taken_back {
            0 => String::new(),
            n => format!(", {n} no longer claimed put back"),
        };
        let then = match self.reloaded {
            true => "reloading item_template",
            false => "an item is removed, so restart the server rather than reload",
        };
        format!(
            "{} item row(s) applied, {} affected{taken_back} — {then}",
            self.rows, self.affected
        )
    }
}

/// **An Apply, with everything it needs to run off the main thread**: the
/// plan, the project handle its revert file is written through, and where the
/// database is. See [`super::queue`].
pub struct ApplyJob {
    plan: Plan,
    project: vale_edit::project::Project,
    at: vale_mangos::conn::Where,
    /// The project's store as it stood when the Apply was asked for — see
    /// [`super::reconcile::refuse_a_taken_id`].
    own: vale_mangos::row::Edits,
    /// Whether the plan may be made live by a reload — see
    /// [`Applied::reloaded`]. Decided here, from the plan, because the finish
    /// needs it whatever the run answered.
    reloaded: bool,
}

/// …and what running one answered, for the main thread's half.
pub struct ApplyDone {
    signature: u64,
    reloaded: bool,
    pub result: Result<Applied, String>,
}

impl ApplyDone {
    /// What the status line says about it.
    pub fn line(&self) -> String {
        match &self.result {
            Ok(done) if done.rows == 0 && done.taken_back == 0 => "nothing to apply".to_string(),
            Ok(done) => done.line(),
            Err(e) => format!("items: {e}"),
        }
    }
}

/// **The main thread's first half of an Apply**: the plan, and whether there is
/// anything to do. `None` when the project claims no item and has applied none.
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
    let reloaded = plan.reload_is_safe();
    Ok(Some(ApplyJob {
        plan,
        project: session.project.clone(),
        at,
        own: session.server_edits.clone(),
        reloaded,
    }))
}

impl ApplyJob {
    /// **The worker's half**: the database and the revert file, in
    /// [`super::reconcile`]'s order, which is the whole of the safety — what
    /// the project applied before is put back, and then each row is read as it
    /// stands, its undo written, and its own statements run.
    pub fn run(self) -> ApplyDone {
        let signature = self.plan.signature();
        let result = (|| {
            let mut db = Db::open(&self.at)?;
            let steps: Vec<super::reconcile::Step> = self
                .plan
                .rows
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
                "the server's items",
                &steps,
                |db, index| undo_of_a_row(db, &self.plan.rows[index], &self.own),
            )?;
            Ok(Applied {
                rows: self.plan.rows.len(),
                affected: done.affected,
                newly_undoable: done.undoable,
                taken_back: done.taken_back,
                reloaded: self.reloaded,
            })
        })();
        ApplyDone {
            signature,
            reloaded: self.reloaded,
            result,
        }
    }
}

/// **The main thread's second half**: what the editor keeps about the
/// database, brought up to date.
///
/// The table has moved whether or not the run succeeded, so what the tool read
/// out of it is stale either way — and the server's copy with it, which is why
/// the reload is asked for in both cases, *deferred*: sent on the next frame
/// when there is a session, and dropped with a line in the log when there is
/// not. See `super::reload::Reloads::when_there_is_a_session`. The signature is
/// kept only for a run that finished, so a failure leaves the project saying it
/// has applied nothing rather than claiming a change that did not land.
pub fn finish_apply(
    session: &mut EditSession,
    reloads: &mut super::reload::Reloads,
    done: &ApplyDone,
) {
    session.item_writes += 1;
    session.applied_items = done.result.as_ref().ok().map(|_| done.signature);
    // **And it is live from here**, which is the whole of what separates this
    // from the creature half — see the module comment — unless the plan
    // removes an item.
    if done.reloaded {
        reloads.when_there_is_a_session(RELOAD_TABLE);
    }
}

/// **An Apply as one step of [`super::stack`]**, which is the only way it
/// runs: `None` when there is nothing to apply.
pub fn apply_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_apply(session, server)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new("applying items", move || {
        let done = job.run();
        let ok = done.result.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            finish_apply(session, reloads, &done);
            session.status = done.line();
        });
        (ok, finish)
    })))
}

/// **One row's undo, read immediately before the row is written** — the
/// subject's half of [`super::reconcile`]'s step 3.
fn undo_of_a_row(
    db: &mut Db,
    row: &Row,
    own: &vale_mangos::row::Edits,
) -> Result<Option<Vec<String>>, String> {
    if row.life == Life::Insert || row.moves() {
        super::reconcile::refuse_a_taken_id(db, row.table, &row.key, &item::REFERENCES, own)?;
    }
    Ok(match row.life {
        // A row this project creates is undone by removing it, which is
        // exactly unmaking what the `INSERT` made — the row is this project's
        // own and was not there before, which the check above establishes.
        Life::Insert => Some(vec![row::delete(row.table, &row.key)]),
        // `None` when the row is not there. A key that matches nothing names a
        // row somebody else removed, and inventing an `INSERT` for it would
        // write a row this project never saw.
        Life::Update => {
            let now = db.row(&item::row_query(row.table, &row.at))?;
            super::reconcile::undo_of_an_update(
                row.table,
                &row.at,
                &row.key,
                &row.changes,
                &item::REFERENCES,
                now.as_ref(),
            )?
        }
        // `None` when the item is not in the database: nothing to put back.
        Life::Delete => snapshot_the_item(db, &row.key)?,
    })
}

/// **Everything a removed item takes with it, as the statements that put it
/// back**: every content-patch version of the template, then the rows of each
/// of `item::DEPENDENTS`. `None` when the item is not in the database.
///
/// The `DELETE`s come first, so the entry can be run against an item it has
/// already restored — see [`super::reconcile`].
fn snapshot_the_item(db: &mut Db, key: &Key) -> Result<Option<Vec<String>>, String> {
    let Some(entry) = key.first() else {
        return Ok(None);
    };
    let versions = db.rows(&format!(
        "SELECT * FROM {} WHERE `entry` = {entry}",
        vale_mangos::sql::name(item::TEMPLATE)
    ))?;
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

/// **A Put back, with everything it needs to run off the main thread.**
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
    /// The worker's half: run the revert file, and forget it. Answers how many
    /// rows it covered.
    pub fn run(self) -> Result<usize, String> {
        let mut db = Db::open(&self.at)?;
        super::reconcile::put_back(&self.project, &mut db, REVERT_VPATH)
    }
}

/// The main thread's second half: nothing this project says is in the
/// database any more, the table has moved, and the server's copy with it.
pub fn finish_revert(session: &mut EditSession, reloads: &mut super::reload::Reloads) {
    session.applied_items = None;
    session.item_writes += 1;
    reloads.when_there_is_a_session(RELOAD_TABLE);
}

/// …and a Put back as one: `None` when this project has applied nothing.
pub fn revert_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_revert(session, server)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new("putting back items", move || {
        let done = job.run();
        let ok = done.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            session.status = match done {
                Ok(rows) => {
                    finish_revert(session, reloads);
                    format!("{rows} item row(s) put back")
                }
                Err(e) => format!("items: {e}"),
            };
        });
        (ok, finish)
    })))
}

/// **What of this project is in the database**, as the two questions a panel
/// asks — [`super::creatures::OnTheServer`] for the item half.
pub struct OnTheServer {
    undo: Undo,
    current: bool,
}

impl OnTheServer {
    /// The revert file alone.
    pub fn read(session: &EditSession) -> OnTheServer {
        OnTheServer {
            undo: Undo::open_at(&session.project, REVERT_VPATH).unwrap_or(Undo {
                entries: Vec::new(),
            }),
            current: false,
        }
    }

    /// …and the same with [`Self::current`] answered, which needs the plan.
    pub fn read_with(session: &EditSession, plan: &Plan) -> OnTheServer {
        OnTheServer {
            current: session
                .applied_items
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

    /// …and whether what it says now is what it applied. `false` after a
    /// relaunch even for a project that applied everything last session, so a
    /// caller uses it to *add* a caveat and never to take one away.
    pub fn current(&self) -> bool {
        self.current
    }
}

/// **`--apply-items` and `--revert-items`**, which are the workspace's two
/// buttons with nobody at the keyboard — see [`crate::Args::apply_items`].
///
/// Fires once, on the first frame there is a session to read the project out
/// of, and after any scripted edit has landed. A flag that fired repeatedly
/// would re-apply after every save.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    server: Res<super::settings::ServerSettings>,
    mut reloads: ResMut<super::reload::Reloads>,
    items: Res<crate::tools::items::Items>,
    mut done: Local<bool>,
) {
    if *done || !(args.apply_items || args.revert_items) {
        return;
    }
    // **Wait for a scripted edit to land**, on `super::creatures`' own rule:
    // this fires on the first frame there is a session and `--item-new` fires
    // when the table read has come back, several seconds later. Without it the
    // apply ran against an empty store and said so.
    if !items.scripted_done {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    // The store is written first, so the flag applies what the project holds
    // rather than what its last save happened to have written.
    super::creatures::save(&mut session);
    save(&mut session);
    // **Through the stack**, as the Server panel's buttons are — see
    // [`super::stack`] — so a flag puts back and applies the later subjects
    // with this one.
    let wanted = [
        (args.revert_items, "--revert-items", super::stack::Wanted::PutBack(super::stack::Subject::Items)),
        (args.apply_items, "--apply-items", super::stack::Wanted::Apply(super::stack::Subject::Items)),
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
    /// key — the entry *and* the patch, which is what stops an edit changing
    /// every content-patch version of the item.
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

    /// **A created item is a `DELETE` of its key and an `INSERT` naming every
    /// column**, so applying twice means the same as applying once.
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

    /// **A creation that does not name every column is not written**, and the
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

    /// **A removal is a row of the plan and forbids the reload** — see the
    /// module comment, and `Item::GetProto`.
    #[test]
    fn a_removal_is_written_and_asks_for_a_restart() {
        let mut edits = Edits::default();
        let key = item::template_key(2589, 0);
        edits.set_life(item::TEMPLATE, &key, Life::Delete);
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (0, 0, 1));
        assert!(!plan.reload_is_safe());
        let sql = plan.statements();
        assert_eq!(sql[0], "DELETE FROM `item_template` WHERE `entry` = 2589;");
        assert_eq!(sql.len(), 1 + item::DEPENDENTS.len());
        assert_eq!(plan.line(), "1 item(s): 1 removed");
    }

    /// **An item whose entry changes is one claim at the new entry**, and it
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

    /// …and a move with no column edit beside it is still a row of the plan: the
    /// claim says nothing but where the row goes.
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

    /// **A created row re-keyed is created at the new entry and nowhere else**,
    /// which is the report this round began with: the old entry stayed in the
    /// database because the store no longer named it and nothing took it back.
    /// The store half is here; taking the old row back is
    /// `super::reconcile`'s, which puts back everything before it applies.
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

    /// **The two halves of the store do not complain about each other.** One
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

        let creatures = super::super::creatures::plan_from(&edits, &Default::default());
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
