//! What a project changes about the server's game objects, as SQL, applied when
//! a person asks.
//!
//! ## The order an apply runs in, shared with [`super::creatures`]
//!
//! The project's store accumulates typed values, a save writes the SQL, nothing
//! reaches the database until Apply is pressed, and the statements that put
//! each row back are read from the database immediately before that row is
//! written. [`super::reconcile`] holds that order, and every subject here
//! applies in it.
//!
//! A game object walks no path, so the plan is rows alone.
//!
//! ## Why an apply asks for a restart rather than a reload
//!
//! `.reload gameobject` calls `LoadGameobjects(true)`, which adds a spawn it has
//! not seen to its grid and never erases one. `.reload gameobject_template`
//! rewrites each `GameObjectInfo` in place, but an object already in the world
//! took its display id, faction, flags and size from the template when it was
//! created. So an apply sends no reload and its line says to restart the
//! server. See `vale_mangos::gameobject`, where the source lines are.
//!
//! ## The three files this subject reads and writes
//!
//! ```text
//! sql\gameobjects.sql         what this project does to the server's game objects
//! sql\gameobjects-revert.sql  the statements that put those rows back
//! server\rows.txt             the store both are written from, shared
//! ```
//!
//! The store is the one file every server subject keeps its rows in, keyed by
//! table; each writer skips the rows the others own. The SQL files are this
//! subject's own because it is applied by its own gesture.

use crate::session::EditSession;
use vale_mangos::conn::Db;
use vale_mangos::gameobject;
use vale_mangos::row::{self, Assignment, Key, Life};
use bevy::prelude::*;

pub use super::creatures::Undo;

/// The project path of the SQL file a save writes.
pub const SQL_VPATH: &str = "sql\\gameobjects.sql";

/// The project path of the revert file an apply writes.
pub const REVERT_VPATH: &str = "sql\\gameobjects-revert.sql";

/// What the revert file's header calls the subject.
const SUBJECT: &str = "the server's game objects";

/// One row's worth of change.
#[derive(Debug, Clone)]
pub struct Row {
    pub table: &'static str,
    pub key: Key,
    pub life: Life,
    pub changes: Vec<Assignment>,
    /// Where the database has the row: [`Self::key`] unless the project
    /// changes the row's id. See `vale_mangos::row::RowEdit::from`.
    pub at: Key,
}

impl Row {
    /// The statements it becomes: one `UPDATE`, a `DELETE` and an `INSERT`, the
    /// three `DELETE`s a spawn and its two dependent tables come to, or the
    /// move of a row whose id changes with everything that names it.
    pub fn statements(&self) -> Vec<String> {
        if self.moves() {
            return row::move_statements(
                self.table,
                &self.at,
                &self.key,
                &self.changes,
                gameobject::references(self.table),
            );
        }
        gameobject::statements(self.table, &self.key, self.life, &self.changes)
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

/// Everything a project changes about the server's game objects.
#[derive(Debug, Default)]
pub struct Plan {
    pub rows: Vec<Row>,
    /// What the store asked for and this writer would not write, each as a
    /// sentence. See `super::creatures::Plan::refused`.
    pub refused: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
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
        match parts.is_empty() {
            true => "no game object edits".to_string(),
            false => format!("{} row(s): {}", self.rows.len(), parts.join(", ")),
        }
    }

    /// What this plan would do to the database, as one number: a hash of
    /// every statement it comes to. See `super::creatures::Plan::signature`.
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

/// What the project's store comes to, as statements.
pub fn plan(session: &EditSession) -> Plan {
    plan_from(&session.server_edits)
}

/// The same over the store alone, so what it comes to can be checked without
/// a session, a project folder or a database.
pub fn plan_from(edits: &vale_mangos::row::Edits) -> Plan {
    let mut out = Plan::default();
    for (table, key, row) in edits.rows() {
        // Another subject's row is skipped without a refusal: the writer that
        // owns it is the one that says whether it can be written.
        let Some(table) = gameobject::table_named(table) else {
            continue;
        };
        if !gameobject::can_live(table, row.life) {
            out.refused.push(format!(
                "{table} {} cannot be {}d, only edited",
                key.text(),
                row.life.word()
            ));
            continue;
        }
        let mut changes = Vec::new();
        for (column, value) in &row.columns {
            let Some(known) = gameobject::column(table, column) else {
                out.refused
                    .push(format!("{table}.{column} is not a column this editor writes"));
                continue;
            };
            if !known.editable() {
                out.refused
                    .push(format!("{table}.{column} is part of the key and is not written"));
                continue;
            }
            changes.push(Assignment { column: known.name, value: value.clone() });
        }
        match row.life {
            // A creation names every column or it is not written. The table's
            // default `animprogress` and respawn time are both 0, which is an
            // object that despawns on use and never returns.
            Life::Insert => {
                let missing: Vec<&str> = gameobject::columns_of(table)
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
        // A removal is of the row where the database has it. See
        // `super::creatures::plan_from`.
        let key = match row.life {
            Life::Delete => at.clone(),
            _ => key.clone(),
        };
        out.rows.push(Row { table, key, life: row.life, changes, at });
    }
    out
}

/// Write `sql\gameobjects.sql`, or remove it when the project changes
/// nothing. Called by every save; it writes a file and touches no database.
pub fn write_sql(session: &mut EditSession) -> Result<usize, String> {
    let plan = plan(session);
    for refused in &plan.refused {
        warn!("{}: {refused}", super::creatures::EDITS_VPATH);
    }
    if plan.is_empty() {
        remove(&session.project, SQL_VPATH);
        return Ok(0);
    }
    let mut body = format!(
        "-- {} — what this project changes about the server's game objects.\n\
         -- Rewritten on every save from the project's own edits, so it is the\n\
         -- whole of what the project does rather than an increment of it.\n\
         --\n\
         -- Nothing applies this by itself. Apply it from the editor's Server\n\
         -- panel, or run it by hand, and then RESTART the server: .reload\n\
         -- gameobject never erases a spawn it has read, and an object already in\n\
         -- the world keeps the display id, faction, flags and size it was\n\
         -- created with.\n\n",
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
    session
        .project
        .write(SQL_VPATH, body.as_bytes())
        .map_err(|e| format!("{SQL_VPATH}: {e}"))?;
    Ok(plan.rows.len())
}

/// The game-object half of a save: the SQL the store comes to.
///
/// The store itself is written by [`super::creatures::save`], which is called
/// on the same save and writes the one file every subject keeps its rows in.
pub fn save(session: &mut EditSession) {
    match write_sql(session) {
        Ok(0) => {}
        Ok(rows) => {
            session.status = format!("{rows} game object change(s) written to {SQL_VPATH}")
        }
        Err(e) => {
            warn!("game objects: {e}");
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
            "{} game object row(s) applied, {} affected{taken_back} — restart the server to \
             see them",
            self.rows, self.affected
        )
    }
}

/// An Apply, with everything it needs to run off the main thread. See
/// [`super::items::ApplyJob`], which has the same three halves.
pub struct ApplyJob {
    plan: Plan,
    project: vale_edit::project::Project,
    at: vale_mangos::conn::Where,
    /// The project's store as it stood when the Apply was asked for. See
    /// [`super::reconcile::refuse_a_taken_id`].
    own: vale_mangos::row::Edits,
}

/// What running an [`ApplyJob`] answered.
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
            Err(e) => format!("game objects: {e}"),
        }
    }
}

/// The main thread's first half of an Apply. `None` when the project claims no
/// game-object row and has applied none.
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
    /// The worker's half of an Apply, in [`super::reconcile::apply`]'s order.
    /// Sends no reload; the module comment says why.
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
                SUBJECT,
                &steps,
                |db, index| undo_of_a_row(db, &self.plan.rows[index], &self.own),
            )?;
            Ok(Applied {
                rows: self.plan.rows.len(),
                affected: done.affected,
                newly_undoable: done.undoable,
                taken_back: done.taken_back,
            })
        })();
        ApplyDone { signature, result }
    }
}

/// The main thread's second half of an Apply. The database has moved whether
/// or not the run finished, so what the tool read out of it is stale either way.
pub fn finish_apply(session: &mut EditSession, done: &ApplyDone) {
    session.wrote_the_database();
    session.applied_gameobjects = done.result.as_ref().ok().map(|_| done.signature);
}

/// An Apply as one step of [`super::stack`], which is the only way it runs.
/// `None` when there is nothing to apply.
pub fn apply_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_apply(session, server)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new("applying game objects", move || {
        let done = job.run();
        let ok = done.result.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, _: &mut super::reload::Reloads| {
            finish_apply(session, &done);
            session.status = done.line();
        });
        (ok, finish)
    })))
}

/// One row's undo, read immediately before the row is written.
fn undo_of_a_row(
    db: &mut Db,
    row: &Row,
    own: &vale_mangos::row::Edits,
) -> Result<Option<Vec<String>>, String> {
    let references = gameobject::references(row.table);
    if row.life == Life::Insert || row.moves() {
        let limit = match row.table {
            gameobject::SPAWN => gameobject::MAX_GUID,
            _ => u64::from(gameobject::MAX_ENTRY),
        };
        super::reconcile::refuse_a_guid_past_the_limit(row.table, &row.key, limit)?;
        super::reconcile::refuse_a_taken_id(db, row.table, &row.key, references, own)?;
    }
    Ok(match row.life {
        // A row this project creates is undone by removing it: a spawn with
        // its two dependent tables, a template on its own.
        Life::Insert => Some(gameobject::insert_undo_statements(row.table, &row.key)),
        // A row this project removes is undone by putting back every row the
        // `DELETE`s take. `None` when the row is not there: there is nothing
        // to put back.
        Life::Delete => snapshot_the_spawn(db, &row.key)?,
        Life::Update => {
            let now = db.row(&gameobject::row_query(row.table, &row.at))?;
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
/// The `DELETE`s come first, so the entry can be run against a spawn it has
/// already restored, which is [`super::reconcile`]'s rule. The `gameobject`
/// row follows, then each dependent table's rows in
/// `gameobject::DEPENDENTS`' order.
fn snapshot_the_spawn(db: &mut Db, key: &Key) -> Result<Option<Vec<String>>, String> {
    let Some(spawn) = db.row(&gameobject::row_query(gameobject::SPAWN, key))? else {
        return Ok(None);
    };
    let Some(guid) = key.first() else {
        return Ok(None);
    };
    let mut out = gameobject::delete_statements(key);
    out.extend(row::insert_from_row(gameobject::SPAWN, &spawn));
    for (table, column) in gameobject::DEPENDENTS {
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
    /// The worker's half: run the revert file, and forget it.
    pub fn run(self) -> Result<usize, String> {
        let mut db = Db::open(&self.at)?;
        super::reconcile::put_back(&self.project, &mut db, REVERT_VPATH)
    }
}

/// The main thread's second half.
pub fn finish_revert(session: &mut EditSession) {
    session.applied_gameobjects = None;
    session.wrote_the_database();
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
    Ok(Some(super::stack::Step::new("putting back game objects", move || {
        let done = job.run();
        let ok = done.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, _: &mut super::reload::Reloads| {
            session.status = match done {
                Ok(rows) => {
                    finish_revert(session);
                    format!(
                        "put back {rows} game object row(s) \u{2014} restart the server. This \
                         project still changes them; Discard gives them up"
                    )
                }
                Err(e) => format!("game objects: {e}"),
            };
        });
        (ok, finish)
    })))
}

/// What of this project is in the database: [`super::creatures::OnTheServer`]
/// for this subject.
pub struct OnTheServer {
    undo: Undo,
    current: bool,
}

impl OnTheServer {
    /// The revert file alone.
    pub fn read(session: &EditSession) -> OnTheServer {
        OnTheServer {
            undo: Undo::open_at(&session.project, REVERT_VPATH).unwrap_or(Undo { entries: Vec::new() }),
            current: false,
        }
    }

    /// The same with [`Self::current`] answered, which needs the plan.
    pub fn read_with(session: &EditSession, plan: &Plan) -> OnTheServer {
        OnTheServer {
            current: session
                .applied_gameobjects
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

    /// Whether what it says now is what it applied. `false` after a relaunch,
    /// so a caller uses it to add a caveat and never to take one away.
    pub fn current(&self) -> bool {
        self.current
    }
}

/// Take a file out of the project, if it is there.
fn remove(project: &vale_edit::project::Project, vpath: &str) {
    if let Some(disk) = project.path_for(vpath) {
        let _ = std::fs::remove_file(disk);
    }
}

/// `--apply-gameobjects` and `--revert-gameobjects`, which are the Server
/// panel's two buttons with nobody at the keyboard.
///
/// Fires once, after any scripted placement or removal has landed. See
/// `crate::tools::gameobjects::GameObjects::scripted_done`.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    server: Res<super::settings::ServerSettings>,
    mut reloads: ResMut<super::reload::Reloads>,
    objects: Res<crate::tools::gameobjects::GameObjects>,
    mut done: Local<bool>,
) {
    if *done || !(args.apply_gameobjects || args.revert_gameobjects) {
        return;
    }
    if !objects.scripted_done || !objects.scripted_template_done {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    // The store is written first, so the flag applies what the project holds
    // rather than what its last save happened to have written.
    super::creatures::save(&mut session);
    save(&mut session);
    // Both run through the stack, as the Server panel's buttons do, so a flag
    // puts back and applies the later subjects with this one. See
    // [`super::stack`].
    let wanted = [
        (args.revert_gameobjects, "--revert-gameobjects", super::stack::Wanted::PutBack(super::stack::Subject::GameObjects)),
        (args.apply_gameobjects, "--apply-gameobjects", super::stack::Wanted::Apply(super::stack::Subject::GameObjects)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use vale_mangos::row::{Edits, RowEdit};

    /// A whole new spawn, as the tool writes one.
    fn a_new_spawn(guid: u64) -> (Key, RowEdit) {
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in gameobject::new_spawn(1731, 0, (-9400.0, -100.0, 60.0), 1.5) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        (Key::one("guid", guid), row)
    }

    fn store(rows: Vec<(Key, RowEdit)>) -> Edits {
        let mut edits = Edits::default();
        for (key, row) in rows {
            edits.set_row_line(gameobject::SPAWN, &key, Some(&row.to_line()));
        }
        edits
    }

    /// A created spawn is a `DELETE` of its key and an `INSERT` naming every
    /// column, which is `GameObject::SaveToDB`'s own pair.
    #[test]
    fn a_created_spawn_is_a_delete_and_an_insert() {
        let (key, row) = a_new_spawn(10_000_001);
        let plan = plan_from(&store(vec![(key, row)]));
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (1, 0, 0));
        let sql = plan.statements();
        assert_eq!(sql.len(), 2);
        assert_eq!(sql[0], "DELETE FROM `gameobject` WHERE `guid` = 10000001;");
        assert!(sql[1].starts_with("INSERT INTO `gameobject` (`guid`, "), "{}", sql[1]);
    }

    /// A creation that does not name every column is not written, and the
    /// refusal names the column.
    #[test]
    fn a_created_spawn_missing_a_column_is_refused() {
        let (key, mut row) = a_new_spawn(10_000_001);
        row.columns.remove("animprogress");
        let plan = plan_from(&store(vec![(key, row)]));
        assert!(plan.rows.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(plan.refused[0].contains("animprogress"), "{:?}", plan.refused);
    }

    /// A removed spawn is the row and its two dependent tables.
    #[test]
    fn a_removed_spawn_is_three_deletes() {
        let mut edits = Edits::default();
        edits.set_life(gameobject::SPAWN, &Key::one("guid", 29993), Life::Delete);
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (0, 0, 1));
        let sql = plan.statements();
        assert_eq!(sql.len(), 3);
        assert_eq!(
            sql.last().map(String::as_str),
            Some("DELETE FROM `gameobject` WHERE `guid` = 29993;")
        );
    }

    /// A template cannot be removed, and the plan says which row it would
    /// have been.
    #[test]
    fn a_template_removal_is_refused_by_name() {
        let mut edits = Edits::default();
        edits.set_life(
            gameobject::TEMPLATE,
            &Key::two(("entry", 1731), ("patch", 0)),
            Life::Delete,
        );
        let plan = plan_from(&edits);
        assert!(plan.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(plan.refused[0].contains("gameobject_template"), "{:?}", plan.refused);
    }

    /// A template this project creates is planned as a `DELETE` of its key
    /// and an `INSERT` naming every column, and one missing a column is
    /// refused.
    #[test]
    fn a_created_template_is_a_delete_and_an_insert() {
        let mut edits = Edits::default();
        let key = gameobject::template_key(2_000_000, 10);
        let mut row = vale_mangos::row::RowEdit { life: Life::Insert, ..Default::default() };
        for change in gameobject::new_template("Test Object") {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(gameobject::TEMPLATE, &key, Some(&row.to_line()));
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (1, 0, 0));
        let sql = plan.statements();
        assert_eq!(sql.len(), 2);
        assert_eq!(
            sql[0],
            "DELETE FROM `gameobject_template` WHERE `entry` = 2000000 AND `patch` = 10;"
        );
        assert!(sql[1].starts_with("INSERT INTO `gameobject_template` (`entry`, `patch`, "));

        let mut short = Edits::default();
        short.set_life(gameobject::TEMPLATE, &key, Life::Insert);
        short.set(gameobject::TEMPLATE, &key, "name", Some("'Half'".into()));
        let plan = plan_from(&short);
        assert!(plan.is_empty());
        assert!(plan.refused[0].contains("is created without"), "{:?}", plan.refused);
    }

    /// Another subject's rows are neither written nor refused here, and a
    /// game object's are neither in the creature writer.
    #[test]
    fn each_writer_skips_the_other_subjects_rows() {
        let mut edits = Edits::default();
        edits.set(
            vale_mangos::creature::SPAWN,
            &Key::one("guid", 19272),
            "position_z",
            Some("98.7".into()),
        );
        edits.set(
            gameobject::TEMPLATE,
            &Key::two(("entry", 1731), ("patch", 0)),
            "size",
            Some("2".into()),
        );
        let plan = plan_from(&edits);
        assert_eq!(plan.rows.len(), 1);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(
            plan.statements(),
            ["UPDATE `gameobject_template` SET `size` = 2 WHERE `entry` = 1731 AND `patch` = 0;"]
        );

        let creatures = super::super::creatures::plan_from(&edits, &Default::default());
        assert_eq!(creatures.rows.len(), 1);
        assert!(creatures.refused.is_empty(), "{:?}", creatures.refused);
    }

    /// A data column is written under its own name, whatever the type calls it.
    #[test]
    fn a_data_column_is_written_as_the_column_it_is() {
        let mut edits = Edits::default();
        edits.set(
            gameobject::TEMPLATE,
            &Key::two(("entry", 1731), ("patch", 0)),
            "data0",
            Some("39".into()),
        );
        assert_eq!(
            plan_from(&edits).statements(),
            ["UPDATE `gameobject_template` SET `data0` = 39 WHERE `entry` = 1731 AND `patch` = 0;"]
        );
    }
}
