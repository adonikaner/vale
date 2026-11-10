//! Saving tiles to the project folder, and packing the project into a patch
//! archive.

use bevy::prelude::*;
use super::EditSession;

impl EditSession {
    /// Write one tile to the project folder, or take it out of the folder
    /// when its bytes are the archives' own. Answers whether the folder now
    /// matches the tile; see [`Self::save_one`].
    pub(crate) fn save(&mut self, coord: (u32, u32)) -> bool {
        self.save_one(coord).is_some()
    }

    /// [`Self::save`], answering which of the two it did.
    ///
    /// A tile is marked unsaved by any edit, including one that is undone and
    /// a drag that ends where it began, so a save can hold exactly what the
    /// archives hold. Kept, such a file is listed as a tile the project
    /// changes, goes into every patch archive, and makes a server tile
    /// regeneration rebuild the tile. It is removed from the folder instead,
    /// and a file an earlier save wrote is removed with it, so a tile whose
    /// edits have all been taken back leaves the project.
    ///
    /// The comparison is of bytes, with the archives read past this project's
    /// own archives in `Data\`. `vale_edit::adt::diff`, which the project
    /// dialog lists tiles by, does not compare every chunk, so a tile it calls
    /// the same can still differ.
    fn save_one(&mut self, coord: (u32, u32)) -> Option<Saved> {
        // Every editor in a shared session holds the edits being saved, so
        // the write is not sent to them. See `crate::collab::files`.
        let _unjournaled = self.project.unjournaled();
        let tile = self.tiles.get(&coord)?;
        let key = self.key(coord);
        let vpath = key.vpath();
        // The folder holds submitted edits only; see `Self::exclude`.
        let bytes = match self.submitted_tile(&key, tile) {
            Some(submitted) => submitted.write(),
            None => tile.write(),
        };
        let unchanged = self.changes_nothing(&vpath, &bytes);
        let done = match unchanged {
            true => self.project.revert(&vpath).map(|_| {
                self.status = format!("{key} matches the archives, so it is not kept in the project");
                Saved::Dropped
            }),
            false => self.project.write(&vpath, &bytes).map(|path| {
                self.status = format!("saved {}", path.display());
                Saved::Written
            }),
        };
        match done {
            Ok(saved) => {
                self.unsaved.remove(&coord);
                // The project's files changed, so whatever listed them reads
                // them again: the Assets workspace and its survey.
                self.files_revision += 1;
                self.saved_tiles.push(key);
                Some(saved)
            }
            Err(e) => {
                self.status = format!("{key}: {e}");
                None
            }
        }
    }

    /// `tile` as the folder is to hold it: every excluded change on it taken
    /// back, newest first. `None` when no excluded change touches it. See
    /// [`Self::exclude`].
    pub(crate) fn submitted_tile(&self, key: &vale_edit::TileKey, tile: &vale_edit::adt::AdtFile) -> Option<vale_edit::adt::AdtFile> {
        let on: Vec<&super::Excluded> = self.exclude.iter().filter(|excluded| excluded.change.tiles().contains(key)).collect();
        if on.is_empty() {
            return None;
        }
        let mut copy = tile.clone();
        for excluded in on.iter().rev() {
            match excluded.forward {
                true => excluded.change.revert(key, &mut copy),
                false => excluded.change.apply(key, &mut copy),
            }
        }
        Some(copy)
    }

    /// Whether `bytes` are what the folder holds for `vpath`: the project's
    /// file, or the archives' bytes when the project has none.
    fn matches_folder(&self, vpath: &str, bytes: &[u8]) -> bool {
        match self.project.read(vpath) {
            Some(file) => super::originals::changes_nothing(vpath, bytes, &file),
            None => self.changes_nothing(vpath, bytes),
        }
    }

    /// After an undo or a redo: each of these tiles, tables and the server
    /// rows that is back to what the folder holds is not unsaved. A save of
    /// it would write the same bytes.
    pub(crate) fn unmark_matching(&mut self, tiles: &[(u32, u32)], tables: &[String], rows: bool) {
        for coord in tiles {
            let Some(tile) = self.tiles.get(coord) else { continue };
            if !self.unsaved.contains(coord) {
                continue;
            }
            let key = self.key(*coord);
            let bytes = self.submitted_tile(&key, tile).unwrap_or_else(|| tile.clone()).write();
            if self.matches_folder(&key.vpath(), &bytes) {
                self.unsaved.remove(coord);
            }
        }
        for name in tables {
            if !self.unsaved_tables.contains(name) {
                continue;
            }
            let Some(bytes) = self.submitted_table(name).map(|table| table.write()) else { continue };
            if self.matches_folder(&vale_assets::tables::dbc::dbc_path(name), &bytes) {
                self.unsaved_tables.remove(name);
            }
        }
        let excluded_rows = self.exclude.iter().any(|excluded| excluded.change.server_steps(true).next().is_some());
        if rows && self.rows.unsaved() && !excluded_rows && self.rows.matches_folder(&self.project) {
            self.rows.edits_unsaved = false;
            self.rows.paths_unsaved = false;
            self.rows.scripts_unsaved = false;
        }
    }

    /// Write `vpath` as the folder is to hold `parsed`, or take the file out
    /// when that is the archives' own bytes.
    pub(crate) fn write_submitted_tile(&mut self, key: &vale_edit::TileKey, parsed: &vale_edit::adt::AdtFile) -> Result<(), String> {
        let path = key.vpath();
        let bytes = match self.submitted_tile(key, parsed) {
            Some(submitted) => {
                self.elsewhere.insert(key.clone());
                submitted.write()
            }
            None => {
                self.elsewhere.remove(key);
                parsed.write()
            }
        };
        match self.changes_nothing(&path, &bytes) {
            true => self.project.revert(&path).map(|_| ()).map_err(|e| e.to_string()),
            false => self.project.write(&path, &bytes).map(|_| ()).map_err(|e| e.to_string()),
        }
    }

    /// Save every unsaved tile, and every tile of another map whose file
    /// leaves out an excluded change ([`Self::elsewhere`]). Answers how many
    /// tiles of the open map the folder now matches, written or removed.
    pub(crate) fn save_all(&mut self) -> usize {
        self.save_elsewhere();
        let unsaved: Vec<(u32, u32)> = self.unsaved.iter().copied().collect();
        let (mut written, mut dropped) = (0, 0);
        for coord in unsaved {
            match self.save_one(coord) {
                Some(Saved::Written) => written += 1,
                Some(Saved::Dropped) => dropped += 1,
                None => {}
            }
        }
        let root = self.project.root.display();
        match (written, dropped) {
            (0, 0) => {}
            (written, 0) => self.status = format!("saved {written} tiles to {root}"),
            (0, dropped) => {
                self.status =
                    format!("{dropped} tiles match the archives, so they are not kept in {root}")
            }
            (written, dropped) => {
                self.status = format!(
                    "saved {written} tiles to {root}; {dropped} match the archives, so they are \
                     not kept"
                )
            }
        }
        written + dropped
    }

    /// Save everything: the open map's tiles, the tiles of other maps whose
    /// files leave out an excluded change, the client tables and the server
    /// rows store, each with the other authors' unsubmitted changes left out
    /// ([`Self::exclude`]), and this editor's own as well while
    /// [`Self::exclude_once`] holds them. Every save is this one or
    /// `crate::server::save_everything`, which also writes the server's
    /// files, so a save leaves nothing of this editor's unsaved and submits
    /// its edits (`crate::collab`'s *What a save writes*). Answers why the
    /// rows store could not be written.
    pub(crate) fn save_files(&mut self) -> Result<(), String> {
        let mut failed = Ok(());
        self.save_files_with(|session| {
            if let Err(e) = session.rows.save(&session.project) {
                failed = Err(e.to_string());
            }
        });
        failed
    }

    /// [`Self::save_files`] with `rows` writing the server rows in place of
    /// the store alone: the server's half of a save, or a subject's SQL.
    /// `rows` runs while the excluded changes are taken back out of the
    /// store.
    pub(crate) fn save_files_with(&mut self, rows: impl FnOnce(&mut EditSession)) {
        let once = std::mem::take(&mut self.exclude_once);
        let own = once.is_empty();
        let kept = self.exclude.clone();
        if !own {
            self.exclude.extend(once);
            self.exclude.sort_by_key(|excluded| excluded.seq);
        }
        self.save_all();
        self.save_all_tables();
        let taken_back = self.take_back_excluded_rows();
        rows(self);
        if taken_back {
            self.put_back_excluded_rows();
        }
        self.exclude = kept;
        self.saved_everything = Some(own);
    }

    /// Take the excluded changes' server rows back out of the rows store, for
    /// a save to write the submitted rows. Answers whether any were.
    pub(crate) fn take_back_excluded_rows(&mut self) -> bool {
        let mut any = false;
        for excluded in self.exclude.iter().rev() {
            if excluded.change.server_steps(true).next().is_some() {
                self.rows.step(&excluded.change, !excluded.forward);
                any = true;
            }
        }
        any
    }

    /// Put the excluded changes' server rows back in after a save, leaving
    /// the store marked as the save left it: what is not in the folder now is
    /// other authors' and is not this editor's to save.
    pub(crate) fn put_back_excluded_rows(&mut self) {
        let unsaved = (self.rows.edits_unsaved, self.rows.paths_unsaved, self.rows.scripts_unsaved);
        for excluded in &self.exclude {
            if excluded.change.server_steps(true).next().is_some() {
                self.rows.step(&excluded.change, excluded.forward);
            }
        }
        (self.rows.edits_unsaved, self.rows.paths_unsaved, self.rows.scripts_unsaved) = unsaved;
    }

    /// Write the [`Self::elsewhere`] tiles again from their overlay bytes,
    /// leaving out the changes still excluded.
    fn save_elsewhere(&mut self) {
        let _unjournaled = self.project.unjournaled();
        let keys: Vec<vale_edit::TileKey> = self.elsewhere.iter().cloned().collect();
        for key in keys {
            let held = self.edited.read().ok().and_then(|edited| edited.get(&key.vpath().to_ascii_lowercase()).cloned());
            let Some(parsed) = held.and_then(|bytes| vale_edit::adt::AdtFile::parse(&bytes).ok()) else {
                self.elsewhere.remove(&key);
                continue;
            };
            if let Err(e) = self.write_submitted_tile(&key, &parsed) {
                self.status = format!("{}: {e}", key.vpath());
            }
            self.files_revision += 1;
        }
    }

    /// Pack the project into a patch archive beside the game's own, replacing
    /// the one it published before — see `Project::publish_into`. Answers
    /// where it landed and how many files went in, or `None` when there was
    /// nothing to pack or the write failed; the status line says which.
    pub(crate) fn publish_archive(&mut self, data_dir: &str) -> Option<(std::path::PathBuf, usize)> {
        let _ = self.save_files();
        match self.project.publish_into(data_dir) {
            Ok(report) => {
                let mut line = match &report.to {
                    Some(to) => format!("{} files written to {}", report.files, to.display()),
                    None => "nothing to publish".to_string(),
                };
                if !report.removed.is_empty() {
                    line.push_str(&format!(
                        "; this project's earlier {} removed",
                        report.removed.join(", ")
                    ));
                }
                for (name, why) in &report.left {
                    warn!("publish: {name} could not be removed: {why}");
                    line.push_str(&format!(
                        "; {name} could not be removed ({why}) and still holds the \
                         project's older files — close the program that has it open and publish again"
                    ));
                }
                self.status = line;
                report.to.map(|to| (to, report.files))
            }
            Err(e) => {
                self.status = format!("publish: {e}");
                None
            }
        }
    }

}

/// What [`EditSession::save_one`] did with a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Saved {
    /// The tile differs from the archives' copy, and its bytes are in the
    /// project folder.
    Written,
    /// The tile is the archives' copy, and the project folder has no file
    /// for it.
    Dropped,
}
