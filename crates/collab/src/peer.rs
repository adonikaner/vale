//! Every editor's half of a session, the host's own included: its edits that
//! the host has not accepted yet, and which of them a remote edit makes the
//! host refuse.
//!
//! ## Waiting edits
//!
//! An edit is applied here when it is made and sent with `base`, the last
//! accepted edit this editor has received. It waits until the host answers.
//! The host answers in the order it decides, and every message reaches this
//! editor in that order. So when a remote edit arrives that shares a
//! [`Group`] with a waiting edit, the host has already accepted the remote
//! edit and will refuse the waiting one. The waiting edit, and every waiting
//! edit after it, is reverted at once, newest first, and the epoch goes up;
//! the host refuses those edits by the old epoch. Only then is the remote
//! edit applied, onto the state it was made against.
//!
//! The waiting edits after the conflicting one may not conflict with
//! anything, and are reverted anyway: each was made on the state the ones
//! before it left. The same holds for the edit being made now, the open
//! gesture, which is passed to [`Peer::receive`] for that reason.
//!
//! ## Forced edits
//!
//! The host's own editor can send an edit that cannot be reverted
//! ([`crate::groups::Payload::forced`]): a file deleted with the unsaved edits
//! to it dropped. The host accepts it whatever came before it, so it is never
//! reverted here: a revert skips it and it stays waiting.
//!
//! A remote edit can still arrive over a waiting forced edit. The host put the
//! remote edit first, since its message came before the forced edit's
//! acceptance, so the forced edit's result is the one that stands. The remote
//! edit is applied and then [`Action::Reapply`] applies the forced edit again
//! over it. A forced edit carries the whole result, the bytes of the files it
//! writes, so applying it twice leaves what applying it once does.

use crate::auth;
use crate::checkout::{Claim, Hold};
use crate::groups::{Group, Payload};
use crate::log::Author;
use crate::wire::{Answer, Database, FileEntry, Presence, Request, Seq, ToHost, ToPeer, UserId, PROTOCOL};
use std::collections::VecDeque;

/// Why one of this editor's edits is reverted.
#[derive(Debug, Clone, PartialEq)]
pub enum Cause {
    /// This user's edit, accepted first, changed something it touches, or
    /// something an edit before it touched.
    Changed { user: UserId },
    /// The host refused it, for this reason.
    Refused(String),
}

/// What the editor does with a message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Action<P> {
    /// This editor is in, as `user`, with these other editors. `database` is
    /// the host's world database; see [`ToPeer::Welcome`].
    Welcome { user: UserId, seq: Seq, log: [u8; 16], users: Vec<(UserId, String)>, database: Option<Database> },
    /// This editor is not let in, or is put out.
    Refused(String),
    /// Revert the edit being made now and end the gesture. See
    /// [`Peer::receive`].
    CancelOpen { cause: Cause },
    /// Revert one of this editor's own edits. Several come newest first.
    Revert { payload: P, cause: Cause },
    /// Apply another editor's edit.
    Apply { user: UserId, payload: P },
    /// Apply one of this editor's own forced edits again, over the remote edit
    /// applied just before. See *Forced edits*.
    Reapply { payload: P },
    Presence { user: UserId, presence: Presence },
    /// A line from the host's editor for the status line.
    Note(String),
    Joined { user: UserId, name: String },
    Left { user: UserId },
    /// The connection to the host is gone.
    Closed(String),
    /// The host's project, sent after the welcome. The editor fetches what it
    /// lacks with [`Peer::fetch`].
    Manifest { project: String, files: Vec<FileEntry> },
    /// A piece of a fetched file. See [`ToPeer::File`].
    File { path: String, offset: u64, total: u64, bytes: Vec<u8> },
    /// Every fetched file is sent; `missing` could not be.
    FilesDone { missing: Vec<String> },
    /// The host's editor only: an editor waits to be let in. See
    /// [`crate::host::Out::Admit`].
    Admit { user: UserId, name: String },
    /// The host's editor only: an editor asks for files.
    Fetch { user: UserId, paths: Vec<String> },
    /// The host's editor only: an editor asks it to make an edit. See
    /// [`ToHost::Ask`].
    Ask { user: UserId, payload: P },
    /// The host's editor only: an edit was accepted as `seq`, to log. See
    /// [`crate::host::Out::Log`].
    Log { seq: Seq, user: UserId, author: Author, payload: P },
    /// The host's editor only: an editor asks about the log, to answer with
    /// [`crate::session::Hosting::answer`].
    Request { user: UserId, author: Author, request: Request },
    /// The host's answer to this editor's [`Peer::request`].
    Answer(Answer<P>),
    /// Every claim held. See [`crate::checkout`].
    Holds(Vec<Hold>),
    /// The numbers `start..end` of `kind` are this editor's to give out. See
    /// [`crate::ids`].
    Ids { kind: String, start: u64, end: u64 },
}

#[derive(Debug)]
struct Waiting<P> {
    id: u64,
    groups: Vec<Group>,
    payload: P,
    /// Sent by the host's editor as forced. See *Forced edits*.
    forced: bool,
}

/// One editor's half of a session. See the module comment.
#[derive(Debug)]
pub struct Peer<P> {
    name: String,
    key: [u8; 32],
    user: Option<UserId>,
    seq: Seq,
    epoch: u32,
    next_id: u64,
    waiting: VecDeque<Waiting<P>>,
    outbox: Vec<ToHost<P>>,
    /// The connection's binding, which a challenge is answered over.
    binding: [u8; 32],
}

impl<P: Payload> Peer<P> {
    /// An editor joining as `author`, with the password, its build and its
    /// archives' fingerprint, over a connection with `binding` (see
    /// [`crate::auth`]). Its first message, the hello, is in the outbox.
    pub fn joining(author: &Author, password: &str, build: &str, archives: [u8; 32], binding: [u8; 32]) -> Peer<P> {
        let mut peer = Peer::new(&author.name, password, None);
        peer.binding = binding;
        let hello = ToHost::Hello { protocol: PROTOCOL, name: author.name.clone(), author: author.id, build: build.to_string(), archives };
        peer.outbox.push(hello);
        peer
    }

    /// The host's own editor, which is in from the start as `user`.
    pub fn local(name: &str, user: UserId) -> Peer<P> {
        Peer::new(name, "", Some(user))
    }

    fn new(name: &str, password: &str, user: Option<UserId>) -> Peer<P> {
        Peer {
            name: name.to_string(),
            key: auth::key(password),
            user,
            seq: 0,
            epoch: 0,
            next_id: 1,
            waiting: VecDeque::new(),
            outbox: Vec::new(),
            binding: [0; 32],
        }
    }

    /// This editor's number, once it is in.
    pub fn user(&self) -> Option<UserId> {
        self.user
    }

    /// The last accepted edit this editor has.
    pub fn seq(&self) -> Seq {
        self.seq
    }

    /// Start the host's own editor at the host's resumed number. See
    /// [`crate::host::Host::resume`].
    pub fn resume(&mut self, seq: Seq) {
        self.seq = seq;
    }

    /// How many of this editor's edits the host has not answered.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Take the edits the host has not answered, oldest first.
    pub fn take_waiting(&mut self) -> Vec<P> {
        self.waiting.drain(..).map(|waiting| waiting.payload).collect()
    }

    /// Send an edit this editor has made and applied.
    pub fn propose(&mut self, payload: P) {
        let id = self.next_id;
        self.next_id += 1;
        let groups = payload.groups();
        let forced = self.user == Some(0) && payload.forced();
        self.outbox.push(ToHost::Propose { id, epoch: self.epoch, base: self.seq, payload: payload.clone() });
        self.waiting.push_back(Waiting { id, groups, payload, forced });
    }

    /// Ask the host's editor to make an edit this editor may not. See
    /// [`ToHost::Ask`].
    pub fn ask(&mut self, payload: P) {
        self.outbox.push(ToHost::Ask(payload));
    }

    /// Say where this editor is.
    /// Nothing is sent before the welcome: the host would have no one to
    /// pass it to.
    pub fn presence(&mut self, presence: Presence) {
        if self.user.is_some() {
            self.outbox.push(ToHost::Presence(presence));
        }
    }

    /// Ask for a block of numbers. See [`crate::ids`].
    pub fn ask_for_ids(&mut self, kind: &str, floor: u64, size: u64) {
        self.outbox.push(ToHost::Ids { kind: kind.to_string(), floor, size });
    }

    /// Ask for the files of the manifest this editor lacks.
    pub fn fetch(&mut self, paths: Vec<String>) {
        self.outbox.push(ToHost::Fetch(paths));
    }

    /// Ask the host about its log.
    pub fn request(&mut self, request: Request) {
        self.outbox.push(ToHost::Request(request));
    }

    /// Check these out. See [`crate::checkout`].
    pub fn check_out(&mut self, claims: Vec<Claim>) {
        self.outbox.push(ToHost::CheckOut(claims));
    }

    /// Release these check outs; with `others`, also another author's.
    pub fn release(&mut self, claims: Vec<Claim>, others: bool) {
        self.outbox.push(ToHost::Release { claims, others });
    }

    /// Leave the session.
    pub fn leave(&mut self) {
        self.outbox.push(ToHost::Bye);
    }

    /// The messages to send the host, in order.
    pub fn take_outbox(&mut self) -> Vec<ToHost<P>> {
        std::mem::take(&mut self.outbox)
    }

    /// One message from the host, and what the editor is to do. `open` is the
    /// edit the editor is in the middle of making, if any: it is reverted
    /// when a remote edit conflicts with it, and whenever waiting edits are
    /// reverted, since it was made on top of them.
    pub fn receive(&mut self, message: ToPeer<P>, open: Option<&P>) -> Vec<Action<P>> {
        match message {
            ToPeer::Challenge { nonce } => {
                self.outbox.push(ToHost::Proof(auth::proof(&self.key, &auth::bound(&nonce, &self.binding), &self.name)));
                Vec::new()
            }
            ToPeer::Welcome { user, seq, log, users, database } => {
                self.user = Some(user);
                self.seq = seq;
                vec![Action::Welcome { user, seq, log, users, database }]
            }
            ToPeer::Refused(reason) => vec![Action::Refused(reason)],
            ToPeer::Accepted { id, seq } => {
                self.waiting.retain(|waiting| waiting.id != id);
                self.seq = self.seq.max(seq);
                Vec::new()
            }
            ToPeer::Rejected { id, reason } => match self.waiting.iter().position(|waiting| waiting.id == id) {
                // The host accepts every forced edit, so this is not expected;
                // one that cannot be reverted is only removed from the waiting edits.
                Some(at) if self.waiting[at].forced => {
                    self.waiting.remove(at);
                    Vec::new()
                }
                Some(at) => self.revert_from(at, open, Cause::Refused(reason)),
                // Already reverted when the conflicting edit arrived.
                None => Vec::new(),
            },
            ToPeer::Change { seq, user, payload } => {
                let groups = payload.groups();
                let cause = Cause::Changed { user };
                let first = self.waiting.iter().position(|waiting| !waiting.forced && overlaps(&waiting.groups, &groups));
                let mut out = match first {
                    Some(at) => self.revert_from(at, open, cause),
                    None => match open.is_some_and(|open| overlaps(&open.groups(), &groups)) {
                        true => vec![Action::CancelOpen { cause }],
                        false => Vec::new(),
                    },
                };
                self.seq = seq;
                let again: Vec<Action<P>> = self
                    .waiting
                    .iter()
                    .filter(|waiting| waiting.forced && overlaps(&waiting.groups, &groups))
                    .map(|waiting| Action::Reapply { payload: waiting.payload.clone() })
                    .collect();
                out.push(Action::Apply { user, payload });
                out.extend(again);
                out
            }
            ToPeer::Ids { kind, start, end } => vec![Action::Ids { kind, start, end }],
            ToPeer::Presence { user, presence } => vec![Action::Presence { user, presence }],
            ToPeer::Note(line) => vec![Action::Note(line)],
            ToPeer::Answer(answer) => vec![Action::Answer(answer)],
            ToPeer::Holds(list) => vec![Action::Holds(list)],
            ToPeer::Manifest { project, files } => vec![Action::Manifest { project, files }],
            ToPeer::File { path, offset, total, bytes } => vec![Action::File { path, offset, total, bytes }],
            ToPeer::FilesDone { missing } => vec![Action::FilesDone { missing }],
            ToPeer::Joined { user, name } => vec![Action::Joined { user, name }],
            ToPeer::Left { user } => vec![Action::Left { user }],
        }
    }

    /// Revert the open gesture and the waiting edits from `at` on, newest
    /// first, and go to the next epoch. A forced edit among them stays
    /// waiting; see *Forced edits*.
    fn revert_from(&mut self, at: usize, open: Option<&P>, cause: Cause) -> Vec<Action<P>> {
        let mut out = Vec::new();
        if open.is_some() {
            out.push(Action::CancelOpen { cause: cause.clone() });
        }
        let later: Vec<Waiting<P>> = self.waiting.drain(at..).collect();
        let mut kept = Vec::new();
        for waiting in later.into_iter().rev() {
            match waiting.forced {
                true => kept.push(waiting),
                false => out.push(Action::Revert { payload: waiting.payload, cause: cause.clone() }),
            }
        }
        self.waiting.extend(kept.into_iter().rev());
        self.epoch += 1;
        out
    }
}

/// Whether two sorted group lists share a group.
fn overlaps(a: &[Group], b: &[Group]) -> bool {
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => return true,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::tests::{chunk, host_author, toy, Toy};
    use crate::host::{Host, Out};

    /// Carry every message between a host and its local peer 0 and a peer 1
    /// that is already in, until none is left. Answers each peer's actions.
    fn settle(host: &mut Host, peers: &mut [Peer<Toy>; 2], open: [Option<&Toy>; 2]) -> [Vec<Action<Toy>>; 2] {
        let mut actions = [Vec::new(), Vec::new()];
        loop {
            let mut outs = Vec::new();
            for (user, peer) in peers.iter_mut().enumerate() {
                for message in peer.take_outbox() {
                    outs.extend(host.receive(user as UserId, message));
                }
            }
            if outs.is_empty() {
                return actions;
            }
            let mut admitted = Vec::new();
            for out in outs {
                match out {
                    Out::To(user, message) => {
                        let at = user as usize;
                        actions[at].extend(peers[at].receive(message, open[at]));
                    }
                    Out::Admit { user, .. } => admitted.extend(host.admit::<Toy>(user, None)),
                    _ => {}
                }
            }
            for out in admitted {
                if let Out::To(user, message) = out {
                    let at = user as usize;
                    actions[at].extend(peers[at].receive(message, open[at]));
                }
            }
        }
    }

    /// The actions about edits, without the claims list every accepted edit
    /// sends.
    fn edits(actions: &[Action<Toy>]) -> Vec<Action<Toy>> {
        actions.iter().filter(|action| !matches!(action, Action::Holds(_))).cloned().collect()
    }

    fn two() -> (Host, [Peer<Toy>; 2]) {
        let mut host = Host::new(&host_author(), "pw", "b1", [1; 32]);
        host.connect(1, [0; 32]);
        let alex = Author { id: [1; 16], name: "Alex".to_string() };
        let mut peers = [Peer::local("Host", 0), Peer::joining(&alex, "pw", "b1", [1; 32], [0; 32])];
        let actions = settle(&mut host, &mut peers, [None, None]);
        assert!(matches!(actions[1][..], [Action::Welcome { user: 1, .. }, Action::Holds(_)]), "{actions:?}");
        (host, peers)
    }

    /// Two editors edit one chunk at once. The host's edit is accepted first;
    /// Alex reverts his edit and the disjoint one he made after it, newest
    /// first, then applies the host's. Both end at the host's sequence and
    /// Alex's next edit is accepted.
    #[test]
    fn a_conflicting_remote_edit_takes_back_the_waiting_edits_first() {
        let (mut host, mut peers) = two();
        peers[0].propose(toy("host", &[chunk(3)]));
        // Alex's two edits are made before the host's reaches him.
        peers[1].propose(toy("a", &[chunk(3)]));
        peers[1].propose(toy("b", &[chunk(9)]));
        let host_out = peers[0].take_outbox();
        let mut outs = Vec::new();
        for message in host_out {
            outs.extend(host.receive(0, message));
        }
        let mut alex = Vec::new();
        for out in outs {
            if let Out::To(1, message) = out {
                alex.extend(peers[1].receive(message, None));
            } else if let Out::To(0, message) = out {
                peers[0].receive(message, None);
            }
        }
        assert_eq!(
            edits(&alex),
            vec![
                Action::Revert { payload: toy("b", &[chunk(9)]), cause: Cause::Changed { user: 0 } },
                Action::Revert { payload: toy("a", &[chunk(3)]), cause: Cause::Changed { user: 0 } },
                Action::Apply { user: 0, payload: toy("host", &[chunk(3)]) },
            ]
        );
        assert_eq!(peers[1].waiting(), 0);
        // The host refuses both of Alex's edits; Alex has already taken them back.
        let actions = settle(&mut host, &mut peers, [None, None]);
        assert!(actions[1].is_empty(), "{actions:?}");

        // Once the host submits, chunk 3 is not held.
        host.release_author::<Toy>(&[0xA0; 16]);
        peers[1].propose(toy("c", &[chunk(3)]));
        let actions = settle(&mut host, &mut peers, [None, None]);
        assert_eq!(edits(&actions[0]), vec![Action::Apply { user: 1, payload: toy("c", &[chunk(3)]) }]);
        assert_eq!((peers[0].seq(), peers[1].seq(), host.seq()), (2, 2, 2));
    }

    /// A remote edit that conflicts with nothing waiting but with the open
    /// gesture cancels the gesture; one that conflicts with neither is only
    /// applied.
    #[test]
    fn a_remote_edit_over_the_open_gesture_cancels_it() {
        let (mut host, mut peers) = two();
        let open = toy("stroke", &[chunk(5)]);
        peers[0].propose(toy("host", &[chunk(5)]));
        let actions = settle(&mut host, &mut peers, [None, Some(&open)]);
        assert_eq!(
            edits(&actions[1]),
            vec![Action::CancelOpen { cause: Cause::Changed { user: 0 } }, Action::Apply { user: 0, payload: toy("host", &[chunk(5)]) }]
        );
        peers[0].propose(toy("other", &[chunk(6)]));
        let actions = settle(&mut host, &mut peers, [None, Some(&open)]);
        assert_eq!(edits(&actions[1]), vec![Action::Apply { user: 0, payload: toy("other", &[chunk(6)]) }]);
    }

    /// The host's forced edit waits while Alex's edit over the same chunk is
    /// accepted first. The host's editor applies Alex's edit and then its own
    /// forced edit again, takes nothing back, and Alex applies the forced
    /// edit after his own.
    #[test]
    fn a_forced_edit_is_applied_again_over_a_remote_edit_put_before_it() {
        use crate::host::tests::forced;
        let (mut host, mut peers) = two();
        peers[0].propose(forced("delete", &[chunk(3)]));
        peers[1].propose(toy("a", &[chunk(3)]));
        let mut outs = Vec::new();
        for message in peers[1].take_outbox() {
            outs.extend(host.receive(1, message));
        }
        let mut local = Vec::new();
        for out in outs {
            match out {
                Out::To(0, message) => local.extend(peers[0].receive(message, None)),
                Out::To(1, message) => {
                    peers[1].receive(message, None);
                }
                _ => {}
            }
        }
        assert_eq!(
            edits(&local),
            vec![
                Action::Apply { user: 1, payload: toy("a", &[chunk(3)]) },
                Action::Reapply { payload: forced("delete", &[chunk(3)]) },
            ]
        );
        assert_eq!(peers[0].waiting(), 1, "the forced edit still waits for its answer");
        let actions = settle(&mut host, &mut peers, [None, None]);
        assert!(edits(&actions[0]).is_empty(), "{actions:?}");
        assert_eq!(edits(&actions[1]), vec![Action::Apply { user: 0, payload: forced("delete", &[chunk(3)]) }]);
        assert_eq!((peers[0].waiting(), host.seq()), (0, 2));
    }

    #[test]
    fn overlap_is_found_in_sorted_lists() {
        assert!(overlaps(&[chunk(1), chunk(4)], &[chunk(2), chunk(4)]));
        assert!(!overlaps(&[chunk(1), chunk(3)], &[chunk(2), chunk(4)]));
        assert!(!overlaps(&[], &[chunk(2)]));
    }
}
