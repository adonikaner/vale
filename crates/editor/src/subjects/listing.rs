//! A server table read whole and searched in memory: the list the Creatures
//! and Game Objects workspaces draw on their left.
//!
//! The item workspace reads `item_template` the same way, with a list of its
//! own; this is the same mechanism written once over any row type that
//! implements [`Listed`], so the two workspaces that came after it do not each
//! carry a copy. It knows nothing about either subject. The row type, the query
//! and the read system are in [`super::creature_data`] and
//! [`super::gameobject_data`].
//!
//! The brief columns of every row the server would load are read on a task
//! when the workspace is first opened and again after every Apply or Put back
//! (`EditSession::database_writes`), and a search is a filter over memory.

use crate::tools::spawns::creatures::TemplateSubject;
use vale_mangos::row::{Edits, Key, Life, RowEdit};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::HashSet;

/// One row of a list: the brief columns of one row of a server table.
pub trait Listed: Clone + Send + Sync + std::fmt::Debug + 'static {
    /// The table the row is in.
    const TABLE: &'static str;
    /// The entry the project says the row has.
    fn entry(&self) -> u32;
    /// The entry the database has the row at. It differs from
    /// [`Listed::entry`] while the project moves the row and has not applied
    /// the move.
    fn read_entry(&self) -> u32;
    /// The content patch of the row the server loads.
    fn patch(&self) -> u32;
    /// Give the row the entry the project says it has.
    fn set_entry(&mut self, entry: u32);
    /// The key an edit to the row is written under.
    fn key(&self) -> Key;
    /// The name and the entry, for a heading.
    fn label(&self) -> String;
    /// The text a search is matched against, in lower case.
    fn haystack(&self) -> String;
    /// The row with the project's edits to its brief columns laid over it.
    fn with_edits(&self, edits: &Edits) -> Self;
    /// A row this project creates, read from its claim in the store.
    fn from_claim(key: &Key, row: &RowEdit) -> Option<Self>;
}

/// What one read of a whole table answered.
#[derive(Debug)]
pub struct ListRead<K> {
    pub rows: Vec<K>,
    /// The highest entry the table holds at any patch, which a new row's
    /// entry is numbered past.
    pub highest: Option<u32>,
}

/// One server table as a list.
#[derive(Debug)]
pub struct Listing<K> {
    /// Every row the server would load, as the last read found it.
    pub all: Vec<K>,
    /// The rows this project creates that the last read did not return. They
    /// are rebuilt from the store whenever it changes.
    pub created: Vec<K>,
    /// The `EditSession::server_edit_revision` [`Self::created`] and the
    /// entries of [`Self::all`] were brought up to.
    followed: Option<u64>,
    task: Option<Task<Result<ListRead<K>, String>>>,
    /// The `EditSession::database_writes` [`Self::all`] was read at. Set when
    /// a read starts, so a failed read is not repeated every frame.
    loaded: Option<u64>,
    /// Why the list is empty, when it is not that the table is.
    pub trouble: Option<String>,
    /// What is typed in the search box.
    pub query: String,
    /// The rows that match the query, as indices over [`Self::at`].
    matches: Vec<usize>,
    /// The query, edit revision and list lengths [`Self::matches`] was built
    /// for.
    built: Option<(String, u64, usize, usize)>,
    /// The open row, by entry.
    pub open: Option<u32>,
    /// The row the list last scrolled to or was clicked on. The list scrolls
    /// to the open row when it is another; see `crate::ui::widgets::theme::list_area`.
    pub revealed: Option<u32>,
}

impl<K> Default for Listing<K> {
    fn default() -> Listing<K> {
        Listing {
            all: Vec::new(),
            created: Vec::new(),
            followed: None,
            task: None,
            loaded: None,
            trouble: None,
            query: String::new(),
            matches: Vec::new(),
            built: None,
            open: None,
            revealed: None,
        }
    }
}

impl<K: Listed> Listing<K> {
    /// A list that has read `all`, for a test.
    #[cfg(test)]
    pub fn holding(all: Vec<K>) -> Listing<K> {
        Listing {
            all,
            loaded: Some(0),
            ..Listing::default()
        }
    }

    /// Whether a read is running.
    pub fn reading(&self) -> bool {
        self.task.is_some()
    }

    /// Whether the table has been read at least once.
    pub fn is_read(&self) -> bool {
        self.loaded.is_some() && self.task.is_none()
    }

    /// How many rows there are: the database's and this project's own.
    pub fn count(&self) -> usize {
        self.all.len() + self.created.len()
    }

    /// One row, by an index over both lists, the database's first.
    pub fn at(&self, index: usize) -> Option<&K> {
        match self.all.get(index) {
            Some(row) => Some(row),
            None => self.created.get(index - self.all.len()),
        }
    }

    /// One row by the entry the project gives it, from either list.
    pub fn by_entry(&self, entry: u32) -> Option<&K> {
        self.all
            .iter()
            .chain(self.created.iter())
            .find(|row| row.entry() == entry)
    }

    /// One row as it is drawn: the database's reading with the project's
    /// edits over it.
    pub fn shown(&self, index: usize, edits: &Edits) -> Option<K> {
        Some(self.at(index)?.with_edits(edits))
    }

    /// The open row, with the project's edits over it.
    pub fn open_row(&self, edits: &Edits) -> Option<K> {
        Some(self.by_entry(self.open?)?.with_edits(edits))
    }

    /// What the form is about: the open row, named as the template window
    /// names its subject.
    pub fn subject(&self, edits: &Edits) -> Option<TemplateSubject> {
        let row = self.open_row(edits)?;
        Some(TemplateSubject {
            entry: row.entry(),
            read_entry: row.read_entry(),
            patch: row.patch(),
            claim: edits.life(K::TABLE, &row.key()),
            label: row.label(),
        })
    }

    /// Whether an entry is a row of either list, by the entry the project
    /// gives it or the one the database has it at.
    pub fn holds(&self, entry: u32) -> bool {
        self.all
            .iter()
            .chain(self.created.iter())
            .any(|row| row.entry() == entry || row.read_entry() == entry)
    }

    /// The rows the query matches, as indices over [`Self::at`]. A number
    /// matches the row with that entry and any text matches a row whose
    /// [`Listed::haystack`] contains it.
    pub fn matches(&mut self, edits: &Edits, revision: u64) -> &[usize] {
        let asked = (
            self.query.trim().to_lowercase(),
            revision,
            self.all.len(),
            self.created.len(),
        );
        if self.built.as_ref() == Some(&asked) {
            return &self.matches;
        }
        let query = asked.0.clone();
        self.built = Some(asked);
        self.matches.clear();
        let by_entry: Option<u32> = query.parse().ok();
        for index in 0..self.count() {
            let Some(row) = self.at(index) else { continue };
            let hit = query.is_empty() || by_entry == Some(row.entry()) || {
                // The project's edits are laid over a row only when the
                // project touches it, which is a handful of thousands.
                match edits.touches(K::TABLE, &row.key()) {
                    true => row.with_edits(edits).haystack().contains(&query),
                    false => row.haystack().contains(&query),
                }
            };
            if hit {
                self.matches.push(index);
            }
        }
        &self.matches
    }

    /// Forget the cached search, for a caller that changed the lists rather
    /// than the query.
    pub fn forget_matches(&mut self) {
        self.built = None;
    }

    /// Bring the lists up to the store: every row read at an entry the
    /// project has moved is given the new entry, and the rows the project
    /// creates and no read returned are listed. Does nothing when the store
    /// has not changed since the last call.
    pub fn follow(&mut self, edits: &Edits, revision: u64) {
        if self.followed == Some(revision) {
            return;
        }
        self.followed = Some(revision);
        fold_moves(&mut self.all, edits);
        let read: HashSet<u32> = self.all.iter().map(Listed::entry).collect();
        self.created = edits
            .rows()
            .filter(|(table, _, row)| *table == K::TABLE && row.life == Life::Insert)
            .filter_map(|(_, key, row)| K::from_claim(key, row))
            // A created row that has been applied is in the table that was
            // read; listing it twice would draw it twice.
            .filter(|row| !read.contains(&row.entry()))
            .collect();
        self.created.sort_by_key(Listed::entry);
        self.forget_matches();
    }

    /// Whether a read should start for the database as it stands at `writes`.
    /// When it should, the list is marked as read at `writes` before the read
    /// starts, so a failed read is not retried every frame.
    pub fn wants_read(&mut self, writes: u64) -> bool {
        if self.reading() || self.loaded == Some(writes) {
            return false;
        }
        self.loaded = Some(writes);
        true
    }

    /// Hold a read that has started.
    pub fn reading_with(&mut self, task: Task<Result<ListRead<K>, String>>) {
        self.task = Some(task);
    }

    /// Say why there is nothing to read from.
    pub fn unreadable(&mut self, why: String) {
        self.all.clear();
        self.created.clear();
        self.trouble = Some(why);
        self.forget_matches();
    }

    /// Collect a finished read. Answers the highest entry the table holds
    /// when one landed.
    pub fn collect(&mut self) -> Option<Option<u32>> {
        let task = self.task.as_mut()?;
        let done = block_on(future::poll_once(task))?;
        self.task = None;
        match done {
            Ok(read) => {
                info!("{}: {} row(s) read", K::TABLE, read.rows.len());
                self.all = read.rows;
                self.trouble = None;
                self.followed = None;
                self.forget_matches();
                Some(read.highest)
            }
            Err(e) => {
                warn!("{}: {e}", K::TABLE);
                self.unreadable(e);
                None
            }
        }
    }
}

/// Give every row the entry the project says it has. Every row is reset to
/// the entry it was read at first, so a move that was undone takes the row
/// back. `Creatures::fold_template_moves` does the same for the rows the
/// creature tool holds.
pub fn fold_moves<K: Listed>(rows: &mut [K], edits: &Edits) {
    let moves: Vec<(u32, u32, u32)> = edits
        .moves(K::TABLE)
        .filter_map(|(from, to)| {
            let patch = from
                .0
                .iter()
                .find(|(column, _)| column == "patch")
                .and_then(|(_, value)| value.parse::<u32>().ok())?;
            Some((from.first()? as u32, patch, to.first()? as u32))
        })
        .collect();
    for row in rows.iter_mut() {
        let entry = moves
            .iter()
            .find(|(from, patch, _)| *from == row.read_entry() && *patch == row.patch())
            .map(|(_, _, to)| *to)
            .unwrap_or(row.read_entry());
        row.set_entry(entry);
    }
}
