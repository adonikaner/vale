//! The MPQ archive chain.
//!
//! A WoW install is a stack of archives, and later patches shadow earlier
//! files. A lookup walks the stack in the client's load order and takes the
//! first hit. A wrong order renders 1.12.0 terrain on a 1.12.1 server without
//! any error.
//!
//! Load order (lowest priority first), matching the retail client:
//! `base < patch.MPQ < patch-2..9 < patch-A..Z`. Lookups search in reverse.
//!
//! ## Loose files under `Interface\AddOns\`
//!
//! A third-party addon is a directory on disk, not an archive entry, and the
//! reference client reads its files by the same virtual paths it uses for the
//! archives. A chain opened with [`Assets::with_loose_root`] answers a path
//! under [`LOOSE_PREFIX`] from `<root>\Interface\AddOns\…` first and from the
//! archives second. Only that prefix is looked up on disk; every other read
//! stays an archive read, so the check costs one prefix comparison.
//!
//! ## The overlay in front of the chain
//!
//! [`Assets::with_overlay`] puts a function in front of the whole chain, for any
//! path. It serves a caller that must answer with a tile being edited, before
//! that tile has been saved anywhere. The reference client has no such
//! mechanism: it reads world data from `Data\*.MPQ` only. The overlay is this
//! project's own mechanism and belongs to the host that installs one, not to
//! this crate.
//!
//! It costs one `Option` check per read when nothing is installed.

use crate::AssetError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use wow_mpq::Archive;

/// The largest file size a read accepts: an upper bound on any file in a 1.12
/// install, with a wide margin.
///
/// An MPQ block-table entry states a file's uncompressed size, and `wow_mpq`
/// begins a read with a `Vec::with_capacity` of it. A damaged entry therefore
/// allocates whatever the damaged bytes say before the content is examined. The
/// process then allocates four gigabytes, stalls and fails, and `wow_mpq`'s
/// warning about the sector offsets of the same entry names no path.
///
/// The bound is 256 MB. Measured over every file of every archive of a 1.12
/// install (97,000 files across seventeen archives), the largest is 16,307,975
/// bytes, and `patch.MPQ`'s largest is 16,300,699. The bound is sixteen times
/// the largest shipped file and still refuses an entry claiming gigabytes.
///
/// A file over the bound is skipped, not treated as an error, because a later
/// archive in the chain may hold a good copy of the same path.
/// [`Assets::damaged`] records the skip.
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// One opened archive plus the name we report it as.
struct Mounted {
    name: String,
    archive: Archive,
}

/// A read-only view over a directory of `.MPQ` files.
pub struct Assets {
    archives: Vec<Mounted>,
    /// Archives that failed to open, kept so callers can surface them instead
    /// of silently rendering an incomplete world.
    pub failures: Vec<(String, String)>,
    /// Entries this chain refused as damaged: the archive, the path, and the
    /// size it claimed. See [`MAX_FILE_BYTES`].
    ///
    /// Recorded, not printed, as with [`Self::failures`]: a library does not
    /// print, and the host decides whether a damaged entry goes to a HUD or to
    /// stdout. Deduplicated by path and capped at [`Self::DAMAGED_KEPT`],
    /// because a caller may request the same damaged file every frame.
    ///
    /// The list belongs to this chain, not to the install, and no host reads it
    /// yet for that reason. A process holds several chains: the renderer's
    /// `GameAssets` resource holds one, and task-pool work that reads tiles and
    /// buildings borrows from a [`ChainPool`] so that several tiles load at
    /// once. Damaged entries are met during tile streaming, so a HUD line fed
    /// from the resource's chain would be empty in the case it exists for.
    ///
    /// A borrowed chain keeps this list when it returns to the pool, so the
    /// pooled chains together accumulate what the whole session met. An
    /// install-wide list still needs either a shared sink handed to every chain
    /// at open, or the pool merging its chains' lists on return. Neither is
    /// written; a host that reads this field without one of them reports only
    /// part of the damaged entries.
    pub damaged: Vec<(String, String, u64)>,
    listing: Option<HashMap<String, usize>>,
    /// The install root whose `Interface\AddOns\` is read before the archives,
    /// or `None` for a chain that answers from the archives alone.
    loose: Option<PathBuf>,
    /// Asked before the archives, for every path. See the module comment.
    overlay: Option<Overlay>,
}

/// A source consulted ahead of the whole chain.
///
/// `Send + Sync` because an [`Assets`] is read from the task pool, where tile
/// loads run; an overlay that could not cross threads would be invisible to the
/// terrain streamer.
pub type Overlay = std::sync::Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// The one virtual prefix answered from disk. Compared case-insensitively.
pub const LOOSE_PREFIX: &str = r"Interface\AddOns\";

/// `<root>` joined with a virtual path, matched component by component without
/// regard to case, since the addon's `.toc` and the disk may spell a name
/// differently and the reference client runs on a case-insensitive file system.
/// `None` when any component is missing.
pub fn resolve_loose(root: &Path, vpath: &str) -> Option<PathBuf> {
    let mut at = root.to_path_buf();
    for component in vpath.split('\\').filter(|c| !c.is_empty()) {
        if component == ".." {
            at.pop();
            continue;
        }
        let exact = at.join(component);
        if exact.exists() {
            at = exact;
            continue;
        }
        let wanted = component.to_ascii_lowercase();
        let found = std::fs::read_dir(&at)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|f| f.to_string_lossy().to_ascii_lowercase() == wanted)
            })?;
        at = found;
    }
    at.is_file().then_some(at)
}

/// Priority for the client's archive load order. Higher wins.
///
/// Ported from the equivalent rule in an earlier JavaScript implementation, so
/// the two resolve the same file to the same bytes.
fn patch_priority(file_name: &str) -> u32 {
    let lower = file_name.to_ascii_lowercase();
    let Some(stem) = lower.strip_suffix(".mpq") else {
        return 0;
    };
    match stem {
        // Un-suffixed `patch.MPQ` outranks every base archive.
        "patch" => 1000,
        _ => match stem.strip_prefix("patch-") {
            // Single character suffix: digits rank below letters.
            Some(suffix) if suffix.len() == 1 => {
                let c = suffix.as_bytes()[0] as u32;
                if suffix.as_bytes()[0].is_ascii_alphabetic() {
                    2000 + c
                } else {
                    1000 + c
                }
            }
            _ => 0, // base archives: dbc, model, terrain, texture, ...
        },
    }
}

impl Assets {
    /// Open every `.MPQ` in `dir` (and in `dir/Data`, so pointing this at a WoW
    /// install root also works).
    pub fn open(dir: impl AsRef<Path>) -> Result<Assets, AssetError> {
        let dir = dir.as_ref();
        let mut found: Vec<(PathBuf, String)> = Vec::new();

        for candidate in [dir.to_path_buf(), dir.join("Data")] {
            let Ok(entries) = std::fs::read_dir(&candidate) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let is_mpq = path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("mpq"));
                if is_mpq && path.is_file() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    found.push((path, name));
                }
            }
        }

        if found.is_empty() {
            return Err(AssetError::NoArchives(dir.display().to_string()));
        }

        // Highest priority first, so the first hit during lookup is the winner.
        found.sort_by(|a, b| {
            patch_priority(&b.1)
                .cmp(&patch_priority(&a.1))
                .then_with(|| a.1.to_ascii_lowercase().cmp(&b.1.to_ascii_lowercase()))
        });

        let mut archives = Vec::new();
        let mut failures = Vec::new();
        for (path, name) in found {
            match Archive::open(&path) {
                Ok(archive) => archives.push(Mounted { name, archive }),
                Err(e) => failures.push((name, e.to_string())),
            }
        }

        Ok(Assets {
            archives,
            failures,
            damaged: Vec::new(),
            listing: None,
            loose: None,
            overlay: None,
        })
    }

    /// Answer [`LOOSE_PREFIX`] paths from `<root>\Interface\AddOns\` on disk
    /// before the archives. See the module comment.
    pub fn with_loose_root(mut self, root: impl AsRef<Path>) -> Assets {
        self.loose = Some(root.as_ref().to_path_buf());
        self
    }

    /// Answer any path from `overlay` before looking in the archives.
    ///
    /// The overlay decides for itself which paths it knows; returning `None`
    /// falls through to the loose root and then to the chain, so installing one
    /// changes nothing about a path it does not carry.
    pub fn with_overlay(mut self, overlay: Overlay) -> Assets {
        self.overlay = Some(overlay);
        self
    }

    /// The same on a chain that is already open, which is how a host installs
    /// one after the archives have been mounted.
    pub fn set_overlay(&mut self, overlay: Option<Overlay>) {
        self.overlay = overlay;
    }

    /// The root given to [`Self::with_loose_root`], if any.
    pub fn loose_root(&self) -> Option<&Path> {
        self.loose.as_deref()
    }

    /// Names of the opened archives, in lookup order.
    pub fn archive_names(&self) -> Vec<&str> {
        self.archives.iter().map(|a| a.name.as_str()).collect()
    }

    /// Read a game file by its virtual path, e.g. `World\Maps\Azeroth\Azeroth.wdt`.
    ///
    /// Forward slashes are accepted and normalised to backslashes, which is
    /// what MPQ stores; without that, a Unix-style path reports "not found".
    pub fn read(&mut self, vpath: &str) -> Result<Vec<u8>, AssetError> {
        self.read_at(vpath, true, &[])
    }

    /// [`Self::read`] with the overlay skipped: what the install holds without
    /// whatever the host has put in front of it.
    ///
    /// The archives and the loose root are still read, because both are part
    /// of the install; only the overlay, which belongs to the running process,
    /// is skipped.
    ///
    /// This is for a caller that compares against the file the game shipped.
    /// With no overlay installed it returns the same as [`Self::read`]. All
    /// callers in this repository but one use [`Self::read`].
    pub fn read_past_overlay(&mut self, vpath: &str) -> Result<Vec<u8>, AssetError> {
        self.read_at(vpath, false, &[])
    }

    /// [`Self::read_past_overlay`], also skipping the named archives: what the
    /// install held before those archives were added to it.
    ///
    /// A caller comparing against the file the game shipped needs this when
    /// some of the archives in the folder are its own output: a comparison
    /// against a chain that includes them compares an edit with itself. Names
    /// are matched without regard to case, as the chain's own order is.
    pub fn read_past_overlay_skipping(
        &mut self,
        vpath: &str,
        skip: &[String],
    ) -> Result<Vec<u8>, AssetError> {
        self.read_at(vpath, false, skip)
    }

    fn read_at(
        &mut self,
        vpath: &str,
        through_overlay: bool,
        skip: &[String],
    ) -> Result<Vec<u8>, AssetError> {
        let normalised = vpath.replace('/', "\\");
        if through_overlay {
            if let Some(overlay) = &self.overlay {
                if let Some(data) = overlay(&normalised) {
                    return Ok(data);
                }
            }
        }
        if let Some(root) = &self.loose {
            if normalised.len() > LOOSE_PREFIX.len()
                && normalised[..LOOSE_PREFIX.len()].eq_ignore_ascii_case(LOOSE_PREFIX)
            {
                if let Some(data) =
                    resolve_loose(root, &normalised).and_then(|path| std::fs::read(path).ok())
                {
                    return Ok(data);
                }
            }
        }
        let mut refused: Option<(String, u64)> = None;
        for mounted in &mut self.archives {
            if skip.iter().any(|name| name.eq_ignore_ascii_case(&mounted.name)) {
                continue;
            }
            // Check the stated size before reading; see [`MAX_FILE_BYTES`].
            // The lookup adds no cost in the ordinary case: for an archive that
            // does not hold the path it replaces a `read_file` that would fail
            // late with a hash lookup, and a chain miss walks every archive.
            match mounted.archive.find_file(&normalised) {
                Ok(Some(info)) if info.file_size > MAX_FILE_BYTES => {
                    refused = Some((mounted.name.clone(), info.file_size));
                    continue;
                }
                Ok(Some(_)) => {}
                // Not in this archive, or its tables will not answer for it.
                _ => continue,
            }
            if let Ok(data) = mounted.archive.read_file(&normalised) {
                return Ok(data);
            }
        }
        if let Some((archive, size)) = refused {
            self.note_damaged(archive, normalised.clone(), size);
        }
        Err(AssetError::NotFound(normalised))
    }

    /// How many damaged entries [`Self::damaged`] keeps.
    pub const DAMAGED_KEPT: usize = 16;

    /// Record one, once per path.
    fn note_damaged(&mut self, archive: String, path: String, size: u64) {
        if self.damaged.iter().any(|(_, had, _)| *had == path) {
            return;
        }
        if self.damaged.len() < Self::DAMAGED_KEPT {
            self.damaged.push((archive, path, size));
        }
    }

    /// Whether a file exists anywhere in the chain.
    ///
    /// A lookup, not a read. `self.read(vpath).is_ok()` would decompress the
    /// whole file, and `vale catalogue` calls this for six thousand paths in
    /// a row.
    pub fn exists(&mut self, vpath: &str) -> bool {
        let normalised = vpath.replace('/', "\\");
        if let Some(overlay) = &self.overlay {
            if overlay(&normalised).is_some() {
                return true;
            }
        }
        if let Some(root) = &self.loose {
            if normalised.len() > LOOSE_PREFIX.len()
                && normalised[..LOOSE_PREFIX.len()].eq_ignore_ascii_case(LOOSE_PREFIX)
                && resolve_loose(root, &normalised).is_some_and(|path| path.is_file())
            {
                return true;
            }
        }
        // Same rule as [`Self::read`], including the damaged-entry bound: a path
        // this chain would refuse to read counts as absent.
        self.archives.iter().any(|mounted| {
            matches!(
                mounted.archive.find_file(&normalised),
                Ok(Some(ref info)) if info.file_size <= MAX_FILE_BYTES
            )
        })
    }

    /// Build (once) a map of every listed file to the index of the archive that
    /// wins for it. Used for prefix queries like "which maps exist".
    fn listing(&mut self) -> &HashMap<String, usize> {
        if self.listing.is_none() {
            let mut map: HashMap<String, usize> = HashMap::new();
            for (idx, mounted) in self.archives.iter_mut().enumerate() {
                let Ok(entries) = mounted.archive.list() else {
                    continue;
                };
                for entry in entries {
                    let key = entry.name.replace('/', "\\").to_ascii_lowercase();
                    // First writer wins: archives are already in priority order.
                    map.entry(key).or_insert(idx);
                }
            }
            self.listing = Some(map);
        }
        self.listing.as_ref().unwrap()
    }

    /// Every known file path starting with `prefix` (case-insensitive).
    pub fn list_prefix(&mut self, prefix: &str) -> Vec<String> {
        let needle = prefix.replace('/', "\\").to_ascii_lowercase();
        let mut out: Vec<String> = self
            .listing()
            .keys()
            .filter(|k| k.starts_with(&needle))
            .cloned()
            .collect();
        out.sort();
        out
    }
}

/// A pool of open chains that are lent out and returned, so that a short task
/// on the task pool does not open the whole archive chain to read a file or
/// two.
///
/// Opening a chain reads seventeen archives in a 1.12 install, and for each one
/// the hash and block tables and the `(attributes)` file, which `wow_mpq` reads
/// and parses unconditionally and nothing in this workspace uses. Measured on a
/// 1.12 install with a warm page cache:
///
/// ```text
/// full chain open, 17 archives   45.1 ms
///   of which the headers alone    1.3 ms
///   of which (attributes)        10.4 ms   (2.3 MB, 97,457 entries, unread)
/// ```
///
/// Reading and parsing the ADT such a chain is usually opened for takes about
/// 27 ms. A tile task that opened its own chain therefore spent more than half
/// its time opening it, and discarded it after a few reads. A chain's contents
/// do not change while the process runs, so chains are kept.
///
/// The lock is held for the hand-over, not for the read; that distinguishes
/// this from sharing one chain behind a mutex. A borrower has a chain to itself
/// for as long as it holds the guard, so tile loads run concurrently and
/// contend only on a `Vec::pop`.
///
/// `(attributes)` is still read, once per chain rather than once per tile:
/// `wow_mpq` 0.7.0 offers no way to skip it. `Archive::open` calls
/// `load_tables`, which calls `load_attributes` unconditionally, and opening
/// with `OpenOptions::load_tables(false)` only defers the whole lot, because
/// the first read calls `load_tables` anyway.
#[derive(Default)]
pub struct ChainPool {
    idle: Mutex<Idle>,
}

/// The free list, and which directory it is of.
#[derive(Default)]
struct Idle {
    dir: PathBuf,
    /// Bumped whenever `dir` changes, so a chain borrowed before the change is
    /// dropped rather than returned onto the new directory's list.
    generation: u64,
    chains: Vec<Assets>,
}

impl ChainPool {
    /// How many idle chains to keep open.
    ///
    /// Bevy caps its async-compute pool at four threads, so at most four chains
    /// are borrowed at once and a fifth would never be reused. Exceeding it is
    /// not an error: a borrower that finds the list empty opens its own chain,
    /// and the chain is dropped on return when the list is full. This bounds
    /// memory, not concurrency: about 4 MB of hash and block tables per chain
    /// for a 1.12 install's 97,457 files.
    pub const KEPT: usize = 4;

    /// Borrow a chain for `dir`, opening one only if none is idle.
    ///
    /// The guard puts it back when it drops. A different `dir` from the one the
    /// pool holds empties the list first, because those chains read another
    /// directory's archives.
    pub fn take(&self, dir: impl AsRef<Path>) -> Result<PooledChain<'_>, AssetError> {
        let dir = dir.as_ref();
        let (chain, generation) = {
            let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
            if idle.dir != dir {
                idle.dir = dir.to_path_buf();
                idle.generation = idle.generation.wrapping_add(1);
                idle.chains.clear();
            }
            (idle.chains.pop(), idle.generation)
        };
        let chain = match chain {
            Some(chain) => chain,
            None => Assets::open(dir)?,
        };
        Ok(PooledChain {
            chain: Some(chain),
            generation,
            pool: self,
        })
    }

    /// How many chains are open and idle. For a test and for a report; a
    /// borrowed chain is not counted.
    pub fn idle(&self) -> usize {
        self.idle.lock().unwrap_or_else(|e| e.into_inner()).chains.len()
    }

    /// Drop every idle chain. For a host that has finished with an install;
    /// borrowed chains go back to being dropped on return.
    pub fn clear(&self) {
        self.idle.lock().unwrap_or_else(|e| e.into_inner()).chains.clear();
    }
}

/// A chain borrowed from a [`ChainPool`], returned when it drops.
///
/// Derefs to the [`Assets`] it holds, so a borrower reads through it exactly as
/// it would through a chain of its own.
pub struct PooledChain<'a> {
    chain: Option<Assets>,
    /// Which directory generation this was taken under — see [`Idle`].
    generation: u64,
    pool: &'a ChainPool,
}

impl std::ops::Deref for PooledChain<'_> {
    type Target = Assets;

    fn deref(&self) -> &Assets {
        self.chain.as_ref().expect("held until drop")
    }
}

impl std::ops::DerefMut for PooledChain<'_> {
    fn deref_mut(&mut self) -> &mut Assets {
        self.chain.as_mut().expect("held until drop")
    }
}

impl Drop for PooledChain<'_> {
    /// Resets the chain to the state [`Assets::open`] would produce, so a
    /// borrowed chain behaves like a fresh one, then returns it to the pool.
    ///
    /// Clearing the overlay is the whole reset: [`Assets::set_overlay`] is the
    /// only `&mut self` method that changes what a path resolves to. A loose
    /// root can only be set by [`Assets::with_loose_root`], which consumes the
    /// chain and cannot be reached through the guard. The listing is a cache of
    /// the archives, not borrower state, and is kept to avoid rebuilding it per
    /// borrow.
    fn drop(&mut self) {
        let Some(mut chain) = self.chain.take() else {
            return;
        };
        chain.set_overlay(None);
        let mut idle = self.pool.idle.lock().unwrap_or_else(|e| e.into_inner());
        if idle.generation == self.generation && idle.chains.len() < ChainPool::KEPT {
            idle.chains.push(chain);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chain with no archives, for the pool's tests; the pool's behaviour
    /// does not depend on a chain's contents.
    fn bare_chain() -> Assets {
        Assets {
            archives: Vec::new(),
            failures: Vec::new(),
            damaged: Vec::new(),
            listing: None,
            loose: None,
            overlay: None,
        }
    }

    /// A pool already holding `n` bare chains for `dir`, so a test does not
    /// open a real archive to borrow one.
    fn seeded(dir: &str, n: usize) -> ChainPool {
        let pool = ChainPool::default();
        {
            let mut idle = pool.idle.lock().expect("fresh");
            idle.dir = PathBuf::from(dir);
            for _ in 0..n {
                idle.chains.push(bare_chain());
            }
        }
        pool
    }

    /// A borrowed chain returns to the pool on drop, so the second tile of a
    /// session reads through the first tile's chain instead of opening
    /// seventeen archives again.
    #[test]
    fn a_chain_goes_back_when_the_borrower_drops_it() {
        let pool = seeded("Data", 1);
        assert_eq!(pool.idle(), 1);
        {
            let _chain = pool.take("Data").expect("the idle one, not a new one");
            assert_eq!(pool.idle(), 0, "borrowed, so not available to anybody else");
        }
        assert_eq!(pool.idle(), 1, "and back again");
        // An empty pool with nowhere to open from fails rather than answering a
        // chain of no archives, which is what every borrower already handles.
        let empty = ChainPool::default();
        assert!(empty.take("no-such-directory").is_err());
    }

    /// An overlay is cleared when the chain is returned. A borrower sets one to
    /// read a host's edited bytes; the next borrower has not asked for it and
    /// must read the archives. A chain that kept a previous borrower's overlay
    /// would reproduce the stale-overlay bug this project has hit three times.
    #[test]
    fn a_borrowers_overlay_does_not_reach_the_next_borrower() {
        let pool = seeded("Data", 1);
        let path = r"World\Maps\Azeroth\Azeroth_32_48.adt";
        {
            let mut chain = pool.take("Data").expect("idle");
            chain.set_overlay(Some(std::sync::Arc::new(|_: &str| {
                Some(b"the edited tile".to_vec())
            })));
            assert_eq!(chain.read(path).unwrap(), b"the edited tile");
        }
        let mut chain = pool.take("Data").expect("the same one back");
        assert!(
            chain.read(path).is_err(),
            "the chain is the one `Assets::open` would have produced"
        );
    }

    /// The list is bounded: a burst wider than [`ChainPool::KEPT`] is served,
    /// and the surplus is dropped rather than held for the session.
    #[test]
    fn the_idle_list_is_capped() {
        let pool = seeded("Data", ChainPool::KEPT + 1);
        let borrowed: Vec<_> = (0..ChainPool::KEPT + 1)
            .map(|_| pool.take("Data").expect("idle"))
            .collect();
        assert_eq!(pool.idle(), 0);
        drop(borrowed);
        assert_eq!(pool.idle(), ChainPool::KEPT, "the surplus went");
    }

    /// A chain of another install's archives is not handed out. The pool
    /// empties when the directory changes, and a chain borrowed before the
    /// change is dropped on return rather than landing on the new list.
    #[test]
    fn changing_the_directory_empties_the_pool() {
        let pool = seeded("Data", 2);
        let borrowed = pool.take("Data").expect("idle");
        assert_eq!(pool.idle(), 1);
        // A request for another directory. It has no archives, so the open
        // fails, but the old directory's idle list is emptied regardless.
        assert!(pool.take("no-such-directory").is_err());
        assert_eq!(pool.idle(), 0, "the old directory's chains went with it");
        drop(borrowed);
        assert_eq!(pool.idle(), 0, "and the one in flight is not returned onto it");
    }

    /// An overlay answers before the archives, and a path it declines falls
    /// through to them.
    ///
    /// The overlay supplies a changed file before it has been written anywhere
    /// the archives can see. Every reader that opens its own chain must be
    /// given the overlay, or it returns the game's bytes for a path the host
    /// overrides. Three readers open their own chains, and each initially
    /// missed the overlay; the symptom each time was a changed file reading
    /// back unchanged.
    #[test]
    fn an_overlay_answers_before_the_archives() {
        let mut chain = Assets {
            archives: Vec::new(),
            failures: Vec::new(),
            damaged: Vec::new(),
            listing: None,
            loose: None,
            overlay: None,
        };
        let path = r"World\Maps\Azeroth\Azeroth_32_48.adt";
        assert!(chain.read(path).is_err(), "no archives and no overlay");

        chain.set_overlay(Some(std::sync::Arc::new(|asked: &str| {
            asked.eq_ignore_ascii_case(r"World\Maps\Azeroth\Azeroth_32_48.adt")
                .then(|| b"the edited tile".to_vec())
        })));
        assert_eq!(chain.read(path).unwrap(), b"the edited tile");
        // Forward slashes are normalised before the overlay sees the path, so a
        // caller may spell it either way.
        assert_eq!(chain.read("World/Maps/Azeroth/Azeroth_32_48.adt").unwrap(), b"the edited tile");
        // …and a path it does not carry still falls through, to nothing here.
        assert!(chain.read(r"World\Maps\Azeroth\Azeroth_00_00.adt").is_err());

        chain.set_overlay(None);
        assert!(chain.read(path).is_err(), "and removing it puts the chain back");
    }

    /// The size bound is above every file a 1.12 install holds.
    ///
    /// Measured over every archive of a real install (97,000 files across
    /// seventeen archives), the largest is 16,307,975 bytes. The assertion is
    /// against that number rather than against the archives, so it means the
    /// same thing on a machine with no `Data\` folder.
    #[test]
    fn the_size_bound_is_far_above_the_largest_file_the_game_ships() {
        const LARGEST_SHIPPED: u64 = 16_307_975;
        assert!(
            MAX_FILE_BYTES > LARGEST_SHIPPED * 8,
            "{MAX_FILE_BYTES} leaves no margin over {LARGEST_SHIPPED}"
        );
        // ...and it is small enough to refuse the entry that was reported: the
        // sector offsets `wow_mpq` warned about were in the same range.
        assert!(MAX_FILE_BYTES < 4_067_989_298);
    }

    /// A loose path resolves through the case the disk has, and `..` climbs.
    #[test]
    fn a_loose_path_resolves_without_regard_to_case() {
        let root = std::env::temp_dir().join("vale-archive-loose");
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("Interface").join("AddOns").join("pfUI").join("img");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Bar.tga"), [1, 2, 3]).unwrap();
        // Compared by contents: a case-insensitive file system answers the
        // exact join with the asked spelling, a case-sensitive one through the
        // directory walk, and both read the same bytes.
        let bytes = |vpath: &str| resolve_loose(&root, vpath).and_then(|p| std::fs::read(p).ok());
        assert_eq!(bytes(r"Interface\AddOns\PFUI\IMG\bar.tga"), Some(vec![1, 2, 3]));
        assert_eq!(bytes(r"Interface\AddOns\pfUI\img\..\img\Bar.tga"), Some(vec![1, 2, 3]));
        assert_eq!(resolve_loose(&root, r"Interface\AddOns\pfUI\img\nope.tga"), None);
        assert_eq!(resolve_loose(&root, r"Interface\AddOns\pfUI\img"), None, "a directory is not a file");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn patch_letters_outrank_digits_and_base() {
        assert!(patch_priority("Patch-A.MPQ") > patch_priority("patch-2.MPQ"));
        assert!(patch_priority("patch-2.MPQ") > patch_priority("patch.MPQ"));
        assert!(patch_priority("patch.MPQ") > patch_priority("terrain.MPQ"));
        assert_eq!(patch_priority("terrain.MPQ"), 0);
    }

    #[test]
    fn later_patch_letters_win() {
        assert!(patch_priority("Patch-D.MPQ") > patch_priority("Patch-B.MPQ"));
    }

    #[test]
    fn case_does_not_matter() {
        assert_eq!(patch_priority("PATCH-A.MPQ"), patch_priority("patch-a.mpq"));
    }
}
