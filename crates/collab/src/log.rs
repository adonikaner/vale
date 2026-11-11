//! A project's log: every accepted edit in order, with who made it and when,
//! and the changelists that label runs of them.
//!
//! The log is a folder beside the project's files, `Edit\<project>\log\`:
//!
//! ```text
//! id.txt           the log's id: 16 random bytes in hex, made with the log
//! edits.bin        one frame per edit: its number, author, time, groups,
//!                  payload (deflated) and whether it continues the author's
//!                  edit before it
//! changelists.txt  one line per changelist: number, time, author id, author
//!                  name, the edits it holds, description
//! ```
//!
//! The log does not know what an edit is. The caller passes the groups an
//! edit touches and the payload already encoded, and reads the payload back
//! as bytes. So the same log holds the editor's steps whatever their type.
//!
//! ## Numbering
//!
//! Edits are numbered from 1 with no gaps, as [`crate::wire::Seq`] numbers
//! them in a session. A host resumes from the log's last number, so a number
//! names the same edit across sessions, and an editor that went offline at
//! edit 412 can ask which groups changed after 412.
//!
//! ## A damaged tail
//!
//! `edits.bin` is only appended to. A write cut short by a crash leaves a
//! partial last frame. Opening the log reads frames until one does not
//! decode, or is not numbered one past the frame before it, and cuts the file
//! there, so the next append follows the last whole edit.
//!
//! ## Edits that continue the one before
//!
//! A stroke in a session is sent in parts a few times a second, so other
//! editors see it while it is drawn, and each part is accepted and numbered
//! as an edit of its own. Each part after the first is stored as continuing
//! the author's edit before it. A whole edit is a first part and the parts
//! that continue it: [`Log::open_count`] and [`Log::whole_edits`] count whole
//! edits, and an interface lists them as one.
//!
//! ## Withdrawn edits
//!
//! An author's open edits that change nothing between them, such as edits
//! all undone again, are withdrawn ([`Log::withdraw`]): they are in no
//! changelist and are no longer open, so nothing counts or lists them. They
//! are written to `changelists.txt` as a line numbered 0, which
//! [`Log::changelists`] leaves out.
//!
//! ## A copy of the log
//!
//! Both of the log's growing files are only appended to, so a joined
//! editor's copy of the project keeps a copy of the host's log by asking for
//! what was appended since the lengths it holds ([`Log::tail`]) and appending
//! it ([`replicate`]). A copy whose host is gone can then host the project
//! with its history. A tail cut part way through a frame is completed by the
//! next one; a log opened from a copy that is cut there drops the partial
//! frame, as it drops a damaged tail.
//!
//! ## Compaction
//!
//! A stroke is stored in parts, each with whole chunk states before and
//! after, so a long stroke stores its chunks many times over. [`Log::compact`]
//! folds each stroke of a closed changelist into its first part and leaves
//! the later parts empty: the numbers, the groups and the counts stay as
//! they were, and applying or reverting the parts in order does what it did.
//! `compacted.txt` records the generation, which each compaction raises, and
//! the last edit compacted. A copy of the log ([`Tail::generation`]) of
//! another generation is started again, since its bytes are no longer the
//! log's.
//!
//! ## Changelists
//!
//! An author's edits that are in no changelist are that author's open edits.
//! [`Log::close`] puts them in a new changelist with a description. A
//! changelist names its edits by number; in a session one author's edits are
//! interleaved with others', so the numbers are not one run.

use crate::groups::Group;
use crate::wire::{decode, read_body, write_frame, Seq};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// A person, by a random id kept by their editor and not by their name, so
/// two people with one name are two authors.
pub type AuthorId = [u8; 16];

/// An author: the id and the name they were shown by when they made an edit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Author {
    pub id: AuthorId,
    pub name: String,
}

/// The log's folder name under a project.
pub const LOG_DIR: &str = vale_edit::project::LOG_DIR;

const ID_FILE: &str = "id.txt";
const EDITS_FILE: &str = "edits.bin";
const CHANGELISTS_FILE: &str = "changelists.txt";
/// The generation and the last edit compacted. See *Compaction*.
const COMPACTED_FILE: &str = "compacted.txt";

/// The changelist number that marks withdrawn edits. See *Withdrawn edits*.
const WITHDRAWN: u32 = 0;

/// One edit as it is stored in `edits.bin`.
#[derive(Serialize, Deserialize)]
struct Stored {
    seq: Seq,
    author: Author,
    time: u64,
    groups: Vec<Group>,
    /// The caller's payload, deflated.
    payload: Vec<u8>,
    /// See *Edits that continue the one before*.
    continues: bool,
}

/// [`Stored`] as it was written before edits carried `continues`. Read so a
/// log written then still opens; its edits each count as whole.
#[derive(Deserialize)]
struct StoredBefore {
    seq: Seq,
    author: Author,
    time: u64,
    groups: Vec<Group>,
    payload: Vec<u8>,
}

/// One frame of `edits.bin`, in either form.
fn read_stored(from: &mut impl Read) -> std::io::Result<Stored> {
    let body = read_body(from)?;
    match decode::<Stored>(&body) {
        Ok(stored) => Ok(stored),
        Err(e) => match decode::<StoredBefore>(&body) {
            Ok(StoredBefore { seq, author, time, groups, payload }) => {
                Ok(Stored { seq, author, time, groups, payload, continues: false })
            }
            Err(_) => Err(e),
        },
    }
}

/// What the log keeps in memory about one edit. The payload stays on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub seq: Seq,
    pub author: AuthorId,
    /// Seconds since 1970, UTC.
    pub time: u64,
    pub groups: Vec<Group>,
    /// Whether it continues the author's edit before it. See *Edits that
    /// continue the one before*.
    pub continues: bool,
    /// The changelist that holds this edit, if one does.
    pub changelist: Option<u32>,
    /// Where the edit's frame starts in `edits.bin`.
    offset: u64,
}

/// A run of one author's edits with a description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Changelist {
    /// From 1, in the order changelists were closed.
    pub number: u32,
    /// When it was closed, in seconds since 1970, UTC.
    pub time: u64,
    pub author: Author,
    pub description: String,
    /// The edits it holds, by number, in order.
    pub edits: Vec<Seq>,
    /// How many whole edits those are; see *Edits that continue the one
    /// before*. Counted by the log, not kept in `changelists.txt`.
    pub whole_edits: usize,
}

/// A project's log. See the module comment.
#[derive(Debug)]
pub struct Log {
    dir: PathBuf,
    id: [u8; 16],
    headers: Vec<Header>,
    changelists: Vec<Changelist>,
    /// The last name each author was seen under.
    names: HashMap<AuthorId, String>,
    /// How many whole edits of each author are in no changelist.
    open: HashMap<AuthorId, usize>,
    /// Raised by each compaction. See *Compaction*.
    generation: u32,
    /// The last edit a compaction looked at.
    compacted: Seq,
}

impl Log {
    /// The log in `dir`, made when it is not there. A damaged tail of
    /// `edits.bin` is cut off; see *A damaged tail*.
    pub fn open(dir: impl AsRef<Path>) -> std::io::Result<Log> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let id = match std::fs::read_to_string(dir.join(ID_FILE)).ok().and_then(|text| parse_hex(text.trim())) {
            Some(id) => id,
            None => {
                let id: [u8; 16] = rand::random();
                std::fs::write(dir.join(ID_FILE), format!("{}\n", hex(&id)))?;
                id
            }
        };
        let (generation, compacted) = read_compacted(&dir);
        let mut log = Log {
            dir,
            id,
            headers: Vec::new(),
            changelists: Vec::new(),
            names: HashMap::new(),
            open: HashMap::new(),
            generation,
            compacted,
        };
        log.read_edits()?;
        log.read_changelists();
        Ok(log)
    }

    /// Read every whole edit of `edits.bin` and cut what follows the last.
    fn read_edits(&mut self) -> std::io::Result<()> {
        let path = self.dir.join(EDITS_FILE);
        let Ok(bytes) = std::fs::read(&path) else { return Ok(()) };
        let mut at = 0usize;
        while at < bytes.len() {
            let mut rest = &bytes[at..];
            let Ok(stored) = read_stored(&mut rest) else { break };
            if stored.seq != self.headers.len() as Seq + 1 {
                break;
            }
            self.names.insert(stored.author.id, stored.author.name.clone());
            *self.open.entry(stored.author.id).or_default() += usize::from(!stored.continues);
            self.headers.push(Header {
                seq: stored.seq,
                author: stored.author.id,
                time: stored.time,
                groups: stored.groups,
                continues: stored.continues,
                changelist: None,
                offset: at as u64,
            });
            at = bytes.len() - rest.len();
        }
        if at < bytes.len() {
            std::fs::OpenOptions::new().write(true).open(&path)?.set_len(at as u64)?;
        }
        Ok(())
    }

    /// Read `changelists.txt`. A line that does not read, or that names an
    /// edit the log does not have, is skipped.
    fn read_changelists(&mut self) {
        let Ok(text) = std::fs::read_to_string(self.dir.join(CHANGELISTS_FILE)) else { return };
        for line in text.lines() {
            let Some(mut changelist) = parse_changelist(line) else { continue };
            if changelist.edits.iter().any(|seq| *seq == 0 || *seq > self.last()) {
                continue;
            }
            changelist.whole_edits = self.whole_edits(&changelist.edits);
            for seq in &changelist.edits {
                self.mark(*seq, changelist.number);
            }
            self.names.entry(changelist.author.id).or_insert_with(|| changelist.author.name.clone());
            if changelist.number != WITHDRAWN {
                self.changelists.push(changelist);
            }
        }
    }

    /// The folder the log is kept in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The log's id. Two logs with the same id are the same history.
    pub fn id(&self) -> [u8; 16] {
        self.id
    }

    /// The last edit's number, or 0 for an empty log.
    pub fn last(&self) -> Seq {
        self.headers.len() as Seq
    }

    /// Every edit, oldest first.
    pub fn headers(&self) -> &[Header] {
        &self.headers
    }

    /// One edit, by number.
    pub fn header(&self, seq: Seq) -> Option<&Header> {
        self.headers.get((seq as usize).checked_sub(1)?)
    }

    /// The name an author was last seen under.
    pub fn name_of(&self, author: &AuthorId) -> Option<&str> {
        self.names.get(author).map(String::as_str)
    }

    /// Every author and the name each was last seen under.
    pub fn names(&self) -> &HashMap<AuthorId, String> {
        &self.names
    }

    /// Store edit `seq`, which must be one past [`Self::last`]. `continues`
    /// says it is a later part of the author's edit before it.
    pub fn append(&mut self, seq: Seq, author: &Author, time: u64, groups: Vec<Group>, continues: bool, payload: &[u8]) -> std::io::Result<()> {
        if seq != self.last() + 1 {
            return Err(std::io::Error::other(format!("edit {seq} does not follow edit {}", self.last())));
        }
        let mut deflated = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        deflated.write_all(payload)?;
        let stored = Stored { seq, author: author.clone(), time, groups, payload: deflated.finish()?, continues };
        let path = self.dir.join(EDITS_FILE);
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        let offset = file.seek(SeekFrom::End(0))?;
        let mut frame = Vec::new();
        write_frame(&mut frame, &stored)?;
        file.write_all(&frame)?;
        self.names.insert(author.id, author.name.clone());
        *self.open.entry(author.id).or_default() += usize::from(!continues);
        self.headers.push(Header { seq, author: author.id, time, groups: stored.groups, continues, changelist: None, offset });
        Ok(())
    }

    /// Put edit `seq` in changelist `number`, and count it out of its
    /// author's open edits.
    fn mark(&mut self, seq: Seq, number: u32) {
        let header = &mut self.headers[seq as usize - 1];
        if header.changelist.is_none() && !header.continues {
            if let Some(open) = self.open.get_mut(&header.author) {
                *open = open.saturating_sub(1);
            }
        }
        header.changelist = Some(number);
    }

    /// How many of an author's whole edits are in no changelist.
    pub fn open_count(&self, author: &AuthorId) -> usize {
        self.open.get(author).copied().unwrap_or(0)
    }

    /// Every author with edits in no changelist, and how many whole edits,
    /// by name.
    pub fn open_counts(&self) -> Vec<(AuthorId, String, usize)> {
        let mut out: Vec<(AuthorId, String, usize)> = self
            .open
            .iter()
            .filter(|(_, count)| **count > 0)
            .map(|(author, count)| (*author, self.names.get(author).cloned().unwrap_or_default(), *count))
            .collect();
        out.sort_by(|a, b| a.1.cmp(&b.1));
        out
    }

    /// The payload of edit `seq`, as it was given to [`Self::append`].
    pub fn read(&self, seq: Seq) -> std::io::Result<Vec<u8>> {
        let header = self.header(seq).ok_or_else(|| std::io::Error::other(format!("the log has no edit {seq}")))?;
        let mut file = std::fs::File::open(self.dir.join(EDITS_FILE))?;
        file.seek(SeekFrom::Start(header.offset))?;
        let stored = read_stored(&mut std::io::BufReader::new(file))?;
        let mut payload = Vec::new();
        flate2::read::DeflateDecoder::new(stored.payload.as_slice()).read_to_end(&mut payload)?;
        Ok(payload)
    }

    /// The last edit that touched each group, and its author.
    pub fn touches(&self) -> HashMap<Group, (Seq, AuthorId)> {
        let mut out = HashMap::new();
        for header in &self.headers {
            for group in &header.groups {
                out.insert(group.clone(), (header.seq, header.author));
            }
        }
        out
    }

    /// Which of `groups` an edit after `after` touched, leaving out the edits
    /// in `except`. Sorted, each once.
    pub fn stale(&self, groups: &[Group], after: Seq, except: &[Seq]) -> Vec<Group> {
        let except: HashSet<Seq> = except.iter().copied().collect();
        let wanted: HashSet<&Group> = groups.iter().collect();
        let mut found: Vec<Group> = self
            .headers
            .iter()
            .skip(after as usize)
            .filter(|header| !except.contains(&header.seq))
            .flat_map(|header| header.groups.iter())
            .filter(|group| wanted.contains(group))
            .cloned()
            .collect();
        found.sort();
        found.dedup();
        found
    }

    /// How many whole edits `seqs` are: the edits among them that do not
    /// continue the one before.
    pub fn whole_edits(&self, seqs: &[Seq]) -> usize {
        seqs.iter().filter(|seq| self.header(**seq).is_some_and(|header| !header.continues)).count()
    }

    /// Withdraw an author's open edits, which the caller has found change
    /// nothing between them. Answers how many there were. See *Withdrawn
    /// edits*.
    pub fn withdraw(&mut self, author: &Author, time: u64) -> std::io::Result<usize> {
        let edits = self.open_edits(&author.id);
        if edits.is_empty() {
            return Ok(0);
        }
        let line = Changelist { number: WITHDRAWN, time, author: author.clone(), description: String::new(), edits: edits.clone(), whole_edits: 0 };
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join(CHANGELISTS_FILE))?;
        file.write_all(format!("{}\n", changelist_line(&line)).as_bytes())?;
        for seq in &edits {
            self.mark(*seq, WITHDRAWN);
        }
        Ok(edits.len())
    }

    /// Every changelist, oldest first.
    pub fn changelists(&self) -> &[Changelist] {
        &self.changelists
    }

    /// One changelist, by number.
    pub fn changelist(&self, number: u32) -> Option<&Changelist> {
        self.changelists.iter().find(|changelist| changelist.number == number)
    }

    /// Every group an edit in no changelist touches, and that edit's author:
    /// what each author holds until they submit. See `crate::host`'s
    /// *Unsubmitted edits*.
    pub fn pending(&self) -> HashMap<Group, AuthorId> {
        let mut out = HashMap::new();
        for header in self.headers.iter().filter(|header| header.changelist.is_none()) {
            for group in &header.groups {
                out.insert(group.clone(), header.author);
            }
        }
        out
    }

    /// An author's edits that are in no changelist, oldest first.
    pub fn open_edits(&self, author: &AuthorId) -> Vec<Seq> {
        self.headers
            .iter()
            .filter(|header| header.author == *author && header.changelist.is_none())
            .map(|header| header.seq)
            .collect()
    }

    /// Put an author's open edits in a new changelist with `description`, and
    /// answer it; `None` when the author has none.
    pub fn close(&mut self, author: &Author, time: u64, description: &str) -> std::io::Result<Option<Changelist>> {
        let edits = self.open_edits(&author.id);
        if edits.is_empty() {
            return Ok(None);
        }
        let number = self.changelists.last().map_or(1, |last| last.number + 1);
        let whole_edits = self.whole_edits(&edits);
        let changelist = Changelist { number, time, author: author.clone(), description: description.trim().to_string(), edits, whole_edits };
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join(CHANGELISTS_FILE))?;
        file.write_all(format!("{}\n", changelist_line(&changelist)).as_bytes())?;
        for seq in &changelist.edits {
            self.mark(*seq, number);
        }
        self.changelists.push(changelist.clone());
        Ok(Some(changelist))
    }

    /// What the log's two files hold past `edits_from` and
    /// `changelists_from` bytes, at most `max` bytes of each, and their
    /// lengths now. See *A copy of the log*.
    pub fn tail(&self, edits_from: u64, changelists_from: u64, max: usize) -> std::io::Result<Tail> {
        let part = |file: &str, from: u64| -> std::io::Result<(Vec<u8>, u64)> {
            let bytes = match std::fs::read(self.dir.join(file)) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(e),
            };
            let start = (from as usize).min(bytes.len());
            let end = start.saturating_add(max).min(bytes.len());
            Ok((bytes[start..end].to_vec(), bytes.len() as u64))
        };
        let (edits, edits_len) = part(EDITS_FILE, edits_from)?;
        let (changelists, changelists_len) = part(CHANGELISTS_FILE, changelists_from)?;
        Ok(Tail {
            log: self.id,
            generation: self.generation,
            compacted: self.compacted,
            edits_from,
            edits,
            edits_len,
            changelists_from,
            changelists,
            changelists_len,
            numbers: String::new(),
        })
    }

    /// Fold each stroke of a closed changelist into its first part. `fold`
    /// is given the payloads of one stroke's parts, oldest first, and answers
    /// one payload for each, or `None` to leave the stroke as it is. Only
    /// strokes with an edit after the last compaction's mark are given; the
    /// mark stops before the first edit in no changelist. Answers how many
    /// strokes were folded. See *Compaction*.
    pub fn compact(&mut self, mut fold: impl FnMut(&[Vec<u8>]) -> Option<Vec<Vec<u8>>>) -> std::io::Result<usize> {
        // Each stroke: an author's edit that does not continue, and that
        // author's edits after it that do.
        let mut strokes: Vec<Vec<usize>> = Vec::new();
        let mut last_of: HashMap<AuthorId, usize> = HashMap::new();
        for (at, header) in self.headers.iter().enumerate() {
            match (header.continues, last_of.get(&header.author)) {
                (true, Some(&stroke)) => strokes[stroke].push(at),
                _ => {
                    last_of.insert(header.author, strokes.len());
                    strokes.push(vec![at]);
                }
            }
        }
        let closed = |at: &usize| matches!(self.headers[*at].changelist, Some(number) if number != WITHDRAWN);
        let mut replaced: HashMap<usize, Vec<u8>> = HashMap::new();
        let mut folded = 0;
        for stroke in strokes.iter().filter(|stroke| stroke.len() > 1 && stroke.iter().all(closed)) {
            if stroke.iter().all(|at| self.headers[*at].seq <= self.compacted) {
                continue;
            }
            let payloads = stroke.iter().map(|at| self.read(self.headers[*at].seq)).collect::<std::io::Result<Vec<_>>>()?;
            let Some(made) = fold(&payloads).filter(|made| made.len() == stroke.len()) else { continue };
            for (at, payload) in stroke.iter().zip(made) {
                replaced.insert(*at, payload);
            }
            folded += 1;
        }
        // Up to the first edit in no changelist: a stroke open now is looked
        // at again by the next compaction, once it is closed.
        let through = self.headers.iter().find(|header| header.changelist.is_none()).map_or(self.last(), |header| header.seq - 1);
        if replaced.is_empty() {
            self.compacted = through;
            write_compacted(&self.dir, self.generation, through)?;
            return Ok(0);
        }
        // Every frame written again into a new file, which then replaces the
        // old one, so a crash leaves one or the other whole.
        let old = std::fs::read(self.dir.join(EDITS_FILE))?;
        let mut rest = old.as_slice();
        let mut out = Vec::with_capacity(old.len());
        for at in 0..self.headers.len() {
            let mut stored = read_stored(&mut rest)?;
            if let Some(payload) = replaced.remove(&at) {
                let mut deflated = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
                deflated.write_all(&payload)?;
                stored.payload = deflated.finish()?;
            }
            write_frame(&mut out, &stored)?;
        }
        let fresh = self.dir.join("edits.new");
        std::fs::write(&fresh, &out)?;
        std::fs::rename(&fresh, self.dir.join(EDITS_FILE))?;
        self.generation += 1;
        self.compacted = through;
        write_compacted(&self.dir, self.generation, through)?;
        // The frames moved: read their offsets again.
        let changelists = std::mem::take(&mut self.changelists);
        let marks: Vec<Option<u32>> = self.headers.iter().map(|header| header.changelist).collect();
        let open = std::mem::take(&mut self.open);
        self.headers.clear();
        self.read_edits()?;
        for (header, mark) in self.headers.iter_mut().zip(marks) {
            header.changelist = mark;
        }
        self.changelists = changelists;
        self.open = open;
        Ok(folded)
    }
}

/// What a log appended past two lengths. See [`Log::tail`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tail {
    /// The id of the log it is from.
    pub log: [u8; 16],
    /// The log's generation and last edit compacted. See *Compaction*.
    pub generation: u32,
    pub compacted: Seq,
    /// Where `edits` starts in `edits.bin`.
    pub edits_from: u64,
    pub edits: Vec<u8>,
    /// How long `edits.bin` is.
    pub edits_len: u64,
    pub changelists_from: u64,
    pub changelists: Vec<u8>,
    pub changelists_len: u64,
    /// The host's allocator of new numbers as its file holds it
    /// (`crate::ids::Allocator::text`), so a copy that hosts goes on
    /// numbering where the host stopped. Filled by the host's editor.
    pub numbers: String,
}

/// The id and the lengths of the two growing files of the copy of a log in
/// `dir`, or zeros and `None` where it has none. See *A copy of the log*.
pub fn copy_lengths(dir: &Path) -> (Option<[u8; 16]>, u64, u64) {
    let id = std::fs::read_to_string(dir.join(ID_FILE)).ok().and_then(|text| parse_hex(text.trim()));
    let len = |file: &str| std::fs::metadata(dir.join(file)).map_or(0, |meta| meta.len());
    (id, len(EDITS_FILE), len(CHANGELISTS_FILE))
}

/// Append a tail to the copy of a log in `dir`. A copy of another log, or
/// one longer than the log it copies (a log that cut a damaged tail), is
/// started again from nothing; the next tail asked for fills it. A tail that
/// does not start at the copy's length is not appended. Answers whether the
/// copy holds all of the log now.
pub fn replicate(dir: &Path, tail: &Tail) -> std::io::Result<bool> {
    std::fs::create_dir_all(dir)?;
    let (id, edits_len, changelists_len) = copy_lengths(dir);
    let (generation, _) = read_compacted(dir);
    let other = id != Some(tail.log) || generation != tail.generation;
    if other || edits_len > tail.edits_len || changelists_len > tail.changelists_len {
        std::fs::write(dir.join(ID_FILE), format!("{}\n", hex(&tail.log)))?;
        std::fs::write(dir.join(EDITS_FILE), b"")?;
        std::fs::write(dir.join(CHANGELISTS_FILE), b"")?;
        write_compacted(dir, tail.generation, tail.compacted)?;
        return Ok(false);
    }
    let append = |file: &str, at: u64, from: u64, bytes: &[u8]| -> std::io::Result<u64> {
        if at != from || bytes.is_empty() {
            return Ok(at);
        }
        let mut out = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(file))?;
        out.write_all(bytes)?;
        Ok(at + bytes.len() as u64)
    };
    let edits = append(EDITS_FILE, edits_len, tail.edits_from, &tail.edits)?;
    let changelists = append(CHANGELISTS_FILE, changelists_len, tail.changelists_from, &tail.changelists)?;
    Ok(edits == tail.edits_len && changelists == tail.changelists_len)
}

/// The generation and last edit compacted of the log in `dir`; zeros when it
/// has never been compacted.
fn read_compacted(dir: &Path) -> (u32, Seq) {
    let text = std::fs::read_to_string(dir.join(COMPACTED_FILE)).unwrap_or_default();
    let mut out = (0, 0);
    for line in text.lines() {
        match line.split_once(' ') {
            Some(("generation", value)) => out.0 = value.trim().parse().unwrap_or(0),
            Some(("through", value)) => out.1 = value.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    out
}

fn write_compacted(dir: &Path, generation: u32, through: Seq) -> std::io::Result<()> {
    std::fs::write(dir.join(COMPACTED_FILE), format!("generation {generation}\nthrough {through}\n"))
}

/// A new random author id.
pub fn new_author() -> AuthorId {
    rand::random()
}

/// The time now, in seconds since 1970, UTC.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// A time in seconds since 1970 as a UTC date and time to the minute:
/// `2026-11-09 14:02 UTC`.
pub fn date(time: u64) -> String {
    let days = (time / 86_400) as i64;
    let minutes = (time % 86_400) / 60;
    // Days since 1970-01-01 to a civil date, by Howard Hinnant's method.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year}-{month:02}-{day:02} {:02}:{:02} UTC", minutes / 60, minutes % 60)
}

/// Bytes as lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Sixteen bytes from 32 hex digits.
pub fn parse_hex(text: &str) -> Option<[u8; 16]> {
    if text.len() != 32 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 16];
    for (at, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// A changelist as one line of `changelists.txt`: number, time, author id,
/// author name, edits and description, separated by tabs. The edits are
/// runs, `3-7,9`. A tab, line break or backslash in the name or description
/// is written as `\t`, `\n` or `\\`.
fn changelist_line(changelist: &Changelist) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        changelist.number,
        changelist.time,
        hex(&changelist.author.id),
        escape(&changelist.author.name),
        runs(&changelist.edits),
        escape(&changelist.description)
    )
}

fn parse_changelist(line: &str) -> Option<Changelist> {
    let mut fields = line.splitn(6, '\t');
    let number = fields.next()?.parse().ok()?;
    let time = fields.next()?.parse().ok()?;
    let id = parse_hex(fields.next()?)?;
    let name = unescape(fields.next()?);
    let edits = parse_runs(fields.next()?)?;
    let description = unescape(fields.next()?);
    Some(Changelist { number, time, author: Author { id, name }, description, edits, whole_edits: 0 })
}

/// Sorted numbers as runs: `[3, 4, 5, 9]` is `3-5,9`.
fn runs(edits: &[Seq]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut at = 0;
    while at < edits.len() {
        let start = edits[at];
        let mut end = start;
        while at + 1 < edits.len() && edits[at + 1] == end + 1 {
            at += 1;
            end = edits[at];
        }
        out.push(match start == end {
            true => start.to_string(),
            false => format!("{start}-{end}"),
        });
        at += 1;
    }
    out.join(",")
}

fn parse_runs(text: &str) -> Option<Vec<Seq>> {
    let mut out = Vec::new();
    for run in text.split(',').filter(|run| !run.is_empty()) {
        match run.split_once('-') {
            Some((start, end)) => {
                let (start, end): (Seq, Seq) = (start.parse().ok()?, end.parse().ok()?);
                if end < start || end - start > 1 << 24 {
                    return None;
                }
                out.extend(start..=end);
            }
            None => out.push(run.parse().ok()?),
        }
    }
    Some(out)
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\t', "\\t").replace('\r', "").replace('\n', "\\n")
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (c, c == '\\') {
            (_, true) => match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            },
            (c, false) => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::tests::chunk;

    fn author(id: u8, name: &str) -> Author {
        Author { id: [id; 16], name: name.to_string() }
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vale-collab-log-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Edits appended to a log read back after the log is opened again: the
    /// same numbers, authors, groups and payloads, the same id, and the last
    /// touch of each group. A number that does not follow is refused.
    #[test]
    fn edits_read_back_after_the_log_is_opened_again() {
        let dir = folder("reopen");
        let (sam, alex) = (author(1, "Sam"), author(2, "Alex"));
        let mut log = Log::open(&dir).unwrap();
        log.append(1, &sam, 100, vec![chunk(3)], false, b"raise").unwrap();
        log.append(2, &alex, 101, vec![chunk(3), chunk(4)], false, &vec![7u8; 10_000]).unwrap();
        assert!(log.append(4, &sam, 102, vec![chunk(1)], false, b"x").is_err());
        let id = log.id();

        let log = Log::open(&dir).unwrap();
        assert_eq!((log.id(), log.last()), (id, 2));
        assert_eq!(log.read(1).unwrap(), b"raise");
        assert_eq!(log.read(2).unwrap(), vec![7u8; 10_000]);
        assert_eq!(log.header(2).unwrap().groups, vec![chunk(3), chunk(4)]);
        assert_eq!(log.name_of(&[2; 16]), Some("Alex"));
        let touches = log.touches();
        assert_eq!(touches[&chunk(3)], (2, [2; 16]));
        assert_eq!(touches[&chunk(4)], (2, [2; 16]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A closed stroke is folded into its first part and its later parts
    /// emptied; an open one is left; the numbers, groups and changelists stay
    /// as they were, and the log opens again the same.
    #[test]
    fn compaction_folds_closed_strokes_only() {
        let dir = folder("compact");
        let (sam, alex) = (author(1, "Sam"), author(2, "Alex"));
        let mut log = Log::open(&dir).unwrap();
        log.append(1, &sam, 100, vec![chunk(3)], false, b"a").unwrap();
        log.append(2, &alex, 100, vec![chunk(9)], false, b"x").unwrap();
        log.append(3, &sam, 101, vec![chunk(3)], true, b"b").unwrap();
        log.append(4, &alex, 101, vec![chunk(9)], true, b"y").unwrap();
        log.close(&sam, 102, "Hill").unwrap();
        let folded = log
            .compact(|parts| {
                let joined: Vec<u8> = parts.concat();
                Some(std::iter::once(joined).chain(parts[1..].iter().map(|_| Vec::new())).collect())
            })
            .unwrap();
        assert_eq!(folded, 1);
        assert_eq!((log.read(1).unwrap(), log.read(3).unwrap()), (b"ab".to_vec(), Vec::new()));
        assert_eq!((log.read(2).unwrap(), log.read(4).unwrap()), (b"x".to_vec(), b"y".to_vec()));
        let again = Log::open(&dir).unwrap();
        assert_eq!((again.last(), again.changelists().len(), again.open_count(&alex.id)), (4, 1, 1));
        assert_eq!(again.header(3).unwrap().groups, vec![chunk(3)]);
        assert_eq!(again.read(1).unwrap(), b"ab");
        assert_eq!(read_compacted(&dir), (1, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A copy kept from tails, a few bytes at a time, opens as the same log.
    /// A copy of another log is started again.
    #[test]
    fn a_copy_kept_from_tails_is_the_same_log() {
        let (dir, copy) = (folder("original"), folder("copy"));
        let sam = author(1, "Sam");
        let mut log = Log::open(&dir).unwrap();
        log.append(1, &sam, 100, vec![chunk(3)], false, b"raise").unwrap();
        log.append(2, &sam, 101, vec![chunk(4)], false, &vec![7u8; 1000]).unwrap();
        log.close(&sam, 102, "Hills").unwrap();
        loop {
            let (_, edits, changelists) = copy_lengths(&copy);
            if replicate(&copy, &log.tail(edits, changelists, 300).unwrap()).unwrap() {
                break;
            }
        }
        let read = Log::open(&copy).unwrap();
        assert_eq!((read.id(), read.last(), read.changelists().len()), (log.id(), 2, 1));
        assert_eq!(read.read(2).unwrap(), vec![7u8; 1000]);

        let other = Log::open(folder("other")).unwrap();
        assert!(!replicate(&copy, &other.tail(0, 0, 300).unwrap()).unwrap());
        assert_eq!(copy_lengths(&copy), (Some(other.id()), 0, 0));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&copy);
    }

    /// A partial last frame is cut off when the log is opened, and the next
    /// edit is stored after the last whole one.
    #[test]
    fn a_damaged_tail_is_cut_off() {
        let dir = folder("tail");
        let sam = author(1, "Sam");
        let mut log = Log::open(&dir).unwrap();
        log.append(1, &sam, 100, vec![chunk(3)], false, b"one").unwrap();
        log.append(2, &sam, 101, vec![chunk(4)], false, b"two").unwrap();
        let path = dir.join(EDITS_FILE);
        let whole = std::fs::read(&path).unwrap();
        std::fs::write(&path, &whole[..whole.len() - 3]).unwrap();

        let mut log = Log::open(&dir).unwrap();
        assert_eq!(log.last(), 1);
        log.append(2, &sam, 102, vec![chunk(5)], false, b"again").unwrap();
        let log = Log::open(&dir).unwrap();
        assert_eq!((log.last(), log.read(2).unwrap()), (2, b"again".to_vec()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A log written before edits carried `continues` opens, and each of its
    /// edits is whole.
    #[test]
    fn a_log_written_before_continues_opens() {
        #[derive(Serialize)]
        struct Before {
            seq: Seq,
            author: Author,
            time: u64,
            groups: Vec<Group>,
            payload: Vec<u8>,
        }
        let dir = folder("before");
        let mut deflated = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        deflated.write_all(b"raise").unwrap();
        let mut bytes = Vec::new();
        let before = Before { seq: 1, author: author(1, "Sam"), time: 100, groups: vec![chunk(3)], payload: deflated.finish().unwrap() };
        write_frame(&mut bytes, &before).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(EDITS_FILE), &bytes).unwrap();

        let mut log = Log::open(&dir).unwrap();
        assert_eq!((log.last(), log.read(1).unwrap()), (1, b"raise".to_vec()));
        assert!(!log.header(1).unwrap().continues);
        log.append(2, &author(1, "Sam"), 101, vec![chunk(3)], true, b"more").unwrap();
        let log = Log::open(&dir).unwrap();
        assert_eq!((log.last(), log.open_count(&[1; 16])), (2, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Closing puts an author's open edits, and only theirs, in a changelist
    /// that reads back with its description; a group touched after a number
    /// is stale unless the edit that touched it is excepted.
    #[test]
    fn a_changelist_holds_one_authors_open_edits() {
        let dir = folder("changelists");
        let (sam, alex) = (author(1, "Sam"), author(2, "Alex"));
        let mut log = Log::open(&dir).unwrap();
        // Sam's edit 4 is a later part of edit 3, so Sam has two whole edits.
        for (seq, who, at, continues) in [(1, &sam, 3, false), (2, &alex, 4, false), (3, &sam, 5, false), (4, &sam, 6, true)] {
            log.append(seq, who, 100, vec![chunk(at)], continues, b"").unwrap();
        }
        assert_eq!(log.open_count(&sam.id), 2);
        assert_eq!(log.whole_edits(&[1, 3, 4]), 2);
        assert_eq!(log.pending().get(&chunk(4)), Some(&alex.id));
        assert_eq!(log.pending().len(), 4);
        let closed = log.close(&sam, 200, "Raised the ridge\tby the lake\nand the bank").unwrap().unwrap();
        assert_eq!((closed.number, closed.edits.clone()), (1, vec![1, 3, 4]));
        assert!(log.close(&sam, 201, "nothing").unwrap().is_none());
        assert_eq!(log.open_edits(&alex.id), vec![2]);
        assert_eq!(log.pending(), HashMap::from([(chunk(4), alex.id)]), "Sam's submitted edits hold nothing");
        assert_eq!((log.open_count(&sam.id), log.open_count(&alex.id)), (0, 1));

        let log = Log::open(&dir).unwrap();
        assert_eq!(log.changelists(), &[closed]);
        assert_eq!(log.open_counts(), vec![([2; 16], "Alex".to_string(), 1)]);
        assert_eq!(log.header(3).unwrap().changelist, Some(1));
        assert!(log.header(4).unwrap().continues, "read back");
        drop(log);

        // Alex's open edit, withdrawn, is in no changelist and not open.
        let mut log = Log::open(&dir).unwrap();
        assert_eq!(log.withdraw(&alex, 300).unwrap(), 1);
        assert_eq!((log.open_count(&alex.id), log.changelists().len()), (0, 1));
        let log = Log::open(&dir).unwrap();
        assert!(log.pending().is_empty() && log.open_edits(&alex.id).is_empty());
        assert_eq!(log.changelists().len(), 1, "a withdrawal is not a changelist");
        assert_eq!(log.stale(&[chunk(4), chunk(5), chunk(9)], 1, &[]), vec![chunk(4), chunk(5)]);
        assert_eq!(log.stale(&[chunk(4), chunk(5)], 1, &[3]), vec![chunk(4)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_time_reads_as_a_utc_date() {
        assert_eq!(date(0), "1970-01-01 00:00 UTC");
        assert_eq!(date(1_762_697_520), "2025-11-09 14:12 UTC");
        assert_eq!(date(951_782_400), "2000-02-29 00:00 UTC");
    }

    #[test]
    fn runs_and_escapes_read_back() {
        assert_eq!(runs(&[3, 4, 5, 9, 11, 12]), "3-5,9,11-12");
        assert_eq!(parse_runs("3-5,9,11-12"), Some(vec![3, 4, 5, 9, 11, 12]));
        assert_eq!(parse_runs("5-3"), None);
        let text = "a\\b\tc\nd";
        assert_eq!(unescape(&escape(text)), text);
        assert_eq!(parse_hex(&hex(&[0xab; 16])), Some([0xab; 16]));
    }
}
