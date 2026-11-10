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
//! and its author. An edit is refused when another author touched one of its
//! groups after the edit's `base`: the editor that made it had not seen that
//! change, so the state it recorded as "before" is not the state the other
//! editors now hold. Edits by the same author never conflict with each other,
//! since that author's editor applied each one before making the next.
//!
//! The host's editor resumes the numbering and the last touches from its log
//! ([`Host::resume`]), so an edit's `base` can be a number from an earlier
//! session.
//!
//! A refusal raises the editor's required epoch, and every edit it sent at the
//! old epoch is refused as well, accepted or not on its own merits. Those
//! edits were made on top of the refused one. The editor takes them back
//! itself; see [`crate::peer`].
//!
//! Two exceptions to the check:
//!
//! * An edit from the host's own editor that is [`Payload::forced`] is
//!   accepted whatever its epoch and whatever touched its groups after its
//!   base. It cannot be reverted, so it is put in the order where it
//!   arrives.
//! * An edit from any other editor must also pass [`Payload::check`], which
//!   says what a joined editor may send. A failed check is a refusal like a
//!   conflict.
//!
//! ## Claims
//!
//! An accepted edit claims every tile, table and row it touches for its
//! author until that author submits ([`Host::release_author`]); a check out
//! claims them before any edit. An edit that is not forced is refused when
//! another author holds a claim on anything it touches, and the reason names
//! the holder. See [`crate::checkout`].
//!
//! The host's editor saves only submitted edits, which it does by taking the
//! other authors' unsubmitted edits back on a copy of what it writes; that is
//! exact only while nobody else has changed what those edits touched, which
//! the claims ensure. The edited claims are read back from the log's edits in
//! no changelist when a session starts ([`Host::resume`]). Every change to
//! the claims is sent to every editor ([`ToPeer::Holds`]), so a joined editor
//! knows what is held before it edits.
//!
//! A [`ToHost::Ask`] is not an edit. It is passed to the host's editor as
//! [`Out::Ask`], unnumbered, for the person hosting to answer.
//!
//! ## The log
//!
//! Every accepted edit is also answered as [`Out::Log`], before the edit is
//! sent anywhere, for the host's editor to write to its log. A
//! [`ToHost::Request`] about the log is passed to the host's editor as
//! [`Out::Request`], since the host's editor holds the log.

use crate::auth;
use crate::checkout::{Claim, Hold};
use crate::groups::{Group, Payload};
use crate::ids::Allocator;
use crate::log::{Author, AuthorId};
use crate::wire::{Database, Request, Seq, ToHost, ToPeer, UserId, PROTOCOL};
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
    /// This editor asks the host's editor to make an edit. See
    /// [`ToHost::Ask`].
    Ask { user: UserId, payload: P },
    /// An edit was accepted as `seq`, for the host's editor to log. Comes
    /// before the edit is sent to any editor.
    Log { seq: Seq, user: UserId, author: Author, payload: P },
    /// This editor asks about the log. See [`ToHost::Request`].
    Request { user: UserId, author: Author, request: Request },
}

/// The longest name an editor is shown by, in characters.
const NAME_LENGTH: usize = 32;

#[derive(Debug)]
enum State {
    /// Connected, with no [`ToHost::Hello`] yet.
    Connected,
    /// Sent a challenge for this name.
    Challenged { name: String, author: AuthorId, nonce: [u8; 32] },
    /// Answered it, and waits for [`Host::admit`].
    Admitting { name: String, author: AuthorId },
    /// In, under `name`, and refusing any edit below `epoch`.
    In { name: String, author: AuthorId, epoch: u32 },
}

/// The host's state. See the module comment.
#[derive(Debug)]
pub struct Host {
    key: [u8; 32],
    build: String,
    archives: [u8; 32],
    users: BTreeMap<UserId, State>,
    seq: Seq,
    /// The last accepted edit that touched each group, and its author.
    touched: HashMap<Group, (Seq, AuthorId)>,
    /// The name each author was last seen under, for a refusal's reason.
    names: HashMap<AuthorId, String>,
    /// The id of the log the numbers are counted in, sent in the welcome.
    log: [u8; 16],
    /// Who holds each claim, and how. See *Claims*.
    holds: BTreeMap<Claim, Held>,
    ids: Allocator,
}

impl Host {
    /// A session hosted by `author`, as user 0, behind `password`. A joining
    /// editor must be the same `build` and hold the same `archives`.
    pub fn new(author: &Author, password: &str, build: &str, archives: [u8; 32]) -> Host {
        let name = shown_name(&author.name, 0);
        let mut users = BTreeMap::new();
        users.insert(0, State::In { name: name.clone(), author: author.id, epoch: 0 });
        Host {
            key: auth::key(password),
            build: build.to_string(),
            archives,
            users,
            seq: 0,
            touched: HashMap::new(),
            names: HashMap::from([(author.id, name)]),
            log: [0; 16],
            holds: BTreeMap::new(),
            ids: Allocator::default(),
        }
    }

    /// Hold the check outs the host's editor kept from an earlier session,
    /// beside the edited claims [`Self::resume`] read. Called before anyone
    /// joins.
    pub fn resume_checkouts(&mut self, list: Vec<Hold>) {
        for hold in list {
            let held = self.holds.entry(hold.claim).or_insert_with(|| Held::by(&hold.author));
            if held.author.id == hold.author.id {
                held.checked_out = true;
            }
        }
    }

    /// Every claim held, in the order of their claims.
    pub fn holds(&self) -> Vec<Hold> {
        self.holds.iter().map(|(claim, held)| held.hold(claim)).collect()
    }

    /// Release every claim an author holds, checked out or edited, as a
    /// Submit of theirs does.
    pub fn release_author<P: Payload>(&mut self, author: &AuthorId) -> Vec<Out<P>> {
        self.changed_holds(|holds| holds.retain(|_, held| held.author.id != *author))
    }

    /// Release what an author's unsubmitted edits hold, and keep their check
    /// outs: for edits that were withdrawn because they change nothing, or
    /// taken back by another editor.
    pub fn release_pending<P: Payload>(&mut self, author: &AuthorId) -> Vec<Out<P>> {
        self.changed_holds(|holds| {
            for held in holds.values_mut().filter(|held| held.author.id == *author) {
                held.edited = false;
            }
            holds.retain(|_, held| held.checked_out || held.edited);
        })
    }

    /// Change the claims, and send every editor the list when it changed.
    fn changed_holds<P: Payload>(&mut self, change: impl FnOnce(&mut BTreeMap<Claim, Held>)) -> Vec<Out<P>> {
        let before = self.holds.clone();
        change(&mut self.holds);
        match self.holds == before {
            true => Vec::new(),
            false => self.to_all(None, ToPeer::Holds(self.holds())),
        }
    }

    /// Continue from a log: its id, its last edit, the last touch of each
    /// group, the groups its unsubmitted edits touch with their authors, and
    /// the names of its authors. Called before anyone joins.
    pub fn resume(
        &mut self,
        log: [u8; 16],
        seq: Seq,
        touched: HashMap<Group, (Seq, AuthorId)>,
        pending: HashMap<Group, AuthorId>,
        names: &HashMap<AuthorId, String>,
    ) {
        self.log = log;
        self.seq = seq;
        self.touched = touched;
        for (author, name) in names {
            self.names.entry(*author).or_insert_with(|| name.clone());
        }
        for (group, author) in pending {
            let Some(claim) = Claim::of(&group) else { continue };
            let name = self.names.get(&author).cloned().unwrap_or_else(|| "another editor".to_string());
            let held = self.holds.entry(claim).or_insert_with(|| Held::by(&Author { id: author, name }));
            if held.author.id == author {
                held.edited = true;
            }
        }
    }

    /// The author of an editor that is in.
    pub fn author_of(&self, user: UserId) -> Option<Author> {
        match self.users.get(&user) {
            Some(State::In { name, author, .. }) => Some(Author { id: *author, name: name.clone() }),
            _ => None,
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
    /// not waiting to be let in. `database` is the host's world database;
    /// see [`ToPeer::Welcome`].
    pub fn admit<P: Payload>(&mut self, user: UserId, database: Option<Database>) -> Vec<Out<P>> {
        let Some(State::Admitting { name, author }) = self.users.get(&user) else {
            return Vec::new();
        };
        let (name, author) = (name.clone(), *author);
        self.names.insert(author, name.clone());
        self.users.insert(user, State::In { name: name.clone(), author, epoch: 0 });
        let welcome = ToPeer::Welcome { user, seq: self.seq, log: self.log, users: self.users(), database };
        let mut out = vec![Out::To(user, welcome), Out::To(user, ToPeer::Holds(self.holds()))];
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
            (State::Connected, ToHost::Hello { protocol, name, author, build, archives }) => {
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
                self.users.insert(from, State::Challenged { name: shown_name(&name, from), author, nonce });
                vec![Out::To(from, ToPeer::Challenge { nonce })]
            }
            (State::Challenged { name, author, nonce }, ToHost::Proof(answer)) => {
                if !auth::check(&self.key, nonce, name, &answer) {
                    return self.refuse(from, "wrong password".to_string());
                }
                let (name, author) = (name.clone(), *author);
                self.users.insert(from, State::Admitting { name: name.clone(), author });
                vec![Out::Admit { user: from, name }]
            }
            (State::In { epoch: required, author, name }, ToHost::Propose { id, epoch, base, payload }) => {
                let author = Author { id: *author, name: name.clone() };
                let forced = from == 0 && payload.forced();
                if epoch < *required && !forced {
                    let reason = "made after a refused edit from this editor".to_string();
                    return vec![Out::To(from, ToPeer::Rejected { id, reason })];
                }
                if from != 0 {
                    if let Err(reason) = payload.check() {
                        *required = epoch + 1;
                        return vec![Out::To(from, ToPeer::Rejected { id, reason })];
                    }
                }
                let groups = payload.groups();
                let mut claims: Vec<Claim> = groups.iter().filter_map(Claim::of).collect();
                claims.sort();
                claims.dedup();
                let held = claims
                    .iter()
                    .filter(|_| !forced)
                    .find_map(|claim| self.holds.get(claim).filter(|held| held.author.id != author.id).map(|held| held.hold(claim)));
                if let Some(hold) = held {
                    *required = epoch + 1;
                    let reason = format!("{} {}", hold.author.name, hold.words());
                    return vec![Out::To(from, ToPeer::Rejected { id, reason })];
                }
                let conflict = groups.iter().filter(|_| !forced).find_map(|group| {
                    self.touched.get(group).filter(|(seq, by)| *seq > base && *by != author.id).map(|(_, by)| *by)
                });
                if let Some(by) = conflict {
                    *required = epoch + 1;
                    let name = self.names.get(&by).cloned().unwrap_or_else(|| "another editor".to_string());
                    let reason = format!("{name} made a conflicting edit first");
                    return vec![Out::To(from, ToPeer::Rejected { id, reason })];
                }
                self.seq += 1;
                let seq = self.seq;
                for group in groups {
                    self.touched.insert(group, (seq, author.id));
                }
                let mut out = vec![
                    Out::Log { seq, user: from, author: author.clone(), payload: payload.clone() },
                    Out::To(from, ToPeer::Accepted { id, seq }),
                ];
                out.extend(self.to_all(Some(from), ToPeer::Change { seq, user: from, payload }));
                // A forced edit over another author's claim leaves the claim
                // theirs.
                out.extend(self.changed_holds(|holds| {
                    for claim in claims {
                        let held = holds.entry(claim).or_insert_with(|| Held::by(&author));
                        if held.author.id == author.id {
                            held.edited = true;
                        }
                    }
                }));
                out
            }
            (State::In { .. }, ToHost::Ids { kind, floor, size }) => {
                let (start, end) = self.ids.grant(&kind, floor, size);
                vec![Out::To(from, ToPeer::Ids { kind, start, end })]
            }
            (State::In { .. }, ToHost::Presence(presence)) => self.to_all(Some(from), ToPeer::Presence { user: from, presence }),
            (State::In { .. }, ToHost::Fetch(paths)) => vec![Out::Fetch { user: from, paths }],
            (State::In { .. }, ToHost::Ask(payload)) => vec![Out::Ask { user: from, payload }],
            (State::In { name, author, .. }, ToHost::Request(request)) => {
                let author = Author { id: *author, name: name.clone() };
                vec![Out::Request { user: from, author, request }]
            }
            (State::In { name, author, .. }, ToHost::CheckOut(claims)) => {
                let author = Author { id: *author, name: name.clone() };
                self.changed_holds(|holds| {
                    for claim in claims {
                        let held = holds.entry(claim).or_insert_with(|| Held::by(&author));
                        if held.author.id == author.id {
                            held.checked_out = true;
                        }
                    }
                })
            }
            // A release ends a check out only; see *Claims*.
            (State::In { author, .. }, ToHost::Release { claims, others }) => {
                let author = *author;
                self.changed_holds(|holds| {
                    for claim in claims {
                        if let Some(held) = holds.get_mut(&claim).filter(|held| others || held.author.id == author) {
                            held.checked_out = false;
                        }
                    }
                    holds.retain(|_, held| held.checked_out || held.edited);
                })
            }
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
}

/// One claim's holder, and how it is held. See *Claims*.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    author: Author,
    checked_out: bool,
    edited: bool,
}

impl Held {
    /// Held by `author`, in neither way yet.
    fn by(author: &Author) -> Held {
        Held { author: author.clone(), checked_out: false, edited: false }
    }

    fn hold(&self, claim: &Claim) -> Hold {
        Hold { claim: claim.clone(), author: self.author.clone(), checked_out: self.checked_out, edited: self.edited }
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

    /// A payload for tests: the groups it says it touches, a name, whether
    /// it is forced, and whether a joined editor may send it.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub(crate) struct Toy {
        pub(crate) name: String,
        pub(crate) groups: Vec<Group>,
        pub(crate) forced: bool,
        pub(crate) refused: bool,
    }

    impl Payload for Toy {
        fn groups(&self) -> Vec<Group> {
            self.groups.clone()
        }

        fn forced(&self) -> bool {
            self.forced
        }

        fn check(&self) -> Result<(), String> {
            match self.refused {
                true => Err("only the host makes this edit".to_string()),
                false => Ok(()),
            }
        }
    }

    pub(crate) fn chunk(chunk: u16) -> Group {
        Group::Chunk { tile: TileKey::new("Azeroth", 32, 48), chunk, part: Part::Shape }
    }

    pub(crate) fn toy(name: &str, groups: &[Group]) -> Toy {
        Toy { name: name.to_string(), groups: groups.to_vec(), forced: false, refused: false }
    }

    pub(crate) fn forced(name: &str, groups: &[Group]) -> Toy {
        Toy { forced: true, ..toy(name, groups) }
    }

    /// The host's own author in tests.
    pub(crate) fn host_author() -> Author {
        Author { id: [0xA0; 16], name: "Host".to_string() }
    }

    /// What the host sends for a message, without the log records, which
    /// [`an_accepted_edit_is_logged_under_its_author`] checks.
    fn sent(host: &mut Host, from: UserId, message: ToHost<Toy>) -> Vec<Out<Toy>> {
        host.receive(from, message).into_iter().filter(|out| !matches!(out, Out::Log { .. })).collect()
    }

    /// A host with user 1 joined under the password `pw`.
    fn with_one_joined() -> Host {
        let mut host = Host::new(&host_author(), "pw", "b1", [1; 32]);
        host.connect(1);
        let out = host.receive::<Toy>(1, ToHost::Hello { protocol: PROTOCOL, name: "Alex".into(), author: [1; 16], build: "b1".into(), archives: [1; 32] });
        let Out::To(1, ToPeer::Challenge { nonce }) = out[0] else { panic!("{out:?}") };
        let out = host.receive::<Toy>(1, ToHost::Proof(auth::proof(&auth::key("pw"), &nonce, "Alex")));
        assert_eq!(out, vec![Out::Admit { user: 1, name: "Alex".into() }]);
        let out = host.admit::<Toy>(1, None);
        assert!(matches!(&out[0], Out::To(1, ToPeer::Welcome { user: 1, seq: 0, database: None, .. })), "{out:?}");
        assert_eq!(out[1], Out::To(1, ToPeer::Holds(Vec::new())));
        assert_eq!(out[2], Out::To(0, ToPeer::Joined { user: 1, name: "Alex".into() }));
        host
    }

    /// A different build, other archives or a wrong password are refused and
    /// the connection closed.
    #[test]
    fn a_mismatch_or_a_wrong_password_is_refused() {
        let mut host = Host::new(&host_author(), "pw", "b1", [1; 32]);
        host.connect(1);
        let out = host.receive::<Toy>(1, ToHost::Hello { protocol: PROTOCOL, name: "A".into(), author: [2; 16], build: "b2".into(), archives: [1; 32] });
        assert!(matches!(&out[..], [Out::To(1, ToPeer::Refused(_)), Out::Close(1)]), "{out:?}");

        host.connect(2);
        let out = host.receive::<Toy>(2, ToHost::Hello { protocol: PROTOCOL, name: "B".into(), author: [3; 16], build: "b1".into(), archives: [2; 32] });
        assert!(matches!(&out[..], [Out::To(2, ToPeer::Refused(_)), Out::Close(2)]), "{out:?}");

        host.connect(3);
        host.receive::<Toy>(3, ToHost::Hello { protocol: PROTOCOL, name: "C".into(), author: [4; 16], build: "b1".into(), archives: [1; 32] });
        let out = host.receive::<Toy>(3, ToHost::Proof(auth::proof(&auth::key("other"), &[0; 32], "C")));
        assert!(matches!(&out[..], [Out::To(3, ToPeer::Refused(_)), Out::Close(3)]), "{out:?}");
        assert_eq!(host.users(), vec![(0, "Host".to_string())]);
    }

    /// An edit on a chunk another editor changed after its base is refused,
    /// later edits at the same epoch are refused with it, and an edit at the
    /// next epoch made after seeing the change is refused while the other
    /// editor's edit is unsubmitted and accepted after.
    #[test]
    fn an_edit_made_before_seeing_a_conflicting_one_is_refused() {
        let mut host = with_one_joined();
        let out = sent(&mut host, 0, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("host", &[chunk(3)]) });
        assert_eq!(out[0], Out::To(0, ToPeer::Accepted { id: 1, seq: 1 }));
        assert!(matches!(&out[1], Out::To(1, ToPeer::Change { seq: 1, user: 0, .. })));

        // Alex had not seen edit 1.
        let out = sent(&mut host, 1, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("a", &[chunk(3), chunk(4)]) });
        assert!(matches!(&out[..], [Out::To(1, ToPeer::Rejected { id: 1, .. })]), "{out:?}");
        // A second edit, disjoint, but at the refused epoch.
        let out = sent(&mut host, 1, ToHost::Propose { id: 2, epoch: 0, base: 0, payload: toy("b", &[chunk(9)]) });
        assert!(matches!(&out[..], [Out::To(1, ToPeer::Rejected { id: 2, .. })]), "{out:?}");
        // After seeing edit 1, while the host has not submitted it.
        let out = sent(&mut host, 1, ToHost::Propose { id: 3, epoch: 1, base: 1, payload: toy("c", &[chunk(3)]) });
        assert!(
            matches!(&out[..], [Out::To(1, ToPeer::Rejected { reason, .. })] if reason == "Host has unsubmitted edits on Azeroth 32_48"),
            "{out:?}"
        );
        // After the host submits.
        host.release_author::<Toy>(&host_author().id);
        let out = sent(&mut host, 1, ToHost::Propose { id: 4, epoch: 2, base: 1, payload: toy("c", &[chunk(3)]) });
        assert_eq!(out[0], Out::To(1, ToPeer::Accepted { id: 4, seq: 2 }));
        // The same editor's edits never conflict with each other.
        let out = sent(&mut host, 1, ToHost::Propose { id: 5, epoch: 2, base: 1, payload: toy("d", &[chunk(3)]) });
        assert_eq!(out[0], Out::To(1, ToPeer::Accepted { id: 5, seq: 3 }));
        // A resumed host holds what the log's unsubmitted edits touched.
        let mut resumed = Host::new(&host_author(), "pw", "b1", [1; 32]);
        resumed.resume([5; 16], 3, HashMap::new(), HashMap::from([(chunk(3), [1; 16])]), &HashMap::from([([1; 16], "Alex".to_string())]));
        let out = sent(&mut resumed, 0, ToHost::Propose { id: 1, epoch: 0, base: 3, payload: toy("host", &[chunk(3)]) });
        assert!(matches!(&out[..], [Out::To(0, ToPeer::Rejected { reason, .. })] if reason.starts_with("Alex has unsubmitted")), "{out:?}");
        assert_eq!(host.seq(), 3);
    }

    /// The host's forced edit is accepted over a conflict and at a refused
    /// epoch, and is a conflict for the edits after it; a joined editor's
    /// forced edit is checked as any other.
    #[test]
    fn a_forced_edit_from_the_host_is_accepted_over_a_conflict() {
        let mut host = with_one_joined();
        let out = sent(&mut host, 1, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("a", &[chunk(3)]) });
        assert_eq!(out[0], Out::To(1, ToPeer::Accepted { id: 1, seq: 1 }));
        // The host had not seen edit 1, and its epoch is behind.
        sent(&mut host, 0, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("host", &[chunk(3)]) });
        let out = sent(&mut host, 0, ToHost::Propose { id: 2, epoch: 0, base: 0, payload: forced("delete", &[chunk(3)]) });
        assert_eq!(out[0], Out::To(0, ToPeer::Accepted { id: 2, seq: 2 }));
        let out = sent(&mut host, 1, ToHost::Propose { id: 2, epoch: 0, base: 1, payload: toy("b", &[chunk(3)]) });
        assert!(matches!(&out[..], [Out::To(1, ToPeer::Rejected { id: 2, .. })]), "{out:?}");
        let out = sent(&mut host, 1, ToHost::Propose { id: 3, epoch: 1, base: 1, payload: forced("c", &[chunk(3)]) });
        assert!(matches!(&out[..], [Out::To(1, ToPeer::Rejected { id: 3, .. })]), "{out:?}");
    }

    /// A joined editor's edit that fails the check is refused and raises its
    /// epoch; the host's own edit is not checked; an ask goes to the host's
    /// editor and to nobody else.
    #[test]
    fn a_checked_edit_is_refused_and_an_ask_reaches_the_host() {
        let mut host = with_one_joined();
        let refused = Toy { refused: true, ..toy("place", &[chunk(1)]) };
        let out = sent(&mut host, 1, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: refused.clone() });
        assert!(matches!(&out[..], [Out::To(1, ToPeer::Rejected { id: 1, .. })]), "{out:?}");
        let out = sent(&mut host, 1, ToHost::Propose { id: 2, epoch: 0, base: 0, payload: toy("raise", &[chunk(2)]) });
        assert!(matches!(&out[..], [Out::To(1, ToPeer::Rejected { id: 2, .. })]), "the epoch rose: {out:?}");
        let out = sent(&mut host, 0, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: refused });
        assert_eq!(out[0], Out::To(0, ToPeer::Accepted { id: 1, seq: 1 }));
        let out = host.receive(1, ToHost::Ask(toy("delete", &[])));
        assert_eq!(out, vec![Out::Ask { user: 1, payload: toy("delete", &[]) }]);
    }

    /// An editor that has answered the challenge is sent no edit until it is
    /// admitted, and is welcomed at the sequence admitting found.
    #[test]
    fn an_editor_is_sent_nothing_until_it_is_admitted() {
        let mut host = Host::new(&host_author(), "pw", "b1", [1; 32]);
        host.connect(1);
        let out = host.receive::<Toy>(1, ToHost::Hello { protocol: PROTOCOL, name: "Alex".into(), author: [1; 16], build: "b1".into(), archives: [1; 32] });
        let Out::To(1, ToPeer::Challenge { nonce }) = out[0] else { panic!("{out:?}") };
        host.receive::<Toy>(1, ToHost::Proof(auth::proof(&auth::key("pw"), &nonce, "Alex")));
        // A presence sent while the welcome is on its way is dropped, not a
        // reason to refuse the editor.
        let presence = crate::wire::Presence { map: "Azeroth".into(), camera: [0.0; 3], cursor: None, tool: "Select".into() };
        assert!(host.receive::<Toy>(1, ToHost::Presence(presence)).is_empty());
        let out = sent(&mut host, 0, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("early", &[chunk(1)]) });
        assert_eq!(out[0], Out::To(0, ToPeer::Accepted { id: 1, seq: 1 }));
        assert!(matches!(&out[1..], [Out::To(0, ToPeer::Holds(_))]), "only the host is told of its claim: {out:?}");
        let out = host.admit::<Toy>(1, Some(Database { fingerprint: [7; 32], shared: None }));
        assert!(matches!(&out[0], Out::To(1, ToPeer::Welcome { user: 1, seq: 1, database: Some(Database { fingerprint: [7, ..], .. }), .. })), "{out:?}");
        assert!(host.admit::<Toy>(1, None).is_empty(), "admitting twice does nothing");
    }

    /// Leaving tells the other editors; a message before the handshake is refused.
    #[test]
    fn leaving_is_announced_and_an_early_message_refused() {
        let mut host = with_one_joined();
        let out = host.receive::<Toy>(1, ToHost::Bye);
        assert_eq!(out, vec![Out::To(0, ToPeer::Left { user: 1 }), Out::Close(1)]);
        host.connect(2);
        let out = sent(&mut host, 2, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("x", &[chunk(1)]) });
        assert!(matches!(&out[..], [Out::To(2, ToPeer::Refused(_)), Out::Close(2)]), "{out:?}");
    }

    /// An accepted edit is logged under its author before it is sent. A host
    /// resumed from a log continues its numbering, and an edit based before a
    /// resumed touch by another author is refused, while one by the same
    /// author on a new connection is not.
    #[test]
    fn an_accepted_edit_is_logged_under_its_author() {
        let mut host = with_one_joined();
        let out = host.receive(1, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("a", &[chunk(3)]) });
        let alex = Author { id: [1; 16], name: "Alex".into() };
        assert_eq!(out[0], Out::Log { seq: 1, user: 1, author: alex.clone(), payload: toy("a", &[chunk(3)]) });
        assert_eq!(out[1], Out::To(1, ToPeer::Accepted { id: 1, seq: 1 }));

        let mut host = Host::new(&host_author(), "pw", "b1", [1; 32]);
        let names = HashMap::from([([1; 16], "Alex".to_string())]);
        host.resume([5; 16], 40, HashMap::from([(chunk(3), (40, [1; 16])), (chunk(4), (12, [0xA0; 16]))]), HashMap::new(), &names);
        let out = sent(&mut host, 0, ToHost::Propose { id: 1, epoch: 0, base: 30, payload: toy("host", &[chunk(3)]) });
        assert!(matches!(&out[..], [Out::To(0, ToPeer::Rejected { reason, .. })] if reason.starts_with("Alex")), "{out:?}");
        let out = sent(&mut host, 0, ToHost::Propose { id: 2, epoch: 1, base: 30, payload: toy("host", &[chunk(4)]) });
        assert_eq!(out[0], Out::To(0, ToPeer::Accepted { id: 2, seq: 41 }));
    }

    /// A tile Alex checks out refuses the host's edit to it and not Alex's;
    /// the host's forced edit goes through; a release by someone else does
    /// nothing unless it breaks the check out, and every change is sent to
    /// everyone.
    #[test]
    fn a_checked_out_tile_refuses_another_authors_edit() {
        let mut host = with_one_joined();
        let tile = Claim::Tile(TileKey::new("Azeroth", 32, 48));
        let out = host.receive::<Toy>(1, ToHost::CheckOut(vec![tile.clone()]));
        assert_eq!(out.len(), 2, "both editors are told: {out:?}");
        let out = sent(&mut host, 0, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("host", &[chunk(3)]) });
        assert!(matches!(&out[..], [Out::To(0, ToPeer::Rejected { reason, .. })] if reason == "Alex has Azeroth 32_48 checked out"), "{out:?}");
        let out = sent(&mut host, 1, ToHost::Propose { id: 1, epoch: 0, base: 0, payload: toy("a", &[chunk(3)]) });
        assert_eq!(out[0], Out::To(1, ToPeer::Accepted { id: 1, seq: 1 }));
        let out = sent(&mut host, 0, ToHost::Propose { id: 2, epoch: 1, base: 1, payload: forced("delete", &[chunk(3)]) });
        assert_eq!(out[0], Out::To(0, ToPeer::Accepted { id: 2, seq: 2 }));

        assert!(host.receive::<Toy>(0, ToHost::Release { claims: vec![tile.clone()], others: false }).is_empty());
        assert_eq!(host.holds().len(), 1);
        let out = host.receive::<Toy>(0, ToHost::Release { claims: vec![tile], others: true });
        // Alex's edit still holds the tile, now as an edited claim only.
        let held = |hold: &Hold| !hold.checked_out && hold.edited && hold.author.name == "Alex";
        assert!(matches!(&out[0], Out::To(_, ToPeer::Holds(list)) if list.len() == 1 && held(&list[0])), "{out:?}");
        host.release_author::<Toy>(&[1; 16]);
        assert!(host.holds().is_empty());
    }
}
