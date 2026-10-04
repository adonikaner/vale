//! What a project changes about area triggers, graveyards and maps on the
//! server, as SQL, applied when a person asks.
//!
//! ## Eight tables, written as keyed rows
//!
//! ```text
//! map_template                  a new map's server row
//! areatrigger_template          a trigger's label, script, condition and
//!                               cooldown: the five columns the client's file
//!                               does not have, edited and never created
//! areatrigger_teleport          where a trigger sends a character
//! areatrigger_tavern            a trigger that is an inn
//! areatrigger_involvedrelation  a trigger that completes an exploration objective
//! areatrigger_bg_entrance       a trigger that opens a battleground's list
//! game_graveyard_zone           which safe place serves which zone
//! world_safe_locs_facing        which way a spirit faces at a safe place
//! ```
//!
//! They are keyed rows in the project's store, on [`super::services`]' terms: a
//! created row is a `DELETE` and an `INSERT` naming every column, an edited one
//! an `UPDATE`, a removed one a `DELETE`, and the statements that put each row
//! back are read from the database immediately before anything runs. The
//! columns and keys are `vale_mangos::trigger`, `vale_mangos::graveyard` and
//! `vale_mangos::map`. A created row is refused, with the loader's reason, when
//! the server would skip it for a reason the row alone shows.
//!
//! The triggers' volumes and the safe places are client tables, and reach the
//! server another way: `AreaTrigger.dbc` as `areatrigger_template` rows
//! written by [`super::rows`], `WorldSafeLocs.dbc` as a file copied into
//! `DataDir\5875\dbc` by [`super::dbcs`]. A template row's volume and its
//! server columns are written to the same dev row at build 5875 by the two
//! writers; see `vale_mangos::trigger`.
//!
//! ## What makes an applied row live
//!
//! Five of the eight tables have a `.reload` whose loader clears its map first,
//! so an apply asks a running playtest for all five, removals included.
//! `areatrigger_template`, `areatrigger_bg_entrance` and
//! `world_safe_locs_facing` are read at startup only. A new `map_template` row
//! is read on its reload, but the server opens a map's grids at startup, so a
//! new map needs a restart before anything can stand on it.
//!
//! ```text
//! sql\places.sql          what this project does to the six tables
//! sql\places-revert.sql   what puts those rows back
//! server\rows.txt         the store both are written from, shared
//! ```

use crate::session::EditSession;
use vale_mangos::conn::Db;
use vale_mangos::row::{self, Assignment, Key, Life};
use vale_mangos::{graveyard, map, trigger};
use bevy::prelude::*;

pub use super::creatures::Undo;

/// What this project does to the eight tables, as SQL.
pub const SQL_VPATH: &str = "sql\\places.sql";

/// What puts it back.
pub const REVERT_VPATH: &str = "sql\\places-revert.sql";

/// The eight tables, in the order a plan writes them. A map's row goes first,
/// since a teleport's target map is checked against it.
pub const TABLES: [&str; 8] = [
    map::TEMPLATE,
    trigger::TEMPLATE,
    trigger::TELEPORT,
    trigger::TAVERN,
    trigger::QUEST,
    trigger::BG_ENTRANCE,
    graveyard::ZONE,
    graveyard::FACING,
];

/// The reload commands that re-read five of them, in the order they are sent.
/// `areatrigger_template`, `areatrigger_bg_entrance` and
/// `world_safe_locs_facing` have none.
pub const RELOADS: [&str; 5] = [
    map::TEMPLATE,
    trigger::TELEPORT,
    trigger::TAVERN,
    trigger::QUEST,
    graveyard::ZONE,
];

/// The static name of one of the eight tables, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// Whether a table read out of the project's store is this subject's.
pub fn owns(table: &str) -> bool {
    table_named(table).is_some()
}

/// Every column of one of the eight tables.
pub fn columns_of(table: &str) -> &'static [vale_mangos::schema::Column] {
    match (trigger::table_named(table), graveyard::table_named(table), map::table_named(table)) {
        (Some(_), _, _) => trigger::columns_of(table),
        (_, Some(_), _) => graveyard::columns_of(table),
        (_, _, Some(_)) => map::columns_of(table),
        _ => &[],
    }
}

/// One column of one of the eight tables.
pub fn column(table: &str, name: &str) -> Option<&'static vale_mangos::schema::Column> {
    columns_of(table).iter().find(|column| column.name == name)
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
        trigger::row_statements(self.table, &self.key, self.life, &self.changes)
    }

    /// How it is named in a file's marker and in a status line.
    pub fn names(&self) -> String {
        format!("{} {}", self.table, self.key.text())
    }
}

/// Everything a project changes about the eight tables.
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
        // A template row is the client table's to make and remove; this
        // subject only sets its server columns.
        if table == trigger::TEMPLATE && row.life != Life::Update {
            out.refused.push(format!(
                "{table} {} cannot be {}d here: a template row follows AreaTrigger.dbc",
                key.text(),
                row.life.word()
            ));
            continue;
        }
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

/// Why the server would skip a created row, from the row alone. Text columns
/// are compared unquoted.
fn faults_of(table: &str, key: &Key, changes: &[Assignment]) -> Vec<String> {
    let mut row = vale_mangos::schema::Row::new();
    for (column, value) in &key.0 {
        row.insert(column.clone(), Some(value.clone()));
    }
    for change in changes {
        let value = change.value.trim_matches('\'').to_string();
        row.insert(change.column.to_string(), Some(value));
    }
    let mut out = trigger::check_created(table, &row);
    out.extend(graveyard::check_created(table, &row));
    out.extend(map::check_created(table, &row));
    out
}

/// Write `sql\places.sql`, or remove it when the project changes nothing.
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
        "-- {} — what this project changes about area triggers, graveyards and maps.\n\
         -- Rewritten on every save from the project's own edits, so it is the\n\
         -- whole of what the project does rather than an increment of it.\n\
         --\n\
         -- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then reload:\n\
         --   .reload map_template\n\
         --   .reload areatrigger_teleport\n\
         --   .reload areatrigger_tavern\n\
         --   .reload areatrigger_involvedrelation\n\
         --   .reload game_graveyard_zone\n\
         -- world_safe_locs_facing has no reload, and a new map needs a restart.\n\n",
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

/// The places half of a save: the SQL the store comes to. The store itself is
/// written by [`super::creatures::save`], on the same save.
pub fn save(session: &mut EditSession) {
    match write_sql(session) {
        Ok(0) => {}
        Ok(rows) => {
            session.status = format!("{rows} trigger, graveyard and map change(s) written to {SQL_VPATH}")
        }
        Err(e) => {
            warn!("places: {e}");
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
            "{} trigger, graveyard and map row(s) applied, {} affected — reloading {}",
            self.rows,
            self.affected,
            RELOADS.join(", ")
        )
    }
}

/// An Apply, with everything it needs to run off the main thread.
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
            Err(e) => format!("triggers, graveyards and maps: {e}"),
        }
    }
}

/// The main thread's first half of an Apply. `None` when the project claims no
/// row of the six tables and has applied none.
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
    /// The worker's half, in [`super::reconcile`]'s order.
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
                "the server's triggers, graveyards and maps",
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

/// The main thread's second half: every read is stale, and the five reloads
/// are asked for.
pub fn finish_apply(session: &mut EditSession, reloads: &mut super::reload::Reloads, done: &ApplyDone) {
    session.wrote_the_database();
    session.applied_places = done.result.as_ref().ok().map(|_| done.signature);
    for table in RELOADS {
        reloads.when_there_is_a_session(table);
    }
}

/// An Apply as one step of [`super::stack`]. `None` when there is nothing to
/// apply.
pub fn apply_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_apply(session, server)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new("applying triggers, graveyards and maps", move || {
        let done = job.run();
        let ok = done.result.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            finish_apply(session, reloads, &done);
            session.status = done.line();
        });
        (ok, finish)
    })))
}

/// One row's undo, read immediately before the row is written. A created row
/// is refused when its key is already there and is not this project's.
///
/// A template row with no dev row yet is made by this apply from the row the
/// server uses, so its undo puts the server columns back to that row's values.
/// The dev row stays, holding the same values, because the client tables'
/// mirror may also be writing it.
fn undo_of_a_row(db: &mut Db, row: &Row) -> Result<Option<Vec<String>>, String> {
    let mut held = db.row(&trigger::row_query(row.table, &row.key))?;
    if row.table == trigger::TEMPLATE && held.is_none() {
        if let Some(id) = row.key.first() {
            held = db.row(&trigger::winning_template_query(id as u32))?;
        }
    }
    if row.life == Life::Insert && held.is_some() {
        return Err(format!(
            "{} {} is already in the database and is not this project's.",
            row.table,
            row.key.text()
        ));
    }
    Ok(match row.life {
        Life::Insert => Some(vec![row::delete(row.table, &row.key)]),
        Life::Delete => held.map(|held| {
            let mut out = vec![row::delete(row.table, &row.key)];
            out.extend(row::insert_from_row(row.table, &held));
            out
        }),
        Life::Update => row::undo(row.table, &row.key, &row.changes, held.as_ref()).map(|one| vec![one]),
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

/// The main thread's second half: the five reloads, since the revert file does
/// not say which tables it touches.
pub fn finish_revert(session: &mut EditSession, reloads: &mut super::reload::Reloads) {
    session.applied_places = None;
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
    Ok(Some(super::stack::Step::new("putting back triggers, graveyards and maps", move || {
        let done = job.run();
        let ok = done.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            session.status = match done {
                Ok(rows) => {
                    finish_revert(session, reloads);
                    format!("{rows} trigger, graveyard and map row(s) put back")
                }
                Err(e) => format!("triggers, graveyards and maps: {e}"),
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
            current: session.applied_places.is_some_and(|had| had == plan.signature()),
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

    /// A new map, a teleport into it and a graveyard link are written in
    /// [`TABLES`]' order: the map's row first.
    #[test]
    fn a_map_its_entrance_and_its_graveyard_are_written_in_order() {
        let mut edits = Edits::default();
        let link = graveyard::Link::new(900, 5000);
        created(graveyard::ZONE, &link.key(), link.assignments(), &mut edits);
        let teleport = trigger::Teleport::new(4500, 534, [10.0, 20.0, 30.0]);
        created(trigger::TELEPORT, &teleport.key(), teleport.assignments(), &mut edits);
        let template = map::Template::new(534, 1, "The Islands");
        created(map::TEMPLATE, &template.key(), template.assignments(), &mut edits);
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        let sql = plan.statements();
        assert_eq!(sql.len(), 6);
        assert!(sql[1].starts_with("INSERT INTO `map_template`"), "{}", sql[1]);
        assert!(sql[3].starts_with("INSERT INTO `areatrigger_teleport`"), "{}", sql[3]);
        assert!(sql[5].starts_with("INSERT INTO `game_graveyard_zone`"), "{}", sql[5]);
    }

    /// A created teleport with no target, and a link for a faction the loader
    /// does not accept, are refused with the loader's reason.
    #[test]
    fn a_row_the_server_would_skip_is_refused() {
        let mut edits = Edits::default();
        let teleport = trigger::Teleport::new(4500, 534, [0.0; 3]);
        created(trigger::TELEPORT, &teleport.key(), teleport.assignments(), &mut edits);
        let link = graveyard::Link { faction: 5, ..graveyard::Link::new(900, 5000) };
        created(graveyard::ZONE, &link.key(), link.assignments(), &mut edits);
        let plan = plan_from(&edits);
        assert!(plan.rows.is_empty());
        assert_eq!(plan.refused.len(), 2, "{:?}", plan.refused);
    }

    #[test]
    fn the_subject_owns_the_eight_tables_and_no_other() {
        for table in TABLES {
            assert!(owns(table));
            assert!(!columns_of(table).is_empty(), "{table}");
        }
        assert!(!owns(vale_mangos::creature::TEMPLATE));
    }

    /// A template edit sets the server columns on the dev row, which it makes
    /// first; a created or removed template row is refused.
    #[test]
    fn a_template_row_is_edited_on_its_dev_row_and_never_created() {
        let mut edits = Edits::default();
        let key = trigger::key(78);
        edits.set(trigger::TEMPLATE, &key, "cooldown", Some("30".to_string()));
        let plan = plan_from(&edits);
        assert_eq!(plan.rows.len(), 1);
        let statements = plan.rows[0].statements();
        assert_eq!(statements.len(), 2);
        assert!(statements[0].starts_with("INSERT IGNORE INTO `areatrigger_template`"));
        assert!(statements[1].contains("`cooldown` = 30"), "{}", statements[1]);
        assert!(statements[1].contains("`build` = 5875"), "{}", statements[1]);

        let mut edits = Edits::default();
        let template = trigger::Template { id: 9000, ..trigger::Template::default() };
        created(trigger::TEMPLATE, &trigger::key(9000), template.assignments(), &mut edits);
        let plan = plan_from(&edits);
        assert!(plan.rows.is_empty());
        assert_eq!(plan.refused.len(), 1);
    }
}
