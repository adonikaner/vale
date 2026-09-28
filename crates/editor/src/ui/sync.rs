//! The Server panel: every server operation for this project. It shows what
//! the project changes, how much of that is in the database, and the four
//! operations that act on it.
//!
//! ## Why all subjects share one panel
//!
//! Several subjects store their edits as rows in vmangos' database rather than
//! as bytes in a file: the client tables (spells and flight nodes), creatures,
//! objects, items, quests, loot, vendors and trainers, and behaviour. Each
//! subject used to keep these operations in its own panel. The result was three names for one operation
//! (*Apply to the server*, *Apply*, and a checkbox in a third window for
//! spells), three names for its inverse (*Put the rows back*, *Revert*, *Put
//! the server back*), and one panel that applied on every save while the other
//! two waited for a button that was not visible where the edit was made.
//!
//! The operations now live here, opened from the bar's *Server…* button. Every
//! subject gets the same block, in the same order, with the same words:
//!
//! ```text
//! ┌ <subject>  N row(s) changed · M applied      [Discard] [Put back] [Apply] ┐
//! │ <a refusal, a trouble, or an unsaved table, when there is one>            │
//! └───────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The subject's name carries the rest in its hover text: the tables it
//! writes, the file the statements are in, and what it takes for an applied
//! change to be live. A block with changes not yet applied is outlined amber.
//!
//! The subject panels show which rows this project changes and what state
//! they are in, and direct a person to this panel for the operations. See
//! [`super::items`] and [`super::creatures`].
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
//! seven row subjects stand in the database in the panel's order, and an item
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
use bevy_egui::egui;

use super::theme;
use crate::server::{
    behaviour, creatures, dbcs, gameobjects, items, loot, queue::ServerQueue, quests, rows,
    services, settings::ServerSettings, stack,
};
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
/// could be shown for longer than [`REFRESH`] and be wrong in a way that
/// matters.
#[derive(Resource, Default)]
pub struct Standings {
    /// `(taken at, what it said)` per subject, in [`Half::ALL`]'s order.
    held: [Option<(f64, Standing)>; HALVES],
    /// The number of server tiles changed since the last regeneration. It is
    /// held longer than the others because computing it hashes every tile the
    /// project carries.
    tiles: Option<(f64, usize)>,
    /// Where the project's server DBC files stand against the live
    /// `DataDir\5875\dbc\`, or why that could not be read. See
    /// `crate::server::dbcs`.
    files: Option<(f64, Result<dbcs::Standing, String>)>,
}

/// How long one of those answers is reused for.
const REFRESH: f64 = 0.5;

/// How long the tile count is reused for. Computing it reads every tile in
/// the project.
const TILES_REFRESH: f64 = 3.0;

impl Standings {
    /// The number of tiles *Regenerate changed tiles* would rebuild. See
    /// `crate::server::datadir::dirty`.
    pub fn changed_tiles(&mut self, session: &EditSession, assets: &GameAssets, now: f64) -> usize {
        let fresh = self
            .tiles
            .is_some_and(|(taken, _)| now - taken < TILES_REFRESH);
        if !fresh {
            let maps = crate::session::map_directories(assets);
            let record = crate::server::datadir::Record::read(
                &session.project,
                crate::server::datadir::LIVE_RECORD,
            );
            let count = crate::server::datadir::dirty(session, assets, &maps, &record).0.len();
            self.tiles = Some((now, count));
        }
        self.tiles.map(|(_, count)| count).unwrap_or(0)
    }

    /// The subject's [`Standing`], recomputed if the held answer is stale.
    fn of(
        &mut self,
        half: Half,
        session: &EditSession,
        assets: &GameAssets,
        now: f64,
    ) -> &Standing {
        let at = half as usize;
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
    pub fn forget(&mut self) {
        self.held = Default::default();
        self.tiles = None;
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
/// The row subjects are listed in [`stack::Subject::ORDER`], the order they
/// stand in the database; the test
/// `the_panel_lists_the_row_subjects_in_the_stacks_order` checks it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Half {
    /// The rows the server keeps of two client tables, `Spell.dbc` and
    /// `TaxiNodes.dbc`, written from the project's copies of those files.
    Tables,
    Creatures,
    GameObjects,
    Items,
    Quests,
    /// The nine loot tables, which have no rail entry of their own: a loot
    /// set is reached from the creature, the object or the item it hangs off.
    Loot,
    /// What creatures sell and teach, reached from a selected creature's
    /// Vendor and Trainer windows.
    Services,
    /// A creature's events, scripts and spell lists, reached from a selected
    /// creature's Events and Spells windows.
    Behaviour,
}

/// The number of subjects. [`Standings`] holds one answer for each.
pub const HALVES: usize = 8;

impl Half {
    pub const ALL: [Half; 8] = [
        Half::Tables,
        Half::Creatures,
        Half::GameObjects,
        Half::Items,
        Half::Quests,
        Half::Loot,
        Half::Services,
        Half::Behaviour,
    ];

    /// The subject's name, in the rail's wording.
    pub fn name(self) -> &'static str {
        match self {
            Half::Tables => "Client tables",
            Half::Creatures => "Creatures",
            Half::GameObjects => "Objects",
            Half::Items => "Items",
            Half::Quests => "Quests",
            Half::Loot => "Loot",
            Half::Services => "Vendors and trainers",
            Half::Behaviour => "Behaviour",
        }
    }

    /// The database tables the subject writes. The name is the editor's word;
    /// the table names are what the database shows.
    pub fn tables(self) -> &'static str {
        match self {
            Half::Tables => "spell_template, taxi_nodes",
            Half::Creatures => "creature_template, creature, creature_movement",
            Half::GameObjects => "gameobject_template, gameobject",
            Half::Items => "item_template",
            Half::Quests => "quest_template, and the four quest relation tables",
            Half::Loot => "the nine *_loot_template tables",
            Half::Services => "npc_vendor, npc_vendor_template, npc_trainer, npc_trainer_template",
            Half::Behaviour => "creature_ai_events, creature_spells, broadcast_text, and the eleven *_scripts tables",
        }
    }

    /// Where a save writes the statements, as a virtual path in the project.
    pub fn file(self) -> &'static str {
        match self {
            Half::Tables => rows::VPATH,
            Half::Creatures => creatures::SQL_VPATH,
            Half::GameObjects => gameobjects::SQL_VPATH,
            Half::Items => items::SQL_VPATH,
            Half::Quests => quests::SQL_VPATH,
            Half::Loot => loot::SQL_VPATH,
            Half::Services => services::SQL_VPATH,
            Half::Behaviour => behaviour::SQL_VPATH,
        }
    }

    /// Where the statements that put the rows back are kept, as a virtual
    /// path in the project.
    pub fn revert_file(self) -> &'static str {
        match self {
            Half::Tables => rows::REVERT_VPATH,
            Half::Creatures => creatures::REVERT_VPATH,
            Half::GameObjects => gameobjects::REVERT_VPATH,
            Half::Items => items::REVERT_VPATH,
            Half::Quests => quests::REVERT_VPATH,
            Half::Loot => loot::REVERT_VPATH,
            Half::Services => services::REVERT_VPATH,
            Half::Behaviour => behaviour::REVERT_VPATH,
        }
    }

    /// What it takes for an applied row to become live. The module comment
    /// lists how this differs between subjects.
    pub fn going_live(self) -> &'static str {
        match self {
            Half::Tables => {
                "A spell is live on `.reload spell_template`, which an apply sends to a \
                 running playtest. A flight node needs a restart: `taxi_nodes` is read at \
                 startup and has no reload. So do the DBC files Apply copies into \
                 DataDir\\5875\\dbc, TaxiPath and TaxiPathNode among them. A spell *removed* \
                 needs a restart too: the reload overwrites and \
                 adds, and never drops one it has already read."
            }
            Half::Creatures => {
                "The server has to be restarted. `.reload creature_template` does not \
                 restat creatures already spawned, and `.reload creature` never erases a \
                 spawn it has read."
            }
            Half::GameObjects => {
                "The server has to be restarted. `.reload gameobject` never erases a spawn \
                 it has read, and an object already in the world keeps the display id, \
                 faction, flags and size it was created with."
            }
            Half::Items => {
                "Live on `.reload item_template`, which an apply sends to a running \
                 playtest — for every copy of the item already in a bag, in the world \
                 and on the auction house. No restart and no relog. An item *removed* \
                 needs a restart, and the apply sends no reload while one is in the \
                 plan: a reload would leave every copy already loaded with no \
                 prototype, which the server does not check for."
            }
            Half::Quests => {
                "Live on `.reload quest_template` and then one reload per relation table \
                 written, which an apply sends to a running playtest in that order \u{2014} \
                 removals included. The reload frees every quest the server holds, so do \
                 not apply while an escort quest is in progress on it."
            }
            Half::Loot => {
                "Live on `.reload <table>` for each loot table written, which an apply sends \
                 to a running playtest \u{2014} removals included, since every loot loader \
                 clears its store before it reads. Loot already rolled onto a corpse or into \
                 an opened chest keeps the list it was given."
            }
            Half::Services => {
                "Live on `.reload npc_vendor` and `.reload npc_trainer`, which an apply sends \
                 to a running playtest \u{2014} removals included, since each re-reads its \
                 template table and its own and clears both lists first. The client asks for a \
                 list each time the window opens, so no relog is needed. A vendor's current \
                 count of a limited item is not reset."
            }
            Half::Behaviour => {
                "Live on `.reload creature_ai_events`, which re-reads creature_ai_scripts \
                 first, and `.reload creature_spells`, which an apply sends to a running \
                 playtest; a creature already in the world keeps the events and the list it \
                 spawned with until it respawns. Five of the script tables reload under \
                 their own names; the other six and broadcast_text are read at start, so a \
                 change to one of those needs a restart."
            }
        }
    }

    /// The sentence added to the Apply and Put back tooltips saying what those
    /// operations also do to the blocks below this one. See
    /// `crate::server::stack`. Empty for the client-table block and for the
    /// last subject in the stack, which has nothing below it.
    pub fn order_note(self) -> &'static str {
        match self.subject() {
            None | Some(stack::Subject::Behaviour) => "",
            Some(_) => {
                " Applied subjects below this one are put back first and applied again \
                 after, because this one's statements can move their rows."
            }
        }
    }

    /// The row subject this block is, in [`stack::Subject`]'s terms. `None`
    /// for the client-table block, which is not in the stack.
    pub fn subject(self) -> Option<stack::Subject> {
        match self {
            Half::Tables => None,
            Half::Creatures => Some(stack::Subject::Creatures),
            Half::GameObjects => Some(stack::Subject::GameObjects),
            Half::Items => Some(stack::Subject::Items),
            Half::Quests => Some(stack::Subject::Quests),
            Half::Loot => Some(stack::Subject::Loot),
            Half::Services => Some(stack::Subject::Services),
            Half::Behaviour => Some(stack::Subject::Behaviour),
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
    /// subject: [`stack::Subject::owns`], which the writers use. The
    /// client-table block owns none, because its edits are files.
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
    /// caveat to [`Standing::line`]. See `creatures::OnTheServer::current`.
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
}

impl Standing {
    /// Whether there is anything to say about this subject at all.
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
                "{applied} row(s) applied and undoable, and this project now changes none \
                 of them"
            ),
            (changed, applied) => {
                let caveat = match (self.outstanding, self.current) {
                    (0, true) => " \u{2014} all applied".to_string(),
                    (0, false) => " \u{2014} all applied, though something has been edited \
                                   since or this is a later session"
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
            let undo = rows::Undo::open(session).unwrap_or(rows::Undo {
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
            }
        }
        Half::Creatures => {
            let plan = creatures::plan(session);
            let on_server = creatures::OnTheServer::read_with(session, &plan);
            Standing {
                changed: plan.rows.len() + plan.paths.len(),
                outstanding: plan
                    .rows
                    .iter()
                    .filter(|row| !on_server.covers(row.table, &row.key))
                    .count()
                    + plan
                        .paths
                        .iter()
                        .filter(|path| !on_server.covers(path.which.table(), &path.key()))
                        .count(),
                applied: on_server.rows(),
                current: on_server.current(),
                refused: plan.refused.clone(),
                trouble: None,
                unsaved: false,
            }
        }
        Half::GameObjects => {
            let plan = gameobjects::plan(session);
            let on_server = gameobjects::OnTheServer::read_with(session, &plan);
            Standing {
                changed: plan.rows.len(),
                outstanding: plan
                    .rows
                    .iter()
                    .filter(|row| !on_server.covers(row.table, &row.key))
                    .count(),
                applied: on_server.rows(),
                current: on_server.current(),
                refused: plan.refused.clone(),
                trouble: None,
                unsaved: false,
            }
        }
        Half::Items => {
            let plan = items::plan(session);
            let on_server = items::OnTheServer::read_with(session, &plan);
            Standing {
                changed: plan.rows.len(),
                outstanding: plan
                    .rows
                    .iter()
                    .filter(|row| !on_server.covers(row.table, &row.key))
                    .count(),
                applied: on_server.rows(),
                current: on_server.current(),
                refused: plan.refused.clone(),
                trouble: None,
                unsaved: false,
            }
        }
        Half::Quests => {
            let plan = quests::plan(session);
            let on_server = quests::OnTheServer::read_with(session, &plan);
            Standing {
                changed: plan.rows.len(),
                outstanding: plan
                    .rows
                    .iter()
                    .filter(|row| !on_server.covers(row.table, &row.key))
                    .count(),
                applied: on_server.rows(),
                current: on_server.current(),
                refused: plan.refused.clone(),
                trouble: None,
                unsaved: false,
            }
        }
        Half::Loot => {
            let plan = loot::plan(session);
            let on_server = loot::OnTheServer::read_with(session, &plan);
            Standing {
                changed: plan.rows.len(),
                outstanding: plan
                    .rows
                    .iter()
                    .filter(|row| !on_server.covers(row.table, &row.key))
                    .count(),
                applied: on_server.rows(),
                current: on_server.current(),
                refused: plan.refused.clone(),
                trouble: None,
                unsaved: false,
            }
        }
        Half::Services => {
            let plan = services::plan(session);
            let on_server = services::OnTheServer::read_with(session, &plan);
            Standing {
                changed: plan.rows.len(),
                outstanding: plan
                    .rows
                    .iter()
                    .filter(|row| !on_server.covers(row.table, &row.key))
                    .count(),
                applied: on_server.rows(),
                current: on_server.current(),
                refused: plan.refused.clone(),
                trouble: None,
                unsaved: false,
            }
        }
        Half::Behaviour => {
            let plan = behaviour::plan(session);
            let on_server = behaviour::OnTheServer::read_with(session, &plan);
            let rows = plan
                .rows
                .iter()
                .filter(|row| !on_server.covers(row.table, &row.key))
                .count();
            let scripts = plan
                .scripts
                .iter()
                .filter(|script| !on_server.covers(script.table, &script.key()))
                .count();
            Standing {
                changed: plan.count(),
                outstanding: rows + scripts,
                applied: on_server.rows(),
                current: on_server.current(),
                refused: plan.refused.clone(),
                trouble: None,
                unsaved: false,
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
        Half::Creatures => creatures::save(work.session),
        Half::GameObjects => {
            creatures::save(work.session);
            gameobjects::save(work.session);
        }
        Half::Items => {
            // The shared store is written first. Both subjects use it, and
            // the creature writer is the one that writes it.
            creatures::save(work.session);
            items::save(work.session);
        }
        Half::Quests => {
            creatures::save(work.session);
            quests::save(work.session);
        }
        Half::Loot => {
            creatures::save(work.session);
            loot::save(work.session);
        }
        Half::Services => {
            creatures::save(work.session);
            services::save(work.session);
        }
        Half::Behaviour => {
            creatures::save(work.session);
            behaviour::save(work.session);
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
            // the press: a few hundred kilobytes, and nothing to wait for.
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
        // The six row subjects go through the stack, which puts back and
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
                        Some(files) => format!("{files}; putting back client table rows"),
                        None => "putting back client table rows".to_string(),
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
            Err(e) => format!("server DBC files not put back: {e}"),
        },
    )
}

/// Gives up this project's claim on the subject's rows. The database is not
/// touched.
pub fn discard(half: Half, session: &mut EditSession) -> String {
    let (rows, paths) =
        session.forget_server_edits(|table| half.owns(table), half == Half::Creatures);
    match (rows, paths) {
        (0, 0) => "nothing to discard".to_string(),
        (rows, 0) => format!("{rows} row edit(s) discarded"),
        (0, paths) => format!("{paths} waypoint path(s) discarded"),
        (rows, paths) => format!("{rows} row edit(s) and {paths} waypoint path(s) discarded"),
    }
}

/// Draws the whole section: the *Apply on save* switch, then one block per
/// subject.
pub fn project(ui: &mut egui::Ui, work: &mut Work<'_>) {
    theme::heading(ui, "This project on the server");
    let have_a_database = work.server.resolve().is_some();

    // One switch covers every subject, as its label says. See
    // `ServerSettings::apply_on_save`.
    if ui
        .add_enabled(
            have_a_database,
            egui::Checkbox::new(&mut work.server.apply_on_save, "Apply on save"),
        )
        .on_hover_text(
            "Every save runs what it has just written, for every subject, and tells a \
             running playtest to re-read the tables that take a reload. Off writes the SQL \
             files and touches nothing.",
        )
        .on_disabled_hover_text("There is no database to apply to.")
        .changed()
    {
        work.server.save(&work.assets.root);
    }
    theme::note(
        ui,
        "A save always writes the statements into the project folder. This decides whether \
         it also runs them.",
    );

    ui.add_space(4.0);
    for half in Half::ALL {
        block(ui, work, half, have_a_database);
        ui.add_space(4.0);
    }
}

/// Draws one subject's block. The module comment shows its layout.
fn block(ui: &mut egui::Ui, work: &mut Work<'_>, half: Half, have_a_database: bool) {
    let standing = work
        .standings
        .of(half, work.session, work.assets, work.now)
        .clone();
    // The line is amber while changed rows are not applied, and plain once the
    // database holds them all. A warning colour that is always shown stops
    // being read as a warning.
    let outstanding = standing.changed > 0 && standing.outstanding > 0;
    let colour = match outstanding {
        false => theme::INK_DIM,
        true => theme::WARN,
    };
    let stroke = match outstanding {
        false => theme::LINE,
        true => theme::WARN,
    };
    egui::Frame::default()
        .fill(theme::SUNK)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            block_body(ui, work, half, have_a_database, &standing, colour);
        });
}

/// The inside of one subject's block: the name, the state and the buttons on
/// one line, and under them, when there is one, the plan's error, each
/// refusal, the server DBC files' line and the unsaved-table note.
fn block_body(
    ui: &mut egui::Ui,
    work: &mut Work<'_>,
    half: Half,
    have_a_database: bool,
    standing: &Standing,
    colour: egui::Color32,
) {
    let about = format!(
        "{}\n\nA save writes the statements to {}; {} holds the ones that put every \
         applied row back.\n\n{}",
        half.tables(),
        half.file(),
        half.revert_file(),
        half.going_live()
    );
    // The client-table block also stands for the DBC files the server reads
    // from DataDir\5875\dbc, which Apply copies and Put back restores.
    let files = match half {
        Half::Tables => Some(work.standings.files(work.session, work.server, work.now).clone()),
        _ => None,
    };
    let (files_outstanding, files_saved) = match &files {
        Some(Ok(files)) => (files.outstanding(), files.saved),
        _ => (false, 0),
    };
    // Apply and Put back are disabled while a write is running, rather than
    // queueing behind it. A second press while the first is still running is
    // almost always a repeat of the same press.
    let busy = work.queue.busy();
    let mut said: Option<Result<String, String>> = None;
    egui::Sides::new().shrink_left().wrap().show(
        ui,
        |ui| {
            ui.label(egui::RichText::new(half.name()).strong().size(13.0).color(theme::INK))
                .on_hover_text(&about);
            ui.label(egui::RichText::new(standing.line()).small().color(colour));
        },
        |ui| {
            buttons(
                ui,
                work,
                half,
                have_a_database && (standing.appliable() || files_outstanding) && !busy,
                have_a_database && (standing.applied > 0 || files_saved > 0) && !busy,
                have_a_database,
                busy,
                standing.changed > 0,
                &mut said,
            );
        },
    );
    if let Some(trouble) = &standing.trouble {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
    }
    for refused in &standing.refused {
        ui.label(egui::RichText::new(refused).small().color(theme::BAD));
    }
    // The client-table block also stands for the DBC files the server reads
    // from DataDir\5875\dbc, which Apply copies and Put back restores.
    match &files {
        Some(Ok(files)) => {
            let colour = match files.outstanding() {
                true => theme::WARN,
                false => theme::INK_DIM,
            };
            ui.label(egui::RichText::new(files.line()).small().color(colour))
                .on_hover_text(format!(
                    "The client tables vmangos reads as files: {}. Apply copies the \
                     project's copies into DataDir\\5875\\dbc, saving the server's own under \
                     the project's {} first; Put back restores those.",
                    match files.carried.is_empty() {
                        true => "none in this project".to_string(),
                        false => files.carried.join(", "),
                    },
                    dbcs::BEFORE_DIR
                ));
        }
        Some(Err(e)) if !dbcs::carried(&work.session.project).is_empty() => {
            ui.label(
                egui::RichText::new(format!("server DBC files: {e}"))
                    .small()
                    .color(theme::BAD),
            );
        }
        _ => {}
    }
    if standing.unsaved {
        ui.label(
            egui::RichText::new(
                "\u{2026}and a table edited since the last save, which is in no count above. \
                 Apply writes it into the project first.",
            )
            .small()
            .color(theme::WARN),
        );
    }

    if let Some(said) = said {
        // Any of these presses makes the numbers above stale. See
        // [`Standings`]: the risk of that cache is a held answer that outlives
        // the press that changed it.
        work.standings.forget();
        work.session.status = match said {
            Ok(line) => {
                bevy::prelude::info!("server: {line}");
                line
            }
            Err(e) => {
                bevy::prelude::warn!("server: {e}");
                format!("{}: {e}", half.name().to_lowercase())
            }
        };
    }
}

/// A block's three buttons, laid out from the right: Apply, Put back, then
/// Discard or the note that the subject is edited as a file. A press is
/// answered into `said`.
#[allow(clippy::too_many_arguments)]
fn buttons(
    ui: &mut egui::Ui,
    work: &mut Work<'_>,
    half: Half,
    can_apply: bool,
    can_put_back: bool,
    have_a_database: bool,
    busy: bool,
    changed: bool,
    said: &mut Option<Result<String, String>>,
) {
    {
        if ui
            .add_enabled(can_apply, egui::Button::new("Apply"))
            .on_hover_text(format!(
                "Write {} and run it against the world database, after writing the \
                 statements that put every row back into {}.{}",
                half.file(),
                half.revert_file(),
                half.order_note()
            ))
            .on_disabled_hover_text(match (have_a_database, busy) {
                (false, _) => "There is no world database to reach.",
                (true, true) => "A server sync is in progress.",
                (true, false) => "This project changes no row of these tables.",
            })
            .clicked()
        {
            *said = Some(apply(half, work));
        }
        if ui
            .add_enabled(can_put_back, egui::Button::new("Put back"))
            .on_hover_text(format!(
                "Run {}, which returns every row this project has applied to what it held \
                 before the project first touched it, and then forgets it.{}",
                half.revert_file(),
                half.order_note()
            ))
            .on_disabled_hover_text(match (have_a_database, busy) {
                (false, _) => "There is no world database to talk to.",
                (true, true) => "A server sync is in progress.",
                (true, false) => "This project has applied nothing.",
            })
            .clicked()
        {
            *said = Some(put_back(half, work));
        }
        if half.discardable() {
            if ui
                .add_enabled(changed, egui::Button::new("Discard"))
                .on_hover_text(
                    "Give up every edit this project carries for these tables. The rows in \
                     the database are not touched, and the record of them is kept: Put back \
                     still works afterwards, and the next Apply puts them back first.",
                )
                .on_disabled_hover_text("This project changes no row of these tables.")
                .clicked()
            {
                *said = Some(Ok(discard(half, work.session)));
            }
        } else {
            ui.label(
                egui::RichText::new("edited as a file")
                    .small()
                    .color(theme::INK_FAINT),
            )
            .on_hover_text(
                "A spell edit is this project's own Spell.dbc against the archives', so \
                 there is no claim to give up: the spell workspace's Undo and Discard are \
                 what take one back.",
            );
        }
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
            .chain(vale_mangos::gameobject::TABLES.iter())
            .chain(vale_mangos::item::TABLES.iter())
            .chain(vale_mangos::quest::TABLES.iter())
            .chain(vale_mangos::loot::TABLES.iter())
            .chain(services::TABLES.iter())
            .chain(vale_mangos::scripts::TABLES.iter())
            .chain([vale_mangos::eventai::TABLE, vale_mangos::creaturespells::TABLE, vale_mangos::broadcast::TABLE].iter())
            .copied()
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
        let listed: Vec<stack::Subject> = Half::ALL.iter().filter_map(|half| half.subject()).collect();
        assert_eq!(listed, stack::Subject::ORDER.to_vec());
    }

    /// A subject indexes the cache by its own discriminant, so the order of
    /// the enum and the order of [`Half::ALL`] must match. Nothing else checks
    /// them against each other, and a mismatch would show the creature numbers
    /// under the item heading rather than failing.
    #[test]
    fn each_subject_indexes_its_own_slot() {
        assert_eq!(Half::ALL.len(), HALVES);
        for (at, half) in Half::ALL.into_iter().enumerate() {
            assert_eq!(half as usize, at, "{half:?} indexes {}", half as usize);
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
