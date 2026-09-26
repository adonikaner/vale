//! One whole record, added or removed, for the undo stack.

use super::DbcFile;

/// A record put in or taken out.
///
/// The counterpart of [`super::Cell`] for the one edit a field write cannot
/// express: a row that did not exist before, or one that no longer does. It
/// carries the record's whole bytes and the index it sits at, so both
/// directions are exact — an add is undone by removing that index, and a
/// removal by putting the same bytes back at the same index.
///
/// **Order matters more here than for a cell.** Removing a record moves every
/// index after it, so a change that adds a row and then writes into it has to
/// apply the row first and the cells second, and revert in the opposite order.
/// `vale_edit::undo::Change` keeps that order; this type only knows one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub index: usize,
    pub bytes: Vec<u8>,
    /// `true` for a row that was added — applying puts it in — and `false` for
    /// one that was removed.
    pub added: bool,
}

impl Row {
    /// Note a record already appended at `index`.
    pub fn added(table: &DbcFile, index: usize) -> Option<Row> {
        Some(Row {
            index,
            bytes: table.record_bytes(index)?.to_vec(),
            added: true,
        })
    }

    /// Note a record about to be removed from `index` — read before the
    /// removal, since afterwards the bytes are gone.
    pub fn removed(table: &DbcFile, index: usize) -> Option<Row> {
        Some(Row {
            index,
            bytes: table.record_bytes(index)?.to_vec(),
            added: false,
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
