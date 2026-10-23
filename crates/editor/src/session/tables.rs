//! The open client tables: opening one, its bytes in the overlay, discarding
//! its unsaved edits, and writing it to the project.

use vale_client::assets::GameAssets;
use vale_edit::dbc::DbcFile;
use bevy::prelude::*;
use super::EditSession;

impl EditSession {
    /// Open a client table for editing, if it is not open already.
    ///
    /// Read through the same chain the renderer uses, so a table edited in an
    /// earlier session comes back edited. `false` for a name the archives do not
    /// answer — and for the four entries under `DBFilesClient\` that are zero
    /// bytes, which are not tables at all. See `vale dbc`.
    pub(crate) fn open_table(&mut self, assets: &GameAssets, name: &str) -> bool {
        if self.tables.contains_key(name) {
            return true;
        }
        let path = vale_assets::tables::dbc::dbc_path(name);
        let bytes = match assets.with_archive(|chain| Ok(chain.read(&path).ok())) {
            Ok(Some(bytes)) => bytes,
            _ => {
                self.status = format!("{path}: not in the archives");
                return false;
            }
        };
        match DbcFile::parse(&bytes) {
            Ok(table) => {
                self.tables.insert(name.to_string(), table);
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

    pub(crate) fn table(&self, name: &str) -> Option<&DbcFile> {
        self.tables.get(name)
    }

    pub(crate) fn table_mut(&mut self, name: &str) -> Option<&mut DbcFile> {
        self.tables.get_mut(name)
    }

    /// Note that a table's bytes are ahead of what is written down.
    ///
    /// The table counterpart of the `unsaved` mark, and not of
    /// [`Self::publish`]: a field edit does not write the file anywhere.
    ///
    /// The reason is cost. `Spell.dbc` is 16 MB, an edit arrives on every
    /// frame a number is dragged, and [`Self::publish_table`] serialises the
    /// whole table — so publishing per edit is a gigabyte a second of copying
    /// for no reader at all. There is no reader because
    /// `GameAssets::display_tables` parses every DBC once and hands out what it
    /// parsed: nothing in a running session reads a table again. What a table
    /// edit has to reach is the project folder and the patch archive, and both
    /// of those are [`Self::save_table`].
    pub(crate) fn table_edited(&mut self, name: &str) {
        self.unsaved_tables.insert(name.to_string());
        self.table_revision += 1;
    }

    /// Write an open table's bytes into the overlay, so the next read of its
    /// path answers with them — the table counterpart of [`Self::publish`].
    ///
    /// Called when a table is saved rather than when it is edited, for the
    /// reason on [`Self::table_edited`]. What it is for is the next thing to
    /// read a table: a playtest logging in, or a relaunch.
    pub(crate) fn publish_table(&mut self, name: &str) {
        let Some(table) = self.tables.get(name) else {
            return;
        };
        let path = vale_assets::tables::dbc::dbc_path(name).to_ascii_lowercase();
        let bytes = table.write();
        if let Ok(mut edited) = self.edited.write() {
            edited.insert(path, bytes);
        }
    }

    /// Take a table's bytes back out of the overlay, so the project folder
    /// or the archives answer for its path again, and have the client read
    /// its tables again. The counterpart of [`Self::publish_table`].
    pub(crate) fn unpublish_table(&mut self, name: &str) {
        let path = vale_assets::tables::dbc::dbc_path(name).to_ascii_lowercase();
        let had = match self.edited.write() {
            Ok(mut edited) => edited.remove(&path).is_some(),
            Err(_) => false,
        };
        if had {
            self.tables_republished = true;
        }
    }

    /// Put a table back to what is written down, dropping every unsaved
    /// edit to it: the bytes are read again through the chain — the project
    /// folder's copy if one has been saved, else the archives' — and the
    /// undo stack forgets the table. `false` for a table that is not open
    /// or will not read again.
    ///
    /// The stack has to forget rather than keep, and the reason is on
    /// `History::forget_table`: its entries were made against the table as it
    /// was, and after the re-read that is a different table. The readers
    /// holding record indices — the browser, the reverse index — rebuild off
    /// the revision, which this bumps.
    pub(crate) fn discard_table(&mut self, assets: &GameAssets, name: &str) -> bool {
        if !self.tables.contains_key(name) {
            return false;
        }
        self.tables.remove(name);
        let dropped = self.history.forget_table(name);
        let had_edits = self.unsaved_tables.remove(name);
        self.table_revision += 1;
        // A preview puts the edited table in the overlay, and the overlay
        // answers a read before the project folder and the archives do.
        // Left there, the read below is answered with the edits it is meant
        // to drop, and the client goes on drawing them.
        self.unpublish_table(name);
        if !self.open_table(assets, name) {
            return false;
        }
        self.status = match (had_edits, dropped) {
            (false, _) => format!("{name}: nothing to discard"),
            (true, 0) => format!("{name}: unsaved edits discarded"),
            (true, 1) => format!("{name}: unsaved edits discarded, 1 undo entry dropped"),
            (true, n) => format!("{name}: unsaved edits discarded, {n} undo entries dropped"),
        };
        true
    }

    /// Write every table whose bytes are ahead of the project folder; how
    /// many were written.
    pub(crate) fn save_all_tables(&mut self) -> usize {
        let unsaved: Vec<String> = self.unsaved_tables.iter().cloned().collect();
        unsaved.iter().filter(|name| self.save_table(name)).count()
    }

    /// …and write it to the project folder.
    ///
    /// A table whose rows are the archives' rows is not kept: the project's
    /// copy is removed and its bytes taken out of the overlay, so an edit that
    /// was undone before the save leaves no file and no entry in the project
    /// dialog. Rows rather than bytes, because [`DbcFile::set_string`] only
    /// appends to the string block and an undone string edit leaves the text
    /// behind it.
    pub(crate) fn save_table(&mut self, name: &str) -> bool {
        let Some(table) = self.tables.get(name) else {
            return false;
        };
        let path = vale_assets::tables::dbc::dbc_path(name);
        let bytes = table.write();
        if self.changes_nothing(&path, &bytes) {
            return match self.project.revert(&path) {
                Ok(_) => {
                    self.unpublish_table(name);
                    self.unsaved_tables.remove(name);
                    self.status = format!("{name} matches the archives, so it is not kept in the project");
                    true
                }
                Err(e) => {
                    self.status = format!("{path}: {e}");
                    false
                }
            };
        }
        match self.project.write(&path, &bytes) {
            Ok(at) => {
                self.publish_table(name);
                self.unsaved_tables.remove(name);
                self.status = format!("saved {}", at.display());
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_edit::project::Project;

    /// A table a preview published is taken back out of the overlay, and the
    /// client is told to read its tables again. Discard depends on this: it
    /// reads the table again, and the overlay answers before the project
    /// folder and the archives.
    #[test]
    fn a_published_table_is_taken_back_out_of_the_overlay() {
        let install =
            std::env::temp_dir().join(format!("vale-session-unpublish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        let mut bytes = b"WDBC".to_vec();
        for word in [0u32, 35, 140, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0);
        let table = DbcFile::parse(&bytes).expect("an empty table");
        session.tables.insert("SpellVisualKit".to_string(), table);
        let path = vale_assets::tables::dbc::dbc_path("SpellVisualKit").to_ascii_lowercase();

        session.publish_table("SpellVisualKit");
        assert!(session.edited.read().unwrap().contains_key(&path));
        session.tables_republished = false;
        session.unpublish_table("SpellVisualKit");
        assert!(!session.edited.read().unwrap().contains_key(&path));
        assert!(session.tables_republished);

        // A table that was never published asks for no re-read.
        session.tables_republished = false;
        session.unpublish_table("SpellVisualKit");
        assert!(!session.tables_republished);
        let _ = std::fs::remove_dir_all(&install);
    }
}
