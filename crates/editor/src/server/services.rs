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
//! `crate::tools::services`.
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

use crate::session::EditSession;
use vale_mangos::conn::Db;
use vale_mangos::row::{self, Assignment, Key, Life};
use vale_mangos::{trainer, vendor};
use bevy::prelude::*;

pub use super::creatures::Undo;

/// What this project does to the four tables, as SQL.
pub const SQL_VPATH: &str = "sql\\services.sql";

/// What puts it back.
pub const REVERT_VPATH: &str = "sql\\services-revert.sql";

/// The four tables, in the order a plan writes them.
pub const TABLES: [&str; 4] = [vendor::TEMPLATE, vendor::VENDOR, trainer::TEMPLATE, trainer::TRAINER];

/// The two reload commands that re-read them, in the order they are sent.
/// Each re-reads its template table first.
pub const RELOADS: [&str; 2] = [vendor::VENDOR, trainer::TRAINER];

/// The static name of one of the four tables, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// Whether a table read out of the project's store is this subject's.
pub fn owns(table: &str) -> bool {
    table_named(table).is_some()
}

/// One column of one of the four tables.
fn column(table: &str, name: &str) -> Option<&'static vale_mangos::schema::Column> {
    vendor::column(table, name).or_else(|| trainer::column(table, name))
}

/// Every column of one of the four tables.
fn columns_of(table: &str) -> &'static [vale_mangos::schema::Column] {
    match (vendor::table_named(table), trainer::table_named(table)) {
        (Some(_), _) => &vendor::COLUMNS,
        (_, Some(_)) => &trainer::COLUMNS,
        _ => &[],
    }
}

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
        vendor::statements(self.table, &self.key, self.life, &self.changes)
    }

    /// How it is named in a file's marker and in a status line.
    pub fn names(&self) -> String {
        format!("{} {}", self.table, self.key.text())
    }
}

/// Everything a project changes about the four tables.
#[derive(Debug, Default)]
pub struct Plan {
    pub rows: Vec<Row>,
    /// What the store asked for and this writer would not write, each as a
    /// sentence; see [`super::creatures::Plan::refused`].
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

    /// The rows in the order they are written: table by table in [`TABLES`]'
    /// order, removals first inside a table, then by key.
    pub fn ordered(&self) -> impl Iterator<Item = &Row> {
        let mut rows: Vec<&Row> = self.rows.iter().collect();
        rows.sort_by_key(|row| {
            (
                TABLES.iter().position(|table| *table == row.table),
                row.life != Life::Delete,
                row.key.clone(),
            )
        });
        rows.into_iter()
    }

    /// What this plan would do to the database, as one number; see
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

/// What the project's store comes to, as statements.
pub fn plan(session: &EditSession) -> Plan {
    plan_from(&session.server_edits)
}

/// The same over the store alone, so it can be checked with no session.
pub fn plan_from(edits: &row::Edits) -> Plan {
    let mut out = Plan::default();
    for (table, key, row) in edits.rows() {
        // Another subject's row, skipped as every writer skips the others'.
        let Some(table) = table_named(table) else {
            continue;
        };
        let mut changes = Vec::new();
        for (name, value) in &row.columns {
            let Some(known) = column(table, name) else {
                out.refused.push(format!("{table}.{name} is not a column this editor writes"));
                continue;
            };
            if !known.editable() {
                out.refused.push(format!("{table}.{name} is part of the key and is not written"));
                continue;
            }
            changes.push(Assignment {
                column: known.name,
                value: value.clone(),
            });
        }
        match row.life {
            // A creation names every column or it is not written, on
            // `super::creatures`' rule, and is checked as the loader checks
            // it, so a row the server would skip is refused with its reason.
            Life::Insert => {
                let missing: Vec<&str> = columns_of(table)
                    .iter()
                    .filter(|column| column.editable())
                    .map(|column| column.name)
                    .filter(|name| !changes.iter().any(|change| change.column == *name))
                    .collect();
                if !missing.is_empty() {
                    out.refused.push(format!(
                        "{table} {} is created without {} and is not written",
                        key.text(),
                        missing.join(", ")
                    ));
                    continue;
                }
                let faults = faults_of(table, key, &changes);
                if !faults.is_empty() {
                    out.refused.push(format!(
                        "{table} {} would be skipped by the server: {}; it is not written",
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

/// A created row's key and columns as one row, for the checks.
fn whole_row(key: &Key, changes: &[Assignment]) -> vale_mangos::schema::Row {
    let mut row = vale_mangos::schema::Row::new();
    for (column, value) in &key.0 {
        row.insert(column.clone(), Some(value.clone()));
    }
    for change in changes {
        row.insert(change.column.to_string(), Some(change.value.clone()));
    }
    row
}

/// Why the server would skip a created row, from the row alone.
fn faults_of(table: &str, key: &Key, changes: &[Assignment]) -> Vec<String> {
    let row = whole_row(key, changes);
    match vendor::table_named(table) {
        Some(_) => vendor::Ware::from_row(&row).map(|ware| ware.check()).unwrap_or_default(),
        None => trainer::Lesson::from_row(&row).map(|lesson| lesson.check()).unwrap_or_default(),
    }
}

/// Write `sql\services.sql`, or remove it when the project changes nothing.
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
        "-- {} — what this project changes about what creatures sell and teach.\n\
         -- Rewritten on every save from the project's own edits, so it is the\n\
         -- whole of what the project does rather than an increment of it.\n\
         --\n\
         -- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then reload both lists:\n\
         --   .reload npc_vendor     (re-reads npc_vendor_template, then npc_vendor)\n\
         --   .reload npc_trainer    (re-reads npc_trainer_template, then npc_trainer)\n\
         -- Both loaders clear their lists first, so removals are live too.\n\n",
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

/// The services half of a save: the SQL the store comes to. The store itself
/// is written by [`super::creatures::save`], on the same save.
pub fn save(session: &mut EditSession) {
    match write_sql(session) {
        Ok(0) => {}
        Ok(rows) => session.status = format!("{rows} vendor and trainer change(s) written to {SQL_VPATH}"),
        Err(e) => {
            warn!("services: {e}");
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
    /// put back and not written again; see [`super::reconcile`].
    pub taken_back: usize,
}

impl Applied {
    pub fn line(&self) -> String {
        format!(
            "{} vendor and trainer row(s) applied, {} affected — reloading {}",
            self.rows,
            self.affected,
            RELOADS.join(", ")
        )
    }
}

/// An Apply, with everything it needs to run off the main thread; see
/// [`super::items::ApplyJob`], which has the same three halves.
pub struct ApplyJob {
    plan: Plan,
    project: vale_edit::project::Project,
    at: vale_mangos::conn::Where,
}

/// What running one answered.
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
            Err(e) => format!("vendors and trainers: {e}"),
        }
    }
}

/// The main thread's first half of an Apply. `None` when the project claims no
/// row of the four tables and has applied none.
pub fn prepare_apply(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<ApplyJob>, String> {
    let plan = plan(session);
    if plan.is_empty() && !super::reconcile::has_applied(session, REVERT_VPATH) {
        return Ok(None);
    }
    let (at, _source) = server.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
    Ok(Some(ApplyJob {
        plan,
        project: session.project.clone(),
        at,
    }))
}

impl ApplyJob {
    /// The worker's half, in [`super::reconcile`]'s order: what the project
    /// applied before is put back, and then each row is read as it stands, its
    /// undo written, and its own statements run.
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
                "the server's vendor and trainer lists",
                &steps,
                |db, index| undo_of_a_row(db, rows[index]),
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

/// The main thread's second half. The tables have moved whether or not the
/// run finished, so every window's read is stale, and both reloads are asked
/// for.
pub fn finish_apply(session: &mut EditSession, reloads: &mut super::reload::Reloads, done: &ApplyDone) {
    session.wrote_the_database();
    session.applied_services = done.result.as_ref().ok().map(|_| done.signature);
    for table in RELOADS {
        reloads.when_there_is_a_session(table);
    }
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
    Ok(Some(super::stack::Step::new("applying vendors and trainers", move || {
        let done = job.run();
        let ok = done.result.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            finish_apply(session, reloads, &done);
            session.status = done.line();
        });
        (ok, finish)
    })))
}

/// One row's undo, read immediately before the row is written: the subject's
/// half of [`super::reconcile`]'s step 3.
///
/// A created row is refused when its key is already there and is not this
/// project's: the vendor tables' primary key is `(entry, item)`, so a second
/// row of one item in one list is refused by MySQL too.
fn undo_of_a_row(db: &mut Db, row: &Row) -> Result<Option<Vec<String>>, String> {
    if row.life == Life::Insert && db.row(&vendor::exists_query(row.table, &row.key))?.is_some() {
        return Err(format!(
            "{} {} is already in the database and is not this project's.",
            row.table,
            row.key.text()
        ));
    }
    Ok(match row.life {
        Life::Insert => Some(vec![row::delete(row.table, &row.key)]),
        // `None` when it is not in the database: nothing to put back. The
        // `DELETE` first, so the entry can be run twice; see
        // `super::reconcile`.
        Life::Delete => db.row(&vendor::row_query(row.table, &row.key))?.map(|held| {
            let mut out = vec![row::delete(row.table, &row.key)];
            out.extend(row::insert_from_row(row.table, &held));
            out
        }),
        Life::Update => {
            let now = db.row(&vendor::row_query(row.table, &row.key))?;
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
    let (at, _source) = server.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
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

/// The main thread's second half: both reloads, since the revert file does not
/// say which tables it touches.
pub fn finish_revert(session: &mut EditSession, reloads: &mut super::reload::Reloads) {
    session.applied_services = None;
    session.wrote_the_database();
    for table in RELOADS {
        reloads.when_there_is_a_session(table);
    }
}

/// A Put back as one step: `None` when this project has applied nothing.
pub fn revert_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_revert(session, server)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new("putting back vendors and trainers", move || {
        let done = job.run();
        let ok = done.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            session.status = match done {
                Ok(rows) => {
                    finish_revert(session, reloads);
                    format!("{rows} vendor and trainer row(s) put back")
                }
                Err(e) => format!("vendors and trainers: {e}"),
            };
        });
        (ok, finish)
    })))
}

/// What of this project is in the database; [`super::items::OnTheServer`] for
/// this subject.
pub struct OnTheServer {
    undo: Undo,
    current: bool,
}

impl OnTheServer {
    pub fn read_with(session: &EditSession, plan: &Plan) -> OnTheServer {
        OnTheServer {
            undo: Undo::open_at(&session.project, REVERT_VPATH).unwrap_or(Undo { entries: Vec::new() }),
            current: session.applied_services.is_some_and(|had| had == plan.signature()),
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

/// `--apply-services` and `--revert-services`: the Server panel's two buttons
/// with nobody at the keyboard. Fires once, after any scripted edit has landed.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    server: Res<super::settings::ServerSettings>,
    mut reloads: ResMut<super::reload::Reloads>,
    services: Res<crate::tools::services::Services>,
    mut done: Local<bool>,
) {
    if *done || !(args.apply_services || args.revert_services) {
        return;
    }
    if !services.scripted_done {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    super::creatures::save(&mut session);
    save(&mut session);
    // Through the stack, as the Server panel's buttons are, so a flag puts
    // back and applies the later subjects with this one.
    let wanted = [
        (
            args.revert_services,
            "--revert-services",
            super::stack::Wanted::PutBack(super::stack::Subject::Services),
        ),
        (
            args.apply_services,
            "--apply-services",
            super::stack::Wanted::Apply(super::stack::Subject::Services),
        ),
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
        let loot = super::super::loot::plan_from(&edits);
        assert_eq!(loot.rows.len(), 1);
        assert!(loot.refused.is_empty(), "{:?}", loot.refused);
        let creatures = super::super::creatures::plan_from(&edits, &vale_mangos::path::Paths::default());
        assert!(creatures.refused.is_empty(), "{:?}", creatures.refused);
    }

    #[test]
    fn the_subject_owns_the_four_tables() {
        for table in TABLES {
            assert!(owns(table));
        }
        assert!(!owns(vale_mangos::creature::TEMPLATE));
    }
}
