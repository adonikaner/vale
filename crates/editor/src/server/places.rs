//! What a project changes about conditions, gossip, maps, area triggers and
//! graveyards on the server, as SQL, applied when a person asks.
//!
//! Five row subjects share this module, one per [`Group`]: each has its own
//! block on the Server panel, its own SQL and revert files, its own reloads and
//! its own place in `super::stack`'s order. They share the row rules, which are
//! the same for all twelve tables.
//!
//! ## Twelve tables, written as keyed rows
//!
//! ```text
//! conditions                    the server's reusable yes-or-no tests
//! npc_text                      what a gossip text says: up to eight lines
//! gossip_menu                   one text of a gossip menu
//! gossip_menu_option            one option of a gossip menu
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
//! sql\<group>.sql         what this project does to each group's tables:
//!                         conditions, gossip, maps, triggers, graveyards
//! sql\<group>-revert.sql  what puts those rows back
//! server\rows.txt         the store all of them are written from, shared
//! ```

use crate::session::EditSession;
use vale_mangos::conn::Db;
use vale_mangos::row::{self, Assignment, Key, Life};
use vale_mangos::{condition, gossip, graveyard, map, trigger};
use bevy::prelude::*;

pub use super::creatures::Undo;

/// One of the five subjects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// The `conditions` table.
    Conditions,
    /// Gossip menus, their options and the `npc_text` rows they show.
    Gossip,
    /// A new map's `map_template` row.
    Maps,
    /// What area triggers do: the template's server columns, teleports, inns,
    /// quest objectives and battleground entrances.
    Triggers,
    /// Which safe place serves which zone, and which way a spirit faces there.
    Graveyards,
}

impl Group {
    /// The five, in `super::stack`'s order. Conditions first, since every other
    /// group names them; maps before triggers, since a teleport's target map is
    /// checked against `map_template`.
    pub const ALL: [Group; 5] = [Group::Conditions, Group::Gossip, Group::Maps, Group::Triggers, Group::Graveyards];

    /// Where [`EditSession::applied_places`] keeps this group's signature.
    pub fn index(self) -> usize {
        match self {
            Group::Conditions => 0,
            Group::Gossip => 1,
            Group::Maps => 2,
            Group::Triggers => 3,
            Group::Graveyards => 4,
        }
    }

    /// The group a table belongs to, or `None` for a table of no group.
    pub fn of(table: &str) -> Option<Group> {
        let table = table_named(table)?;
        Group::ALL.into_iter().find(|group| group.tables().contains(&table))
    }

    /// Its tables, in the order a plan writes them.
    pub fn tables(self) -> &'static [&'static str] {
        match self {
            Group::Conditions => &condition::TABLES,
            Group::Gossip => &gossip::TABLES,
            Group::Maps => &map::TABLES,
            Group::Triggers => &trigger::TABLES,
            Group::Graveyards => &graveyard::TABLES,
        }
    }

    /// The reload commands that make an apply live, in the order they are sent.
    pub fn reloads(self) -> &'static [&'static str] {
        match self {
            Group::Conditions => &condition::TABLES,
            Group::Gossip => &gossip::TABLES,
            Group::Maps => &[map::TEMPLATE],
            Group::Triggers => &trigger::RELOADS,
            Group::Graveyards => &[graveyard::ZONE],
        }
    }

    /// What this project does to the group's tables, as SQL.
    pub fn sql_vpath(self) -> &'static str {
        match self {
            Group::Conditions => "sql\\conditions.sql",
            Group::Gossip => "sql\\gossip.sql",
            Group::Maps => "sql\\maps.sql",
            Group::Triggers => "sql\\triggers.sql",
            Group::Graveyards => "sql\\graveyards.sql",
        }
    }

    /// What puts it back.
    pub fn revert_vpath(self) -> &'static str {
        match self {
            Group::Conditions => "sql\\conditions-revert.sql",
            Group::Gossip => "sql\\gossip-revert.sql",
            Group::Maps => "sql\\maps-revert.sql",
            Group::Triggers => "sql\\triggers-revert.sql",
            Group::Graveyards => "sql\\graveyards-revert.sql",
        }
    }

    /// What its rows are called in a status line.
    pub fn noun(self) -> &'static str {
        match self {
            Group::Conditions => "condition",
            Group::Gossip => "gossip",
            Group::Maps => "map",
            Group::Triggers => "area trigger",
            Group::Graveyards => "graveyard",
        }
    }

    /// …and the subject, plural, for a file's header and an error.
    pub fn subject(self) -> &'static str {
        match self {
            Group::Conditions => "conditions",
            Group::Gossip => "gossip menus",
            Group::Maps => "maps",
            Group::Triggers => "area triggers",
            Group::Graveyards => "graveyards",
        }
    }
}

/// The twelve tables, by group. [`Group::tables`] names each group's.
pub const TABLES: [&str; 12] = [
    condition::TABLE,
    gossip::NPC_TEXT,
    gossip::MENU,
    gossip::OPTION,
    map::TEMPLATE,
    trigger::TEMPLATE,
    trigger::TELEPORT,
    trigger::TAVERN,
    trigger::QUEST,
    trigger::BG_ENTRANCE,
    graveyard::ZONE,
    graveyard::FACING,
];

/// The static name of one of the twelve tables, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// Whether a table read out of the project's store is this subject's.
pub fn owns(table: &str) -> bool {
    table_named(table).is_some()
}

/// Every column of one of the twelve tables.
pub fn columns_of(table: &str) -> &'static [vale_mangos::schema::Column] {
    match Group::of(table) {
        Some(Group::Conditions) => condition::columns_of(table),
        Some(Group::Gossip) => gossip::columns_of(table),
        Some(Group::Maps) => map::columns_of(table),
        Some(Group::Triggers) => trigger::columns_of(table),
        Some(Group::Graveyards) => graveyard::columns_of(table),
        None => &[],
    }
}

/// One column of one of the twelve tables.
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

/// Everything a project changes about one group's tables.
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

/// What the project's store comes to for one group, as statements.
pub fn plan(session: &EditSession, group: Group) -> Plan {
    plan_from(&session.server_edits, group)
}

/// The same over the store alone, so it can be checked with no session.
pub fn plan_from(edits: &row::Edits, group: Group) -> Plan {
    let mut out = Plan::default();
    for (table, key, row) in edits.rows() {
        // Another subject's row, skipped as every writer skips the others'.
        let Some(table) = table_named(table).filter(|table| Group::of(table) == Some(group)) else {
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
    out.extend(condition::check_created(table, &row));
    out.extend(gossip::check_created(table, &row));
    out
}

/// Write the group's SQL file, or remove it when the project changes nothing
/// in the group.
pub fn write_sql(session: &mut EditSession, group: Group) -> Result<usize, String> {
    let plan = plan(session, group);
    let vpath = group.sql_vpath();
    for refused in &plan.refused {
        warn!("{}: {refused}", super::creatures::EDITS_VPATH);
    }
    if plan.is_empty() {
        remove(&session.project, vpath);
        return Ok(0);
    }
    let count = plan.rows.len();
    let reloads: String = group.reloads().iter().map(|table| format!("--   .reload {table}\n")).collect();
    let mut body = format!(
        "-- {} — what this project changes about {}.\n\
         -- Rewritten on every save from the project's own edits, so it is the\n\
         -- whole of what the project does rather than an increment of it.\n\
         --\n\
         -- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then reload:\n\
         {reloads}\
         -- A table with no reload is read at server start.\n\n",
        session.project.name,
        group.subject()
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
        .write(vpath, body.as_bytes())
        .map_err(|e| format!("{vpath}: {e}"))?;
    Ok(count)
}

/// The three groups' half of a save: the SQL the store comes to. The store
/// itself is written by [`super::creatures::save`], on the same save.
pub fn save(session: &mut EditSession) {
    for group in Group::ALL {
        save_group(session, group);
    }
}

/// …one group of it.
pub fn save_group(session: &mut EditSession, group: Group) {
    match write_sql(session, group) {
        Ok(0) => {}
        Ok(rows) => {
            session.status = format!("{rows} {} change(s) written to {}", group.noun(), group.sql_vpath())
        }
        Err(e) => {
            warn!("{}: {e}", group.subject());
            session.status = format!("saved, but {} did not: {e}", group.sql_vpath());
        }
    }
}

/// What an apply did.
#[derive(Debug, Default)]
pub struct Applied {
    pub group: Option<Group>,
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
            "{} {} row(s) applied, {} affected — reloading {}",
            self.rows,
            self.group.map_or("", Group::noun),
            self.affected,
            self.group.map_or(String::new(), |group| group.reloads().join(", "))
        )
    }
}

/// An Apply, with everything it needs to run off the main thread.
pub struct ApplyJob {
    group: Group,
    plan: Plan,
    project: vale_edit::project::Project,
    at: vale_mangos::conn::Where,
}

/// What running one answered.
pub struct ApplyDone {
    group: Group,
    signature: u64,
    pub result: Result<Applied, String>,
}

impl ApplyDone {
    /// What the status line says about it.
    pub fn line(&self) -> String {
        match &self.result {
            Ok(done) if done.rows == 0 && done.taken_back == 0 => "nothing to apply".to_string(),
            Ok(done) => done.line(),
            Err(e) => format!("{}: {e}", self.group.subject()),
        }
    }
}

/// The main thread's first half of an Apply. `None` when the project claims no
/// row of the group's tables and has applied none.
pub fn prepare_apply(
    session: &EditSession,
    server: &super::settings::ServerSettings,
    group: Group,
) -> Result<Option<ApplyJob>, String> {
    let plan = plan(session, group);
    if plan.is_empty() && !super::reconcile::has_applied(session, group.revert_vpath()) {
        return Ok(None);
    }
    let (at, _source) = server.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
    Ok(Some(ApplyJob {
        group,
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
                self.group.revert_vpath(),
                &format!("the server's {}", self.group.subject()),
                &steps,
                |db, index| undo_of_a_row(db, rows[index]),
            )?;
            Ok(Applied {
                group: Some(self.group),
                rows: self.plan.rows.len(),
                affected: done.affected,
                newly_undoable: done.undoable,
                taken_back: done.taken_back,
            })
        })();
        ApplyDone {
            group: self.group,
            signature,
            result,
        }
    }
}

/// The main thread's second half: every read is stale, and the group's
/// reloads are asked for.
pub fn finish_apply(session: &mut EditSession, reloads: &mut super::reload::Reloads, done: &ApplyDone) {
    session.wrote_the_database();
    session.applied_places[done.group.index()] = done.result.as_ref().ok().map(|_| done.signature);
    for table in done.group.reloads() {
        reloads.when_there_is_a_session(table);
    }
}

/// An Apply as one step of [`super::stack`]. `None` when there is nothing to
/// apply.
pub fn apply_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
    group: Group,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_apply(session, server, group)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new(format!("applying {}", group.subject()), move || {
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
    group: Group,
    project: vale_edit::project::Project,
    at: vale_mangos::conn::Where,
}

/// The main thread's first half of a Put back: `None` when this project has
/// applied nothing.
pub fn prepare_revert(
    session: &EditSession,
    server: &super::settings::ServerSettings,
    group: Group,
) -> Result<Option<RevertJob>, String> {
    if !super::reconcile::has_applied(session, group.revert_vpath()) {
        return Ok(None);
    }
    let (at, _source) = server.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
    Ok(Some(RevertJob {
        group,
        project: session.project.clone(),
        at,
    }))
}

impl RevertJob {
    /// The worker's half: run the revert file, and forget it.
    pub fn run(self) -> Result<usize, String> {
        let mut db = Db::open(&self.at)?;
        super::reconcile::put_back(&self.project, &mut db, self.group.revert_vpath())
    }
}

/// The main thread's second half: the group's reloads, since the revert file
/// does not say which tables it touches.
pub fn finish_revert(session: &mut EditSession, reloads: &mut super::reload::Reloads, group: Group) {
    session.applied_places[group.index()] = None;
    session.wrote_the_database();
    for table in group.reloads() {
        reloads.when_there_is_a_session(table);
    }
}

/// A Put back as one step: `None` when this project has applied nothing.
pub fn revert_step(
    session: &EditSession,
    server: &super::settings::ServerSettings,
    group: Group,
) -> Result<Option<super::stack::Step>, String> {
    let Some(job) = prepare_revert(session, server, group)? else {
        return Ok(None);
    };
    Ok(Some(super::stack::Step::new(format!("putting back {}", group.subject()), move || {
        let done = job.run();
        let ok = done.is_ok();
        let finish: super::queue::Finish = Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            session.status = match done {
                Ok(rows) => {
                    finish_revert(session, reloads, group);
                    format!("{rows} {} row(s) put back", group.noun())
                }
                Err(e) => format!("{}: {e}", group.subject()),
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
    pub fn read_with(session: &EditSession, plan: &Plan, group: Group) -> OnTheServer {
        OnTheServer {
            undo: Undo::open_at(&session.project, group.revert_vpath()).unwrap_or(Undo { entries: Vec::new() }),
            current: session.applied_places[group.index()].is_some_and(|had| had == plan.signature()),
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

    /// A new map, a teleport into it and a graveyard link each go to their
    /// own group's plan.
    #[test]
    fn a_map_its_entrance_and_its_graveyard_are_three_subjects() {
        let mut edits = Edits::default();
        let link = graveyard::Link::new(900, 5000);
        created(graveyard::ZONE, &link.key(), link.assignments(), &mut edits);
        let teleport = trigger::Teleport::new(4500, 534, [10.0, 20.0, 30.0]);
        created(trigger::TELEPORT, &teleport.key(), teleport.assignments(), &mut edits);
        let template = map::Template::new(534, 1, "The Islands");
        created(map::TEMPLATE, &template.key(), template.assignments(), &mut edits);
        for (group, table) in [
            (Group::Maps, "map_template"),
            (Group::Triggers, "areatrigger_teleport"),
            (Group::Graveyards, "game_graveyard_zone"),
        ] {
            let plan = plan_from(&edits, group);
            assert!(plan.refused.is_empty(), "{:?}", plan.refused);
            let sql = plan.statements();
            assert_eq!(sql.len(), 2, "{group:?}");
            assert!(sql[1].starts_with(&format!("INSERT INTO `{table}`")), "{}", sql[1]);
        }
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
        for group in [Group::Triggers, Group::Graveyards] {
            let plan = plan_from(&edits, group);
            assert!(plan.rows.is_empty());
            assert_eq!(plan.refused.len(), 1, "{:?}", plan.refused);
        }
    }

    #[test]
    fn the_subjects_own_the_twelve_tables_and_no_other() {
        for table in TABLES {
            assert!(owns(table));
            assert!(!columns_of(table).is_empty(), "{table}");
            let group = Group::of(table).unwrap();
            assert!(group.tables().contains(&table), "{table} is not in {group:?}'s slice");
        }
        let count: usize = Group::ALL.iter().map(|group| group.tables().len()).sum();
        assert_eq!(count, TABLES.len());
        assert!(!owns(vale_mangos::creature::TEMPLATE));
    }

    /// A template edit sets the server columns on the dev row, which it makes
    /// first; a created or removed template row is refused.
    #[test]
    fn a_template_row_is_edited_on_its_dev_row_and_never_created() {
        let mut edits = Edits::default();
        let key = trigger::key(78);
        edits.set(trigger::TEMPLATE, &key, "cooldown", Some("30".to_string()));
        let plan = plan_from(&edits, Group::Triggers);
        assert_eq!(plan.rows.len(), 1);
        let statements = plan.rows[0].statements();
        assert_eq!(statements.len(), 2);
        assert!(statements[0].starts_with("INSERT IGNORE INTO `areatrigger_template`"));
        assert!(statements[1].contains("`cooldown` = 30"), "{}", statements[1]);
        assert!(statements[1].contains("`build` = 5875"), "{}", statements[1]);

        let mut edits = Edits::default();
        let template = trigger::Template { id: 9000, ..trigger::Template::default() };
        created(trigger::TEMPLATE, &trigger::key(9000), template.assignments(), &mut edits);
        let plan = plan_from(&edits, Group::Triggers);
        assert!(plan.rows.is_empty());
        assert_eq!(plan.refused.len(), 1);
    }
}
