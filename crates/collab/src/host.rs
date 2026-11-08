//! The host's half of a session: who is in, the order of every accepted edit,
//! and the check that refuses an edit made on out-of-date state.
//!
//! [`Host`] holds no connection. It is given each message with the editor it
//! came from and answers the messages to send, as [`Out`], so it is tested
//! without a network; [`crate::session::Hosting`] carries them. The host's
//! own editor is user 0 and is in from the start: its edits arrive here
//! through the same [`Host::receive`] as anyone's.
//!
//! ## The check
//!
//! For every [`Group`] the host keeps the last accepted edit that touched it
//! and who made it. An edit is refused when another editor touched one of its
//! groups after the edit's `base`: the editor that made it had not seen that
//! change, so the state it recorded as "before" is not the state the others
//! now hold. Edits by the same editor never conflict with each other, since
//! that editor applied each one before making the next.
//!
//! A refusal raises the editor's required epoch, and every edit it sent at the
//! old epoch is refused as well, accepted or not on its own merits. Those
//! edits were made on top of the refused one. The editor takes them back
//! itself; see [`crate::peer`].

use crate::auth;
use crate::groups::{Group, Payload};
use crate::ids::Allocator;
use crate::wire::{Seq, ToHost, ToPeer, UserId, PROTOCOL};
use std::collections::{BTreeMap, HashMap};

/// A message the host sends, or a connection it closes.
#[derive(Debug, Clone, PartialEq)]
pub enum Out<P> {
    To(UserId, ToPeer<P>),
    /// Close this editor's connection once the messages before this are sent.
    Close(UserId),
    /// This editor answered the challenge. The host's editor saves, makes the
    /// manifest and calls [`Host::admit`]; until then the editor is sent no
    /// edits, so none can reach it before the files they are made on.
    Admit { user: UserId, name: String },
    /// This editor asks for files of the manifest. See [`ToHost::Fetch`].
    Fetch { user: UserId, paths: Vec<String> },
}

/// The longest name an editor is shown by, in characters.
const NAME_LENGTH: usize = 32;

#[derive(Debug)]
enum State {
    /// Connected, with no [`ToHost::Hello`] yet.
    Connected,
    /// Sent a challenge for this name.
    Challenged { name: String, nonce: [u8; 32] },
    /// Answered it, and waits for [`Host::admit`].
    Admitting { name: String },
    /// In, under `name`, and refusing any edit below `epoch`.
    In { name: String, epoch: u32 },
}

/// The host's state. See the module comment.
#[derive(Debug)]
pub struct Host {
    key: [u8; 32],
    build: String,
    archives: [u8; 32],
    users: BTreeMap<UserId, State>,
    seq: Seq,
    /// The last accepted edit that touched each group, and its editor.
    touched: HashMap<Group, (Seq, UserId)>,
    ids: Allocator,
}

impl Host {
    /// A session hosted by `name`, as user 0, behind `password`. A joining
    /// editor must be the same `build` and hold the same `archives`.
    pub fn new(name: &str, password: &str, build: &str, archives: [u8; 32]) -> Host {
        let mut users = BTreeMap::new();
        users.insert(0, State::In { name: shown_name(name, 0), epoch: 0 });
        Host {
            key: auth::key(password),
            build: build.to_string(),
            archives,
            users,
            seq: 0,
            touched: HashMap::new(),
            ids: Allocator::default(),
        }
    }

    /// The last accepted edit.
    pub fn seq(&self) -> Seq {
        self.seq
    }

    /// Everyone in, by number.
    pub fn users(&self) -> Vec<(UserId, String)> {
        self.users
            .iter()
            .filter_map(|(user, state)| match state {
                State::In { name, .. } => Some((*user, name.clone())),
                _ => None,
            })
            .collect()
    }

    /// A new connection, which has to say hello and answer a challenge.
    pub fn connect(&mut self, user: UserId) {
        self.users.insert(user, State::Connected);
    }

    /// Let in an editor that answered the challenge: it is welcomed at the
    /// current sequence and receives every edit after it. The caller sends
    /// the manifest straight after the welcome. Nothing for an editor that is
    /// not waiting to be let in.
    pub fn admit<P: Payload>(&mut self, user: UserId) -> Vec<Out<P>> {
        let Some(State::Admitting { name }) = self.users.get(&user) else {
            return Vec::new();
        };
        let name = name.clone();
        self.users.insert(user, State::In { name: name.clone(), epoch: 0 });
        let mut out = vec![Out::To(user, ToPeer::Welcome { user, seq: self.seq, users: self.users() })];
        out.extend(self.to_all(Some(user), ToPeer::Joined { user, name }));
        out
    }

    /// A connection gone. The others are told when the editor was in.
    pub fn disconnect<P: Payload>(&mut self, user: UserId) -> Vec<Out<P>> {
        match self.users.remove(&user) {
            Some(State::In { .. }) => self.to_all(Some(user), ToPeer::Left { user }),
            _ => Vec::new(),
        }
    }

    /// One message from `from`, and what it comes to.
    pub fn receive<P: Payload>(&mut self, from: UserId, message: ToHost<P>) -> Vec<Out<P>> {
        let Some(state) = self.users.get_mut(&from) else {
            return Vec::new();
        };
        match (state, message) {
            (State::Connected, ToHost::Hello { protocol, name, build, archives }) => {
                let refusal = if protocol != PROTOCOL {
                    Some(format!("the host speaks protocol {PROTOCOL}, this editor {protocol}"))
                } else if build != self.build {
                    Some(format!("the host is build {}, this editor {build}", self.build))
                } else if archives != self.archives {
                    Some("the game archives differ from the host's".to_string())
                } else {
                    None
                };
                if let Some(refusal) = refusal {
                    return self.refuse(from, refusal);
                }
                let nonce = auth::nonce();
                self.users.insert(from, State::Challenged { name: shown_name(&name, from), nonce });
                vec![Out::To(from, ToPeer::Challenge { nonce })]
            }
            (State::Challenged { name, nonce }, ToHost::Proof(answer)) => {
                if !auth::check(&self.key, nonce, name, &answer) {
                    return self.refuse(from, "wrong password".to_string());
                }
                let name = name.clone();
                self.users.insert(from, State::Admitting { name: name.clone() });
                vec![Out::Admit { user: from, name }]
            }
            (State::In { epoch: required, .. }, ToHost::Propose { id, epoch, base, payload }) => {
                if epoch < *required {
                    let reason = "made after an edit of yours that was refused".to_string();
                    return vec![Out::To(from, ToPeer::Rejected { id, reason })];
                }
                let groups = payload.groups();
                let conflict = groups.iter().find_map(|group| {
                    self.touched.get(group).filter(|(seq, by)| *seq > base && *by != from).map(|(_, by)| *by)
                });
                if let Some(by) = conflict {
                    *required = epoch + 1;
                    let reason = format!("{} changed the same thing first", self.name_of(by));
                    return vec![Out::To(from, ToPeer::Rejected { id, reason })];
                }
                self.seq += 1;
                let seq = self.seq;
                for group in groups {
                    self.touched.insert(group, (seq, from));
                }
                let mut out = vec![Out::To(from, ToPeer::Accepted { id, seq })];
                out.extend(self.to_all(Some(from), ToPeer::Change { seq, user: from, payload }));
                out
            }
            (State::In { .. }, ToHost::Ids { kind, floor, size }) => {
                let (start, end) = self.ids.grant(&kind, floor, size);
                vec![Out::To(from, ToPeer::Ids { kind, start, end })]
            }
            (State::In { .. }, ToHost::Presence(presence)) => self.to_all(Some(from), ToPeer::Presence { user: from, presence }),
            (State::In { .. }, ToHost::Fetch(paths)) => vec![Out::Fetch { user: from, paths }],
            // Where an editor is says nothing that matters before it is in,
            // and one sent while the welcome is on its way is not a fault.
            (_, ToHost::Presence(_)) => Vec::new(),
            (_, ToHost::Bye) => {
                let mut out = self.disconnect(from);
                out.push(Out::Close(from));
                out
            }
            _ => self.refuse(from, "a message out of order".to_string()),
        }
    }

    fn refuse<P>(&mut self, user: UserId, reason: String) -> Vec<Out<P>> {
        self.users.remove(&user);
        vec![Out::To(user, ToPeer::Refused(reason)), Out::Close(user)]
    }

    /// The message to everyone in, but `except`.
    fn to_all<P: Clone>(&self, except: Option<UserId>, message: ToPeer<P>) -> Vec<Out<P>> {
        self.users
            .iter()
            .filter(|(user, state)| Some(**user) != except && matches!(state, State::In { .. }))
            .map(|(user, _)| Out::To(*user, message.clone()))
            .collect()
    }

    fn name_of(&self, user: UserId) -> String {
        match self.users.get(&user) {
            Some(State::In { name, .. }) => name.clone(),
            _ => format!("Editor {user}"),
        }
    }
}

/// The name an editor is shown by: its own, cut to [`NAME_LENGTH`], or
/// `Editor N` when it gave none.
fn shown_name(name: &str, user: UserId) -> String {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(NAME_LENGTH).collect();
    match name.is_empty() {
        true => format!("Editor {user}"),
        false => name,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::groups::Part;
    use serde::{Deserialize, Serialize};
    use vale_edit::TileKey;

    /// A payload for tests: the groups it says it touches, and a name.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub(crate) struct Toy {
        pub(crate) name: String,
        pub(crate) groups: Vec<Group>,
    }

    impl Payload for Toy {
        fn groups(&self) -> Vec<Group> {
            self.groups.clone()
        }
    }

    pub(crate) fn chunk(chunk: u16) -> Group {
        Group::Chunk { tile: TileKey::new("Azeroth", 32, 48), chunk, part: Part::Shape }
    }

    pub(crate) fn toy(name: &str, groups: &[Group]) -> Toy {
        Toy { name: name.to_string(), groups: groups.to_vec() }
    }

    /// A host with user 1 joined under the password `pw`.
    fn with_one_joined() -> Host {
        let mut host = Host::new("Host", "pw", "b1", [1; 32]);
        host.connect(1);
        let out = host.receive::<Toy>(1, ToHost::Hello { protocol: PROTOCOL, name: "Alex".into(), build: "b1".into(), archives: [1; 32] });
        let Out::To(1, ToPeer::Challenge { nonce }) = out[0] else { panic!("{out:?}") };
        let out = host.receive::<Toy>(1, ToHost::Proof(auth::proof(&auth::key("pw"), &nonce, "Alex")));
        assert_eq!(out, vec![Out::Admit { user: 1, name: "Alex".into() }]);
        let out = host.admit::<Toy>(1);
        assert!(matches!(&out[0], Out::To(1, ToPeer::Welcome { user: 1, seq: 0, .. })), "{out:?}");
        assert_eq!(out[1], Out::To(0, ToPeer::Joined { user: 1, name: "Alex".into() }));
        host
    }

    /// A different build, other archives or a wrong password are refused and
    /// the connection closed.
    #[test]
    fn a_mismatch_or_a_wrong_password_is_refused() {
        let mut host = Host::new("Host", "pw", "b1", [1; 32]);
        host.connect(1);
        let out = host.receive::<Toy>(1, ToHost::Hello { protocol: PROTOCOL, name: "A".into(), build: "b2".into(), archives: [1; 32] });
        assert!(matches!(&out[..], [Out::To(1, ToPeer::Refused(_)), Out::Close(1)]), "{out:?}");

        host.connect(2);
        let out = host.receive::<Toy>(2, ToHost::Hello { protocol: PROTOCOL, name: "B".into(), build: "b1".into(), archives: [2; 32] });
        assert!(matches!(&out[..], [Out::To(2, ToPeer::Refused(_)), Out::Close(2)]), "{out:?}");

        host.connect(3);
        host.receive::<Toy>(3, ToHost::Hello { protocol: PROTOCOL, name: "C".into(), build: "b1".into(), archives: [1; 32] });
        let out = host.receive::<Toy>(3, ToHost::Proof(auth::proof(&auth::key("other"), &[0; 32], "C")));
        assert!(matches!(&out[..], [Out::To(3, ToPeer::Refused(_)), Out::Close(3)]), "{out:?}");
        assert_eq!(host.users(), vec![(0, "Host".to_string())]);
    }

    /// An edit on a chunk another editor changed after its base is refused,
    /// later edits at the same epoch are refused with it, and an edit at the
    /// next epoch made after seeing the change is accepted.
    #[test]
    fn an_edit_made_before_seeing_a_conflicting_one_is_refused() {
        let mut host = with_one_joined();
        let out = host.receive(0, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("host", &[chunk(3)]) });
        assert_eq!(out[0], Out::To(0, ToPeer::Accepted { id: 1, seq: 1 }));
        assert!(matches!(&out[1], Out::To(1, ToPeer::Change { seq: 1, user: 0, .. })));

        // Alex had not seen edit 1.
        let out = host.receive(1, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("a", &[chunk(3), chunk(4)]) });
        assert!(matches!(&out[..], [Out::To(1, ToPeer::Rejected { id: 1, .. })]), "{out:?}");
        // A second edit, disjoint, but at the refused epoch.
        let out = host.receive(1, ToHost::Propose { id: 2, epoch: 0, base: 0, payload: toy("b", &[chunk(9)]) });
        assert!(matches!(&out[..], [Out::To(1, ToPeer::Rejected { id: 2, .. })]), "{out:?}");
        // After taking both back and seeing edit 1.
        let out = host.receive(1, ToHost::Propose { id: 3, epoch: 1, base: 1, payload: toy("c", &[chunk(3)]) });
        assert_eq!(out[0], Out::To(1, ToPeer::Accepted { id: 3, seq: 2 }));
        // The same editor's edits never conflict with each other.
        let out = host.receive(1, ToHost::Propose { id: 4, epoch: 1, base: 1, payload: toy("d", &[chunk(3)]) });
        assert_eq!(out[0], Out::To(1, ToPeer::Accepted { id: 4, seq: 3 }));
        assert_eq!(host.seq(), 3);
    }

    /// An editor that has answered the challenge is sent no edit until it is
    /// admitted, and is welcomed at the sequence admitting found.
    #[test]
    fn an_editor_is_sent_nothing_until_it_is_admitted() {
        let mut host = Host::new("Host", "pw", "b1", [1; 32]);
        host.connect(1);
        let out = host.receive::<Toy>(1, ToHost::Hello { protocol: PROTOCOL, name: "Alex".into(), build: "b1".into(), archives: [1; 32] });
        let Out::To(1, ToPeer::Challenge { nonce }) = out[0] else { panic!("{out:?}") };
        host.receive::<Toy>(1, ToHost::Proof(auth::proof(&auth::key("pw"), &nonce, "Alex")));
        // A presence sent while the welcome is on its way is dropped, not a
        // reason to refuse the editor.
        let presence = crate::wire::Presence { map: "Azeroth".into(), camera: [0.0; 3], cursor: None, tool: "Select".into() };
        assert!(host.receive::<Toy>(1, ToHost::Presence(presence)).is_empty());
        let out = host.receive(0, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("early", &[chunk(1)]) });
        assert_eq!(out, vec![Out::To(0, ToPeer::Accepted { id: 1, seq: 1 })]);
        let out = host.admit::<Toy>(1);
        assert!(matches!(&out[0], Out::To(1, ToPeer::Welcome { user: 1, seq: 1, .. })), "{out:?}");
        assert!(host.admit::<Toy>(1).is_empty(), "admitting twice does nothing");
    }

    /// Leaving tells the others; a message before the handshake is refused.
    #[test]
    fn leaving_is_announced_and_an_early_message_refused() {
        let mut host = with_one_joined();
        let out = host.receive::<Toy>(1, ToHost::Bye);
        assert_eq!(out, vec![Out::To(0, ToPeer::Left { user: 1 }), Out::Close(1)]);
        host.connect(2);
        let out = host.receive(2, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("x", &[chunk(1)]) });
        assert!(matches!(&out[..], [Out::To(2, ToPeer::Refused(_)), Out::Close(2)]), "{out:?}");
    }
}
