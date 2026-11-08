//! The messages of a session and how one is framed on a stream.
//!
//! A frame is a four-byte little-endian length and then the message in
//! postcard's encoding. A length over [`MAX_FRAME`] closes the connection
//! rather than allocating it, since the length is read from the network.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// The version of these messages. A joining editor sends it in
/// [`ToHost::Hello`] and the host refuses any other, since postcard encodes
/// fields by position and two builds that differ read each other wrongly.
pub const PROTOCOL: u32 = 2;

/// The largest frame either end reads, in bytes: 64 MiB.
pub const MAX_FRAME: usize = 64 << 20;

/// Who an editor is in a session. The host is 0; a joining editor is given
/// the next number when it connects, and numbers are not reused.
pub type UserId = u32;

/// The number of an accepted edit, from 1. 0 is the state the session
/// started from.
pub type Seq = u64;

/// Where an editor is and what it is doing, sent a few times a second and
/// drawn by the others.
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
    /// the host's: the editor's own build, and a fingerprint of the game's
    /// archives, since an edit is applied to the bytes those archives hold.
    Hello { protocol: u32, name: String, build: String, archives: [u8; 32] },
    /// The answer to [`ToPeer::Challenge`]. See [`crate::auth::proof`].
    Proof([u8; 32]),
    /// An edit this editor has made and applied. `id` is the editor's own
    /// number for it, `base` the last edit it had received, and `epoch` the
    /// count of times it has taken waiting edits back.
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
    /// The editor is leaving.
    Bye,
}

/// What the host sends an editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToPeer<P> {
    /// The value [`ToHost::Proof`] is made from.
    Challenge { nonce: [u8; 32] },
    /// The editor is in: its number, the last edit accepted so far, and who
    /// else is in. [`ToPeer::Manifest`] follows at once.
    Welcome { user: UserId, seq: Seq, users: Vec<(UserId, String)> },
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
    Joined { user: UserId, name: String },
    Left { user: UserId },
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
    let mut length = [0u8; 4];
    from.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err(std::io::Error::other(format!("a frame of {length} bytes is over the limit")));
    }
    let mut body = vec![0u8; length];
    from.read_exact(&mut body)?;
    postcard::from_bytes(&body).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
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
