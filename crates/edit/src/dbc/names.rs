//! Which columns of a table hold text, and which column names its rows, read
//! from the schema where `vale_assets::tables::schema` has one and guessed from
//! the file where it has none.
//!
//! A DBC column holds strings as offsets into the table's string block, and a
//! table with no schema says nothing about which columns those are. The
//! guesses here read the first [`GUESS_ROWS`] rows and accept a column only
//! when every sampled value is zero or the start of a string.

use super::DbcFile;
use vale_assets::tables::schema;

/// The column a row of `table` is named by: its schema's name column when it
/// has a schema, and [`guess_name_field`] when it has none.
pub fn name_field(table: &str, file: &DbcFile) -> Option<usize> {
    match schema::for_table(table) {
        Some(described) => described.name_field(),
        None => guess_name_field(file),
    }
}

/// The string a field's value starts, when the value is the offset of a
/// string's first byte in the table's string block: past the block's
/// opening NUL, directly after another NUL, and printable. `None` for a
/// value that is a number.
pub(crate) fn string_started_at(table: &DbcFile, raw: u32) -> Option<String> {
    let offset = raw as usize;
    if offset == 0 || offset >= table.string_size() {
        return None;
    }
    if table.text_at(offset - 1).as_deref() != Some("") {
        return None;
    }
    table
        .text_at(offset)
        .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

/// How many rows the guesses about a table with no schema read.
pub(crate) const GUESS_ROWS: usize = 64;

/// Whether a column of a table with no schema holds strings, and in how
/// many of the sampled rows: every sampled row holds zero or the offset of
/// a string, and the strings are not all one string.
///
/// The test is on the column and not on one value. A small number can be
/// the offset of a string's first byte: `Faction.dbc`'s field 1 holds 1 on
/// some rows, which is where the block's first string starts. A column of
/// such numbers fails on the rows that hold any other number, and a
/// column that holds one number throughout fails the last condition.
fn strings_in(table: &DbcFile, field: usize) -> Option<usize> {
    let rows = table.record_count().min(GUESS_ROWS);
    let mut texts: Vec<String> = Vec::new();
    for record in 0..rows {
        match table.u32_at(record, field).unwrap_or(0) {
            0 => {}
            raw => texts.push(string_started_at(table, raw)?),
        }
    }
    let held = texts.len();
    texts.sort();
    texts.dedup();
    (texts.len() > 1 || (rows == 1 && held == 1)).then_some(held)
}

/// Which columns of a table with no schema hold strings, by field. Field 0
/// is the id and never does.
pub fn text_fields(table: &DbcFile) -> Vec<bool> {
    (0..table.field_count())
        .map(|field| field > 0 && strings_in(table, field).is_some())
        .collect()
}

/// The column a table with no schema is named by, guessed from the file:
/// the first column of strings in which at least half the sampled rows
/// hold one.
pub fn guess_name_field(table: &DbcFile) -> Option<usize> {
    let rows = table.record_count().min(GUESS_ROWS);
    (1..table.field_count())
        .find(|&field| strings_in(table, field).is_some_and(|held| held * 2 >= rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty table of `fields` columns, for a test to add rows to.
    fn empty(fields: u32) -> DbcFile {
        let mut bytes = b"WDBC".to_vec();
        for word in [0u32, fields, fields * 4, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0);
        DbcFile::parse(&bytes).expect("an empty table")
    }

    /// Add a row with `id`, the numbers in `set` and the strings in `text`.
    fn add(table: &mut DbcFile, id: u32, set: &[(usize, u32)], text: &[(usize, &str)]) {
        let blank = table.blank_record(id);
        let record = table.push_record(&blank).expect("a record");
        for &(field, value) in set {
            table.set_u32(record, field, value);
        }
        for &(field, value) in text {
            table.set_string(record, field, value);
        }
    }

    /// The name column of a table with no schema is the first whose values
    /// are the offsets of differing strings. A column of ones is the offset
    /// of the block's first string on every row and is not a name.
    #[test]
    fn a_table_with_no_schema_is_named_by_its_first_column_of_strings() {
        let mut table = empty(4);
        add(&mut table, 1, &[(1, 1), (2, 40)], &[(3, "Alpha")]);
        add(&mut table, 2, &[(1, 1), (2, 41)], &[(3, "Beta")]);
        add(&mut table, 3, &[(1, 1), (2, 42)], &[(3, "Gamma")]);
        assert_eq!(string_started_at(&table, 1).as_deref(), Some("Alpha"));
        assert_eq!(string_started_at(&table, 2), None, "inside a string");
        assert_eq!(string_started_at(&table, 0), None, "the empty string");
        assert_eq!(string_started_at(&table, 4000), None, "past the block");
        assert_eq!(guess_name_field(&table), Some(3));
        assert_eq!(guess_name_field(&empty(4)), None);
        // Field 1 holds 1 on every row, the offset of the first string, and
        // field 2 holds numbers that are no string's offset.
        assert_eq!(text_fields(&table), vec![false, false, false, true]);
        // A column that holds a string's offset on one row and another
        // number on the next is numbers.
        let mut mixed = empty(3);
        add(&mut mixed, 1, &[(1, 1)], &[(2, "Alpha")]);
        add(&mut mixed, 2, &[(1, 3)], &[(2, "Beta")]);
        assert_eq!(text_fields(&mixed), vec![false, false, true]);

        // A table with no schema is named by the guess, and a described
        // table by its schema whatever the file holds.
        assert_eq!(name_field("Lock", &table), Some(3));
        assert_eq!(name_field("ItemSet", &table), Some(1));
    }
}
