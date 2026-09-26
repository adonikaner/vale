//! Where an edit is written, and how it reaches a client.
//!
//! ## A project is a folder of loose files, keyed by virtual path
//!
//! `<root>\project\World\Maps\Azeroth\Azeroth_32_48.adt` is the edited tile
//! that shadows the one in the archives. Nothing is compressed, nothing is
//! packed, and a saved tile is a file on disk that can be deleted, copied,
//! diffed or reverted with the tools already on the machine.
//!
//! ## The folder is two folders and one file
//!
//! ```text
//! Edit\<name>\
//!   project\      every virtual path: the game data the client reads, and the
//!                 sql\ and server\ bookkeeping the server half keeps
//!   publish\      what a publish wrote: the patch folders and their records
//!   settings.txt  the editor's own switches for this project
//! ```
//!
//! Every virtual path lands under `project\` through [`Project::path_for`],
//! except one whose first component is `publish`, which lands under
//! `publish\`. A folder written before the split, with its virtual paths at
//! the root, is not read; there were none outside this repository's own tests.
//!
//! This is an invented mechanism and not the reference client's. 1.12 reads
//! world data out of `Data\*.MPQ` and from nowhere else; there is no loose-file
//! override for it, the way there is for `Interface\AddOns\`. What the reference
//! does have is the patch archive, which is what [`Project::publish`] writes: a
//! project becomes a `patch-<X>.MPQ` beside the others and the real client reads
//! it on its next launch. The loose folder is the edit loop, and the archive is
//! the result.
//!
//! ## Publishing does not reach the server
//!
//! A tile in a patch archive changes what a client draws and what it walks on
//! locally. The server keeps its own collision — vmangos reads `vmaps` and
//! `mmaps` built from the same ADTs — so ground raised here is ground the server
//! still thinks is at the old height, and a character standing on it is
//! corrected back down. Terrain edits are for looking at until the server's maps
//! are rebuilt from the same files.

use crate::adt::AdtFile;
use crate::{EditError, TileKey};
use std::path::{Path, PathBuf};

/// A folder of edited files.
#[derive(Debug, Clone)]
pub struct Project {
    /// The folder the virtual paths hang under.
    pub root: PathBuf,
    /// What to call it in an interface, which is the folder's own name.
    pub name: String,
}

/// Where a project folder goes when the caller does not say: `Edit\<name>` under
/// the install, beside `Data\` and `WTF\`.
pub const PROJECTS_DIR: &str = "Edit";

/// The folder under a project that holds its virtual paths — see the module
/// comment.
pub const FILES_DIR: &str = "project";

/// …and the one that holds what a publish wrote.
pub const PUBLISH_DIR: &str = "publish";

/// …and the editor's own switches for the project, a file beside the two.
/// Not a virtual path, so never packed into an archive.
pub const SETTINGS_FILE: &str = "settings.txt";

/// The project the editor last opened, as a file under `Edit\` holding
/// one name: `Edit\last-project.txt`. Read at startup when no project is
/// named on the command line; written on every open and switch.
pub const LAST_OPENED_FILE: &str = "last-project.txt";

/// The project last opened on this install, if its folder is still there.
pub fn last_opened(install: impl AsRef<Path>) -> Option<String> {
    let root = install.as_ref().join(PROJECTS_DIR);
    let text = std::fs::read_to_string(root.join(LAST_OPENED_FILE)).ok()?;
    let name = text.lines().next()?.trim().to_string();
    (is_a_name(&name) && root.join(&name).is_dir()).then_some(name)
}

/// …and write it. A failure is not an error: the next launch opens the
/// default.
pub fn remember_opened(install: impl AsRef<Path>, name: &str) {
    if !is_a_name(name) {
        return;
    }
    let root = install.as_ref().join(PROJECTS_DIR);
    let _ = std::fs::create_dir_all(&root);
    let _ = std::fs::write(root.join(LAST_OPENED_FILE), format!("{}\n", name.trim()));
}

/// Every project under an install folder, by name, sorted.
///
/// A folder under `Edit\` is a project — there is no manifest and nothing to
/// parse, which is the same decision as the loose files inside it: what is on
/// disk is the whole of the state. A folder that has just been made and holds
/// nothing is a project with no edits in it, which is exactly what a new one
/// is.
pub fn projects(install: impl AsRef<Path>) -> Vec<String> {
    let root = install.as_ref().join(PROJECTS_DIR);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort_by_key(|name| name.to_ascii_lowercase());
    found
}

/// What a project folder holds, counted by kind, for a list that has to say
/// which of several projects is which. The kinds are
/// [`crate::manifest::Kind`]'s; [`crate::manifest::manifest`] describes the
/// same files one by one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// `.adt` files: edited or created tiles.
    pub tiles: usize,
    /// `.dbc` files: edited client tables.
    pub tables: usize,
    /// `.m2` files: baked model copies.
    pub models: usize,
    /// `.blp` files under `textures\Minimap\`.
    pub minimaps: usize,
    /// Any other `.blp`.
    pub textures: usize,
    /// `.wdt` files.
    pub wdts: usize,
    /// Files under `sql\`.
    pub sql: usize,
    /// Files under `server\`.
    pub server: usize,
    /// Everything else.
    pub others: usize,
    /// When the newest file in it was written, or `None` for an empty folder.
    pub modified: Option<std::time::SystemTime>,
}

impl Summary {
    pub fn total(&self) -> usize {
        self.counts().iter().map(|(count, _)| count).sum()
    }

    /// Every count with its kind, in the order [`Self::line`] prints them.
    fn counts(&self) -> [(usize, crate::manifest::Kind); 9] {
        use crate::manifest::Kind;
        [
            (self.tiles, Kind::Tile),
            (self.tables, Kind::Table),
            (self.models, Kind::Model),
            (self.minimaps, Kind::Minimap),
            (self.textures, Kind::Texture),
            (self.wdts, Kind::Wdt),
            (self.sql, Kind::Sql),
            (self.server, Kind::Server),
            (self.others, Kind::Other),
        ]
    }

    /// `3 tiles, 1 table, 2 models, 1 WDT, 2 SQL files`, or `empty`.
    pub fn line(&self) -> String {
        let mut parts = Vec::new();
        for (count, kind) in self.counts() {
            let (one, many) = kind.words();
            match count {
                0 => {}
                1 => parts.push(format!("1 {one}")),
                n => parts.push(format!("{n} {many}")),
            }
        }
        match parts.is_empty() {
            true => "empty".to_string(),
            false => parts.join(", "),
        }
    }
}

/// Count what a project under an install folder holds. A project that is
/// not there, or cannot be read, is an empty one.
pub fn summary(install: impl AsRef<Path>, name: &str) -> Summary {
    summary_at(install.as_ref().join(PROJECTS_DIR).join(name), name)
}

/// …by folder rather than by name, for the two callers that already have one.
fn summary_at(root: impl AsRef<Path>, name: &str) -> Summary {
    let project = Project {
        root: root.as_ref().to_path_buf(),
        name: name.to_string(),
    };
    let mut out = Summary::default();
    for vpath in project.files() {
        use crate::manifest::Kind;
        match Kind::of(&vpath) {
            Kind::Tile => out.tiles += 1,
            Kind::Table => out.tables += 1,
            Kind::Model => out.models += 1,
            Kind::Minimap => out.minimaps += 1,
            Kind::Texture => out.textures += 1,
            Kind::Wdt => out.wdts += 1,
            Kind::Sql => out.sql += 1,
            Kind::Server => out.server += 1,
            Kind::Other => out.others += 1,
        }
        if let Some(at) = project.path_for(&vpath) {
            if let Ok(modified) = std::fs::metadata(at).and_then(|m| m.modified()) {
                out.modified = Some(out.modified.map_or(modified, |had| had.max(modified)));
            }
        }
    }
    out
}

/// The project every install has, and the one name [`delete`] refuses.
///
/// It is where the editor's edits go when nobody has said otherwise, so a
/// session that deleted it would be a session with nowhere to write. Its
/// files can go, see [`clear`], which reaches the same end and keeps the folder.
pub const DEFAULT: &str = "default";

/// Delete a project: the folder and everything under it.
///
/// Returns how many files went. Irreversible, and the caller is the one that
/// has to have asked: nothing here confirms anything.
///
/// Refused for [`DEFAULT`] and for a name that is not a project's — see
/// [`is_a_name`], which is what keeps this inside `Edit\`. Whether a
/// project is open is the caller's business: this crate has no session
/// and cannot know.
pub fn delete(install: impl AsRef<Path>, name: &str) -> Result<usize, EditError> {
    if name.trim() == DEFAULT {
        return Err(EditError::Refused(format!(
            "{DEFAULT} is where edits go when nobody has said otherwise, so it cannot be              deleted; its files can be cleared instead"
        )));
    }
    let root = folder(install, name)?;
    let went = summary_at(&root, name).total();
    match root.exists() {
        true => std::fs::remove_dir_all(&root)?,
        // Nothing there is not a failure: the end state is the one asked for.
        false => return Ok(0),
    }
    Ok(went)
}

/// Empty a project, keeping the folder.
///
/// [`DEFAULT`]'s stand-in for [`delete`], and it works for any project: what
/// a project is is the files under it, so one with none is a new one. The
/// folder is left there, so nothing that holds the path is left naming
/// nothing.
///
/// Returns how many files went.
pub fn clear(install: impl AsRef<Path>, name: &str) -> Result<usize, EditError> {
    let root = folder(install, name)?;
    let held = summary_at(&root, name).total();
    if !root.exists() {
        std::fs::create_dir_all(&root)?;
        return Ok(0);
    }
    // Every entry is attempted, and the first failure does not end the
    // walk. The loop used to be `?` on each removal, so one file the
    // filesystem would not remove (Windows refuses a `remove_dir_all` over
    // a tree holding anything open) stopped the clear where it stood and left
    // the rest of the project in place. The count then said how many files
    // there had been, and nothing said which were still there.
    let mut trouble: Option<std::io::Error> = None;
    for entry in std::fs::read_dir(&root)?.flatten() {
        let path = entry.path();
        // The editor's switches are not the project's files: what is being
        // thrown away is the edits, and a cleared project keeps its settings.
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case(SETTINGS_FILE) {
            continue;
        }
        let went = match path.is_dir() {
            true => std::fs::remove_dir_all(&path),
            false => std::fs::remove_file(&path),
        };
        if let Err(e) = went {
            trouble.get_or_insert(e);
        }
    }
    // …and the count is what is gone, measured afterwards, rather than what
    // was there before. A clear that removed half the project now says so.
    let left = summary_at(&root, name).total();
    if left > 0 {
        let why = match trouble {
            Some(e) => e.to_string(),
            None => "no reason the filesystem gave".to_string(),
        };
        return Err(EditError::Refused(format!(
            "{name}: {} of {held} file(s) thrown away, {left} could not be removed ({why}). \
             Something still has them open",
            held - left
        )));
    }
    Ok(held)
}

/// The folder a named project is in, refusing a name that is not one.
///
/// The one place either of the two above turns a name into a path, so the
/// check that it cannot climb out of `Edit\` is made once.
fn folder(install: impl AsRef<Path>, name: &str) -> Result<PathBuf, EditError> {
    if !is_a_name(name) {
        return Err(EditError::Refused(format!("{name:?} is not a project's name")));
    }
    Ok(install.as_ref().join(PROJECTS_DIR).join(name.trim()))
}

/// Is this a name a project may have?
///
/// It becomes a folder name, so the refusals are the filesystem's: nothing
/// empty, nothing with a separator or a drive letter in it, and none of the two
/// relative names. Checked before the folder is made rather than after, so a
/// name that would have escaped `Edit\` is refused with something to say rather
/// than creating a directory somewhere else.
pub fn is_a_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.len() <= 64
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        && !name.chars().any(|c| c.is_control())
}

impl Project {
    /// A project under an install folder, created if it is not there.
    pub fn open(install: impl AsRef<Path>, name: &str) -> Result<Project, EditError> {
        Self::at(install.as_ref().join(PROJECTS_DIR).join(name))
    }

    /// A project at an exact folder, created if it is not there.
    pub fn at(root: impl AsRef<Path>) -> Result<Project, EditError> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".into());
        Ok(Project { root, name })
    }

    /// Where a virtual path lands on disk: under `project\`, or under
    /// `publish\` for a path whose first component is `publish`.
    ///
    /// A path with `..` in it, or an absolute one, is refused: a project's files
    /// come from the archives' own namespace and nothing there climbs out of it.
    pub fn path_for(&self, vpath: &str) -> Option<PathBuf> {
        let mut parts = vpath.split(['\\', '/']).filter(|p| !p.is_empty()).peekable();
        let mut at = match parts.peek() {
            Some(first) if first.eq_ignore_ascii_case(PUBLISH_DIR) => {
                parts.next();
                self.publish_dir()
            }
            _ => self.root.join(FILES_DIR),
        };
        for part in parts {
            if part == ".." || part.contains(':') {
                return None;
            }
            at.push(part);
        }
        Some(at)
    }

    /// The folder a publish writes into: `<root>\publish`.
    pub fn publish_dir(&self) -> PathBuf {
        self.root.join(PUBLISH_DIR)
    }

    /// The editor's own switches for this project: `<root>\settings.txt`.
    pub fn settings_path(&self) -> PathBuf {
        self.root.join(SETTINGS_FILE)
    }

    /// The edited bytes of a virtual path, or `None` when this project does not
    /// carry it and the archives should answer instead.
    pub fn read(&self, vpath: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path_for(vpath)?).ok()
    }

    pub fn has(&self, vpath: &str) -> bool {
        self.path_for(vpath).is_some_and(|path| path.is_file())
    }

    /// Write bytes for a virtual path, creating the directories under the root.
    ///
    /// The file is either the old bytes or the new ones, never part of
    /// either. The bytes go to a temporary file beside it, which is then
    /// renamed over it. A plain write truncates first, and a failure after that
    /// leaves a short file: for a tile, a tile that will not parse; for a
    /// revert file, a database with rows applied and no record of how to put
    /// them back.
    pub fn write(&self, vpath: &str, bytes: &[u8]) -> Result<PathBuf, EditError> {
        let path = self
            .path_for(vpath)
            .ok_or_else(|| EditError::malformed("path", format!("{vpath} leaves the project")))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_replacing(&path, bytes)?;
        Ok(path)
    }

    /// Forget an edit, so the archives answer for it again.
    pub fn revert(&self, vpath: &str) -> Result<bool, EditError> {
        let Some(path) = self.path_for(vpath) else {
            return Ok(false);
        };
        if !path.is_file() {
            return Ok(false);
        }
        std::fs::remove_file(path)?;
        Ok(true)
    }

    /// Save a tile.
    pub fn save_tile(&self, key: &TileKey, tile: &AdtFile) -> Result<PathBuf, EditError> {
        self.write(&key.vpath(), &tile.write())
    }

    /// Load a tile this project has edited.
    pub fn load_tile(&self, key: &TileKey) -> Option<Result<AdtFile, EditError>> {
        self.read(&key.vpath()).map(|bytes| AdtFile::parse(&bytes))
    }

    /// The folders a project keeps that are not game data, and so are not
    /// packed into a patch archive.
    ///
    /// A project folder holds two different kinds of thing. Most of it is files
    /// the client reads, under the virtual paths the archives use —
    /// `World\\Maps\\…`, `DBFilesClient\\…`, `textures\\Minimap\\…` — and
    /// those are what [`Self::publish`] packs. The rest is the project's own
    /// bookkeeping: SQL for the server, and the record of which rows it
    /// changes. Neither is a path the client would ever ask for, and an archive
    /// carrying them ships a project's working notes to whoever opens it.
    ///
    /// Matched on the first path component, case-insensitively, so a file
    /// anywhere under one of these is excluded.
    pub const NOT_GAME_DATA: [&'static str; 3] = ["sql", "server", "publish"];

    /// Whether a virtual path is one [`Self::publish`] packs.
    pub fn is_game_data(vpath: &str) -> bool {
        let first = vpath
            .split(['/', '\\'])
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        !Self::NOT_GAME_DATA.iter().any(|skip| *skip == first)
    }

    /// Every virtual path this project carries, in a stable order.
    ///
    /// Everything, including the bookkeeping — this is what a clear counts
    /// and what a summary describes. [`Self::publish`] filters it through
    /// [`Self::is_game_data`].
    pub fn files(&self) -> Vec<String> {
        let mut found = Vec::new();
        let files = self.root.join(FILES_DIR);
        let mut stack = vec![files.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if is_temporary(&path) {
                    // A write that stopped before its rename. Not an edit, and
                    // never a file to publish.
                    continue;
                } else if let Ok(rest) = path.strip_prefix(&files) {
                    found.push(
                        rest.components()
                            .map(|c| c.as_os_str().to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join("\\"),
                    );
                }
            }
        }
        found.sort();
        found
    }

    /// Pack the project into a patch archive.
    ///
    /// Returns how many files went in. The archive is written whole rather than
    /// added to: the load order is decided by the file name and an archive that
    /// is rebuilt each time cannot accumulate a file the project no longer has.
    pub fn publish(&self, to: impl AsRef<Path>) -> Result<usize, EditError> {
        // The client's files and not the project's own bookkeeping — see
        // [`Self::NOT_GAME_DATA`]. A patch archive is read by the game, and
        // `sql\\world.sql` is not a path the game has ever asked for.
        let files: Vec<String> = self
            .files()
            .into_iter()
            .filter(|vpath| Self::is_game_data(vpath))
            .collect();
        if files.is_empty() {
            return Ok(0);
        }
        let mut builder = wow_mpq::ArchiveBuilder::new()
            // 1.12 reads version 1 archives. A later format opens in this
            // crate's own reader and not in the reference client's.
            .version(wow_mpq::FormatVersion::V1)
            .listfile_option(wow_mpq::ListfileOption::Generate);
        for vpath in &files {
            let Some(path) = self.path_for(vpath) else {
                continue;
            };
            let bytes = std::fs::read(&path)?;
            builder = builder.add_file_data(bytes, vpath);
        }
        let to = to.as_ref();
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        builder
            .build(to)
            .map_err(|e| EditError::Archive(e.to_string()))?;
        Ok(files.len())
    }

    /// Every patch archive this project has written into an install, as
    /// [`Self::publish_into`] recorded them. Empty for a project that has never
    /// published, and for one that published before the record existed.
    pub fn published(&self) -> Vec<Published> {
        let Some(bytes) = self.read(PUBLISHED_VPATH) else {
            return Vec::new();
        };
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
            .filter_map(Published::from_line)
            .collect()
    }

    fn record_published(&self, archives: &[Published]) -> Result<(), EditError> {
        if archives.is_empty() {
            if let Some(disk) = self.path_for(PUBLISHED_VPATH) {
                match std::fs::remove_file(disk) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
            }
            return Ok(());
        }
        let mut body = format!(
            "# {} — the patch archives this project wrote into an install.\n\
             # One a line: file name, size in bytes, FNV-1a 64 of its bytes. A\n\
             # publish replaces or removes only an archive whose size and hash\n\
             # still match, so a file somebody else has put under the same name is\n\
             # left alone.\n",
            self.name
        );
        for archive in archives {
            body.push_str(&archive.to_line());
            body.push('\n');
        }
        self.write(PUBLISHED_VPATH, body.as_bytes())?;
        Ok(())
    }

    /// Publish into an install's `Data\`, replacing what this project
    /// published there before.
    ///
    /// A project publishes one archive. It is rebuilt whole each time, under
    /// the letter it already has while that is still the highest in the
    /// folder; otherwise under the next letter, and the older one is removed.
    /// So a file the project no longer carries is not left behind in an
    /// archive the client still reads, and a project does not use up a letter
    /// per publish.
    ///
    /// Only an archive the record names and whose bytes still match it is
    /// replaced or removed. One that cannot be — the game has it open, or the
    /// file has changed — is reported in [`PublishReport::left`] and stays in
    /// the record, so the next publish tries again.
    ///
    /// A project with no game data publishes nothing, and removes what it
    /// published before: its archive would describe edits it no longer has.
    pub fn publish_into(&self, data_dir: impl AsRef<Path>) -> Result<PublishReport, EditError> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir)?;
        let mine: Vec<Published> = self
            .published()
            .into_iter()
            .filter(|archive| archive.matches(data_dir))
            .collect();
        let files = self.files().into_iter().filter(|vpath| Self::is_game_data(vpath)).count();
        let mut report = PublishReport::default();
        let mut keep: Vec<Published> = Vec::new();
        let mut target: Option<Published> = None;
        if files > 0 {
            let temporary = data_dir.join(format!(".publish.{}{TEMPORARY}", std::process::id()));
            let built = self.publish(&temporary);
            let written = built.and_then(|count| {
                let bytes = std::fs::read(&temporary)?;
                Ok((count, bytes.len() as u64, fnv1a(&bytes)))
            });
            let (count, size, hash) = match written {
                Ok(done) => done,
                Err(e) => {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(e);
                }
            };
            // The letter this project already holds, when nothing outranks it.
            let highest = highest_lettered_patch(data_dir);
            let own_highest = highest
                .as_ref()
                .filter(|name| mine.iter().any(|archive| archive.name.eq_ignore_ascii_case(name)));
            let mut placed: Option<String> = None;
            if let Some(name) = own_highest {
                if std::fs::rename(&temporary, data_dir.join(name)).is_ok() {
                    placed = Some(name.clone());
                }
            }
            if placed.is_none() {
                let Some(name) = next_patch_name(data_dir) else {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(EditError::Archive(
                        "every patch letter up to Z is taken in this Data folder".into(),
                    ));
                };
                if let Err(e) = std::fs::rename(&temporary, data_dir.join(&name)) {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(e.into());
                }
                placed = Some(name);
            }
            let name = placed.unwrap_or_default();
            report.to = Some(data_dir.join(&name));
            report.files = count;
            target = Some(Published { name, bytes: size, hash });
        }
        for archive in mine {
            if target.as_ref().is_some_and(|t| t.name.eq_ignore_ascii_case(&archive.name)) {
                continue;
            }
            match std::fs::remove_file(data_dir.join(&archive.name)) {
                Ok(()) => report.removed.push(archive.name.clone()),
                Err(e) => {
                    report.left.push((archive.name.clone(), e.to_string()));
                    keep.push(archive);
                }
            }
        }
        keep.extend(target);
        self.record_published(&keep)?;
        Ok(report)
    }
}

/// Where a project records the archives it published — see
/// [`Project::published`]. Under `publish\`, which is not game data.
pub const PUBLISHED_VPATH: &str = "publish\\archives.txt";

/// One patch archive a project wrote: its file name in `Data\`, and what
/// its bytes were, so a later publish can tell it from a file somebody else
/// put under the same name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub name: String,
    pub bytes: u64,
    pub hash: u64,
}

impl Published {
    fn to_line(&self) -> String {
        format!("{}\t{}\t{:016x}", self.name, self.bytes, self.hash)
    }

    fn from_line(line: &str) -> Option<Published> {
        let mut fields = line.split('\t');
        let name = fields.next()?.trim().to_string();
        let bytes = fields.next()?.trim().parse().ok()?;
        let hash = u64::from_str_radix(fields.next()?.trim(), 16).ok()?;
        // A name, never a path: the record may not point outside `Data\`.
        if name.is_empty() || name.contains(['\\', '/', ':']) || name.contains("..") {
            return None;
        }
        Some(Published { name, bytes, hash })
    }

    /// Whether the file in `data_dir` is still the one this record describes.
    fn matches(&self, data_dir: &Path) -> bool {
        let path = data_dir.join(&self.name);
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() == self.bytes => {
                std::fs::read(&path).is_ok_and(|bytes| fnv1a(&bytes) == self.hash)
            }
            _ => false,
        }
    }
}

/// What [`Project::publish_into`] did.
#[derive(Debug, Default)]
pub struct PublishReport {
    /// The archive written, or `None` when the project had no game data.
    pub to: Option<PathBuf>,
    /// How many files went into it.
    pub files: usize,
    /// Archives this project published before and has now removed.
    pub removed: Vec<String>,
    /// …and ones it could not remove, with why. Still in the record.
    pub left: Vec<(String, String)>,
}

/// FNV-1a, 64-bit: a stable hash with no dependency, for telling one file's
/// bytes from another's. Not a security measure.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The lettered patch archive that wins in `data_dir` — `Patch-D.MPQ` in an
/// install holding `Patch-A` and `Patch-D` — by its name as it is on disk.
fn highest_lettered_patch(data_dir: &Path) -> Option<String> {
    let mut highest: Option<(char, String)> = None;
    for entry in std::fs::read_dir(data_dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_ascii_lowercase();
        let Some(suffix) = lower.strip_suffix(".mpq").and_then(|stem| stem.strip_prefix("patch-"))
        else {
            continue;
        };
        let mut chars = suffix.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_alphabetic() && highest.as_ref().is_none_or(|(had, _)| c > *had) {
                highest = Some((c, name));
            }
        }
    }
    highest.map(|(_, name)| name)
}

/// What a temporary file written by [`write_replacing`] ends in.
const TEMPORARY: &str = ".vale-tmp";

/// Whether a file is one of those.
fn is_temporary(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(TEMPORARY))
}

/// Replace a file's contents in one step: write a temporary file beside it
/// and rename it over the original. The rename replaces an existing file on
/// Windows as well as elsewhere, so a reader sees the old bytes or the new ones.
///
/// The temporary file is removed when any step fails, so a failed write leaves
/// the original as it was and nothing beside it.
pub fn write_replacing(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let Some(name) = path.file_name() else {
        return std::fs::write(path, bytes);
    };
    let temporary = path.with_file_name(format!(
        ".{}.{}{TEMPORARY}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let written = (|| {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// The name of the first patch archive that would outrank every one already in
/// `data_dir`.
///
/// The client's load order is `base < patch.MPQ < patch-2..9 < patch-A..Z`, so
/// the archive that wins is the one with the latest suffix. An install with
/// `Patch-D.MPQ` in it therefore wants `Patch-E.MPQ` and not `Patch-9.MPQ`,
/// which would be shadowed by four archives and read as an edit that did
/// nothing.
pub fn next_patch_name(data_dir: impl AsRef<Path>) -> Option<String> {
    let mut highest: Option<char> = None;
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return Some("Patch-A.MPQ".into());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let Some(stem) = name.strip_suffix(".mpq") else {
            continue;
        };
        let Some(suffix) = stem.strip_prefix("patch-") else {
            continue;
        };
        let mut chars = suffix.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_alphabetic() => {
                let c = c.to_ascii_uppercase();
                if highest.is_none_or(|had| c > had) {
                    highest = Some(c);
                }
            }
            _ => {}
        }
    }
    match highest {
        None => Some("Patch-A.MPQ".into()),
        Some('Z') => None,
        Some(c) => Some(format!("Patch-{}.MPQ", (c as u8 + 1) as char)),
    }
}

#[cfg(test)]
mod tests {

    /// The folders under `Edit\` are the list, and a name that would not
    /// be one is refused before anything is created.
    #[test]
    fn projects_are_the_folders_and_nothing_else() {
        let install = std::env::temp_dir().join(format!("vale-projects-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        assert!(projects(&install).is_empty(), "an install with no Edit folder");

        Project::open(&install, "default").expect("default");
        Project::open(&install, "goldshire").expect("goldshire");
        // A loose file beside them is not a project.
        std::fs::write(install.join(PROJECTS_DIR).join("notes.txt"), b"x").unwrap();

        assert_eq!(projects(&install), vec!["default", "goldshire"]);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A project's summary counts its files by kind, and a project that is
    /// not there is an empty one.
    #[test]
    fn a_summary_counts_a_projects_files_by_kind() {
        let install = std::env::temp_dir().join(format!("vale-summary-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        assert_eq!(summary(&install, "nobody"), Summary::default());
        assert_eq!(summary(&install, "nobody").line(), "empty");

        let project = Project::open(&install, "goldshire").unwrap();
        project.write(r"World\Maps\Azeroth\Azeroth_32_48.adt", b"t").unwrap();
        project.write(r"World\Maps\Azeroth\Azeroth_32_49.adt", b"t").unwrap();
        project.write(r"DBFilesClient\Spell.dbc", b"s").unwrap();
        project.write(r"Custom\Thing_pos.m2", b"m").unwrap();
        project.write(r"World\Maps\Azeroth\Azeroth.wdt", b"w").unwrap();
        project.write(r"sql\creatures.sql", b"--").unwrap();
        project.write(r"sql\creatures-revert.sql", b"--").unwrap();
        project.write(r"server\rows.txt", b"").unwrap();
        project.write(r"notes.txt", b"").unwrap();

        let counted = summary(&install, "goldshire");
        assert_eq!((counted.tiles, counted.tables, counted.models, counted.wdts), (2, 1, 1, 1));
        assert_eq!((counted.sql, counted.server, counted.others), (2, 1, 1));
        assert_eq!(counted.total(), 9);
        assert!(counted.modified.is_some());
        assert_eq!(
            counted.line(),
            "2 tiles, 1 table, 1 model, 1 WDT, 2 SQL files, 1 server record, 1 other file"
        );
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A project is deleted folder and all, and the default never is.
    ///
    /// The one name that cannot go is where edits land when nobody has said
    /// otherwise: deleting it would leave the session with nowhere to write.
    #[test]
    fn a_project_is_deleted_folder_and_all() {
        let install = std::env::temp_dir().join(format!("vale-delete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let goldshire = Project::open(&install, "goldshire").unwrap();
        goldshire.write(r"World\Maps\Azeroth\Azeroth_32_48.adt", b"t").unwrap();
        goldshire.write(r"DBFilesClient\Spell.dbc", b"s").unwrap();
        Project::open(&install, DEFAULT).unwrap();

        assert_eq!(delete(&install, "goldshire").unwrap(), 2, "both its files");
        assert!(!goldshire.root.exists(), "and the folder with them");
        assert_eq!(projects(&install), vec![DEFAULT]);
        assert_eq!(delete(&install, "goldshire").unwrap(), 0, "deleting twice is not a failure");

        assert!(delete(&install, DEFAULT).is_err(), "the default cannot go");
        assert!(delete(&install, "../../Data").is_err(), "nor can anything outside Edit\\");
        assert!(delete(&install, "  ").is_err());
        assert_eq!(projects(&install), vec![DEFAULT], "and none of those touched it");
        let _ = std::fs::remove_dir_all(&install);
    }

    /// …and clearing takes the files and leaves the project, which is what
    /// the default gets instead.
    #[test]
    fn clearing_empties_a_project_and_keeps_it() {
        let install = std::env::temp_dir().join(format!("vale-clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = Project::open(&install, DEFAULT).unwrap();
        project.write(r"World\Maps\Azeroth\Azeroth_32_48.adt", b"t").unwrap();
        project.write(r"Custom\Thing_pos.m2", b"m").unwrap();

        assert_eq!(clear(&install, DEFAULT).unwrap(), 2);
        assert!(project.root.exists(), "the folder stays");
        assert!(project.files().is_empty(), "and is empty");
        assert_eq!(summary(&install, DEFAULT).line(), "empty");
        assert_eq!(projects(&install), vec![DEFAULT]);
        // …including the folders the virtual paths made, not just the files.
        assert!(!project.path_for("World").unwrap().exists());
        assert_eq!(clear(&install, DEFAULT).unwrap(), 0, "clearing twice is not a failure");
        let _ = std::fs::remove_dir_all(&install);
    }

    /// The last project opened is remembered by name, and forgotten when its
    /// folder is gone.
    #[test]
    fn the_last_opened_project_is_remembered_while_it_exists() {
        let install = std::env::temp_dir().join(format!("vale-last-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        assert_eq!(last_opened(&install), None);
        Project::open(&install, "goldshire").unwrap();
        remember_opened(&install, "goldshire");
        assert_eq!(last_opened(&install).as_deref(), Some("goldshire"));
        remember_opened(&install, "../escape");
        assert_eq!(last_opened(&install).as_deref(), Some("goldshire"), "a bad name is not written");
        delete(&install, "goldshire").unwrap();
        assert_eq!(last_opened(&install), None, "a project that is gone is not opened");
        let _ = std::fs::remove_dir_all(&install);
    }

    #[test]
    fn a_name_that_is_not_a_folder_name_is_refused() {
        for good in ["default", "goldshire", "my map 2", "a-b_c.1"] {
            assert!(is_a_name(good), "{good}");
        }
        // Every one of these would have put a folder somewhere else, or
        // nowhere.
        for bad in ["", "   ", ".", "..", "a/b", r"a\b", "C:", "a*b", "a?b"] {
            assert!(!is_a_name(bad), "{bad:?}");
        }
    }

    use super::*;

    /// A patch archive carries the client's files and not the project's own
    /// bookkeeping.
    ///
    /// `sql\` and `server\` are what this project writes for vmangos and its
    /// own record of which rows it changes. Neither is a path the game has ever
    /// asked for, and an archive carrying them ships a project's working notes
    /// to whoever opens it.
    #[test]
    fn publishing_leaves_the_projects_own_bookkeeping_out() {
        for vpath in [
            r"World\Maps\Azeroth\Azeroth_31_31.adt",
            r"DBFilesClient\Spell.dbc",
            r"textures\Minimap\8b89a1a8.blp",
        ] {
            assert!(Project::is_game_data(vpath), "{vpath}");
        }
        for vpath in [
            r"sql\world.sql",
            r"sql\creatures.sql",
            r"sql\creatures-revert.sql",
            r"server\rows.txt",
            "SQL/world.sql",
            "Server/rows.txt",
        ] {
            assert!(!Project::is_game_data(vpath), "{vpath}");
        }
    }

    /// Clearing a project takes the subfolders with it.
    ///
    /// A project is `World\Maps\<map>\*.adt`, `textures\Minimap\*.blp`,
    /// `DBFilesClient\*.dbc` and `sql\*.sql` — four levels of nesting and not
    /// one file at the root. A clear that walked only the root's files would
    /// report a count and leave the tiles and the minimap pictures where they
    /// were.
    #[test]
    fn clearing_takes_the_subfolders_with_it() {
        let dir = std::env::temp_dir().join("vale-clear-test");
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join(PROJECTS_DIR).join("p");
        for vpath in [
            "World/Maps/Azeroth/Azeroth_31_31.adt",
            "textures/Minimap/8b89a1a8.blp",
            "DBFilesClient/Spell.dbc",
            "sql/world.sql",
        ] {
            let at = root.join(FILES_DIR).join(vpath);
            std::fs::create_dir_all(at.parent().expect("a parent")).expect("a temp folder");
            std::fs::write(&at, b"x").expect("a temp file");
        }
        let went = clear(&dir, "p").expect("a project to clear");
        assert_eq!(went, 4, "every file is counted");
        let left: Vec<_> = std::fs::read_dir(&root).expect("the folder stays").flatten().collect();
        assert!(left.is_empty(), "nothing is left: {:?}", left.iter().map(|e| e.path()).collect::<Vec<_>>());
        let _ = std::fs::remove_dir_all(&dir);
    }


    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join("vale-edit-tests").join(name);
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    /// A virtual path becomes a folder under the root, and one that climbs out
    /// of it is refused.
    #[test]
    fn a_virtual_path_lands_under_the_root() {
        let project = Project::at(scratch("paths")).unwrap();
        let at = project
            .path_for(r"World\Maps\Azeroth\Azeroth_32_48.adt")
            .unwrap();
        assert!(at.ends_with(
            std::path::Path::new("World")
                .join("Maps")
                .join("Azeroth")
                .join("Azeroth_32_48.adt")
        ));
        assert_eq!(project.path_for(r"..\..\Data\base.MPQ"), None);
        assert_eq!(project.path_for(r"C:\Windows\notepad.exe"), None);
    }

    /// What is written comes back, is listed, and can be reverted.
    #[test]
    fn a_file_is_written_read_listed_and_reverted() {
        let project = Project::at(scratch("files")).unwrap();
        let vpath = r"World\Maps\Azeroth\Azeroth_32_48.adt";
        assert!(!project.has(vpath));
        project.write(vpath, b"tile").unwrap();
        assert!(project.has(vpath));
        assert_eq!(project.read(vpath).unwrap(), b"tile");
        assert_eq!(project.files(), vec![vpath.to_string()]);
        assert!(project.revert(vpath).unwrap());
        assert!(!project.has(vpath));
        assert!(!project.revert(vpath).unwrap(), "reverting twice is not an error");
    }

    /// The next patch letter outranks the ones already there.
    #[test]
    fn the_next_patch_outranks_the_archives_present() {
        let data = scratch("data");
        std::fs::create_dir_all(&data).unwrap();
        assert_eq!(next_patch_name(&data).as_deref(), Some("Patch-A.MPQ"));
        for name in ["base.MPQ", "patch.MPQ", "patch-2.MPQ", "Patch-D.MPQ"] {
            std::fs::write(data.join(name), b"").unwrap();
        }
        assert_eq!(next_patch_name(&data).as_deref(), Some("Patch-E.MPQ"));
    }

    /// A project with files in it packs into an archive those files read back
    /// out of.
    #[test]
    fn a_project_publishes_an_archive_its_files_read_out_of() {
        let project = Project::at(scratch("publish")).unwrap();
        let vpath = r"World\Maps\Azeroth\Azeroth_32_48.adt";
        project.write(vpath, b"the edited tile").unwrap();
        let archive = project.root.join("..").join("published.MPQ");
        assert_eq!(project.publish(&archive).unwrap(), 1);

        let mut opened = wow_mpq::Archive::open(&archive).expect("the archive opens");
        assert_eq!(opened.read_file(vpath).unwrap(), b"the edited tile");
    }

    /// A publish replaces the project's own archive rather than adding a
    /// letter, and a file taken out of the project is gone from what the
    /// client reads.
    #[test]
    fn a_second_publish_replaces_the_first() {
        let install = scratch("republish");
        let data = install.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Patch-D.MPQ"), b"somebody else's").unwrap();
        let project = Project::at(install.join("Edit").join("p")).unwrap();
        let first = r"World\Maps\Azeroth\Azeroth_32_48.adt";
        let second = r"World\Maps\Azeroth\Azeroth_32_49.adt";
        project.write(first, b"one").unwrap();
        project.write(second, b"two").unwrap();

        let report = project.publish_into(&data).unwrap();
        assert_eq!(report.to, Some(data.join("Patch-E.MPQ")));
        assert_eq!(report.files, 2);
        assert_eq!(project.published().len(), 1);

        project.revert(second).unwrap();
        let report = project.publish_into(&data).unwrap();
        assert_eq!(report.to, Some(data.join("Patch-E.MPQ")), "the same letter");
        assert!(!data.join("Patch-F.MPQ").exists());
        let mut opened = wow_mpq::Archive::open(data.join("Patch-E.MPQ")).unwrap();
        assert_eq!(opened.read_file(first).unwrap(), b"one");
        assert!(opened.read_file(second).is_err(), "the reverted tile is not served");
        // Nobody else's archive was touched, and the record is not game data.
        assert_eq!(std::fs::read(data.join("Patch-D.MPQ")).unwrap(), b"somebody else's");
        assert!(!Project::is_game_data(PUBLISHED_VPATH));
    }

    /// When another archive outranks the project's, the project moves above
    /// it and removes its own older one.
    #[test]
    fn an_outranked_archive_moves_up_and_the_old_one_goes() {
        let install = scratch("outranked");
        let data = install.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        let project = Project::at(install.join("Edit").join("p")).unwrap();
        project.write(r"World\Maps\Azeroth\Azeroth_32_48.adt", b"one").unwrap();
        project.publish_into(&data).unwrap();
        assert!(data.join("Patch-A.MPQ").exists());
        std::fs::write(data.join("Patch-B.MPQ"), b"a later patch").unwrap();

        let report = project.publish_into(&data).unwrap();
        assert_eq!(report.to, Some(data.join("Patch-C.MPQ")));
        assert_eq!(report.removed, vec!["Patch-A.MPQ".to_string()]);
        assert!(!data.join("Patch-A.MPQ").exists());
        assert!(data.join("Patch-B.MPQ").exists());
        let names: Vec<String> = project.published().into_iter().map(|a| a.name).collect();
        assert_eq!(names, vec!["Patch-C.MPQ".to_string()]);
    }

    /// An archive is replaced or removed only while it is still the bytes
    /// the project wrote. A file somebody put under the same name is left.
    #[test]
    fn a_changed_archive_is_not_the_projects_any_more() {
        let install = scratch("changed");
        let data = install.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        let project = Project::at(install.join("Edit").join("p")).unwrap();
        project.write(r"World\Maps\Azeroth\Azeroth_32_48.adt", b"one").unwrap();
        project.publish_into(&data).unwrap();
        std::fs::write(data.join("Patch-A.MPQ"), b"replaced by hand").unwrap();

        let report = project.publish_into(&data).unwrap();
        assert_eq!(report.to, Some(data.join("Patch-B.MPQ")));
        assert_eq!(std::fs::read(data.join("Patch-A.MPQ")).unwrap(), b"replaced by hand");
    }

    /// A write replaces the file whole and leaves nothing beside it, and a
    /// temporary file left by an interrupted write is never listed.
    #[test]
    fn a_write_replaces_the_file_and_a_leftover_is_not_an_edit() {
        let project = Project::at(scratch("atomic")).unwrap();
        let vpath = r"sql\creatures-revert.sql";
        project.write(vpath, b"a long first version").unwrap();
        project.write(vpath, b"short").unwrap();
        assert_eq!(project.read(vpath).unwrap(), b"short");
        assert_eq!(project.files(), vec![vpath.to_string()]);
        let leftover = project
            .path_for("sql")
            .unwrap()
            .join(".creatures-revert.sql.1.vale-tmp");
        std::fs::write(&leftover, b"half").unwrap();
        assert_eq!(project.files(), vec![vpath.to_string()]);
    }

    #[test]
    fn a_published_record_cannot_name_a_path() {
        assert!(Published::from_line("Patch-E.MPQ\t12\t00ff").is_some());
        for bad in ["..\\x.MPQ\t1\t0", "C:\\x.MPQ\t1\t0", "sub/x.MPQ\t1\t0", "\t1\t0"] {
            assert!(Published::from_line(bad).is_none(), "{bad:?}");
        }
    }
}
