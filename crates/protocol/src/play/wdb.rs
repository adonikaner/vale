//! **The query answers that survive a logout** — this client's `WDB\` caches.
//!
//! ## Why the answers are kept on disk
//!
//! `Item.dbc`, `creature_template`, `quest_template` and the rest are not in
//! the archives: what a thing is called is a `CMSG_*_QUERY` round trip away.
//! The round trip is the problem rather than the packet. 1.12's interface
//! reads a name **once, when a window opens**: `LootFrame_Update` calls
//! `GetLootSlotInfo` for every row from `LOOT_OPENED` and never again. The
//! client's own event table is 383 names and not one of them announces a
//! query answer arriving. So an
//! answer that lands one millisecond after the window opened is one the player
//! will not see until the window is opened again.
//!
//! The reference solves that by already knowing. It keeps thirteen caches,
//! each a file name and a four-byte signature:
//! `itemcache.wdb` (`BDIW`), `creaturecache.wdb` (`BOMW`),
//! `namecache.wdb` (`MANW`), `gameobjectcache.wdb`, `questcache.wdb`,
//! `npccache.wdb`, `pagetextcache.wdb`, `itemtextcache.wdb`,
//! `petnamecache.wdb`, `guildcache.wdb`, `petitioncache.wdb`,
//! `itemnamecache.wdb` and `wowcache.wdb`. Each is written as its response
//! arrives and read at start-up. This module is that arrangement for the nine
//! answers this client asks for; [`Kind`] lists them and names the reference's
//! file beside each.
//!
//! ## A record is the packet
//!
//! A record is the response body **verbatim**, and it is read back by handing
//! it to the same parser the socket's answer goes through — for four of the
//! kinds by feeding it through the packet dispatch itself, as if it had just
//! arrived. An `ItemInfo` is fifty-odd fields over four arrays, and a
//! hand-rolled serialiser for it is a second, silently divergent reading of
//! the same layout. There is exactly one parser per packet and these files are
//! upstream of it.
//!
//! ## Two ways an answer is used
//!
//! * **State kinds** — item, creature, game object, player name, pet name —
//!   are parsed into `ObjectManager`'s tables before the session thread
//!   starts, so the first loot window of a session already has its names and
//!   the query pass never asks for them.
//! * **On-demand kinds** — quest, NPC text, page text, item text — are held as
//!   bodies. The session loop answers `CMSG_QUEST_QUERY`,
//!   `CMSG_NPC_TEXT_QUERY`, `CMSG_PAGE_TEXT_QUERY` and `CMSG_ITEM_TEXT_QUERY`
//!   out of them by running the body through the dispatch instead of sending
//!   the packet. Every reader sees the answer exactly as it would see the
//!   server's, one tick later rather than one round trip later.
//!
//! ## The container is not a `.wdb`
//!
//! The real container's header and record framing were not measured, and a
//! file that claims to be a WoW cache while being something else would be
//! plausibly wrong. So each file has its own magic and the extension `.wdb2`.
//! They live in the reference's own folder, `WDB\`; **there is no locale
//! sub-directory in 1.12** — `WDB\enUS\` is a 2.x arrangement, and the 1.12.1
//! client's file names are bare and lower-case.
//!
//! ```text
//! u8[4]  magic           — per kind, see Kind::magic
//! u32    format version  — 2
//! u32    client build    — 5875
//! u32    realm key       — a hash of the server address
//! then repeating:
//! u64    key             — entry, id, or guid; 0 ends the file
//! u32    body length
//! u8[]   the response body
//! ```
//!
//! A header that does not match is an absent cache, not an error: a different
//! build or a different server has different templates under the same
//! entries. The next append rewrites the file from its header down.
//!
//! A key that appears twice is read **last wins**: a pet renamed under the
//! same pet number is appended again rather than edited in place.
//!
//! A damaged tail stops the read cleanly. The file is appended to while the
//! client runs, and a crash is a half-written last record, which must not
//! discard the good ones in front of it.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::bytes::{Reader, Writer};
use crate::opcodes::Opcode;

const VERSION: u32 = 2;
pub const HEADER_LEN: usize = 16;

/// The largest body a file will believe. The length is a `u32` read off a file
/// that may have been truncated mid-write. The largest real answer is a quest
/// template at a few kilobytes; anything past this stops the read rather than
/// allocating it.
const MAX_BODY: usize = 16 * 1024;

/// How many records one file may hold. 1.12 has ~19,000 items and ~9,000
/// creature templates and a character will not meet them all; an addon asking
/// about entries in a loop should not fill a disk.
const MAX_RECORDS: usize = 32_768;

/// One kind of answer, and one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum Kind {
    /// `SMSG_ITEM_QUERY_SINGLE_RESPONSE`, keyed by item entry.
    Item,
    /// `SMSG_CREATURE_QUERY_RESPONSE`, keyed by creature entry.
    Creature,
    /// `SMSG_GAMEOBJECT_QUERY_RESPONSE`, keyed by game-object entry.
    GameObject,
    /// `SMSG_NAME_QUERY_RESPONSE`, keyed by player guid.
    Name,
    /// `SMSG_PET_NAME_QUERY_RESPONSE`, keyed by pet number. Appended again on a
    /// rename; the reader takes the last record for a key.
    PetName,
    /// `SMSG_QUEST_QUERY_RESPONSE`, keyed by quest id.
    Quest,
    /// `SMSG_NPC_TEXT_UPDATE`, keyed by `npc_text` id.
    NpcText,
    /// `SMSG_PAGE_TEXT_QUERY_RESPONSE`, keyed by page id.
    PageText,
    /// `SMSG_ITEM_TEXT_QUERY_RESPONSE`, keyed by item text id.
    ItemText,
}

impl Kind {
    pub const ALL: [Kind; 9] = [
        Kind::Item,
        Kind::Creature,
        Kind::GameObject,
        Kind::Name,
        Kind::PetName,
        Kind::Quest,
        Kind::NpcText,
        Kind::PageText,
        Kind::ItemText,
    ];

    pub const COUNT: usize = Kind::ALL.len();

    /// The file's stem: the reference's own stem, capitalised, so the two
    /// families sort together in the folder.
    pub fn stem(self) -> &'static str {
        match self {
            Kind::Item => "ItemCache",
            Kind::Creature => "CreatureCache",
            Kind::GameObject => "GameObjectCache",
            Kind::Name => "NameCache",
            Kind::PetName => "PetNameCache",
            Kind::Quest => "QuestCache",
            Kind::NpcText => "NpcCache",
            Kind::PageText => "PageTextCache",
            Kind::ItemText => "ItemTextCache",
        }
    }

    /// The reference's file for the same answers.
    pub fn reference_file(self) -> &'static str {
        match self {
            Kind::Item => "itemcache.wdb",
            Kind::Creature => "creaturecache.wdb",
            Kind::GameObject => "gameobjectcache.wdb",
            Kind::Name => "namecache.wdb",
            Kind::PetName => "petnamecache.wdb",
            Kind::Quest => "questcache.wdb",
            Kind::NpcText => "npccache.wdb",
            Kind::PageText => "pagetextcache.wdb",
            Kind::ItemText => "itemtextcache.wdb",
        }
    }

    /// The file's magic. Each kind has its own so that a file renamed by hand
    /// is refused rather than parsed as another kind's records.
    pub fn magic(self) -> &'static [u8; 4] {
        match self {
            Kind::Item => b"AZIC",
            Kind::Creature => b"AZCC",
            Kind::GameObject => b"AZGC",
            Kind::Name => b"AZNC",
            Kind::PetName => b"AZPN",
            Kind::Quest => b"AZQC",
            Kind::NpcText => b"AZNT",
            Kind::PageText => b"AZPT",
            Kind::ItemText => b"AZIT",
        }
    }

    /// The opcode a record is replayed as.
    pub fn opcode(self) -> Opcode {
        match self {
            Kind::Item => Opcode::SMSG_ITEM_QUERY_SINGLE_RESPONSE,
            Kind::Creature => Opcode::SMSG_CREATURE_QUERY_RESPONSE,
            Kind::GameObject => Opcode::SMSG_GAMEOBJECT_QUERY_RESPONSE,
            Kind::Name => Opcode::SMSG_NAME_QUERY_RESPONSE,
            Kind::PetName => Opcode::SMSG_PET_NAME_QUERY_RESPONSE,
            Kind::Quest => Opcode::SMSG_QUEST_QUERY_RESPONSE,
            Kind::NpcText => Opcode::SMSG_NPC_TEXT_UPDATE,
            Kind::PageText => Opcode::SMSG_PAGE_TEXT_QUERY_RESPONSE,
            Kind::ItemText => Opcode::SMSG_ITEM_TEXT_QUERY_RESPONSE,
        }
    }

    /// Whether the answer is held as a body and given when asked for, rather
    /// than parsed into a table at start-up. See the module doc.
    pub fn on_demand(self) -> bool {
        matches!(
            self,
            Kind::Quest | Kind::NpcText | Kind::PageText | Kind::ItemText
        )
    }

    /// The key a body carries, read off its first field.
    ///
    /// Every one of the nine responses begins with its key: a `u32` entry or
    /// id, or the `u64` guid for a name. A refusal — the entry with its high
    /// bit set, which is how vmangos declines an item, creature or game-object
    /// query — is not a record and answers `None`.
    pub fn key_of(self, body: &[u8]) -> Option<u64> {
        let mut r = Reader::new(body);
        match self {
            Kind::Name => r.has(8).then(|| r.u64()),
            Kind::Item | Kind::Creature | Kind::GameObject => {
                if !r.has(4) {
                    return None;
                }
                let entry = r.u32();
                (entry & 0x8000_0000 == 0).then_some(u64::from(entry))
            }
            _ => r.has(4).then(|| u64::from(r.u32())),
        }
    }
}

/// One answer learned this session and not on disk yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Learned {
    pub kind: Kind,
    pub key: u64,
    pub body: Vec<u8>,
}

/// The on-disk records of one kind for one (build, realm) pair.
pub struct CacheFile {
    kind: Kind,
    path: PathBuf,
    build: u32,
    realm: u32,
    /// What was read at open, last record per key, in file order of first
    /// appearance.
    seed: Vec<(u64, Vec<u8>)>,
    /// Every distinct key on disk, read or appended.
    known: HashSet<u64>,
    /// Records on disk, including repeats, against [`MAX_RECORDS`].
    records: usize,
    /// The header is missing or disagrees, so the next write starts the file
    /// again rather than appending to somebody else's.
    rewrite: bool,
}

impl CacheFile {
    /// Open (or invent) one kind's cache for one server.
    ///
    /// Never fails. An unreadable, absent, truncated or foreign file is an
    /// empty cache: the client works without one, and a session should not go
    /// down over a cache file.
    pub fn open(path: impl Into<PathBuf>, kind: Kind, build: u32, realm: u32) -> CacheFile {
        let mut cache = CacheFile {
            kind,
            path: path.into(),
            build,
            realm,
            seed: Vec::new(),
            known: HashSet::new(),
            records: 0,
            rewrite: true,
        };
        let Ok(mut file) = File::open(&cache.path) else {
            return cache;
        };
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            return cache;
        }
        cache.read(&bytes);
        cache
    }

    /// A cache with no file behind it.
    pub fn none(kind: Kind) -> CacheFile {
        CacheFile {
            kind,
            path: PathBuf::new(),
            build: 0,
            realm: 0,
            seed: Vec::new(),
            known: HashSet::new(),
            records: 0,
            rewrite: false,
        }
    }

    fn read(&mut self, bytes: &[u8]) {
        let mut r = Reader::new(bytes);
        if !r.has(HEADER_LEN) {
            return;
        }
        if r.bytes(4) != self.kind.magic() || r.u32() != VERSION {
            return;
        }
        if r.u32() != self.build || r.u32() != self.realm {
            return;
        }
        // The header is ours: whatever follows is appendable.
        self.rewrite = false;
        // Where each key sits in `seed`, so a repeat replaces in place.
        let mut index: HashMap<u64, usize> = HashMap::new();
        while self.records < MAX_RECORDS {
            if !r.has(12) {
                break;
            }
            let key = r.u64();
            if key == 0 {
                break;
            }
            let len = r.u32() as usize;
            // A length past the end is a truncated tail, which is what a crash
            // mid-append leaves. Stop, keep what came before.
            if len == 0 || len > MAX_BODY || !r.has(len) {
                break;
            }
            let body = r.bytes(len);
            self.records += 1;
            self.known.insert(key);
            match index.get(&key) {
                Some(&at) => self.seed[at].1 = body,
                None => {
                    index.insert(key, self.seed.len());
                    self.seed.push((key, body));
                }
            }
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The records read from disk, one per key.
    pub fn seed(&self) -> &[(u64, Vec<u8>)] {
        &self.seed
    }

    /// How many distinct keys are on disk.
    pub fn len(&self) -> usize {
        self.known.len()
    }

    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    /// Whether this cache has anywhere to write.
    pub fn enabled(&self) -> bool {
        !self.path.as_os_str().is_empty()
    }

    /// Append these records, answering how many were written.
    ///
    /// No de-duplication here: the world decides what is new (see
    /// `ObjectManager::remember`), and a repeat for a key is a replacement on
    /// the next read. The file's own bound is [`MAX_RECORDS`].
    pub fn append(&mut self, records: &[(u64, &[u8])]) -> io::Result<usize> {
        if !self.enabled() {
            return Ok(0);
        }
        let mut out = Writer::new();
        let mut written = 0usize;
        for (key, body) in records {
            if *key == 0 || body.is_empty() || body.len() > MAX_BODY {
                continue;
            }
            if self.records >= MAX_RECORDS {
                break;
            }
            self.records += 1;
            self.known.insert(*key);
            out.u64(*key);
            out.u32(body.len() as u32);
            out.bytes(body);
            written += 1;
        }
        if written == 0 && !self.rewrite {
            return Ok(0);
        }
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        // The header decides whether this is an append or a fresh file. A
        // foreign cache is not deleted at open, so a read-only run leaves it
        // alone; it is overwritten the first time this session has something
        // of its own to write.
        let mut file = match self.rewrite {
            true => {
                let mut file = File::create(&self.path)?;
                let mut header = Writer::new();
                header.bytes(self.kind.magic());
                header.u32(VERSION);
                header.u32(self.build);
                header.u32(self.realm);
                file.write_all(&header.buf)?;
                self.rewrite = false;
                file
            }
            false => OpenOptions::new().append(true).open(&self.path)?,
        };
        file.write_all(&out.buf)?;
        Ok(written)
    }
}

/// The nine files of one server, opened together.
pub struct Caches {
    files: Vec<CacheFile>,
}

impl Caches {
    /// Open every kind's file under `dir` for one (build, realm) pair.
    pub fn open(dir: impl AsRef<Path>, build: u32, realm: u32) -> Caches {
        let dir = dir.as_ref();
        Caches {
            files: Kind::ALL
                .iter()
                .map(|&kind| CacheFile::open(cache_path(dir, kind, realm), kind, build, realm))
                .collect(),
        }
    }

    /// Caches with no files behind them, for the tests and for a caller that
    /// has nowhere to put one. A session is identical either way, except that
    /// every window naming something it has never met is blank on its first
    /// opening.
    pub fn none() -> Caches {
        Caches {
            files: Kind::ALL.iter().map(|&kind| CacheFile::none(kind)).collect(),
        }
    }

    pub fn get(&self, kind: Kind) -> &CacheFile {
        &self.files[kind as usize]
    }

    /// The records read from disk for one kind.
    pub fn seed(&self, kind: Kind) -> &[(u64, Vec<u8>)] {
        self.files[kind as usize].seed()
    }

    /// Whether anything can be written.
    pub fn enabled(&self) -> bool {
        self.files.iter().any(CacheFile::enabled)
    }

    /// Distinct keys on disk, over every kind.
    pub fn total(&self) -> usize {
        self.files.iter().map(CacheFile::len).sum()
    }

    pub fn files(&self) -> &[CacheFile] {
        &self.files
    }

    /// Append everything learned, kind by kind, answering how many records
    /// were written. A file that cannot be written costs that file's records
    /// and nothing else.
    pub fn append(&mut self, learned: &[Learned]) -> io::Result<usize> {
        let mut written = 0usize;
        let mut first_error = None;
        for file in &mut self.files {
            let records: Vec<(u64, &[u8])> = learned
                .iter()
                .filter(|l| l.kind == file.kind)
                .map(|l| (l.key, l.body.as_slice()))
                .collect();
            if records.is_empty() {
                continue;
            }
            match file.append(&records) {
                Ok(n) => written += n,
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        match first_error {
            Some(e) if written == 0 => Err(e),
            _ => Ok(written),
        }
    }

    /// One line per kind that holds anything: `ItemCache 412, CreatureCache 90`.
    pub fn summary(&self) -> String {
        let parts: Vec<String> = self
            .files
            .iter()
            .filter(|f| !f.is_empty())
            .map(|f| format!("{} {}", f.kind.stem(), f.len()))
            .collect();
        match parts.is_empty() {
            true => "nothing on disk".to_string(),
            false => parts.join(", "),
        }
    }
}

/// A stable key for one server address, so two realms cannot share a file's
/// entries. FNV-1a over the bytes: this needs to be the same number next
/// launch and nothing else, so a hash with a randomised seed would be wrong.
pub fn realm_key(address: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in address.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Where one kind's file lives for one server: `<dir>/<Stem>-<realm>.wdb2`.
///
/// The realm key is in the name as well as in the header because two servers'
/// caches sit in one `WDB\` folder: the header check is what makes a
/// mismatched file safe, and the name is what stops the second server
/// rewriting the first one's file on every append.
pub fn cache_path(dir: impl AsRef<Path>, kind: Kind, realm: u32) -> PathBuf {
    dir.as_ref().join(format!("{}-{realm:08x}.wdb2", kind.stem()))
}

/// Every realm key that has a cache file under `dir`, from the file names.
///
/// For a check that has no socket in hand: the key in a session's file name
/// is a hash of the world socket's peer address, which no configuration
/// states, so a reader that wants to open what a session wrote scans the
/// folder rather than computing a key from `realmlist.wtf`.
pub fn realms_on_disk(dir: impl AsRef<Path>) -> Vec<u32> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut realms: Vec<u32> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let stem = name.strip_suffix(".wdb2")?;
            let (_, hex) = stem.rsplit_once('-')?;
            (hex.len() == 8).then(|| u32::from_str_radix(hex, 16).ok())?
        })
        .collect();
    realms.sort_unstable();
    realms.dedup();
    realms
}

/// Where the caches live, relative to whatever directory the client was run
/// from — which for a drop-in build is the install root, so this is the
/// reference's own `WDB\`, the folder its own thirteen `.wdb` files go into.
///
/// Here rather than in `vale_config` with the rest of the folder's shape,
/// because this crate is the one that writes the files and does not depend on
/// that one.
pub const CACHE_DIR: &str = "WDB";

/// The caches for one world server, at this client's build.
///
/// The one call both the renderer and the CLI make, so that a `WDB\` written
/// by `vale live` is the same set of files the game reads.
pub fn for_session(session: &crate::socket::world::WorldSession) -> Caches {
    // The socket's own peer, not the configured address: a realmlist line and
    // a realm-list entry can name the same machine two ways, and two names for
    // one server would be two caches with the same templates in them. An
    // unaskable socket falls back to a shared key rather than to no cache.
    match session.peer_address() {
        Some(address) => for_server(&address),
        None => for_server("unknown"),
    }
}

/// …and by address, for a caller with no socket in hand.
pub fn for_server(address: &str) -> Caches {
    let realm = realm_key(address);
    Caches::open(CACHE_DIR, u32::from(crate::version::BUILD), realm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push("vale-wdb-tests");
        let _ = fs::create_dir_all(&dir);
        dir.push(name);
        let _ = fs::remove_file(&dir);
        dir
    }

    fn body(entry: u32, name: &str) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(entry);
        w.u32(0); // class
        w.u32(0); // subclass
        w.bytes(name.as_bytes());
        w.u8(0);
        w.buf
    }

    fn learned(kind: Kind, key: u64, body: Vec<u8>) -> Learned {
        Learned { kind, key, body }
    }

    /// `WDB\` is where 5875 keeps its thirteen caches, and `.wdb2` is what
    /// stops anything reading these as the fourteenth.
    #[test]
    fn a_cache_lands_in_the_references_own_folder() {
        let path = cache_path(CACHE_DIR, Kind::Item, 0xd7ca_03f3);
        assert_eq!(path, PathBuf::from("WDB").join("ItemCache-d7ca03f3.wdb2"));
        assert_eq!(
            cache_path(CACHE_DIR, Kind::Creature, 0xd7ca_03f3),
            PathBuf::from("WDB").join("CreatureCache-d7ca03f3.wdb2")
        );
        // Two servers share one folder, so the key has to be in the name as
        // well as in the header.
        assert_ne!(cache_path(CACHE_DIR, Kind::Item, 1), cache_path(CACHE_DIR, Kind::Item, 2));
        // …and every kind has its own file, stem, magic and reference file.
        for (i, a) in Kind::ALL.iter().enumerate() {
            for b in &Kind::ALL[i + 1..] {
                assert_ne!(a.stem(), b.stem());
                assert_ne!(a.magic(), b.magic());
                assert_ne!(a.reference_file(), b.reference_file());
                assert_ne!(a.opcode(), b.opcode());
            }
        }
    }

    /// Written, then read back, then appended to — across three opens.
    #[test]
    fn a_cache_survives_being_closed_and_reopened() {
        let path = temp("roundtrip.cache");
        let mut cache = CacheFile::open(&path, Kind::Item, 5875, 7);
        assert!(cache.is_empty());
        assert_eq!(
            cache
                .append(&[(2589, &body(2589, "Linen Cloth")), (858, &body(858, "Minor Healing Potion"))])
                .expect("writes"),
            2
        );

        let reopened = CacheFile::open(&path, Kind::Item, 5875, 7);
        assert_eq!(reopened.len(), 2);
        assert_eq!(reopened.seed().len(), 2);
        assert_eq!(reopened.seed()[0], (2589, body(2589, "Linen Cloth")));

        // A second session appends without losing the first's.
        let mut reopened = reopened;
        assert_eq!(reopened.append(&[(117, &body(117, "Tough Jerky"))]).unwrap(), 1);
        assert_eq!(CacheFile::open(&path, Kind::Item, 5875, 7).len(), 3);
    }

    /// A key written twice reads as its last body: a renamed pet is a new
    /// record under the old number.
    #[test]
    fn a_repeated_key_reads_last_wins() {
        let path = temp("repeat.cache");
        let mut cache = CacheFile::open(&path, Kind::PetName, 5875, 7);
        cache.append(&[(4, b"\x04\0\0\0Rex\0\x01\0\0\0")]).unwrap();
        cache.append(&[(4, b"\x04\0\0\0Fang\0\x02\0\0\0")]).unwrap();
        let reopened = CacheFile::open(&path, Kind::PetName, 5875, 7);
        assert_eq!(reopened.len(), 1, "one key");
        assert_eq!(reopened.seed()[0].1, b"\x04\0\0\0Fang\0\x02\0\0\0");
    }

    /// A foreign header is an empty cache, not somebody else's names. A
    /// different build or realm reuses the same entry numbers for different
    /// things, and so does a file of another kind.
    #[test]
    fn a_cache_from_another_build_realm_or_kind_is_not_read() {
        let path = temp("foreign.cache");
        CacheFile::open(&path, Kind::Item, 5875, 7)
            .append(&[(2589, &body(2589, "Linen Cloth"))])
            .expect("writes");
        assert_eq!(CacheFile::open(&path, Kind::Item, 5875, 7).len(), 1);
        assert_eq!(CacheFile::open(&path, Kind::Item, 5875, 8).len(), 0, "another realm");
        assert_eq!(CacheFile::open(&path, Kind::Item, 8606, 7).len(), 0, "another build");
        assert_eq!(CacheFile::open(&path, Kind::Creature, 5875, 7).len(), 0, "another kind");

        // The mismatched open rewrites rather than appending, so the file never
        // holds two headers' worth of records.
        let mut other = CacheFile::open(&path, Kind::Item, 5875, 8);
        other.append(&[(117, &body(117, "Tough Jerky"))]).expect("writes");
        assert_eq!(CacheFile::open(&path, Kind::Item, 5875, 8).len(), 1);
        assert_eq!(CacheFile::open(&path, Kind::Item, 5875, 7).len(), 0, "the old one is gone");
    }

    /// A half-written record is dropped and the rest kept. This file is
    /// appended to by a running client, so a crash leaves exactly that.
    #[test]
    fn a_truncated_tail_keeps_everything_in_front_of_it() {
        let path = temp("truncated.cache");
        CacheFile::open(&path, Kind::Item, 5875, 7)
            .append(&[
                (2589, &body(2589, "Linen Cloth")),
                (858, &body(858, "Minor Healing Potion")),
            ])
            .expect("writes");
        let whole = fs::read(&path).expect("reads");
        for cut in [whole.len() - 1, whole.len() - 4, whole.len() - 8, whole.len() - 12] {
            let path = temp(&format!("truncated-{cut}.cache"));
            fs::write(&path, &whole[..cut]).expect("writes");
            assert_eq!(CacheFile::open(&path, Kind::Item, 5875, 7).len(), 1, "cut at {cut}");
        }
        // A header alone is a valid empty cache rather than a foreign one.
        let path = temp("header-only.cache");
        fs::write(&path, &whole[..HEADER_LEN]).expect("writes");
        assert_eq!(CacheFile::open(&path, Kind::Item, 5875, 7).len(), 0);
        // …and a body length past the end of the file stops the read.
        let mut damaged = whole[..HEADER_LEN].to_vec();
        damaged.extend_from_slice(&2589u64.to_le_bytes());
        damaged.extend_from_slice(&u32::MAX.to_le_bytes());
        let path = temp("absurd-length.cache");
        fs::write(&path, &damaged).expect("writes");
        assert_eq!(CacheFile::open(&path, Kind::Item, 5875, 7).len(), 0);
    }

    /// A cache with nowhere to write is inert rather than a special case at
    /// every call site.
    #[test]
    fn caches_with_no_path_write_nothing() {
        let mut caches = Caches::none();
        assert!(!caches.enabled());
        assert_eq!(
            caches
                .append(&[learned(Kind::Item, 2589, body(2589, "Linen Cloth"))])
                .unwrap(),
            0
        );
        assert!(caches.seed(Kind::Item).is_empty());
        assert_eq!(caches.summary(), "nothing on disk");
    }

    /// The set routes each record to its kind's file, and reads them back
    /// under the same kind.
    #[test]
    fn the_set_routes_each_kind_to_its_own_file() {
        let mut dir = std::env::temp_dir();
        dir.push("vale-wdb-tests");
        dir.push("set");
        let _ = fs::remove_dir_all(&dir);
        let mut caches = Caches::open(&dir, 5875, 7);
        let written = caches
            .append(&[
                learned(Kind::Item, 2589, body(2589, "Linen Cloth")),
                learned(Kind::Creature, 69, body(69, "Diseased Wolf")),
                learned(Kind::Creature, 299, body(299, "Young Wolf")),
                learned(Kind::Name, 0x1234, b"\x34\x12\0\0\0\0\0\0Bram\0\0\x03\0\0\0\0\0\0\0\x01\0\0\0".to_vec()),
            ])
            .unwrap();
        assert_eq!(written, 4);
        let reopened = Caches::open(&dir, 5875, 7);
        assert_eq!(reopened.seed(Kind::Item).len(), 1);
        assert_eq!(reopened.seed(Kind::Creature).len(), 2);
        assert_eq!(reopened.seed(Kind::Name).len(), 1);
        assert_eq!(reopened.seed(Kind::Quest).len(), 0);
        assert_eq!(reopened.total(), 4);
        assert_eq!(reopened.summary(), "ItemCache 1, CreatureCache 2, NameCache 1");
    }

    /// Every response begins with its key, and a refusal is not a record.
    #[test]
    fn the_key_is_read_off_the_front_of_the_body() {
        assert_eq!(Kind::Item.key_of(&body(2589, "Linen Cloth")), Some(2589));
        assert_eq!(Kind::Item.key_of(&(2589u32 | 0x8000_0000).to_le_bytes()), None);
        assert_eq!(Kind::Creature.key_of(&69u32.to_le_bytes()), Some(69));
        assert_eq!(Kind::Name.key_of(&0x1234u64.to_le_bytes()), Some(0x1234));
        assert_eq!(Kind::Name.key_of(&[1, 2, 3]), None);
        assert_eq!(Kind::Quest.key_of(&47u32.to_le_bytes()), Some(47));
    }

    /// The folder scan finds every realm with a file, once, by the hex key in
    /// the file name, and ignores anything else in the folder.
    #[test]
    fn the_realms_on_disk_are_read_off_the_file_names() {
        let mut dir = std::env::temp_dir();
        dir.push("vale-wdb-tests");
        dir.push("realms");
        let _ = fs::remove_dir_all(&dir);
        assert!(realms_on_disk(&dir).is_empty(), "no folder is no realms");
        fs::create_dir_all(&dir).unwrap();
        fs::write(cache_path(&dir, Kind::Item, 0xd7ca_03f3), b"").unwrap();
        fs::write(cache_path(&dir, Kind::Creature, 0xd7ca_03f3), b"").unwrap();
        fs::write(cache_path(&dir, Kind::Quest, 0x0000_0007), b"").unwrap();
        fs::write(dir.join("itemcache.wdb"), b"").unwrap();
        fs::write(dir.join("notes.txt"), b"").unwrap();
        assert_eq!(realms_on_disk(&dir), vec![0x0000_0007, 0xd7ca_03f3]);
    }

    /// The realm key is a stable hash: the same number next launch.
    #[test]
    fn the_realm_key_is_stable_and_distinguishes_servers() {
        assert_eq!(realm_key("127.0.0.1:8085"), realm_key("127.0.0.1:8085"));
        assert_ne!(realm_key("127.0.0.1:8085"), realm_key("127.0.0.1:8086"));
        assert_eq!(realm_key("127.0.0.1:8085"), 0xd7ca_03f3, "FNV-1a, pinned");
    }
}
