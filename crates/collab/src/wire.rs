//! The messages of a session and how one is framed on a stream.
//!
//! A frame is a four-byte little-endian length and then the message in
//! postcard's encoding. A length over [`MAX_FRAME`] closes the connection
//! rather than allocating it, since the length is read from the network.

use crate::checkout::{Claim, Hold};
use crate::groups::Group;
use crate::log::{AuthorId, Changelist};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// The version of these messages. A joining editor sends it in
/// [`ToHost::Hello`] and the host refuses any other, since postcard encodes
/// fields by position and two builds that differ read each other wrongly.
pub const PROTOCOL: u32 = 7;

/// The largest frame either end reads, in bytes: 64 MiB.
pub const MAX_FRAME: usize = 64 << 20;

/// Who an editor is in a session. The host is 0; a joining editor is given
/// the next number when it connects, and numbers are not reused.
pub type UserId = u32;

/// The number of an accepted edit, from 1. 0 is the state before the
/// project's first edit. A host resumes from its log's last number, so a
/// number names the same edit across sessions. See [`crate::log`].
pub type Seq = u64;

/// Where an editor is and what it is doing, sent a few times a second and
/// drawn by the other editors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    /// The map the editor has open, by its directory name.
    pub map: String,
    /// The camera's position, in world coordinates.
    pub camera: [f32; 3],
    /// The point under the pointer, when the pointer is over the world.
    pub cursor: Option<[f32; 3]>,
    /// The tool in use, by its name on the rail.
    pub tool: String,
}

/// The host's world database, as the welcome describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Database {
    /// A hash of the database's location, which a joined editor compares
    /// with its own to tell whether the two are one database.
    pub fingerprint: [u8; 32],
    /// The location and login, as vmangos writes them
    /// (`host;port;user;password;database`), when the host shares its
    /// database with joined editors. The session is not encrypted, so the
    /// login crosses the network as it is.
    pub shared: Option<String>,
}

/// One file of the host's project, as [`ToPeer::Manifest`] lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// The virtual path, as the project folder keeps it (`World\Maps\…`).
    pub path: String,
    pub size: u64,
    /// The blake3 hash of the bytes.
    pub hash: [u8; 32],
}

/// How much of a file one [`ToPeer::File`] carries: 4 MiB.
pub const FILE_PIECE: usize = 4 << 20;

/// What an editor sends the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToHost<P> {
    /// The first message on a connection. `build` and `archives` must match
    /// the host's build and archives: the editor's own build, and a
    /// fingerprint of the game's archives, since an edit is applied to the
    /// bytes those archives hold. `author` is the person's id, which their
    /// edits are logged under; see [`crate::log::Author`].
    Hello { protocol: u32, name: String, author: AuthorId, build: String, archives: [u8; 32] },
    /// The answer to [`ToPeer::Challenge`]. See [`crate::auth::proof`].
    Proof([u8; 32]),
    /// An edit this editor has made and applied. `id` is the editor's own
    /// number for it, `base` the last edit it had received, and `epoch` the
    /// count of times it has reverted its waiting edits.
    Propose { id: u64, epoch: u32, base: Seq, payload: P },
    /// A block of `size` new numbers of `kind`, none below `floor`. See
    /// [`crate::ids`].
    Ids { kind: String, floor: u64, size: u64 },
    Presence(Presence),
    /// An edit this editor may not make, for the host's editor to make
    /// itself or decline: a joined editor's request to delete files. It is
    /// not numbered and reaches no other editor. A decline is answered with
    /// [`ToPeer::Note`].
    Ask(P),
    /// The files of the [`ToPeer::Manifest`] this editor lacks, or holds
    /// different bytes of. Answered with their [`ToPeer::File`] pieces and
    /// then [`ToPeer::FilesDone`], also when the list is empty.
    Fetch(Vec<String>),
    /// A question about the host's log, answered with [`ToPeer::Answer`].
    Request(Request),
    /// Check these out for this editor's author. A claim another author
    /// holds is left with them. See [`crate::checkout`].
    CheckOut(Vec<Claim>),
    /// Release these check outs: this author's, or with `others` anyone's,
    /// which breaks them. A claim held by unsubmitted edits stays held.
    Release { claims: Vec<Claim>, others: bool },
    /// The editor is leaving.
    Bye,
}

/// What the host sends an editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToPeer<P> {
    /// The value [`ToHost::Proof`] is made from.
    Challenge { nonce: [u8; 32] },
    /// The editor is in: its number, the last edit accepted so far, and who
    /// else is in. [`ToPeer::Manifest`] follows at once. `log` is the id of
    /// the host's log, which `seq` is counted in; see [`crate::log`].
    ///
    /// `database` is the host's world database, or `None` when the host has
    /// none. See [`Database`].
    Welcome { user: UserId, seq: Seq, log: [u8; 16], users: Vec<(UserId, String)>, database: Option<Database> },
    /// The host's project as it stood at the welcome's `seq`, saved: its name
    /// and every file. Every [`ToPeer::Change`] after it is made on top of
    /// these files.
    Manifest { project: String, files: Vec<FileEntry> },
    /// A piece of a fetched file: `bytes` at `offset` of a file `total` bytes
    /// long. A file's pieces come in order.
    File { path: String, offset: u64, total: u64, bytes: Vec<u8> },
    /// Every fetched file has been sent, or could not be: `missing` names the
    /// ones the host could not read.
    FilesDone { missing: Vec<String> },
    /// The editor is not let in, or is put out, and why. The connection is
    /// closed after it.
    Refused(String),
    /// One of this editor's edits, by its own `id`, is accepted as `seq`.
    Accepted { id: u64, seq: Seq },
    /// One of this editor's edits is refused.
    Rejected { id: u64, reason: String },
    /// Another editor's edit, accepted as `seq`.
    Change { seq: Seq, user: UserId, payload: P },
    /// The numbers `start..end` of `kind` are this editor's to give out.
    Ids { kind: String, start: u64, end: u64 },
    Presence { user: UserId, presence: Presence },
    /// A line from the host's editor for this editor's status line: what it
    /// did with an [`ToHost::Ask`].
    Note(String),
    /// The answer to a [`ToHost::Request`].
    Answer(Answer<P>),
    /// Every claim held, checked out or edited, sent after the welcome and
    /// whenever one changes. See [`crate::checkout`].
    Holds(Vec<Hold>),
    Joined { user: UserId, name: String },
    Left { user: UserId },
}

/// A question a joined editor asks about the host's log. The host's editor
/// holds the log and answers it. See [`crate::log`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    /// Put the asking author's open edits in a changelist with this
    /// description. Answered with [`Answer::Submitted`].
    Submit { description: String },
    /// Every changelist. Answered with [`Answer::History`].
    History,
    /// Which of `groups` an edit after `after` touched, leaving out the edits
    /// in `except`. `log` is the id of the log `after` was counted in; when it
    /// is not the host's, every group is stale. Answered with
    /// [`Answer::Stale`].
    Stale { groups: Vec<Group>, after: Seq, log: [u8; 16], except: Vec<Seq> },
    /// These edits' payloads, for a revert. Answered with [`Answer::Edits`].
    Edits(Vec<Seq>),
    /// Edits in no changelist, which the host's files do not hold: the
    /// asking author's, for a Discard, or with `everyone` every author's, for
    /// an editor that has just joined. Answered with [`Answer::Open`].
    Open { everyone: bool },
    /// Withdraw the asking author's edits in no changelist when they change
    /// nothing, as after an undo or a Discard. Answered with
    /// [`Answer::Withdrawn`] when they were withdrawn, and not at all when
    /// they change something.
    Withdraw,
    /// What the log appended past these lengths of its two files, for a
    /// joined editor's copy of the log. See [`crate::log::Log::tail`].
    Tail { edits_from: u64, changelists_from: u64 },
}

/// The host's answer to a [`Request`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Answer<P> {
    /// The changelist the open edits were put in, or `None` when there were
    /// none.
    Submitted(Option<Changelist>),
    /// The log's id and every changelist, oldest first.
    History { log: [u8; 16], changelists: Vec<Changelist> },
    /// The stale groups, and the host's last edit when it answered. Every
    /// edit up to `seq` reaches the editor before the answer.
    Stale { groups: Vec<Group>, seq: Seq },
    /// The edits asked for, by number, with their payloads. An edit the log
    /// cannot read is left out.
    Edits(Vec<(Seq, P)>),
    /// The edits in no changelist asked for, oldest first, each with its
    /// author and payload, and whether every author's were asked for.
    Open { everyone: bool, edits: Vec<(Seq, AuthorId, P)> },
    /// This editor's edits in no changelist change nothing, and the host's
    /// editor withdrew them from the log (`crate::log::Log::withdraw`). The
    /// editor has none open now.
    Withdrawn,
    /// See [`Request::Tail`].
    Tail(crate::log::Tail),
}

/// A value in the encoding a frame carries, without the frame's length.
pub fn encode<T: Serialize>(value: &T) -> std::io::Result<Vec<u8>> {
    postcard::to_stdvec(value).map_err(|e| std::io::Error::other(e.to_string()))
}

/// A value read back from [`encode`]'s bytes.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> std::io::Result<T> {
    postcard::from_bytes(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

/// Write one message as a frame.
pub fn write_frame<T: Serialize>(to: &mut impl Write, message: &T) -> std::io::Result<()> {
    let body = postcard::to_stdvec(message).map_err(|e| std::io::Error::other(e.to_string()))?;
    if body.len() > MAX_FRAME {
        return Err(std::io::Error::other(format!("a {} byte message is over the frame limit", body.len())));
    }
    to.write_all(&(body.len() as u32).to_le_bytes())?;
    to.write_all(&body)?;
    to.flush()
}

/// Read one frame and decode its message. An end of stream before the length
/// is [`std::io::ErrorKind::UnexpectedEof`].
pub fn read_frame<T: DeserializeOwned>(from: &mut impl Read) -> std::io::Result<T> {
    let body = read_body(from)?;
    postcard::from_bytes(&body).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

/// Read one frame's body without decoding it.
pub fn read_body(from: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut length = [0u8; 4];
    from.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err(std::io::Error::other(format!("a frame of {length} bytes is over the limit")));
    }
    let mut body = vec![0u8; length];
    from.read_exact(&mut body)?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A message written as a frame reads back as itself, and a length over
    /// the limit is refused before anything is allocated.
    #[test]
    fn a_frame_reads_back_and_an_oversized_one_is_refused() {
        let sent: ToPeer<String> = ToPeer::Change { seq: 9, user: 2, payload: "raise".to_string() };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &sent).unwrap();
        let read: ToPeer<String> = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(read, sent);

        let mut huge = ((MAX_FRAME + 1) as u32).to_le_bytes().to_vec();
        huge.extend([0; 8]);
        assert!(read_frame::<ToPeer<String>>(&mut huge.as_slice()).is_err());
    }
}
