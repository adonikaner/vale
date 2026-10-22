//! What every row subject shares: the plan a project's stores come to, the SQL
//! file a save writes, and (in `apply.rs`) the Apply and Put back that run it.
//!
//! ```text
//! mod.rs     the RowSubject trait, the plan of one subject, and its SQL file
//! apply.rs   an Apply and a Put back as steps of super::stack, each in
//!            super::reconcile's order, and what of a project is on the server
//! flags.rs   --apply <subject> and --revert <subject>, the Server panel's two
//!            buttons from the command line
//! ```
//!
//! ## One lifecycle, twelve subjects
//!
//! Every row subject is written and applied the same way. The project's row
//! store (`server\rows.txt`, shared by all of them and keyed by table) holds
//! typed values; a save writes the statements they come to into the subject's
//! own SQL file; nothing reaches the database until Apply, which first writes
//! the statements that put each row back into the subject's revert file. See
//! [`super::reconcile`] for the order an Apply runs in and [`super::stack`]
//! for the order the subjects stand in.
//!
//! A subject is a [`RowSubject`]: its tables, its two files, its reloads, its
//! words, and the hooks where it differs from the rule: a created row the
//! server would skip, the undo of a creation or a removal that takes more than
//! one row, and the rows written whole under one id (a creature's waypoint
//! path, a behaviour script) that the row store does not hold.
//!
//! Each writer walks the whole store and skips the rows of tables it does not
//! own, without a refusal: the writer that owns a row is the one that says
//! whether it can be written, and a refusal from every other writer would put
//! the same complaint on several blocks of the Server panel.

pub mod apply;
pub mod flags;

use super::stack::Subject;
use crate::session::EditSession;
use vale_mangos::conn::Db;
use vale_mangos::row::{self, Assignment, Edits, Key, Life};
use bevy::prelude::*;

/// Where the project keeps the server rows it changes: one store for every
/// row subject, keyed by table.
pub const EDITS_VPATH: &str = "server\\rows.txt";

/// One subject whose edits are rows of vmangos' world database, written and
/// applied on the module's terms. Everything a subject differs in is here;
/// everything else is the module's.
pub trait RowSubject: Sync {
    /// Its place in [`super::stack`]'s order.
    fn subject(&self) -> Subject;

    /// What a status line and a step's label call it, plural: `creatures`.
    fn name(&self) -> &'static str;

    /// The Server panel's name for its block: `Creatures`.
    fn label(&self) -> &'static str;

    /// The word `--apply` and `--revert` name it by: `creatures`.
    fn word(&self) -> &'static str;

    /// What one of its rows is called on a status line: `creature`.
    fn noun(&self) -> &'static str;

    /// The tables of the row store it writes, in the order a plan writes them.
    fn tables(&self) -> &'static [row::Table];

    /// The database tables it writes, as the Server panel lists them.
    fn table_list(&self) -> &'static str;

    /// Whether a table read out of the project's row store is this subject's.
    fn owns(&self, table: &str) -> bool {
        row::table_named(self.tables(), table).is_some()
    }

    /// Where a save writes its statements, as a virtual path in the project.
    fn sql_vpath(&self) -> &'static str;

    /// Where an Apply keeps the statements that put its rows back.
    fn revert_vpath(&self) -> &'static str;

    /// What the SQL file's first line says the project changes:
    /// `the server's creatures`.
    fn sql_about(&self) -> &'static str;

    /// The SQL file's note on what makes an applied row live, as comment lines.
    fn sql_note(&self) -> String;

    /// What it takes for an applied row to become live, for the hover text of
    /// the Server panel's block.
    fn going_live(&self) -> &'static str;

    /// The reload commands that make an applied row live, in the order they
    /// are sent. Empty when only a restart does. A Put back asks for all of
    /// them, since the revert file does not say which tables it touches.
    fn reloads(&self) -> Vec<&'static str>;

    /// The reloads an Apply of `plan` asks for. [`Self::reloads`] unless the
    /// plan holds a change a reload must not follow.
    fn reloads_for(&self, _plan: &Plan) -> Vec<&'static str> {
        self.reloads()
    }

    /// What an Apply's status line says when it asks for no reload.
    fn restart_note(&self) -> &'static str {
        "restart the server to load them"
    }

    /// Whether a table no subject owns is refused by name. One subject does
    /// this, so an unknown table in the store is reported once.
    fn refuses_unowned(&self) -> bool {
        false
    }

    /// Why the server would skip a created row, from the row alone (its key
    /// and columns, unquoted). A created row with a fault is refused with it.
    fn faults(&self, _table: &'static row::Table, _row: &vale_mangos::schema::Row) -> Vec<String> {
        Vec::new()
    }

    /// The statements one planned row becomes.
    fn statements(&self, row: &Row) -> Vec<String> {
        row.statements()
    }

    /// The sets of rows written whole under one id that this subject's plan
    /// holds besides its keyed rows, read from the session's other stores.
    /// `rows` is the plan so far, for a check against it; a set that is not
    /// written is named in `refused`.
    fn wholes(&self, _stores: &Stores<'_>, _rows: &[Row], _refused: &mut Vec<String>) -> Vec<Whole> {
        Vec::new()
    }

    /// Refuse a row arriving at a key, immediately before it is created or
    /// moved there. By default a row already at the whole key is not this
    /// project's and is refused.
    fn refuse_arrival(&self, db: &mut Db, row: &Row, _own: &Edits) -> Result<(), String> {
        refuse_a_present_key(db, row)
    }

    /// The statements that undo a created row.
    fn undo_of_a_creation(&self, row: &Row) -> Vec<String> {
        vec![row::delete(row.table.name, &row.key)]
    }

    /// The statements that undo a removed row, read immediately before it is
    /// removed. `None` when the row is not there: nothing to put back.
    fn undo_of_a_removal(&self, db: &mut Db, row: &Row) -> Result<Option<Vec<String>>, String> {
        snapshot_one_row(db, row)
    }

    /// What an edited row holds now, for its undo.
    fn held(&self, db: &mut Db, row: &Row) -> Result<Option<vale_mangos::schema::Row>, String> {
        db.row(&row::row_query(row.table.name, &row.at))
    }
}

/// The session's three stores a plan is read from: the row store, the
/// waypoint paths and the behaviour scripts.
pub struct Stores<'a> {
    pub edits: &'a Edits,
    pub paths: &'a vale_mangos::path::Paths,
    pub scripts: &'a vale_mangos::scripts::Scripts,
}

impl<'a> Stores<'a> {
    /// The session's own.
    pub fn of(session: &'a EditSession) -> Stores<'a> {
        Stores {
            edits: &session.server_edits,
            paths: &session.server_paths,
            scripts: &session.server_scripts,
        }
    }
}

/// One row's worth of change: what is to become of it, and the columns it
/// sets.
#[derive(Debug, Clone)]
pub struct Row {
    pub table: &'static row::Table,
    pub key: Key,
    /// Whether the row is edited, created or removed; see
    /// [`vale_mangos::row::Life`].
    pub life: Life,
    pub changes: Vec<Assignment>,
    /// Where the database has the row: [`Self::key`] unless the project
    /// changes the row's id. See `vale_mangos::row::RowEdit::from`.
    pub at: Key,
}

impl Row {
    /// The statements it becomes by its table's rule: one `UPDATE`, the
    /// `DELETE` and `INSERT` a creation is, the removal with its cascade, or
    /// the move of a row whose id changes with every column that names it.
    pub fn statements(&self) -> Vec<String> {
        if self.moves() {
            return row::move_statements(self.table.name, &self.at, &self.key, &self.changes, self.table.references);
        }
        self.table.statements(&self.key, self.life, &self.changes)
    }

    /// Whether the project changes this row's id.
    pub fn moves(&self) -> bool {
        self.life == Life::Update && self.at != self.key
    }

    /// How it is named in a file's marker and in a status line.
    pub fn names(&self) -> String {
        format!("{} {}", self.table.name, self.key.text())
    }
}

/// A set of rows written whole under one id, which has no key of its own per
/// row: a `DELETE` of the id and an `INSERT` a row.
#[derive(Debug, Clone)]
pub enum Whole {
    /// A creature's waypoint path.
    Path(vale_mangos::path::Path),
    /// A behaviour script.
    Script(vale_mangos::scripts::Script),
}

impl Whole {
    /// The table its rows are in.
    pub fn table(&self) -> &'static str {
        match self {
            Whole::Path(path) => path.which.table(),
            Whole::Script(script) => script.table,
        }
    }

    /// The id it is written under, as a key.
    pub fn key(&self) -> Key {
        match self {
            Whole::Path(path) => path.key(),
            Whole::Script(script) => script.key(),
        }
    }

    /// The statements it becomes.
    pub fn statements(&self) -> Vec<String> {
        match self {
            Whole::Path(path) => vale_mangos::path::statements(path),
            Whole::Script(script) => vale_mangos::scripts::statements(script),
        }
    }

    /// Its heading in the SQL file: the owner and the number of rows, which
    /// is what somebody reviewing the file checks.
    fn heading(&self) -> String {
        match self {
            Whole::Path(path) => format!(
                "{} {} = {} \u{2014} {} node(s)",
                path.which.table(),
                path.which.key_column(),
                path.owner,
                path.nodes.len()
            ),
            Whole::Script(script) => {
                format!("{} id = {} \u{2014} {} row(s)", script.table, script.id, script.rows.len())
            }
        }
    }

    /// What the database holds under its id now, as the statements that put
    /// it back. A set with no rows still gets an undo, the `DELETE` alone,
    /// which is the inverse of creating one where there was none.
    pub fn undo(&self, db: &mut Db) -> Result<Vec<String>, String> {
        match self {
            Whole::Path(path) => {
                let rows = db.rows(&vale_mangos::path::whole_rows_query(path.which, path.owner))?;
                Ok(vale_mangos::path::undo_from_rows(path.which, path.owner, &rows))
            }
            Whole::Script(script) => {
                let rows = db.rows(&vale_mangos::scripts::rows_query(script.table, script.id))?;
                Ok(vale_mangos::scripts::undo_from_rows(script.table, script.id, &rows))
            }
        }
    }
}

/// Everything a project changes about one subject.
pub struct Plan {
    pub subject: &'static dyn RowSubject,
    /// The keyed rows, in the order they are written: table by table in the
    /// subject's order, removals first inside a table, then by key. Removals
    /// go first because a row moved between two keys that share a unique
    /// index (a loot row regrouped) is a removal and a creation, and the
    /// `INSERT` fails while the old row stands.
    pub rows: Vec<Row>,
    /// The sets written whole, after the rows: a spawn this project creates
    /// has to exist before its path is written.
    pub wholes: Vec<Whole>,
    /// What the store asked for and this writer would not write, each as a
    /// sentence.
    ///
    /// A table or a column this editor has no schema for, a key column an
    /// edit tried to move, a row a table cannot create or remove, a creation
    /// that does not name every column or that the server would skip. Reported
    /// rather than skipped silently: the store is a text file somebody may have
    /// edited, and an edit that reaches no statement never happens.
    pub refused: Vec<String>,
}

/// One row or one whole set of a plan, as a file and an Apply need it.
pub struct Unit {
    pub table: String,
    pub key: Key,
    pub statements: Vec<String>,
    /// Its heading in the SQL file.
    pub heading: String,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.wholes.is_empty()
    }

    /// How many rows and whole sets it writes.
    pub fn count(&self) -> usize {
        self.rows.len() + self.wholes.len()
    }

    /// How many rows it creates, edits and removes.
    pub fn counts(&self) -> (usize, usize, usize) {
        let of = |life: Life| self.rows.iter().filter(|row| row.life == life).count();
        (of(Life::Insert), of(Life::Update), of(Life::Delete))
    }

    /// The tables its rows touch, in the subject's order.
    pub fn tables(&self) -> Vec<&'static str> {
        self.subject
            .tables()
            .iter()
            .map(|table| table.name)
            .filter(|name| self.rows.iter().any(|row| row.table.name == *name))
            .collect()
    }

    /// Every row and whole set, in the order they are written.
    pub fn units(&self) -> Vec<Unit> {
        let rows = self.rows.iter().map(|row| Unit {
            table: row.table.name.to_string(),
            key: row.key.clone(),
            statements: self.subject.statements(row),
            heading: row.names(),
        });
        let wholes = self.wholes.iter().map(|whole| Unit {
            table: whole.table().to_string(),
            key: whole.key(),
            statements: whole.statements(),
            heading: whole.heading(),
        });
        rows.chain(wholes).collect()
    }

    /// Every statement this plan comes to, rows first.
    pub fn statements(&self) -> Vec<String> {
        self.units().into_iter().flat_map(|unit| unit.statements).collect()
    }

    /// What this plan would do to the database, as one number.
    ///
    /// Two plans with the same signature apply the same statements, so a
    /// project whose signature is the one the last Apply used is a project
    /// whose edits are in the database. The revert file cannot answer that: it
    /// records what a row held before this project touched it, which does not
    /// change when the edit does. The hash is over the statements rather than
    /// the row count: two edits to one row are one row and two different
    /// databases.
    pub fn signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for statement in self.statements() {
            statement.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// One line for a panel: rows created, edited and removed, and whole sets
    /// replaced.
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
        let paths = self.wholes.iter().filter(|whole| matches!(whole, Whole::Path(_))).count();
        let scripts = self.wholes.len() - paths;
        let mut out: Vec<String> = Vec::new();
        if !rows.is_empty() {
            out.push(rows);
        }
        match paths {
            0 => {}
            1 => out.push("1 path replaced".to_string()),
            n => out.push(format!("{n} paths replaced")),
        }
        if scripts > 0 {
            out.push(format!("{scripts} script(s) replaced"));
        }
        match out.is_empty() {
            true => format!("no {} edits", self.subject.noun()),
            false => out.join(", "),
        }
    }
}

/// What the project's stores come to for one subject, as statements.
pub fn plan(session: &EditSession, subject: &'static dyn RowSubject) -> Plan {
    plan_from(subject, &Stores::of(session))
}

/// The same over the stores alone, so what they come to can be checked
/// without a session, a project folder or a database.
///
/// Each row of the subject's tables goes through `vale_mangos::row::plan`,
/// which checks the columns and the life against the table; a created row is
/// then checked as the server checks it. The column's name in a statement is
/// always one of `vale_mangos`' own constants rather than text read off a
/// disk.
pub fn plan_from(subject: &'static dyn RowSubject, stores: &Stores<'_>) -> Plan {
    let mut out = Plan {
        subject,
        rows: Vec::new(),
        wholes: Vec::new(),
        refused: Vec::new(),
    };
    for (name, key, edit) in stores.edits.rows() {
        let Some(table) = row::table_named(subject.tables(), name) else {
            if subject.refuses_unowned()
                && !Subject::ORDER.iter().any(|other| other.row_subject().owns(name))
            {
                out.refused.push(format!("{name} is not a table this editor writes"));
            }
            continue;
        };
        let (planned, refused) = row::plan(table, key, edit);
        out.refused.extend(refused);
        let Some(planned) = planned else { continue };
        if edit.life == Life::Insert {
            let whole = row::whole(
                &planned.key,
                planned.changes.iter().map(|change| (change.column, change.value.as_str())),
            );
            let faults = subject.faults(table, &whole);
            if !faults.is_empty() {
                out.refused.push(format!(
                    "{} {} would be skipped by the server: {}; it is not written",
                    table.name,
                    key.text(),
                    faults.join("; ")
                ));
                continue;
            }
        }
        out.rows.push(Row {
            table,
            key: planned.key,
            life: edit.life,
            changes: planned.changes,
            at: planned.at,
        });
    }
    out.rows.sort_by_key(|row| {
        (
            subject.tables().iter().position(|table| table.name == row.table.name),
            row.life != Life::Delete,
            row.key.clone(),
        )
    });
    out.wholes = subject.wholes(stores, &out.rows, &mut out.refused);
    out
}

/// Write the subject's SQL file, or remove it when the project changes nothing
/// in the subject. Called by every save; it writes a file and touches no
/// database. Answers how many rows and whole sets it wrote.
pub fn write_sql(session: &mut EditSession, subject: &'static dyn RowSubject) -> Result<usize, String> {
    let plan = plan(session, subject);
    let vpath = subject.sql_vpath();
    for refused in &plan.refused {
        warn!("{EDITS_VPATH}: {refused}");
    }
    if plan.is_empty() {
        remove(&session.project, vpath)?;
        return Ok(0);
    }
    let mut body = format!(
        "-- {} \u{2014} what this project changes about {}.\n\
         -- Rewritten on every save from the project's own edits, so it is the\n\
         -- whole of what the project does rather than an increment of it.\n\
         --\n\
         {}\n\n",
        session.project.name,
        subject.sql_about(),
        subject.sql_note().trim_end()
    );
    for unit in plan.units() {
        body.push_str(&format!("-- {}\n", unit.heading));
        for statement in &unit.statements {
            body.push_str(statement);
            body.push('\n');
        }
        body.push('\n');
    }
    session
        .project
        .write(vpath, body.as_bytes())
        .map_err(|e| format!("{vpath}: {e}"))?;
    Ok(plan.count())
}

/// One subject's part of a save: its SQL file, and a status line. The stores
/// are written by [`save_stores`], which every save calls first.
pub fn save(session: &mut EditSession, subject: &'static dyn RowSubject) {
    match write_sql(session, subject) {
        Ok(0) => {}
        Ok(count) => {
            session.status = format!("{count} {} change(s) written to {}", subject.noun(), subject.sql_vpath())
        }
        Err(e) => {
            warn!("{}: {e}", subject.name());
            session.status = format!("saved, but writing {} failed: {e}", subject.sql_vpath());
        }
    }
}

/// The project's three server stores: the rows, the waypoint paths and the
/// scripts. `false` when one could not be written, with the reason already on
/// the status line.
pub fn save_stores(session: &mut EditSession) -> bool {
    session.save_server_edits() && session.save_server_paths() && session.save_server_scripts()
}

/// Every row subject's half of a save: the stores, then each subject's SQL in
/// [`Subject::ORDER`]. Nothing is written from a store that could not be
/// saved, so a SQL file never says more than the project folder holds.
pub fn save_all(session: &mut EditSession) {
    if !save_stores(session) {
        return;
    }
    for subject in Subject::ORDER {
        save(session, subject.row_subject());
    }
}

/// The default refusal of a row arriving at a key: a row already at the whole
/// key is not this project's, since the database holds none of this project's
/// rows while an Apply runs (see [`super::reconcile`]).
pub fn refuse_a_present_key(db: &mut Db, row: &Row) -> Result<(), String> {
    if db.row(&row::exists_query(row.table.name, &row.key))?.is_some() {
        return Err(format!(
            "{} {} is already in the database and is not this project's.",
            row.table.name,
            row.key.text()
        ));
    }
    Ok(())
}

/// The default undo of a removal: the `DELETE` first, so the entry can be run
/// against a row it has already restored (see [`super::reconcile`]), then the
/// row as it stands. `None` when the row is not there.
pub fn snapshot_one_row(db: &mut Db, row: &Row) -> Result<Option<Vec<String>>, String> {
    Ok(db.row(&row::row_query(row.table.name, &row.key))?.map(|held| {
        let mut out = vec![row::delete(row.table.name, &row.key)];
        out.extend(row::insert_from_row(row.table.name, &held));
        out
    }))
}

/// Refuse a row created or moved to an id that is taken or that something
/// names, with the table's own references; see
/// [`super::reconcile::refuse_a_taken_id`]. With a `limit`, an id above it is
/// refused first: a spawn's guid past the guid space stops the server.
pub fn refuse_a_taken_id(db: &mut Db, row: &Row, own: &Edits, limit: Option<u64>) -> Result<(), String> {
    if let Some(limit) = limit {
        super::reconcile::refuse_a_guid_past_the_limit(row.table.name, &row.key, limit)?;
    }
    super::reconcile::refuse_a_taken_id(db, row.table.name, &row.key, row.table.references, own)
}

/// The undo of a removal that takes rows of other tables with it, as the
/// statements that put them back: `deletes` first, so the entry can be run
/// against a row it has already restored, then the row as it stands, then
/// every row of each `(table, column)` in `dependents` that names its id, in
/// that order. `None` when the row is not there.
pub fn snapshot_with_dependents(
    db: &mut Db,
    row: &Row,
    deletes: Vec<String>,
    dependents: &[(&str, &str)],
) -> Result<Option<Vec<String>>, String> {
    let Some(held) = db.row(&row::row_query(row.table.name, &row.key))? else {
        return Ok(None);
    };
    let Some(id) = row.key.first() else {
        return Ok(None);
    };
    let mut out = deletes;
    out.extend(row::insert_from_row(row.table.name, &held));
    for (table, column) in dependents {
        for named in db.rows(&row::rows_where(table, column, id))? {
            out.extend(row::insert_from_row(table, &named));
        }
    }
    Ok(Some(out))
}

/// Take a file out of the project, if it is there. A file that is there and
/// cannot be removed is an error: a stale SQL file says the project changes
/// rows it no longer does.
fn remove(project: &vale_edit::project::Project, vpath: &str) -> Result<(), String> {
    project.revert(vpath).map(|_| ()).map_err(|e| format!("{vpath}: {e}"))
}

/// A plan over a row store alone, with no paths and no scripts.
#[cfg(test)]
pub(crate) fn plan_of(subject: &'static dyn RowSubject, edits: &Edits) -> Plan {
    let paths = vale_mangos::path::Paths::default();
    let scripts = vale_mangos::scripts::Scripts::default();
    plan_from(subject, &Stores { edits, paths: &paths, scripts: &scripts })
}
