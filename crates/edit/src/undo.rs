//! The stack the two directions of an edit are read off.
//!
//! ## A stroke is one entry, not one entry per frame
//!
//! A held brush produces edits every frame, and each one is a complete inverse
//! of the frame before it. Pushing them separately would make one drag of the
//! mouse forty presses of undo. So a stroke is opened once, absorbs whatever it
//! produces, and closes as one [`Change`]: an edit that lands on a chunk the
//! stroke has already touched keeps the `before` from the first time and takes
//! the `after` from the last.
//!
//! ## …and it is one entry however many tiles it crosses
//!
//! A brush is a circle in the world and a tile is a 533-yard square, so a stroke
//! near a border edits both sides of it. Every edit therefore carries the
//! [`TileKey`] it belongs to and a change holds edits for as many tiles as it
//! touched. A change that could only name one tile is a change that has to stop
//! at the border, which leaves a step in the ground exactly there.
//!
//! ## …and a gesture with no end is one entry too
//!
//! A stroke has a press and a release to be opened and closed by. Three of the
//! things this editor does have neither: turning the wheel, holding an arrow
//! key, and dragging a number field in a panel. Each produces a run of separate
//! changes with nothing to say the run is over, so each one used to be its own
//! entry — twenty notches of the wheel was twenty presses of undo.
//!
//! [`History::begin_gesture`] is the answer for those. A change carries what it
//! was about (`doodad 7 scale`, not just the label) and when it was last added
//! to; a change with the same subject arriving within [`History::gap`] of the
//! last one takes the entry back off the stack and continues it. So a run of
//! events is one entry, and stopping for half a second starts a new one.
//!
//! ## The history does not touch a tile
//!
//! [`History::record`] is told what has already happened, and
//! [`History::undo`] hands back the change rather than applying it. The stack
//! knows which tiles a change is about and nothing about where they are kept;
//! the caller has the map and applies [`Change::revert`] to each one it holds.

use crate::adt::AdtFile;
use crate::dbc::{Cell, DbcFile, Row};
use crate::ops::Edit;
use crate::TileKey;

/// One column of one row of the server's database, set to one value.
///
/// The fourth kind of thing a [`Change`] carries, and the only one whose
/// subject this crate cannot reach: a tile is a file it holds, a DBC cell is a
/// record in a file it holds, and this is a row in a MySQL database this
/// crate has no connection to and does not need one.
///
/// So it is addressed by text: a table name, a key written as its own
/// `column=value;column=value` form, and a column name. What the key means is
/// the caller's — `vale-mangos` builds it and reads it back — and what this
/// crate does with it is put the value in the caller's own store and take it
/// out again. That keeps `vale-edit` what it is: no connection, no driver,
/// and a `Change` that is still one list of things that happened.
///
/// `before` and `after` are `Option<String>` because absent is a value: a
/// column the project does not change has no entry in the store at all, which
/// is different from one it sets to the empty string. Undoing the first edit to
/// a column must remove it rather than write something.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerCell {
    /// The server table, as `vale-mangos` names it: `creature_template`.
    pub table: String,
    /// Which row of it, as `vale_mangos::row::Key::text` writes one:
    /// `entry=68;patch=0`.
    pub key: String,
    pub column: String,
    /// What the project's store held before, or `None` for a column it was not
    /// changing at all.
    pub before: Option<String>,
    /// …and after.
    pub after: Option<String>,
}

impl ServerCell {
    /// Whether two cells are about the same column of the same row, which is
    /// what [`Change::absorb_server_cell`] folds on.
    pub fn names_the_same_column_as(&self, other: &ServerCell) -> bool {
        self.table == other.table && self.key == other.key && self.column == other.column
    }

    /// This cell reversed, for an undo.
    pub fn reversed(&self) -> ServerCell {
        ServerCell {
            table: self.table.clone(),
            key: self.key.clone(),
            column: self.column.clone(),
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

/// A whole waypoint path this project claims, before and after.
///
/// [`ServerCell`]'s sibling for the one server subject whose edit is a set of
/// rows rather than a column. A path is written by replacing every row under
/// its key — vmangos renumbers the points at each start, so a per-row edit
/// would address rows that move — and the undo therefore has to carry the whole
/// path rather than one value. See `vale_mangos::path`.
///
/// The path is carried as text for [`ServerCell`]'s reason: this crate is
/// what an edit to a file is and knows nothing about a database, so what it
/// stores is what the caller's own store writes, opaque to everything here.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerPath {
    /// The movement table: `creature_movement` or `creature_movement_template`.
    pub table: String,
    /// Which path, as `vale_mangos::row::Key::text` writes one: `id=12345`.
    pub key: String,
    /// The path the project claimed before, or `None` when it claimed none.
    ///
    /// `None` and an empty path are different, which is the whole reason
    /// this is an `Option<String>` and not a `String`: claiming no path leaves
    /// the database's own, and claiming an empty one deletes it.
    pub before: Option<String>,
    /// …and after.
    pub after: Option<String>,
}

impl ServerPath {
    /// Whether two entries are about the same path, which is what
    /// [`Change::absorb_server_path`] folds on.
    pub fn names_the_same_path_as(&self, other: &ServerPath) -> bool {
        self.table == other.table && self.key == other.key
    }

    /// This entry reversed, for an undo.
    pub fn reversed(&self) -> ServerPath {
        ServerPath {
            table: self.table.clone(),
            key: self.key.clone(),
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

/// A whole row of the server's database this project creates or removes,
/// before and after.
///
/// [`ServerCell`]'s sibling for the change that is not a column. Creating a
/// creature spawn sets no column of a row that is already there — there is no
/// row — and removing one sets no column at all. What moves in both cases is
/// the project's whole claim on the row, so that is what the entry carries.
///
/// The claim is carried as text, as [`ServerPath`]'s path is and for its
/// reason: this crate is what an edit to a file is and knows nothing about a
/// database, so what it stores is what the caller's own store writes, opaque to
/// everything here. `vale_mangos::row::Edits::row_line` is the writer.
///
/// `None` is the project saying nothing about the row, which is what
/// undoing a creation leaves behind and is different from a claim that happens
/// to name no column.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerRow {
    /// The server table, as `vale-mangos` names it: `creature`.
    pub table: String,
    /// Which row of it, as `vale_mangos::row::Key::text` writes one:
    /// `guid=10000001`.
    pub key: String,
    /// What the project claimed about the row before, or `None` for nothing.
    pub before: Option<String>,
    /// …and after.
    pub after: Option<String>,
}

impl ServerRow {
    /// Whether two entries are about the same row, which is what
    /// [`Change::absorb_server_row`] folds on.
    pub fn names_the_same_row_as(&self, other: &ServerRow) -> bool {
        self.table == other.table && self.key == other.key
    }

    /// This entry reversed, for an undo.
    pub fn reversed(&self) -> ServerRow {
        ServerRow {
            table: self.table.clone(),
            key: self.key.clone(),
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

/// One entry on the stack: everything one action did, across every tile it did
/// it to.
///
/// ## Table edits are a second list, not a second stack
///
/// A change carries tile edits and table edits side by side, in two lists
/// rather than one. The two apply to different objects — a tile edit is
/// `apply(&mut AdtFile)` and a table edit is `apply(&mut DbcFile)` — so
/// merging them into one enum would give every existing arm a target it
/// cannot use. There is still one stack: pressing `Ctrl+Z` undoes the last
/// thing done, whether that was a wall or a spell's name.
#[derive(Debug, Clone)]
pub struct Change {
    /// What to call it in an interface. "Raise terrain", "Move doodad".
    pub label: String,
    /// Each edit and the tile it belongs to, in the order they were made.
    pub edits: Vec<(TileKey, Edit)>,
    /// …and each field edit with the table it belongs to, named as the table is
    /// (`"Spell"`), in the order they were made.
    pub cells: Vec<(String, Cell)>,
    /// …and each whole record added or removed, with its table, in the order
    /// they were made.
    ///
    /// Applied before the cells and reverted after them. A change that adds
    /// a row and then writes its fields has to put the row in before the fields
    /// can be written, and take the fields back before the row goes; and a
    /// removal moves every index after it, so a cell recorded after one is
    /// addressed against the table as the removal left it. The two lists are
    /// kept apart so that order is a rule here rather than a property of how
    /// the caller happened to record them.
    pub rows: Vec<(String, Row)>,
    /// …and each column of a row in the server's database, in the order
    /// they were made — see [`ServerCell`].
    ///
    /// A fourth list rather than a variant of [`Self::cells`], because the two
    /// are addressed differently and cannot be folded together: a DBC cell is a
    /// record index and a field index into a file this crate holds, and a
    /// server cell is a keyed row in a database this crate cannot reach. What
    /// they have in common is only that both are one column set to one value.
    pub server_cells: Vec<ServerCell>,
    /// …and each whole waypoint path, in the order they were made — see
    /// [`ServerPath`].
    ///
    /// A fifth list rather than a variant of [`Self::server_cells`], on that
    /// list's own argument: a column and a set of rows are addressed
    /// differently and cannot be folded together.
    pub server_paths: Vec<ServerPath>,
    /// …and each whole row of the server's database created or removed, in the
    /// order they were made — see [`ServerRow`].
    ///
    /// A sixth list on the same argument as the fifth, and the claim it carries
    /// is not a column: creating a row sets no column of anything that exists,
    /// and removing one sets none at all.
    ///
    /// Applied before the cells and reverted after them, for
    /// [`Self::rows`]' reason one container along: a change that creates a row
    /// and then writes a column of it has to have the row first, and taking the
    /// row away has to come after the column. The two lists are kept apart so
    /// that the order is a rule here rather than a property of how the caller
    /// happened to record them.
    pub server_rows: Vec<ServerRow>,
    /// What this change is about, for the gestures that have no release to
    /// close them — `doodad 7 scale`, `doodad 7 position`. Two changes with the
    /// same subject close together in time are one action; two with different
    /// subjects never are, however close together they arrive. `None` for a
    /// stroke, which is closed by the button coming up and needs no window.
    ///
    /// See [`History::begin_gesture`].
    pub gesture: Option<String>,
    /// When this change was last added to, on whatever clock the caller passes
    /// [`History::begin_gesture`]. Meaningless while `gesture` is `None`.
    pub at: f64,
}

impl Change {
    pub fn new(label: impl Into<String>) -> Change {
        Change {
            label: label.into(),
            edits: Vec::new(),
            cells: Vec::new(),
            rows: Vec::new(),
            server_cells: Vec::new(),
            server_paths: Vec::new(),
            server_rows: Vec::new(),
            gesture: None,
            at: 0.0,
        }
    }

    /// Add a field edit, folding it into one already here about the same field
    /// of the same record of the same table.
    ///
    /// The folding is what makes typing into a name box one undo step rather
    /// than one per keystroke: the first `before` stands and the last `after`
    /// wins, which is the same rule [`Self::absorb`] applies to a held brush.
    pub fn absorb_cell(&mut self, table: &str, cell: Cell) {
        for (had, existing) in self.cells.iter_mut() {
            if had == table && existing.record == cell.record && existing.field == cell.field {
                existing.after = cell.after;
                return;
            }
        }
        self.cells.push((table.to_string(), cell));
    }

    /// Add a whole record, added or removed. Never folded: two rows are two
    /// rows.
    pub fn absorb_row(&mut self, table: &str, row: Row) {
        self.rows.push((table.to_string(), row));
    }

    /// Add a column of a server row, folding it into one already here about the
    /// same column of the same row of the same table.
    ///
    /// [`Self::absorb_cell`]'s rule, for [`Self::absorb_cell`]'s reason: the
    /// first `before` stands and the last `after` wins, so typing into a name
    /// box is one undo step and a drag across a courtyard is one entry rather
    /// than sixty.
    pub fn absorb_server_cell(&mut self, cell: ServerCell) {
        for existing in self.server_cells.iter_mut() {
            if existing.names_the_same_column_as(&cell) {
                existing.after = cell.after;
                return;
            }
        }
        self.server_cells.push(cell);
    }

    /// Take a whole path into this change, folding it into one already here.
    ///
    /// [`Self::absorb_server_cell`]'s rule for the same reason: dragging a node
    /// writes the path on every frame of the drag, and the gesture has to be
    /// one undo step rather than sixty.
    pub fn absorb_server_path(&mut self, path: ServerPath) {
        for existing in self.server_paths.iter_mut() {
            if existing.names_the_same_path_as(&path) {
                existing.after = path.after;
                return;
            }
        }
        self.server_paths.push(path);
    }

    /// Take a whole row claim into this change, folding it into one already
    /// here about the same row.
    ///
    /// [`Self::absorb_server_path`]'s rule for its reason: the first `before`
    /// stands and the last `after` wins, so a row created and then removed
    /// inside one gesture is one entry that does nothing rather than two.
    pub fn absorb_server_row(&mut self, row: ServerRow) {
        for existing in self.server_rows.iter_mut() {
            if existing.names_the_same_row_as(&row) {
                existing.after = row.after;
                return;
            }
        }
        self.server_rows.push(row);
    }

    /// The tables this change touched, in the order they were first touched.
    pub fn tables(&self) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for table in self
            .rows
            .iter()
            .map(|(table, _)| table)
            .chain(self.cells.iter().map(|(table, _)| table))
        {
            if !found.iter().any(|had| had == table) {
                found.push(table.clone());
            }
        }
        found
    }

    /// Put this change's edits for one table back in: the rows first, then the
    /// fields — see [`Self::rows`].
    pub fn apply_table(&self, table: &str, into: &mut DbcFile) {
        for (_, row) in self.rows.iter().filter(|(had, _)| had == table) {
            row.apply(into);
        }
        for (_, cell) in self.cells.iter().filter(|(had, _)| had == table) {
            cell.apply(into);
        }
    }

    /// …and take them back out, in reverse for [`Self::revert`]'s reason: the
    /// fields first, then the rows.
    pub fn revert_table(&self, table: &str, into: &mut DbcFile) {
        for (_, cell) in self.cells.iter().filter(|(had, _)| had == table).rev() {
            cell.revert(into);
        }
        for (_, row) in self.rows.iter().filter(|(had, _)| had == table).rev() {
            row.revert(into);
        }
    }

    /// Whether this change adds or removes a record of `table`, which moves
    /// record indices — a caller holding one has to look its row up again.
    pub fn renumbers_records(&self, table: &str) -> bool {
        self.rows.iter().any(|(had, _)| had == table)
    }

    /// The records this change touched in one table, for a panel that has one
    /// of them open.
    pub fn records(&self, table: &str) -> Vec<usize> {
        let mut found: Vec<usize> = Vec::new();
        for (_, cell) in self.cells.iter().filter(|(had, _)| had == table) {
            if !found.contains(&cell.record) {
                found.push(cell.record);
            }
        }
        found
    }

    /// Add an edit, folding it into one already here when the two are about the
    /// same thing on the same tile.
    ///
    /// The search runs from the newest edit back and stops at the last edit on
    /// the tile that renumbers its placements. A `Doodad` or `Building` edit
    /// names a row by index, and an [`Edit::Placements`] edit between two of
    /// them can give that index to a different placement. Folding across it
    /// would join two placements' records into one edit, and a `Placements` edit
    /// folded back past an index edit would be applied before that edit on a
    /// redo. The case is a group move followed by its members being put in the
    /// tiles their origins are in, all in one entry.
    pub fn absorb(&mut self, tile: &TileKey, edit: Edit) {
        for (had, existing) in self.edits.iter_mut().rev() {
            if had != tile {
                continue;
            }
            if let Some(merged) = merge(existing, &edit) {
                *existing = merged;
                return;
            }
            if existing.renumbers_placements() || edit.renumbers_placements() {
                break;
            }
        }
        self.edits.push((tile.clone(), edit));
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
            && self.cells.is_empty()
            && self.rows.is_empty()
            && self.server_cells.is_empty()
            && self.server_paths.is_empty()
            && self.server_rows.is_empty()
    }

    /// Drop everything this change says about one table, leaving what it says
    /// about other tables and about tiles. `true` when anything was dropped.
    /// See [`History::forget_table`].
    pub fn forget_table(&mut self, table: &str) -> bool {
        let before = self.cells.len() + self.rows.len();
        self.cells.retain(|(had, _)| had != table);
        self.rows.retain(|(had, _)| had != table);
        before != self.cells.len() + self.rows.len()
    }

    /// The tiles this change touched, in the order they were first touched.
    pub fn tiles(&self) -> Vec<TileKey> {
        let mut found: Vec<TileKey> = Vec::new();
        for (tile, _) in &self.edits {
            if !found.contains(tile) {
                found.push(tile.clone());
            }
        }
        found
    }

    /// The chunks it touched on one tile, for a caller rebuilding meshes.
    pub fn chunks(&self, tile: &TileKey) -> Vec<usize> {
        let mut found: Vec<usize> = Vec::new();
        for chunk in self
            .edits
            .iter()
            .filter(|(had, _)| had == tile)
            .filter_map(|(_, edit)| edit.chunk())
        {
            if !found.contains(&chunk) {
                found.push(chunk);
            }
        }
        found
    }

    /// …and the `MDDF` entries it moved on one tile, for a caller that has them
    /// drawn — the placement counterpart of [`Self::chunks`].
    pub fn placements(&self, tile: &TileKey) -> Vec<usize> {
        let mut found: Vec<usize> = Vec::new();
        for index in self
            .edits
            .iter()
            .filter(|(had, _)| had == tile)
            .filter_map(|(_, edit)| edit.placement())
        {
            if !found.contains(&index) {
                found.push(index);
            }
        }
        found
    }

    /// …and the `MODF` entries it moved, which is a separate list from
    /// [`Self::placements`] because `MDDF` and `MODF` are numbered separately.
    pub fn buildings(&self, tile: &TileKey) -> Vec<usize> {
        let mut found: Vec<usize> = Vec::new();
        for index in self
            .edits
            .iter()
            .filter(|(had, _)| had == tile)
            .filter_map(|(_, edit)| edit.building())
        {
            if !found.contains(&index) {
                found.push(index);
            }
        }
        found
    }

    /// …and the chunks it repainted on one tile, for a caller that has the
    /// blend maps on the GPU. A different set from [`Self::chunks`] because a
    /// repaint takes a different route to the screen: see [`Edit::painted`].
    pub fn painted(&self, tile: &TileKey) -> Vec<usize> {
        let mut found: Vec<usize> = Vec::new();
        for chunk in self
            .edits
            .iter()
            .filter(|(had, _)| had == tile)
            .filter_map(|(_, edit)| edit.painted())
        {
            if !found.contains(&chunk) {
                found.push(chunk);
            }
        }
        found
    }

    /// Whether this change gave a chunk on one tile a texture it did not have,
    /// which is the half of a repaint that the atlas cannot carry — see
    /// [`Edit::changes_the_texture_set`].
    pub fn changes_the_texture_set(&self, tile: &TileKey) -> bool {
        self.edits
            .iter()
            .any(|(had, edit)| had == tile && edit.changes_the_texture_set())
    }

    /// Whether this change renumbers a tile's placement lists — see
    /// [`Edit::renumbers_placements`]. A caller holding an index into them has
    /// to let go of it rather than reconcile.
    pub fn renumbers_placements(&self, tile: &TileKey) -> bool {
        self.edits
            .iter()
            .any(|(had, edit)| had == tile && edit.renumbers_placements())
    }

    /// …and whether it changes the ground mesh rather than anything the mesh
    /// holds, so the tile has to be read again — see [`Edit::remeshes`]. An undo
    /// of a hole is the case: nothing on screen can be patched back.
    pub fn remeshes(&self, tile: &TileKey) -> bool {
        self.edits
            .iter()
            .any(|(had, edit)| had == tile && edit.remeshes())
    }

    /// Put this change's edits for one tile back in.
    pub fn apply(&self, tile: &TileKey, into: &mut AdtFile) {
        for (_, edit) in self.edits.iter().filter(|(had, _)| had == tile) {
            edit.apply(into);
        }
    }

    /// …and take them back out. In reverse, because two edits about one chunk
    /// only invert in the order they were made.
    pub fn revert(&self, tile: &TileKey, into: &mut AdtFile) {
        for (_, edit) in self
            .edits
            .iter()
            .filter(|(had, _)| had == tile)
            .rev()
        {
            edit.revert(into);
        }
    }
}

/// Two edits about the same thing, as one edit spanning both.
fn merge(first: &Edit, second: &Edit) -> Option<Edit> {
    match (first, second) {
        (
            Edit::Heights {
                chunk: a, before, ..
            },
            Edit::Heights {
                chunk: b, after, ..
            },
        ) if a == b => Some(Edit::Heights {
            chunk: *a,
            before: before.clone(),
            after: after.clone(),
        }),
        (
            Edit::Normals {
                chunk: a, before, ..
            },
            Edit::Normals {
                chunk: b, after, ..
            },
        ) if a == b => Some(Edit::Normals {
            chunk: *a,
            before: before.clone(),
            after: after.clone(),
        }),
        (
            Edit::Doodad {
                index: a, before, ..
            },
            Edit::Doodad {
                index: b, after, ..
            },
        ) if a == b => Some(Edit::Doodad {
            index: *a,
            before: *before,
            after: *after,
        }),
        (Edit::Placements { before, .. }, Edit::Placements { after, .. }) => {
            Some(Edit::Placements {
                before: before.clone(),
                after: after.clone(),
            })
        }
        (
            Edit::Building {
                index: a, before, ..
            },
            Edit::Building {
                index: b, after, ..
            },
        ) if a == b => Some(Edit::Building {
            index: *a,
            before: *before,
            after: *after,
        }),
        (
            Edit::Paint {
                chunk: a, before, ..
            },
            Edit::Paint {
                chunk: b, after, ..
            },
        ) if a == b => Some(Edit::Paint {
            chunk: *a,
            before: before.clone(),
            after: after.clone(),
        }),
        _ => None,
    }
}

/// What has been done, and what has been undone.
#[derive(Debug, Default)]
pub struct History {
    done: Vec<Change>,
    undone: Vec<Change>,
    /// The stroke being held, if one is.
    open: Option<Change>,
    /// How many entries to keep. A height edit is about a kilobyte per chunk, so
    /// a few hundred strokes is a few megabytes.
    pub depth: usize,
    /// How long after a gesture's last edit another edit of the same subject
    /// still counts as the same action — see [`History::begin_gesture`].
    ///
    /// Half a second. Long enough that no run of wheel notches, key repeats or
    /// dragged frames falls out of it, and short enough that a deliberate second
    /// nudge is a second entry. It is a field rather than a constant so a test
    /// can set it to zero and check that the window is what does the folding.
    pub gap: f64,
    /// When the button holding the current gesture went down, or `None` when
    /// no button holds one. See [`History::hold`].
    held_since: Option<f64>,
}

impl History {
    pub fn new() -> History {
        History {
            depth: 256,
            gap: 0.5,
            ..History::default()
        }
    }

    /// Open a stroke. Anything recorded until [`History::end`] folds into it.
    pub fn begin(&mut self, label: impl Into<String>) {
        self.end();
        self.open = Some(Change::new(label));
    }

    /// Open a stroke that continues the last one when it was the same
    /// gesture: the same `subject`, less than [`History::gap`] ago.
    ///
    /// For the actions with no release to close them — the wheel, a held key, a
    /// number field dragged in a panel. Each of those produces a run of separate
    /// changes and nothing says the run has ended, so the entry is taken back off
    /// the stack and continued rather than a new one being pushed. Close it with
    /// [`History::end`] exactly as a stroke is closed; leaving it open costs
    /// nothing, because the next `begin` or `begin_gesture` closes it.
    ///
    /// `now` is any monotonic clock in seconds — the caller's frame time. This
    /// crate has none of its own and is not going to grow one.
    ///
    /// A change is never continued across an undo. Anything on the redo
    /// branch means the last thing that happened was an undo and not an edit, so
    /// continuing would fold new work into an entry the person has already
    /// stepped past.
    pub fn begin_gesture(
        &mut self,
        label: impl Into<String>,
        subject: impl Into<String>,
        now: f64,
    ) {
        self.end();
        let subject = subject.into();
        let held = self.held_since;
        let continues = self.undone.is_empty()
            && self.done.last().is_some_and(|last| {
                last.gesture.as_deref() == Some(subject.as_str())
                    && (now - last.at <= self.gap || held.is_some_and(|since| last.at >= since))
            });
        let mut change = match continues {
            true => self.done.pop().expect("just tested"),
            false => Change::new(label),
        };
        change.gesture = Some(subject);
        change.at = now;
        self.open = Some(change);
    }

    /// Say that a button is holding the gesture being made, so a pause in it
    /// does not end it.
    ///
    /// A pointer drag of a spawn writes through [`History::begin_gesture`] on
    /// every frame the pointer moves, and nothing on a frame it does not. Held
    /// still for longer than [`History::gap`] and then moved again, the drag
    /// became two entries. While a hold is on, a gesture continues the last
    /// entry of the same subject when that entry was added to after the hold
    /// began, however long ago that was. An entry from before the hold is
    /// continued only under the ordinary window, so a second drag of the same
    /// thing is a second entry.
    ///
    /// `now` is the clock [`History::begin_gesture`] is given. Calling this on
    /// every frame of a drag keeps the first time. [`History::release`] ends
    /// it.
    pub fn hold(&mut self, now: f64) {
        self.held_since.get_or_insert(now);
    }

    /// The button holding a gesture has come up. See [`History::hold`].
    pub fn release(&mut self) {
        self.held_since = None;
    }

    /// Re-open the change that was just pushed, so a write that is a
    /// consequence of it folds into it instead of becoming a second entry.
    ///
    /// The case it exists for: the WMO tool re-fits a placement's `MODF` box
    /// once a drag has settled, and the settle happens after the drag's own
    /// change has been closed and pushed. Nobody asked for that write and
    /// nobody wants to take it back on its own, but as an entry of its own it is
    /// what the first `Ctrl+Z` after a move lands on — so the building stays
    /// where it was put and it takes a second press to bring it back.
    ///
    /// Returns false when there is nothing to amend and the caller must open a
    /// change of its own. Two cases: an empty stack, and a redo branch,
    /// where the last thing that happened was an undo rather than an edit and
    /// folding would put new work into an entry that has already been stepped
    /// past. That is [`History::begin_gesture`]'s rule, here for its reason.
    pub fn amend(&mut self) -> bool {
        self.end();
        if !self.undone.is_empty() {
            return false;
        }
        match self.done.pop() {
            Some(change) => {
                self.open = Some(change);
                true
            }
            None => false,
        }
    }

    /// Record what has already been applied to a tile.
    pub fn record(&mut self, tile: &TileKey, edits: impl IntoIterator<Item = Edit>) {
        match self.open.as_mut() {
            Some(change) => {
                for edit in edits {
                    change.absorb(tile, edit);
                }
            }
            // Outside a stroke each call is its own entry, so a caller that
            // forgets to open one still gets a working undo.
            None => {
                let mut change = Change::new("Edit");
                for edit in edits {
                    change.absorb(tile, edit);
                }
                if !change.is_empty() {
                    self.push(change);
                }
            }
        }
    }

    /// Record a field edit that has already been applied to a table.
    ///
    /// The table counterpart of [`History::record`], and it follows the same
    /// rule: inside a gesture it folds, outside one it is its own entry. A cell
    /// that moved nothing is dropped here rather than making an undo step that
    /// does nothing — a panel writes its value back on every frame it is drawn,
    /// so most of what arrives here is a field being set to what it already is.
    pub fn record_cell(&mut self, table: &str, cell: Cell) {
        if !cell.moves() {
            return;
        }
        match self.open.as_mut() {
            Some(change) => change.absorb_cell(table, cell),
            None => {
                let mut change = Change::new("Edit");
                change.absorb_cell(table, cell);
                self.push(change);
            }
        }
    }

    /// Record a column of a server row, already applied to the caller's own
    /// store.
    ///
    /// [`History::record_cell`]'s counterpart one container along, and it
    /// follows the same two rules: inside a gesture it folds, outside one it is
    /// its own entry, and a cell that moved nothing is dropped here rather than
    /// making an undo step that does nothing. That check matters here for the
    /// same reason it does there: a form writes its value back on every frame
    /// it is drawn.
    pub fn record_server_cell(&mut self, cell: ServerCell) {
        if cell.before == cell.after {
            return;
        }
        match self.open.as_mut() {
            Some(change) => change.absorb_server_cell(cell),
            None => {
                let mut change = Change::new("Edit");
                change.absorb_server_cell(cell);
                self.push(change);
            }
        }
    }

    /// Record a whole waypoint path, already applied to the caller's own store.
    ///
    /// [`Self::record_server_cell`]'s counterpart for the subject whose edit is
    /// a set of rows, and it follows the same three rules: inside a gesture it
    /// folds, outside one it is its own entry, and a path that moved nothing is
    /// dropped here rather than making an undo step that does nothing.
    pub fn record_server_path(&mut self, path: ServerPath) {
        if path.before == path.after {
            return;
        }
        match self.open.as_mut() {
            Some(change) => change.absorb_server_path(path),
            None => {
                let mut change = Change::new("Edit");
                change.absorb_server_path(path);
                self.push(change);
            }
        }
    }

    /// Record a whole server row created or removed, already applied to the
    /// caller's own store.
    ///
    /// [`Self::record_server_path`]'s counterpart for the claim that is not a
    /// column, and it follows the same three rules: inside a gesture it folds,
    /// outside one it is its own entry, and a claim that moved nothing is
    /// dropped here rather than making an undo step that does nothing.
    pub fn record_server_row(&mut self, row: ServerRow) {
        if row.before == row.after {
            return;
        }
        match self.open.as_mut() {
            Some(change) => change.absorb_server_row(row),
            None => {
                let mut change = Change::new("Edit");
                change.absorb_server_row(row);
                self.push(change);
            }
        }
    }

    /// Record a record added or removed, already applied to the table.
    ///
    /// Inside a stroke it joins the stroke; outside one it is an entry of its
    /// own. See [`Change::rows`] for the order it is applied in.
    pub fn record_row(&mut self, table: &str, row: Row) {
        match self.open.as_mut() {
            Some(change) => change.absorb_row(table, row),
            None => {
                let mut change = Change::new(match row.added {
                    true => "Add row",
                    false => "Remove row",
                });
                change.absorb_row(table, row);
                self.push(change);
            }
        }
    }

    /// Close the stroke and put it on the stack. A stroke that changed nothing
    /// is dropped rather than pushed, so an accidental click costs no undo.
    pub fn end(&mut self) {
        if let Some(change) = self.open.take() {
            if !change.is_empty() {
                self.push(change);
            }
        }
    }

    /// Forget every entry about a row in the server's database, on both
    /// branches and in the stroke being held. An entry that was only about
    /// those goes; one that also touched a tile or a client table keeps that
    /// half. Returns how many entries were dropped.
    ///
    /// For a store that has been emptied — a discard. [`Self::forget_table`]'s
    /// argument exactly: an entry's `before` and `after` are addressed against
    /// the store as it was when the entry was made, so undoing against an empty
    /// one would put a value back into a row the project has stopped claiming.
    pub fn forget_server_rows(&mut self) -> usize {
        self.forget_server_rows_where(|_| true, true)
    }

    /// …and the same for one subject's tables, which is what a discard is.
    ///
    /// The store holds every subject's rows together and so does the stack, so
    /// a discard that forgot both outright would take the panel next door's
    /// undo with it. `wanted` is asked the table's name, exactly as the store's
    /// own `forget_where` is. A [`ServerPath`] entry goes by its table too:
    /// the waypoint entries with the creature tables, the script entries with
    /// the script tables. `paths` clears every path entry as well, for a
    /// caller that has emptied the path store outright.
    pub fn forget_server_rows_where(
        &mut self,
        mut wanted: impl FnMut(&str) -> bool,
        paths: bool,
    ) -> usize {
        let mut strip = |change: &mut Change| {
            change.server_cells.retain(|cell| !wanted(&cell.table));
            change.server_rows.retain(|row| !wanted(&row.table));
            change.server_paths.retain(|path| !wanted(&path.table));
            if paths {
                change.server_paths.clear();
            }
        };
        let mut dropped = 0;
        for branch in [&mut self.done, &mut self.undone] {
            let before = branch.len();
            for change in branch.iter_mut() {
                strip(change);
            }
            branch.retain(|change| !change.is_empty());
            dropped += before - branch.len();
        }
        if let Some(open) = self.open.as_mut() {
            strip(open);
            if open.is_empty() {
                self.open = None;
                dropped += 1;
            }
        }
        dropped
    }

    /// Forget everything on the stack about one table, on both branches
    /// and in the stroke being held. An entry that was only about that table
    /// goes; one that also touched a tile or another table keeps that half.
    /// Returns how many entries were dropped.
    ///
    /// For a table whose bytes have been put back to the file's — a discard.
    /// An entry's `before` and `after` are addressed against the table as it
    /// was when the entry was made, and after a re-read that table is a
    /// different one: a record it added is not there to remove, and a cell it
    /// changed reads the file's value where it expects its own. Undoing
    /// against it would write a plausible wrong value rather than fail, so the
    /// entries are dropped rather than left.
    pub fn forget_table(&mut self, table: &str) -> usize {
        let mut dropped = 0;
        for branch in [&mut self.done, &mut self.undone] {
            let before = branch.len();
            for change in branch.iter_mut() {
                change.forget_table(table);
            }
            branch.retain(|change| !change.is_empty());
            dropped += before - branch.len();
        }
        if let Some(change) = self.open.as_mut() {
            change.forget_table(table);
            if change.is_empty() {
                self.open = None;
                dropped += 1;
            }
        }
        dropped
    }

    fn push(&mut self, change: Change) {
        self.done.push(change);
        // A new action is a new branch: whatever was undone cannot be reached
        // from here any more.
        self.undone.clear();
        while self.done.len() > self.depth {
            self.done.remove(0);
        }
    }

    /// Take the last change off the stack.
    ///
    /// It is not applied: the caller holds the tiles and puts it back with
    /// [`Change::revert`], once per tile in [`Change::tiles`].
    pub fn undo(&mut self) -> Option<Change> {
        self.end();
        let change = self.done.pop()?;
        self.undone.push(change.clone());
        Some(change)
    }

    /// …and the last one undone, on the same terms.
    pub fn redo(&mut self) -> Option<Change> {
        let change = self.undone.pop()?;
        self.done.push(change.clone());
        Some(change)
    }

    /// Whether any entry on the stack — done, undone, or the one being held —
    /// names this tile.
    ///
    /// It used to be the gate on closing a tile, because undo wrote into the
    /// open tile and skipped one that was not there. `EditSession::undo` opens
    /// what it names now, which gives the same guarantee from the other end
    /// and is the only one a map-wide edit can live with: an entry naming 687
    /// tiles would have pinned every one of them for the rest of the session.
    ///
    /// Kept because it answers "is this tile spoken for", which is a question
    /// a report can ask.
    pub fn touches(&self, tile: &TileKey) -> bool {
        self.done
            .iter()
            .chain(self.undone.iter())
            .chain(self.open.iter())
            .any(|change| change.edits.iter().any(|(key, _)| key == tile))
    }

    /// What undo would take back, and what redo would put in.
    pub fn next_undo(&self) -> Option<&Change> {
        self.done.last()
    }

    pub fn next_redo(&self) -> Option<&Change> {
        self.undone.last()
    }

    pub fn depth_done(&self) -> usize {
        self.done.len()
    }

    /// Whether a stroke is being held.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(x: u32, y: u32) -> TileKey {
        TileKey::new("Azeroth", x, y)
    }

    fn edit(chunk: usize, before: f32, after: f32) -> Edit {
        Edit::Heights {
            chunk,
            before: vec![before; 145],
            after: vec![after; 145],
        }
    }

    fn server_cell(column: &str, before: Option<&str>, after: Option<&str>) -> ServerCell {
        ServerCell {
            table: "creature_template".into(),
            key: "entry=68;patch=0".into(),
            column: column.into(),
            before: before.map(str::to_string),
            after: after.map(str::to_string),
        }
    }

    fn server_path(before: Option<&str>, after: Option<&str>) -> ServerPath {
        ServerPath {
            table: "creature_movement".into(),
            key: "id=12345".into(),
            before: before.map(str::to_string),
            after: after.map(str::to_string),
        }
    }

    /// A change that carries only a path is not an empty change.
    ///
    /// This guards a regression from the waypoint feature: a path edit makes
    /// a `Change` with no `server_cells` at all, and the session stepped its
    /// paths after an `is_empty` return on that other list. The entry was on
    /// the stack and the Undo button was lit, but pressing it did nothing at
    /// all. Anything that walks a change by asking one list whether it is
    /// empty is wrong for the same reason.
    #[test]
    fn a_change_carrying_only_a_path_is_not_empty() {
        let mut change = Change::new("Add waypoint");
        assert!(change.is_empty());
        change.absorb_server_path(server_path(None, Some("1,2,3,100,0,0,0,0")));
        assert!(!change.is_empty(), "a path-only change must reach every step of an undo");
        assert!(change.server_cells.is_empty(), "and it has no cells, which is the trap");
    }

    /// A path edit outside a gesture is its own entry, and inside one it folds
    /// — which is what makes a drag across a field one press of undo rather
    /// than one per frame.
    #[test]
    fn server_paths_fold_within_a_gesture() {
        let mut history = History::new();
        history.begin_gesture("Move waypoint", "creature_movement 12345 path", 1.0);
        history.record_server_path(server_path(Some("a"), Some("b")));
        history.record_server_path(server_path(Some("b"), Some("c")));
        history.end();
        let change = history.next_undo().expect("one entry");
        assert_eq!(change.server_paths.len(), 1);
        assert_eq!(change.server_paths[0].before.as_deref(), Some("a"));
        assert_eq!(change.server_paths[0].after.as_deref(), Some("c"));
    }

    /// A path that moved nothing makes no entry, because the window writes
    /// what it holds on every frame it is drawn.
    #[test]
    fn a_path_that_did_not_change_makes_no_entry() {
        let mut history = History::new();
        history.record_server_path(server_path(Some("a"), Some("a")));
        assert!(history.next_undo().is_none());
    }

    /// Reversing swaps the two readings, which is the whole of an undo for this
    /// kind — including the two `None` cases, which are "the project claimed no
    /// path" and are not the same as an empty path.
    #[test]
    fn reversing_a_path_swaps_what_the_project_claims() {
        let made = server_path(None, Some("1,2,3,100,0,0,0,0"));
        let back = made.reversed();
        assert_eq!(back.before.as_deref(), Some("1,2,3,100,0,0,0,0"));
        assert_eq!(back.after, None);
        assert_eq!(back.reversed(), made);
        // An emptied path is a claim of its own and reverses like any other.
        let emptied = server_path(Some("1,2,3,100,0,0,0,0"), Some(""));
        assert_eq!(emptied.reversed().after.as_deref(), Some("1,2,3,100,0,0,0,0"));
    }

    /// Two edits to the same column inside one gesture are one entry, with
    /// the first `before` and the last `after`. That is what makes typing a
    /// name one press of undo rather than one per keystroke.
    #[test]
    fn server_cells_fold_by_column_within_a_gesture() {
        let mut history = History::new();
        history.begin_gesture("Edit creature", "creature_template name", 0.0);
        history.record_server_cell(server_cell("name", Some("'A'"), Some("'AB'")));
        history.record_server_cell(server_cell("name", Some("'AB'"), Some("'ABC'")));
        history.end();

        let change = history.undo().expect("one entry");
        assert_eq!(change.server_cells.len(), 1);
        assert_eq!(change.server_cells[0].before.as_deref(), Some("'A'"));
        assert_eq!(change.server_cells[0].after.as_deref(), Some("'ABC'"));
        assert!(history.undo().is_none(), "the two keystrokes were one entry");
    }

    fn server_row(before: Option<&str>, after: Option<&str>) -> ServerRow {
        ServerRow {
            table: "creature".into(),
            key: "guid=10000001".into(),
            before: before.map(str::to_string),
            after: after.map(str::to_string),
        }
    }

    /// A change carrying only a created row is not empty, which is the same
    /// trap the paths have: creating a spawn makes a `Change` with no
    /// `server_cells` at all, and anything that asks one list whether it is
    /// empty reads the whole entry as nothing and drops it. The entry would be
    /// on no stack and the Undo button would be grey.
    #[test]
    fn a_change_carrying_only_a_created_row_is_not_empty() {
        let mut history = History::new();
        history.begin_gesture("New spawn", "creature 10000001", 0.0);
        history.record_server_row(server_row(None, Some("insert\tid=68")));
        history.end();

        let change = history.next_undo().expect("the creation is on the stack");
        assert!(!change.is_empty());
        assert!(change.server_cells.is_empty(), "and it has no cells, which is the trap");
        assert_eq!(change.server_rows.len(), 1);
    }

    /// A row claimed and then given up inside one gesture folds to one entry,
    /// and the first `before` stands.
    #[test]
    fn server_rows_fold_within_a_gesture() {
        let mut history = History::new();
        history.begin_gesture("New spawn", "creature 10000001", 0.0);
        history.record_server_row(server_row(None, Some("insert\tid=68")));
        history.record_server_row(server_row(Some("insert\tid=68"), Some("insert\tid=69")));
        history.end();

        let change = history.undo().expect("one entry");
        assert_eq!(change.server_rows.len(), 1);
        assert_eq!(change.server_rows[0].before, None);
        assert_eq!(change.server_rows[0].after.as_deref(), Some("insert\tid=69"));
        // …and reversed is what an undo applies: the claim goes away again.
        assert_eq!(change.server_rows[0].reversed().after, None);
    }

    /// A claim that moved nothing is not an entry, on the cells' own rule:
    /// a panel redrawing every frame must not fill the stack.
    #[test]
    fn a_server_row_that_moved_nothing_is_dropped() {
        let mut history = History::new();
        history.record_server_row(server_row(Some("delete"), Some("delete")));
        history.record_server_row(server_row(None, None));
        assert!(history.next_undo().is_none());
    }

    /// Discarding the project's server edits drops the row claims with
    /// everything else, or an undo would put back a row the project has stopped
    /// claiming.
    #[test]
    fn forgetting_the_server_rows_takes_the_claims_too() {
        let mut history = History::new();
        history.record_server_row(server_row(None, Some("insert\tid=68")));
        assert_eq!(history.forget_server_rows(), 1);
        assert!(history.next_undo().is_none());
    }

    /// …and a discard of one subject leaves the other subject's entries.
    ///
    /// The stack holds every subject's server rows together, as the store does,
    /// so a filter that forgot both would be the store's own fault one layer
    /// up: giving up a creature edit would make the item edits beside it
    /// un-undoable while the project went on claiming them.
    #[test]
    fn forgetting_one_subjects_rows_leaves_the_others() {
        let mut history = History::new();
        history.record_server_row(server_row(None, Some("insert\tid=68")));
        history.record_server_cell(ServerCell {
            table: "item_template".into(),
            key: "entry=19019;patch=10".into(),
            column: "Quality".into(),
            before: Some("3".into()),
            after: Some("4".into()),
        });

        let creatures = |table: &str| table == "creature" || table == "creature_template";
        assert_eq!(history.forget_server_rows_where(creatures, true), 1);

        let change = history.next_undo().expect("the item edit is still on the stack");
        assert!(change.server_rows.is_empty(), "the creature claim went");
        assert_eq!(change.server_cells.len(), 1);
        assert_eq!(change.server_cells[0].table, "item_template");
    }

    /// An edit is undoable the moment it is made.
    ///
    /// `begin_gesture` leaves the change open, and an open change is not on
    /// the stack: `next_undo` reads `done`, and a panel greys its Undo button
    /// off that. A caller that opened a gesture and did not close it left the
    /// last edit made unreachable: pressing Undo either did nothing or took
    /// back the edit before it, which was reported as a field that would not
    /// go back to what it had been. `EditSession::set_server_edit` ends its
    /// gesture for exactly this reason, as `tables::set_fields` already did.
    #[test]
    fn one_edit_is_undoable_without_waiting_for_a_second() {
        let mut history = History::new();
        history.begin_gesture("Edit creature", "creature_template name", 0.0);
        history.record_server_cell(server_cell("name", None, Some("'A'")));
        history.end();

        assert!(history.next_undo().is_some(), "the only edit made is on the stack");
        let change = history.undo().expect("one entry");
        assert_eq!(change.server_cells.len(), 1);
        assert_eq!(change.server_cells[0].after.as_deref(), Some("'A'"));
    }

    /// …and two edits to different columns of the same row are two cells on
    /// one entry, not one cell. A move writes three columns and comes back in
    /// one press; it must put all three back.
    #[test]
    fn a_move_is_three_cells_on_one_entry() {
        let mut history = History::new();
        history.begin_gesture("Move creature", "creature 19272 position", 0.0);
        for column in ["position_x", "position_y", "position_z"] {
            history.record_server_cell(server_cell(column, Some("1"), Some("2")));
        }
        history.end();

        let change = history.undo().expect("one entry");
        assert_eq!(change.server_cells.len(), 3);
        assert_eq!(change.label, "Move creature");
    }

    /// A cell that moved nothing is not an entry. A form writes its value
    /// back on every frame it is drawn, so without this, opening a panel would
    /// fill the undo stack with entries that do nothing.
    #[test]
    fn a_server_cell_that_moved_nothing_is_dropped() {
        let mut history = History::new();
        history.record_server_cell(server_cell("name", Some("'A'"), Some("'A'")));
        assert!(history.undo().is_none());
        history.record_server_cell(server_cell("name", None, None));
        assert!(history.undo().is_none());
    }

    /// Absent is a value. Undoing the first edit to a column has to remove
    /// it from the project's store, not write an empty string — the project
    /// would otherwise go on claiming to change a column it had stopped
    /// changing.
    #[test]
    fn reversing_a_first_edit_removes_the_column() {
        let cell = server_cell("level_min", None, Some("56"));
        let back = cell.reversed();
        assert_eq!(back.before.as_deref(), Some("56"));
        assert_eq!(back.after, None);
        assert_eq!(back.reversed(), cell);
    }

    /// A change carrying only server cells is not empty, or `History::end`
    /// would drop it and the edit would have no undo at all.
    #[test]
    fn a_change_of_server_cells_alone_is_not_empty() {
        let mut change = Change::new("Edit creature");
        assert!(change.is_empty());
        change.absorb_server_cell(server_cell("name", None, Some("'A'")));
        assert!(!change.is_empty());
    }

    /// Two cells about different rows of the same table never fold, however
    /// close together they arrive: they are two creatures.
    #[test]
    fn two_rows_are_two_cells() {
        let mut change = Change::new("Edit creature");
        change.absorb_server_cell(server_cell("name", None, Some("'A'")));
        let mut other = server_cell("name", None, Some("'B'"));
        other.key = "entry=69;patch=0".into();
        change.absorb_server_cell(other);
        assert_eq!(change.server_cells.len(), 2);
    }

    /// A consequence of a change folds into it, so one press of undo takes
    /// back both halves.
    ///
    /// The WMO tool's case: a drag is one change, and the `MODF` box is re-fitted
    /// from the drawn geometry once the drag has settled — after the drag's
    /// change has been closed and pushed. Without [`History::amend`] that write
    /// is what the first press of undo hits, so the building does not move and
    /// it takes a second press to reach the drag.
    #[test]
    fn an_amendment_folds_into_the_change_it_follows() {
        let mut history = History::new();
        history.begin("Move WMO");
        history.record(&key(32, 48), [edit(0, 1.0, 2.0)]);
        history.end();
        assert_eq!(history.depth_done(), 1);

        assert!(history.amend(), "there is a change to fold into");
        history.record(&key(32, 48), [edit(1, 3.0, 4.0)]);
        history.end();
        assert_eq!(history.depth_done(), 1, "still one entry, and not two");

        let change = history.undo().expect("one press takes back both");
        assert_eq!(change.label, "Move WMO");
        assert_eq!(change.chunks(&key(32, 48)), vec![0, 1]);
        assert_eq!(history.depth_done(), 0);
    }

    /// ...and it declines rather than folding into an entry that has been
    /// stepped past. Both cases: an empty stack, and a redo branch, where
    /// the last thing that happened was an undo and not an edit.
    #[test]
    fn an_amendment_declines_where_there_is_nothing_to_fold_into() {
        let mut history = History::new();
        assert!(!history.amend(), "an empty stack has nothing to amend");

        history.begin("Move WMO");
        history.record(&key(32, 48), [edit(0, 1.0, 2.0)]);
        history.end();
        history.undo();
        assert!(
            !history.amend(),
            "a redo branch means the last thing that happened was an undo"
        );
        assert_eq!(history.depth_done(), 0, "and nothing was taken off the stack");
    }

    /// A stroke that touches one chunk forty times is one entry, spanning the
    /// whole of it.
    #[test]
    fn a_stroke_folds_into_one_entry() {
        let mut history = History::new();
        history.begin("Raise terrain");
        for step in 0..40 {
            history.record(&key(32, 48), [edit(70, step as f32, step as f32 + 1.0)]);
        }
        history.end();
        assert_eq!(history.depth_done(), 1);
        let change = history.next_undo().unwrap();
        assert_eq!(change.edits.len(), 1);
        match &change.edits[0].1 {
            Edit::Heights { before, after, .. } => {
                assert_eq!(before[0], 0.0, "the first before is kept");
                assert_eq!(after[0], 40.0, "the last after is taken");
            }
            other => panic!("{other:?}"),
        }
    }

    /// Two chunks in one stroke stay two edits, because neither inverts the
    /// other.
    #[test]
    fn two_chunks_in_one_stroke_stay_two_edits() {
        let mut history = History::new();
        history.begin("Raise terrain");
        history.record(&key(32, 48), [edit(70, 0.0, 1.0), edit(71, 0.0, 1.0)]);
        history.end();
        let change = history.next_undo().unwrap();
        assert_eq!(change.edits.len(), 2);
        assert_eq!(change.chunks(&key(32, 48)), vec![70, 71]);
    }

    /// The same chunk index on two tiles is two edits. A stroke on a border
    /// edits both sides of it, and chunk 70 of one tile has nothing to do with
    /// chunk 70 of the next: folding them together would undo one and corrupt
    /// the other.
    #[test]
    fn one_stroke_across_a_tile_border_is_one_entry_and_two_tiles() {
        let mut history = History::new();
        history.begin("Raise terrain");
        history.record(&key(32, 48), [edit(70, 0.0, 1.0)]);
        history.record(&key(33, 48), [edit(70, 5.0, 6.0)]);
        history.record(&key(32, 48), [edit(70, 1.0, 2.0)]);
        history.end();

        assert_eq!(history.depth_done(), 1, "one stroke is one entry");
        let change = history.next_undo().unwrap();
        assert_eq!(change.edits.len(), 2, "one edit per tile, folded");
        assert_eq!(change.tiles(), vec![key(32, 48), key(33, 48)]);
        // The fold happened on the right one: the first tile's edit spans both
        // of its steps and the second tile's is untouched.
        match &change.edits[0].1 {
            Edit::Heights { before, after, .. } => {
                assert_eq!((before[0], after[0]), (0.0, 2.0));
            }
            other => panic!("{other:?}"),
        }
        match &change.edits[1].1 {
            Edit::Heights { before, after, .. } => {
                assert_eq!((before[0], after[0]), (5.0, 6.0));
            }
            other => panic!("{other:?}"),
        }
    }

    /// A stroke that changed nothing is not on the stack.
    #[test]
    fn an_empty_stroke_is_dropped() {
        let mut history = History::new();
        history.begin("Raise terrain");
        history.end();
        assert_eq!(history.depth_done(), 0);
    }

    /// Twenty notches of the wheel are one entry. Before `begin_gesture`,
    /// scaling a doodad with control and the wheel pushed one change per
    /// event, so undoing a resize took twenty presses.
    #[test]
    fn a_run_of_one_gesture_is_one_entry() {
        let mut history = History::new();
        for step in 0..20 {
            let now = step as f64 * 0.05;
            history.begin_gesture("Scale doodad", "doodad 7 scale", now);
            history.record(
                &key(32, 48),
                [Edit::Doodad {
                    index: 3,
                    before: doodad(1024 + step * 64),
                    after: doodad(1024 + (step + 1) * 64),
                }],
            );
            history.end();
        }
        assert_eq!(history.depth_done(), 1);
        let change = history.next_undo().unwrap();
        assert_eq!(change.edits.len(), 1, "folded into one edit");
        match &change.edits[0].1 {
            // The first before and the last after, which is what makes one press
            // of undo put the placement back where the run started.
            Edit::Doodad { before, after, .. } => {
                assert_eq!(before.scale, 1024);
                assert_eq!(after.scale, 1024 + 20 * 64);
            }
            other => panic!("{other:?}"),
        }
    }

    /// …and stopping for longer than the gap starts a new one.
    #[test]
    fn a_pause_ends_a_gesture() {
        let mut history = History::new();
        for now in [0.0, 0.1, 5.0, 5.1] {
            history.begin_gesture("Scale doodad", "doodad 7 scale", now);
            history.record(&key(32, 48), [edit(70, 0.0, 1.0)]);
            history.end();
        }
        assert_eq!(history.depth_done(), 2);
    }

    /// A different subject is a different entry however close together they
    /// are. Turning a placement and then scaling it within the same tenth of a
    /// second are two actions, and folding them would make one undo do both.
    #[test]
    fn two_subjects_never_fold_into_each_other() {
        let mut history = History::new();
        history.begin_gesture("Turn doodad", "doodad 7 rotation", 0.0);
        history.record(&key(32, 48), [edit(70, 0.0, 1.0)]);
        history.end();
        history.begin_gesture("Scale doodad", "doodad 7 scale", 0.01);
        history.record(&key(32, 48), [edit(70, 1.0, 2.0)]);
        history.end();
        assert_eq!(history.depth_done(), 2);
        // …and neither does a gesture fold into a plain stroke beside it.
        history.begin("Raise terrain");
        history.record(&key(32, 48), [edit(70, 2.0, 3.0)]);
        history.end();
        history.begin_gesture("Scale doodad", "doodad 7 scale", 0.02);
        history.record(&key(32, 48), [edit(70, 3.0, 4.0)]);
        history.end();
        assert_eq!(history.depth_done(), 4);
    }

    /// A gesture is not continued across an undo. The entry on top of the
    /// stack after an undo is one the person has already stepped back past;
    /// folding new work into it would make the next undo take back both.
    #[test]
    fn an_undo_ends_the_gesture_it_took_back() {
        let mut history = History::new();
        history.begin_gesture("Scale doodad", "doodad 7 scale", 0.0);
        history.record(&key(32, 48), [edit(70, 0.0, 1.0)]);
        history.end();
        history.begin_gesture("Scale doodad", "doodad 7 scale", 0.1);
        history.record(&key(32, 48), [edit(71, 0.0, 1.0)]);
        history.end();
        assert_eq!(history.depth_done(), 1, "one gesture so far");

        history.undo();
        assert_eq!(history.depth_done(), 0);
        history.begin_gesture("Scale doodad", "doodad 7 scale", 0.15);
        history.record(&key(32, 48), [edit(72, 0.0, 1.0)]);
        history.end();
        assert_eq!(history.depth_done(), 1, "a new entry, not the undone one");
        assert!(history.next_redo().is_none(), "and the branch is cleared");
    }

    /// One placement record, differing only in the field the test is about.
    fn doodad(scale: u16) -> crate::adt::Doodad {
        crate::adt::Doodad {
            name_id: 0,
            unique_id: 7,
            position: [0.0; 3],
            rotation: [0.0; 3],
            scale,
            flags: 0,
        }
    }

    /// A new change clears the redo branch.
    #[test]
    fn acting_after_an_undo_clears_the_redo_branch() {
        let mut history = History::new();
        history.begin("one");
        history.record(&key(32, 48), [edit(70, 0.0, 1.0)]);
        history.end();

        history.undo();
        assert!(history.next_redo().is_some());

        history.begin("two");
        history.record(&key(32, 48), [edit(70, 0.0, 2.0)]);
        history.end();
        assert!(history.next_redo().is_none());
    }

    /// Undo and redo move a real tile's heights, in both directions, and only
    /// the tile they are asked about.
    #[test]
    fn a_change_reverts_and_reapplies_one_tile_at_a_time() {
        let mut tile = AdtFile {
            version: 18,
            header: [0; 16],
            textures: Vec::new(),
            models: Vec::new(),
            model_offsets: Vec::new(),
            buildings: Vec::new(),
            building_offsets: Vec::new(),
            doodads: Vec::new(),
            placements: Vec::new(),
            chunks: vec![blank_chunk()],
        };
        crate::adt::heights::set_heights(tile.chunk_mut(0).unwrap(), &[7.0; 145]);

        let mut change = Change::new("Raise terrain");
        change.absorb(
            &key(32, 48),
            Edit::Heights {
                chunk: 0,
                before: vec![3.0; 145],
                after: vec![7.0; 145],
            },
        );

        change.revert(&key(32, 48), &mut tile);
        assert_eq!(crate::adt::heights::heights(tile.chunk(0).unwrap())[0], 3.0);
        // A tile the change does not name is not touched by it.
        change.revert(&key(99, 99), &mut tile);
        assert_eq!(crate::adt::heights::heights(tile.chunk(0).unwrap())[0], 3.0);
        change.apply(&key(32, 48), &mut tile);
        assert_eq!(crate::adt::heights::heights(tile.chunk(0).unwrap())[0], 7.0);
    }

    /// A map chunk with a heights region and nothing else, for the test above.
    fn blank_chunk() -> crate::adt::MapChunk {
        let mut regions: [Option<crate::adt::SubChunk>; 9] = Default::default();
        regions[crate::adt::Region::Heights as usize] =
            Some(crate::adt::SubChunk::new(vec![0u8; 145 * 4]));
        crate::adt::MapChunk {
            header: [0; crate::adt::file::MCNK_HEADER],
            regions,
            index_extra: [0, 0],
        }
    }

    /// A drag held still for longer than the gap and then moved again is one
    /// entry while the hold is on, and a second drag of the same thing after the
    /// release is a second entry.
    #[test]
    fn a_held_gesture_survives_a_pause_and_a_second_drag_does_not_join_it() {
        let mut history = History::new();
        let subject = "creature group 7 position";
        history.hold(10.0);
        history.begin_gesture("Move 3 creatures", subject, 10.0);
        history.record_server_cell(server_cell("position_x", Some("1"), Some("2")));
        history.end();
        // Three seconds without a write, and the drag goes on.
        history.begin_gesture("Move 3 creatures", subject, 13.0);
        history.record_server_cell(server_cell("position_x", Some("2"), Some("3")));
        history.end();
        assert_eq!(history.depth_done(), 1, "one drag with a pause in it");
        history.release();

        // The next drag, with its own hold, starts an entry of its own.
        history.hold(20.0);
        history.begin_gesture("Move 3 creatures", subject, 20.0);
        history.record_server_cell(server_cell("position_x", Some("3"), Some("4")));
        history.end();
        history.release();
        assert_eq!(history.depth_done(), 2, "two drags");
    }

    /// Two moves of one index on either side of a renumbering stay two edits,
    /// and the entry undoes and redoes exactly.
    ///
    /// The group case: a member is moved, the rows of the tile are renumbered
    /// when another member is put in the tile its origin is in, and a later
    /// write names the same index, which now holds a different placement.
    #[test]
    fn an_index_edit_does_not_fold_across_a_renumbering() {
        use crate::adt::place::Doodad;
        use crate::ops::Placements;
        let mut tile = crate::adt::blank::blank_tile(32, 48, "a.blp", 0.0, 0);
        let placed = |unique_id: u32, x: f32| Doodad {
            name_id: 0,
            unique_id,
            position: [x, 0.0, 0.0],
            rotation: [0.0; 3],
            scale: 1024,
            flags: 0,
        };
        tile.name_model("tree.m2");
        tile.add_doodad(placed(1, 100.0), 5.0);
        tile.add_doodad(placed(2, 200.0), 5.0);
        let start = Placements::capture(&tile);

        let mut history = History::new();
        history.begin("Move 2 doodads");
        let first = Edit::move_doodad(&tile, 0, placed(1, 110.0)).unwrap();
        first.apply(&mut tile);
        history.record(&key(32, 48), [first]);
        let before = Placements::capture(&tile);
        tile.remove_doodad(0);
        history.record(
            &key(32, 48),
            [Edit::Placements {
                before: Box::new(before),
                after: Box::new(Placements::capture(&tile)),
            }],
        );
        // Index 0 is the second placement now.
        let second = Edit::move_doodad(&tile, 0, placed(2, 210.0)).unwrap();
        second.apply(&mut tile);
        history.record(&key(32, 48), [second]);
        history.end();
        let end = Placements::capture(&tile);

        let change = history.undo().expect("one entry");
        assert_eq!(change.edits.len(), 3, "nothing folded across the renumbering");
        change.revert(&key(32, 48), &mut tile);
        assert!(Placements::capture(&tile) == start, "undo puts both back");
        change.apply(&key(32, 48), &mut tile);
        assert!(Placements::capture(&tile) == end, "redo puts both in");
    }
}
