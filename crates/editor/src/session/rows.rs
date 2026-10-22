//! The server row store: the project's row edits, waypoint paths and scripts,
//! what undo entry each write goes on, and their files in the project.

use bevy::prelude::*;
use super::EditSession;

/// What undo entry a server-row edit goes on, and what folds into it.
///
/// `subject` is the gesture key `vale_edit::undo::History::begin_gesture`
/// folds on, so it alone decides what becomes one entry. A form passes a
/// subject naming the column, so typing into a name box is one entry and
/// editing two columns is two. A drag passes one naming the row, so the
/// three position columns it writes are one entry — without that, moving a
/// creature would take three presses of `Ctrl+Z` to put back, one per axis.
#[derive(Debug, Clone, Copy)]
pub struct Gesture<'a> {
    /// What to call it in the history: "Move creature", "Edit creature".
    pub label: &'a str,
    /// What it is about. Two edits with the same subject close together in time
    /// are one entry; two with different subjects never are.
    pub subject: &'a str,
    /// The frame clock the fold window is measured on.
    pub now: f64,
}

impl EditSession {
    /// Run `write` with every server-row write in it on one undo entry.
    ///
    /// For a change to a group of spawns: each spawn's writer names its own row
    /// in its gesture subject, which is right for one spawn and makes a group
    /// move one entry per member. Inside this, every write that carries a
    /// gesture is given `label` and `subject` instead, so the members fold into
    /// one entry, and a drag that calls this on every frame keeps folding into
    /// the same entry because the subject does not change.
    ///
    /// Writes made by an undo carry no gesture and are unaffected.
    pub fn as_one<R>(
        &mut self,
        label: &str,
        subject: &str,
        write: impl FnOnce(&mut EditSession) -> R,
    ) -> R {
        let outer = self
            .one_gesture
            .replace((label.to_string(), subject.to_string()));
        let answer = write(self);
        self.one_gesture = outer;
        answer
    }

    /// Open the undo entry a server-row write goes on: its own gesture, or the
    /// group's while [`Self::as_one`] is running.
    fn open_gesture(&mut self, gesture: Gesture<'_>) {
        match &self.one_gesture {
            Some((label, subject)) => {
                self.history
                    .begin_gesture(label.clone(), subject.clone(), gesture.now)
            }
            None => self
                .history
                .begin_gesture(gesture.label, gesture.subject, gesture.now),
        }
    }

    /// Set one column of one row of one of the server's tables, or clear it.
    ///
    /// `value` is already a SQL literal — quoted and escaped by
    /// `vale_mangos::sql` at the moment it is typed, so there is one
    /// escaping rule and it is applied once. `None` takes the edit back, which
    /// removes the column from the project's file and so from the next save's
    /// statements.
    ///
    /// Nothing is sent anywhere here. The project's file is written and the
    /// database is reached by [`crate::server::rows::save`], which is what
    /// Save and a playtest call.
    pub fn set_server_edit(
        &mut self,
        table: &str,
        key: &vale_mangos::row::Key,
        column: &str,
        value: Option<String>,
        // …and the entry it goes on the undo stack as. `None` is an edit made
        // by an undo, which must not push one of its own.
        under: Option<Gesture<'_>>,
    ) {
        if self.server_edits.get(table, key, column) == value.as_deref() {
            // Nothing changed, so nothing is marked unsaved. A form redraws
            // every frame and writes what its widget holds; without this, a
            // panel merely being open would leave the project permanently
            // ahead of its folder and Save permanently lit — and every frame it
            // was drawn would be an entry on the undo stack.
            return;
        }
        if let Some(gesture) = under {
            let before = self
                .server_edits
                .get(table, key, column)
                .map(str::to_string);
            self.open_gesture(gesture);
            self.history
                .record_server_cell(vale_edit::undo::ServerCell {
                    table: table.to_string(),
                    key: key.text(),
                    column: column.to_string(),
                    before,
                    after: value.clone(),
                });
            // Closed at once, as `tables::fields::set_fields` closes its own.
            // `History::begin_gesture` leaves the change open, and an open
            // change is not on the stack: `next_undo` reads `done` and the panel
            // greys its button off that, so the last edit made could not be
            // undone at all and the one before it came back instead. The next
            // gesture with the same subject inside the fold window pops this
            // straight back off and continues it, so folding still works.
            self.history.end();
        }
        self.server_edits.set(table, key, column, value);
        self.server_edits_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// Record that the world database has changed under every cache of it:
    /// an Apply or a Put back of any subject finished, whether or not it
    /// succeeded, or the editor was pointed at another database.
    pub fn wrote_the_database(&mut self) {
        self.database_writes += 1;
    }

    /// Record that this project creates a row, removes one, or says nothing
    /// about it at all.
    ///
    /// [`Self::set_server_edit`]'s counterpart for the claim that is not a
    /// column. Creating a spawn sets no column of a row that is already there,
    /// because there is no row; removing one sets no column at all. What moves
    /// in both cases is the project's whole claim, so that is what is written
    /// and what goes on the undo stack — see `vale_edit::undo::ServerRow`.
    ///
    /// `None` takes the claim back entirely. Undoing a creation does this, and
    /// so does keeping a spawn that was marked for removal.
    ///
    /// Nothing is sent anywhere. The project's file is written and the database
    /// is reached by [`crate::server::creatures::apply`], which is a button.
    pub fn set_server_row(
        &mut self,
        table: &str,
        key: &vale_mangos::row::Key,
        row: Option<&vale_mangos::row::RowEdit>,
        // …and the entry it goes on the undo stack as. `None` is an edit made
        // by an undo, which must not push one of its own.
        under: Option<Gesture<'_>>,
    ) {
        let before = self.server_edits.row_line(table, key);
        let after = row.map(vale_mangos::row::RowEdit::to_line);
        if before == after {
            // Nothing changed, so nothing is marked unsaved. See
            // [`Self::set_server_edit`], where the guard is and why.
            return;
        }
        if let Some(gesture) = under {
            self.open_gesture(gesture);
            self.history
                .record_server_row(vale_edit::undo::ServerRow {
                    table: table.to_string(),
                    key: key.text(),
                    before,
                    after: after.clone(),
                });
            // Closed at once, for `set_server_edit`'s reason: an open
            // change is not on the stack, so the last edit made would be the
            // one that could not be undone.
            self.history.end();
        }
        self.server_edits.set_row_line(table, key, after.as_deref());
        self.server_edits_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// Set what this project says a creature's waypoint path is, or stop
    /// saying anything about it.
    ///
    /// [`Self::set_server_edit`]'s counterpart for the subject whose edit is a
    /// set of rows. `None` takes the claim back entirely, which leaves the
    /// database's own path alone; a `Some` holding a path with no nodes is
    /// the edit that deletes the path. The two are kept distinct on purpose.
    ///
    /// Nothing is sent anywhere. The project's file is written and the database
    /// is reached by [`crate::server::creatures::apply`], which is a button.
    pub fn set_server_path(
        &mut self,
        which: vale_mangos::path::Which,
        owner: u64,
        path: Option<&vale_mangos::path::Path>,
        // …and the entry it goes on the undo stack as. `None` is an edit made
        // by an undo, which must not push one of its own.
        under: Option<Gesture<'_>>,
    ) {
        let before = self.server_paths.get(which, owner);
        let after = path.cloned();
        if before == after {
            // Nothing changed, so nothing is marked unsaved. The form
            // redraws every frame and writes what it holds; without this a
            // panel merely being open would leave Save permanently lit and put
            // an entry on the undo stack every frame. See
            // [`Self::set_server_edit`], where the same guard is and why.
            return;
        }
        if let Some(gesture) = under {
            self.open_gesture(gesture);
            self.history
                .record_server_path(vale_edit::undo::ServerPath {
                    table: which.table().to_string(),
                    key: vale_mangos::row::Key::one(which.key_column(), owner).text(),
                    before: before.as_ref().map(vale_mangos::path::Path::to_line),
                    after: after.as_ref().map(vale_mangos::path::Path::to_line),
                });
            // Closed at once, for `set_server_edit`'s reason: an open change
            // is not on the stack, so the last edit made would be the one that
            // could not be undone.
            self.history.end();
        }
        match path {
            Some(path) => self.server_paths.set(path),
            None => self.server_paths.forget(which, owner),
        }
        self.server_paths_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// Set what this project says a script is, or stop saying anything about
    /// it. [`Self::set_server_path`] for the other subject whose edit is a
    /// set of rows: `None` takes the claim back, and a script with no rows is
    /// the edit that removes it.
    ///
    /// The undo entry is a [`vale_edit::undo::ServerPath`] under the
    /// script table's name, which is how [`Self::step_server_paths`] tells
    /// the two stores apart.
    pub fn set_server_script(
        &mut self,
        table: &'static str,
        id: u32,
        script: Option<&vale_mangos::scripts::Script>,
        under: Option<Gesture<'_>>,
    ) {
        let before = self.server_scripts.get(table, id);
        let after = script.cloned();
        if before == after {
            return;
        }
        if let Some(gesture) = under {
            self.open_gesture(gesture);
            self.history
                .record_server_path(vale_edit::undo::ServerPath {
                    table: table.to_string(),
                    key: vale_mangos::row::Key::one("id", u64::from(id)).text(),
                    before: before.as_ref().map(vale_mangos::scripts::Script::to_line),
                    after: after.as_ref().map(vale_mangos::scripts::Script::to_line),
                });
            self.history.end();
        }
        match script {
            Some(script) => self.server_scripts.set(script),
            None => self.server_scripts.forget(table, id),
        }
        self.server_scripts_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// Put a change's server-row edits in, or take them out.
    ///
    /// The server-row counterpart of [`Self::step_tables`], and simpler:
    /// there is no file to be open or not, because the store is the session's
    /// own and is always there. What it cannot do is reach the database — an
    /// undone edit is written back to the server by the next save, exactly as
    /// the edit itself was, because the statements a save emits are a diff of
    /// the project rather than a log of what was typed.
    pub(super) fn step_server(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        // The paths first, and outside the guard in `step_server_cells`. A
        // waypoint edit is a change with no `server_cells` at all, so when
        // this call came after an `is_empty` return on that list, Undo and
        // Redo did nothing to a path edit even though the entry was on the
        // stack and the button was lit. Each list of a change must be stepped
        // independently of the others being empty.
        self.step_server_paths(change, forward);
        // A created or removed row is stepped before its columns going
        // forward, and after them coming back. A change that creates a spawn
        // and then writes one of its columns has to have the row first, and
        // taking the row away has to come after the column — see
        // `vale_edit::undo::Change::server_rows`.
        if forward {
            self.step_server_rows(change, forward);
        }
        self.step_server_cells(change, forward);
        if !forward {
            self.step_server_rows(change, forward);
        }
    }

    /// …and the cells on their own, so [`Self::step_server`] can put the row
    /// claims on either side of them.
    fn step_server_cells(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        if change.server_cells.is_empty() {
            return;
        }
        for cell in &change.server_cells {
            let cell = match forward {
                true => cell.clone(),
                false => cell.reversed(),
            };
            let Some(key) = vale_mangos::row::Key::parse(&cell.key) else {
                // The key is written by `Key::text` and read by `Key::parse`,
                // so this cannot happen from anything this crate wrote. It is
                // logged rather than ignored, because otherwise an undo would
                // do nothing and give no reason.
                warn!("undo: {} is not a row key", cell.key);
                continue;
            };
            self.server_edits
                .set(&cell.table, &key, &cell.column, cell.after.clone());
        }
        self.server_edits_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// …and the same for the whole-row claims, which are a list of their own
    /// on the change for the paths' reason: a column and a claim about whether
    /// the row exists at all are addressed differently.
    fn step_server_rows(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        if change.server_rows.is_empty() {
            return;
        }
        for entry in &change.server_rows {
            let entry = match forward {
                true => entry.clone(),
                false => entry.reversed(),
            };
            let Some(key) = vale_mangos::row::Key::parse(&entry.key) else {
                warn!("undo: {} is not a row key", entry.key);
                continue;
            };
            self.server_edits
                .set_row_line(&entry.table, &key, entry.after.as_deref());
        }
        self.server_edits_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// …and the same for the paths, which are a list of their own on the change.
    ///
    /// Separate because the two are addressed differently — a column and a set
    /// of rows — and folding them would need one of them to pretend to be the
    /// other. See [`vale_edit::undo::ServerPath`].
    fn step_server_paths(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        if change.server_paths.is_empty() {
            return;
        }
        for entry in &change.server_paths {
            let entry = match forward {
                true => entry.clone(),
                false => entry.reversed(),
            };
            // A script's entry carries a script table's name; see
            // [`Self::set_server_script`].
            if let Some(table) = vale_mangos::scripts::table_named(&entry.table) {
                let Some(id) = vale_mangos::row::Key::parse(&entry.key).and_then(|key| key.first())
                else {
                    warn!("undo: {} is not a script key", entry.key);
                    continue;
                };
                let id = id as u32;
                match &entry.after {
                    Some(line) => self
                        .server_scripts
                        .set(&vale_mangos::scripts::Script::from_line(table, id, line)),
                    None => self.server_scripts.forget(table, id),
                }
                self.server_scripts_unsaved = true;
                continue;
            }
            let Some(which) = vale_mangos::path::Which::named(&entry.table) else {
                warn!("undo: {} is not a movement table", entry.table);
                continue;
            };
            let Some(owner) =
                vale_mangos::row::Key::parse(&entry.key).and_then(|key| key.first())
            else {
                warn!("undo: {} is not a path key", entry.key);
                continue;
            };
            match &entry.after {
                Some(line) => self
                    .server_paths
                    .set(&vale_mangos::path::Path::from_line(which, owner, line)),
                None => self.server_paths.forget(which, owner),
            }
        }
        self.server_paths_unsaved = true;
        self.server_edit_revision += 1;
    }

    /// Throw away one subject's server-row edits, and the undo entries
    /// addressed against them. Answers `(rows, paths)`.
    ///
    /// `wanted` is asked each table's name, because the store holds every
    /// subject's rows together: a creature discard that emptied it would take
    /// the item workspace's rows with it and count them as creatures. See
    /// `vale_mangos::row::Edits::forget_where`.
    ///
    /// The entries go for the reason [`History::forget_table`]'s do: an entry's
    /// `before` and `after` are addressed against the store as it was when the
    /// entry was made, and after it is emptied there is nothing for them to put
    /// back — undoing would write a value into a row the project has stopped
    /// claiming.
    pub fn forget_server_edits(
        &mut self,
        mut wanted: impl FnMut(&str) -> bool,
        paths: bool,
    ) -> (usize, usize) {
        let rows = self.server_edits.forget_where(&mut wanted);
        // The paths go with them for the subject that has any, because
        // Discard is one gesture about one subject, and leaving the paths
        // behind would report "edits discarded" while the next save went on
        // writing a `DELETE` and a dozen `INSERT`s. The scripts go by table,
        // as the rows do.
        let mut dropped = match paths {
            true => {
                let had = self.server_paths.len();
                self.server_paths = vale_mangos::path::Paths::default();
                had
            }
            false => 0,
        };
        let scripts: Vec<(&'static str, u32)> = self
            .server_scripts
            .iter()
            .filter(|script| wanted(script.table))
            .map(|script| (script.table, script.id))
            .collect();
        for (table, id) in &scripts {
            self.server_scripts.forget(table, *id);
        }
        dropped += scripts.len();
        // The waypoint entries go with the creature discard by their table,
        // so the script entries, which share the undo's list, stay.
        self.history.forget_server_rows_where(
            |table| wanted(table) || (paths && vale_mangos::path::table_named(table).is_some()),
            false,
        );
        self.server_edits_unsaved = true;
        if paths {
            self.server_paths_unsaved = true;
        }
        if !scripts.is_empty() {
            self.server_scripts_unsaved = true;
        }
        self.server_edit_revision += 1;
        (rows, dropped)
    }

    /// Whether the server half of this project is ahead of its folder.
    ///
    /// The third thing a save writes, beside the tiles and the tables. When the
    /// Save button read only the tile and table lists, it showed "Saved" and
    /// disabled itself while a creature edit was outstanding, so the keystroke
    /// was the only way to write it. After a revert of the rows, the store
    /// still claimed them, `sql\creatures.sql` was still on disk, and nothing
    /// asked for a save.
    ///
    /// Both stores, because both are written by the same press and neither has
    /// a count worth showing — what a person needs to know is that a save is
    /// owed, not how many rows it is.
    pub fn server_unsaved(&self) -> bool {
        self.server_edits_unsaved || self.server_paths_unsaved || self.server_scripts_unsaved
    }

    /// Write the project's scripts to their own file, or remove the file when
    /// there are none left. [`Self::save_server_paths`]' sibling, for the
    /// same reason: a script is written from a different shape.
    pub fn save_server_scripts(&mut self) -> bool {
        let path = crate::server::behaviour::SCRIPTS_VPATH;
        if self.server_scripts.is_empty() {
            if let Some(disk) = self.project.path_for(path) {
                let _ = std::fs::remove_file(disk);
            }
            self.server_scripts_unsaved = false;
            return true;
        }
        let body = self.server_scripts.to_text(&self.project.name);
        match self.project.write(path, body.as_bytes()) {
            Ok(_) => {
                self.server_scripts_unsaved = false;
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

    /// Write the project's row edits to its own file, or remove the file when
    /// there are none left.
    ///
    /// Separate from [`Self::save_all_tables`] because it is a different
    /// container: a DBC is bytes in the overlay and this is a text file nothing
    /// but this editor and a person reads.
    pub fn save_server_edits(&mut self) -> bool {
        let path = crate::server::creatures::EDITS_VPATH;
        if self.server_edits.is_empty() {
            if let Some(disk) = self.project.path_for(path) {
                let _ = std::fs::remove_file(disk);
            }
            self.server_edits_unsaved = false;
            return true;
        }
        let body = self.server_edits.to_text(&self.project.name);
        match self.project.write(path, body.as_bytes()) {
            Ok(_) => {
                self.server_edits_unsaved = false;
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

    /// Write the project's waypoint paths to its own file, or remove the file
    /// when there are none left.
    ///
    /// [`Self::save_server_edits`]' sibling and a second file for the reason
    /// the store is a second store: the two are written from different shapes
    /// and one file holding both could not be read by a person or corrected by
    /// hand, which is the property that makes these text at all.
    pub fn save_server_paths(&mut self) -> bool {
        let path = crate::server::creatures::PATHS_VPATH;
        if self.server_paths.is_empty() {
            if let Some(disk) = self.project.path_for(path) {
                let _ = std::fs::remove_file(disk);
            }
            self.server_paths_unsaved = false;
            return true;
        }
        let body = self.server_paths.to_text(&self.project.name);
        match self.project.write(path, body.as_bytes()) {
            Ok(_) => {
                self.server_paths_unsaved = false;
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

}

/// Read the project's row edits, or nothing.
///
/// A project with no server edits has no file, which is the ordinary case and
/// is not an error. A file that will not decode as text is reported and treated
/// as empty rather than failing the open: the alternative is a project that
/// cannot be opened at all because of a file no other tool reads.
pub(super) fn read_server_edits(project: &vale_edit::project::Project) -> vale_mangos::row::Edits {
    let Some(bytes) = project.read(crate::server::creatures::EDITS_VPATH) else {
        return vale_mangos::row::Edits::default();
    };
    match String::from_utf8(bytes) {
        Ok(text) => {
            // A line that could not be read is said, one warning each: it
            // is an edit the project no longer carries, and the next save
            // rewrites the file without it.
            let (edits, refused) = vale_mangos::row::Edits::read(&text);
            for line in refused {
                warn!(
                    "{}: not read, because its key or value is not a SQL literal: {line}",
                    crate::server::creatures::EDITS_VPATH
                );
            }
            edits
        }
        Err(e) => {
            warn!("{}: {e}", crate::server::creatures::EDITS_VPATH);
            vale_mangos::row::Edits::default()
        }
    }
}

/// The scripts the project changes, read out of its own file, or none.
pub(super) fn read_server_scripts(project: &vale_edit::project::Project) -> vale_mangos::scripts::Scripts {
    let Some(bytes) = project.read(crate::server::behaviour::SCRIPTS_VPATH) else {
        return vale_mangos::scripts::Scripts::default();
    };
    match String::from_utf8(bytes) {
        Ok(text) => vale_mangos::scripts::Scripts::from_text(&text),
        Err(e) => {
            warn!("{}: {e}", crate::server::behaviour::SCRIPTS_VPATH);
            vale_mangos::scripts::Scripts::default()
        }
    }
}

/// Read the project's waypoint paths, or nothing.
///
/// [`read_server_edits`]' sibling, on the same terms: no file is the ordinary
/// case, and a file that will not decode is reported and treated as empty
/// rather than failing the open.
pub(super) fn read_server_paths(project: &vale_edit::project::Project) -> vale_mangos::path::Paths {
    let Some(bytes) = project.read(crate::server::creatures::PATHS_VPATH) else {
        return vale_mangos::path::Paths::default();
    };
    match String::from_utf8(bytes) {
        Ok(text) => vale_mangos::path::Paths::from_text(&text),
        Err(e) => {
            warn!("{}: {e}", crate::server::creatures::PATHS_VPATH);
            vale_mangos::path::Paths::default()
        }
    }
}
