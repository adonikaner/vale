//! The state and the operations behind the Server panel: which subjects have
//! rows in vmangos' database, what each one's project changes and how much of
//! that is applied, and Save, Apply, Put back and Discard for each. Nothing
//! here draws; [`crate::ui::shell::sync`] is the panel that shows it.
//!
//! ## The four operations
//!
//! ```text
//! Save      write what this project claims into its own folder, as SQL.
//!           Never touches the database. Ctrl+S, and the bar's Save button
//! Apply     run those statements against the database, after writing the
//!           statements that put every row back
//! Put back  run those, and forget them
//! Discard   give up the project's claim on the rows. The database is not
//!           touched — put it back first if it has been applied
//! ```
//!
//! For every subject, Apply can always be reversed and Discard never touches
//! the server.
//!
//! Apply and Put back on a row subject also act on the blocks below it. The
//! twelve row subjects stand in the database in the panel's order, and an item
//! renumber moves loot rows and quest columns, so every applied subject below
//! the one pressed is put back first and applied again after. See
//! `crate::server::stack`. The client-table block is outside that order.
//!
//! ## What an applied row needs to become live, per subject
//!
//! What it takes for an applied row to be live differs per subject, and this
//! panel cannot change it:
//!
//! ```text
//! spell_template    live on .reload spell_template
//! taxi_nodes        the server has to be restarted: it is read at startup and
//!                   has no reload
//! skill_line_ability  the same: read at startup, no reload
//! area_template     the same: read at startup, no reload
//! item_template    live on .reload item_template, for every copy already in
//!                   the world — LoadItemPrototypes clears its map before it
//!                   reads and Item::GetProto is a lookup per call
//! creature_template the server has to be restarted: .reload does not restat
//! creature          creatures that are already spawned, and never erases a
//!                   spawn it has already read
//! quest_template    live on .reload quest_template followed by one reload per
//! and the relations relation table written, removals included. The reload
//!                   frees every Quest, which an escort script in progress
//!                   holds a pointer to
//! *_loot_template   live on one .reload per loot table written, removals
//!                   included; loot already rolled in the world keeps its list
//! npc_vendor        live on .reload npc_vendor and .reload npc_trainer, each
//! npc_trainer       of which re-reads its template table first; removals
//! and templates     included. The client asks for a list each time it opens
//! creature_ai_events live on .reload creature_ai_events, which re-reads
//! creature_spells   creature_ai_scripts first, and .reload creature_spells;
//! *_scripts         five script tables reload under their own names and six
//!                   are read at startup; a creature in the world keeps its
//!                   events and list until it respawns
//! broadcast_text    the server has to be restarted: it is read at startup and
//!                   has no reload
//! ```
//!
//! Each block states its own requirement in the hover text of its name.

use bevy::prelude::Resource;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};

use crate::server::reconcile::Undo;
use crate::server::stack::{self, Subject};
use crate::server::subject::{self, apply::OnTheServer};
use crate::server::{dbcs, queue::ServerQueue, rows, settings::ServerSettings};
use crate::session::EditSession;
use vale_client::assets::GameAssets;

/// The last computed [`Standing`] of each subject, with the time it was
/// computed.
///
/// An open panel is drawn sixty times a second, and one of these answers is
/// expensive: the client-table plan reads the project's whole `Spell.dbc` and
/// diffs it against the archives' copy, which is 22,360 records, a file read
/// and a pass per frame. The row subjects read the project's revert files,
/// which is two small reads a frame for a number that only changes when a
/// button is pressed.
///
/// Each answer is therefore recomputed at most every [`REFRESH`] seconds, and
/// [`Self::forget`] drops them all as soon as anything is applied, put back or
/// discarded. Those presses are the only events after which a stale number
/// could be shown for longer than [`REFRESH`] and be wrong about the block's
/// line or which of its buttons are enabled.
///
/// The subject panels read their own subject's answer from here as well: the
/// item, quest, creature and object workspaces, and the creature and object
/// tools in the inspector. Each draws a line of the same numbers every frame.
#[derive(Resource, Default)]
pub struct Standings {
    /// `(taken at, what it said)` per subject, in [`Half::ALL`]'s order.
    held: [Option<(f64, Standing)>; HALVES],
    /// The number of server tiles changed since the last regeneration. It is
    /// held longer than the others because computing it hashes every tile the
    /// project carries.
    tiles: Option<(f64, usize)>,
    /// The tile count being taken, on a worker. Taken on the main thread it
    /// stopped the window every [`TILES_REFRESH`] seconds for a time that grew
    /// with the number of tiles.
    counting: Option<Task<usize>>,
    /// Where the project's server DBC files stand against the live
    /// `DataDir\5875\dbc\`, or why that could not be read. See
    /// `crate::server::dbcs`.
    files: Option<(f64, Result<dbcs::Standing, String>)>,
}

/// How long a held [`Standing`] is reused, in seconds.
const REFRESH: f64 = 0.5;

/// How long the tile count is reused for. Computing it reads every tile in
/// the project.
const TILES_REFRESH: f64 = 3.0;

impl Standings {
    /// The number of tiles *Regenerate changed tiles* would rebuild, or `None`
    /// until the first count has finished. See `crate::server::datadir::dirty`.
    ///
    /// The count is taken on a worker. A stale count is shown until the next
    /// one arrives.
    pub fn changed_tiles(
        &mut self,
        session: &EditSession,
        assets: &GameAssets,
        now: f64,
    ) -> Option<usize> {
        use crate::server::datadir::{self, Record, TileReader, LIVE_RECORD};
        if let Some(task) = self.counting.as_mut() {
            if let Some(count) = block_on(future::poll_once(task)) {
                self.counting = None;
                self.tiles = Some((now, count));
            }
        }
        let fresh = self
            .tiles
            .is_some_and(|(taken, _)| now - taken < TILES_REFRESH);
        if !fresh && self.counting.is_none() {
            let project = session.project.clone();
            let reader = TileReader::new(&project, assets);
            self.counting = Some(AsyncComputeTaskPool::get().spawn(async move {
                let record = Record::read(&project, LIVE_RECORD);
                datadir::dirty(&reader, &record, &mut |_, _| {}).0.len()
            }));
        }
        self.tiles.map(|(_, count)| count)
    }

    /// The subject's [`Standing`], recomputed if the held answer is stale.
    pub(crate) fn of(
        &mut self,
        half: Half,
        session: &EditSession,
        assets: &GameAssets,
        now: f64,
    ) -> &Standing {
        let at = half.index();
        let fresh = self.held[at]
            .as_ref()
            .is_some_and(|(taken, _)| now - *taken < REFRESH);
        if !fresh {
            self.held[at] = Some((now, standing(half, session, assets)));
        }
        &self.held[at].as_ref().expect("filled above").1
    }

    /// Where the project's server DBC files stand against the live folder,
    /// recomputed if the held answer is stale. It reads each carried file and
    /// its live copy, so it is held as long as the tile count.
    pub fn files(
        &mut self,
        session: &EditSession,
        server: &ServerSettings,
        now: f64,
    ) -> &Result<dbcs::Standing, String> {
        let fresh = self
            .files
            .as_ref()
            .is_some_and(|(taken, _)| now - *taken < TILES_REFRESH);
        if !fresh {
            let answer = crate::server::datadir::data_dir(server)
                .map(|dir| dbcs::standing(&session.project, &vale_mangos::datadir::dbc_dir(&dir)));
            self.files = Some((now, answer));
        }
        &self.files.as_ref().expect("filled above").1
    }

    /// Drops every held answer. Called by code that has just changed one of
    /// them.
    ///
    /// The tile count is marked stale rather than dropped, so the button shows
    /// the last count until the next one arrives. A count still running was
    /// started before the change and is dropped, which cancels it.
    pub fn forget(&mut self) {
        self.held = Default::default();
        self.tiles = self.tiles.map(|(_, count)| (f64::NEG_INFINITY, count));
        self.counting = None;
        self.files = None;
    }
}

/// Everything the panel reads and writes, as one argument.
pub struct Work<'a> {
    pub session: &'a mut EditSession,
    /// The archives, for the one subject whose plan is a diff of two files
    /// rather than a store of typed values. See [`rows`].
    pub assets: &'a GameAssets,
    pub server: &'a mut ServerSettings,
    /// The queue an Apply or a Put back is pushed onto. See
    /// [`crate::server::queue`]. Each write requests its own `.reload` when it
    /// completes; the request is deferred while no playtest is running.
    pub queue: &'a mut ServerQueue,
    /// The cached standing of each subject. See [`Standings`]; the cache stops
    /// this panel reading a 22,360-row table sixty times a second.
    pub standings: &'a mut Standings,
    /// The app's clock, which the [`Standings`] cache is aged against.
    pub now: f64,
}

/// One subject whose edits are rows in the server's database, and one block
/// of the panel.
///
/// The row subjects are listed in [`Subject::ORDER`], the order they stand in
/// the database; the test
/// `the_panel_lists_the_row_subjects_in_the_stacks_order` checks it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Half {
    /// The rows the server keeps of the client tables, `Spell.dbc` and
    /// `TaxiNodes.dbc` among them, written from the project's copies of those
    /// files.
    Tables,
    /// One of the row subjects, whose edits are a store of typed values; see
    /// [`crate::server::subject`].
    Row(Subject),
}

/// The number of subjects. [`Standings`] holds one answer for each.
pub const HALVES: usize = 1 + Subject::COUNT;

impl Half {
    /// The client-table block, then every row subject in the stack's order.
    pub const ALL: [Half; HALVES] = {
        let mut out = [Half::Tables; HALVES];
        let mut at = 0;
        while at < Subject::COUNT {
            out[at + 1] = Half::Row(Subject::ORDER[at]);
            at += 1;
        }
        out
    };

    /// Its place in [`Self::ALL`], which indexes [`Standings`].
    pub fn index(self) -> usize {
        match self {
            Half::Tables => 0,
            Half::Row(subject) => 1 + subject.index(),
        }
    }

    /// The subject's name, in the rail's wording.
    pub fn name(self) -> &'static str {
        match self {
            Half::Tables => "Client tables",
            Half::Row(subject) => subject.row_subject().label(),
        }
    }

    /// The database tables the subject writes. The name is the editor's word;
    /// the table names are what the database shows.
    pub fn tables(self) -> &'static str {
        match self {
            Half::Tables => "spell_template, taxi_nodes, skill_line_ability, area_template, areatrigger_template",
            Half::Row(subject) => subject.row_subject().table_list(),
        }
    }

    /// Where a save writes the statements, as a virtual path in the project.
    pub fn file(self) -> &'static str {
        match self {
            Half::Tables => rows::VPATH,
            Half::Row(subject) => subject.row_subject().sql_vpath(),
        }
    }

    /// Where the statements that put the rows back are kept, as a virtual
    /// path in the project.
    pub fn revert_file(self) -> &'static str {
        match self {
            Half::Tables => rows::REVERT_VPATH,
            Half::Row(subject) => subject.row_subject().revert_vpath(),
        }
    }

    /// What it takes for an applied row to become live. The module comment
    /// lists how this differs between subjects.
    pub fn going_live(self) -> &'static str {
        match self {
            Half::Tables => {
                "A spell is live on `.reload spell_template`, which an apply sends when it \
                 is made with the panels open over a playtest; applied at any other time, \
                 it is live after a restart. A flight node, a skill line ability and an area \
                 need a restart: `taxi_nodes`, `skill_line_ability` and `area_template` \
                 are read at startup and have no reload. So do the DBC files Apply copies into DataDir\\5875\\dbc: \
                 TaxiPath, TaxiPathNode, SkillLine, SkillRaceClassInfo and ItemSet among \
                 them. A removed spell \
                 also needs a restart: the reload overwrites and \
                 adds rows, and never drops a row it has already read."
            }
            Half::Row(subject) => subject.row_subject().going_live(),
        }
    }

    /// The sentence added to the Apply and Put back tooltips saying what those
    /// operations also do to the blocks below this one. See
    /// `crate::server::stack`. Empty for the client-table block and for the
    /// last subject in the stack, which has nothing below it.
    pub fn order_note(self) -> &'static str {
        match self.subject() {
            None => "",
            Some(subject) if subject.index() + 1 == Subject::COUNT => "",
            Some(_) => {
                " Applied subjects below this one are restored first and applied again \
                 after, because this one's statements can move their rows."
            }
        }
    }

    /// The row subject this block is. `None` for the client-table block,
    /// which is not in the stack.
    pub fn subject(self) -> Option<Subject> {
        match self {
            Half::Tables => None,
            Half::Row(subject) => Some(subject),
        }
    }

    /// Whether this subject's claim is a store of rows, which Discard can give
    /// up.
    ///
    /// False for the client tables, whose claim is the project's own copies of
    /// `Spell.dbc` and `TaxiNodes.dbc`. The statements are a diff of those
    /// files against the archives', so giving up the claim means editing the
    /// table back. For spells that is the spell workspace's own Undo and
    /// Discard, which are not server operations.
    pub fn discardable(self) -> bool {
        !matches!(self, Half::Tables)
    }

    /// Whether a table read out of the project's store belongs to this
    /// subject: [`Subject::owns`], which the writers use. The client-table
    /// block owns none, because its edits are files.
    pub fn owns(self, table: &str) -> bool {
        self.subject().is_some_and(|subject| subject.owns(table))
    }
}

/// The state of one subject: what the project changes, and how much of that
/// the database already holds.
#[derive(Default, Clone)]
pub struct Standing {
    /// Rows (and, for the creatures, paths) this project changes.
    pub changed: usize,
    /// How many of the changed rows have not been applied to the database.
    pub outstanding: usize,
    /// How many rows this project has applied and can take back. This is the
    /// revert file's own count.
    pub applied: usize,
    /// Whether what the project says now is what was applied. It only adds a
    /// caveat to [`Standing::line`]. See `subject::apply::OnTheServer::current`.
    pub current: bool,
    /// What the writer refused to write, each as a sentence.
    pub refused: Vec<String>,
    /// The error that stopped the plan being built. Only the client-table
    /// block can set it, because only its plan reads files.
    pub trouble: Option<String>,
    /// Whether a table is edited and not yet written to the project folder.
    /// Only the client-table block can set it: its plan is a diff of the
    /// project's own table files against the archives', so a table edited in
    /// this session and not saved is in no plan, and the numbers above are
    /// from the last save. Apply writes the tables first; this flag tells the
    /// person so before Apply is pressed.
    pub unsaved: bool,
    /// The plan's own line of what it creates, edits and removes, as
    /// `subject::Plan::line` words it. Empty for the client tables. The
    /// creature and object tool panels print it.
    pub summary: String,
    /// The rows the revert file holds, by table and key: the rows this project
    /// has applied. Empty for the client tables. The creature and object tool
    /// panels say of the selected spawn whether it is applied. Shared, so a
    /// panel's copy of a [`Standing`] does not copy the list.
    pub written: std::sync::Arc<Vec<(String, vale_mangos::row::Key)>>,
}

impl Standing {
    /// Whether this project has applied that row. Answers only for the
    /// subjects that fill [`Self::written`].
    pub fn covers(&self, table: &str, key: &vale_mangos::row::Key) -> bool {
        self.written
            .iter()
            .any(|(had_table, had_key)| had_table == table && had_key == key)
    }

    /// Whether there is nothing to say about this subject: no change, no
    /// applied row, no unsaved table, no refusal and no error.
    pub fn quiet(&self) -> bool {
        self.changed == 0
            && self.applied == 0
            && !self.unsaved
            && self.refused.is_empty()
            && self.trouble.is_none()
    }

    /// Whether Apply would do anything.
    ///
    /// Rows that were applied and are no longer claimed count. An apply makes
    /// the database what it was plus what the project says now, so a project
    /// that changes nothing but has applied rows in the database has those
    /// rows to take back. See `crate::server::reconcile`.
    pub fn appliable(&self) -> bool {
        self.changed > 0 || self.unsaved || self.applied > 0
    }

    /// One line stating what the project changes and how much of it has been
    /// applied.
    pub fn line(&self) -> String {
        match (self.changed, self.applied) {
            (0, 0) => "nothing changed".to_string(),
            (changed, 0) => format!("{changed} row(s) changed, none applied"),
            (0, applied) => format!(
                "{applied} row(s) applied, none of them changed by this project any more; \
                 Restore reverts them"
            ),
            (changed, applied) => {
                let caveat = match (self.outstanding, self.current) {
                    (0, true) => " \u{2014} all applied".to_string(),
                    (0, false) => " \u{2014} all applied; the project may have changed since \
                                   the last Apply"
                        .to_string(),
                    (out, _) => format!(" \u{2014} {out} not applied yet"),
                };
                format!("{changed} row(s) changed, {applied} applied{caveat}")
            }
        }
    }
}

/// Computes one subject's [`Standing`] from the project and the revert file.
///
/// Reads no database. Every number comes from the project folder, so the
/// panel can be drawn with no server to reach. See `OnTheServer`, whose
/// record is the revert file.
pub fn standing(half: Half, session: &EditSession, assets: &GameAssets) -> Standing {
    match half {
        Half::Tables => {
            let plan = match rows::plan(session, assets) {
                Ok(plan) => plan,
                Err(e) => {
                    return Standing {
                        trouble: Some(e),
                        ..Standing::default()
                    };
                }
            };
            let undo = Undo::open_at(&session.project, rows::REVERT_VPATH).unwrap_or(Undo {
                entries: Vec::new(),
            });
            Standing {
                changed: plan.rows.len(),
                outstanding: plan
                    .rows
                    .iter()
                    .filter(|row| !undo.covers(row.table, &row.key))
                    .count(),
                applied: undo.entries.len(),
                current: session.applied_signature == Some(plan.signature()),
                unsaved: rows::MAPPED
                    .iter()
                    .any(|(table, _)| session.unsaved_tables.contains(*table)),
                refused: plan
                    .refused
                    .iter()
                    .map(|&(dbc, id)| rows::refusal(dbc, id))
                    .collect(),
                trouble: None,
                ..Standing::default()
            }
        }
        Half::Row(subject) => {
            let plan = subject::plan(session, subject.row_subject());
            let on_server = OnTheServer::read_with(session, &plan);
            let rows = plan
                .rows
                .iter()
                .filter(|row| !on_server.covers(row.table.name, &row.key))
                .count();
            let wholes = plan
                .wholes
                .iter()
                .filter(|whole| !on_server.covers(whole.table(), &whole.key()))
                .count();
            Standing {
                changed: plan.count(),
                outstanding: rows + wholes,
                applied: on_server.rows(),
                current: on_server.current(),
                refused: plan.refused.clone(),
                trouble: None,
                unsaved: false,
                summary: plan.line(),
                written: std::sync::Arc::new(on_server.written()),
            }
        }
    }
}

/// Writes this subject's statements into the project folder, and does
/// nothing else.
///
/// Ctrl+S does this for every subject through `crate::server::save`, the one
/// caller a person triggers. Apply also calls it, so that Apply writes before
/// it runs. Without that, Apply would run what the last save held rather than
/// what the project says now.
pub fn save(half: Half, work: &mut Work<'_>) {
    match half {
        // Writes the client tables into the project folder. The client-table
        // plan is a diff of the project's own table files (`Spell.dbc`,
        // `TaxiNodes.dbc`) against the archives', so a table edited and not
        // yet saved is not in it. An Apply that did not write first would run
        // the statements for the last save and report them as what is on
        // screen. The SQL file itself is written by `rows::save`, which the
        // apply below calls in its own order.
        Half::Tables => {
            work.session.save_all_tables();
        }
        // The stores first, which every row subject shares, then this
        // subject's SQL.
        Half::Row(subject) => {
            if subject::save_stores(work.session) {
                subject::save(work.session, subject.row_subject());
            }
        }
    }
}

/// Queues this subject's statements to run against the database, after
/// writing the statements that put every row back.
///
/// The file is written here, when the button is pressed. The database part is
/// a write on [`crate::server::queue`], which reports what it did on the
/// status line when it completes. Returns the status-bar line for now: that
/// the write is queued, or why there is nothing to queue.
pub fn apply(half: Half, work: &mut Work<'_>) -> Result<String, String> {
    save(half, work);
    let work_for = match half {
        Half::Tables => {
            // The files the server reads from DataDir\5875\dbc are copied here, on
            // the press: a few hundred kilobytes, small enough to copy on the main thread.
            let files = sync_files(work);
            let plan = rows::plan(work.session, work.assets)?;
            if plan.nothing() {
                if let Some(files) = files {
                    let rows = put_back(Half::Tables, work)?;
                    return Ok(match rows.as_str() {
                        "this project has applied nothing" => files,
                        _ => format!("{files}; {rows}"),
                    });
                }
                // The project no longer changes any client-table row. What it
                // applied before is still in the database, and an apply of
                // nothing takes that back.
                return put_back(Half::Tables, work).map(|line| match line.as_str() {
                    "this project has applied nothing" => "nothing to apply".to_string(),
                    _ => line,
                });
            }
            // The SQL file is written first, as the other subjects' saves do.
            // The durable result of an apply is the reviewable SQL in the
            // project folder; the database change can be put back.
            rows::write_file(work.session, &plan)?;
            let label = match files {
                Some(files) => format!("{files}; applying client table rows"),
                None => "applying client table rows".to_string(),
            };
            Some((label, rows::apply_work(work.session, plan, work.server)?))
        }
        // The twelve row subjects go through the stack, which puts back and
        // applies again every applied subject after this one. See
        // `crate::server::stack`.
        _ => match half.subject() {
            Some(subject) => stack::work(stack::Wanted::Apply(subject), work.session, work.server)?,
            None => None,
        },
    };
    match work_for {
        Some((label, write)) => {
            work.queue.push(label.clone(), write);
            Ok(format!("{label}\u{2026}"))
        }
        None => Ok("nothing to apply".to_string()),
    }
}

/// Queues a write that returns every row this project has applied to what it
/// held before, and then forgets the undo.
pub fn put_back(half: Half, work: &mut Work<'_>) -> Result<String, String> {
    let work_for = match (half, half.subject()) {
        (Half::Tables, _) => {
            let files = put_back_files(work);
            let rows = rows::revert_work(work.session, work.server)?;
            match (files, rows) {
                (Some(files), None) => return Ok(files),
                (files, Some(write)) => {
                    let label = match files {
                        Some(files) => format!("{files}; restoring client table rows"),
                        None => "restoring client table rows".to_string(),
                    };
                    Some((label, write))
                }
                (None, None) => None,
            }
        }
        // Row subjects go through the stack, for the same reason as in Apply.
        (_, Some(subject)) => {
            if !subject.applied(work.session) {
                return Ok("this project has applied nothing".to_string());
            }
            stack::work(stack::Wanted::PutBack(subject), work.session, work.server)?
        }
        (_, None) => None,
    };
    match work_for {
        Some((label, write)) => {
            work.queue.push(label.clone(), write);
            Ok(format!("{label}\u{2026}"))
        }
        None => Ok("this project has applied nothing".to_string()),
    }
}

/// Copy the project's server DBC files into the live `DataDir\5875\dbc\` for the
/// client-table block's Apply. `None` when there is nothing to copy or put
/// back; otherwise the status line, which says so when the folder could not
/// be reached.
fn sync_files(work: &mut Work<'_>) -> Option<String> {
    let project = &work.session.project;
    if dbcs::carried(project).is_empty() && dbcs::saved(project).is_empty() {
        return None;
    }
    Some(
        match crate::server::datadir::data_dir(work.server)
            .and_then(|dir| dbcs::apply_at(project, &vale_mangos::datadir::dbc_dir(&dir)))
        {
            Ok(done) => done.line(),
            Err(e) => format!("server DBC files not copied: {e}"),
        },
    )
}

/// Put the server's own DBC files back, for the client-table block's Put back.
/// `None` when this project has copied none.
fn put_back_files(work: &mut Work<'_>) -> Option<String> {
    let project = &work.session.project;
    if dbcs::saved(project).is_empty() {
        return None;
    }
    Some(
        match crate::server::datadir::data_dir(work.server)
            .and_then(|dir| dbcs::put_back_at(project, &vale_mangos::datadir::dbc_dir(&dir)))
        {
            Ok(done) => done.line(),
            Err(e) => format!("server DBC files not restored: {e}"),
        },
    )
}

/// Gives up this project's claim on the subject's rows. The database is not
/// touched.
pub fn discard(half: Half, session: &mut EditSession) -> String {
    let (rows, paths) =
        session.forget_server_edits(|table| half.owns(table), half == Half::Row(Subject::Creatures));
    match (rows, paths) {
        (0, 0) => "nothing to discard".to_string(),
        (rows, 0) => format!("{rows} row edit(s) discarded"),
        (0, paths) => format!("{paths} waypoint path(s) discarded"),
        (rows, paths) => format!("{rows} row edit(s) and {paths} waypoint path(s) discarded"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every subject writes its own two files, so one Apply applies one
    /// subject. If two subjects shared an SQL file, applying one would also
    /// apply the other's rows.
    #[test]
    fn no_two_subjects_share_a_file() {
        let mut seen: Vec<&str> = Vec::new();
        for half in Half::ALL {
            for path in [half.file(), half.revert_file()] {
                assert!(
                    !seen.contains(&path),
                    "{} names {path}, which another subject already writes",
                    half.name()
                );
                seen.push(path);
            }
        }
    }

    /// Every table the store can hold belongs to exactly one subject, so a
    /// discard gives up what it says it gives up and no row is left without an
    /// owner. The client-table block owns none, because its edits are files.
    #[test]
    fn each_table_belongs_to_one_subject() {
        let tables: Vec<&str> = vale_mangos::creature::TABLES
            .iter()
            .chain(vale_mangos::gameobject::TABLES)
            .chain(vale_mangos::item::TABLES)
            .chain(vale_mangos::quest::TABLES)
            .chain(vale_mangos::loot::TABLES)
            .chain(crate::server::services::TABLES)
            .chain(crate::server::places::TABLES)
            .chain(crate::server::behaviour::ROW_TABLES)
            .map(|table| table.name)
            .chain(vale_mangos::scripts::TABLES)
            .collect();
        for table in tables {
            let owners: Vec<Half> = Half::ALL
                .into_iter()
                .filter(|half| half.owns(table))
                .collect();
            assert_eq!(owners.len(), 1, "{table} is owned by {owners:?}");
        }
        assert!(
            Half::ALL
                .into_iter()
                .all(|half| !half.owns("spell_template")),
            "a spell edit is a file, and the store never holds one"
        );
    }

    /// The panel lists the row subjects in the stack's order. The Apply and
    /// Put back tooltips refer to "subjects below this one", which is only
    /// correct in that order.
    #[test]
    fn the_panel_lists_the_row_subjects_in_the_stacks_order() {
        let listed: Vec<Subject> = Half::ALL.iter().filter_map(|half| half.subject()).collect();
        assert_eq!(listed, Subject::ORDER.to_vec());
    }

    /// A subject indexes the cache by [`Half::index`], so it and the order of
    /// [`Half::ALL`] must match. Nothing else checks them against each other,
    /// and a mismatch would show the creature numbers under the item heading
    /// rather than failing.
    #[test]
    fn each_subject_indexes_its_own_slot() {
        assert_eq!(Half::ALL.len(), HALVES);
        for (at, half) in Half::ALL.into_iter().enumerate() {
            assert_eq!(half.index(), at, "{half:?} indexes {}", half.index());
        }
    }

    /// The line distinguishes rows that are all applied from rows still
    /// outstanding. The panel this one replaced read the same in both states.
    #[test]
    fn the_line_tells_applied_from_outstanding() {
        let all_in = Standing {
            changed: 3,
            outstanding: 0,
            applied: 3,
            current: true,
            ..Standing::default()
        };
        assert!(all_in.line().contains("all applied"), "{}", all_in.line());

        let owed = Standing {
            changed: 3,
            outstanding: 2,
            applied: 1,
            current: false,
            ..Standing::default()
        };
        assert!(owed.line().contains("2 not applied"), "{}", owed.line());

        let untouched = Standing {
            changed: 4,
            ..Standing::default()
        };
        assert!(
            untouched.line().contains("none applied"),
            "{}",
            untouched.line()
        );

        // A project that changes nothing in the subject is quiet. The item and
        // quest panels check `quiet` before they compose a line, and draw no
        // block when it holds.
        assert!(Standing::default().quiet());
        assert!(!untouched.quiet());
    }
}
