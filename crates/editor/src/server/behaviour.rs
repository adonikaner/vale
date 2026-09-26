//! What a project changes about a creature's behaviour, as SQL, applied when
//! a person asks, and live on a reload of each table written.
//!
//! ## Three tables, two shapes
//!
//! `creature_ai_events` and `creature_spells` are keyed rows in the project's
//! store, on [`super::loot`]'s terms: a created row is a `DELETE` and an
//! `INSERT`, an edited one an `UPDATE`, a removed one a `DELETE`. The eleven
//! `*_scripts` tables have no key, so a script is the unit and lives in the
//! session's second store (`EditSession::server_scripts`), written whole on
//! [`super::creatures`]' path terms: `DELETE` under the id, then one `INSERT`
//! per row. `vale_mangos::scripts`' module comment is where that was
//! decided.
//!
//! ## Live on a reload
//!
//! `.reload creature_ai_events` re-reads `creature_ai_scripts` and then the
//! events; `.reload creature_spells` re-reads the lists; five of the script
//! tables reload under their own names and the other six are read at start
//! (`vale_mangos::scripts::Table::reload`). A creature already in the
//! world keeps the events and the list it spawned with until it respawns.
//!
//! ```text
//! sql\behaviour.sql          what this project does to the behaviour tables
//! sql\behaviour-revert.sql   what puts those rows back
//! server\rows.txt            the store the events and lists are written from
//! server\scripts.txt         the store the scripts are written from
//! ```

use crate::session::EditSession;
use vale_mangos::conn::Db;
use vale_mangos::row::{self, Assignment, Key, Life};
use vale_mangos::scripts::Script;
use vale_mangos::{creaturespells, eventai, scripts};
use bevy::prelude::*;

pub use super::creatures::Undo;

/// What this project does to the server's behaviour tables, as SQL.
pub const SQL_VPATH: &str = "sql\\behaviour.sql";

/// What puts it back.
pub const REVERT_VPATH: &str = "sql\\behaviour-revert.sql";

/// Where the project keeps the scripts it changes, a store of their own for
/// the reason `super::creatures::PATHS_VPATH` is.
pub const SCRIPTS_VPATH: &str = "server\\scripts.txt";

/// The tables the row store may hold for this subject.
pub const ROW_TABLES: [&str; 2] = [eventai::TABLE, creaturespells::TABLE];

/// Whether a table is this subject's: an event, a spell list or a script.
pub fn owns(table: &str) -> bool {
    eventai::table_named(table).is_some()
        || creaturespells::table_named(table).is_some()
        || scripts::table_named(table).is_some()
}

/// One keyed row's change: what is to become of it, and the columns it sets.
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
        match self.table {
            eventai::TABLE => eventai::statements(&self.key, self.life, &self.changes),
            _ => creaturespells::statements(&self.key, self.life, &self.changes),
        }
    }

    /// How it is named in a file's marker and in a status line.
    pub fn names(&self) -> String {
        format!("{} {}", self.table, self.key.text())
    }
}

/// Everything a project changes about behaviour.
#[derive(Debug, Default)]
pub struct Plan {
    pub rows: Vec<Row>,
    pub scripts: Vec<Script>,
    /// What the store asked for and this writer would not write, each as a
    /// sentence.
    pub refused: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.scripts.is_empty()
    }

    /// How many rows and scripts it writes.
    pub fn count(&self) -> usize {
        self.rows.len() + self.scripts.len()
    }

    /// Every statement this plan comes to: the events and lists table by
    /// table, then the scripts.
    pub fn statements(&self) -> Vec<String> {
        let mut out: Vec<String> = self.rows.iter().flat_map(Row::statements).collect();
        for script in &self.scripts {
            out.extend(scripts::statements(script));
        }
        out
    }

    /// The reloads an apply asks for: every table with a reload, since the
    /// put-back before the apply may have written any of them.
    pub fn reloads() -> Vec<&'static str> {
        let mut out = vec!["creature_ai_events", "creature_spells"];
        for table in scripts::ALL {
            if let Some(reload) = table.reload {
                if !out.contains(&reload) {
                    out.push(reload);
                }
            }
        }
        out
    }

    /// What this plan would do to the database, as one number; see
    /// `super::creatures::Plan::signature`.
    pub fn signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for statement in self.statements() {
            statement.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// One line for the panel: rows created, edited and removed, and scripts.
    pub fn line(&self) -> String {
        let of = |life: Life| self.rows.iter().filter(|row| row.life == life).count();
        let mut parts = Vec::new();
        match of(Life::Update) {
            0 => {}
            n => parts.push(format!("{n} row(s) edited")),
        }
        match of(Life::Insert) {
            0 => {}
            n => parts.push(format!("{n} created")),
        }
        match of(Life::Delete) {
            0 => {}
            n => parts.push(format!("{n} removed")),
        }
        match self.scripts.len() {
            0 => {}
            n => parts.push(format!("{n} script(s) replaced")),
        }
        parts.join(", ")
    }
}

/// What the project's stores come to, as statements.
pub fn plan(session: &EditSession) -> Plan {
    plan_from(&session.server_edits, &session.server_scripts)
}

/// The same over the stores alone, so it can be checked with no session.
pub fn plan_from(edits: &row::Edits, scripts: &scripts::Scripts) -> Plan {
    let mut out = Plan::default();
    for (table, key, row) in edits.rows() {
        let (table, columns): (&'static str, &'static [vale_mangos::schema::Column]) =
            match (eventai::table_named(table), creaturespells::table_named(table)) {
                (Some(table), _) => (table, &eventai::COLUMNS),
                (_, Some(table)) => (table, &creaturespells::COLUMNS),
                _ => continue,
            };
        let mut changes = Vec::new();
        for (column, value) in &row.columns {
            let Some(known) = columns.iter().find(|known| known.name == column) else {
                out.refused.push(format!("{table}.{column} is not a column this editor writes"));
                continue;
            };
            if !known.editable() {
                out.refused.push(format!("{table}.{column} is part of the key and is not written"));
                continue;
            }
            changes.push(Assignment { column: known.name, value: value.clone() });
        }
        match row.life {
            // A creation names every column or it is not written, on
            // `super::creatures`' rule.
            Life::Insert => {
                let missing: Vec<&str> = columns
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
                if changes.is_empty() {
                    continue;
                }
            }
        }
        out.rows.push(Row { table, key: key.clone(), life: row.life, changes });
    }
    out.scripts = scripts.iter().collect();
    out
}

/// Write `sql\behaviour.sql`, or remove it when the project changes nothing.
pub fn write_sql(session: &mut EditSession) -> Result<usize, String> {
    let plan = plan(session);
    for refused in &plan.refused {
        warn!("{}: {refused}", super::creatures::EDITS_VPATH);
    }
    if plan.is_empty() {
        remove(&session.project, SQL_VPATH);
        return Ok(0);
    }
    let count = plan.count();
    let mut body = format!(
        "-- {} — what this project changes about creature behaviour: events,\n\
         -- scripts and spell lists.\n\
         -- Rewritten on every save from the project's own edits, so it is the\n\
         -- whole of what the project does rather than an increment of it.\n\
         --\n\
         -- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then reload what it names:\n\
         --   .reload creature_ai_events    (re-reads creature_ai_scripts first)\n\
         --   .reload creature_spells\n\
         --   .reload gossip_scripts, generic_scripts, event_scripts,\n\
         --           quest_start_scripts, creature_spells_scripts, as written\n\
         -- The other script tables are read at start, so restart the server for them.\n\n",
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
    for script in &plan.scripts {
        body.push_str(&format!("-- {} id = {} — {} row(s)\n", script.table, script.id, script.rows.len()));
        for statement in scripts::statements(script) {
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

/// The behaviour half of a save: the scripts store and the SQL. The row
/// store is written by [`super::creatures::save`], on the same save.
pub fn save(session: &mut EditSession) {
    if !session.save_server_scripts() {
        return;
    }
    match write_sql(session) {
        Ok(0) => {}
        Ok(rows) => session.status = format!("{rows} behaviour change(s) written to {SQL_VPATH}"),
        Err(e) => {
            warn!("behaviour: {e}");
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
    pub taken_back: usize,
    pub tables: Vec<&'static str>,
}

impl Applied {
    pub fn line(&self) -> String {
        format!(
            "{} behaviour change(s) applied, {} affected — reloading {}",
            self.rows,
            self.affected,
            self.tables.join(", ")
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
    pub fn line(&self) -> String {
        match &self.result {
            Ok(done) if done.rows == 0 && done.taken_back == 0 => "nothing to apply".to_string(),
            Ok(done) => done.line(),
            Err(e) => format!("behaviour: {e}"),
        }
    }
}

/// The main thread's first half of an Apply. `None` when the project claims
/// nothing of this subject and has applied nothing.
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
    Ok(Some(ApplyJob { plan, project: session.project.clone(), at }))
}

impl ApplyJob {
    /// The worker's half, in [`super::reconcile`]'s order: rows, then scripts.
    pub fn run(self) -> ApplyDone {
        let signature = self.plan.signature();
        let plan = &self.plan;
        let result = (|| {
            let mut db = Db::open(&self.at)?;
            let mut steps: Vec<super::reconcile::Step> = plan
                .rows
                .iter()
                .map(|row| super::reconcile::Step {
                    table: row.table.to_string(),
                    key: row.key.clone(),
                    statements: row.statements(),
                })
                .collect();
            steps.extend(plan.scripts.iter().map(|script| super::reconcile::Step {
                table: script.table.to_string(),
                key: script.key(),
                statements: scripts::statements(script),
            }));
            let rows = plan.rows.len();
            let done = super::reconcile::apply(
                &self.project,
                &mut db,
                REVERT_VPATH,
                "the server's creature behaviour",
                &steps,
                |db, index| match index < rows {
                    true => undo_of_a_row(db, &plan.rows[index]),
                    false => undo_of_a_script(db, &plan.scripts[index - rows]).map(Some),
                },
            )?;
            Ok(Applied {
                rows: plan.count(),
                affected: done.affected,
                newly_undoable: done.undoable,
                taken_back: done.taken_back,
                tables: Plan::reloads(),
            })
        })();
        ApplyDone { signature, result }
    }
}

/// The main thread's second half: the tables have moved, so every window's
/// read is stale, and a reload of each table with one is asked for.
pub fn finish_apply(
    session: &mut EditSession,
    reloads: &mut super::reload::Reloads,
    done: &ApplyDone,
) {
    session.behaviour_writes += 1;
    session.applied_behaviour = done.result.as_ref().ok().map(|_| done.signature);
    for table in Plan::reloads() {
        reloads.when_there_is_a_session(table);
    }
}

/// An Apply as one step of [`super::stack`]: `None` when there is nothing
/// to apply.
pub fn apply_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_apply(session, server)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new("applying behaviour", move || {
        let done = job.run();
        let ok = done.result.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            finish_apply(session, reloads, &done);
            session.status = done.line();
        });
        (ok, finish)
    })))
}

/// One row's undo, read immediately before the row is written. A created
/// row is refused when its key is already there and not this project's.
fn undo_of_a_row(db: &mut Db, row: &Row) -> Result<Option<Vec<String>>, String> {
    let (exists, read) = match row.table {
        eventai::TABLE => (eventai::exists_query(&row.key), eventai::row_query(&row.key)),
        _ => (
            creaturespells::exists_query(&row.key),
            format!(
                "SELECT * FROM {} WHERE {} LIMIT 1",
                vale_mangos::sql::name(row.table),
                row.key.where_clause()
            ),
        ),
    };
    if row.life == Life::Insert && db.row(&exists)?.is_some() {
        return Err(format!(
            "{} {} is already in the database and is not this project's.",
            row.table,
            row.key.text()
        ));
    }
    Ok(match row.life {
        Life::Insert => Some(vec![row::delete(row.table, &row.key)]),
        Life::Delete => db.row(&read)?.map(|held| {
            let mut out = vec![row::delete(row.table, &row.key)];
            out.extend(row::insert_from_row(row.table, &held));
            out
        }),
        Life::Update => {
            let now = db.row(&read)?;
            row::undo(row.table, &row.key, &row.changes, now.as_ref()).map(|one| vec![one])
        }
    })
}

/// A script's undo: every row under its id as it stands, restored the same
/// way the script is written.
fn undo_of_a_script(db: &mut Db, script: &Script) -> Result<Vec<String>, String> {
    let rows = db.rows(&scripts::rows_query(script.table, script.id))?;
    Ok(scripts::undo_from_rows(script.table, script.id, &rows))
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
    Ok(Some(RevertJob { project: session.project.clone(), at }))
}

impl RevertJob {
    /// The worker's half: run the revert file, and forget it.
    pub fn run(self) -> Result<usize, String> {
        let mut db = Db::open(&self.at)?;
        super::reconcile::put_back(&self.project, &mut db, REVERT_VPATH)
    }
}

/// The main thread's second half: every table with a reload reloaded, since
/// the revert file does not say which it touched.
pub fn finish_revert(session: &mut EditSession, reloads: &mut super::reload::Reloads) {
    session.applied_behaviour = None;
    session.behaviour_writes += 1;
    for table in Plan::reloads() {
        reloads.when_there_is_a_session(table);
    }
}

/// A Put back as one step of the stack: `None` when this project has applied
/// nothing.
pub fn revert_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_revert(session, server)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new("putting back behaviour", move || {
        let done = job.run();
        let ok = done.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            session.status = match done {
                Ok(rows) => {
                    finish_revert(session, reloads);
                    format!("{rows} behaviour change(s) put back")
                }
                Err(e) => format!("behaviour: {e}"),
            };
        });
        (ok, finish)
    })))
}

/// What of this project is in the database; `super::loot::OnTheServer` for
/// this subject.
pub struct OnTheServer {
    undo: Undo,
    current: bool,
}

impl OnTheServer {
    pub fn read_with(session: &EditSession, plan: &Plan) -> OnTheServer {
        OnTheServer {
            undo: Undo::open_at(&session.project, REVERT_VPATH).unwrap_or(Undo { entries: Vec::new() }),
            current: session
                .applied_behaviour
                .is_some_and(|had| had == plan.signature()),
        }
    }

    /// How many rows and scripts this project has applied and can take back.
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

/// `--apply-behaviour` and `--revert-behaviour`: the Server panel's two
/// buttons with nobody at the keyboard. Fires once, after any scripted edit
/// has landed.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    server: Res<super::settings::ServerSettings>,
    mut reloads: ResMut<super::reload::Reloads>,
    behaviour: Res<crate::tools::behaviour::Behaviour>,
    mut done: Local<bool>,
) {
    if *done || !(args.apply_behaviour || args.revert_behaviour) {
        return;
    }
    if !behaviour.scripted_done {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    super::creatures::save(&mut session);
    save(&mut session);
    let wanted = [
        (args.revert_behaviour, "--revert-behaviour", super::stack::Wanted::PutBack(super::stack::Subject::Behaviour)),
        (args.apply_behaviour, "--apply-behaviour", super::stack::Wanted::Apply(super::stack::Subject::Behaviour)),
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

    /// An event edit is one `UPDATE`, a created event names every column, and
    /// a script is written whole after the rows.
    #[test]
    fn the_plan_holds_rows_and_scripts() {
        let mut edits = Edits::default();
        edits.set(eventai::TABLE, &eventai::key(6801), "event_chance", Some("50".into()));
        let event = eventai::Event::new(6806, 68);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in event.assignments() {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(eventai::TABLE, &event.key(), Some(&row.to_line()));
        // Another subject's row is not this plan's.
        edits.set(vale_mangos::item::TEMPLATE, &vale_mangos::item::template_key(19019, 10), "Quality", Some("4".into()));
        let mut store = scripts::Scripts::default();
        let mut talk = scripts::ScriptRow::new(0);
        talk.dataint[0] = 1234;
        store.set(&Script { table: scripts::CREATURE_AI, id: 6806, rows: vec![talk] });

        let plan = plan_from(&edits, &store);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.rows.len(), 2);
        assert_eq!(plan.scripts.len(), 1);
        let sql = plan.statements();
        assert_eq!(sql[0], "UPDATE `creature_ai_events` SET `event_chance` = 50 WHERE `id` = 6801;");
        assert!(sql[1].starts_with("DELETE FROM `creature_ai_events` WHERE `id` = 6806;"));
        assert!(sql[2].starts_with("INSERT INTO `creature_ai_events`"));
        assert_eq!(sql[3], "DELETE FROM `creature_ai_scripts` WHERE `id` = 6806;");
        assert!(sql[4].starts_with("INSERT INTO `creature_ai_scripts`"));
        assert_eq!(plan.line(), "1 row(s) edited, 1 created, 1 script(s) replaced");
        let empty = plan_from(&Edits::default(), &scripts::Scripts::default());
        assert!(empty.is_empty());
    }

    /// A created row missing a column is refused by name, and a key column
    /// is not written.
    #[test]
    fn a_partial_creation_and_a_key_column_are_refused() {
        let mut edits = Edits::default();
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        row.columns.insert("event_type".into(), "4".into());
        edits.set_row_line(eventai::TABLE, &eventai::key(6807), Some(&row.to_line()));
        edits.set(creaturespells::TABLE, &creaturespells::key(680), "entry", Some("681".into()));
        let plan = plan_from(&edits, &scripts::Scripts::default());
        assert!(plan.rows.is_empty());
        assert_eq!(plan.refused.len(), 2);
        assert!(plan.refused.iter().any(|why| why.contains("created without")), "{:?}", plan.refused);
        assert!(plan.refused.iter().any(|why| why.contains("part of the key")), "{:?}", plan.refused);
    }

    /// The reloads an apply asks for are the ones the server has.
    #[test]
    fn the_reloads_are_the_servers_own() {
        let reloads = Plan::reloads();
        assert_eq!(&reloads[..2], &["creature_ai_events", "creature_spells"]);
        assert!(reloads.contains(&"gossip_scripts"));
        assert!(!reloads.contains(&"quest_end_scripts"));
        assert!(owns(scripts::GENERIC));
        assert!(owns(eventai::TABLE));
        assert!(!owns(vale_mangos::loot::CREATURE));
    }
}
