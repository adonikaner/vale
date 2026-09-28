//! What the editor sends to the server it playtests against, and what a save
//! writes for that server.
//!
//! ```text
//! reload.rs      the chat wire: a GM command sent on the playtest's own
//!                session, and the server's answer matched back to it
//! rows.rs        the rows the server keeps of two client tables, written as a
//!                diff of the project's DBC against the archives': Spell.dbc to
//!                spell_template, TaxiNodes.dbc to taxi_nodes
//! mod.rs         `save`, the one function every subject's SQL is written by,
//!                and the statements run when the Server panel's switch is on
//! creatures.rs   creature_template and creature, which have no client file,
//!                kept in the project's own store. No reload makes a change
//!                live; a restart does
//! gameobjects.rs gameobject_template and gameobject, on the creature tables'
//!                terms: live after a restart
//! items.rs       item_template, the same shape, and live on a `.reload`: the
//!                prototype map is cleared before it is read and
//!                Item::GetProto is a lookup per call
//! quests.rs      quest_template and its four relation tables, the same shape;
//!                a removal is live on a reload too, because both loaders clear
//!                their maps first
//! loot.rs        the nine *_loot_template tables, one schema under nine names,
//!                each live on its own reload, removals included
//! services.rs    npc_vendor, npc_trainer and their two template tables: what a
//!                creature sells and teaches, live on two reloads, removals
//!                included
//! behaviour.rs   creature_ai_events and creature_spells as keyed rows, and the
//!                eleven *_scripts tables as whole scripts under an id, since a
//!                script row has no key; live on a reload of the events, the
//!                lists and five of the script tables
//! queue.rs       where applies run: off the main thread, one at a time, with a
//!                count of every read and write in progress for the toast
//! reconcile.rs   the order every apply runs in: what the project applied
//!                before is put back, then each row is read, its undo kept and
//!                its statements run, so the database always holds what it
//!                held before plus what the project says now
//! stack.rs       the order the seven row subjects stand in the database: to
//!                apply or put back one, every applied subject after it is put
//!                back first and applied again after, so each revert file runs
//!                against the database it was read from
//! follow.rs      changing a row's id in the project's store: the row's claim
//!                re-keyed, and the project's other claims that name the id
//!                rewritten to name the new one
//! settings.rs    where this machine's server is, a setting of the editor's own
//!                because a 1.12 client has nowhere to keep one
//! dbcs.rs        the DBC files the server reads from DataDir\5875\dbc, copied
//!                from the project into the live folder by the client-table
//!                block's Apply and by Apply on save, with the server's own
//!                copies kept in the project for Put back
//! held.rs        what a project folder has in the database and the live
//!                server, read from its revert files: the gate on Clear and
//!                Delete, which remove that record
//! release.rs     what a patch gives the server as rows: one migration in
//!                vmangos' own form holding all of the project's rows, and the
//!                DBCs the server reads as files
//! datadir.rs     the server's tiles (maps, vmaps, mmaps), regenerated through
//!                the four vmangos tools for the tiles a project changed.
//!                Written into the live DataDir by the dev-time buttons and
//!                into a patch folder by a publish
//! patch.rs       the patch a publish makes: one folder holding the archive,
//!                the DBCs, the migration, the server tiles, a README saying
//!                where each goes and a manifest with a hash of each. A publish
//!                changes nothing in this machine's install
//! ```
//!
//! ## How the subjects are written and applied
//!
//! The subjects are written differently and applied the same way. [`save`] is
//! everything a save does to the server, and [`crate::ui::sync`] is everything
//! a person can ask of it. Each writer here answers a plan, an apply and a
//! revert, and neither of those two files knows anything about a subject beyond
//! what the writer answers.
//!
//! The plans come from two kinds of source. [`rows`] diffs the project's
//! `Spell.dbc` and `TaxiNodes.dbc` against the archives' copies, so its
//! statements are the same whatever order the edits were made in, and an edit
//! undone drops out of the plan. [`creatures`], [`gameobjects`], [`items`],
//! [`quests`], [`loot`], [`services`] and [`behaviour`] read a store of typed
//! values, shared and keyed by table, each writer skipping the others' rows.
//! [`dbcs`] copies files rather than writing rows.
//!
//! What makes an applied change live differs by table, and each block of the
//! Server panel states it:
//!
//! ```text
//! spell_template       .reload spell_template
//! taxi_nodes           a restart; it has no reload
//! DataDir\5875\dbc     a restart; its files, TaxiPath and TaxiPathNode among
//!                      them
//! item_template        .reload item_template, live for every copy of the item
//!                      already in a bag, in the world and on the auction
//!                      house: LoadItemPrototypes clears its map before it
//!                      reads and Item::GetProto is a lookup per call
//! creature_template    a restart. .reload does not update creatures already
//! creature             spawned, and never erases a spawn it has read
//! gameobject_template  a restart, for the same two reasons: an object in the
//! gameobject           world keeps what it was created with, and a spawn that
//!                      has been read is never erased
//! quest_template       .reload quest_template and then one per relation table
//! *_questrelation      written, in that order, removals included, because
//! *_involvedrelation   both loaders clear their maps before they read
//! *_loot_template      .reload <table>, one per loot table written, removals
//!                      included, for the same reason. Loot already rolled onto
//!                      a corpse in the world keeps its list
//! npc_vendor           .reload npc_vendor and .reload npc_trainer, each of
//! npc_trainer          which re-reads its template table first, removals
//! and their templates  included
//! ```
//!
//! ## Why the server half is a directory of its own
//!
//! The rest of this crate edits files (an ADT, a DBC, the project folder) and
//! the client reads what it wrote. vmangos reads its world from `DataDir`'s
//! derived tiles and a MySQL database, both once at startup. An edit that is to
//! reach the running server therefore needs a second destination with its own
//! writer, its own reload and its own failure modes, and this directory is the
//! editor's end of it.
//!
//! It is built in order: the wire ([`reload`]) first, then
//! the row writers and the tile driver. What a row means is decided in
//! `crates/mangos`, on the rule `crates/assets` keeps for files.
//!
//! ## Commands go over chat
//!
//! There is no editor opcode. Every command goes out as a line of chat,
//! authenticated by the account's GM level, and is answered by a line of chat,
//! so this client stays a 1.12 client and the server needs no version
//! handshake to talk to it. The plan's section *What is deliberately not in
//! this plan* gives the argument.

pub mod behaviour;
pub mod creatures;
pub mod dbcs;
pub mod datadir;
pub mod follow;
pub mod gameobjects;
pub mod held;
pub mod items;
pub mod loot;
pub mod patch;
pub mod queue;
pub mod quests;
pub mod reconcile;
pub mod release;
pub mod reload;
pub mod services;
pub mod rows;
pub mod settings;
pub mod stack;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// The server half of a save, for every subject.
///
/// One function, because a save is one gesture. When each caller listed the
/// writers it knew about, there were three callers and three writers, and
/// `sql\items.sql` was written by none of them: an item edit was in the
/// project's store and in no file until Apply was pressed.
///
/// What it does, in order:
///
/// ```text
/// 1  the store           server\rows.txt, which every subject's rows share
/// 2  the SQL             one file per subject, always, whatever the switch says
/// 3  the files           the server DBCs copied into DataDir\5875\dbc, when
///                        `apply_on_save` is on; see [`dbcs`]
/// 4  the database        the statements queued to run, when `apply_on_save`
///                        is on; see [`queue`]
/// 5  the reload          for the tables an apply can make live, requested by
///                        each write when it completes
/// ```
///
/// Steps 3 and 4 cover every subject; see
/// [`settings::ServerSettings::apply_on_save`]. The subjects differ in what
/// makes an applied row live, and each block of the Server panel says which.
///
/// Nothing here fails a save. The project folder is written first, and every
/// failure after that is a line on the status bar: the tiles, the tables and
/// the SQL are what the person asked for, and a missing database is not a
/// reason to lose them.
pub fn save(
    session: &mut crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    server: &settings::ServerSettings,
    queue: &mut queue::ServerQueue,
) {
    // The store and the creature SQL. The store is shared, so this writes the
    // item rows too; see `creatures::save`.
    creatures::save(session);
    gameobjects::save(session);
    items::save(session);
    quests::save(session);
    loot::save(session);
    services::save(session);
    behaviour::save(session);
    // The client-table rows, which are a diff of two files rather than a
    // store, and which return their own apply under the same switch.
    if let Some(work) = rows::save(session, assets, server) {
        queue.push("applying spells", work);
    }
    // The DBC files the server reads from DataDir\5875\dbc go with the rows under
    // the same switch. They need no database, only the folder.
    if server.apply_on_save {
        save_files(session, server);
    }
    if !server.apply_on_save || server.resolve().is_none() {
        return;
    }
    // The signature stops a save from applying a plan the database already
    // holds and reporting it again. The client-table rows make the same check
    // inside `rows::save`; the row subjects make it in `stack::order`, which
    // starts at the first subject whose plan is not what the database holds.
    //
    // An empty plan still applies when the project has rows in the database:
    // the project has stopped claiming them, and an apply takes them back. See
    // `reconcile`.
    //
    // All five row subjects are one write, in the stack's order; see
    // [`stack`]. Applying each subject on its own lost a loot row's original
    // value under an item renumber.
    match stack::work(stack::Wanted::Save, session, server) {
        Ok(Some((label, work))) => queue.push(label, work),
        Ok(None) => {}
        Err(e) => {
            warn!("server: {e}");
            session.status = format!("saved, but the server rows were not applied: {e}");
        }
    }
}

/// Copy the project's server DBC files into the live folder for a save with
/// Apply on save on. Says so on the status line only when something was
/// copied or put back, or when the folder could not be reached with files to
/// copy.
fn save_files(session: &mut crate::session::EditSession, server: &settings::ServerSettings) {
    let project = &session.project;
    if dbcs::carried(project).is_empty() && dbcs::saved(project).is_empty() {
        return;
    }
    let apply = |dir: std::path::PathBuf| {
        dbcs::apply_at(project, &vale_mangos::datadir::dbc_dir(&dir))
    };
    match datadir::data_dir(server).and_then(apply) {
        Ok(done) if done.written.is_empty() && done.restored.is_empty() => {}
        Ok(done) => {
            info!("server: {}", done.line());
            session.status = done.line();
        }
        Err(e) => {
            warn!("server: DBC files not copied: {e}");
            session.status = format!("saved, but the server DBC files were not copied: {e}");
        }
    }
}

/// What a save and a playtest need to reach the server, as one system
/// parameter.
///
/// Four resources used together: where the database is, the queue a save's
/// writes go on, whether the session a playtest starts keeps its query
/// answers, and the tile regeneration's progress. They are bundled because the
/// system that holds them (`tools::shortcuts`, which is Ctrl+S and Ctrl+P) is
/// at Bevy's limit of sixteen parameters; `ui::topbar::Session` is bundled for
/// the same reason.
#[derive(SystemParam)]
pub struct Reach<'w> {
    /// The queue a save's database writes go on; see [`queue`]. Each write
    /// requests its own `.reload` when it completes.
    pub queue: ResMut<'w, queue::ServerQueue>,
    /// Where this machine's server is, and the two switches; see [`settings`].
    pub settings: Res<'w, settings::ServerSettings>,
    /// Whether the next session keeps its `WDB\` answers; see
    /// `vale_client::world::session::QueryCaches` and
    /// [`settings::ServerSettings::disable_caching`].
    pub caches: ResMut<'w, vale_client::world::session::QueryCaches>,
    /// What the tile regeneration is doing now, for the toast; see
    /// [`datadir::Step`].
    pub step: Res<'w, datadir::Step>,
}

/// Everything the editor's server half adds to the app.
pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((reload::ReloadPlugin, settings::SettingsPlugin, queue::QueuePlugin))
            .init_resource::<datadir::Step>()
            // The one-shot command-line flags, which need no ordering.
            .add_systems(
                Update,
                (
                    rows::on_the_command_line,
                    creatures::on_the_command_line,
                    gameobjects::on_the_command_line,
                    items::on_the_command_line,
                    quests::on_the_command_line,
                    loot::on_the_command_line,
                    services::on_the_command_line,
                    behaviour::on_the_command_line,
                    patch::on_the_command_line,
                    release::migration_on_the_command_line,
                    datadir::on_the_command_line,
                ),
            );
    }
}
