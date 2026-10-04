//! What a save does for the server: the SQL it writes, the rows it applies,
//! and how to take them back, for the client tables whose rows the server
//! reads from its own SQL tables. [`MAPPED`] lists them: `Spell.dbc` becomes
//! `spell_template`, `TaxiNodes.dbc` becomes `taxi_nodes`,
//! `SkillLineAbility.dbc` becomes `skill_line_ability`, `AreaTable.dbc`
//! becomes `area_template`, and `AreaTrigger.dbc` becomes
//! `areatrigger_template`.
//!
//! ## The file is rewritten whole on every save
//!
//! The statements are a diff: each edited DBC against the file the archives
//! ship. They describe everything this project changes, not an increment of
//! it. Rewriting the file on every save therefore keeps it equal to what the
//! project does to the server, whatever order the edits were made in, and an
//! undone edit drops out of the file without needing a statement to undo it.
//! A folder of stamped files would instead be a set of overlapping increments
//! that must be applied in order.
//!
//! The stamped `migrations/<UTC>_world.sql` the plan describes is the output of
//! Publish and is built from this file. A migration is handed over once, which
//! is a different moment from a save.
//!
//! ## This file handles the DBC-backed tables; `creatures.rs` handles the others
//!
//! An edit to a mapped DBC is a diff of two files, so the project's copy of
//! the DBC is the complete record of it and the statements come from comparing
//! the two. `creature_template` and `creature` have no client file: an edit to
//! one is a value a person typed, kept in the project's own store, and applied
//! when a person presses a button rather than as part of a save.
//!
//! The two were once a single [`Plan`], and that did not fit: they have
//! different sources, different undo rules, different files and, since the
//! creature half no longer applies on save, different moments. See
//! [`super::creatures`], which holds the second.
//!
//! ## Reading the file the game shipped
//!
//! The diff needs both sides. `GameAssets`' chain answers a path such as
//! `DBFilesClient\Spell.dbc` with the project's edited copy, because that is
//! what the overlay does. `read_past_overlay` returns the archives' own bytes,
//! skipping whatever the host put in front of them. In the client, which
//! installs no overlay, it returns the same bytes as an ordinary read.
//!
//! ## How an apply is undone
//!
//! A spell edit is a row at `build = 5875`, and removing it makes vmangos fall
//! back to the highest shipped build, so undoing an edit is usually a
//! `DELETE`. The exception is the 1,012 entries that already ship a 5875 row:
//! deleting one would destroy a row this project does not own. The undo is
//! therefore taken per row, from the database, immediately before the write,
//! and is one of two statements: a `DELETE` for a row this project is about to
//! create, or an `UPDATE` back to the current column values for a row that
//! already exists. A flight node is a `taxi_nodes` dev row at build 5875 on
//! the same terms, written through `vale_mangos::taxi`. A skill line ability
//! is a `skill_line_ability` row at build 5875 with no lower build behind
//! it, since the loader reads that build alone, so its undo is a `DELETE`
//! only for a row this project added. An area is an `area_template` row
//! keyed by its entry alone, with no build, on the same terms. An area trigger
//! is an `areatrigger_template` dev row at build 5875, on the flight node's
//! terms. See [`Undo`], and `vale_mangos::spell::undo`,
//! `vale_mangos::taxi::undo`, `vale_mangos::skills::undo`,
//! `vale_mangos::area::undo` and `vale_mangos::trigger::undo`, which hold the
//! rule.
//!
//! There is no permanent apply. The durable output is the SQL file in the
//! project folder, which can be reviewed, and Publish hands it over as a
//! migration. The client half works the same way: its durable output is the
//! project folder, not the overlay.
//!
//! ## Why the comparison is a byte comparison
//!
//! A project's `Spell.dbc` is the whole table, so a field-by-field diff
//! formats 22,360 rows times 145 columns on every save. The fast path compares
//! the two records' bytes with a memcmp, and the result is exact:
//! `DbcFile::set_string` only appends to the string block (`append_text`), so
//! no existing string offset moves. Two records with equal bytes therefore
//! have equal field values and equal strings, and only a record whose bytes
//! differ can be a change.

use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_edit::dbc::DbcFile;
use vale_mangos::conn::Db;
use vale_mangos::row::Key;
use vale_mangos::area;
use vale_mangos::skills;
use vale_mangos::spell::{self, Assignment};
use vale_mangos::taxi;
use vale_mangos::trigger;
use bevy::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

/// Where the file lives under the project root.
pub const VPATH: &str = "sql\\world.sql";

/// Where the file that puts the server back lives, beside [`VPATH`]. See
/// [`Undo`].
pub const REVERT_VPATH: &str = "sql\\revert.sql";

/// The client tables whose rows the server reads from its own SQL table, as
/// `(DBC name, the table it becomes)`.
///
/// Every other DBC the server reads, it reads as a file in `DataDir\5875\dbc\`, and
/// a publish copies it there; see `super::release::copy_server_dbcs`.
pub const MAPPED: [(&str, &str); 5] = [
    ("Spell", spell::TABLE),
    ("TaxiNodes", taxi::TABLE),
    ("SkillLineAbility", skills::TABLE),
    ("AreaTable", area::TABLE),
    ("AreaTrigger", trigger::TEMPLATE),
];

/// Whether the server has a `.reload` for a mapped table. `spell_template` has
/// one; `taxi_nodes`, `skill_line_ability`, `area_template` and
/// `areatrigger_template` are read once at startup and have none.
pub fn reloadable(table: &str) -> bool {
    table == spell::TABLE
}

/// The sentence for a row the server's table cannot hold.
pub fn refusal(dbc: &str, id: u32) -> String {
    match dbc {
        "TaxiNodes" => format!(
            "flight node {id} cannot go to the server: node ids stop at {}",
            taxi::MAX_ID
        ),
        "SkillLineAbility" => format!(
            "skill line ability {id} cannot go to the server: its id and its two spells \
             must each be at most {}, since skill_line_ability holds them as smallints",
            skills::MAX_SMALLINT
        ),
        "AreaTable" => format!(
            "area {id} cannot go to the server: its explore bit must be under {}, its \
             name at most {} bytes, and its team and liquid type at most {}",
            area::EXPLORE_BITS,
            area::MAX_NAME,
            area::MAX_TINYINT
        ),
        "AreaTrigger" => format!(
            "area trigger {id} cannot go to the server: trigger ids stop at {}, since \
             areatrigger_template.id is a smallint",
            trigger::MAX_ID
        ),
        _ => format!(
            "spell {id} cannot go to the server: ids above {} do not fit \
             spell_template.entry, which is a smallint",
            spell::MAX_ENTRY
        ),
    }
}

/// The change to one server row: the columns it sets, and the statements that
/// set them.
#[derive(Debug)]
pub struct Row {
    /// The server table it lands in.
    pub table: &'static str,
    /// Which row of that table: `(entry, build)` for a spell, `(id, build)` for
    /// a flight node and for a skill line ability, `entry` for an area. See
    /// `vale_mangos::row::Key`.
    pub key: Key,
    /// The columns whose values this project sets.
    pub changes: Vec<Assignment>,
    /// The statements that apply [`Row::changes`], in order.
    pub statements: Vec<String>,
}

impl Row {
    /// One line naming the row, for a status line and for the undo file's
    /// marker.
    pub fn names(&self) -> String {
        format!("{} {}", self.table, self.key.text())
    }
}

/// Everything a project changes on the server.
#[derive(Debug, Default)]
pub struct Plan {
    /// The server tables with at least one row in them, in the order they were
    /// walked.
    pub tables: Vec<&'static str>,
    pub rows: Vec<Row>,
    /// Rows the server's table cannot hold, as `(DBC name, id)`; [`refusal`]
    /// words each one. For a spell see `vale_mangos::spell::MAX_ENTRY`:
    /// `spell_template.entry` is a `smallint unsigned` and `Spell.dbc`'s id is
    /// a `u32`, so a spell can be valid in the client's file and impossible to
    /// store on the server. For a flight node see `vale_mangos::taxi::MAX_ID`.
    /// Each one is reported rather than skipped, because leaving one out
    /// without a word hides the change in the same way the database clamping
    /// it would.
    pub refused: Vec<(&'static str, u32)>,
    /// Rows the archives ship that the project's copy of the DBC no longer
    /// has, as `(DBC name, id)`. Nothing is written for them: the server's
    /// table keeps the row, and removing it would remove a row this project
    /// does not own. Reported so the difference is known.
    pub removed: Vec<(&'static str, u32)>,
}

impl Plan {
    pub fn nothing(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn statements(&self) -> usize {
        self.rows.iter().map(|row| row.statements.len()).sum()
    }

    /// Whether any change touches a column vmangos reads. An edit only to a
    /// spell's description or tooltip changes the client's file and nothing on
    /// the server (see `vale_mangos::spell::IGNORED`), and a reload sent for
    /// it would report the change as live on both sides when it reached only
    /// the client. Every `taxi_nodes` and `skill_line_ability` change counts
    /// as reaching the server: a plan holds a row of either only when a
    /// column the server reads changed.
    pub fn reaches_the_server(&self) -> bool {
        self.rows
            .iter()
            .any(|row| row.table != spell::TABLE || spell::reaches_the_server(&row.changes))
    }

    /// Whether any row is in a table the server reads only at startup.
    pub fn needs_a_restart(&self) -> bool {
        !self.read_at_startup().is_empty()
    }

    /// The tables with a row in this plan that the server reads only at
    /// startup, in [`MAPPED`]'s order.
    pub fn read_at_startup(&self) -> Vec<&'static str> {
        MAPPED
            .iter()
            .map(|(_, table)| *table)
            .filter(|table| !reloadable(table) && self.rows.iter().any(|row| row.table == *table))
            .collect()
    }

    /// A hash of what this plan would do to the database.
    ///
    /// Two plans with the same signature apply the same statements, so applying
    /// the second changes nothing, and a `.reload` sent for it would report a
    /// change that was already live. [`save`] compares signatures: a project
    /// carrying an edit from a previous session re-applies it on the first save
    /// of this session and not after.
    ///
    /// The hash covers the statements, not the row count, because two
    /// different edits to the same row are one row but produce two different
    /// databases.
    pub fn signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for row in &self.rows {
            for statement in &row.statements {
                statement.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// One line for the status bar.
    pub fn line(&self) -> String {
        if self.nothing() {
            return "nothing for the server".to_string();
        }
        match self.reaches_the_server() {
            true => format!(
                "{} row(s), {} statement(s)",
                self.rows.len(),
                self.statements()
            ),
            // Stated in words, because a count alone would read as a change
            // that reached the server.
            false => format!(
                "{} row(s) — none of it is a column the server reads",
                self.rows.len()
            ),
        }
    }
}

/// What an apply did.
#[derive(Debug, Default)]
pub struct Applied {
    pub rows: usize,
    /// Rows the database reported as affected. Not the statement count: two of
    /// every three statements are `INSERT IGNORE` and most of them do nothing.
    pub affected: u64,
    /// How many entries the undo file learnt about on this apply.
    pub newly_undoable: usize,
    pub tables: Vec<&'static str>,
}

/// The server half of a save, called from the three Save call sites. It works
/// out the plan, writes the file, and, when there is a database to talk to,
/// returns the write that applies the plan and keeps the undo.
///
/// It never fails a save. A project with no mapped table edited does nothing,
/// which covers every save of every terrain project. A failure anywhere is
/// reported on the status line and leaves the DBC and the tiles written,
/// because those are what the person asked to save.
///
/// The database write is returned rather than run, for the caller to queue;
/// see [`crate::server::queue`]. The result is `None` when nothing is to reach
/// the database. The write's own finish reports what happened and requests
/// the reloads, so a reload is never sent for a change that did not reach the
/// database.
pub fn save(
    session: &mut EditSession,
    assets: &GameAssets,
    server: &crate::server::settings::ServerSettings,
) -> Option<crate::server::queue::Work> {
    let plan = match plan(session, assets) {
        Ok(plan) => plan,
        Err(e) => {
            warn!("server: {e}");
            session.status = format!("saved, but the server's half did not: {e}");
            return None;
        }
    };
    if let Err(e) = write_file(session, &plan) {
        warn!("server: {e}");
        session.status = format!("saved, but {VPATH} did not: {e}");
        return None;
    }
    // Reported whether or not anything else happens: a refused row is a change
    // the person made that will not reach the server, and without this line
    // the save would look like a success.
    for &(dbc, id) in &plan.refused {
        let line = refusal(dbc, id);
        warn!("server: {line}");
        session.status = line;
    }
    for &(dbc, id) in &plan.removed {
        warn!(
            "server: {dbc} {id} is removed from the project's {dbc}.dbc; the server's own row \
             is kept"
        );
    }
    if plan.nothing() {
        // Nothing to report and nothing to apply. Every terrain save ends here.
        //
        // The exception is a project that has rows in the database and no
        // longer claims any, which happens when every edit to a mapped table
        // has been taken back since the last apply. The database should hold
        // what it held before plus what the project claims, and the project
        // claims nothing, so the applied rows are put back. See
        // `super::reconcile`.
        if server.apply_on_save && server.resolve().is_some() {
            match revert_work(session, server) {
                Ok(work) => return work,
                Err(e) => warn!("server: {e}"),
            }
        }
        return None;
    }
    if !server.apply_on_save {
        info!("server: {} into {VPATH}, not applied", plan.line());
        session.status = format!("{} into {VPATH} — applying is off", plan.line());
        return None;
    }
    // Skip the apply when the database already holds this plan.
    //
    // The statements are a diff of the project, not of what changed since the
    // last save, so a project carrying an edit from a previous session emits the
    // same statements on every save and at the start of every playtest. Applying
    // them again is harmless, but the `.reload` that follows is not: it reports
    // a change as just gone live when nothing changed, and a status that
    // reports a change on every save no longer tells the person anything.
    //
    // The signature is compared per process and not stored in the project. The
    // editor cannot know what the database holds across launches: a row may
    // have been changed by hand, by another project, or by a database update.
    // So the first save of a session applies, and later saves that would apply
    // the same statements do not.
    if session.applied_signature == Some(plan.signature()) {
        info!("server: {} already applied", plan.line());
        session.status = format!("{} — already applied", plan.line());
        return None;
    }
    match apply_work(session, plan, server) {
        Ok(work) => Some(work),
        Err(e) => {
            warn!("server: {e}");
            session.status = format!("saved, but the server rows were not applied: {e}");
            None
        }
    }
}

fn line_for(plan: &Plan, applied: &Applied) -> String {
    let undo = match applied.newly_undoable {
        0 => String::new(),
        n => format!(", {n} newly undoable"),
    };
    let restart = match plan.read_at_startup().as_slice() {
        [] => String::new(),
        [one] => format!(" \u{2014} {one} is read at startup: restart the server"),
        many => format!(
            " \u{2014} {} are read at startup: restart the server",
            many.join(" and ")
        ),
    };
    let undo = format!("{undo}{restart}");
    match plan.reaches_the_server() {
        true => format!(
            "{} row(s) applied, {} row(s) affected{undo}",
            applied.rows, applied.affected
        ),
        false => format!(
            "{} row(s) applied{undo} — none of it is a column the server reads",
            applied.rows
        ),
    }
}

/// What a project changes on the server, worked out once and used three ways:
/// written to the file, applied, and undone.
///
/// Reads the project's own copy of each table in [`MAPPED`] rather than the
/// copy open in this session, so the plan is complete across sessions: a
/// project edited yesterday and opened today still has its rows without the
/// table being opened.
///
/// The shipped side of the diff is read from the install without this
/// project's own patch archives, as well as without the overlay. A published
/// project's `Spell.dbc` is in `Data\` from the next launch, and a diff
/// against a chain that includes it compares the edit with itself. The diff
/// was then empty, so a save removed the project's rows from the database
/// while the client still shipped the edit. See `Project::published`.
pub fn plan(session: &EditSession, assets: &GameAssets) -> Result<Plan, String> {
    let mut out = Plan::default();
    let own: Vec<String> = session
        .project
        .published()
        .into_iter()
        .map(|archive| archive.name)
        .collect();
    for (table, server_table) in MAPPED {
        let path = vale_assets::tables::dbc::dbc_path(table);
        let Some(edited) = session.project.read(&path) else {
            continue;
        };
        let edited = DbcFile::parse(&edited).map_err(|e| format!("{path}: {e}"))?;
        let shipped = assets.read_past_overlay_skipping(&path, &own)?;
        let shipped = DbcFile::parse(&shipped).map_err(|e| format!("{path} (shipped): {e}"))?;

        let mut any = false;
        match table {
            "TaxiNodes" => {
                use vale_edit::dbc::taxi as nodes;
                let before: HashMap<u32, nodes::Node> = nodes::nodes(&shipped)
                    .into_iter()
                    .map(|node| (node.id, node))
                    .collect();
                for entry in changed_entries(&shipped, &edited) {
                    let Some(node) = edited
                        .row_of(entry)
                        .and_then(|record| nodes::node_at(&edited, record))
                    else {
                        continue;
                    };
                    let was = before.get(&entry);
                    let changes = taxi::changes(was, &node);
                    if changes.is_empty() {
                        continue;
                    }
                    if !taxi::fits(entry) {
                        out.refused.push((table, entry));
                        continue;
                    }
                    any = true;
                    out.rows.push(Row {
                        table: server_table,
                        key: taxi::key(entry),
                        statements: taxi::statements(was, &node),
                        changes,
                    });
                }
            }
            "AreaTrigger" => {
                use vale_edit::dbc::places;
                let before: HashMap<u32, places::Trigger> = places::triggers(&shipped)
                    .into_iter()
                    .map(|trigger| (trigger.id, trigger))
                    .collect();
                for entry in changed_entries(&shipped, &edited) {
                    let Some(volume) = edited
                        .row_of(entry)
                        .and_then(|record| places::trigger_at(&edited, record))
                    else {
                        continue;
                    };
                    let was = before.get(&entry);
                    let changes = trigger::changes(was, &volume);
                    if changes.is_empty() {
                        continue;
                    }
                    if !trigger::fits(entry) {
                        out.refused.push((table, entry));
                        continue;
                    }
                    any = true;
                    out.rows.push(Row {
                        table: server_table,
                        key: trigger::key(entry),
                        statements: trigger::statements(was, &volume),
                        changes,
                    });
                }
            }
            "SkillLineAbility" => {
                for entry in changed_entries(&shipped, &edited) {
                    let changes = skills::changes(&shipped, &edited, entry);
                    if changes.is_empty() {
                        continue;
                    }
                    if !skills::fits(&edited, entry) {
                        out.refused.push((table, entry));
                        continue;
                    }
                    any = true;
                    out.rows.push(Row {
                        table: server_table,
                        key: skills::key(entry),
                        statements: skills::statements(&shipped, &edited, entry),
                        changes,
                    });
                }
            }
            "AreaTable" => {
                for entry in changed_entries(&shipped, &edited) {
                    let changes = area::changes(&shipped, &edited, entry);
                    if changes.is_empty() {
                        continue;
                    }
                    if !area::fits(&edited, entry) {
                        out.refused.push((table, entry));
                        continue;
                    }
                    any = true;
                    out.rows.push(Row {
                        table: server_table,
                        key: area::key(entry),
                        statements: area::statements(&shipped, &edited, entry),
                        changes,
                    });
                }
            }
            _ => {
                for entry in changed_entries(&shipped, &edited) {
                    let changes = spell::changes(&shipped, &edited, entry);
                    if changes.is_empty() {
                        continue;
                    }
                    if !spell::fits(entry) {
                        out.refused.push((table, entry));
                        continue;
                    }
                    any = true;
                    out.rows.push(Row {
                        table: server_table,
                        key: Key::two(
                            ("entry", u64::from(entry)),
                            ("build", u64::from(spell::BUILD)),
                        ),
                        statements: spell::statements(&shipped, &edited, entry),
                        changes,
                    });
                }
            }
        }
        for id in removed_entries(&shipped, &edited) {
            out.removed.push((table, id));
        }
        if any {
            out.tables.push(server_table);
        }
    }
    Ok(out)
}

/// Rewrites the project's `sql\world.sql`, or removes it when the plan is
/// empty.
pub fn write_file(session: &EditSession, plan: &Plan) -> Result<Option<PathBuf>, String> {
    let Some(disk) = session.project.path_for(VPATH) else {
        return Err(format!("{VPATH} leaves the project"));
    };
    if plan.nothing() {
        // A project with every edit undone removes its file. Leaving the last
        // save's statements behind would claim a change the project no longer
        // carries, and Publish would hand that change over.
        let _ = std::fs::remove_file(&disk);
        return Ok(None);
    }
    let mut body = format!(
        "-- {} — what this project changes on the server.\n\
         -- Written by the editor on every save, from the difference between the\n\
         -- project's DBCs and the files the archives ship. Rewritten whole each\n\
         -- time, so it is the project's change and not an increment of it.\n\n",
        session.project.name
    );
    for table in &plan.tables {
        body.push_str(&format!("-- {table}\n"));
        for row in plan.rows.iter().filter(|row| row.table == *table) {
            for statement in &row.statements {
                body.push_str(statement);
                body.push('\n');
            }
        }
        body.push('\n');
    }
    session
        .project
        .write(VPATH, body.as_bytes())
        .map_err(|e| format!("{VPATH}: {e}"))?;
    Ok(Some(disk))
}

/// The database address, from whichever source this machine uses to name the
/// server.
///
/// One function rather than `Where::find()` at each call site, because the
/// server's location has three sources: two environment variables and the
/// editor's own panel. A call site that read only the environment variables
/// would ignore the panel's setting without reporting it. See
/// [`crate::server::settings::ServerSettings::resolve`].
fn address(
    server: &crate::server::settings::ServerSettings,
) -> Result<vale_mangos::conn::Where, String> {
    server
        .resolve()
        .map(|(at, _source)| at)
        .ok_or_else(vale_mangos::conn::Where::absent)
}

/// Applies the plan to the server's database and keeps the undo.
///
/// The order of the steps is what makes this safe. For every row not already
/// covered by the revert file, the dev row is read in its current state and the
/// statement that puts it back is appended to `sql\revert.sql`, and the file is
/// written before any statement runs. A crash between the two leaves an undo
/// for a change that did not happen, which is harmless because it restores
/// what is already there. The reverse order could leave a change with no undo.
///
/// What the project applied before is put back first, in the same order and
/// for the same reason as [`super::reconcile`]: a row whose edit was taken back
/// since the last apply is in no plan, and only a put-back removes its dev row.
/// Every snapshot is then taken of a database that holds none of this project,
/// so it is always the earliest state. The rows of this plan do not depend on
/// each other, since each is one spell's, one flight node's or one skill line
/// ability's row at build 5875, so they
/// are snapshotted together and run as one list.
pub fn apply_at(
    project: &vale_edit::project::Project,
    plan: &Plan,
    at: &vale_mangos::conn::Where,
) -> Result<Applied, String> {
    let mut db = Db::open(at)?;
    let had = Undo::open_at(project)?;
    if !had.entries.is_empty() {
        let back: Vec<String> = had
            .entries
            .iter()
            .rev()
            .map(|(_, _, sql)| sql.clone())
            .collect();
        db.run(&back).map_err(|e| {
            format!("putting back what this project applied before did not finish: {e}")
        })?;
        forget_revert_file(project)?;
    }
    let mut undo = Undo {
        entries: Vec::new(),
    };
    let mut out = Applied {
        rows: plan.rows.len(),
        tables: plan.tables.clone(),
        ..Applied::default()
    };

    for row in &plan.rows {
        let Some(entry) = row.key.first() else {
            continue;
        };
        let entry = entry as u32;
        let statement = match row.table {
            taxi::TABLE => {
                let now = db.row(&taxi::dev_row_query(entry))?;
                taxi::undo(entry, &row.changes, now.as_ref())
            }
            skills::TABLE => {
                let now = db.row(&skills::dev_row_query(entry))?;
                skills::undo(entry, &row.changes, now.as_ref())
            }
            area::TABLE => {
                let now = db.row(&area::dev_row_query(entry))?;
                area::undo(entry, &row.changes, now.as_ref())
            }
            trigger::TEMPLATE => {
                let now = db.row(&trigger::dev_row_query(entry))?;
                trigger::undo(entry, &row.changes, now.as_ref())
            }
            _ => {
                let now = db.row(&spell::dev_row_query(entry))?;
                spell::undo(entry, &row.changes, now.as_ref())
            }
        };
        let Some(statement) = statement else {
            continue;
        };
        undo.add(row.table, &row.key, &statement);
        out.newly_undoable += 1;
    }
    undo.write(project)?;

    let statements: Vec<String> = plan
        .rows
        .iter()
        .flat_map(|row| row.statements.iter().cloned())
        .collect();
    out.affected = db.run(&statements)?;
    Ok(out)
}

/// Puts the server back (the Server panel's Put back) and removes the undo
/// file.
///
/// The file is removed only after the statements have run. A revert that fails
/// part way keeps its file, so it can be run again or read by hand.
pub fn revert_at(
    project: &vale_edit::project::Project,
    at: &vale_mangos::conn::Where,
) -> Result<Reverted, String> {
    let undo = Undo::open_at(project)?;
    if undo.entries.is_empty() {
        return Ok(Reverted::default());
    }
    let mut db = Db::open(at)?;
    let statements: Vec<String> = undo.entries.iter().map(|(_, _, sql)| sql.clone()).collect();
    // Removals are counted from the statements before they run, not from what
    // the database reports: a `DELETE` and an `UPDATE` each affect one row, so
    // only the verb distinguishes them. See [`Reverted::removals`] for why the
    // count is reported.
    let removals = statements
        .iter()
        .filter(|sql| sql.trim_start().starts_with("DELETE"))
        .count();
    let affected = db.run(&statements)?;
    let tables: Vec<&'static str> = MAPPED
        .iter()
        .map(|(_, table)| *table)
        .filter(|table| undo.entries.iter().any(|(had, _, _)| had == table))
        .collect();
    let out = Reverted {
        rows: undo.entries.len(),
        affected,
        tables,
        removals,
    };
    forget_revert_file(project)?;
    Ok(out)
}

/// Removes `sql\revert.sql` once its statements have run, and returns an error
/// when it cannot: a file left behind is put back again at every later Apply.
fn forget_revert_file(project: &vale_edit::project::Project) -> Result<(), String> {
    let Some(disk) = project.path_for(REVERT_VPATH) else {
        return Ok(());
    };
    match std::fs::remove_file(&disk) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "{REVERT_VPATH} has been put back but could not be removed: {e}. Remove it by \
             hand before the next Apply."
        )),
    }
}

/// [`apply_at`] with the session's project and the panel's address, run on the
/// calling thread.
pub fn apply(
    session: &mut EditSession,
    plan: &Plan,
    server: &crate::server::settings::ServerSettings,
) -> Result<Applied, String> {
    apply_at(&session.project, plan, &address(server)?)
}

/// [`revert_at`] with the session's project and the panel's address, run on
/// the calling thread. Used by the scripted `--revert`.
pub fn revert(
    session: &mut EditSession,
    server: &crate::server::settings::ServerSettings,
) -> Result<Reverted, String> {
    revert_at(&session.project, &address(server)?)
}

/// An Apply of the plan as a queued write; see [`crate::server::queue`]. The
/// finish stores the plan's signature and requests a reload for each
/// reloadable table, but only when a change reaches a column the server reads:
/// a reload sent for an edited tooltip would report as live a change that
/// never reached the server. A table with no reload (`taxi_nodes`,
/// `skill_line_ability`) gets a restart notice on the status line instead.
pub fn apply_work(
    session: &EditSession,
    plan: Plan,
    server: &crate::server::settings::ServerSettings,
) -> Result<crate::server::queue::Work, String> {
    let project = session.project.clone();
    let at = address(server)?;
    Ok(Box::new(move || {
        let result = apply_at(&project, &plan, &at);
        Box::new(
            move |session: &mut EditSession, reloads: &mut crate::server::reload::Reloads| {
                session.status = match result {
                    Ok(applied) => {
                        session.applied_signature = Some(plan.signature());
                        if plan.reaches_the_server() {
                            for table in applied.tables.iter().filter(|t| reloadable(t)) {
                                reloads.when_there_is_a_session(table);
                            }
                        }
                        info!("server: {}", line_for(&plan, &applied));
                        line_for(&plan, &applied)
                    }
                    // The file is written and the database is not. The status
                    // line says so, because the project still carries the
                    // change and can be applied later.
                    Err(e) => {
                        warn!("server: {e}");
                        format!("{} into {VPATH}, not applied: {e}", plan.line())
                    }
                };
            },
        )
    }))
}

/// A Put back as a queued write. `None` when this project has applied no row
/// to a mapped table.
pub fn revert_work(
    session: &EditSession,
    server: &crate::server::settings::ServerSettings,
) -> Result<Option<crate::server::queue::Work>, String> {
    let applied = Undo::open(session).is_ok_and(|undo| !undo.entries.is_empty());
    if !applied {
        return Ok(None);
    }
    let project = session.project.clone();
    let at = address(server)?;
    Ok(Some(Box::new(move || {
        let result = revert_at(&project, &at);
        Box::new(
            move |session: &mut EditSession, reloads: &mut crate::server::reload::Reloads| {
                session.status = match result {
                    Ok(done) if done.rows == 0 => "this project has applied nothing".to_string(),
                    Ok(done) => {
                        session.applied_signature = None;
                        for table in done.tables.iter().filter(|t| reloadable(t)) {
                            reloads.when_there_is_a_session(table);
                        }
                        info!("server: {}", done.line());
                        done.line()
                    }
                    Err(e) => {
                        warn!("server: {e}");
                        format!("server rows: {e}")
                    }
                };
            },
        )
    })))
}

/// What a revert did.
#[derive(Debug, Default)]
pub struct Reverted {
    pub rows: usize,
    pub affected: u64,
    pub tables: Vec<&'static str>,
    /// How many of the rows were removed rather than restored. The person is
    /// told this number.
    ///
    /// A restored spell row goes live on `.reload spell_template`. A removed
    /// one does not and cannot: `SpellMgr::LoadSpells` (`SpellMgr.cpp:3599`)
    /// grows `mSpellEntryMap` and overwrites the entries its query returns, and
    /// never clears the map, so a spell whose row is gone keeps its in-memory
    /// entry until the server restarts. The reload adds and overwrites; it does
    /// not remove.
    ///
    /// The count is reported and not worked around, because no GM command
    /// drops a spell from that map.
    pub removals: usize,
}

impl Reverted {
    /// One line for the status bar: what was put back, and which removals
    /// still need a server restart.
    pub fn line(&self) -> String {
        let put_back = format!("put back {} row(s), {} affected", self.rows, self.affected);
        match self.removals {
            0 => put_back,
            n => format!(
                "{put_back} \u{2014} {n} of them removed a row, which a reload cannot undo: \
                 the server keeps it until it restarts"
            ),
        }
    }
}

/// `sql\revert.sql`, the statements that put the server back.
///
/// The file is plain SQL with a marker line before each statement naming the
/// table and the row. An apply reads the markers to learn which rows it has
/// already snapshotted. A side file in another serialisation would be a second
/// format, and the undo would no longer be a file a person can read and run by
/// hand, which is what lets the person check it.
///
/// ```text
/// -- row spell_template entry=133;build=5875
/// -- row creature_template entry=68;patch=0
/// -- entry 133                    (what this file said before creatures)
/// ```
///
/// The third form is read and never written. A project applied by an earlier
/// build of this editor has a file of them, and each means `spell_template`,
/// the only table that build wrote. Failing to recognise one would re-snapshot
/// a row that is already edited, and Revert would then restore the edit
/// instead of removing it.
pub struct Undo {
    /// `(table, key, statement)`, oldest first.
    pub entries: Vec<(String, Key, String)>,
}

impl Undo {
    const MARKER: &'static str = "-- row ";
    /// The marker used before the file named a table. Read, never written.
    const OLD_MARKER: &'static str = "-- entry ";

    pub fn open(session: &EditSession) -> Result<Undo, String> {
        Undo::open_at(&session.project)
    }

    /// [`Undo::open`] from a project handle, which is what a write running off
    /// the main thread holds. See [`crate::server::queue`].
    pub fn open_at(project: &vale_edit::project::Project) -> Result<Undo, String> {
        let Some(text) = project.read(REVERT_VPATH) else {
            return Ok(Undo {
                entries: Vec::new(),
            });
        };
        let text = String::from_utf8(text).map_err(|e| format!("{REVERT_VPATH}: {e}"))?;
        Ok(Undo::from_text(&text))
    }

    /// The parse on its own, so it can be tested without a project on disk.
    pub fn from_text(text: &str) -> Undo {
        let mut entries = Vec::new();
        let mut at: Option<(String, Key)> = None;
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix(Self::MARKER) {
                at = rest
                    .split_once(' ')
                    .and_then(|(table, key)| Some((table.trim().to_string(), Key::parse(key)?)));
                continue;
            }
            if let Some(rest) = line.strip_prefix(Self::OLD_MARKER) {
                // The old form: an entry with no table and no build, which was
                // always a spell's dev row.
                at = rest.trim().parse::<u64>().ok().map(|entry| {
                    (
                        spell::TABLE.to_string(),
                        Key::two(("entry", entry), ("build", u64::from(spell::BUILD))),
                    )
                });
                continue;
            }
            // A comment that is not a marker clears nothing and carries
            // nothing, so the file's own header cannot become an entry.
            if line.is_empty() || line.starts_with("--") {
                continue;
            }
            if let Some((table, key)) = at.take() {
                entries.push((table, key, line.to_string()));
            }
        }
        Undo { entries }
    }

    /// Whether this row has already been snapshotted.
    ///
    /// The check uses the table and the key together. `creature_template` entry
    /// 68 and `spell_template` entry 68 are different rows, and a check on the
    /// id alone would give the second of them the first one's undo.
    pub fn covers(&self, table: &str, key: &Key) -> bool {
        self.entries
            .iter()
            .any(|(had_table, had_key, _)| had_table == table && had_key == key)
    }

    pub fn add(&mut self, table: &str, key: &Key, statement: &str) {
        self.entries
            .push((table.to_string(), key.clone(), statement.to_string()));
    }

    pub fn write(&self, project: &vale_edit::project::Project) -> Result<(), String> {
        if self.entries.is_empty() {
            return Ok(());
        }
        let mut body = format!(
            "-- {} — how to put the server back.\n\
             -- Written before each apply, and only for rows it had not already\n\
             -- covered, so running this returns them to what they were before\n\
             -- this project first touched them. Plain SQL: it can be run by hand.\n\n",
            project.name
        );
        for (table, key, statement) in &self.entries {
            body.push_str(&format!(
                "{}{table} {}\n{statement}\n\n",
                Self::MARKER,
                key.text()
            ));
        }
        project
            .write(REVERT_VPATH, body.as_bytes())
            .map_err(|e| format!("{REVERT_VPATH}: {e}"))?;
        Ok(())
    }
}

/// `--revert`: the popover's Put back button, run from the command line. See
/// [`crate::Args::revert`].
///
/// Runs once, on the first frame that has a session to read the project from,
/// and never again. A flag that ran on every frame would put the server back
/// after every save that had just applied something.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    server: Res<crate::server::settings::ServerSettings>,
    mut done: Local<bool>,
) {
    if *done || !args.revert {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    match revert(&mut session, &server) {
        Ok(done) if done.rows == 0 => {
            info!("--revert: this project has applied nothing");
            session.status = "nothing to put back".into();
        }
        Ok(done) => {
            info!("--revert: {}", done.line());
            session.status = done.line();
        }
        Err(e) => {
            warn!("--revert: {e}");
            session.status = format!("could not put the server back: {e}");
        }
    }
}

/// The entries whose record bytes differ, plus every entry the shipped file
/// does not have at all.
///
/// Comparing record bytes is what keeps a save from taking seconds. The module
/// comment explains why the comparison is exact and not only a filter.
fn changed_entries(shipped: &DbcFile, edited: &DbcFile) -> Vec<u32> {
    let mut was: HashMap<u32, usize> = HashMap::with_capacity(shipped.record_count());
    for record in 0..shipped.record_count() {
        if let Some(id) = shipped.u32_at(record, 0) {
            was.insert(id, record);
        }
    }
    let mut out = Vec::new();
    for record in 0..edited.record_count() {
        let Some(id) = edited.u32_at(record, 0) else {
            continue;
        };
        let now = edited.record_bytes(record);
        let before = was.get(&id).and_then(|&row| shipped.record_bytes(row));
        if before != now {
            out.push(id);
        }
    }
    out
}

/// The ids the shipped file has and the edited one does not.
fn removed_entries(shipped: &DbcFile, edited: &DbcFile) -> Vec<u32> {
    let now: std::collections::HashSet<u32> = (0..edited.record_count())
        .filter_map(|record| edited.u32_at(record, 0))
        .collect();
    (0..shipped.record_count())
        .filter_map(|record| shipped.u32_at(record, 0))
        .filter(|id| !now.contains(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The undo file reads back the entries it wrote, through its marker.
    ///
    /// This is the only parsing in this module. The marker is what stops a
    /// second apply from overwriting the first snapshot: a marker that was no
    /// longer recognised would read as "this row has no undo yet", re-snapshot
    /// the already-edited row, and leave Revert restoring the edit instead of
    /// removing it, with no error reported.
    #[test]
    fn the_undo_file_reads_back_what_it_wrote() {
        let written = "-- a project — how to put the server back.\n\
             -- a second comment line that is not a marker\n\n\
             -- row spell_template entry=133;build=5875\n\
             UPDATE `spell_template` SET `effectBasePoints1` = 13 WHERE `entry` = 133;\n\n\
             -- row spell_template entry=2136;build=5875\n\
             DELETE FROM `spell_template` WHERE `entry` = 2136 AND `build` = 5875;\n";
        let undo = Undo::from_text(written);
        assert_eq!(undo.entries.len(), 2);
        assert_eq!(undo.entries[0].0, spell::TABLE);
        assert!(undo.entries[0].2.starts_with("UPDATE"));
        assert!(undo.entries[1].2.starts_with("DELETE"));
        assert!(undo.covers(spell::TABLE, &Key::two(("entry", 133), ("build", 5875))));
        assert!(!undo.covers(spell::TABLE, &Key::two(("entry", 1), ("build", 5875))));
    }

    /// An undo covers a row by its whole key and its table. A dev row is
    /// `(entry, build)`, and an undo matched on the entry alone would treat a
    /// row at another build as covered by this one's snapshot.
    #[test]
    fn the_whole_key_is_part_of_what_an_undo_covers() {
        let mut undo = Undo {
            entries: Vec::new(),
        };
        undo.add(
            spell::TABLE,
            &Key::two(("entry", 133), ("build", 5875)),
            "UPDATE `spell_template` …;",
        );
        assert!(!undo.covers(spell::TABLE, &Key::two(("entry", 133), ("build", 5464))));
        assert!(!undo.covers(
            "creature_template",
            &Key::two(("entry", 133), ("build", 5875))
        ));
    }

    /// A file written before the marker named a table is still read.
    ///
    /// The old marker was `-- entry <id>`, and its table was always
    /// `spell_template`. A project applied by an earlier build has a file of
    /// them. Failing to recognise one would re-snapshot a row that is already
    /// edited, and Revert would then restore the edit rather than remove it.
    #[test]
    fn the_old_marker_is_still_read_as_a_spell() {
        let undo = Undo::from_text(
            "-- entry 133\n\
             UPDATE `spell_template` SET `effectBasePoints1` = 13 WHERE `entry` = 133;\n",
        );
        assert_eq!(undo.entries.len(), 1);
        assert_eq!(undo.entries[0].0, spell::TABLE);
        assert!(undo.covers(spell::TABLE, &Key::two(("entry", 133), ("build", 5875))));
    }

    /// A comment that is not a marker carries no statement with it, so a header
    /// cannot become an entry.
    #[test]
    fn a_comment_is_not_an_entry() {
        assert!(Undo::from_text("-- just a note\nUPDATE x;\n")
            .entries
            .is_empty());
    }

    /// Two plans with the same statements have the same signature. This stops
    /// a playtest from re-applying an unchanged project and sending a `.reload`
    /// that reports a change nobody made.
    #[test]
    fn the_signature_is_the_statements_and_nothing_else() {
        let row = |value: &str| Row {
            table: spell::TABLE,
            key: Key::two(("entry", 133), ("build", 5875)),
            changes: Vec::new(),
            statements: vec![format!(
                "UPDATE `spell_template` SET `effectBasePoints1` = {value};"
            )],
        };
        let one = Plan {
            tables: vec![spell::TABLE],
            rows: vec![row("13")],
            refused: Vec::new(),
            removed: Vec::new(),
        };
        let same = Plan {
            tables: vec![spell::TABLE],
            rows: vec![row("13")],
            refused: Vec::new(),
            removed: Vec::new(),
        };
        let other = Plan {
            tables: vec![spell::TABLE],
            rows: vec![row("41")],
            refused: Vec::new(),
            removed: Vec::new(),
        };
        assert_eq!(one.signature(), same.signature());
        assert_ne!(one.signature(), other.signature());
        assert_eq!(Plan::default().signature(), Plan::default().signature());
    }

    /// `vale-assets` builds a `.dbc` path for every DBC name in [`MAPPED`].
    /// [`plan`] reads the project folder by that path.
    #[test]
    fn every_mapped_table_has_a_path() {
        for (table, _) in MAPPED {
            let path = vale_assets::tables::dbc::dbc_path(table);
            assert!(path.ends_with(".dbc"), "{path}");
            assert!(path.contains(table), "{path}");
        }
    }

    /// A plan names the tables of its rows that the server reads only at
    /// startup, in the mapped list's order, and a spell row is not one: its
    /// table has a reload.
    #[test]
    fn a_plan_names_the_tables_read_at_startup() {
        let row = |table: &'static str| Row {
            table,
            key: Key::two(("id", 1), ("build", 5875)),
            changes: Vec::new(),
            statements: Vec::new(),
        };
        let spells = Plan {
            rows: vec![row(spell::TABLE)],
            ..Plan::default()
        };
        assert!(spells.read_at_startup().is_empty() && !spells.needs_a_restart());
        let mixed = Plan {
            rows: vec![row(skills::TABLE), row(spell::TABLE), row(taxi::TABLE)],
            ..Plan::default()
        };
        assert_eq!(mixed.read_at_startup(), vec![taxi::TABLE, skills::TABLE]);
        assert!(mixed.needs_a_restart());
        assert!(!reloadable(skills::TABLE) && !reloadable(taxi::TABLE));
        assert!(refusal("SkillLineAbility", 70_000).contains("skill line ability 70000"));
        let areas = Plan {
            rows: vec![row(area::TABLE)],
            ..Plan::default()
        };
        assert_eq!(areas.read_at_startup(), vec![area::TABLE]);
        assert!(!reloadable(area::TABLE));
        assert!(refusal("AreaTable", 3487).contains("area 3487"));
    }

    /// The byte comparison finds a record whose bytes changed, and no other.
    #[test]
    fn only_a_record_whose_bytes_changed_is_a_candidate() {
        let shipped = table_of(&[1, 2, 3]);
        let mut edited = table_of(&[1, 2, 3]);
        assert!(changed_entries(&shipped, &edited).is_empty());
        edited.set_u32(1, 5, 99);
        assert_eq!(changed_entries(&shipped, &edited), [2]);
    }

    /// The byte comparison also finds a record the shipped file does not have.
    #[test]
    fn a_new_record_is_a_candidate() {
        let shipped = table_of(&[1, 2]);
        let edited = table_of(&[1, 2, 7]);
        assert_eq!(changed_entries(&shipped, &edited), [7]);
    }

    fn table_of(ids: &[u32]) -> DbcFile {
        const FIELDS: usize = 173;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"WDBC");
        bytes.extend_from_slice(&(ids.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(FIELDS as u32).to_le_bytes());
        bytes.extend_from_slice(&((FIELDS * 4) as u32).to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for id in ids {
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend(std::iter::repeat_n(0u8, (FIELDS - 1) * 4));
        }
        bytes.push(0);
        DbcFile::parse(&bytes).expect("a table this test builds itself")
    }
}
