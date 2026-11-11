//! A shared editing session from the editor's side: hosting one or joining
//! one, the project a joining editor fetches, every edit applied on the other
//! editors as it is made, and where each editor is. The protocol is
//! `vale_collab`'s; the Session popover is `crate::ui::shell::session`.
//!
//! ```text
//! mod.rs     the session's state, the frame's work and the plugin
//! changes.rs Submit, Revert, and the conflicts a revert leaves: steps split
//!            by group, inverted, and rebased over the state now
//! depot.rs   a session hosted with no window (`--depot`): the host's half
//!            without the renderer, saving what joined editors submit
//! files.rs   project files written or removed by an operation, sent as one
//!            step and applied on the other editors
//! fetch.rs   a joining editor's copy of the host's project: what it lacks,
//!            the pieces as they arrive, and the hash check
//! log.rs     the project's log: this install's author id, the log of the
//!            open project, and the steps written to it
//! offline.rs a joined editor's copy linked to the host: the link, the edits
//!            made on it offline, and sending them when it connects again
//! people.rs  the other editors drawn in the viewport: a ring at each one's
//!            pointer, a pole with their name at what they look at, and the
//!            chunks they changed in the last moments outlined
//! ```
//!
//! ## What is shared
//!
//! Two [`Step`] variants carry edits. The first is what the undo stack holds: terrain,
//! paint, liquid, areas, holes, placements, DBC table fields and records, and
//! server rows. The undo stack's journal
//! ([`vale_edit::undo::History::journal`]) hands each one over as it is made,
//! a held stroke in parts ten times a second, and each undo and redo. Each is
//! applied on the other editors through [`EditSession::apply_step`], the path
//! an undo takes.
//!
//! The second is project files written or removed whole, which the project's
//! own journal records: imports, deletes, created and pasted tiles, minimaps,
//! bakes, world map art and locks. See [`files`]. A save is not sent, because
//! every editor already holds the edits it writes.
//!
//! A shadow rebake is an edit like the others: each chunk's new `MCSH` is
//! an entry on the undo stack (`vale_edit::ops::Edit::Shadow`).
//!
//! A joined editor's edits go to the host like the host's own edits. The
//! number a new record takes (a placement's `uniqueId`, a table row's id, a
//! spawn's guid) comes from blocks the host hands each editor
//! (`crate::session::numbers`), so two editors never give two new records one
//! number. A joined editor's file steps may write textures, models, WMOs,
//! sounds, locks, tiles, WDTs, minimap pictures and the minimap index only
//! ([`files::joinable`]). Any other file step is reverted on the joined
//! editor with a status line, and the host refuses it as well.
//!
//! ## Deleting files
//!
//! Only the host deletes files. A joined editor's Delete asks the host
//! ([`Step::Delete`] in a `vale_collab::wire::ToHost::Ask`). The host sees the
//! confirmation its own Delete shows, with the name of the editor that asked,
//! and deletes the files or keeps them. Either way the editor that asked is
//! told. The host's delete is a forced file step, so it reaches every editor
//! in the host's order and is never reverted. The confirmation names the
//! other editors whose unsaved edits to the files are lost with them
//! ([`Collab::unsaved_by`]).
//!
//! ## Server writes
//!
//! A joined editor reads and playtests on one of two servers
//! ([`PlaytestServer`]), set by "Server" in the Session popover: this
//! machine's own, or the host's. Only the project's edits are shared; each
//! machine's server and world database stay its own.
//!
//! ```text
//! this machine's   reads the database Edit\server.txt names, playtests on
//!                  the realm in realmlist.wtf, and its Server panel applies
//!                  to that database
//! the host's       playtests on the host's realm, and reads the host's world
//!                  database when the host shares it; its Server panel is
//!                  disabled, because only the host applies to that server
//! ```
//!
//! The host shares its world database unless the Host form's Share the world
//! database is off: the welcome then carries the database's location and
//! login (`vale_collab::wire::Database`), and the editor reads it
//! (`crate::server::settings::ServerSettings::shared`) only while it uses
//! the host's server. The welcome picks the host's server when the host
//! shares its database or this machine has none, and this machine's
//! otherwise; a choice made in the popover is kept over later welcomes.
//!
//!
//! On this machine's server, the Server panel is disabled when this
//! machine's world database is the host's, as it is for two editors on one
//! machine whose `Edit\server.txt` files name the same `mangosd.conf`: the
//! welcome carries a fingerprint of the host's database location
//! ([`database_fingerprint`]), and the two fingerprints are compared. Two
//! editors applying to one database would each keep revert files for the
//! same rows, and a Restore from one would undo the other's applies. See
//! [`ServerGate`]. Offline, a copy uses this machine's database
//! like any project.
//!
//! The files that record what an editor applied to its own server are kept on
//! that editor's machine: a join neither sends the host's nor removes the
//! joined editor's (`fetch::machine_local`). Publishing and writing a
//! migration stay with the host. The Server panel's Drop Changes is an edit
//! like any other (`crate::server::sync::discard`): it is logged, undone and
//! shared, and a joined editor's drop that removes whole rows is reverted as
//! any such edit of a joined editor's is.
//!
//! ## When two editors change the same thing
//!
//! The host accepts whichever edit reaches it first and refuses the other
//! edit; `vale_collab::peer` says how the editor whose edit lost learns of it
//! before the refusal arrives. Here, a refused edit is reverted through
//! [`EditSession::apply_step`], and so is the part of a held stroke not sent
//! yet (the journal's open segment), and the status line names the editor that
//! made a conflicting edit first, in the words the host's refusal uses.
//!
//! ## Claims
//!
//! An edit claims the tiles, client table records (or whole tables, when
//! their ids repeat) and server rows it touches for its author until they
//! submit, and a check out claims a tile, table or row before any edit
//! (`vale_collab::checkout`). The host refuses another author's edit to a
//! claim and sends every editor the list ([`Collab::holds`]): the Session
//! popover lists it and the map window marks the tiles. Each editor undoes
//! its own edit on a claim another author holds before sending it, with the
//! holder's name: in a session from the host's list, and alone from the
//! log's unsubmitted edits ([`log_holds`]). The host's editor keeps the check
//! outs in `log\checkouts.txt` ([`CHECKOUTS_FILE`]) and resumes them when it
//! hosts again; the edited claims come back from the log. A Submit releases
//! the submitting author's claims.
//!
//! ## What a save writes
//!
//! The project's files hold its submitted changelists and nothing else. An
//! edit is everyone's to see as soon as it is made, and it is in the files
//! once its author submits:
//!
//! ```text
//! alone or hosting   a full save writes this editor's edits and submits
//!                    them, with the description typed or none
//! joined             Submit closes the changelist on the host, and the host
//!                    saves it (changes::save_submitted)
//! ```
//!
//! Every save writes everything: the open map's tiles, the tiles of other
//! maps, the client tables and the server rows
//! (`EditSession::save_files`). Alone and hosting, a save therefore leaves
//! this editor nothing unsaved and submits its edits, and this editor's edits
//! in no changelist are exactly its edits since its last save. Discard and
//! the recovery after a crash read them from the log; nothing else records
//! what the files hold.
//!
//! Another author's unsubmitted changes applied here are listed in
//! `EditSession::exclude`, and every save writes the tiles, tables and rows
//! with those taken back on a copy; the claims make that exact (*Claims*).
//! They are not this editor's unsaved edits, so Discard leaves them, and
//! History's Take Back closes them when their author is not connected
//! ([`changes`]'s *Taking back another author's edits*).
//!
//! A file step (an import, a delete, a minimap) of this editor's own is
//! written to the folder when it is made, and the next save submits it.
//! Another author's file step is held in the overlay the archives are read
//! through (`EditSession::staged`), so the host's folder does not hold it
//! until they submit; their Submit writes it ([`changes::save_submitted`]),
//! Take Back drops it, and a restart puts it back from the log.
//!
//! ## Undo
//!
//! Each editor's undo stack holds its own edits only; a remote edit is
//! applied beside it. Undoing an entry writes the state it found, which
//! would overwrite whatever another editor did there since. So when a remote
//! edit lands, every entry of this editor's stack that touches the same
//! groups is dropped ([`vale_edit::undo::History::forget_where`]): an editor
//! cannot undo over a change another editor made since.
//!
//! ## Joining
//!
//! The host saves its tiles, tables and server rows and sends the list of its
//! project's files, with a hash of each. The joining editor keeps a copy of
//! the host's project in `Edit\<project> (shared)\`, asks for the files its
//! copy lacks or holds different bytes of, and removes the files the host
//! does not have. Once every file is in, it opens the copy and asks the host
//! for every author's edits in no changelist, which the host's files do not
//! hold (`vale_collab::wire::Request::Open`). It applies those, then the
//! edits that arrived since the welcome, and goes to the host's map and view.
//! Its own project is saved and closed while it is joined and opened again
//! when it leaves.
//!
//! ## Edits that change nothing
//!
//! Edits undone again leave nothing changed, and are not edits. Alone and
//! hosting, this editor's are withdrawn from the log when nothing of its own
//! is unsaved ([`changes`]'s *Edits that change nothing*). A joined editor
//! asks the host to check after an undo, a redo or a Discard
//! (`vale_collab::wire::Request::Withdraw`): the host takes the author's
//! unsubmitted changes back on copies of what they touched, and when that
//! gives what it holds now, withdraws them and says so. Offline, the pending
//! edits are emptied the same way.

pub(crate) mod changes;
pub(crate) mod depot;
mod fetch;
pub(crate) mod files;
pub(crate) mod log;
pub(crate) mod offline;
mod people;

use crate::session::EditSession;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use vale_client::assets::GameAssets;
use vale_collab::log::{Author, AuthorId};
use vale_collab::wire::Answer;
use vale_collab::{Action, Cause, Claim, Group, Hold, Hosting, Joined, Log, Payload, Presence, Session, UserId};
use vale_edit::undo::Change;

/// One shared step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Step {
    /// A change off the undo stack, applied forward (an edit or a redo) or
    /// reverted (an undo). `continues` marks a later part of one undo entry
    /// sent in parts, which the log and History count with the part before it
    /// as one edit. See `vale_edit::undo::Journaled`.
    Edit { change: Change, forward: bool, continues: bool },
    /// Project files written or removed whole. See [`files`].
    Files(files::Files),
    /// Project files a joined editor asks the host to delete. Sent only as
    /// an ask, never as an edit. See *Deleting files*.
    Delete(Vec<String>),
}

impl Step {
    /// A forward edit of `change`, whole.
    pub(crate) fn edit(change: Change) -> Step {
        Step::Edit { change, forward: true, continues: false }
    }

    /// Whether this step is a later part of the edit before it.
    pub(crate) fn continues(&self) -> bool {
        matches!(self, Step::Edit { continues: true, .. })
    }

    /// The same step marked as a later part of the edit before it, or not.
    /// Only a change carries the mark.
    pub(crate) fn continuing(mut self, part: bool) -> Step {
        if let Step::Edit { continues, .. } = &mut self {
            *continues = part;
        }
        self
    }

    /// From a journal step.
    fn journaled(step: vale_edit::undo::Journaled) -> Step {
        Step::Edit { change: step.change, forward: step.forward, continues: step.continues }
    }
}

/// How many whole edits `steps` are: the steps that do not continue the one
/// before.
pub(crate) fn whole_edits<'a>(steps: impl IntoIterator<Item = &'a Step>) -> usize {
    steps.into_iter().filter(|step| !step.continues()).count()
}

impl Payload for Step {
    fn groups(&self) -> Vec<Group> {
        match self {
            Step::Edit { change, .. } => change.groups(),
            Step::Files(files) => files.groups(),
            Step::Delete(_) => Vec::new(),
        }
    }

    /// The host's file steps. See [`files`]'s *The host's steps are forced*.
    fn forced(&self) -> bool {
        matches!(self, Step::Files(_))
    }

    fn check(&self) -> Result<(), String> {
        match self {
            Step::Edit { .. } => Ok(()),
            Step::Files(files) => files.check(),
            Step::Delete(_) => Err("a delete is asked for, not sent".to_string()),
        }
    }
}

/// The port a session is hosted on unless the form says another.
pub(crate) const PORT: u16 = 7878;

/// The build a joining editor must match, besides the protocol.
const BUILD: &str = env!("CARGO_PKG_VERSION");

/// How often a held stroke is sent, in seconds.
const SEGMENT_EVERY: f64 = 0.1;

/// How often this editor says where it is, in seconds.
const PRESENCE_EVERY: f64 = 0.2;

/// Where the session stands.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) enum State {
    /// No session.
    #[default]
    Off,
    /// Hosting `project` on `addr`.
    Hosting { project: String, addr: std::net::SocketAddr },
    /// Connecting to a host and answering its challenge.
    Connecting { to: String },
    /// Fetching the host's project: files and bytes in so far, of how many.
    Fetching { project: String, files: usize, of_files: usize, bytes: u64, of_bytes: u64 },
    /// Joined, with the copy of the host's project open.
    Joined { project: String },
    /// No session, since the last one ended for this reason.
    Ended(String),
}

/// Which of the Session popover's two forms is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Mode {
    #[default]
    Host,
    Join,
}

/// What the Session popover or the Changes window asks for. Carried out by
/// [`run`] on the next frame, which holds the session, the archives and the
/// camera.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Ask {
    Host,
    Join,
    Leave,
    GoTo(UserId),
    Remove(UserId),
    /// A joined editor: ask the host to delete these project files.
    Delete(Vec<String>),
    /// The host: keep the files of the first request in [`Collab::requests`]
    /// and tell the editor that asked.
    Keep,
    /// Put this editor's open edits in a changelist. See [`changes`].
    Submit,
    /// Revert a changelist, by number.
    Revert(u32),
    /// Reapply a conflict's edits, or keep the current version, by the
    /// conflict's place in the list.
    Reapply(usize),
    KeepCurrent(usize),
    /// A joined editor: ask the host for its changelists.
    History,
    /// A joined editor: leave the session and keep the copy open offline.
    GoOffline,
    /// An offline copy: join the host it is linked to.
    Reconnect,
    /// An offline copy with no pending edits: remove its link.
    Unlink,
    /// Make the open offline copy the project, with the copy of the host's
    /// log, so it can host. See [`offline`]'s *Taking over*.
    TakeOver,
    /// In a session: check these out for this editor's author.
    CheckOut(Vec<Claim>),
    /// In a session: release these check outs of this author's.
    Release(Vec<Claim>),
    /// In a session: release another author's check outs.
    Break(Vec<Claim>),
    /// Restore the edits a crash left out of the project's files. See
    /// [`changes`]'s *Recovered edits*.
    RestoreRecovered,
    /// Take back the unsubmitted edits of an author who is not connected.
    /// See [`changes`]'s *Taking back another author's edits*.
    TakeBack(AuthorId),
    /// Show or hide a changelist's edits in History.
    Detail(u32),
}

/// A claim on one tile of the open map, as the map window draws it. See
/// [`Collab::map_claims`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TileClaim {
    pub(crate) author: AuthorId,
    /// The holder's name, or `this editor` for this editor's author.
    pub(crate) who: String,
    /// Held by this editor's author.
    pub(crate) mine: bool,
    pub(crate) checked_out: bool,
    pub(crate) edited: bool,
    /// The holder's colour, as the session popover draws the person.
    pub(crate) colour: [u8; 3],
}

impl TileClaim {
    /// How the tile is held, for the map window's status line:
    /// `checked out by Sam`, `held by Sam's unsubmitted edits`.
    pub(crate) fn words(&self) -> String {
        match (self.checked_out, self.edited) {
            (true, true) => format!("checked out by {}, with unsubmitted edits", self.who),
            (true, false) => format!("checked out by {}", self.who),
            (false, _) => format!("held by {}'s unsubmitted edits", self.who),
        }
    }
}

/// The claims on the open map's tiles, for the map window.
#[derive(Debug, Clone, Default)]
pub(crate) struct MapClaims {
    pub(crate) tiles: HashMap<(u32, u32), TileClaim>,
    /// Whether this editor is hosting or joined, which a check out, a
    /// release and a break need: the host keeps the claims.
    pub(crate) can_check_out: bool,
    /// The map the tiles are on, for the claims those three ask for.
    pub(crate) map: String,
}

impl MapClaims {
    /// The claim on one tile.
    pub(crate) fn claim(&self, at: (u32, u32)) -> Claim {
        Claim::Tile(vale_edit::TileKey::new(self.map.clone(), at.0, at.1))
    }
}

/// The file in the log folder where the host's editor keeps the check outs.
pub(crate) const CHECKOUTS_FILE: &str = "checkouts.txt";

/// What this editor may do in a shared session, for the panels whose
/// buttons depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Role {
    /// No session.
    #[default]
    Alone,
    Hosting,
    /// Joined to another's session, or joining one.
    Joined,
}

impl Role {
    pub(crate) fn joined(self) -> bool {
        self == Role::Joined
    }
}

/// Which server a joined editor reads and playtests on: the realm its
/// playtest logs in to, and the world database it reads. Picked at the
/// welcome and changed in the Session popover. See *Server writes*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PlaytestServer {
    /// This machine's: the realm in `realmlist.wtf` and the world database
    /// `Edit\server.txt` names.
    #[default]
    Local,
    /// The host's: the realm at the address this editor joined, and the
    /// host's world database when the host shares it.
    Host,
}

/// Whether this editor may write to the server its Server panel names: apply,
/// restore, regenerate server tiles. Every editor may, except a joined editor
/// that uses the host's server, or whose own world database is the host's.
/// See *Server writes*.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ServerGate {
    pub(crate) role: Role,
    /// The reason this editor may not write to its server, for the hover text
    /// of a disabled button. `None` when it may.
    pub(crate) refused: Option<String>,
    /// The host's name, while this editor is joined.
    pub(crate) host: Option<String>,
    /// The open project is a copy of a host's, joined or offline. See
    /// [`offline`].
    pub(crate) copy: bool,
}

impl ServerGate {
    pub(crate) fn writes(&self) -> bool {
        self.refused.is_none()
    }

    /// Why this editor may not publish or write a migration, or `None` when
    /// it may. Releases are the host's: a joined editor and an offline copy
    /// make none.
    pub(crate) fn release_refused(&self) -> Option<&'static str> {
        match (self.role.joined(), self.copy) {
            (true, _) => Some("In a shared session only the host publishes and writes migrations."),
            (false, true) => Some("This project is a copy of a host's project. Only the host publishes it and writes its migrations."),
            (false, false) => None,
        }
    }
}

/// A joined editor's request to delete project files, waiting for the host
/// to answer it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Request {
    pub(crate) user: UserId,
    pub(crate) name: String,
    pub(crate) paths: Vec<String>,
}

/// The Session popover's fields.
#[derive(Debug, Clone)]
pub(crate) struct Form {
    pub(crate) mode: Mode,
    pub(crate) name: String,
    pub(crate) password: String,
    /// The host to join: an address, with or without `:port`.
    pub(crate) address: String,
    /// The port to host on.
    pub(crate) port: String,
    /// Whether joined editors are sent this machine's world database
    /// location and login. See *Server writes*.
    pub(crate) share_database: bool,
}

impl Default for Form {
    fn default() -> Form {
        Form {
            mode: Mode::Host,
            name: std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default(),
            password: String::new(),
            address: String::new(),
            port: PORT.to_string(),
            share_database: true,
        }
    }
}

/// Someone in the session.
#[derive(Debug, Clone)]
pub(crate) struct Person {
    pub(crate) name: String,
    pub(crate) presence: Option<Presence>,
}

/// The session and everything the Session popover shows about it.
#[derive(Resource, Default)]
pub(crate) struct Collab {
    session: Option<Session<Step>>,
    /// A connection being opened off the frame, which can take seconds.
    connecting: Option<std::sync::Mutex<std::sync::mpsc::Receiver<std::io::Result<Joined<Step>>>>>,
    pub(crate) state: State,
    /// This editor's number, once it is in.
    pub(crate) me: Option<UserId>,
    pub(crate) people: BTreeMap<UserId, Person>,
    pub(crate) form: Form,
    pub(crate) ask: Option<Ask>,
    fetch: Option<fetch::Fetch>,
    /// Edits received while the project is fetched, and who made each,
    /// applied once it is open.
    held: Vec<(UserId, Step)>,
    /// The project this editor had open before it joined.
    own_project: Option<String>,
    presence_at: f64,
    segment_at: f64,
    /// Whole edits sent and received, and this editor's steps reverted, for
    /// the Session popover. A stroke sent in parts counts once.
    pub(crate) sent: u64,
    pub(crate) received: u64,
    pub(crate) reverted: u64,
    /// The chunks each other editor changed, by tile and chunk, and when:
    /// outlined in their colour for [`people::RECENT`] seconds.
    pub(crate) recent: HashMap<(UserId, vale_edit::TileKey, usize), f64>,
    /// The host: joined editors' requests to delete files, oldest first. The
    /// first is shown as a confirmation until it is answered.
    pub(crate) requests: VecDeque<Request>,
    /// Who else has edited each tile and table file since it was last saved,
    /// by lower-case virtual path. See [`Collab::unsaved_by`].
    edited: HashMap<String, BTreeSet<UserId>>,
    /// Project files another editor's step wrote or removed this frame, for
    /// the Assets workspace to forget what it read of them.
    changed: Vec<String>,
    /// Which server this editor reads and playtests on while it is joined.
    pub(crate) playtest_server: PlaytestServer,
    /// [`Self::playtest_server`] was chosen in the Session popover, so a
    /// welcome after a reconnect keeps it. Cleared when the session ends.
    pub(crate) server_chosen: bool,
    /// The host's address as this editor joined it, without the session's
    /// port: the realm address of a playtest on the host's server.
    host_address: Option<String>,
    /// The fingerprint of the host's world database location, from the
    /// welcome. See [`database_fingerprint`].
    host_database: Option<[u8; 32]>,
    /// The fingerprint of this machine's world database, read again once a
    /// second, and its location for the Server panel's hover text, without
    /// the password.
    own_database: Option<[u8; 32]>,
    own_line: Option<String>,
    database_at: f64,
    /// The frame's clock, for the actions that note a time.
    now: f64,
    /// The open project's log, while this editor is alone or hosting. See
    /// [`log`].
    pub(crate) log: Option<Log>,
    /// The folder of the project [`Self::log`] belongs to.
    log_for: Option<std::path::PathBuf>,
    /// This install's author id, read the first time it is needed.
    author_id: Option<AuthorId>,
    /// The host's answers to this editor's requests about the log, oldest
    /// first, for [`changes::answered`] to take.
    pub(crate) answers: Vec<Answer<Step>>,
    /// The Changes window's state. See [`changes`].
    pub(crate) changes: changes::Changes,
    /// The id of the host's log, from the welcome, while joined.
    host_log: Option<[u8; 16]>,
    /// The host: this machine's world database as the welcome shares it, read
    /// again once a second while hosting with Share the world database on.
    shared_info: Option<String>,
    /// A joined editor: the host's world database, when the host shares it.
    /// Put in `ServerSettings::shared` while this editor uses the host's
    /// server ([`PlaytestServer::Host`]).
    pub(crate) shared_database: Option<vale_mangos::conn::Where>,
    /// The open project's link to a host, when it is a joined editor's copy.
    /// See [`offline`].
    pub(crate) link: Option<offline::Link>,
    /// How many edits the open copy holds offline.
    pub(crate) pending: usize,
    /// When the link's number was last written, while joined.
    link_at: f64,
    /// The open project's log could not be opened, so it is not tried again
    /// until another project opens.
    log_failed: bool,
    /// The address the last join was made to, with its port, and the
    /// project's name on the host, from the manifest: what a link records.
    join_address: Option<String>,
    host_project: Option<String>,
    /// Every claim held, as the host last sent them; alone, the claims of
    /// the log's unsubmitted edits ([`log_holds`]). See *Claims*.
    pub(crate) holds: Vec<Hold>,
    /// When the claims were last read from the log, alone.
    holds_at: f64,
    /// A depot, which has no camera or pointer to say where it is, so it
    /// sends no presence and joined editors do not go to it. See [`depot`].
    pub(crate) headless: bool,
    /// A joined editor waits for every author's edits in no changelist,
    /// which it applies before the edits held since the welcome. See
    /// *Joining*.
    open_wanted: bool,
    /// The host's last edit at the welcome, which the fetched files are as
    /// of. See [`offline`]'s *The copy of the host's log*.
    welcome_seq: vale_collab::Seq,
    /// When the copy of the host's log was last asked for, and whether an
    /// answer is awaited.
    pub(crate) tail_at: f64,
    pub(crate) tail_asked: bool,
    /// A copy that took over from its host: the edits it kept offline, to
    /// apply and log once the project's log is open. See [`offline`]'s
    /// *Taking over*.
    pub(crate) taken_over: Option<Vec<Step>>,
}

impl Collab {
    /// Whether this editor is joined to another's session, and so may not
    /// save, publish or apply: the host does those.
    pub(crate) fn is_joined(&self) -> bool {
        matches!(self.state, State::Connecting { .. } | State::Fetching { .. } | State::Joined { .. })
    }

    /// Whether an editor of `author` is in the session this editor hosts.
    pub(crate) fn connected(&self, author: &AuthorId) -> bool {
        match &self.session {
            Some(Session::Hosting(hosting)) => {
                hosting.users().into_iter().any(|(user, _)| hosting.author_of(user).is_some_and(|held| held.id == *author))
            }
            _ => false,
        }
    }

    /// What this editor may do in the session. See [`Role`].
    pub(crate) fn role(&self) -> Role {
        match self.state {
            State::Hosting { .. } => Role::Hosting,
            _ if self.is_joined() => Role::Joined,
            _ => Role::Alone,
        }
    }

    /// The host's name, when joined.
    pub(crate) fn host_name(&self) -> Option<&str> {
        self.people.get(&0).map(|person| person.name.as_str())
    }

    /// The names of the other editors whose edits to any of these files are not
    /// saved yet, and so are lost when the files are deleted.
    pub(crate) fn unsaved_by(&self, vpaths: &[String]) -> Vec<String> {
        let mut users = BTreeSet::new();
        for vpath in vpaths {
            if let Some(by) = self.edited.get(&vpath.to_ascii_lowercase()) {
                users.extend(by.iter().copied().filter(|user| Some(*user) != self.me));
            }
        }
        users.iter().filter_map(|user| self.people.get(user)).map(|person| person.name.clone()).collect()
    }

    /// Whether this editor may write to its server. See [`ServerGate`].
    pub(crate) fn server_gate(&self) -> ServerGate {
        let role = self.role();
        let host = self.host_name().unwrap_or("the host");
        let refused = match (&self.state, self.playtest_server) {
            (State::Connecting { .. } | State::Fetching { .. }, _) => {
                Some("Not available while joining a shared session.".to_string())
            }
            (State::Joined { .. }, PlaytestServer::Host) => Some(match self.shared_database.is_some() {
                true => format!(
                    "This editor reads {host}'s world database and playtests on {host}'s server, and only {host} applies changes to that server. To apply changes to this machine's server, set \"Server\" in the Session popover to \"This machine's server\"."
                ),
                false => format!(
                    "This editor playtests on {host}'s server, and only {host} applies changes to that server. To apply changes to this machine's server, set \"Server\" in the Session popover to \"This machine's server\"."
                ),
            }),
            (State::Joined { .. }, PlaytestServer::Local) if self.shares_the_hosts_database() => Some(format!(
                "This machine's world database, {}, is the same world database as {host}'s, and only {host} applies changes to that database. Two editors on one machine use one world database when their Edit\\server.txt files name the same mangosd.conf. To apply changes from this editor, set another world database in Settings.",
                self.own_line.as_deref().unwrap_or("the one in Edit\\server.txt")
            )),
            _ => None,
        };
        let host = role.joined().then(|| host.to_string());
        ServerGate { role, refused, host, copy: self.link.is_some() }
    }

    /// Whether this machine's world database is the same database as the host's.
    fn shares_the_hosts_database(&self) -> bool {
        self.own_database.is_some() && self.own_database == self.host_database
    }

    /// The host's address, for a playtest on the host's server.
    pub(crate) fn host_address(&self) -> Option<&str> {
        self.host_address.as_deref()
    }

    /// Whether the open project is a copy of a host's, with no session: its
    /// edits are kept as pending. See [`offline`].
    pub(crate) fn offline(&self) -> bool {
        self.link.is_some() && self.session.is_none() && self.connecting.is_none()
    }

    /// The claims on tiles of the open map, by coordinate, and whether this
    /// editor can check tiles out. See [`MapClaims`].
    pub(crate) fn map_claims(&self, map: &str) -> MapClaims {
        let tiles = self
            .holds
            .iter()
            .filter_map(|held| match &held.claim {
                Claim::Tile(tile) if tile.map.eq_ignore_ascii_case(map) => {
                    let mine = Some(held.author.id) == self.author_id;
                    let who = match mine {
                        true => "this editor".to_string(),
                        false => held.author.name.clone(),
                    };
                    Some((
                        (tile.x, tile.y),
                        TileClaim {
                            author: held.author.id,
                            who,
                            mine,
                            checked_out: held.checked_out,
                            edited: held.edited,
                            colour: self.author_colour(&held.author, mine),
                        },
                    ))
                }
                _ => None,
            })
            .collect();
        MapClaims {
            tiles,
            can_check_out: self.session.is_some() && matches!(self.state, State::Hosting { .. } | State::Joined { .. }),
            map: map.to_string(),
        }
    }

    /// An author's colour: the colour of the person of that name in the
    /// session, so the map window and the people list agree, or for an author
    /// who is not connected, a colour taken from the author id.
    fn author_colour(&self, author: &Author, mine: bool) -> [u8; 3] {
        let user = match mine {
            true => self.me,
            false => self.people.iter().find(|(_, person)| person.name == author.name).map(|(user, _)| *user),
        };
        let id = author.id;
        colour(user.unwrap_or_else(|| u32::from_le_bytes([id[0], id[1], id[2], id[3]])))
    }

    /// The first claim on `groups` that an author other than this editor's
    /// holds. See *Claims*.
    pub(crate) fn held_by_another(&self, groups: &[Group]) -> Option<&Hold> {
        let claims: Vec<Claim> = groups.iter().filter_map(Claim::of).flat_map(|claim| claim.covering()).collect();
        self.holds.iter().find(|held| Some(held.author.id) != self.author_id && claims.contains(&held.claim))
    }

    /// Whether `author` is this editor's author.
    pub(crate) fn is_me(&self, author: &AuthorId) -> bool {
        Some(*author) == self.author_id
    }

    /// How many of this editor's edits are in no changelist. Joined, the
    /// count of steps sent since the last Submit, which the host does not
    /// report; offline, the pending edits.
    pub(crate) fn open_edits(&self) -> usize {
        if self.is_joined() {
            return self.changes.unsubmitted as usize;
        }
        if self.offline() {
            return self.pending;
        }
        match (self.log.as_ref(), self.author_id.as_ref()) {
            (Some(log), Some(id)) => log.open_count(id),
            _ => 0,
        }
    }

    /// What a joined editor's Submit closes, for the Save button's label.
    /// See [`ToSubmit`].
    pub(crate) fn to_submit(&self) -> ToSubmit {
        let mut tiles = BTreeSet::new();
        let mut tables = BTreeSet::new();
        let mut rows = false;
        for hold in self.holds.iter().filter(|hold| hold.edited && self.is_me(&hold.author.id)) {
            match &hold.claim {
                Claim::Tile(tile) => {
                    tiles.insert((tile.map.to_ascii_lowercase(), tile.x, tile.y));
                }
                Claim::Table(name) | Claim::Record { table: name, .. } => {
                    tables.insert(name.clone());
                }
                Claim::Row { .. } => rows = true,
            }
        }
        ToSubmit { edits: self.changes.unsubmitted as usize, tiles: tiles.len(), tables: tables.len(), rows }
    }

    /// This editor's author: the install's id and the form's name. See
    /// [`log`]'s *The author*.
    pub(crate) fn author(&mut self, install: &str) -> Author {
        let id = *self.author_id.get_or_insert_with(|| log::author_id(install));
        log::author(id, &self.form.name)
    }
}

/// What a joined editor's Submit closes. `edits` is the count the Changes
/// window shows: the edits sent since the last Submit. The tiles, client
/// tables and server rows are the claims of this editor's unsubmitted edits,
/// as the host last sent them, so they can trail an edit by a moment. The
/// session's own unsaved tiles are not used: a joined copy is never saved,
/// and they also hold every tile another editor's edit changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ToSubmit {
    pub(crate) edits: usize,
    pub(crate) tiles: usize,
    pub(crate) tables: usize,
    pub(crate) rows: bool,
}

/// A fingerprint of a world database's location: its host, port and database
/// name, hashed, so it can be compared between editors without sending the
/// location. A loopback host (`127.0.0.1`, `localhost`, `::1`) is replaced by
/// this machine's network address first. The host's own `127.0.0.1` and a
/// joined editor's `192.168.1.20` for the same machine then give the same
/// fingerprint. A machine with no network address uses its computer name
/// instead, so the loopback databases of two such machines differ. A machine
/// name and an address for one machine give different fingerprints.
pub(crate) fn database_fingerprint(at: &vale_mangos::conn::Where) -> [u8; 32] {
    let host = at.host.trim().to_ascii_lowercase();
    let loopback = matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1" | ".");
    let host = match loopback {
        true => match lan_address() {
            Some(ip) => ip.to_string(),
            None => std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).map_or(host, |name| name.to_ascii_lowercase()),
        },
        false => host,
    };
    let text = format!("{host}\n{}\n{}", at.port(), at.database.trim().to_ascii_lowercase());
    *blake3::hash(text.as_bytes()).as_bytes()
}

/// The host's shared world database as a joined editor reaches it: a
/// loopback host is the host's machine, so it becomes the address this editor
/// joined.
fn shared_location(info: &str, host: Option<&str>) -> Option<vale_mangos::conn::Where> {
    let mut at = vale_mangos::conn::Where::parse(info)?;
    let loopback = matches!(at.host.trim().to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "::1" | ".");
    if let (true, Some(host)) = (loopback, host) {
        at.host = host.to_string();
    }
    Some(at)
}

/// The host part of an address typed into the Join form: `192.168.1.20` for
/// `192.168.1.20:7878`, and the whole of an address with no port.
fn host_part(address: &str) -> String {
    match address.parse::<std::net::SocketAddr>() {
        Ok(at) => at.ip().to_string(),
        Err(_) => address.rsplit_once(':').map_or(address, |(host, _)| host).to_string(),
    }
}

/// The colour a person is drawn in, by their number: the host first.
pub(crate) fn colour(user: UserId) -> [u8; 3] {
    const PEOPLE: [[u8; 3]; 8] = [
        [0xB4, 0x8C, 0xF2],
        [0x4F, 0xD1, 0xC5],
        [0xF2, 0x8C, 0xC2],
        [0xB8, 0xE0, 0x6A],
        [0xF5, 0xB9, 0x8A],
        [0x9A, 0xA8, 0xFF],
        [0x8E, 0xF0, 0xB5],
        [0xFF, 0x8F, 0xA3],
    ];
    PEOPLE[user as usize % PEOPLE.len()]
}

/// The address other machines on this network reach this one at, found by
/// asking the system which interface it would send from. Nothing is sent.
pub(crate) fn lan_address() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// A fingerprint of the game's archives: each `.MPQ` in the data folder by
/// name and size. Two machines holding different archives would apply an
/// edit to different bytes.
fn archives(assets: &GameAssets) -> [u8; 32] {
    let mut found: Vec<(String, u64)> = std::fs::read_dir(&assets.gamedata_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let size = entry.metadata().ok()?.len();
            name.ends_with(".mpq").then_some((name, size))
        })
        .collect();
    found.sort();
    let mut hasher = blake3::Hasher::new();
    for (name, size) in found {
        hasher.update(name.as_bytes());
        hasher.update(&size.to_le_bytes());
    }
    *hasher.finalize().as_bytes()
}

/// `--host` and `--join`: fill the form and ask, once the map has been open
/// for two seconds, so the host's ground is loaded before anyone joins.
fn on_the_command_line(
    args: Res<crate::args::Args>,
    mut collab: ResMut<Collab>,
    session: Option<Res<EditSession>>,
    time: Res<Time>,
    mut done: Local<bool>,
) {
    if *done || session.is_none() || time.elapsed_secs_f64() < 2.0 {
        return;
    }
    *done = true;
    let flags = &args.server;
    if let Some(name) = &flags.session_name {
        collab.form.name = name.clone();
    }
    if let Some(password) = &flags.session_password {
        collab.form.password = password.clone();
    }
    if let Some(port) = flags.host {
        collab.form.mode = Mode::Host;
        collab.form.port = port.to_string();
        collab.ask = Some(Ask::Host);
    } else if let Some(address) = &flags.join {
        collab.form.mode = Mode::Join;
        collab.form.address = address.clone();
        collab.ask = Some(Ask::Join);
    }
}

/// The frame's work: what the windows asked, the steps this editor made, the
/// messages from the session, and this editor's presence.
#[allow(clippy::too_many_arguments)]
fn run(
    mut collab: ResMut<Collab>,
    mut session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    mut camera: ResMut<crate::camera::EditorCamera>,
    cursor: Res<crate::pick::Cursor>,
    tool: Res<crate::tools::Tool>,
    time: Res<Time>,
    mut fly: MessageWriter<crate::camera::FlyTo>,
    // Told which project files another editor changed, so the Project Files
    // list and its pictures are read again.
    mut project_files: ResMut<crate::import::assets::Assets>,
    // This machine's world database, to compare with the host's database
    // and to share; and where a joined editor is given the host's.
    mut server: ResMut<crate::server::settings::ServerSettings>,
    // Given the host's realm address while this editor playtests on the
    // host's server.
    mut login: ResMut<crate::playtest::Login>,
) {
    let Some(session) = session.as_mut() else { return };
    let collab = &mut *collab;
    let now = time.elapsed_secs_f64();
    collab.now = now;
    if collab.author_id.is_none() {
        collab.author_id = Some(log::author_id(&assets.root));
    }
    keep_the_log(collab, session, &assets);
    keep_the_numbers(collab, session);
    // Alone, the claims are the log's, read once a second.
    if collab.session.is_none() && collab.connecting.is_none() && now - collab.holds_at >= 1.0 {
        collab.holds_at = now;
        collab.holds = collab.log.as_ref().map(log_holds).unwrap_or_default();
    }
    // `ServerSettings::resolve` reads `mangosd.conf`, so it is read once a
    // second rather than every frame.
    if now - collab.database_at >= 1.0 {
        collab.database_at = now;
        let own = server.resolve_own().map(|(at, _)| at);
        collab.own_database = own.as_ref().map(database_fingerprint);
        collab.own_line = own.as_ref().map(|at| at.line());
        collab.shared_info = match (&collab.state, collab.form.share_database) {
            (State::Hosting { .. }, true) => own.map(|at| format!("{};{};{};{};{}", at.host, at.port(), at.user, at.password, at.database)),
            _ => None,
        };
    }
    // The host's database is read only while the host's server is used.
    let reads = match collab.playtest_server {
        PlaytestServer::Host => collab.shared_database.clone(),
        PlaytestServer::Local => None,
    };
    if server.shared != reads {
        server.shared = reads;
    }
    let realm = match (&collab.state, collab.playtest_server) {
        (State::Joined { .. }, PlaytestServer::Host) => collab.host_address.clone(),
        _ => None,
    };
    if login.realm != realm {
        login.realm = realm;
    }
    collab.recent.retain(|_, at| now - *at < people::RECENT);
    // An edit saved since is not lost by a delete.
    collab.edited.retain(|vpath, _| still_unsaved(session, vpath));
    project_files.changed.append(&mut collab.changed);
    if let Some(ask) = collab.ask.take() {
        carry_out(collab, session, &assets, &mut camera, &mut fly, ask);
    }
    take_the_connection(collab, session);
    // With no session, this editor's steps go to the project's log, or to an
    // offline copy's pending edits. While a join is being opened they wait
    // in the journals.
    if collab.session.is_none() {
        match (collab.connecting.is_some(), collab.link.is_some()) {
            (true, _) => {}
            (false, true) => keep_pending(collab, session, &assets),
            (false, false) => log_own_steps(collab, session, &assets),
        }
        after_the_steps(collab, session, &assets);
        return;
    }
    keep_the_link(collab, session, now);
    offline::keep_the_log_copy(collab, session, now);
    if let State::Hosting { project, .. } = &collab.state {
        if *project != session.project.name {
            let reason = format!("the session ended: {project} is no longer the open project");
            end(collab, session, &assets, reason);
            return;
        }
    }
    share_own_steps(collab, session, &assets, now);

    // The part of a held stroke not sent yet, which a remote edit over it
    // reverts. See `vale_collab::peer`.
    let open = session.history.segment().cloned().map(Step::edit);
    let actions = collab.session.as_mut().map(|running| running.poll(open.as_ref())).unwrap_or_default();
    let mut admits = Vec::new();
    for action in actions {
        match action {
            Action::Admit { user, name } => admits.push((user, name)),
            action => act(collab, session, &assets, &mut camera, &mut fly, action),
        }
        if collab.session.is_none() {
            return;
        }
    }
    // After every other action of the frame, so the files the manifest lists
    // hold every edit accepted before the welcome's sequence.
    for (user, name) in admits {
        admit(collab, session, user, &name);
    }
    changes::answered(collab, session, &assets);
    after_the_steps(collab, session, &assets);

    if !collab.headless && now - collab.presence_at >= PRESENCE_EVERY {
        collab.presence_at = now;
        let presence = Presence {
            map: session.map.clone(),
            camera: camera.target.to_array(),
            cursor: cursor.surface.map(|at| at.to_array()),
            tool: tool.name().to_string(),
        };
        if let Some(me) = collab.me {
            if let Some(person) = collab.people.get_mut(&me) {
                person.presence = Some(presence.clone());
            }
        }
        if let Some(running) = collab.session.as_mut() {
            running.presence(presence);
        }
    }
    project_files.changed.append(&mut collab.changed);
}

/// Keep the journals on and the open project's log open. Both journals are
/// on for every project, since the log reads them alone as well as in a
/// session. A project switch replaces the undo stack and the project, whose
/// journals start off. A joined editor holds no log; the host writes it.
fn keep_the_log(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets) {
    if !session.history.journaling() {
        session.history.journal(true);
    }
    if !session.project.journaling() {
        session.project.journal(true);
    }
    if collab.log_for.as_ref() != Some(&session.project.root) {
        // A project switch saves the project it leaves
        // (`EditSession::switch_to`), and that save submits into the log of
        // the project it saved, which is still the one held here.
        if collab.log.is_some() && session.saved_everything == Some(true) && !collab.is_joined() && collab.link.is_none() {
            changes::close_on_save(collab, session, assets);
        }
        session.saved_tiles.clear();
        session.saved_tables.clear();
        session.rows.saved = false;
        session.saved_everything = None;
        collab.log_for = Some(session.project.root.clone());
        collab.link = offline::Link::read(&session.project);
        collab.pending = whole_edits(&offline::read(&session.project));
        // A copy numbers from the blocks it kept, any other project from its
        // allocator. See `crate::session::numbers`.
        match collab.link.is_some() {
            true => session.numbers.use_blocks(offline::read_blocks(&session.project), false),
            false => session.numbers.use_allocator(log::read_numbers(&session.project)),
        }
        collab.log = None;
        collab.log_failed = false;
        collab.changes.recovered.clear();
        collab.holds_at = f64::NEG_INFINITY;
    }
    // A joined editor's history is the host's, and so are its saves; an
    // offline copy's history is the host's too.
    if collab.is_joined() || collab.link.is_some() {
        collab.log = None;
        return;
    }
    if collab.log.is_some() || collab.log_failed {
        return;
    }
    match log::open(&session.project) {
        Ok(mut log) => {
            // Strokes of closed changelists stored in parts are folded once
            // a session. See `vale_collab::log`'s *Compaction*.
            match log.compact(log::fold) {
                Ok(0) => {}
                Ok(n) => info!("collab: folded {n} strokes of {}'s log", session.project.name),
                Err(e) => warn!("collab: the log of {} could not be compacted: {e}", session.project.name),
            }
            let fetched = offline::Fetched::read(&session.project);
            collab.changes.recovered = changes::recovered(&log, &collab.author_id.unwrap_or_default(), fetched.as_ref());
            if !collab.changes.recovered.is_empty() {
                let n = collab.changes.recovered.len();
                session.status = format!(
                    "{} in the project's log {} not in its files; restore or discard {} in the Changes window",
                    counted(n, "edit", "edits"),
                    if n == 1 { "is" } else { "are" },
                    if n == 1 { "it" } else { "them" },
                );
            }
            collab.log = Some(log);
            if let Some(pending) = collab.taken_over.take() {
                changes::took_over(collab, session, assets, pending);
            }
        }
        Err(e) => {
            collab.log_failed = true;
            session.status = format!("the project's log could not be opened, so edits are not logged: {e}");
            warn!("collab: log for {}: {e}", session.project.name);
        }
    }
}

/// The numbers of new records: whether the copy is connected, the blocks to
/// ask for, the files they are kept in, and a refusal on the status line.
/// See `crate::session::numbers`.
fn keep_the_numbers(collab: &mut Collab, session: &mut EditSession) {
    use vale_edit::numbers::{dbc, UNIQUE_ID};
    let joined = matches!(collab.state, State::Joined { .. }) && collab.link.is_some();
    session.numbers.set_connected(joined);
    if joined {
        // The types used most are asked for once the copy is open, so the
        // first placement or row does not wait for a block.
        if !session.numbers.has(UNIQUE_ID) {
            let floor = crate::tools::placements::group::next_minted(session);
            session.numbers.want(UNIQUE_ID, u64::from(floor));
        }
        let names: Vec<String> = session.tables.keys().filter(|name| !session.numbers.has(&dbc(name))).cloned().collect();
        for name in names {
            if let Some(floor) = session.tables.get(&name).and_then(|table| table.max_id().checked_add(1)) {
                session.numbers.want(&dbc(&name), u64::from(floor));
            }
        }
        if let Some(running) = collab.session.as_mut() {
            for (kind, floor, size) in session.numbers.take_wanted() {
                running.ask_for_ids(&kind, floor, size);
            }
        }
    }
    if std::mem::take(&mut session.numbers.blocks_changed) && collab.link.is_some() {
        if let Err(e) = offline::write_blocks(&session.project, &session.numbers.blocks) {
            warn!("collab: the blocks of new numbers could not be kept: {e}");
        }
    }
    let text = session.numbers.allocator().and_then(|shared| {
        let mut allocator = shared.lock().expect("numbers");
        allocator.take_changed().then(|| allocator.text())
    });
    if let Some(text) = text {
        if let Err(e) = log::write_numbers(&session.project, &text) {
            warn!("collab: the allocator of new numbers could not be kept: {e}");
        }
    }
    if let Some(why) = session.numbers.refused.take() {
        session.status = why;
    }
}

/// After a save made this frame, once this frame's edits are logged: alone
/// or hosting, a save that left nothing of this editor's unsaved closes its
/// changelist with the description typed in the Changes window. A save that
/// left this editor's own unsubmitted edits out (`EditSession::exclude_once`)
/// wrote none of them, so their subjects stay unsaved. See *What a save
/// writes*.
fn after_a_save(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets) {
    // A clear deletes the history with the files, so the log and the link
    // are read again from what is left.
    if std::mem::take(&mut session.cleared) {
        collab.log = None;
        collab.log_for = None;
        collab.changes.recovered.clear();
        collab.changes.history = None;
        collab.changes.detail = None;
    }
    let tiles = std::mem::take(&mut session.saved_tiles);
    let tables = std::mem::take(&mut session.saved_tables);
    let rows = std::mem::take(&mut session.rows.saved);
    let full = session.saved_everything.take();
    if tiles.is_empty() && tables.is_empty() && !rows && full.is_none() {
        return;
    }
    if full == Some(false) {
        let groups = changes::own_open_groups(collab);
        changes::mark_unsaved(session, &groups);
        return;
    }
    let everything = session.unsaved.is_empty() && session.unsaved_tables.is_empty() && !session.rows.unsaved();
    if everything && collab.link.is_none() && !collab.is_joined() {
        changes::close_on_save(collab, session, assets);
        // The files hold every submitted changelist now. See `offline`'s
        // *Taking over*.
        if collab.changes.recovered.is_empty() {
            offline::Fetched::remove(&session.project);
        }
    }
}

/// Once this frame's edits are logged, sent or kept: the discards asked for,
/// then what a save does. Both read the edits made so far.
fn after_the_steps(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets) {
    for key in std::mem::take(&mut session.discard_asked) {
        changes::discard(collab, session, assets, (key != "all").then_some(key));
    }
    after_a_save(collab, session, assets);
    changes::withdraw_undone(collab, session, assets);
}

/// Write the link's number while joined, every few seconds when it moved, so
/// a copy left by a crash is offline from a recent number.
fn keep_the_link(collab: &mut Collab, session: &EditSession, now: f64) {
    if !matches!(collab.state, State::Joined { .. }) || now - collab.link_at < 5.0 {
        return;
    }
    collab.link_at = now;
    let Some(seq) = collab.session.as_ref().map(|running| running.me().seq()) else { return };
    if let Some(link) = collab.link.as_mut() {
        if link.synced != seq {
            link.synced = seq;
            let _ = link.write(&session.project);
        }
    }
}

/// Whether the tile or table file at `vpath` has edits the session has not
/// saved. A tile of another map than the open one is written as it is
/// changed, so it never has.
fn still_unsaved(session: &EditSession, vpath: &str) -> bool {
    if let Some(key) = vale_edit::TileKey::from_vpath(vpath) {
        return key.map.eq_ignore_ascii_case(&session.map) && session.unsaved.contains(&(key.x, key.y));
    }
    session
        .unsaved_tables
        .iter()
        .any(|name| vale_assets::tables::dbc::dbc_path(name).eq_ignore_ascii_case(vpath))
}

/// Carry out what the Session popover or the Changes window asked.
fn carry_out(
    collab: &mut Collab,
    session: &mut EditSession,
    assets: &GameAssets,
    camera: &mut crate::camera::EditorCamera,
    fly: &mut MessageWriter<crate::camera::FlyTo>,
    ask: Ask,
) {
    let name = collab.form.name.trim().to_string();
    match ask {
        Ask::Host if collab.session.is_none() && collab.link.is_some() => {
            collab.state = State::Ended(format!(
                "{} is a copy of another host's project and cannot be hosted. Only a project that is not a copy can be hosted.",
                session.project.name
            ));
        }
        Ask::Host if collab.session.is_none() => {
            let Ok(port) = collab.form.port.trim().parse::<u16>() else {
                collab.state = State::Ended(format!("{} is not a port", collab.form.port.trim()));
                return;
            };
            let author = collab.author(&assets.root);
            match Hosting::start(&format!("0.0.0.0:{port}"), &author, &collab.form.password, BUILD, archives(assets)) {
                Ok(mut hosting) => {
                    // The session's numbers continue the log's, so a number
                    // names the same edit in every session of this project.
                    if let Some(log) = collab.log.as_ref() {
                        hosting.resume(log.id(), log.last(), log.touches(), log.pending(), log.names());
                    }
                    hosting.resume_checkouts(read_checkouts(&session.project));
                    // Joined editors' blocks come from the allocator this
                    // editor numbers from.
                    if let Some(ids) = session.numbers.allocator() {
                        hosting.share_ids(ids.clone());
                    }
                    collab.holds = hosting.holds();
                    // So the shared database is read before anyone joins.
                    collab.database_at = f64::NEG_INFINITY;
                    let addr = std::net::SocketAddr::new(lan_address().unwrap_or(hosting.addr().ip()), hosting.addr().port());
                    collab.people = hosting.users().into_iter().map(|(user, name)| (user, Person { name, presence: None })).collect();
                    collab.session = Some(Session::Hosting(hosting));
                    collab.me = Some(0);
                    collab.state = State::Hosting { project: session.project.name.clone(), addr };
                    collab.sent = 0;
                    collab.received = 0;
                    session.status = format!("hosting {} on {addr}", session.project.name);
                    info!("collab: hosting {} on {addr}", session.project.name);
                }
                Err(e) => collab.state = State::Ended(format!("could not host on port {port}: {e}")),
            }
        }
        Ask::Join if collab.session.is_none() && collab.connecting.is_none() => {
            let mut address = collab.form.address.trim().to_string();
            if address.is_empty() {
                collab.state = State::Ended("no host address in the Join form".to_string());
                return;
            }
            if !address.contains(':') {
                address = format!("{address}:{PORT}");
            }
            let (password, archives) = (collab.form.password.clone(), archives(assets));
            let author = collab.author(&assets.root);
            let (done, connecting) = std::sync::mpsc::channel();
            let to = address.clone();
            std::thread::spawn(move || {
                let _ = done.send(Joined::connect(&to, &author, &password, BUILD, archives));
            });
            collab.connecting = Some(std::sync::Mutex::new(connecting));
            collab.host_address = Some(host_part(&address));
            collab.join_address = Some(address.clone());
            collab.state = State::Connecting { to: address };
        }
        Ask::Leave => {
            let reason = match collab.state {
                State::Hosting { .. } => "the session was ended",
                _ => "left the session",
            };
            end(collab, session, assets, reason.to_string());
            collab.state = State::Off;
        }
        Ask::GoOffline if matches!(collab.state, State::Joined { .. }) => {
            let host = collab.host_name().unwrap_or("the host").to_string();
            go_offline(collab, session, format!("left {host}'s session; the copy is open offline"));
        }
        Ask::Reconnect => {
            if let Some(link) = collab.link.clone() {
                collab.form.mode = Mode::Join;
                collab.form.address = link.address;
                carry_out(collab, session, assets, camera, fly, Ask::Join);
            }
        }
        Ask::RestoreRecovered => changes::restore_recovered(collab, session, assets),
        Ask::TakeBack(author) => changes::take_back(collab, session, assets, author),
        Ask::Detail(number) => changes::show_detail(collab, session, number),
        Ask::CheckOut(claims) => {
            if let Some(running) = collab.session.as_mut() {
                running.check_out(claims);
            }
        }
        Ask::Release(claims) => {
            if let Some(running) = collab.session.as_mut() {
                running.release(claims, false);
            }
        }
        Ask::Break(claims) => {
            if let Some(running) = collab.session.as_mut() {
                running.release(claims, true);
            }
        }
        Ask::TakeOver if collab.offline() => offline::take_over(collab, session, assets),
        Ask::Unlink if collab.offline() && collab.pending == 0 => {
            match offline::Link::remove(&session.project).and_then(|()| offline::remove_log_copy(&session.project)) {
                Ok(()) => {
                    collab.link = None;
                    collab.log_for = None;
                    session.status = format!("{} is a project of its own now", session.project.name);
                }
                Err(e) => session.status = format!("the link could not be removed: {e}"),
            }
        }
        Ask::GoTo(user) => {
            let Some(presence) = collab.people.get(&user).and_then(|person| person.presence.clone()) else {
                return;
            };
            if !presence.map.eq_ignore_ascii_case(&session.map) {
                let Some((id, map)) = session.maps.iter().find(|(_, map)| map.eq_ignore_ascii_case(&presence.map)).cloned() else {
                    session.status = format!("{} is on {}, which this editor's archives do not have", collab.people[&user].name, presence.map);
                    return;
                };
                session.switch_map(map, id, camera);
            }
            fly.write(crate::camera::FlyTo::at(Vec3::from(presence.camera)));
        }
        Ask::Remove(user) => {
            if let Some(Session::Hosting(hosting)) = collab.session.as_mut() {
                let actions = hosting.remove(user, "removed from the session by the host");
                for action in actions {
                    act(collab, session, assets, camera, fly, action);
                }
            }
        }
        Ask::Delete(paths) if matches!(collab.state, State::Joined { .. }) => {
            let host = collab.host_name().unwrap_or("the host").to_string();
            session.status = format!("asked {host} to delete {}", count_files(paths.len()));
            if let Some(running) = collab.session.as_mut() {
                running.ask(Step::Delete(paths));
            }
        }
        Ask::Submit => changes::submit(collab, session, assets),
        Ask::Revert(number) => changes::revert(collab, session, assets, number),
        Ask::Reapply(at) => changes::reapply(collab, session, assets, at),
        Ask::KeepCurrent(at) => changes::keep_current(collab, session, at),
        Ask::History => {
            collab.changes.asked = false;
            changes::ask_history(collab);
        }
        Ask::Keep => {
            let Some(request) = collab.requests.pop_front() else { return };
            if let Some(Session::Hosting(hosting)) = collab.session.as_ref() {
                hosting.note(request.user, format!("{name} declined to delete the {}", count_files(request.paths.len())));
            }
            session.status = format!("kept the {} {} asked to delete", count_files(request.paths.len()), request.name);
        }
        _ => {}
    }
}

/// `1 edit` or `n edits`: a count with the word for one or for several.
pub(crate) fn counted(n: usize, one: &str, many: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

/// `1 file` or `n files`.
fn count_files(n: usize) -> String {
    counted(n, "file", "files")
}

/// Take a connection the joining thread opened.
fn take_the_connection(collab: &mut Collab, session: &mut EditSession) {
    let Some(connecting) = collab.connecting.as_ref() else { return };
    let Ok(opened) = connecting.lock().expect("connecting").try_recv() else { return };
    collab.connecting = None;
    match opened {
        Ok(joined) => {
            collab.session = Some(Session::Joined(joined));
            collab.own_project = Some(session.project.name.clone());
            collab.sent = 0;
            collab.received = 0;
        }
        Err(e) => {
            let to = match &collab.state {
                State::Connecting { to } => to.clone(),
                _ => "the host".to_string(),
            };
            collab.state = State::Ended(format!("could not reach {to}: {e}"));
        }
    }
}

/// Send the steps this editor made: the files it wrote or removed first, so a
/// paint that uses an imported texture arrives after the texture, then the
/// undo stack's steps. A joined editor's file step that only the host may
/// make ([`files::joinable`]) is reverted instead.
fn share_own_steps(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, now: f64) {
    let joined = match collab.state {
        State::Joined { .. } => true,
        State::Hosting { .. } => false,
        _ => return,
    };
    if now - collab.segment_at >= SEGMENT_EVERY {
        collab.segment_at = now;
        session.history.close_segment();
    }
    let (steps, too_large) = files::take(session);
    if let Some(first) = too_large.first() {
        let mb = files::STEP_BYTES >> 20;
        session.status = format!("{first} is larger than {mb} MB and was not sent to the other editors");
    }
    for step in steps {
        if joined {
            if let Err(why) = step.check() {
                let _ = files::apply(session, assets, &step.backward());
                collab.changed.extend(step.files.iter().map(|file| file.path.clone()));
                collab.reverted += 1;
                let why = why.split_once(": ").map_or(why.as_str(), |(_, why)| why);
                session.status = format!("{} was undone: {why}", step.what());
                continue;
            }
        }
        if let Some(running) = collab.session.as_mut() {
            running.propose(Step::Files(step));
            collab.sent += 1;
            collab.changes.unsubmitted += 1;
        }
    }
    let mut undone = false;
    for step in session.take_journal() {
        // A claim another author holds is refused by the host; undone here
        // first, so nothing waits on the refusal. See *Claims*.
        let groups = step.change.groups();
        if let Some(held) = collab.held_by_another(&groups) {
            let line = format!("{} was undone: {} {}", step.change.label, held.author.name, held.words());
            session.apply_step(assets, &step.change, !step.forward);
            forget_overlapping(session, &groups);
            collab.reverted += 1;
            session.status = line;
            continue;
        }
        undone |= !step.forward;
        if let Some(running) = collab.session.as_mut() {
            let whole = u64::from(!step.continues);
            running.propose(Step::journaled(step));
            collab.sent += whole;
            collab.changes.unsubmitted += whole;
        }
    }
    // An undo may leave this editor's edits changing nothing. See *Edits
    // that change nothing*.
    if undone && joined {
        if let Some(running) = collab.session.as_mut() {
            running.request(vale_collab::wire::Request::Withdraw);
        }
    }
}

/// Offline: add the steps this editor made to the copy's pending edits, the
/// files first. See [`offline`].
fn keep_pending(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets) {
    let (files, too_large) = files::take(session);
    if let Some(first) = too_large.first() {
        let mb = files::STEP_BYTES >> 20;
        session.status = format!("{first} is larger than {mb} MB and is not kept to send to the host");
    }
    let steps: Vec<Step> = files
        .into_iter()
        .map(Step::Files)
        .chain(session.take_journal().into_iter().map(Step::journaled))
        .collect();
    if steps.is_empty() {
        return;
    }
    let undone = steps.iter().any(|step| matches!(step, Step::Edit { forward: false, .. }));
    match offline::append(&session.project, &steps) {
        Ok(()) => collab.pending += whole_edits(&steps),
        Err(e) => session.status = format!("an edit made offline could not be kept to send to the host: {e}"),
    }
    if undone {
        changes::empty_pending_when_unchanged(collab, session, assets);
    }
}

/// Alone: write the steps this editor made to the project's log, the files
/// first, as a session sends them.
fn log_own_steps(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets) {
    let (files, too_large) = files::take(session);
    if let Some(first) = too_large.first() {
        let mb = files::STEP_BYTES >> 20;
        session.status = format!("{first} is larger than {mb} MB and was not written to the project's log");
    }
    let steps: Vec<Step> = files
        .into_iter()
        .map(Step::Files)
        .chain(session.take_journal().into_iter().map(Step::journaled))
        .collect();
    if steps.is_empty() {
        return;
    }
    let author = collab.author(&assets.root);
    let Some(log) = collab.log.as_mut() else { return };
    // What other authors' unsubmitted edits claim is theirs, as a session's
    // host rules it. See *Claims*.
    let held: Vec<Hold> = log_holds(log).into_iter().filter(|held| held.author.id != author.id).collect();
    for step in steps {
        if let (false, Step::Edit { change, forward, .. }) = (held.is_empty(), &step) {
            let claims: Vec<Claim> = step.groups().iter().filter_map(Claim::of).flat_map(|claim| claim.covering()).collect();
            if let Some(holder) = held.iter().find(|held| claims.contains(&held.claim)) {
                session.apply_step(assets, change, !forward);
                forget_overlapping(session, &step.groups());
                session.status = format!(
                    "{} was undone: {} {}; take them back from History to edit there",
                    change.label,
                    holder.author.name,
                    holder.words()
                );
                continue;
            }
        }
        let seq = log.last() + 1;
        log::append(log, seq, &author, &step, &mut session.status);
    }
}

/// The claims the log's unsubmitted edits hold, by author, as a host
/// resumed from the log holds them (`vale_collab::Host::resume`).
pub(crate) fn log_holds(log: &Log) -> Vec<Hold> {
    let mut out: BTreeMap<Claim, Hold> = BTreeMap::new();
    for (group, author) in log.pending() {
        let Some(claim) = Claim::of(&group) else { continue };
        let name = log.name_of(&author).unwrap_or("another editor").to_string();
        out.entry(claim.clone())
            .or_insert_with(|| Hold { claim, author: Author { id: author, name }, checked_out: false, edited: true });
    }
    out.into_values().collect()
}

/// Drop every entry of this editor's undo stack that touches one of
/// `groups`. See the module comment's *Undo*.
fn forget_overlapping(session: &mut EditSession, groups: &[Group]) -> usize {
    session
        .history
        .forget_where(|change| change.groups().iter().any(|group| groups.binary_search(group).is_ok()))
}

/// The status line for one of this editor's edits that was reverted.
fn reverted_line(collab: &Collab, label: &str, cause: &Cause) -> String {
    match cause {
        Cause::Changed { user } => {
            let name = collab.people.get(user).map_or("another editor", |person| person.name.as_str());
            format!("{label} was undone: {name} made a conflicting edit first")
        }
        Cause::Refused(reason) => format!("{label} was undone: the host refused it ({reason})"),
    }
}

/// One action from the session.
fn act(
    collab: &mut Collab,
    session: &mut EditSession,
    assets: &GameAssets,
    camera: &mut crate::camera::EditorCamera,
    fly: &mut MessageWriter<crate::camera::FlyTo>,
    action: Action<Step>,
) {
    match action {
        Action::Welcome { user, seq, log, users, database } => {
            collab.host_log = Some(log);
            collab.welcome_seq = seq;
            collab.tail_asked = false;
            collab.me = Some(user);
            collab.changes.history = None;
            collab.changes.asked = false;
            collab.people = users.into_iter().map(|(user, name)| (user, Person { name, presence: None })).collect();
            collab.host_database = database.as_ref().map(|database| database.fingerprint);
            collab.shared_database = database.and_then(|database| database.shared).and_then(|info| shared_location(&info, collab.host_address.as_deref()));
            // With the host's database shared, the host's server. Otherwise
            // this machine's, unless it has no world database of its own. A
            // choice made in the Session popover is kept.
            if !collab.server_chosen {
                collab.playtest_server = match (&collab.shared_database, collab.own_database) {
                    (Some(_), _) | (None, None) => PlaytestServer::Host,
                    (None, Some(_)) => PlaytestServer::Local,
                };
            }
        }
        Action::Refused(reason) | Action::Closed(reason) => match collab.state {
            State::Joined { .. } => {
                go_offline(collab, session, format!("the session ended: {reason}. The copy is open offline; Reconnect is in the Session popover"));
            }
            _ => end(collab, session, assets, format!("could not join: {reason}")),
        },
        Action::Apply { user, payload } => {
            collab.received += u64::from(!payload.continues());
            match collab.state {
                State::Joined { .. } if collab.open_wanted => collab.held.push((user, payload)),
                State::Joined { .. } | State::Hosting { .. } => apply_remote(collab, session, assets, user, &payload),
                _ => collab.held.push((user, payload)),
            }
        }
        Action::Revert { payload, cause } => match &payload {
            Step::Edit { change, forward, .. } => {
                session.apply_step(assets, change, !forward);
                forget_overlapping(session, &payload.groups());
                collab.reverted += 1;
                // The host refused it, so it is not one of the unsubmitted.
                collab.changes.unsubmitted = collab.changes.unsubmitted.saturating_sub(u64::from(!payload.continues()));
                session.status = reverted_line(collab, &change.label, &cause);
            }
            Step::Files(step) => {
                let _ = files::apply(session, assets, &step.backward());
                collab.changed.extend(step.files.iter().map(|file| file.path.clone()));
                collab.reverted += 1;
                session.status = reverted_line(collab, &step.what(), &cause);
            }
            Step::Delete(_) => {}
        },
        Action::Reapply { payload } => {
            if let Step::Files(step) = &payload {
                if let Err(e) = files::apply(session, assets, step) {
                    session.status = format!("{} could not be written again: {e}", step.what());
                }
                collab.changed.extend(step.files.iter().map(|file| file.path.clone()));
            }
        }
        Action::Note(line) => session.status = line,
        Action::Ask { user, payload } => {
            let Step::Delete(paths) = payload else { return };
            let name = collab.people.get(&user).map_or_else(|| format!("Editor {user}"), |person| person.name.clone());
            // Only the files the Project Files list shows, which are the ones
            // a joined editor's Delete can name.
            let paths: Vec<String> = paths
                .into_iter()
                .filter(|path| vale_edit::project::Project::is_game_data(path) && session.project.has(path))
                .collect();
            if paths.is_empty() {
                if let Some(Session::Hosting(hosting)) = collab.session.as_ref() {
                    hosting.note(user, format!("none of the files asked to be deleted is in {}", session.project.name));
                }
                return;
            }
            session.status = format!("{name} asks to delete {}", count_files(paths.len()));
            collab.requests.push_back(Request { user, name, paths });
        }
        Action::CancelOpen { cause } => {
            if let Some(segment) = session.history.drop_segment() {
                session.apply_step(assets, &segment, false);
                forget_overlapping(session, &segment.groups());
                collab.reverted += 1;
                session.status = reverted_line(collab, &segment.label, &cause);
            }
        }
        Action::Presence { user, presence } => {
            if let Some(person) = collab.people.get_mut(&user) {
                person.presence = Some(presence);
            }
        }
        Action::Joined { user, name } => {
            session.status = format!("{name} joined the session");
            collab.people.insert(user, Person { name, presence: None });
        }
        Action::Left { user } => {
            collab.requests.retain(|request| request.user != user);
            if let Some(person) = collab.people.remove(&user) {
                session.status = format!("{} left the session", person.name);
            }
        }
        Action::Manifest { project, files } => {
            collab.host_project = Some(project.clone());
            match fetch::Fetch::start(assets, &project, files) {
                Ok((fetch, wanted)) => {
                    collab.state = fetch.state();
                    collab.fetch = Some(fetch);
                    if let Some(running) = collab.session.as_mut() {
                        running.fetch(wanted);
                    }
                }
                Err(e) => end(collab, session, assets, format!("could not keep a copy of {project}: {e}")),
            }
        }
        Action::File { path, offset, total, bytes } => {
            if let Some(fetch) = collab.fetch.as_mut() {
                fetch.piece(&path, offset, total, bytes);
                collab.state = fetch.state();
            }
        }
        Action::FilesDone { missing } => {
            let Some(fetch) = collab.fetch.take() else { return };
            match fetch.finish(missing) {
                Ok(project) => opened_the_copy(collab, session, assets, camera, fly, project),
                Err(e) => end(collab, session, assets, e),
            }
        }
        Action::Fetch { user, paths } => {
            // Only what a manifest can list: a joined editor asks by path,
            // and the host's `sql\` folder is not sent.
            let files = paths
                .into_iter()
                .filter(|path| fetch::listed(path))
                .filter_map(|path| {
                    let on_disk = session.project.path_for(&path)?;
                    Some((path, on_disk))
                })
                .collect();
            if let Some(Session::Hosting(hosting)) = collab.session.as_ref() {
                hosting.serve(user, files);
            }
        }
        Action::Admit { user, name } => admit(collab, session, user, &name),
        Action::Log { seq, user, author, mut payload } => {
            // A joined editor's file step arrives without its bytes before;
            // they are this editor's, read before the step is applied.
            if let (true, Step::Files(step)) = (Some(user) != collab.me, &mut payload) {
                for file in &mut step.files {
                    file.before = session.project.read(&file.path);
                }
            }
            if let Some(log) = collab.log.as_mut() {
                log::append(log, seq, &author, &payload, &mut session.status);
            }
            // Another author's change stays out of the folder until they
            // submit. See *What a save writes*.
            if let (true, Step::Edit { change, forward, .. }) = (Some(user) != collab.me, &payload) {
                session.exclude.push(crate::session::Excluded { seq, author: author.id, change: change.clone(), forward: *forward });
            }
        }
        Action::Request { user, author, request: vale_collab::wire::Request::Withdraw } => {
            if changes::withdraw_theirs(collab, session, assets, &author) {
                if let Some(Session::Hosting(hosting)) = collab.session.as_ref() {
                    hosting.answer(user, Answer::Withdrawn);
                }
            }
        }
        Action::Request { user, author, request } => {
            let mut released = Vec::new();
            let mut submitted = false;
            if let (Some(log), Some(Session::Hosting(hosting))) = (collab.log.as_mut(), collab.session.as_mut()) {
                let answer = log::answer(log, &author, request, &mut session.status);
                // A Submit releases the author's claims, and so does a Submit
                // with nothing to submit, which still ends the check outs. A
                // Submit whose changelist could not be written leaves edits
                // open, and keeps them.
                if let Some(vale_collab::wire::Answer::Submitted(closed)) = &answer {
                    if closed.is_some() || log.open_count(&author.id) == 0 {
                        released = hosting.release_author(&author.id);
                    }
                    submitted = closed.is_some();
                }
                if let Some(answer) = answer {
                    hosting.answer(user, answer);
                }
            }
            for action in released {
                act(collab, session, assets, camera, fly, action);
            }
            if submitted {
                changes::save_submitted(collab, session, assets, &author.id);
            }
        }
        Action::Holds(list) => holds_changed(collab, session, list),
        Action::Answer(answer) => collab.answers.push(answer),
        Action::Ids { kind, start, end } => session.numbers.hold(&kind, start, end),
    }
}

/// Apply another editor's step, and note what it changed: the chunks to
/// outline in that editor's colour, who has unsaved edits in which file, and
/// the files the Assets workspace reads again.
fn apply_remote(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, user: UserId, step: &Step) {
    match step {
        Step::Edit { change, forward, .. } => {
            session.apply_step(assets, change, *forward);
            forget_overlapping(session, &step.groups());
            // Hosting, another author's edit is not this editor's to save:
            // the folder leaves it out until they submit. Its subjects stay
            // unsaved only where this editor has edits of its own.
            if matches!(collab.state, State::Hosting { .. }) {
                changes::unmark_theirs(collab, session, &step.groups());
            }
            let invalidated = change.invalidates();
            for redraw in &invalidated.tiles {
                for chunk in redraw.chunks.iter().chain(&redraw.painted) {
                    collab.recent.insert((user, redraw.tile.clone(), *chunk), collab.now);
                }
                collab.edited.entry(redraw.tile.vpath().to_ascii_lowercase()).or_default().insert(user);
            }
            for name in &invalidated.tables {
                let vpath = vale_assets::tables::dbc::dbc_path(name).to_ascii_lowercase();
                collab.edited.entry(vpath).or_default().insert(user);
            }
        }
        Step::Files(files) => {
            let name = collab.people.get(&user).map_or("another editor", |person| person.name.as_str()).to_string();
            // Hosting, another author's files stay out of the folder until
            // they submit. See *What a save writes*.
            let author = match (&collab.session, user) {
                (Some(Session::Hosting(hosting)), user) if user != 0 => hosting.author_of(user),
                _ => None,
            };
            let done = match author {
                Some(author) => {
                    files::stage(session, assets, author.id, files);
                    Ok(())
                }
                None => files::apply(session, assets, files),
            };
            session.status = match done {
                Ok(()) => format!("{name} {}", files.words()),
                Err(e) => format!("{name} {}, but {e}", files.words()),
            };
            forget_overlapping(session, &step.groups());
            for file in &files.files {
                // The file replaces whatever was edited in it.
                collab.edited.remove(&file.path.to_ascii_lowercase());
                collab.changed.push(file.path.clone());
            }
        }
        Step::Delete(_) => {}
    }
}

/// A joined editor: apply an edit in no changelist, which the host holds and
/// its files do not, to this editor's copy. See *Joining*.
fn apply_open(session: &mut EditSession, assets: &GameAssets, step: &Step) {
    match step {
        Step::Edit { change, forward, .. } => {
            session.apply_step(assets, change, *forward);
            forget_overlapping(session, &step.groups());
        }
        Step::Files(files) => {
            if let Err(e) = files::apply(session, assets, files) {
                session.status = format!("{} could not be written: {e}", files.what());
            }
        }
        Step::Delete(_) => {}
    }
}

/// Let in an editor that answered the challenge: save, list the project's
/// files with their hashes, and welcome it.
fn admit(collab: &mut Collab, session: &mut EditSession, user: UserId, name: &str) {
    if let Err(e) = session.save_files() {
        session.status = format!("{name} was not let in: the server rows could not be saved: {e}");
        if let Some(Session::Hosting(hosting)) = collab.session.as_mut() {
            hosting.remove(user, "the host could not save its project");
        }
        return;
    }
    let files = fetch::manifest(&session.project);
    let project = session.project.name.clone();
    let Some(Session::Hosting(hosting)) = collab.session.as_mut() else { return };
    let database = collab.own_database.map(|fingerprint| vale_collab::wire::Database { fingerprint, shared: collab.shared_info.clone() });
    for action in hosting.admit(user, project, files, database) {
        if let Action::Joined { user, name } = action {
            session.status = format!("{name} joined the session; the project was saved for them");
            collab.people.insert(user, Person { name, presence: None });
        }
    }
    // So the new editor learns the host's map and view at once.
    collab.presence_at = 0.0;
}

/// Every file is in: open the copy, apply what arrived meanwhile, and go to
/// the host's map and view.
fn opened_the_copy(
    collab: &mut Collab,
    session: &mut EditSession,
    assets: &GameAssets,
    camera: &mut crate::camera::EditorCamera,
    fly: &mut MessageWriter<crate::camera::FlyTo>,
    project: String,
) {
    if !session.open_again(assets, &project) {
        end(collab, session, assets, format!("could not open {project}"));
        return;
    }
    collab.state = State::Joined { project: project.clone() };
    // Every author's edits in no changelist come first: the host's files do
    // not hold them. See *Joining*.
    if let Some(running) = collab.session.as_mut() {
        running.request(vale_collab::wire::Request::Open { everyone: true });
        collab.open_wanted = true;
    }
    link_the_copy(collab, session);
    session.status = match collab.host_name() {
        Some(host) => format!("joined {host}'s session; {project} is open"),
        None => format!("joined; {project} is open"),
    };
    if collab.people.get(&0).and_then(|host| host.presence.as_ref()).is_some() {
        carry_out(collab, session, assets, camera, fly, Ask::GoTo(0));
    }
}

/// The copy is open: write its link, and send what it holds offline. See
/// [`offline`]'s *Connecting again*.
fn link_the_copy(collab: &mut Collab, session: &mut EditSession) {
    let old = offline::Link::read(&session.project);
    let pending = offline::read(&session.project);
    let seq = collab.session.as_ref().map_or(0, |running| running.me().seq());
    let link = offline::Link {
        address: collab.join_address.clone().unwrap_or_default(),
        project: collab.host_project.clone().unwrap_or_default(),
        log: collab.host_log.unwrap_or_default(),
        synced: seq,
        fetched: collab.welcome_seq,
        open: Vec::new(),
    };
    if let Err(e) = link.write(&session.project) {
        session.status = format!("the copy's link to the host could not be written: {e}");
    }
    collab.link = Some(link);
    collab.log_for = Some(session.project.root.clone());
    collab.pending = whole_edits(&pending);
    if !pending.is_empty() {
        let (after, log) = old.map_or((0, [0; 16]), |old| (old.synced, old.log));
        changes::send_pending(collab, pending, after, log);
    }
}

/// Leave a joined session and keep the copy open offline. The edits the host
/// had not answered become pending; see [`offline`].
fn go_offline(collab: &mut Collab, session: &mut EditSession, reason: String) {
    keep_unanswered(collab, session);
    close(collab);
    info!("collab: {reason}");
    session.status = reason.clone();
    collab.state = State::Ended(reason);
}

/// Before a joined session ends: write the copy's link at the last edit it
/// has, and keep its edits the host had not answered as pending.
fn keep_unanswered(collab: &mut Collab, session: &mut EditSession) {
    if !matches!(collab.state, State::Joined { .. }) {
        return;
    }
    let Some(running) = collab.session.as_mut() else { return };
    let seq = running.me().seq();
    let waiting: Vec<Step> = running.take_waiting().into_iter().filter(|step| !matches!(step, Step::Delete(_))).collect();
    if let Some(link) = collab.link.as_mut() {
        link.synced = seq;
        let _ = link.write(&session.project);
    }
    if let Err(e) = offline::append(&session.project, &waiting) {
        session.status = format!("edits the host had not answered could not be kept: {e}");
    }
    collab.pending += whole_edits(&waiting);
}

/// The host sent every claim: hold them, and keep the check outs in the log
/// folder when this editor hosts.
pub(super) fn holds_changed(collab: &mut Collab, session: &mut EditSession, list: Vec<Hold>) {
    if matches!(collab.state, State::Hosting { .. }) {
        if let Err(e) = write_checkouts(&session.project, &list) {
            session.status = format!("the check outs could not be written to the log folder: {e}");
        }
    }
    collab.holds = list;
}

/// The check outs the host's editor kept in the project's log folder.
fn read_checkouts(project: &vale_edit::project::Project) -> Vec<Hold> {
    let path = project.root.join(vale_collab::log::LOG_DIR).join(CHECKOUTS_FILE);
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(Claim::parse)
        .map(|(claim, author)| Hold { claim, author, checked_out: true, edited: false })
        .collect()
}

/// Keep the check outs of `list` in the project's log folder, or remove the
/// file when there are none. The claims held by edits are not kept: the log
/// holds them.
fn write_checkouts(project: &vale_edit::project::Project, list: &[Hold]) -> std::io::Result<()> {
    let list: Vec<&Hold> = list.iter().filter(|held| held.checked_out).collect();
    let dir = project.root.join(vale_collab::log::LOG_DIR);
    let path = dir.join(CHECKOUTS_FILE);
    if list.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(&dir)?;
    let mut text = String::from("# Check outs in this project's shared sessions, written by the world editor.\n");
    for held in list {
        text.push_str(&held.claim.line(&held.author));
        text.push('\n');
    }
    std::fs::write(path, text)
}

/// Drop the session and everything about the people in it.
fn close(collab: &mut Collab) {
    collab.session = None;
    collab.connecting = None;
    collab.fetch = None;
    collab.held.clear();
    collab.people.clear();
    collab.requests.clear();
    collab.edited.clear();
    collab.host_address = None;
    collab.host_database = None;
    collab.shared_database = None;
    collab.holds.clear();
    collab.holds_at = f64::NEG_INFINITY;
    collab.open_wanted = false;
    collab.me = None;
}

/// End the session: leave or stop hosting, open this editor's own project
/// again if it had joined, and say why.
fn end(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, reason: String) {
    let was_joined = collab.is_joined();
    collab.server_chosen = false;
    keep_unanswered(collab, session);
    collab.session = None;
    collab.connecting = None;
    collab.fetch = None;
    collab.held.clear();
    collab.people.clear();
    collab.requests.clear();
    collab.edited.clear();
    collab.host_address = None;
    collab.host_database = None;
    collab.holds.clear();
    collab.holds_at = f64::NEG_INFINITY;
    collab.open_wanted = false;
    collab.me = None;
    if let (true, Some(own)) = (was_joined, collab.own_project.take()) {
        if own != session.project.name {
            session.switch_to(assets, &own);
        }
    }
    info!("collab: {reason}");
    session.status = reason.clone();
    collab.state = State::Ended(reason);
}

pub(crate) struct CollabPlugin;

impl Plugin for CollabPlugin {
    fn build(&self, app: &mut App) {
        // Late in the frame, after every tool has written, so a step made this
        // frame is sent this frame.
        app.init_resource::<Collab>()
            .add_systems(Update, on_the_command_line)
            .add_systems(PostUpdate, run)
            .add_systems(
                Update,
                people::draw.after(crate::pick::aim).after(vale_client::world::camera::place),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raise made on one session, taken off its journal, sent through the
    /// wire's encoding and applied to a second session holding the same tile,
    /// leaves the two tiles byte for byte the same; the host's undo, sent the
    /// same way, puts the second back as it was.
    #[test]
    fn a_shared_step_leaves_the_other_tile_the_same() {
        let install = std::env::temp_dir().join(format!("vale-collab-step-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let open = |name: &str| EditSession::for_tests(vale_edit::project::Project::open(&install, name).unwrap());
        let (mut host, mut other) = (open("Host"), open("Other"));
        let assets = GameAssets::new(install.join("Data").to_string_lossy().into_owned());
        let coord = (32, 48);
        let tile = vale_edit::adt::blank::blank_tile(coord.0, coord.1, r"Tileset\Elwynn\ElwynnGrass01.blp", 10.0, 12);
        let original = tile.write();
        host.tiles.insert(coord, tile.clone());
        other.tiles.insert(coord, tile);
        host.history.journal(true);

        let before = vale_edit::adt::heights::heights(host.tiles[&coord].chunk(5).unwrap());
        let after: Vec<f32> = before.iter().map(|height| height + 7.5).collect();
        let edit = vale_edit::ops::Edit::Heights { chunk: 5, before, after };
        edit.apply(host.tiles.get_mut(&coord).unwrap());
        host.history.begin("Raise terrain");
        host.history.record(&host.key(coord), [edit]);
        host.history.end();
        host.history.undo();

        let steps = host.history.take_journal();
        assert_eq!(steps.iter().map(|step| step.forward).collect::<Vec<_>>(), [true, false]);
        let mut sent = Vec::new();
        for step in steps {
            let mut bytes = Vec::new();
            vale_collab::wire::write_frame(&mut bytes, &Step::journaled(step)).unwrap();
            match vale_collab::wire::read_frame::<Step>(&mut bytes.as_slice()).unwrap() {
                Step::Edit { change, forward, .. } => sent.push((change, forward)),
                other => panic!("{other:?}"),
            }
        }

        other.apply_step(&assets, &sent[0].0, sent[0].1);
        assert_ne!(other.tiles[&coord].write(), original, "the raise reached the other session");
        let raised = sent[0].0.clone();
        let mut host_raised = host.tiles[&coord].clone();
        raised.apply(&host.key(coord), &mut host_raised);
        assert_eq!(other.tiles[&coord].write(), host_raised.write());
        assert!(other.unsaved.contains(&coord));

        other.apply_step(&assets, &sent[1].0, sent[1].1);
        assert_eq!(other.tiles[&coord].write(), original);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A loopback host and this machine's network address give one
    /// fingerprint; another port or database gives another. The host part of
    /// a typed address drops the session's port.
    #[test]
    fn a_database_fingerprint_names_the_machine_and_not_the_spelling() {
        let at = |info: &str| vale_mangos::conn::Where::parse(info).unwrap();
        let local = database_fingerprint(&at("127.0.0.1;3306;mangos;secret;mangos"));
        assert_eq!(local, database_fingerprint(&at("LOCALHOST;3306;other;password;Mangos")));
        if let Some(ip) = lan_address() {
            assert_eq!(local, database_fingerprint(&at(&format!("{ip};3306;mangos;secret;mangos"))));
        }
        assert_ne!(local, database_fingerprint(&at("127.0.0.1;3307;mangos;secret;mangos")));
        assert_ne!(local, database_fingerprint(&at("127.0.0.1;3306;mangos;secret;world")));
        assert_eq!(host_part("192.168.1.20:7878"), "192.168.1.20");
        assert_eq!(host_part("hostpc:7878"), "hostpc");
        assert_eq!(host_part("192.168.1.20"), "192.168.1.20");
        let shared = shared_location("127.0.0.1;3306;mangos;secret;mangos", Some("192.168.1.20")).unwrap();
        assert_eq!((shared.host.as_str(), shared.port(), shared.password.as_str()), ("192.168.1.20", 3306, "secret"));
        let elsewhere = shared_location("db.lan;3307;mangos;secret;world", Some("192.168.1.20")).unwrap();
        assert_eq!(elsewhere.host, "db.lan");
    }

    /// A joined editor may write to its server only when it uses this
    /// machine's server and that server's database is not the host's. The
    /// host and an editor in no session always may.
    #[test]
    fn a_joined_editor_writes_only_to_a_server_that_is_not_the_hosts() {
        let mut collab = Collab::default();
        assert!(collab.server_gate().writes());
        collab.state = State::Joined { project: "Goldshire".into() };
        collab.people.insert(0, Person { name: "Sam".into(), presence: None });
        collab.own_database = Some([1; 32]);
        collab.host_database = Some([2; 32]);
        collab.playtest_server = PlaytestServer::Local;
        assert!(collab.server_gate().writes());
        collab.playtest_server = PlaytestServer::Host;
        let refused = collab.server_gate().refused.unwrap();
        assert!(refused.contains("only Sam applies changes"), "{refused}");
        collab.playtest_server = PlaytestServer::Local;
        collab.host_database = Some([1; 32]);
        let refused = collab.server_gate().refused.unwrap();
        assert!(refused.contains("same world database"), "{refused}");
        collab.state = State::Hosting { project: "Goldshire".into(), addr: "127.0.0.1:7878".parse().unwrap() };
        assert!(collab.server_gate().writes());
    }

    /// A remote edit on a chunk drops this editor's undo entries that touch
    /// that chunk, and keeps the ones elsewhere.
    #[test]
    fn a_remote_edit_drops_the_undo_entries_it_overwrites() {
        let install = std::env::temp_dir().join(format!("vale-collab-forget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Join").unwrap());
        let tile = session.key((32, 48));
        for chunk in [3, 9] {
            session.history.begin("Raise");
            session.history.record(&tile, [vale_edit::ops::Edit::Heights { chunk, before: vec![0.0], after: vec![1.0] }]);
            session.history.end();
        }
        let mut remote = vale_edit::undo::History::new();
        remote.begin("Lower");
        remote.record(&tile, [vale_edit::ops::Edit::Heights { chunk: 3, before: vec![1.0], after: vec![0.5] }]);
        remote.end();
        let remote = remote.undo().unwrap();
        assert_eq!(forget_overlapping(&mut session, &remote.groups()), 1);
        assert_eq!(session.history.undo().unwrap().tiles(), [tile]);
        assert!(session.history.undo().is_none(), "only the entry on chunk 9 was left");
        let _ = std::fs::remove_dir_all(&install);
    }
}
