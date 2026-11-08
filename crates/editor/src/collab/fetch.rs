//! A joining editor's copy of the host's project.
//!
//! The copy is `Edit\<project> (shared)\`. The host's manifest names every
//! file of its project with its size and blake3 hash, except the files that
//! describe the host's own server ([`machine_local`]). A file the copy holds
//! with the same hash is kept; the other files are fetched; a file of the copy
//! the manifest does not name is removed, except a file that describes this
//! machine's own server. Each fetched file is checked against its hash before
//! it is written, so a host that saved while the editor was joining is caught
//! rather than opened half old.

use std::collections::HashMap;
use vale_client::assets::GameAssets;
use vale_collab::wire::FileEntry;
use vale_edit::project::Project;

/// A fetch in progress.
pub(super) struct Fetch {
    copy: Project,
    /// The files asked for, by path.
    wanted: HashMap<String, FileEntry>,
    /// The bytes of each file so far.
    partial: HashMap<String, Vec<u8>>,
    files: usize,
    bytes: u64,
    of_bytes: u64,
    /// Files that arrived with the wrong hash, or could not be written.
    bad: Vec<String>,
}

/// The name of the copy of a host's project.
pub(super) fn copy_name(project: &str) -> String {
    format!("{project} (shared)")
}

impl Fetch {
    /// Compare the manifest with the copy, remove what the host does not
    /// have, and answer the fetch and the paths to ask for.
    pub(super) fn start(assets: &GameAssets, project: &str, files: Vec<FileEntry>) -> Result<(Fetch, Vec<String>), String> {
        let name = copy_name(project);
        if !vale_edit::project::is_a_name(&name) {
            return Err(format!("{project} is not a project name this editor can keep"));
        }
        let copy = Project::open(&assets.root, &name).map_err(|e| e.to_string())?;
        let listed: HashMap<String, FileEntry> = files.into_iter().map(|file| (key(&file.path), file)).collect();
        for path in copy.files() {
            if !listed.contains_key(&key(&path)) && !machine_local(&path) {
                let _ = copy.revert(&path);
            }
        }
        let wanted: HashMap<String, FileEntry> = listed
            .into_values()
            .filter(|file| copy.read(&file.path).is_none_or(|bytes| *blake3::hash(&bytes).as_bytes() != file.hash))
            .map(|file| (file.path.clone(), file))
            .collect();
        let paths = wanted.keys().cloned().collect();
        let of_bytes = wanted.values().map(|file| file.size).sum();
        Ok((Fetch { copy, wanted, partial: HashMap::new(), files: 0, bytes: 0, of_bytes, bad: Vec::new() }, paths))
    }

    /// Where the fetch stands, for the panel.
    pub(super) fn state(&self) -> super::State {
        super::State::Fetching {
            project: self.copy.name.clone(),
            files: self.files,
            of_files: self.wanted.len(),
            bytes: self.bytes,
            of_bytes: self.of_bytes,
        }
    }

    /// One piece of a file. The last piece checks the hash and writes it.
    pub(super) fn piece(&mut self, path: &str, offset: u64, total: u64, bytes: Vec<u8>) {
        let Some(entry) = self.wanted.get(path) else { return };
        let had = self.partial.entry(path.to_string()).or_default();
        if had.len() as u64 != offset || total != entry.size {
            self.bad.push(path.to_string());
            self.partial.remove(path);
            return;
        }
        self.bytes += bytes.len() as u64;
        had.extend_from_slice(&bytes);
        if (had.len() as u64) < total {
            return;
        }
        let whole = self.partial.remove(path).unwrap_or_default();
        self.files += 1;
        if *blake3::hash(&whole).as_bytes() != entry.hash || self.copy.write(path, &whole).is_err() {
            self.bad.push(path.to_string());
        }
    }

    /// The host has sent everything: the copy's name, or why it cannot be
    /// opened.
    pub(super) fn finish(self, missing: Vec<String>) -> Result<String, String> {
        let unfinished = self.wanted.len() - self.files.min(self.wanted.len());
        if !missing.is_empty() || !self.bad.is_empty() || unfinished > 0 {
            let first = missing.first().or(self.bad.first()).cloned().unwrap_or_else(|| "a file".to_string());
            return Err(format!(
                "could not join: {first} did not arrive whole, which happens when the host saves while an editor joins. Join again."
            ));
        }
        Ok(self.copy.name)
    }
}

/// Whether a project file describes the server of the machine it is on: the
/// SQL files and revert files under `sql\`, the server DBC files saved under
/// `server\dbc-before\` before an Apply replaced them, and the record of the
/// tiles regenerated into the `DataDir`. Each editor keeps its own. A joined
/// editor that applies to its own server writes these files into its copy,
/// and a join neither sends the host's nor removes the joined editor's.
pub(super) fn machine_local(path: &str) -> bool {
    let lower = path.replace('/', "\\").to_ascii_lowercase();
    let before = format!("{}\\", crate::server::dbcs::BEFORE_DIR.to_ascii_lowercase());
    vale_edit::manifest::Kind::of(path) == vale_edit::manifest::Kind::Sql
        || lower.starts_with(&before)
        || lower == crate::server::datadir::LIVE_RECORD.to_ascii_lowercase()
}

/// Whether a project file goes in the manifest: every file that is not
/// [`machine_local`].
pub(super) fn listed(path: &str) -> bool {
    !machine_local(path)
}

/// The manifest of a project as it stands on disk: every file [`listed`], its
/// size and its hash.
pub(super) fn manifest(project: &Project) -> Vec<FileEntry> {
    project
        .files()
        .into_iter()
        .filter(|path| listed(path))
        .filter_map(|path| {
            let bytes = project.read(&path)?;
            Some(FileEntry { size: bytes.len() as u64, hash: *blake3::hash(&bytes).as_bytes(), path })
        })
        .collect()
}

/// A path compared without regard to case or separator, as the archives
/// compare them.
fn key(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copy that matches a file keeps it, one that differs or lacks it asks
    /// for it, and a file the host does not have is removed; a fetched file
    /// with the wrong hash fails the join. The files that describe a
    /// machine's own server are neither listed nor removed.
    #[test]
    fn the_copy_fetches_only_what_differs() {
        let install = std::env::temp_dir().join(format!("vale-collab-fetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let host = Project::open(&install, "Host").unwrap();
        host.write("World\\a.adt", b"same").unwrap();
        host.write("World\\b.adt", b"new bytes").unwrap();
        host.write("sql\\creatures-revert.sql", b"host's database").unwrap();
        host.write("server\\rows.txt", b"rows").unwrap();
        let copy = Project::open(&install, &copy_name("Host")).unwrap();
        copy.write("World\\a.adt", b"same").unwrap();
        copy.write("World\\b.adt", b"old").unwrap();
        copy.write("World\\gone.adt", b"x").unwrap();
        copy.write("sql\\creatures-revert.sql", b"this machine's database").unwrap();
        copy.write("server\\dbc-before\\Spell.dbc", b"this machine's DataDir").unwrap();
        copy.write("server\\rows.txt", b"rows").unwrap();

        let listed: Vec<String> = manifest(&host).into_iter().map(|file| file.path).collect();
        assert!(!listed.contains(&"sql\\creatures-revert.sql".to_string()));
        assert!(listed.contains(&"server\\rows.txt".to_string()), "the project's rows are shared");

        let mut assets = GameAssets::new(install.join("Data").to_string_lossy().into_owned());
        assets.root = install.to_string_lossy().into_owned();
        let (mut fetch, wanted) = Fetch::start(&assets, "Host", manifest(&host)).unwrap();
        assert_eq!(wanted, vec!["World\\b.adt".to_string()]);
        assert!(!copy.has("World\\gone.adt"));
        assert_eq!(copy.read("sql\\creatures-revert.sql").as_deref(), Some(&b"this machine's database"[..]));
        assert!(copy.has("server\\dbc-before\\Spell.dbc"));
        fetch.piece("World\\b.adt", 0, 9, b"new ".to_vec());
        fetch.piece("World\\b.adt", 4, 9, b"bytes".to_vec());
        assert_eq!(fetch.finish(Vec::new()), Ok(copy_name("Host")));
        assert_eq!(copy.read("World\\b.adt").as_deref(), Some(&b"new bytes"[..]));

        copy.write("World\\b.adt", b"old").unwrap();
        let (mut fetch, _) = Fetch::start(&assets, "Host", manifest(&host)).unwrap();
        fetch.piece("World\\b.adt", 0, 9, b"tampered!".to_vec());
        assert!(fetch.finish(Vec::new()).is_err());
        let _ = std::fs::remove_dir_all(&install);
    }
}
