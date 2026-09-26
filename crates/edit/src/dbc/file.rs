//! The container: the header, the records, the string block, the writer.

use crate::EditError;

/// What a DBC opens with.
const MAGIC: &[u8; 4] = b"WDBC";
/// Magic plus the four counts.
const HEADER_LEN: usize = 20;
/// Every field is four bytes wide, whatever it holds.
pub const FIELD_LEN: usize = 4;

/// One `DBFilesClient\*.dbc`, carried whole.
///
/// The four header words are kept as the file stated them and are updated only
/// by the operations that change what they count. The two blocks are the file's
/// own bytes. See the module comment for why nothing here is parsed into a
/// model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbcFile {
    record_count: u32,
    field_count: u32,
    record_size: u32,
    string_size: u32,
    /// `recordCount * recordSize` bytes, as read.
    records: Vec<u8>,
    /// The NUL-separated block every string field points into, as read.
    strings: Vec<u8>,
    /// Anything after it. No shipped table has any; a patched one might, and a
    /// writer that dropped it would be lossy in the one way that is invisible.
    tail: Vec<u8>,
}

impl DbcFile {
    /// Take a table apart.
    ///
    /// The header is believed about the record block, which is checked against
    /// the file's length, and *not* about the string block: a table whose stated
    /// string size runs past the end of the file keeps the bytes it actually
    /// has, and [`Self::write`] still reproduces it exactly, because the stated
    /// word is written back as it was read.
    pub fn parse(buf: &[u8]) -> Result<DbcFile, EditError> {
        if buf.len() < HEADER_LEN || &buf[0..4] != MAGIC {
            return Err(EditError::malformed("DBC", "missing WDBC magic"));
        }
        let word = |at: usize| u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]);
        let record_count = word(4);
        let field_count = word(8);
        let record_size = word(12);
        let string_size = word(16);

        let records_len = (record_count as usize)
            .checked_mul(record_size as usize)
            .ok_or_else(|| EditError::malformed("DBC", "record block size overflows"))?;
        let records_end = HEADER_LEN
            .checked_add(records_len)
            .ok_or_else(|| EditError::malformed("DBC", "record block size overflows"))?;
        if records_end > buf.len() {
            return Err(EditError::malformed(
                "DBC",
                format!("record block runs past EOF ({records_end} > {})", buf.len()),
            ));
        }
        let strings_end = records_end
            .saturating_add(string_size as usize)
            .min(buf.len());

        Ok(DbcFile {
            record_count,
            field_count,
            record_size,
            string_size,
            records: buf[HEADER_LEN..records_end].to_vec(),
            strings: buf[records_end..strings_end].to_vec(),
            tail: buf[strings_end..].to_vec(),
        })
    }

    /// …and put it back together.
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            HEADER_LEN + self.records.len() + self.strings.len() + self.tail.len(),
        );
        out.extend_from_slice(MAGIC);
        for word in [
            self.record_count,
            self.field_count,
            self.record_size,
            self.string_size,
        ] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(&self.records);
        out.extend_from_slice(&self.strings);
        out.extend_from_slice(&self.tail);
        out
    }

    pub fn record_count(&self) -> usize {
        self.record_count as usize
    }

    pub fn field_count(&self) -> usize {
        self.field_count as usize
    }

    pub fn record_size(&self) -> usize {
        self.record_size as usize
    }

    /// The string block's length, which an edit to a string grows.
    pub fn string_size(&self) -> usize {
        self.strings.len()
    }

    /// Where `(record, field)` starts in the record block, or `None` when either
    /// is out of range **or the field does not fit in the record**.
    ///
    /// The second is not hypothetical: `CharBaseInfo.dbc` states records of two
    /// bytes, so its one field is half a field wide and no four-byte read of it
    /// is valid. See the module comment.
    fn field_at(&self, record: usize, field: usize) -> Option<usize> {
        if record >= self.record_count() || field >= self.field_count() {
            return None;
        }
        let within = field.checked_mul(FIELD_LEN)?;
        if within + FIELD_LEN > self.record_size() {
            return None;
        }
        let at = record.checked_mul(self.record_size())?.checked_add(within)?;
        (at + FIELD_LEN <= self.records.len()).then_some(at)
    }

    pub fn u32_at(&self, record: usize, field: usize) -> Option<u32> {
        let at = self.field_at(record, field)?;
        self.records
            .get(at..at + FIELD_LEN)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
    }

    pub fn i32_at(&self, record: usize, field: usize) -> Option<i32> {
        self.u32_at(record, field).map(|v| v as i32)
    }

    pub fn f32_at(&self, record: usize, field: usize) -> Option<f32> {
        self.u32_at(record, field).map(f32::from_bits)
    }

    /// Write a raw word. `false` when the field is out of range, which is the
    /// same answer as the readers give and for the same reason.
    pub fn set_u32(&mut self, record: usize, field: usize, value: u32) -> bool {
        let Some(at) = self.field_at(record, field) else {
            return false;
        };
        self.records[at..at + FIELD_LEN].copy_from_slice(&value.to_le_bytes());
        true
    }

    pub fn set_i32(&mut self, record: usize, field: usize, value: i32) -> bool {
        self.set_u32(record, field, value as u32)
    }

    pub fn set_f32(&mut self, record: usize, field: usize, value: f32) -> bool {
        self.set_u32(record, field, value.to_bits())
    }

    /// One record's own bytes, whatever its width.
    pub fn record_bytes(&self, record: usize) -> Option<&[u8]> {
        if record >= self.record_count() {
            return None;
        }
        let at = record * self.record_size();
        self.records.get(at..at + self.record_size())
    }

    /// Replace one record's bytes, which must be exactly one record wide.
    pub fn set_record_bytes(&mut self, record: usize, bytes: &[u8]) -> bool {
        if record >= self.record_count() || bytes.len() != self.record_size() {
            return false;
        }
        let size = self.record_size();
        let at = record * size;
        self.records[at..at + size].copy_from_slice(bytes);
        true
    }

    /// Add a record at the end, returning its index.
    ///
    /// The bytes must be one record wide. Nothing here checks that field 0 is an
    /// id nobody else holds — that is a rule about a *table*, and this is the
    /// container.
    pub fn push_record(&mut self, bytes: &[u8]) -> Option<usize> {
        if bytes.len() != self.record_size() {
            return None;
        }
        self.records.extend_from_slice(bytes);
        self.record_count += 1;
        Some(self.record_count() - 1)
    }

    /// Put a record in at `index`, moving the ones after it up by one.
    ///
    /// The bytes must be one record wide. `index == record_count()` appends,
    /// which is what [`Self::push_record`] does; anything past that is refused.
    /// A string offset a record carries stays valid, because the string block
    /// is addressed absolutely and is not touched here.
    pub fn insert_record(&mut self, index: usize, bytes: &[u8]) -> bool {
        if bytes.len() != self.record_size() || index > self.record_count() {
            return false;
        }
        let at = index * self.record_size();
        self.records.splice(at..at, bytes.iter().copied());
        self.record_count += 1;
        true
    }

    /// Take a record out, returning its bytes, and move the ones after it down
    /// by one. `None` when there is no such record.
    ///
    /// **Every record index past `index` changes**, which is why a caller holding
    /// one — an open row, a reverse index — has to be told; see
    /// `vale_edit::undo::Change::rows`. The string block keeps whatever the
    /// record pointed at, on the append-only rule.
    pub fn remove_record(&mut self, index: usize) -> Option<Vec<u8>> {
        if index >= self.record_count() {
            return None;
        }
        let size = self.record_size();
        let at = index * size;
        let bytes: Vec<u8> = self.records.drain(at..at + size).collect();
        self.record_count -= 1;
        Some(bytes)
    }

    /// A record of this table's width with `id` in field 0 and zero everywhere
    /// else, for a caller making a new row.
    ///
    /// Zero is the empty string in every shipped table, so a string column of
    /// the new row reads as "" rather than as garbage. What a column's own
    /// *none* is — `-1` for a kit's effect slots — is a rule about the table
    /// and is the caller's to write afterwards.
    pub fn blank_record(&self, id: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; self.record_size()];
        if bytes.len() >= FIELD_LEN {
            bytes[..FIELD_LEN].copy_from_slice(&id.to_le_bytes());
        }
        bytes
    }

    /// The text at a byte offset into the string block.
    pub fn text_at(&self, offset: usize) -> Option<String> {
        let rest = self.strings.get(offset..)?;
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        Some(String::from_utf8_lossy(&rest[..end]).into_owned())
    }

    /// …and the text a string field points at.
    pub fn string_at(&self, record: usize, field: usize) -> Option<String> {
        self.text_at(self.u32_at(record, field)? as usize)
    }

    /// Point a string field at `text`, appending it to the block if it is not
    /// already there.
    ///
    /// **No offset the file already holds changes.** See the module comment: the
    /// block is append-only, because a string column is a `u32` like any other
    /// and this crate cannot know which of a table's columns are strings.
    pub fn set_string(&mut self, record: usize, field: usize, text: &str) -> bool {
        if self.field_at(record, field).is_none() {
            return false;
        }
        let offset = match self.find_text(text) {
            Some(offset) => offset,
            None => self.append_text(text),
        };
        self.set_u32(record, field, offset)
    }

    /// Where `text` already sits in the block, matching only at a string's own
    /// start so that a search for "Fire" cannot land inside "Fireball".
    fn find_text(&self, text: &str) -> Option<u32> {
        if text.is_empty() {
            // Offset 0 is the empty string in every shipped table, and a block
            // that does not open with a NUL cannot be used that way.
            return (self.strings.first() == Some(&0)).then_some(0);
        }
        let needle = text.as_bytes();
        let mut at = 0usize;
        while at < self.strings.len() {
            let end = self.strings[at..]
                .iter()
                .position(|&b| b == 0)
                .map(|found| at + found)
                .unwrap_or(self.strings.len());
            if &self.strings[at..end] == needle {
                return Some(at as u32);
            }
            at = end + 1;
        }
        None
    }

    /// …and putting it there when it is not.
    fn append_text(&mut self, text: &str) -> u32 {
        // A block that is empty gets its leading NUL first, so that offset 0
        // keeps meaning the empty string rather than becoming this text.
        if self.strings.is_empty() {
            self.strings.push(0);
        }
        let offset = self.strings.len() as u32;
        self.strings.extend_from_slice(text.as_bytes());
        self.strings.push(0);
        self.string_size = self.strings.len() as u32;
        offset
    }

    /// The record holding `id` in field 0.
    ///
    /// Field 0 is the id in every 1.12 table; the reader in `vale_assets`
    /// assumes the same thing and `vale dbc <Table> <id>` is written against
    /// it. A table whose first column is not an id answers `None` rather than
    /// the wrong row, because no row matches.
    pub fn row_of(&self, id: u32) -> Option<usize> {
        (0..self.record_count()).find(|&record| self.u32_at(record, 0) == Some(id))
    }

    /// The largest id in field 0, for a caller minting a new one.
    pub fn max_id(&self) -> u32 {
        (0..self.record_count())
            .filter_map(|record| self.u32_at(record, 0))
            .max()
            .unwrap_or(0)
    }
}
