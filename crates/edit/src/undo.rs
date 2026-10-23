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

/// One edit to the server's database, before and after, as text.
///
/// The one entry whose subject this crate cannot reach: a tile is a file it
/// holds, a DBC cell is a record in a file it holds, and this is a row in a
/// MySQL database this crate has no connection to. So it is addressed by
/// text: a table name, a key written as its own `column=value;column=value`
/// form, and what [`Shape`] of edit it is. What the key and the values mean is
/// the caller's (`vale-mangos` writes and reads them); this crate carries them
/// between the caller's store and the stack and reads none of them.
///
/// `before` and `after` are `Option<String>` because absent is a value. `None`
/// is the project saying nothing about the subject, which is different from a
/// claim of the empty string: undoing the first edit to a column removes the
/// column from the store rather than writing an empty value, and claiming no
/// path leaves the database's own path where claiming an empty one deletes it.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerEntry {
    pub shape: Shape,
    /// The server table, as `vale-mangos` names it: `creature_template`,
    /// `creature`, `creature_movement`, or a script table.
    pub table: String,
    /// Which row or path, as `vale_mangos::row::Key::text` writes one:
    /// `entry=68;patch=0`, `guid=10000001`, `id=12345`.
    pub key: String,
    /// What the project's store held before, or `None` for nothing.
    pub before: Option<String>,
    /// What it holds after, or `None` for nothing.
    pub after: Option<String>,
}

/// What a [`ServerEntry`] is an edit of, which decides how the caller's store
/// applies it and what two entries fold on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// One column of one row, set to one value. The column is named here.
    Column(String),
    /// The project's whole claim on one row: created, removed, or nothing.
    /// Creating a spawn sets no column of a row that exists, and removing one
    /// sets none at all, so the claim is the unit. The text is what
    /// `vale_mangos::row::Edits::row_line` writes.
    Row,
    /// Every row under one key, replaced together: a waypoint path, or a
    /// script under a script table's name. vmangos renumbers a path's points
    /// at each start, so a per-row edit would address rows that move, and a
    /// script's rows have no key of their own.
    Path,
}

impl ServerEntry {
    /// One column of one row, before and after.
    pub fn column(
        table: impl Into<String>,
        key: impl Into<String>,
        column: impl Into<String>,
        before: Option<String>,
        after: Option<String>,
    ) -> ServerEntry {
        ServerEntry {
            shape: Shape::Column(column.into()),
            table: table.into(),
            key: key.into(),
            before,
            after,
        }
    }

    /// The project's claim on one whole row, before and after.
    pub fn row(
        table: impl Into<String>,
        key: impl Into<String>,
        before: Option<String>,
        after: Option<String>,
    ) -> ServerEntry {
        ServerEntry {
            shape: Shape::Row,
            table: table.into(),
            key: key.into(),
            before,
            after,
        }
    }

    /// One whole path or script, before and after.
    pub fn path(
        table: impl Into<String>,
        key: impl Into<String>,
        before: Option<String>,
        after: Option<String>,
    ) -> ServerEntry {
        ServerEntry {
            shape: Shape::Path,
            table: table.into(),
            key: key.into(),
            before,
            after,
        }
    }

    /// Whether two entries edit the same thing: the same shape, table and key,
    /// and for a column the same column. What [`Change::absorb_server`] folds
    /// on.
    pub(crate) fn names_the_same_subject_as(&self, other: &ServerEntry) -> bool {
        self.shape == other.shape && self.table == other.table && self.key == other.key
    }

    /// This entry reversed, for an undo.
    pub fn reversed(&self) -> ServerEntry {
        ServerEntry {
            shape: self.shape.clone(),
            table: self.table.clone(),
            key: self.key.clone(),
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

/// One thing a [`Change`] did.
///
/// The variants apply to different objects: a tile edit to an `AdtFile`, a
/// cell or row to a `DbcFile`, and a server entry to the caller's own store.
/// So each is applied through its own method on [`Change`] ([`Change::apply`]
/// for a tile, [`Change::apply_table`] for a table, [`Change::server_steps`]
/// for the server store), and every one of those reads the list in the order
/// [`Change::in_order`] gives.
#[derive(Debug, Clone)]
pub enum Entry {
    /// A tile edit and the tile it belongs to.
    Tile(TileKey, Edit),
    /// A field edit and its table, named as the table is (`"Spell"`).
    Cell(String, Cell),
    /// A whole record added or removed, and its table.
    Row(String, Row),
    /// An edit to the server's database.
    Server(ServerEntry),
}

/// Where an entry falls in the order a change is applied in. See
/// [`Change::in_order`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Entries that add or remove a record: a DBC row and a server row claim.
    Records,
    /// Every other entry.
    Contents,
}

impl Entry {
    fn stage(&self) -> Stage {
        match self {
            Entry::Row(..) => Stage::Records,
            Entry::Server(entry) if entry.shape == Shape::Row => Stage::Records,
            _ => Stage::Contents,
        }
    }

    /// The client table this entry edits, if it edits one.
    fn table(&self) -> Option<&str> {
        match self {
            Entry::Cell(table, _) | Entry::Row(table, _) => Some(table),
            _ => None,
        }
    }
}

/// One entry on the stack: everything one action did, to tiles, client tables
/// and server rows.
///
/// What the action did is one list of [`Entry`] in the order it was done, and
/// there is one stack: `Ctrl+Z` undoes the last thing done, whether that was a
/// wall, a spell's name or a creature's level. The order the list is applied
/// and reverted in is one rule, [`Self::in_order`], which every apply and
/// revert path reads.
#[derive(Debug, Clone)]
pub struct Change {
    /// What to call it in an interface. "Raise terrain", "Move doodad".
    pub label: String,
    /// What it did, in the order it was done. Applied and reverted through
    /// [`Self::in_order`], not in this order.
    entries: Vec<Entry>,
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
            entries: Vec::new(),
            gesture: None,
            at: 0.0,
        }
    }

    /// The entries in the order they are applied (`forward`) or reverted.
    ///
    /// Applied: every entry that adds or removes a record (an [`Entry::Row`]
    /// and a server [`Shape::Row`]) in the order made, then every other entry
    /// in the order made. Reverted: the exact reverse, so the other entries
    /// last first and then the record entries last first.
    ///
    /// Records come first because a change that adds a row and then writes
    /// into it has to have the row before the write, and has to take the write
    /// back before the row goes. A DBC removal moves every index after it, so a
    /// cell recorded after one is addressed against the table as the removal
    /// left it. Within each stage the order made is kept, because two edits to
    /// one chunk only invert in the order they were made, and a placement edit
    /// names an index that an [`Edit::Placements`] edit before it can move.
    ///
    /// Each target filters this to its own entries, so the order holds per
    /// tile, per table and for the server store, and does not depend on the
    /// order the caller recorded rows and cells in.
    pub fn in_order(&self, forward: bool) -> impl Iterator<Item = &Entry> + '_ {
        let stages = match forward {
            true => [Stage::Records, Stage::Contents],
            false => [Stage::Contents, Stage::Records],
        };
        let count = self.entries.len();
        stages.into_iter().flat_map(move |stage| {
            (0..count)
                .map(move |at| match forward {
                    true => at,
                    false => count - 1 - at,
                })
                .map(move |at| &self.entries[at])
                .filter(move |entry| entry.stage() == stage)
        })
    }

    /// Add a field edit, folding it into one already here about the same field
    /// of the same record of the same table.
    ///
    /// The folding is what makes typing into a name box one undo step rather
    /// than one per keystroke: the first `before` stands and the last `after`
    /// wins, which is the same rule [`Self::absorb`] applies to a held brush.
    pub(crate) fn absorb_cell(&mut self, table: &str, cell: Cell) {
        for entry in self.entries.iter_mut() {
            if let Entry::Cell(had, existing) = entry {
                if had == table && existing.record == cell.record && existing.field == cell.field {
                    existing.after = cell.after;
                    return;
                }
            }
        }
        self.entries.push(Entry::Cell(table.to_string(), cell));
    }

    /// Add a whole record, added or removed. Never folded: two rows are two
    /// rows.
    pub(crate) fn absorb_row(&mut self, table: &str, row: Row) {
        self.entries.push(Entry::Row(table.to_string(), row));
    }

    /// Add a server entry, folding it into one already here about the same
    /// subject (see [`ServerEntry::names_the_same_subject_as`]).
    ///
    /// [`Self::absorb_cell`]'s rule for its reason: the first `before` stands
    /// and the last `after` wins. Typing into a name box is one undo step, a
    /// node dragged across a field writes its path on every frame and is one
    /// entry, and a row created and then removed inside one gesture is one
    /// entry that does nothing.
    pub(crate) fn absorb_server(&mut self, server: ServerEntry) {
        for entry in self.entries.iter_mut() {
            if let Entry::Server(existing) = entry {
                if existing.names_the_same_subject_as(&server) {
                    existing.after = server.after;
                    return;
                }
            }
        }
        self.entries.push(Entry::Server(server));
    }

    /// The tables this change touched, in the order they were first touched.
    pub fn tables(&self) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for table in self.entries.iter().filter_map(Entry::table) {
            if !found.iter().any(|had| had == table) {
                found.push(table.to_string());
            }
        }
        found
    }

    /// Put this change's edits for one table back in, in [`Self::in_order`]'s
    /// order: the rows first, then the fields.
    pub fn apply_table(&self, table: &str, into: &mut DbcFile) {
        for entry in self.in_order(true).filter(|entry| entry.table() == Some(table)) {
            match entry {
                Entry::Row(_, row) => row.apply(into),
                Entry::Cell(_, cell) => cell.apply(into),
                _ => {}
            }
        }
    }

    /// …and take them back out, in the reverse: the fields first, then the
    /// rows.
    pub fn revert_table(&self, table: &str, into: &mut DbcFile) {
        for entry in self.in_order(false).filter(|entry| entry.table() == Some(table)) {
            match entry {
                Entry::Row(_, row) => row.revert(into),
                Entry::Cell(_, cell) => cell.revert(into),
                _ => {}
            }
        }
    }

    /// The records this change touched in one table, for a panel that has one
    /// of them open.
    pub fn records(&self, table: &str) -> Vec<usize> {
        let mut found: Vec<usize> = Vec::new();
        for entry in &self.entries {
            if let Entry::Cell(had, cell) = entry {
                if had == table && !found.contains(&cell.record) {
                    found.push(cell.record);
                }
            }
        }
        found
    }

    /// The server entries in [`Self::in_order`]'s order, each as the edit the
    /// caller's store makes: as recorded going forward, reversed going back.
    pub fn server_steps(&self, forward: bool) -> impl Iterator<Item = ServerEntry> + '_ {
        self.in_order(forward).filter_map(move |entry| match entry {
            Entry::Server(server) => Some(match forward {
                true => server.clone(),
                false => server.reversed(),
            }),
            _ => None,
        })
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
    pub(crate) fn absorb(&mut self, tile: &TileKey, edit: Edit) {
        for entry in self.entries.iter_mut().rev() {
            let Entry::Tile(had, existing) = entry else {
                continue;
            };
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
        self.entries.push(Entry::Tile(tile.clone(), edit));
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drop everything this change says about one table, leaving what it says
    /// about other tables, tiles and server rows. `true` when anything was
    /// dropped. See [`History::forget_table`].
    pub fn forget_table(&mut self, table: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.table() != Some(table));
        before != self.entries.len()
    }

    /// The tile edits on one tile, in [`Self::in_order`]'s order. Every tile
    /// edit is in the same stage, so forward this is the order they were made.
    fn on_tile<'a>(&'a self, tile: &'a TileKey, forward: bool) -> impl Iterator<Item = &'a Edit> + 'a {
        self.in_order(forward).filter_map(move |entry| match entry {
            Entry::Tile(had, edit) if had == tile => Some(edit),
            _ => None,
        })
    }

    /// The tiles this change touched, in the order they were first touched.
    pub fn tiles(&self) -> Vec<TileKey> {
        let mut found: Vec<TileKey> = Vec::new();
        for entry in &self.entries {
            if let Entry::Tile(tile, _) = entry {
                if !found.contains(tile) {
                    found.push(tile.clone());
                }
            }
        }
        found
    }

    /// The indices `pick` names among the edits on one tile, each once, in the
    /// order first named.
    fn indices(&self, tile: &TileKey, pick: impl Fn(&Edit) -> Option<usize>) -> Vec<usize> {
        let mut found: Vec<usize> = Vec::new();
        for index in self.on_tile(tile, true).filter_map(pick) {
            if !found.contains(&index) {
                found.push(index);
            }
        }
        found
    }

    /// The chunks it touched on one tile, for a caller rebuilding meshes.
    pub fn chunks(&self, tile: &TileKey) -> Vec<usize> {
        self.indices(tile, Edit::chunk)
    }

    /// …and the `MDDF` entries it moved on one tile, for a caller that has them
    /// drawn — the placement counterpart of [`Self::chunks`].
    pub fn placements(&self, tile: &TileKey) -> Vec<usize> {
        self.indices(tile, Edit::placement)
    }

    /// …and the `MODF` entries it moved, which is a separate list from
    /// [`Self::placements`] because `MDDF` and `MODF` are numbered separately.
    pub fn buildings(&self, tile: &TileKey) -> Vec<usize> {
        self.indices(tile, Edit::building)
    }

    /// …and the chunks it repainted on one tile, for a caller that has the
    /// blend maps on the GPU. A different set from [`Self::chunks`] because a
    /// repaint takes a different route to the screen: see [`Edit::painted`].
    pub fn painted(&self, tile: &TileKey) -> Vec<usize> {
        self.indices(tile, Edit::painted)
    }

    /// Whether this change gave a chunk on one tile a texture it did not have,
    /// which is the half of a repaint that the atlas cannot carry — see
    /// [`Edit::changes_the_texture_set`].
    pub fn changes_the_texture_set(&self, tile: &TileKey) -> bool {
        self.on_tile(tile, true).any(Edit::changes_the_texture_set)
    }

    /// Whether this change renumbers a tile's placement lists — see
    /// [`Edit::renumbers_placements`]. A caller holding an index into them has
    /// to let go of it rather than reconcile.
    pub(crate) fn renumbers_placements(&self, tile: &TileKey) -> bool {
        self.on_tile(tile, true).any(Edit::renumbers_placements)
    }

    /// …and whether it changes the ground mesh rather than anything the mesh
    /// holds, so the tile has to be read again — see [`Edit::remeshes`]. An undo
    /// of a hole is the case: nothing on screen can be patched back.
    pub fn remeshes(&self, tile: &TileKey) -> bool {
        self.on_tile(tile, true).any(Edit::remeshes)
    }

    /// What applying or reverting this change leaves out of date on screen:
    /// for each tile it names, in the order they were first touched, the
    /// chunks, placements and blend maps to patch and whether the tile has to
    /// be read again; and the client tables it edits. The answer is the same in
    /// both directions, because an undo and a redo touch the same things.
    ///
    /// A tile whose placement lists are renumbered lists no placement or
    /// building indices, because no index survives the renumbering, and is
    /// marked to be read again instead. A tile with a hole, liquid or path-list
    /// edit is also marked to be read again; see [`Edit::remeshes`].
    pub fn invalidates(&self) -> Invalidated {
        let tiles = self
            .tiles()
            .into_iter()
            .map(|tile| {
                let renumbers = self.renumbers_placements(&tile);
                TileRedraw {
                    chunks: self.chunks(&tile),
                    placements: match renumbers {
                        true => Vec::new(),
                        false => self.placements(&tile),
                    },
                    buildings: match renumbers {
                        true => Vec::new(),
                        false => self.buildings(&tile),
                    },
                    painted: self.painted(&tile),
                    regroup: self.changes_the_texture_set(&tile),
                    reread: renumbers || self.remeshes(&tile),
                    tile,
                }
            })
            .collect();
        Invalidated {
            tiles,
            tables: self.tables(),
        }
    }

    /// Put this change's edits for one tile back in.
    pub fn apply(&self, tile: &TileKey, into: &mut AdtFile) {
        for edit in self.on_tile(tile, true) {
            edit.apply(into);
        }
    }

    /// …and take them back out. In reverse, because two edits about one chunk
    /// only invert in the order they were made.
    pub fn revert(&self, tile: &TileKey, into: &mut AdtFile) {
        for edit in self.on_tile(tile, false) {
            edit.revert(into);
        }
    }
}

/// What a [`Change`] leaves out of date once it is applied or reverted, as
/// [`Change::invalidates`] answers it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Invalidated {
    /// One entry per tile the change names, in the order it first touched them.
    pub tiles: Vec<TileRedraw>,
    /// The client tables it edits, by their bare name (`"Spell"`), in the order
    /// it first touched them.
    pub tables: Vec<String>,
}

/// What one tile needs redrawn after a change to it.
#[derive(Debug, Clone, PartialEq)]
pub struct TileRedraw {
    pub tile: TileKey,
    /// The chunks whose ground vertices changed: see [`Edit::chunk`].
    pub chunks: Vec<usize>,
    /// The `MDDF` entries moved, turned or scaled. Empty when `reread` is set
    /// because the lists were renumbered.
    pub placements: Vec<usize>,
    /// …and the `MODF` entries, numbered separately from `MDDF`.
    pub buildings: Vec<usize>,
    /// The chunks whose blend maps changed: see [`Edit::painted`].
    pub painted: Vec<usize>,
    /// A chunk carries a different set of textures, so the tile's draw groups
    /// have to be cut again: see [`Edit::changes_the_texture_set`].
    pub regroup: bool,
    /// The tile has to be read again, because its placement lists were
    /// renumbered or its mesh changed: see [`Edit::renumbers_placements`] and
    /// [`Edit::remeshes`].
    pub reread: bool,
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

    /// Fold what `fold` adds into the stroke being held, or outside one make
    /// it an entry of its own under `label`, so a caller that forgets to open
    /// a stroke still gets a working undo. An entry that ends up empty is not
    /// pushed.
    fn take(&mut self, label: &str, fold: impl FnOnce(&mut Change)) {
        match self.open.as_mut() {
            Some(change) => fold(change),
            None => {
                let mut change = Change::new(label);
                fold(&mut change);
                if !change.is_empty() {
                    self.push(change);
                }
            }
        }
    }

    /// Record what has already been applied to a tile.
    pub fn record(&mut self, tile: &TileKey, edits: impl IntoIterator<Item = Edit>) {
        self.take("Edit", |change| {
            for edit in edits {
                change.absorb(tile, edit);
            }
        });
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
        self.take("Edit", |change| change.absorb_cell(table, cell));
    }

    /// Record an edit to the server's database, already applied to the
    /// caller's own store.
    ///
    /// [`History::record_cell`]'s counterpart for the server, with the same
    /// rules: inside a gesture it folds, outside one it is its own entry, and
    /// an entry that moved nothing is dropped rather than making an undo step
    /// that does nothing, because a form writes its value back on every frame
    /// it is drawn.
    pub fn record_server(&mut self, entry: ServerEntry) {
        if entry.before == entry.after {
            return;
        }
        self.take("Edit", |change| change.absorb_server(entry));
    }

    /// Record a record added or removed, already applied to the table.
    ///
    /// Inside a stroke it joins the stroke; outside one it is an entry of its
    /// own. See [`Change::in_order`] for the order it is applied in.
    pub fn record_row(&mut self, table: &str, row: Row) {
        let label = match row.added {
            true => "Add row",
            false => "Remove row",
        };
        self.take(label, |change| change.absorb_row(table, row));
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
    #[cfg(test)]
    pub(crate) fn forget_server_rows(&mut self) -> usize {
        self.forget_server_rows_where(|_| true, true)
    }

    /// …and the same for one subject's tables, which is what a discard is.
    ///
    /// The store holds every subject's rows together and so does the stack, so
    /// a discard that forgot both outright would take the panel next door's
    /// undo with it. `wanted` is asked the table's name, exactly as the store's
    /// own `forget_where` is. A [`Shape::Path`] entry goes by its table too:
    /// the waypoint entries with the creature tables, the script entries with
    /// the script tables. `paths` clears every path entry as well, for a
    /// caller that has emptied the path store outright.
    pub fn forget_server_rows_where(
        &mut self,
        mut wanted: impl FnMut(&str) -> bool,
        paths: bool,
    ) -> usize {
        self.strip(|change| {
            change.entries.retain(|entry| match entry {
                Entry::Server(server) => {
                    !(wanted(&server.table) || (paths && server.shape == Shape::Path))
                }
                _ => true,
            })
        })
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
        self.strip(|change| {
            change.forget_table(table);
        })
    }

    /// Run `strip` over every entry on both branches and the stroke being
    /// held, and drop each entry it leaves empty. Returns how many were
    /// dropped.
    fn strip(&mut self, mut strip: impl FnMut(&mut Change)) -> usize {
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
            .any(|change| {
                change
                    .entries
                    .iter()
                    .any(|entry| matches!(entry, Entry::Tile(key, _) if key == tile))
            })
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

    fn server_cell(column: &str, before: Option<&str>, after: Option<&str>) -> ServerEntry {
        ServerEntry::column(
            "creature_template",
            "entry=68;patch=0",
            column,
            before.map(str::to_string),
            after.map(str::to_string),
        )
    }

    fn server_path(before: Option<&str>, after: Option<&str>) -> ServerEntry {
        ServerEntry::path(
            "creature_movement",
            "id=12345",
            before.map(str::to_string),
            after.map(str::to_string),
        )
    }

    /// The server entries of one shape, in the order made.
    fn server_of(change: &Change, shape: fn(&Shape) -> bool) -> Vec<&ServerEntry> {
        change
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Server(server) if shape(&server.shape) => Some(server),
                _ => None,
            })
            .collect()
    }

    fn columns(change: &Change) -> Vec<&ServerEntry> {
        server_of(change, |shape| matches!(shape, Shape::Column(_)))
    }

    fn paths(change: &Change) -> Vec<&ServerEntry> {
        server_of(change, |shape| *shape == Shape::Path)
    }

    fn rows(change: &Change) -> Vec<&ServerEntry> {
        server_of(change, |shape| *shape == Shape::Row)
    }

    /// The tile edits, in the order made.
    fn tile_edits(change: &Change) -> Vec<&Edit> {
        change
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Tile(_, edit) => Some(edit),
                _ => None,
            })
            .collect()
    }

    /// A change that carries only a path is not an empty change.
    ///
    /// This guards a regression from the waypoint feature: a path edit makes
    /// a `Change` with no column entries at all, and the session stepped its
    /// paths after an `is_empty` return on the columns. The entry was on
    /// the stack and the Undo button was lit, but pressing it did nothing at
    /// all. Anything that walks a change by asking whether one shape of entry
    /// is empty is wrong for the same reason.
    #[test]
    fn a_change_carrying_only_a_path_is_not_empty() {
        let mut change = Change::new("Add waypoint");
        assert!(change.is_empty());
        change.absorb_server(server_path(None, Some("1,2,3,100,0,0,0,0")));
        assert!(!change.is_empty(), "a path-only change must reach every step of an undo");
        assert!(columns(&change).is_empty(), "and it has no cells, which is the trap");
    }

    /// A path edit outside a gesture is its own entry, and inside one it folds
    /// — which is what makes a drag across a field one press of undo rather
    /// than one per frame.
    #[test]
    fn server_paths_fold_within_a_gesture() {
        let mut history = History::new();
        history.begin_gesture("Move waypoint", "creature_movement 12345 path", 1.0);
        history.record_server(server_path(Some("a"), Some("b")));
        history.record_server(server_path(Some("b"), Some("c")));
        history.end();
        let change = history.next_undo().expect("one entry");
        assert_eq!(paths(&change).len(), 1);
        assert_eq!(paths(&change)[0].before.as_deref(), Some("a"));
        assert_eq!(paths(&change)[0].after.as_deref(), Some("c"));
    }

    /// A path that moved nothing makes no entry, because the window writes
    /// what it holds on every frame it is drawn.
    #[test]
    fn a_path_that_did_not_change_makes_no_entry() {
        let mut history = History::new();
        history.record_server(server_path(Some("a"), Some("a")));
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
        history.record_server(server_cell("name", Some("'A'"), Some("'AB'")));
        history.record_server(server_cell("name", Some("'AB'"), Some("'ABC'")));
        history.end();

        let change = history.undo().expect("one entry");
        assert_eq!(columns(&change).len(), 1);
        assert_eq!(columns(&change)[0].before.as_deref(), Some("'A'"));
        assert_eq!(columns(&change)[0].after.as_deref(), Some("'ABC'"));
        assert!(history.undo().is_none(), "the two keystrokes were one entry");
    }

    fn server_row(before: Option<&str>, after: Option<&str>) -> ServerEntry {
        ServerEntry::row(
            "creature",
            "guid=10000001",
            before.map(str::to_string),
            after.map(str::to_string),
        )
    }

    /// A change carrying only a created row is not empty, which is the same
    /// trap the paths have: creating a spawn makes a `Change` with no
    /// column entries at all, and anything that asks one list whether it is
    /// empty reads the whole entry as nothing and drops it. The entry would be
    /// on no stack and the Undo button would be grey.
    #[test]
    fn a_change_carrying_only_a_created_row_is_not_empty() {
        let mut history = History::new();
        history.begin_gesture("New spawn", "creature 10000001", 0.0);
        history.record_server(server_row(None, Some("insert\tid=68")));
        history.end();

        let change = history.next_undo().expect("the creation is on the stack");
        assert!(!change.is_empty());
        assert!(columns(&change).is_empty(), "and it has no cells, which is the trap");
        assert_eq!(rows(&change).len(), 1);
    }

    /// A row claimed and then given up inside one gesture folds to one entry,
    /// and the first `before` stands.
    #[test]
    fn server_rows_fold_within_a_gesture() {
        let mut history = History::new();
        history.begin_gesture("New spawn", "creature 10000001", 0.0);
        history.record_server(server_row(None, Some("insert\tid=68")));
        history.record_server(server_row(Some("insert\tid=68"), Some("insert\tid=69")));
        history.end();

        let change = history.undo().expect("one entry");
        assert_eq!(rows(&change).len(), 1);
        assert_eq!(rows(&change)[0].before, None);
        assert_eq!(rows(&change)[0].after.as_deref(), Some("insert\tid=69"));
        // …and reversed is what an undo applies: the claim goes away again.
        assert_eq!(rows(&change)[0].reversed().after, None);
    }

    /// A claim that moved nothing is not an entry, on the cells' own rule:
    /// a panel redrawing every frame must not fill the stack.
    #[test]
    fn a_server_row_that_moved_nothing_is_dropped() {
        let mut history = History::new();
        history.record_server(server_row(Some("delete"), Some("delete")));
        history.record_server(server_row(None, None));
        assert!(history.next_undo().is_none());
    }

    /// Discarding the project's server edits drops the row claims with
    /// everything else, or an undo would put back a row the project has stopped
    /// claiming.
    #[test]
    fn forgetting_the_server_rows_takes_the_claims_too() {
        let mut history = History::new();
        history.record_server(server_row(None, Some("insert\tid=68")));
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
        history.record_server(server_row(None, Some("insert\tid=68")));
        history.record_server(ServerEntry::column(
            "item_template",
            "entry=19019;patch=10",
            "Quality",
            Some("3".into()),
            Some("4".into()),
        ));

        let creatures = |table: &str| table == "creature" || table == "creature_template";
        assert_eq!(history.forget_server_rows_where(creatures, true), 1);

        let change = history.next_undo().expect("the item edit is still on the stack");
        assert!(rows(&change).is_empty(), "the creature claim went");
        assert_eq!(columns(&change).len(), 1);
        assert_eq!(columns(&change)[0].table, "item_template");
    }

    /// An edit is undoable the moment it is made.
    ///
    /// `begin_gesture` leaves the change open, and an open change is not on
    /// the stack: `next_undo` reads `done`, and a panel greys its Undo button
    /// off that. A caller that opened a gesture and did not close it left the
    /// last edit made unreachable: pressing Undo either did nothing or took
    /// back the edit before it, which was reported as a field that would not
    /// go back to what it had been. `EditSession::set_server_edit` ends its
    /// gesture for exactly this reason, as `tables::fields::set_fields` already did.
    #[test]
    fn one_edit_is_undoable_without_waiting_for_a_second() {
        let mut history = History::new();
        history.begin_gesture("Edit creature", "creature_template name", 0.0);
        history.record_server(server_cell("name", None, Some("'A'")));
        history.end();

        assert!(history.next_undo().is_some(), "the only edit made is on the stack");
        let change = history.undo().expect("one entry");
        assert_eq!(columns(&change).len(), 1);
        assert_eq!(columns(&change)[0].after.as_deref(), Some("'A'"));
    }

    /// …and two edits to different columns of the same row are two cells on
    /// one entry, not one cell. A move writes three columns and comes back in
    /// one press; it must put all three back.
    #[test]
    fn a_move_is_three_cells_on_one_entry() {
        let mut history = History::new();
        history.begin_gesture("Move creature", "creature 19272 position", 0.0);
        for column in ["position_x", "position_y", "position_z"] {
            history.record_server(server_cell(column, Some("1"), Some("2")));
        }
        history.end();

        let change = history.undo().expect("one entry");
        assert_eq!(columns(&change).len(), 3);
        assert_eq!(change.label, "Move creature");
    }

    /// A cell that moved nothing is not an entry. A form writes its value
    /// back on every frame it is drawn, so without this, opening a panel would
    /// fill the undo stack with entries that do nothing.
    #[test]
    fn a_server_cell_that_moved_nothing_is_dropped() {
        let mut history = History::new();
        history.record_server(server_cell("name", Some("'A'"), Some("'A'")));
        assert!(history.undo().is_none());
        history.record_server(server_cell("name", None, None));
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
        change.absorb_server(server_cell("name", None, Some("'A'")));
        assert!(!change.is_empty());
    }

    /// Two cells about different rows of the same table never fold, however
    /// close together they arrive: they are two creatures.
    #[test]
    fn two_rows_are_two_cells() {
        let mut change = Change::new("Edit creature");
        change.absorb_server(server_cell("name", None, Some("'A'")));
        let mut other = server_cell("name", None, Some("'B'"));
        other.key = "entry=69;patch=0".into();
        change.absorb_server(other);
        assert_eq!(columns(&change).len(), 2);
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
        assert_eq!(tile_edits(&change).len(), 1);
        match tile_edits(&change)[0] {
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
        assert_eq!(tile_edits(&change).len(), 2);
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
        assert_eq!(tile_edits(&change).len(), 2, "one edit per tile, folded");
        assert_eq!(change.tiles(), vec![key(32, 48), key(33, 48)]);
        // The fold happened on the right one: the first tile's edit spans both
        // of its steps and the second tile's is untouched.
        match tile_edits(&change)[0] {
            Edit::Heights { before, after, .. } => {
                assert_eq!((before[0], after[0]), (0.0, 2.0));
            }
            other => panic!("{other:?}"),
        }
        match tile_edits(&change)[1] {
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
        assert_eq!(tile_edits(&change).len(), 1, "folded into one edit");
        match tile_edits(&change)[0] {
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
        history.record_server(server_cell("position_x", Some("1"), Some("2")));
        history.end();
        // Three seconds without a write, and the drag goes on.
        history.begin_gesture("Move 3 creatures", subject, 13.0);
        history.record_server(server_cell("position_x", Some("2"), Some("3")));
        history.end();
        assert_eq!(history.depth_done(), 1, "one drag with a pause in it");
        history.release();

        // The next drag, with its own hold, starts an entry of its own.
        history.hold(20.0);
        history.begin_gesture("Move 3 creatures", subject, 20.0);
        history.record_server(server_cell("position_x", Some("3"), Some("4")));
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
        assert_eq!(tile_edits(&change).len(), 3, "nothing folded across the renumbering");
        change.revert(&key(32, 48), &mut tile);
        assert!(Placements::capture(&tile) == start, "undo puts both back");
        change.apply(&key(32, 48), &mut tile);
        assert!(Placements::capture(&tile) == end, "redo puts both in");
    }

    /// `invalidates` names, per tile and in the order the tiles were first
    /// touched, the chunks, placements and painted chunks to patch, and the
    /// tables the change edits.
    #[test]
    fn invalidates_names_what_each_tile_needs_redrawn() {
        let tile = crate::adt::blank::blank_tile(32, 48, "a.blp", 0.0, 0);
        let paint = crate::ops::ChunkPaint::capture(&tile, 4);
        let mut change = Change::new("Several things");
        change.absorb(&key(33, 48), edit(5, 0.0, 1.0));
        change.absorb(&key(32, 48), edit(3, 0.0, 1.0));
        change.absorb(&key(32, 48), Edit::Doodad { index: 2, before: doodad(1024), after: doodad(2048) });
        change.absorb(
            &key(32, 48),
            Edit::Paint { chunk: 4, before: Box::new(paint.clone()), after: Box::new(paint) },
        );
        change.absorb_cell("Spell", crate::dbc::Cell { record: 1, field: 2, before: 0, after: 1 });

        let invalidated = change.invalidates();
        assert_eq!(invalidated.tables, vec!["Spell".to_string()]);
        let tiles: Vec<&TileKey> = invalidated.tiles.iter().map(|redraw| &redraw.tile).collect();
        assert_eq!(tiles, vec![&key(33, 48), &key(32, 48)]);
        let first = &invalidated.tiles[0];
        assert_eq!(first.chunks, vec![5]);
        assert!(first.placements.is_empty() && first.painted.is_empty());
        assert!(!first.regroup && !first.reread);
        let second = &invalidated.tiles[1];
        assert_eq!(second.chunks, vec![3], "a paint edit moves no vertex");
        assert_eq!(second.placements, vec![2]);
        assert!(second.buildings.is_empty());
        assert_eq!(second.painted, vec![4]);
        assert!(!second.regroup, "the same layers on both sides keep the texture set");
        assert!(!second.reread);
    }

    /// A renumbering of the placement lists names no index, because none
    /// survives it, and marks the tile to be read again.
    #[test]
    fn a_renumbering_rereads_the_tile_instead_of_naming_indices() {
        use crate::ops::Placements;
        let tile = crate::adt::blank::blank_tile(32, 48, "a.blp", 0.0, 0);
        let mut change = Change::new("Remove doodad");
        change.absorb(&key(32, 48), Edit::Doodad { index: 2, before: doodad(1024), after: doodad(2048) });
        change.absorb(
            &key(32, 48),
            Edit::Placements {
                before: Box::new(Placements::capture(&tile)),
                after: Box::new(Placements::capture(&tile)),
            },
        );
        let redraw = &change.invalidates().tiles[0];
        assert!(redraw.placements.is_empty() && redraw.buildings.is_empty());
        assert!(redraw.reread);
    }

    /// A hole changes the mesh, so the tile is read again, and a placement
    /// moved in the same change is still named.
    #[test]
    fn a_hole_rereads_the_tile_and_keeps_its_placements() {
        let mut change = Change::new("Cut hole");
        change.absorb(&key(32, 48), Edit::Holes { chunk: 7, before: 0, after: 1 });
        change.absorb(&key(32, 48), Edit::Doodad { index: 0, before: doodad(1024), after: doodad(2048) });
        let redraw = &change.invalidates().tiles[0];
        assert!(redraw.chunks.is_empty(), "a hole is not a moved vertex");
        assert_eq!(redraw.placements, vec![0]);
        assert!(redraw.reread);
        assert!(change.invalidates().tables.is_empty());
    }

    /// A table of one field, one record per value, for the test below.
    fn one_field_table(values: &[u32]) -> DbcFile {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(values.len() as u32).to_le_bytes()); // records
        out.extend_from_slice(&1u32.to_le_bytes()); // fields
        out.extend_from_slice(&4u32.to_le_bytes()); // record size
        out.extend_from_slice(&1u32.to_le_bytes()); // string block
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.push(0);
        DbcFile::parse(&out).unwrap()
    }

    /// A change holding every type of entry applies the record entries first
    /// and then the rest, each group in the order made, and reverts in exactly
    /// the reverse.
    ///
    /// The cell is recorded before the row it writes into. It is still applied
    /// after the row and reverted before it, so the table comes back byte for
    /// byte in both directions.
    #[test]
    fn a_mixed_change_applies_records_first_and_reverts_in_reverse() {
        fn name(entry: &Entry) -> &'static str {
            match entry {
                Entry::Tile(..) => "tile",
                Entry::Cell(..) => "cell",
                Entry::Row(..) => "row",
                Entry::Server(server) => match server.shape {
                    Shape::Column(_) => "server column",
                    Shape::Row => "server row",
                    Shape::Path => "server path",
                },
            }
        }

        let mut table = one_field_table(&[5]);
        let original = table.write();
        assert!(table.insert_record(1, &7u32.to_le_bytes()));
        let row = Row::added(&table, 1).unwrap();
        let cell = Cell::new(&table, 1, 0, 9).unwrap();
        cell.apply(&mut table);
        let edited = table.write();

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

        let mut change = Change::new("Everything");
        change.absorb(&key(32, 48), edit(0, 3.0, 7.0));
        change.absorb_cell("T", cell);
        change.absorb_server(server_cell("name", None, Some("'A'")));
        change.absorb_row("T", row);
        change.absorb_server(server_row(None, Some("insert	id=68")));
        change.absorb_server(server_path(None, Some("1,2,3,100,0,0,0,0")));
        change.absorb(&key(33, 48), edit(0, 1.0, 2.0));

        let forward: Vec<&str> = change.in_order(true).map(name).collect();
        assert_eq!(
            forward,
            vec!["row", "server row", "tile", "cell", "server column", "server path", "tile"]
        );
        let backward: Vec<&str> = change.in_order(false).map(name).collect();
        let mut reversed = forward.clone();
        reversed.reverse();
        assert_eq!(backward, reversed);

        // The server store gets its entries in the same order, each reversed
        // going back.
        let column = Shape::Column("name".into());
        let ahead: Vec<Shape> = change.server_steps(true).map(|step| step.shape).collect();
        assert_eq!(ahead, vec![Shape::Row, column.clone(), Shape::Path]);
        let back: Vec<ServerEntry> = change.server_steps(false).collect();
        let shapes: Vec<Shape> = back.iter().map(|step| step.shape.clone()).collect();
        assert_eq!(shapes, vec![Shape::Path, column, Shape::Row]);
        assert!(back.iter().all(|step| step.before.is_some() && step.after.is_none()));

        change.revert_table("T", &mut table);
        assert_eq!(table.write(), original, "the cell came out before the row");
        change.apply_table("T", &mut table);
        assert_eq!(table.write(), edited, "the row went in before the cell");

        change.revert(&key(32, 48), &mut tile);
        assert_eq!(crate::adt::heights::heights(tile.chunk(0).unwrap())[0], 3.0);
        change.apply(&key(32, 48), &mut tile);
        assert_eq!(crate::adt::heights::heights(tile.chunk(0).unwrap())[0], 7.0);
    }
}
