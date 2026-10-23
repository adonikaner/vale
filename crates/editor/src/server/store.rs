//! The server row store: the project's row edits, waypoint paths and scripts,
//! what undo entry each write goes on, and their files in the project.
//!
//! [`ServerRows`] holds the three stores and their files, and steps an undo
//! entry's server half. The writes that put an entry on the undo stack are
//! [`EditSession`] methods here, because the stack is the session's.

use bevy::prelude::*;
use vale_edit::project::Project;
use crate::session::EditSession;

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

/// The rows this project changes in the server's database, held by
/// [`EditSession::rows`].
#[derive(Default)]
pub struct ServerRows {
    /// The column and whole-row edits.
    ///
    /// Held by the session and not in the tool, for the reason the open tables
    /// are: a project's edits outlive whichever tool is chosen, and a save has
    /// to be able to write all of them without asking a panel what it is
    /// holding.
    ///
    /// Empty for every project that has not touched a server table, which is
    /// every terrain project. See [`vale_mangos::row::Edits`], and
    /// [`crate::server::rows`], which writes them.
    pub edits: vale_mangos::row::Edits,
    /// …and the waypoint paths it changes, which are the one server subject
    /// whose edit is a set of rows rather than a column.
    ///
    /// A second store rather than more entries in the first, because the two
    /// are written differently: a column edit is an `UPDATE` of a row that is
    /// already there, and a path is every row under a key replaced. See
    /// [`vale_mangos::path::Paths`], and the reason the unit is the whole
    /// path in that module's own comment.
    pub paths: vale_mangos::path::Paths,
    /// The scripts it changes, which are the other subject whose edit is a
    /// set of rows: a script's rows have no key of their own, so every row
    /// under an id is replaced. See [`vale_mangos::scripts::Scripts`].
    pub scripts: vale_mangos::scripts::Scripts,
    /// How many row edits have landed, for anything that cached a reading
    /// of the database.
    ///
    /// `crate::tools::spawns::creatures` is the one reader: it holds a map's worth of
    /// spawns read over a connection, and an edit to a position has to move the
    /// marker without a second round trip. A counter rather than a flag,
    /// because what it answers is "is what I read still true".
    pub revision: u64,
    /// Whether [`Self::edits`] is ahead of the project folder.
    pub edits_unsaved: bool,
    /// …and whether [`Self::paths`] is.
    pub paths_unsaved: bool,
    /// …and whether [`Self::scripts`] is.
    pub scripts_unsaved: bool,
    /// The undo entry every server-row write goes on while a group is being
    /// changed, as `(label, subject)`. See [`EditSession::as_one`].
    ///
    /// A spawn's writers each name their own row in their gesture subject, so a
    /// move of twelve creatures would be twelve entries on the stack. While this
    /// is set, every write that carries a gesture takes this one instead, and
    /// the twelve fold into one.
    one_gesture: Option<(String, String)>,
}

impl ServerRows {
    /// The three stores read out of `project`'s files, with nothing unsaved.
    pub(crate) fn read(project: &Project) -> ServerRows {
        ServerRows {
            edits: read_server_edits(project),
            paths: read_server_paths(project),
            scripts: read_server_scripts(project),
            ..ServerRows::default()
        }
    }

    /// Replace the three stores with `project`'s, as a project switch does,
    /// and move [`Self::revision`] so every cached reading is taken again.
    pub(crate) fn reread(&mut self, project: &Project) {
        self.edits = read_server_edits(project);
        self.paths = read_server_paths(project);
        self.scripts = read_server_scripts(project);
        self.revision += 1;
        self.edits_unsaved = false;
        self.paths_unsaved = false;
        self.scripts_unsaved = false;
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
    /// All three stores, because all are written by the same press and none has
    /// a count worth showing — what a person needs to know is that a save is
    /// owed, not how many rows it is.
    pub fn unsaved(&self) -> bool {
        self.edits_unsaved || self.paths_unsaved || self.scripts_unsaved
    }

    /// Put a change's server-row edits in, or take them out.
    ///
    /// The server-row counterpart of the session's table step, and simpler:
    /// there is no file to be open or not, because the store is the session's
    /// own and is always there. What it cannot do is reach the database — an
    /// undone edit is written back to the server by the next save, exactly as
    /// the edit itself was, because the statements a save emits are a diff of
    /// the project rather than a log of what was typed.
    pub(crate) fn step(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        // The paths first, and outside the guard in `step_cells`. A
        // waypoint edit is a change with no `server_cells` at all, so when
        // this call came after an `is_empty` return on that list, Undo and
        // Redo did nothing to a path edit even though the entry was on the
        // stack and the button was lit. Each list of a change must be stepped
        // independently of the others being empty.
        self.step_paths(change, forward);
        // A created or removed row is stepped before its columns going
        // forward, and after them coming back. A change that creates a spawn
        // and then writes one of its columns has to have the row first, and
        // taking the row away has to come after the column — see
        // `vale_edit::undo::Change::server_rows`.
        if forward {
            self.step_rows(change, forward);
        }
        self.step_cells(change, forward);
        if !forward {
            self.step_rows(change, forward);
        }
    }

    /// …and the cells on their own, so [`Self::step`] can put the row
    /// claims on either side of them.
    fn step_cells(&mut self, change: &vale_edit::undo::Change, forward: bool) {
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
            self.edits
                .set(&cell.table, &key, &cell.column, cell.after.clone());
        }
        self.edits_unsaved = true;
        self.revision += 1;
    }

    /// …and the same for the whole-row claims, which are a list of their own
    /// on the change for the paths' reason: a column and a claim about whether
    /// the row exists at all are addressed differently.
    fn step_rows(&mut self, change: &vale_edit::undo::Change, forward: bool) {
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
            self.edits
                .set_row_line(&entry.table, &key, entry.after.as_deref());
        }
        self.edits_unsaved = true;
        self.revision += 1;
    }

    /// …and the same for the paths, which are a list of their own on the change.
    ///
    /// Separate because the two are addressed differently — a column and a set
    /// of rows — and folding them would need one of them to pretend to be the
    /// other. See [`vale_edit::undo::ServerPath`].
    fn step_paths(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        if change.server_paths.is_empty() {
            return;
        }
        for entry in &change.server_paths {
            let entry = match forward {
                true => entry.clone(),
                false => entry.reversed(),
            };
            // A script's entry carries a script table's name; see
            // [`EditSession::set_server_script`].
            if let Some(table) = vale_mangos::scripts::table_named(&entry.table) {
                let Some(id) = vale_mangos::row::Key::parse(&entry.key).and_then(|key| key.first())
                else {
                    warn!("undo: {} is not a script key", entry.key);
                    continue;
                };
                let id = id as u32;
                match &entry.after {
                    Some(line) => self
                        .scripts
                        .set(&vale_mangos::scripts::Script::from_line(table, id, line)),
                    None => self.scripts.forget(table, id),
                }
                self.scripts_unsaved = true;
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
                    .paths
                    .set(&vale_mangos::path::Path::from_line(which, owner, line)),
                None => self.paths.forget(which, owner),
            }
        }
        self.paths_unsaved = true;
        self.revision += 1;
    }

    /// Throw away one subject's rows, and with `paths` every waypoint path.
    /// Answers `(rows, paths and scripts)`. The undo entries addressed against
    /// them are the session's to forget; see
    /// [`EditSession::forget_server_edits`].
    fn forget_where(&mut self, wanted: &mut impl FnMut(&str) -> bool, paths: bool) -> (usize, usize) {
        let rows = self.edits.forget_where(&mut *wanted);
        // The paths go with them for the subject that has any, because
        // Discard is one gesture about one subject, and leaving the paths
        // behind would report "edits discarded" while the next save went on
        // writing a `DELETE` and a dozen `INSERT`s. The scripts go by table,
        // as the rows do.
        let mut dropped = match paths {
            true => {
                let had = self.paths.len();
                self.paths = vale_mangos::path::Paths::default();
                had
            }
            false => 0,
        };
        let scripts: Vec<(&'static str, u32)> = self
            .scripts
            .iter()
            .filter(|script| wanted(script.table))
            .map(|script| (script.table, script.id))
            .collect();
        for (table, id) in &scripts {
            self.scripts.forget(table, *id);
        }
        dropped += scripts.len();
        self.edits_unsaved = true;
        if paths {
            self.paths_unsaved = true;
        }
        if !scripts.is_empty() {
            self.scripts_unsaved = true;
        }
        self.revision += 1;
        (rows, dropped)
    }

    /// Write the three stores to their files: the rows, the paths, then the
    /// scripts. Stops at the first that cannot be written and answers what
    /// went wrong, as `<path>: <error>`.
    pub fn save(&mut self, project: &Project) -> Result<(), String> {
        self.save_edits(project)?;
        self.save_paths(project)?;
        self.save_scripts(project)
    }

    /// Write the project's scripts to their own file, or remove the file when
    /// there are none left. [`Self::save_paths`]' sibling, for the
    /// same reason: a script is written from a different shape.
    fn save_scripts(&mut self, project: &Project) -> Result<(), String> {
        let path = crate::server::behaviour::SCRIPTS_VPATH;
        if self.scripts.is_empty() {
            if let Some(disk) = project.path_for(path) {
                let _ = std::fs::remove_file(disk);
            }
            self.scripts_unsaved = false;
            return Ok(());
        }
        let body = self.scripts.to_text(&project.name);
        match project.write(path, body.as_bytes()) {
            Ok(_) => {
                self.scripts_unsaved = false;
                Ok(())
            }
            Err(e) => Err(format!("{path}: {e}")),
        }
    }

    /// Write the project's row edits to its own file, or remove the file when
    /// there are none left.
    ///
    /// Separate from [`EditSession::save_all_tables`] because it is a different
    /// container: a DBC is bytes in the overlay and this is a text file nothing
    /// but this editor and a person reads.
    fn save_edits(&mut self, project: &Project) -> Result<(), String> {
        let path = crate::server::subject::EDITS_VPATH;
        if self.edits.is_empty() {
            if let Some(disk) = project.path_for(path) {
                let _ = std::fs::remove_file(disk);
            }
            self.edits_unsaved = false;
            return Ok(());
        }
        let body = self.edits.to_text(&project.name);
        match project.write(path, body.as_bytes()) {
            Ok(_) => {
                self.edits_unsaved = false;
                Ok(())
            }
            Err(e) => Err(format!("{path}: {e}")),
        }
    }

    /// Write the project's waypoint paths to its own file, or remove the file
    /// when there are none left.
    ///
    /// [`Self::save_edits`]' sibling and a second file for the reason
    /// the store is a second store: the two are written from different shapes
    /// and one file holding both could not be read by a person or corrected by
    /// hand, which is the property that makes these text at all.
    fn save_paths(&mut self, project: &Project) -> Result<(), String> {
        let path = crate::server::creatures::PATHS_VPATH;
        if self.paths.is_empty() {
            if let Some(disk) = project.path_for(path) {
                let _ = std::fs::remove_file(disk);
            }
            self.paths_unsaved = false;
            return Ok(());
        }
        let body = self.paths.to_text(&project.name);
        match project.write(path, body.as_bytes()) {
            Ok(_) => {
                self.paths_unsaved = false;
                Ok(())
            }
            Err(e) => Err(format!("{path}: {e}")),
        }
    }
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
            .rows
            .one_gesture
            .replace((label.to_string(), subject.to_string()));
        let answer = write(self);
        self.rows.one_gesture = outer;
        answer
    }

    /// Open the undo entry a server-row write goes on: its own gesture, or the
    /// group's while [`Self::as_one`] is running.
    fn open_gesture(&mut self, gesture: Gesture<'_>) {
        match &self.rows.one_gesture {
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
        if self.rows.edits.get(table, key, column) == value.as_deref() {
            // Nothing changed, so nothing is marked unsaved. A form redraws
            // every frame and writes what its widget holds; without this, a
            // panel merely being open would leave the project permanently
            // ahead of its folder and Save permanently lit — and every frame it
            // was drawn would be an entry on the undo stack.
            return;
        }
        if let Some(gesture) = under {
            let before = self
                .rows
                .edits
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
        self.rows.edits.set(table, key, column, value);
        self.rows.edits_unsaved = true;
        self.rows.revision += 1;
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
    /// is reached by an Apply on the Server panel, which is a button.
    pub fn set_server_row(
        &mut self,
        table: &str,
        key: &vale_mangos::row::Key,
        row: Option<&vale_mangos::row::RowEdit>,
        // …and the entry it goes on the undo stack as. `None` is an edit made
        // by an undo, which must not push one of its own.
        under: Option<Gesture<'_>>,
    ) {
        let before = self.rows.edits.row_line(table, key);
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
        self.rows.edits.set_row_line(table, key, after.as_deref());
        self.rows.edits_unsaved = true;
        self.rows.revision += 1;
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
    /// is reached by an Apply on the Server panel, which is a button.
    pub fn set_server_path(
        &mut self,
        which: vale_mangos::path::Which,
        owner: u64,
        path: Option<&vale_mangos::path::Path>,
        // …and the entry it goes on the undo stack as. `None` is an edit made
        // by an undo, which must not push one of its own.
        under: Option<Gesture<'_>>,
    ) {
        let before = self.rows.paths.get(which, owner);
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
            Some(path) => self.rows.paths.set(path),
            None => self.rows.paths.forget(which, owner),
        }
        self.rows.paths_unsaved = true;
        self.rows.revision += 1;
    }

    /// Set what this project says a script is, or stop saying anything about
    /// it. [`Self::set_server_path`] for the other subject whose edit is a
    /// set of rows: `None` takes the claim back, and a script with no rows is
    /// the edit that removes it.
    ///
    /// The undo entry is a [`vale_edit::undo::ServerPath`] under the
    /// script table's name, which is how [`ServerRows::step`] tells
    /// the two stores apart.
    pub fn set_server_script(
        &mut self,
        table: &'static str,
        id: u32,
        script: Option<&vale_mangos::scripts::Script>,
        under: Option<Gesture<'_>>,
    ) {
        let before = self.rows.scripts.get(table, id);
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
            Some(script) => self.rows.scripts.set(script),
            None => self.rows.scripts.forget(table, id),
        }
        self.rows.scripts_unsaved = true;
        self.rows.revision += 1;
    }

    /// Throw away one subject's server-row edits, and the undo entries
    /// addressed against them. Answers `(rows, paths)`.
    ///
    /// `wanted` is asked each table's name, because the store holds every
    /// subject's rows together: a creature discard that emptied it would take
    /// the item workspace's rows with it and count them as creatures. See
    /// `vale_mangos::row::Edits::forget_where`.
    ///
    /// The entries go for the reason `History::forget_table`'s do: an entry's
    /// `before` and `after` are addressed against the store as it was when the
    /// entry was made, and after it is emptied there is nothing for them to put
    /// back — undoing would write a value into a row the project has stopped
    /// claiming.
    pub fn forget_server_edits(
        &mut self,
        mut wanted: impl FnMut(&str) -> bool,
        paths: bool,
    ) -> (usize, usize) {
        let dropped = self.rows.forget_where(&mut wanted, paths);
        // The waypoint entries go with the creature discard by their table,
        // so the script entries, which share the undo's list, stay.
        self.history.forget_server_rows_where(
            |table| wanted(table) || (paths && vale_mangos::path::table_named(table).is_some()),
            false,
        );
        dropped
    }
}

/// Read the project's row edits, or nothing.
///
/// A project with no server edits has no file, which is the ordinary case and
/// is not an error. A file that will not decode as text is reported and treated
/// as empty rather than failing the open: the alternative is a project that
/// cannot be opened at all because of a file no other tool reads.
fn read_server_edits(project: &Project) -> vale_mangos::row::Edits {
    let Some(bytes) = project.read(crate::server::subject::EDITS_VPATH) else {
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
                    crate::server::subject::EDITS_VPATH
                );
            }
            edits
        }
        Err(e) => {
            warn!("{}: {e}", crate::server::subject::EDITS_VPATH);
            vale_mangos::row::Edits::default()
        }
    }
}

/// The scripts the project changes, read out of its own file, or none.
fn read_server_scripts(project: &Project) -> vale_mangos::scripts::Scripts {
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
fn read_server_paths(project: &Project) -> vale_mangos::path::Paths {
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
