//! What the editor has done to a server's `DataDir`: which project built each
//! tile, every tile file it has written, the copy of each file as it was
//! before the first write, and the files a run no longer produces.
//!
//! ## Where it is kept, and why there
//!
//! `DataDir\vale-editor\tiles.txt` and `DataDir\vale-editor\originals\`. The
//! server reads only the files it names, so a folder of its own beside
//! `maps\`, `vmaps\` and `mmaps\` is not read. The ledger is kept in the
//! `DataDir` and not in a project because the `DataDir` is one folder that
//! every project writes into. A record kept in the project is lost when the
//! project is deleted, and does not show the tiles another project built.
//!
//! ## What a run may do without being asked
//!
//! A run writes and replaces tile files. It never removes one. A file a run no
//! longer produces is marked stale, with the reason, and stays where it is
//! until someone removes it ([`Ledger::remove`]) or puts the originals back
//! ([`Ledger::put_back`]). Both are decisions made in the editor's *Server tile
//! files* window, not side effects of a regeneration.
//!
//! ## The lines of `tiles.txt`
//!
//! ```text
//! tile    <map> <x> <y> <project> <vpath> <hash>   the ADT a tile's .map was built from
//! untile  <map> <x> <y>                            the tile is the archives' own again
//! file    <sub>\<name> <project> original|new      a file the editor has written, and
//!                                                  whether the file it replaced is kept
//!                                                  in originals\ or there was none
//! unfile  <sub>\<name>                             the file is as the editor found it again
//! stale   <sub>\<name> <why>                       a file no run produces any longer
//! unstale <sub>\<name>
//! ```
//!
//! Fields are separated by tabs. Lines are appended as each change is made,
//! so a run that stops part way leaves the ledger agreeing with the files.
//! Reading folds them in order, the last line about a file winning, and
//! [`Ledger::compact`] rewrites the file as one line per entry. `<hash>` is the
//! FNV-1a 64 of the ADT's bytes, in hex.
//!
//! A file written before the ledger existed has no line, so the next write
//! keeps it as an original although an earlier editor run may have written
//! it.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::Tile;

/// The ledger's folder under `DataDir`.
pub const FOLDER: &str = "vale-editor";

/// What a tile's `.map` was built from: the project whose run built it, the
/// ADT's path in that project, and the hash of the ADT's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub project: String,
    pub vpath: String,
    pub hash: u64,
}

/// A file the editor has written: the project whose run wrote it last, and
/// whether the file it first replaced is kept in `originals\`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub project: String,
    pub original: bool,
}

/// The ledger of one `DataDir`. Keys of [`Ledger::files`] and
/// [`Ledger::stale`] are `<sub>\<name>`: `maps\0004832.map`.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    data_dir: PathBuf,
    pub tiles: BTreeMap<Tile, Built>,
    pub files: BTreeMap<String, Written>,
    pub stale: BTreeMap<String, String>,
}

/// What [`Ledger::put_back`] did, file by file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PutBack {
    /// Files replaced by the copy kept in `originals\`.
    pub restored: Vec<String>,
    /// Files removed because there was none before the editor wrote them.
    pub removed: Vec<String>,
    /// Files left, each with why.
    pub left: Vec<(String, String)>,
}

/// The key of a file in `sub`: `maps\0004832.map`.
pub fn key(sub: &str, name: &str) -> String {
    format!("{sub}\\{name}")
}

/// `maps\0004832.map` as `("maps", "0004832.map")`.
fn split_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('\\')
}

impl Ledger {
    /// The ledger of `data_dir`, read from its `tiles.txt`. A missing file is
    /// an empty ledger: the editor has written nothing there yet.
    pub fn open(data_dir: &Path) -> Result<Ledger, String> {
        let mut out = Ledger {
            data_dir: data_dir.to_path_buf(),
            ..Ledger::default()
        };
        let path = out.text_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        for line in text.lines() {
            out.fold(line);
        }
        Ok(out)
    }

    fn text_path(&self) -> PathBuf {
        self.data_dir.join(FOLDER).join("tiles.txt")
    }

    /// Where the original of `key` is kept.
    pub fn original_path(&self, key: &str) -> Option<PathBuf> {
        let (sub, name) = split_key(key)?;
        Some(self.data_dir.join(FOLDER).join("originals").join(sub).join(name))
    }

    /// Where `key` is in the `DataDir`.
    pub fn file_path(&self, key: &str) -> Option<PathBuf> {
        let (sub, name) = split_key(key)?;
        Some(self.data_dir.join(sub).join(name))
    }

    /// Apply one line of `tiles.txt`. A line that does not parse is ignored.
    fn fold(&mut self, line: &str) {
        let fields: Vec<&str> = line.split('\t').collect();
        let tile = |m: &str, x: &str, y: &str| -> Option<Tile> {
            Some(Tile::new(m.parse().ok()?, x.parse().ok()?, y.parse().ok()?))
        };
        match fields.as_slice() {
            ["tile", m, x, y, project, vpath, hash] => {
                if let (Some(tile), Ok(hash)) = (tile(m, x, y), u64::from_str_radix(hash, 16)) {
                    self.tiles.insert(
                        tile,
                        Built {
                            project: project.to_string(),
                            vpath: vpath.to_string(),
                            hash,
                        },
                    );
                }
            }
            ["untile", m, x, y] => {
                if let Some(tile) = tile(m, x, y) {
                    self.tiles.remove(&tile);
                }
            }
            ["file", key, project, kept] => {
                self.files.insert(
                    key.to_string(),
                    Written {
                        project: project.to_string(),
                        original: *kept == "original",
                    },
                );
            }
            ["unfile", key] => {
                self.files.remove(*key);
            }
            ["stale", key, why] => {
                self.stale.insert(key.to_string(), why.to_string());
            }
            ["unstale", key] => {
                self.stale.remove(*key);
            }
            _ => {}
        }
    }

    /// Append one line to `tiles.txt` and apply it.
    fn log(&mut self, line: String) -> Result<(), String> {
        let path = self.text_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        writeln!(file, "{line}").map_err(|e| format!("{}: {e}", path.display()))?;
        self.fold(&line);
        Ok(())
    }

    /// Call before `project`'s run writes `<sub>\<name>`. The first time the
    /// editor writes the file, the file there is copied to `originals\`, or
    /// noted as absent. The file stops being stale, since a run produces it.
    pub fn before_write(&mut self, sub: &str, name: &str, project: &str) -> Result<(), String> {
        let key = key(sub, name);
        match self.files.get(&key) {
            Some(written) if written.project == project => {}
            Some(written) => {
                let kept = if written.original { "original" } else { "new" };
                self.log(format!("file\t{key}\t{project}\t{kept}"))?;
            }
            None => {
                let here = self.data_dir.join(sub).join(name);
                let kept = match here.is_file() {
                    true => {
                        let to = self.original_path(&key).expect("a key with a folder");
                        if let Some(parent) = to.parent() {
                            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                        }
                        std::fs::copy(&here, &to).map_err(|e| format!("{} -> {}: {e}", here.display(), to.display()))?;
                        "original"
                    }
                    false => "new",
                };
                self.log(format!("file\t{key}\t{project}\t{kept}"))?;
            }
        }
        if self.stale.contains_key(&key) {
            self.log(format!("unstale\t{key}"))?;
        }
        Ok(())
    }

    /// Mark `<sub>\<name>` as a file no run produces any longer, when it is
    /// there. Nothing is removed.
    pub fn mark_stale(&mut self, sub: &str, name: &str, why: &str) -> Result<(), String> {
        let key = key(sub, name);
        if !self.data_dir.join(sub).join(name).is_file() || self.stale.get(&key).is_some_and(|w| w == why) {
            return Ok(());
        }
        let why = why.replace(['\t', '\n', '\r'], " ");
        self.log(format!("stale\t{key}\t{why}"))
    }

    /// Note that `tile`'s `.map` is now built from `built`, or from the
    /// archives or nothing when `None`.
    pub fn note_tile(&mut self, tile: Tile, built: Option<Built>) -> Result<(), String> {
        let (m, x, y) = (tile.map, tile.x, tile.y);
        match built {
            Some(b) if self.tiles.get(&tile) != Some(&b) => {
                self.log(format!("tile\t{m}\t{x}\t{y}\t{}\t{}\t{:016x}", b.project, b.vpath, b.hash))
            }
            Some(_) => Ok(()),
            None if self.tiles.contains_key(&tile) => self.log(format!("untile\t{m}\t{x}\t{y}")),
            None => Ok(()),
        }
    }

    /// The stale files that are still there, with why.
    pub fn stale_files(&self) -> Vec<(String, String)> {
        self.stale
            .iter()
            .filter(|(key, _)| self.file_path(key).is_some_and(|p| p.is_file()))
            .map(|(key, why)| (key.clone(), why.clone()))
            .collect()
    }

    /// Remove the files named by `keys`, as asked in the editor. A file the
    /// editor has never written is the server's own: it is moved to
    /// `originals\` rather than deleted, so [`Ledger::put_back`] can restore
    /// it. A file the editor wrote is deleted; its original, if there was
    /// one, is already kept. Answers the keys removed.
    pub fn remove(&mut self, keys: &[String]) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for key in keys {
            let Some(here) = self.file_path(key) else {
                continue;
            };
            if !here.is_file() {
                if self.stale.contains_key(key) {
                    self.log(format!("unstale\t{key}"))?;
                }
                continue;
            }
            match self.files.get(key) {
                Some(_) => std::fs::remove_file(&here).map_err(|e| format!("{}: {e}", here.display()))?,
                None => {
                    let to = self.original_path(key).expect("a key with a folder");
                    if let Some(parent) = to.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                    }
                    if std::fs::rename(&here, &to).is_err() {
                        std::fs::copy(&here, &to).map_err(|e| format!("{}: {e}", to.display()))?;
                        std::fs::remove_file(&here).map_err(|e| format!("{}: {e}", here.display()))?;
                    }
                    self.log(format!("file\t{key}\t-\toriginal"))?;
                }
            }
            if self.stale.contains_key(key) {
                self.log(format!("unstale\t{key}"))?;
            }
            out.push(key.clone());
        }
        Ok(out)
    }

    /// The files [`Ledger::put_back`] would act on for `project`, as `(key,
    /// original)`: every file whose last write was that project's run.
    pub fn written_by(&self, project: &str) -> Vec<(String, bool)> {
        self.files
            .iter()
            .filter(|(_, w)| w.project == project)
            .map(|(key, w)| (key.clone(), w.original))
            .collect()
    }

    /// The projects that have written files here, each with how many.
    pub fn projects(&self) -> Vec<(String, usize)> {
        let mut out: BTreeMap<String, usize> = BTreeMap::new();
        for written in self.files.values() {
            if written.project != "-" {
                *out.entry(written.project.clone()).or_default() += 1;
            }
        }
        out.into_iter().collect()
    }

    /// Put the `DataDir` back as it was before `project`'s runs, as asked in
    /// the editor: each file that project wrote is replaced by its kept
    /// original, or removed when it had none, and the tiles it built are
    /// forgotten.
    ///
    /// A map's `.vmtree` and `.vmtile` files are one set (the tree indexes
    /// the tiles), so they are put back together or not at all. When another
    /// project's run wrote part of a map's set since, that map's vmap files
    /// are left, and named in [`PutBack::left`].
    pub fn put_back(&mut self, project: &str) -> Result<PutBack, String> {
        let mut out = PutBack::default();
        let set_of = |key: &str| -> Option<String> {
            let (sub, name) = split_key(key)?;
            let lower = name.to_ascii_lowercase();
            (sub == "vmaps" && (lower.ends_with(".vmtile") || lower.ends_with(".vmtree")))
                .then(|| name.get(0..3).unwrap_or("").to_string())
        };
        let mut mixed: Vec<String> = Vec::new();
        for (key, written) in &self.files {
            if let Some(map) = set_of(key) {
                if written.project != project && !mixed.contains(&map) {
                    mixed.push(map);
                }
            }
        }
        for (key, original) in self.written_by(project) {
            if set_of(&key).is_some_and(|map| mixed.contains(&map)) {
                out.left.push((
                    key,
                    "another project's run wrote part of this map's vmap files since".to_string(),
                ));
                continue;
            }
            let here = self.file_path(&key).expect("a key with a folder");
            match original {
                true => {
                    let kept = self.original_path(&key).expect("a key with a folder");
                    if !kept.is_file() {
                        out.left.push((key, "its original is missing from originals\\".to_string()));
                        continue;
                    }
                    let bytes = std::fs::read(&kept).map_err(|e| format!("{}: {e}", kept.display()))?;
                    if let Some(parent) = here.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                    }
                    vale_edit::project::write_replacing(&here, &bytes)
                        .map_err(|e| format!("{}: {e}", here.display()))?;
                    let _ = std::fs::remove_file(&kept);
                    out.restored.push(key.clone());
                }
                false => {
                    if here.is_file() {
                        std::fs::remove_file(&here).map_err(|e| format!("{}: {e}", here.display()))?;
                    }
                    out.removed.push(key.clone());
                }
            }
            self.log(format!("unfile\t{key}"))?;
            if self.stale.contains_key(&key) {
                self.log(format!("unstale\t{key}"))?;
            }
        }
        let built: Vec<Tile> = self
            .tiles
            .iter()
            .filter(|(_, b)| b.project == project)
            .map(|(tile, _)| *tile)
            .collect();
        for tile in built {
            self.note_tile(tile, None)?;
        }
        Ok(out)
    }

    /// Rewrite `tiles.txt` as one line per entry, dropping stale entries for
    /// files that are gone.
    pub fn compact(&mut self) -> Result<(), String> {
        let gone: Vec<String> = self
            .stale
            .keys()
            .filter(|key| !self.file_path(key).is_some_and(|p| p.is_file()))
            .cloned()
            .collect();
        for key in gone {
            self.stale.remove(&key);
        }
        let mut text = String::from(
            "# What the world editor has done to this DataDir. See vale_mangos::datadir::ledger.\n",
        );
        for (tile, b) in &self.tiles {
            text.push_str(&format!(
                "tile\t{}\t{}\t{}\t{}\t{}\t{:016x}\n",
                tile.map, tile.x, tile.y, b.project, b.vpath, b.hash
            ));
        }
        for (key, w) in &self.files {
            let kept = if w.original { "original" } else { "new" };
            text.push_str(&format!("file\t{key}\t{}\t{kept}\n", w.project));
        }
        for (key, why) in &self.stale {
            text.push_str(&format!("stale\t{key}\t{why}\n"));
        }
        let path = self.text_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        vale_edit::project::write_replacing(&path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vale-ledger-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["maps", "vmaps", "mmaps"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        dir
    }

    /// The first write keeps the file it replaces, a later one does not, and
    /// a file that was not there is noted as new. The ledger reads back the
    /// same after a compaction.
    #[test]
    fn the_first_write_keeps_the_original() {
        let dir = folder("first");
        std::fs::write(dir.join("maps").join("0004832.map"), b"shipped").unwrap();
        let mut ledger = Ledger::open(&dir).unwrap();
        ledger.before_write("maps", "0004832.map", "p").unwrap();
        std::fs::write(dir.join("maps").join("0004832.map"), b"edited").unwrap();
        ledger.before_write("maps", "0004832.map", "p").unwrap();
        ledger.before_write("mmaps", "0004832.mmtile", "p").unwrap();
        let kept = ledger.original_path("maps\\0004832.map").unwrap();
        assert_eq!(std::fs::read(kept).unwrap(), b"shipped");
        assert!(ledger.files["maps\\0004832.map"].original);
        assert!(!ledger.files["mmaps\\0004832.mmtile"].original);

        let read = Ledger::open(&dir).unwrap();
        assert_eq!(read.files, ledger.files);
        ledger.compact().unwrap();
        assert_eq!(Ledger::open(&dir).unwrap().files, ledger.files);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stale file stays until it is removed. Removing a file the editor
    /// never wrote moves it to originals\, and putting the project's files
    /// back restores originals and removes the files that were new.
    #[test]
    fn removal_and_put_back_are_asked_for() {
        let dir = folder("asked");
        let file = |sub: &str, name: &str| dir.join(sub).join(name);
        std::fs::write(file("maps", "0002238.map"), b"left").unwrap();
        std::fs::write(file("maps", "0004832.map"), b"shipped").unwrap();
        let mut ledger = Ledger::open(&dir).unwrap();
        ledger.mark_stale("maps", "0002238.map", "no tile").unwrap();
        ledger.mark_stale("maps", "0009999.map", "not there").unwrap();
        assert_eq!(ledger.stale_files().len(), 1, "a missing file is not stale");
        assert!(file("maps", "0002238.map").is_file(), "marking removes nothing");

        assert_eq!(ledger.remove(&["maps\\0002238.map".into()]).unwrap().len(), 1);
        assert!(!file("maps", "0002238.map").exists());
        assert!(ledger.original_path("maps\\0002238.map").unwrap().is_file(), "the server's own file is kept");
        assert!(ledger.stale_files().is_empty());

        ledger.before_write("maps", "0004832.map", "p").unwrap();
        std::fs::write(file("maps", "0004832.map"), b"edited").unwrap();
        ledger.before_write("mmaps", "0004832.mmtile", "p").unwrap();
        std::fs::write(file("mmaps", "0004832.mmtile"), b"new").unwrap();
        ledger
            .note_tile(Tile::new(0, 32, 48), Some(Built { project: "p".into(), vpath: "v".into(), hash: 1 }))
            .unwrap();
        let back = ledger.put_back("p").unwrap();
        assert_eq!(back.restored, vec!["maps\\0004832.map"]);
        assert_eq!(back.removed, vec!["mmaps\\0004832.mmtile"]);
        assert_eq!(std::fs::read(file("maps", "0004832.map")).unwrap(), b"shipped");
        assert!(!file("mmaps", "0004832.mmtile").exists());
        assert!(ledger.tiles.is_empty() && ledger.written_by("p").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A map's vmap files are put back together or not at all.
    #[test]
    fn a_map_s_vmap_set_is_put_back_whole() {
        let dir = folder("set");
        let mut ledger = Ledger::open(&dir).unwrap();
        ledger.before_write("vmaps", "000.vmtree", "p").unwrap();
        ledger.before_write("vmaps", "000_32_48.vmtile", "q").unwrap();
        ledger.before_write("vmaps", "001.vmtree", "p").unwrap();
        let back = ledger.put_back("p").unwrap();
        assert_eq!(back.removed, vec!["vmaps\\001.vmtree"]);
        assert_eq!(back.left.len(), 1);
        assert_eq!(back.left[0].0, "vmaps\\000.vmtree");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
