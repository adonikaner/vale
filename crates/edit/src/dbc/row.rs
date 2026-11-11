//! One whole record, added or removed, for the undo stack.

use super::DbcFile;

/// A record put in or taken out.
///
/// The counterpart of [`super::Cell`] for the one edit a field write cannot
/// express: a row that did not exist before, or one that no longer does. It
/// carries the record's whole bytes and the index it sits at, so both
/// directions are exact: an add is undone by removing that index, and a
/// removal by putting the same bytes back at the same index.
///
/// Removing a record moves every index after it, so a change that adds a row
/// and then writes into it has to apply the row first and the cells second,
/// and revert in the opposite order. `vale_edit::undo::Change` keeps that
/// order; this type only knows one row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Row {
    pub index: usize,
    pub bytes: Vec<u8>,
    /// `true` for a row that was added (applying puts it in) and `false` for
    /// one that was removed.
    pub added: bool,
    /// The record's id, when the table's ids are unique
    /// ([`DbcFile::id_of`]). The undo stack then keeps it as a [`Record`].
    /// Not stored.
    #[serde(skip)]
    pub id: Option<u32>,
}

/// A record put in or taken out, found by its id rather than its index when
/// it is taken out: the undo stack's form of a [`Row`] for a table whose ids
/// are unique. Records added or removed before it do not move what it
/// addresses.
///
/// A record put in goes back at the index it was made at, or at the end of a
/// table that is shorter now, so an undo and a redo on one editor give the
/// same bytes. Two editors that add records at once may hold them in
/// different orders; nothing in a table depends on the order of its records.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub id: u32,
    pub index: usize,
    pub bytes: Vec<u8>,
    pub added: bool,
}

impl Record {
    fn put_in(&self, table: &mut DbcFile) {
        if table.row_of(self.id).is_none() {
            table.insert_record(self.index.min(table.record_count()), &self.bytes);
        }
    }

    fn take_out(&self, table: &mut DbcFile) {
        if let Some(at) = table.row_of(self.id) {
            table.remove_record(at);
        }
    }

    /// Do it: put an added record in, take a removed one out.
    pub fn apply(&self, table: &mut DbcFile) {
        match self.added {
            true => self.put_in(table),
            false => self.take_out(table),
        }
    }

    /// Undo it: take an added record out, put a removed one back.
    pub fn revert(&self, table: &mut DbcFile) {
        match self.added {
            true => self.take_out(table),
            false => self.put_in(table),
        }
    }
}

impl Row {
    /// Note a record already appended at `index`.
    pub fn added(table: &DbcFile, index: usize) -> Option<Row> {
        Some(Row {
            index,
            bytes: table.record_bytes(index)?.to_vec(),
            added: true,
            id: table.id_of(index),
        })
    }

    /// Note a record about to be removed from `index`, read before the
    /// removal, since afterwards the bytes are gone.
    pub fn removed(table: &DbcFile, index: usize) -> Option<Row> {
        Some(Row {
            index,
            bytes: table.record_bytes(index)?.to_vec(),
            added: false,
            id: table.id_of(index),
        })
    }

    /// Do it: put an added row in, take a removed row out.
    pub fn apply(&self, table: &mut DbcFile) {
        match self.added {
            true => {
                table.insert_record(self.index, &self.bytes);
            }
            false => {
                table.remove_record(self.index);
            }
        }
    }

    /// Undo it: take an added row out, put a removed row back.
    pub fn revert(&self, table: &mut DbcFile) {
        match self.added {
            true => {
                table.remove_record(self.index);
            }
            false => {
                table.insert_record(self.index, &self.bytes);
            }
        }
    }
}
