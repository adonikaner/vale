//! One field of one record, before and after.

use super::DbcFile;

/// A change to a single field.
///
/// A field is one word. Every field in a DBC is four bytes, and a string field
/// holds a byte offset into the string block rather than the text, so editing
/// a name is appending the text (which is not undone, and does not need to be:
/// see the module comment on the append-only block) and then writing the new
/// offset here. One type therefore covers every field edit this container
/// has, and an undo is exact: the old offset still points at the old text.
///
/// An offset means something only in the block it was made in. Two editors
/// append to their own blocks, so an edit that leaves this editor carries the
/// text instead ([`Text`], `crate::undo::Change::with_texts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    pub record: usize,
    pub field: usize,
    pub before: u32,
    pub after: u32,
    /// The record's id, when the table's ids are unique
    /// ([`DbcFile::id_of`]). The undo stack then keeps the edit as a
    /// [`Field`], addressed by the id. Not stored: a stored cell is
    /// addressed by its index.
    #[serde(skip)]
    pub id: Option<u32>,
    /// The field is a string, written by [`set_text`]: `before` and `after`
    /// are offsets into the string block. Not stored: a stored edit carries
    /// the text.
    #[serde(skip)]
    pub text: bool,
}

/// A field edit addressed by its record's id rather than its index, which
/// the undo stack keeps for a table whose ids are unique. Records added or
/// removed before it, by this editor or another, do not move what it
/// addresses.
///
/// An edit of field 0 changes the id itself. It finds its record by `before`
/// going forward and by `after` going back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Field {
    pub id: u32,
    pub field: usize,
    pub before: u32,
    pub after: u32,
    /// The record's index when the edit was made, for a list of what an edit
    /// touched.
    pub record: usize,
    /// The field is a string, as [`Cell::text`]. Not stored.
    #[serde(skip)]
    pub text: bool,
}

/// A string field set to a text, rather than to an offset: what a string
/// edit is once it leaves the editor that made it. Each editor puts the text
/// in its own string block ([`DbcFile::set_string`]), so the edit means the
/// same on every editor whatever each block holds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Text {
    /// The record's id, when the table's ids are unique; the record is then
    /// found by it.
    pub id: Option<u32>,
    /// The record's index, which finds it when there is no id.
    pub record: usize,
    pub field: usize,
    pub before: String,
    pub after: String,
}

impl Text {
    fn record_in(&self, table: &DbcFile) -> Option<usize> {
        match self.id {
            Some(id) => table.row_of(id),
            None => (self.record < table.record_count()).then_some(self.record),
        }
    }

    pub fn apply(&self, table: &mut DbcFile) {
        if let Some(record) = self.record_in(table) {
            table.set_string(record, self.field, &self.after);
        }
    }

    pub fn revert(&self, table: &mut DbcFile) {
        if let Some(record) = self.record_in(table) {
            table.set_string(record, self.field, &self.before);
        }
    }

    /// The field's text now, for a rebase. `None` when the record is gone.
    pub fn now(&self, table: &DbcFile) -> Option<String> {
        table.string_at(self.record_in(table)?, self.field)
    }
}

impl Field {
    /// The record the edit lands on: the one with its id going forward, and
    /// for an edit of the id, the one with the id it wrote going back.
    fn record_in(&self, table: &DbcFile, forward: bool) -> Option<usize> {
        let id = match (self.field == 0, forward) {
            (true, false) => self.after,
            _ => self.id,
        };
        table.row_of(id)
    }

    pub fn apply(&self, table: &mut DbcFile) {
        if let Some(record) = self.record_in(table, true) {
            table.set_u32(record, self.field, self.after);
        }
    }

    pub fn revert(&self, table: &mut DbcFile) {
        if let Some(record) = self.record_in(table, false) {
            table.set_u32(record, self.field, self.before);
        }
    }

    /// The ids the edit names: its record's, and for an edit of the id, the
    /// id it writes.
    pub fn ids(&self) -> Vec<u32> {
        match self.field == 0 && self.before != self.after {
            true => vec![self.before, self.after],
            false => vec![self.id],
        }
    }

    /// The field's value now, for a rebase. `None` when no record has the id.
    pub fn now(&self, table: &DbcFile, forward: bool) -> Option<u32> {
        table.u32_at(self.record_in(table, forward)?, self.field)
    }
}

impl Cell {
    /// Read the field's current value and pair it with what it is becoming.
    ///
    /// `None` when the field does not exist — which is not only an index
    /// mistake: `CharBaseInfo`'s records are two bytes, so its fields cannot be
    /// written as words at all. See [`DbcFile::u32_at`].
    pub fn new(table: &DbcFile, record: usize, field: usize, after: u32) -> Option<Cell> {
        Some(Cell {
            record,
            field,
            before: table.u32_at(record, field)?,
            after,
            id: table.id_of(record),
            text: false,
        })
    }

    pub fn apply(&self, table: &mut DbcFile) {
        table.set_u32(self.record, self.field, self.after);
    }

    pub fn revert(&self, table: &mut DbcFile) {
        table.set_u32(self.record, self.field, self.before);
    }

    /// Whether this changed anything. A cell that writes what was already there
    /// is not an undo step — see `History::end`, which drops an empty change.
    pub fn moves(&self) -> bool {
        self.before != self.after
    }
}

/// A string edit, as the append plus the cell it produces.
///
/// The append happens here rather than in the caller because the offset the
/// cell carries is only meaningful once the text is in the block. `None` when
/// the field does not exist.
pub fn set_text(table: &mut DbcFile, record: usize, field: usize, text: &str) -> Option<Cell> {
    let before = table.u32_at(record, field)?;
    table.set_string(record, field, text);
    let after = table.u32_at(record, field)?;
    Some(Cell {
        record,
        field,
        before,
        after,
        id: table.id_of(record),
        text: true,
    })
}
