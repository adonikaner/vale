//! The two ways an editor is in a session.
//!
//! [`Hosting`] is a [`Host`], this editor's own [`Peer`] as user 0, and the
//! [`Listener`]: a message for user 0 goes straight to the peer, any other to
//! its connection. [`Joined`] is a peer and its [`Connection`]. Both answer
//! the same calls, gathered in [`Session`], and the editor treats them alike:
//! it proposes its edits, sends its presence, and on every frame polls for
//! the [`Action`]s to carry out.

use crate::groups::Payload;
use crate::checkout::{Claim, Hold};
use crate::groups::Group;
use crate::host::{Host, Out};
use crate::log::{Author, AuthorId};
use crate::net::{Connection, Event, Listener};
use crate::peer::{Action, Peer};
use crate::wire::{Answer, Database, FileEntry, Presence, Request, Seq, ToHost, ToPeer, UserId, FILE_PIECE};
use std::collections::HashMap;

/// Hosting a session. Dropping it closes every connection.
pub struct Hosting<P: Payload> {
    host: Host,
    me: Peer<P>,
    net: Listener<ToHost<P>, ToPeer<P>>,
}

impl<P: Payload> Hosting<P> {
    /// Host a session on `addr` (`0.0.0.0:7878`) as `author`, behind
    /// `password`. Joining editors must be `build` and hold `archives`.
    pub fn start(addr: &str, author: &Author, password: &str, build: &str, archives: [u8; 32]) -> std::io::Result<Hosting<P>> {
        Ok(Hosting {
            host: Host::new(author, password, build, archives),
            me: Peer::local(&author.name, 0),
            net: Listener::bind(addr)?,
        })
    }

    /// Continue from the host's log. See [`Host::resume`]. Called before
    /// anyone joins; this editor's own peer starts at `seq` as well.
    pub fn resume(
        &mut self,
        log: [u8; 16],
        seq: Seq,
        touched: HashMap<Group, (Seq, AuthorId)>,
        pending: HashMap<Group, AuthorId>,
        names: &HashMap<AuthorId, String>,
    ) {
        self.host.resume(log, seq, touched, pending, names);
        self.me.resume(seq);
    }

    /// Grant blocks of numbers from `ids`. See [`Host::share_ids`].
    pub fn share_ids(&mut self, ids: crate::ids::Shared) {
        self.host.share_ids(ids);
    }

    /// The author of an editor that is in.
    pub fn author_of(&self, user: UserId) -> Option<Author> {
        self.host.author_of(user)
    }

    /// Hold the check outs kept from an earlier session. See
    /// [`Host::resume_checkouts`].
    pub fn resume_checkouts(&mut self, list: Vec<Hold>) {
        self.host.resume_checkouts(list);
    }

    /// Every claim held. See [`crate::checkout`].
    pub fn holds(&self) -> Vec<Hold> {
        self.host.holds()
    }

    /// Release every claim an author holds, as a Submit of theirs does, and
    /// answer what this editor is to do.
    pub fn release_author(&mut self, author: &AuthorId) -> Vec<Action<P>> {
        let outs = self.host.release_author(author);
        let mut actions = Vec::new();
        self.carry(outs, None, &mut actions);
        actions
    }

    /// Release what an author's withdrawn or taken back edits held, and
    /// answer what this editor is to do. See [`Host::release_pending`].
    pub fn release_pending(&mut self, author: &AuthorId) -> Vec<Action<P>> {
        let outs = self.host.release_pending(author);
        let mut actions = Vec::new();
        self.carry(outs, None, &mut actions);
        actions
    }

    /// Answer an editor's [`Action::Request`].
    pub fn answer(&self, user: UserId, answer: Answer<P>) {
        self.net.send(user, ToPeer::Answer(answer));
    }

    /// The address and port joining editors connect to.
    pub fn addr(&self) -> std::net::SocketAddr {
        self.net.addr()
    }

    /// Everyone in, the host first.
    pub fn users(&self) -> Vec<(UserId, String)> {
        self.host.users()
    }

    /// Let in an editor that waits after [`Action::Admit`]: it is welcomed at
    /// the current sequence and sent the manifest, `project` and `files`,
    /// which must describe the project as it stands now, saved. `database` is
    /// the host's world database; see [`ToPeer::Welcome`]. Answers what this
    /// editor is to do, which is the [`Action::Joined`].
    pub fn admit(&mut self, user: UserId, project: String, files: Vec<FileEntry>, database: Option<Database>) -> Vec<Action<P>> {
        let outs = self.host.admit(user, database);
        let mut actions = Vec::new();
        self.carry(outs, None, &mut actions);
        self.net.send(user, ToPeer::Manifest { project, files });
        actions
    }

    /// Put an editor out: it is told why and its connection closed.
    pub fn remove(&mut self, user: UserId, reason: &str) -> Vec<Action<P>> {
        self.net.send(user, ToPeer::Refused(reason.to_string()));
        self.net.close(user);
        let outs = self.host.disconnect(user);
        let mut actions = Vec::new();
        self.carry(outs, None, &mut actions);
        actions
    }

    /// Send an editor a line for its status line: the answer to its
    /// [`Action::Ask`].
    pub fn note(&self, user: UserId, line: String) {
        self.net.send(user, ToPeer::Note(line));
    }

    /// Send files an editor fetched, from a thread of their own, in
    /// [`FILE_PIECE`] pieces and then [`ToPeer::FilesDone`]. Each is the
    /// virtual path and where it is on disk. A file that cannot be read is
    /// named in `missing`.
    pub fn serve(&self, user: UserId, files: Vec<(String, std::path::PathBuf)>) {
        let Some(send) = self.net.sender(user) else { return };
        let spawned = std::thread::Builder::new().name(format!("collab-serve-{user}")).spawn(move || {
            let mut missing = Vec::new();
            for (path, on_disk) in files {
                let Ok(bytes) = std::fs::read(&on_disk) else {
                    missing.push(path);
                    continue;
                };
                let total = bytes.len() as u64;
                let mut offset = 0;
                // An empty file is one empty piece, so it is still written.
                loop {
                    let end = (offset + FILE_PIECE).min(bytes.len());
                    let piece = ToPeer::File { path: path.clone(), offset: offset as u64, total, bytes: bytes[offset..end].to_vec() };
                    if send.send(piece).is_err() {
                        return;
                    }
                    offset = end;
                    if offset >= bytes.len() {
                        break;
                    }
                }
            }
            let _ = send.send(ToPeer::FilesDone { missing });
        });
        if spawned.is_err() {
            self.net.close(user);
        }
    }

    /// Carry everything waiting, from the network and from this editor's
    /// peer, and answer what this editor is to do. `open` is the edit being
    /// made now; see [`Peer::receive`].
    pub fn poll(&mut self, open: Option<&P>) -> Vec<Action<P>> {
        let mut outs = Vec::new();
        for event in self.net.poll() {
            match event {
                Event::Connected(user, _, binding) => self.host.connect(user, binding),
                Event::Message(user, message) => outs.extend(self.host.receive(user, message)),
                Event::Closed(user, _) => outs.extend(self.host.disconnect(user)),
            }
        }
        let mut actions = Vec::new();
        self.carry(outs, open, &mut actions);
        actions
    }

    /// Route the host's answers, and this editor's own messages, until none is
    /// left. The host's peer answers at once, so one call settles its edits.
    fn carry(&mut self, mut outs: Vec<Out<P>>, open: Option<&P>, actions: &mut Vec<Action<P>>) {
        loop {
            for message in self.me.take_outbox() {
                outs.extend(self.host.receive(0, message));
            }
            if outs.is_empty() {
                return;
            }
            for out in std::mem::take(&mut outs) {
                match out {
                    Out::To(0, message) => actions.extend(self.me.receive(message, open)),
                    Out::To(user, message) => self.net.send(user, message),
                    Out::Close(user) => self.net.close(user),
                    Out::Admit { user, name } => actions.push(Action::Admit { user, name }),
                    Out::Fetch { user, paths } => actions.push(Action::Fetch { user, paths }),
                    Out::Ask { user, payload } => actions.push(Action::Ask { user, payload }),
                    Out::Log { seq, user, author, payload } => actions.push(Action::Log { seq, user, author, payload }),
                    Out::Request { user, author, request } => actions.push(Action::Request { user, author, request }),
                }
            }
        }
    }
}

/// In a session another editor hosts. Dropping it leaves.
pub struct Joined<P: Payload> {
    me: Peer<P>,
    net: Connection<ToPeer<P>, ToHost<P>>,
    closed: bool,
}

impl<P: Payload> Joined<P> {
    /// Connect to the host at `addr` as `author`. The handshake runs on the
    /// following polls; [`Action::Welcome`] or [`Action::Refused`] ends it.
    pub fn connect(addr: &str, author: &Author, password: &str, build: &str, archives: [u8; 32]) -> std::io::Result<Joined<P>> {
        let net = Connection::open(addr)?;
        let me = Peer::joining(author, password, build, archives, net.binding());
        let mut joined = Joined { me, net, closed: false };
        joined.flush();
        Ok(joined)
    }

    /// See [`Hosting::poll`]. The connection ending is [`Action::Closed`],
    /// once.
    pub fn poll(&mut self, open: Option<&P>) -> Vec<Action<P>> {
        let mut actions = Vec::new();
        for received in self.net.poll() {
            match received {
                Ok(message) => actions.extend(self.me.receive(message, open)),
                Err(e) if !self.closed => {
                    self.closed = true;
                    actions.push(Action::Closed(e));
                }
                Err(_) => {}
            }
        }
        self.flush();
        actions
    }

    fn flush(&mut self) {
        for message in self.me.take_outbox() {
            self.net.send(message);
        }
    }
}

impl<P: Payload> Drop for Joined<P> {
    fn drop(&mut self) {
        self.me.leave();
        self.flush();
    }
}

/// Hosting or joined. An editor holds one, so the size of the larger variant
/// costs nothing.
#[allow(clippy::large_enum_variant)]
pub enum Session<P: Payload> {
    Hosting(Hosting<P>),
    Joined(Joined<P>),
}

impl<P: Payload> Session<P> {
    fn peer(&mut self) -> &mut Peer<P> {
        match self {
            Session::Hosting(hosting) => &mut hosting.me,
            Session::Joined(joined) => &mut joined.me,
        }
    }

    /// This editor's peer, for its number and its sequence.
    pub fn me(&self) -> &Peer<P> {
        match self {
            Session::Hosting(hosting) => &hosting.me,
            Session::Joined(joined) => &joined.me,
        }
    }

    /// Send an edit this editor has made and applied. It is carried on the
    /// next [`Self::poll`].
    pub fn propose(&mut self, payload: P) {
        self.peer().propose(payload);
    }

    /// Ask the host's editor to make an edit, on the next poll. See
    /// [`Peer::ask`].
    pub fn ask(&mut self, payload: P) {
        self.peer().ask(payload);
    }

    /// Say where this editor is, on the next poll.
    pub fn presence(&mut self, presence: Presence) {
        self.peer().presence(presence);
    }

    /// Ask for the files of the manifest this editor lacks, on the next poll.
    pub fn fetch(&mut self, paths: Vec<String>) {
        self.peer().fetch(paths);
    }

    /// Ask for a block of numbers, on the next poll.
    pub fn ask_for_ids(&mut self, kind: &str, floor: u64, size: u64) {
        self.peer().ask_for_ids(kind, floor, size);
    }

    /// This editor's edits the host has not answered, oldest first, taken
    /// out of the peer. For a connection that is gone: whether the host
    /// accepted them is not known.
    pub fn take_waiting(&mut self) -> Vec<P> {
        self.peer().take_waiting()
    }

    /// Check these out, on the next poll. See [`crate::checkout`].
    pub fn check_out(&mut self, claims: Vec<Claim>) {
        self.peer().check_out(claims);
    }

    /// Release these check outs, on the next poll; with `others`, also
    /// another author's.
    pub fn release(&mut self, claims: Vec<Claim>, others: bool) {
        self.peer().release(claims, others);
    }

    /// Ask the host about its log, on the next poll. Only a joined editor
    /// asks; the host's editor reads its own log.
    pub fn request(&mut self, request: Request) {
        self.peer().request(request);
    }

    /// See [`Hosting::poll`].
    pub fn poll(&mut self, open: Option<&P>) -> Vec<Action<P>> {
        match self {
            Session::Hosting(hosting) => hosting.poll(open),
            Session::Joined(joined) => joined.poll(open),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::tests::{chunk, toy, Toy};
    use std::time::{Duration, Instant};

    /// Poll every session until `done` holds for the actions gathered, or two
    /// seconds pass.
    fn until(sessions: &mut [&mut Session<Toy>], done: impl Fn(&[Vec<Action<Toy>>]) -> bool) -> Vec<Vec<Action<Toy>>> {
        let mut gathered = vec![Vec::new(); sessions.len()];
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            for (at, session) in sessions.iter_mut().enumerate() {
                let actions = session.poll(None);
                // The host lets each editor in at once, with an empty project.
                for action in &actions {
                    if let (Action::Admit { user, .. }, Session::Hosting(hosting)) = (action, &mut **session) {
                        gathered[at].extend(hosting.admit(*user, "Test".into(), Vec::new(), None));
                    }
                }
                gathered[at].extend(actions);
            }
            if done(&gathered) {
                return gathered;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out with {gathered:?}");
    }

    /// Over loopback TCP: two editors join a host, a wrong password is
    /// refused, an edit by one reaches the other two, and two edits to one
    /// chunk made at once end with one accepted everywhere and the other
    /// reverted by the editor that made it.
    #[test]
    fn two_editors_join_and_share_edits_over_tcp() {
        let me = |id: u8, name: &str| Author { id: [id; 16], name: name.to_string() };
        let mut host = Session::Hosting(Hosting::<Toy>::start("127.0.0.1:0", &me(0, "Host"), "pw", "b1", [1; 32]).unwrap());
        let addr = match &host {
            Session::Hosting(hosting) => hosting.addr().to_string(),
            Session::Joined(_) => unreachable!(),
        };
        let mut alex = Session::Joined(Joined::connect(&addr, &me(1, "Alex"), "pw", "b1", [1; 32]).unwrap());
        let mut sam = Session::Joined(Joined::connect(&addr, &me(2, "Sam"), "pw", "b1", [1; 32]).unwrap());
        let mut eve = Session::Joined(Joined::connect(&addr, &me(3, "Eve"), "guess", "b1", [1; 32]).unwrap());
        let welcomed = |actions: &[Action<Toy>]| actions.iter().any(|action| matches!(action, Action::Welcome { .. }));
        let seen = until(&mut [&mut host, &mut alex, &mut sam, &mut eve], |all| {
            welcomed(&all[1]) && welcomed(&all[2]) && all[3].iter().any(|action| matches!(action, Action::Refused(_)))
        });
        assert!(!welcomed(&seen[3]));

        alex.propose(toy("raise", &[chunk(3)]));
        let applied = |actions: &[Action<Toy>], name: &str| {
            actions.iter().any(|action| matches!(action, Action::Apply { payload, .. } if payload.name == name))
        };
        until(&mut [&mut host, &mut alex, &mut sam], |all| applied(&all[0], "raise") && applied(&all[2], "raise"));

        // Both edit chunk 4 before either hears of the other.
        alex.propose(toy("alex", &[chunk(4)]));
        sam.propose(toy("sam", &[chunk(4)]));
        let all = until(&mut [&mut host, &mut alex, &mut sam], |all| {
            let reverted = |actions: &[Action<Toy>]| actions.iter().any(|action| matches!(action, Action::Revert { .. }));
            (applied(&all[0], "alex") || applied(&all[0], "sam")) && (reverted(&all[1]) || reverted(&all[2]))
        });
        let winner = if applied(&all[0], "alex") { "alex" } else { "sam" };
        let loser = if winner == "alex" { 2 } else { 1 };
        assert!(applied(&all[loser], winner), "the editor whose edit lost applies the winner's: {all:?}");
        assert!(!applied(&all[0], if winner == "alex" { "sam" } else { "alex" }));
        let start = Instant::now();
        while alex.me().waiting() + sam.me().waiting() > 0 && start.elapsed() < Duration::from_secs(2) {
            for session in [&mut host, &mut alex, &mut sam] {
                session.poll(None);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(alex.me().waiting() + sam.me().waiting(), 0, "both edits are answered");
    }
}
