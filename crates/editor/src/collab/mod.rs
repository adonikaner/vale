//! A shared editing session from the editor's side: hosting one or joining
//! one, the project a joining editor fetches, every edit the host makes
//! applied on the others as it is made, and where everyone is. The protocol
//! is `vale_collab`'s; the panel is `crate::ui::shell::session`.
//!
//! ```text
//! mod.rs     the session's state, the frame's work and the plugin
//! fetch.rs   a joining editor's copy of the host's project: what it lacks,
//!            the pieces as they arrive, and the hash check
//! people.rs  the others in the session drawn in the viewport: a ring at each
//!            one's pointer, a pole with their name at what they look at, and
//!            the chunks they changed in the last moments outlined
//! ```
//!
//! ## What is shared
//!
//! What the undo stack holds: terrain, paint, liquid, areas, holes,
//! placements, client table fields and records, and server rows. The undo
//! stack's journal ([`vale_edit::undo::History::journal`]) hands each one
//! over as it is made, a held stroke in parts ten times a second, and each
//! undo and redo. Every one is a [`Step`], sent to the host, and applied on
//! the others through [`EditSession::apply_step`], the path an undo takes.
//! What the undo stack does not hold is not shared: imported files, new and
//! deleted tiles, locks and the world map.
//!
//! A joined editor's edits go to the host like the host's own, except the
//! ones that need a number or a list only the host may hand out
//! ([`joinable`]): adding, removing or renaming doodads and buildings,
//! adding or removing client table records, and creating, removing or
//! replacing whole server rows. Those are taken back with a line saying so.
//!
//! ## When two editors change the same thing
//!
//! The host accepts whichever edit reaches it first and refuses the other;
//! `vale_collab::peer` says how the editor whose edit lost learns of it
//! before the refusal arrives. Here, an edit taken back is reverted through
//! [`EditSession::apply_step`], and so is the part of a held stroke not sent
//! yet (the journal's open segment), and the status line names who changed
//! the same thing first.
//!
//! ## Undo
//!
//! Each editor's undo stack holds its own edits only; a remote edit is
//! applied beside it. Undoing an entry writes the state it found, which
//! would overwrite whatever another editor did there since. So when a remote
//! edit lands, every entry of this editor's stack that touches the same
//! groups is dropped ([`vale_edit::undo::History::forget_where`]): what
//! someone else has changed since cannot be undone from here.
//!
//! ## Joining
//!
//! The host saves its tiles, tables and server rows and sends the list of its
//! project's files, with a hash of each. The joining editor keeps a copy of
//! the host's project in `Edit\<project> (shared)\`, asks for the files its
//! copy lacks or holds different bytes of, and removes the ones the host does
//! not have. Once every file is in, it opens the copy, applies the edits that
//! arrived meanwhile, and goes to the host's map and view. Its own project is
//! saved and closed while it is joined and opened again when it leaves.

mod fetch;
mod people;

use crate::session::EditSession;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use vale_client::assets::GameAssets;
use vale_collab::{Action, Cause, Group, Hosting, Joined, Payload, Presence, Session, UserId};
use vale_edit::undo::Change;

/// One shared step: a change, applied forward (an edit or a redo) or
/// reverted (an undo).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Step {
    pub(crate) change: Change,
    pub(crate) forward: bool,
}

impl Payload for Step {
    fn groups(&self) -> Vec<Group> {
        self.change.groups()
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

/// Which half of the panel's form is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Mode {
    #[default]
    Host,
    Join,
}

/// What the panel asks for. Carried out by [`run`] on the next frame, which
/// holds the session, the archives and the camera.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Ask {
    Host,
    Join,
    Leave,
    GoTo(UserId),
    Remove(UserId),
}

/// The panel's fields.
#[derive(Debug, Clone)]
pub(crate) struct Form {
    pub(crate) mode: Mode,
    pub(crate) name: String,
    pub(crate) password: String,
    /// The host to join: an address, with or without `:port`.
    pub(crate) address: String,
    /// The port to host on.
    pub(crate) port: String,
}

impl Default for Form {
    fn default() -> Form {
        Form {
            mode: Mode::Host,
            name: std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default(),
            password: String::new(),
            address: String::new(),
            port: PORT.to_string(),
        }
    }
}

/// Someone in the session.
#[derive(Debug, Clone)]
pub(crate) struct Person {
    pub(crate) name: String,
    pub(crate) presence: Option<Presence>,
}

/// The session and everything the panel shows about it.
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
    /// Edits received while the project is fetched, applied once it is open.
    held: Vec<Step>,
    /// The project this editor had open before it joined.
    own_project: Option<String>,
    presence_at: f64,
    segment_at: f64,
    /// Steps sent and received, and this editor's taken back, for the panel.
    pub(crate) sent: u64,
    pub(crate) received: u64,
    pub(crate) taken_back: u64,
    /// The chunks each other editor changed, by tile and chunk, and when:
    /// outlined in their colour for [`people::RECENT`] seconds.
    pub(crate) recent: std::collections::HashMap<(UserId, vale_edit::TileKey, usize), f64>,
    /// The frame's clock, for the actions that note a time.
    now: f64,
}

impl Collab {
    /// Whether this editor is joined to another's session, and so may not
    /// save, publish or apply: the host does those.
    pub(crate) fn is_joined(&self) -> bool {
        matches!(self.state, State::Connecting { .. } | State::Fetching { .. } | State::Joined { .. })
    }

    /// The host's name, when joined.
    pub(crate) fn host_name(&self) -> Option<&str> {
        self.people.get(&0).map(|person| person.name.as_str())
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

/// The frame's work: what the panel asked, the steps this editor made, the
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
) {
    let Some(session) = session.as_mut() else { return };
    let collab = &mut *collab;
    let now = time.elapsed_secs_f64();
    collab.now = now;
    collab.recent.retain(|_, at| now - *at < people::RECENT);
    if let Some(ask) = collab.ask.take() {
        carry_out(collab, session, &assets, &mut camera, &mut fly, ask);
    }
    take_the_connection(collab, session);
    if collab.session.is_none() {
        return;
    }
    if let State::Hosting { project, .. } = &collab.state {
        if *project != session.project.name {
            let reason = format!("the session ended: {project} is no longer the open project");
            end(collab, session, &assets, reason);
            return;
        }
    }
    share_own_steps(collab, session, &assets, now);

    // The part of a held stroke not sent yet, which a remote edit over it
    // takes back. See `vale_collab::peer`.
    let open = session.history.segment().cloned().map(|change| Step { change, forward: true });
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

    if now - collab.presence_at >= PRESENCE_EVERY {
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
}

/// Carry out what the panel asked.
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
        Ask::Host if collab.session.is_none() => {
            let Ok(port) = collab.form.port.trim().parse::<u16>() else {
                collab.state = State::Ended(format!("{} is not a port", collab.form.port.trim()));
                return;
            };
            match Hosting::start(&format!("0.0.0.0:{port}"), &name, &collab.form.password, BUILD, archives(assets)) {
                Ok(hosting) => {
                    let addr = std::net::SocketAddr::new(lan_address().unwrap_or(hosting.addr().ip()), hosting.addr().port());
                    collab.people = hosting.users().into_iter().map(|(user, name)| (user, Person { name, presence: None })).collect();
                    collab.session = Some(Session::Hosting(hosting));
                    collab.me = Some(0);
                    collab.state = State::Hosting { project: session.project.name.clone(), addr };
                    collab.sent = 0;
                    collab.received = 0;
                    session.history.journal(true);
                    session.status = format!("hosting {} on {addr}", session.project.name);
                    info!("collab: hosting {} on {addr}", session.project.name);
                }
                Err(e) => collab.state = State::Ended(format!("could not host on port {port}: {e}")),
            }
        }
        Ask::Join if collab.session.is_none() && collab.connecting.is_none() => {
            let mut address = collab.form.address.trim().to_string();
            if address.is_empty() {
                collab.state = State::Ended("type the host's address".to_string());
                return;
            }
            if !address.contains(':') {
                address = format!("{address}:{PORT}");
            }
            let (password, archives) = (collab.form.password.clone(), archives(assets));
            let (done, connecting) = std::sync::mpsc::channel();
            let to = address.clone();
            std::thread::spawn(move || {
                let _ = done.send(Joined::connect(&to, &name, &password, BUILD, archives));
            });
            collab.connecting = Some(std::sync::Mutex::new(connecting));
            collab.state = State::Connecting { to: address };
        }
        Ask::Leave => {
            let reason = match collab.state {
                State::Hosting { .. } => "you ended the session",
                _ => "you left the session",
            };
            end(collab, session, assets, reason.to_string());
            collab.state = State::Off;
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
                let actions = hosting.remove(user, "the host removed you from the session");
                for action in actions {
                    act(collab, session, assets, camera, fly, action);
                }
            }
        }
        _ => {}
    }
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

/// Send the steps this editor made. A joined editor's step that only the
/// host may make ([`joinable`]) is taken back instead.
fn share_own_steps(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, now: f64) {
    if now - collab.segment_at >= SEGMENT_EVERY {
        collab.segment_at = now;
        session.history.close_segment();
    }
    let joined = match collab.state {
        State::Joined { .. } => true,
        State::Hosting { .. } => false,
        _ => return,
    };
    for (change, forward) in session.history.take_journal() {
        if joined {
            if let Err(what) = joinable(&change) {
                session.apply_step(assets, &change, !forward);
                forget_overlapping(session, &change.groups());
                collab.taken_back += 1;
                session.status = format!("{} was taken back: {what} is the host's in this version", change.label);
                continue;
            }
        }
        if let Some(running) = collab.session.as_mut() {
            running.propose(Step { change, forward });
            collab.sent += 1;
        }
    }
}

/// Whether a joined editor may make this change, or what in it is the
/// host's alone. Each needs a number or a list the host hands out: a new
/// `uniqueId` or a renumbered placement list, a new record's index, a new
/// row's key. The host's number blocks (`vale_collab::ids`) are what will
/// lift this.
pub(crate) fn joinable(change: &Change) -> Result<(), &'static str> {
    use vale_edit::ops::Edit;
    use vale_edit::undo::{Entry, Shape};
    for entry in change.in_order(true) {
        match entry {
            Entry::Tile(_, Edit::Placements { .. } | Edit::Names { .. }) => {
                return Err("adding, removing and renaming doodads and buildings")
            }
            Entry::Row(..) => return Err("adding and removing table records"),
            Entry::Server(server) if server.shape == Shape::Row => {
                return Err("creating, removing and replacing whole server rows")
            }
            _ => {}
        }
    }
    Ok(())
}

/// Drop every entry of this editor's undo stack that touches one of
/// `groups`. See the module comment's *Undo*.
fn forget_overlapping(session: &mut EditSession, groups: &[Group]) -> usize {
    session
        .history
        .forget_where(|change| change.groups().iter().any(|group| groups.binary_search(group).is_ok()))
}

/// The status line for one of this editor's edits taken back.
fn taken_back(collab: &Collab, label: &str, cause: &Cause) -> String {
    match cause {
        Cause::Changed { user } => {
            let name = collab.people.get(user).map_or("another editor", |person| person.name.as_str());
            format!("{label} was taken back: {name} changed the same thing first")
        }
        Cause::Refused(reason) => format!("{label} was taken back: the host refused it ({reason})"),
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
        Action::Welcome { user, users } => {
            collab.me = Some(user);
            collab.people = users.into_iter().map(|(user, name)| (user, Person { name, presence: None })).collect();
        }
        Action::Refused(reason) | Action::Closed(reason) => {
            let reason = match collab.state {
                State::Joined { .. } => format!("the session ended: {reason}"),
                _ => format!("could not join: {reason}"),
            };
            end(collab, session, assets, reason);
        }
        Action::Apply { user, payload } => {
            collab.received += 1;
            match collab.state {
                State::Joined { .. } | State::Hosting { .. } => {
                    session.apply_step(assets, &payload.change, payload.forward);
                    forget_overlapping(session, &payload.groups());
                    for redraw in payload.change.invalidates().tiles {
                        for chunk in redraw.chunks.iter().chain(&redraw.painted) {
                            collab.recent.insert((user, redraw.tile.clone(), *chunk), collab.now);
                        }
                    }
                }
                _ => collab.held.push(payload),
            }
        }
        Action::Revert { payload, cause } => {
            session.apply_step(assets, &payload.change, !payload.forward);
            forget_overlapping(session, &payload.groups());
            collab.taken_back += 1;
            session.status = taken_back(collab, &payload.change.label, &cause);
        }
        Action::CancelOpen { cause } => {
            if let Some(segment) = session.history.drop_segment() {
                session.apply_step(assets, &segment, false);
                forget_overlapping(session, &segment.groups());
                collab.taken_back += 1;
                session.status = taken_back(collab, &segment.label, &cause);
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
            if let Some(person) = collab.people.remove(&user) {
                session.status = format!("{} left the session", person.name);
            }
        }
        Action::Manifest { project, files } => match fetch::Fetch::start(assets, &project, files) {
            Ok((fetch, wanted)) => {
                collab.state = fetch.state();
                collab.fetch = Some(fetch);
                if let Some(running) = collab.session.as_mut() {
                    running.fetch(wanted);
                }
            }
            Err(e) => end(collab, session, assets, format!("could not keep a copy of {project}: {e}")),
        },
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
            let files = paths
                .into_iter()
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
    }
}

/// Let in an editor that answered the challenge: save, list the project's
/// files with their hashes, and welcome it.
fn admit(collab: &mut Collab, session: &mut EditSession, user: UserId, name: &str) {
    session.save_all();
    session.save_all_tables();
    if let Err(e) = session.rows.save(&session.project) {
        session.status = format!("{name} was not let in: the server rows could not be saved: {e}");
        if let Some(Session::Hosting(hosting)) = collab.session.as_mut() {
            hosting.remove(user, "the host could not save its project");
        }
        return;
    }
    let files = fetch::manifest(&session.project);
    let project = session.project.name.clone();
    let Some(Session::Hosting(hosting)) = collab.session.as_mut() else { return };
    for action in hosting.admit(user, project, files) {
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
    session.history.journal(true);
    collab.state = State::Joined { project: project.clone() };
    for step in std::mem::take(&mut collab.held) {
        session.apply_step(assets, &step.change, step.forward);
    }
    session.status = match collab.host_name() {
        Some(host) => format!("joined {host}'s session; {project} is open"),
        None => format!("joined; {project} is open"),
    };
    if collab.people.get(&0).and_then(|host| host.presence.as_ref()).is_some() {
        carry_out(collab, session, assets, camera, fly, Ask::GoTo(0));
    }
}

/// End the session: leave or stop hosting, open this editor's own project
/// again if it had joined, and say why.
fn end(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, reason: String) {
    let was_joined = collab.is_joined();
    collab.session = None;
    collab.connecting = None;
    collab.fetch = None;
    collab.held.clear();
    collab.people.clear();
    collab.me = None;
    session.history.journal(false);
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
        assert_eq!(steps.iter().map(|(_, forward)| *forward).collect::<Vec<_>>(), [true, false]);
        let mut sent = Vec::new();
        for (change, forward) in steps {
            let mut bytes = Vec::new();
            vale_collab::wire::write_frame(&mut bytes, &Step { change, forward }).unwrap();
            sent.push(vale_collab::wire::read_frame::<Step>(&mut bytes.as_slice()).unwrap());
        }

        other.apply_step(&assets, &sent[0].change, sent[0].forward);
        assert_ne!(other.tiles[&coord].write(), original, "the raise reached the other session");
        let raised = sent[0].change.clone();
        let mut host_raised = host.tiles[&coord].clone();
        raised.apply(&host.key(coord), &mut host_raised);
        assert_eq!(other.tiles[&coord].write(), host_raised.write());
        assert!(other.unsaved.contains(&coord));

        other.apply_step(&assets, &sent[1].change, sent[1].forward);
        assert_eq!(other.tiles[&coord].write(), original);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A joined editor may send terrain and column edits, and not the edits
    /// that need a number or list the host hands out.
    #[test]
    fn a_joined_editor_sends_terrain_but_not_new_placements_or_rows() {
        use vale_edit::undo::{History, ServerEntry};
        let tile = vale_edit::TileKey::new("Azeroth", 32, 48);
        let one = |record: &dyn Fn(&mut History)| {
            let mut history = History::new();
            history.begin("Edit");
            record(&mut history);
            history.end();
            history.undo().expect("the change")
        };
        let raise = one(&|h| h.record(&tile, [vale_edit::ops::Edit::Heights { chunk: 3, before: vec![0.0], after: vec![1.0] }]));
        assert!(joinable(&raise).is_ok());
        let column = one(&|h| h.record_server(ServerEntry::column("creature", "guid=7", "position_x", None, Some("1".into()))));
        assert!(joinable(&column).is_ok());
        let row = one(&|h| h.record_server(ServerEntry::row("creature", "guid=7", None, Some("…".into()))));
        assert!(joinable(&row).is_err());
        let blank = vale_edit::adt::blank::blank_tile(32, 48, r"Tileset\Elwynn\ElwynnGrass01.blp", 0.0, 12);
        let lists = vale_edit::ops::Placements::capture(&blank);
        let placed = one(&|h| h.record(&tile, [vale_edit::ops::Edit::Placements { before: Box::new(lists.clone()), after: Box::new(lists.clone()) }]));
        assert!(joinable(&placed).is_err());
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
