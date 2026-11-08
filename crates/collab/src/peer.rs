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
//! edit after it, is taken back at once, newest first, and the epoch goes up;
//! the host refuses those edits by the old epoch. Only then is the remote
//! edit applied, onto the state it was made against.
//!
//! The waiting edits after the conflicting one may not conflict with
//! anything, and are taken back anyway: each was made on the state the ones
//! before it left. The same holds for the edit being made now, the open
//! gesture, which is passed to [`Peer::receive`] for that reason.

use crate::auth;
use crate::groups::{Group, Payload};
use crate::ids::Blocks;
use crate::wire::{FileEntry, Presence, Seq, ToHost, ToPeer, UserId, PROTOCOL};
use std::collections::VecDeque;

/// Why one of this editor's edits is taken back.
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
    /// This editor is in, as `user`, with these others.
    Welcome { user: UserId, users: Vec<(UserId, String)> },
    /// This editor is not let in, or is put out.
    Refused(String),
    /// Take back the edit being made now and end the gesture. See
    /// [`Peer::receive`].
    CancelOpen { cause: Cause },
    /// Take back one of this editor's own edits. Several come newest first.
    Revert { payload: P, cause: Cause },
    /// Apply another editor's edit.
    Apply { user: UserId, payload: P },
    Presence { user: UserId, presence: Presence },
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
}

#[derive(Debug)]
struct Waiting<P> {
    id: u64,
    groups: Vec<Group>,
    payload: P,
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
    /// The numbers this editor may give out. See [`crate::ids`].
    pub blocks: Blocks,
}

impl<P: Payload> Peer<P> {
    /// An editor joining under `name`, with the password, its build and its
    /// archives' fingerprint. Its first message, the hello, is in the outbox.
    pub fn joining(name: &str, password: &str, build: &str, archives: [u8; 32]) -> Peer<P> {
        let mut peer = Peer::new(name, password, None);
        peer.outbox.push(ToHost::Hello { protocol: PROTOCOL, name: name.to_string(), build: build.to_string(), archives });
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
            blocks: Blocks::default(),
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

    /// How many of this editor's edits the host has not answered.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Send an edit this editor has made and applied.
    pub fn propose(&mut self, payload: P) {
        let id = self.next_id;
        self.next_id += 1;
        let groups = payload.groups();
        self.outbox.push(ToHost::Propose { id, epoch: self.epoch, base: self.seq, payload: payload.clone() });
        self.waiting.push_back(Waiting { id, groups, payload });
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

    /// Leave the session.
    pub fn leave(&mut self) {
        self.outbox.push(ToHost::Bye);
    }

    /// The messages to send the host, in order.
    pub fn take_outbox(&mut self) -> Vec<ToHost<P>> {
        std::mem::take(&mut self.outbox)
    }

    /// One message from the host, and what the editor is to do. `open` is the
    /// edit the editor is in the middle of making, if any: it is taken back
    /// when a remote edit conflicts with it, and whenever waiting edits are
    /// taken back, since it was made on top of them.
    pub fn receive(&mut self, message: ToPeer<P>, open: Option<&P>) -> Vec<Action<P>> {
        match message {
            ToPeer::Challenge { nonce } => {
                self.outbox.push(ToHost::Proof(auth::proof(&self.key, &nonce, &self.name)));
                Vec::new()
            }
            ToPeer::Welcome { user, seq, users } => {
                self.user = Some(user);
                self.seq = seq;
                vec![Action::Welcome { user, users }]
            }
            ToPeer::Refused(reason) => vec![Action::Refused(reason)],
            ToPeer::Accepted { id, seq } => {
                self.waiting.retain(|waiting| waiting.id != id);
                self.seq = self.seq.max(seq);
                Vec::new()
            }
            ToPeer::Rejected { id, reason } => match self.waiting.iter().position(|waiting| waiting.id == id) {
                Some(at) => self.take_back(at, open, Cause::Refused(reason)),
                // Already taken back when the conflicting edit arrived.
                None => Vec::new(),
            },
            ToPeer::Change { seq, user, payload } => {
                let groups = payload.groups();
                let cause = Cause::Changed { user };
                let mut out = match self.waiting.iter().position(|waiting| overlaps(&waiting.groups, &groups)) {
                    Some(at) => self.take_back(at, open, cause),
                    None => match open.is_some_and(|open| overlaps(&open.groups(), &groups)) {
                        true => vec![Action::CancelOpen { cause }],
                        false => Vec::new(),
                    },
                };
                self.seq = seq;
                out.push(Action::Apply { user, payload });
                out
            }
            ToPeer::Ids { kind, start, end } => {
                self.blocks.hold(&kind, start, end);
                Vec::new()
            }
            ToPeer::Presence { user, presence } => vec![Action::Presence { user, presence }],
            ToPeer::Manifest { project, files } => vec![Action::Manifest { project, files }],
            ToPeer::File { path, offset, total, bytes } => vec![Action::File { path, offset, total, bytes }],
            ToPeer::FilesDone { missing } => vec![Action::FilesDone { missing }],
            ToPeer::Joined { user, name } => vec![Action::Joined { user, name }],
            ToPeer::Left { user } => vec![Action::Left { user }],
        }
    }

    /// Take back the open gesture and the waiting edits from `at` on, newest
    /// first, and go to the next epoch.
    fn take_back(&mut self, at: usize, open: Option<&P>, cause: Cause) -> Vec<Action<P>> {
        let mut out = Vec::new();
        if open.is_some() {
            out.push(Action::CancelOpen { cause: cause.clone() });
        }
        for waiting in self.waiting.drain(at..).rev() {
            out.push(Action::Revert { payload: waiting.payload, cause: cause.clone() });
        }
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
    use crate::host::tests::{chunk, toy, Toy};
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
                    Out::Admit { user, .. } => admitted.extend(host.admit::<Toy>(user)),
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

    fn two() -> (Host, [Peer<Toy>; 2]) {
        let mut host = Host::new("Host", "pw", "b1", [1; 32]);
        host.connect(1);
        let mut peers = [Peer::local("Host", 0), Peer::joining("Alex", "pw", "b1", [1; 32])];
        let actions = settle(&mut host, &mut peers, [None, None]);
        assert!(matches!(actions[1][..], [Action::Welcome { user: 1, .. }]), "{actions:?}");
        (host, peers)
    }

    /// Two editors edit one chunk at once. The host's edit is accepted first;
    /// Alex takes back his edit and the disjoint one he made after it, newest
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
            alex,
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

        peers[1].propose(toy("c", &[chunk(3)]));
        let actions = settle(&mut host, &mut peers, [None, None]);
        assert_eq!(actions[0], vec![Action::Apply { user: 1, payload: toy("c", &[chunk(3)]) }]);
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
            actions[1],
            vec![Action::CancelOpen { cause: Cause::Changed { user: 0 } }, Action::Apply { user: 0, payload: toy("host", &[chunk(5)]) }]
        );
        peers[0].propose(toy("other", &[chunk(6)]));
        let actions = settle(&mut host, &mut peers, [None, Some(&open)]);
        assert_eq!(actions[1], vec![Action::Apply { user: 0, payload: toy("other", &[chunk(6)]) }]);
    }

    #[test]
    fn overlap_is_found_in_sorted_lists() {
        assert!(overlaps(&[chunk(1), chunk(4)], &[chunk(2), chunk(4)]));
        assert!(!overlaps(&[chunk(1), chunk(3)], &[chunk(2), chunk(4)]));
        assert!(!overlaps(&[], &[chunk(2)]));
    }
}
