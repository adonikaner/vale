//! One field of one record, before and after.

use super::DbcFile;

/// A change to a single field.
///
/// **One word, always.** Every field in a DBC is four bytes, and a string field
/// holds a byte offset into the string block rather than the text — so editing
/// a name is appending the text (which is not undone, and does not need to be:
/// see the module comment on the append-only block) and then writing the new
/// offset here. That makes one type enough for every kind of edit this
/// container has, and makes an undo exact: the old offset still points at the
/// old text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub record: usize,
    pub field: usize,
    pub before: u32,
    pub after: u32,
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
    })
}
