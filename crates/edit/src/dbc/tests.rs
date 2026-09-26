//! The round trip, which is the only check that says a writer is lossless.
//!
//! The same argument as `crate::adt::tests`, one container along: a writer can
//! produce a file nothing loads, which fails immediately, or a file that loads
//! with something silently dropped, which fails the next time the client asks
//! the table a question nobody tested. Comparing the bytes catches both.
//!
//! There is one difference from the tile checks, and it is why this file runs
//! over *every* table rather than over a named handful. A tile is 256 chunks of
//! the same shape and five of them exercise the format; a DBC's shape is one
//! header and two blocks, and what varies between tables is exactly the edge
//! cases — a record narrower than a field, an empty table, a table with no
//! strings at all. So the check is the whole of `DBFilesClient\`.

use super::*;

/// The archives, or `None` on a machine that has no install.
///
/// `VALE_GAMEDATA` moves the folder; otherwise it is the `Data\` beside this
/// repository, which is where the drop-in rule puts it.
fn archives() -> Option<vale_assets::Assets> {
    let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("Data")
            .to_string_lossy()
            .into_owned()
    });
    match vale_assets::Assets::open(&root) {
        Ok(assets) => Some(assets),
        Err(e) => {
            eprintln!("no archives at {root}: {e} — the table checks did not run");
            None
        }
    }
}

/// A table built by hand, for the checks that need no archives: three records of
/// two fields, an id and a string.
fn synthetic() -> Vec<u8> {
    let strings: Vec<u8> = b"\0first\0second\0".to_vec();
    let mut out = Vec::new();
    out.extend_from_slice(b"WDBC");
    out.extend_from_slice(&3u32.to_le_bytes()); // records
    out.extend_from_slice(&2u32.to_le_bytes()); // fields
    out.extend_from_slice(&8u32.to_le_bytes()); // record size
    out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    for (id, at) in [(10u32, 1u32), (20, 7), (30, 0)] {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
    }
    out.extend_from_slice(&strings);
    out
}

#[test]
fn every_table_in_the_archives_writes_back_byte_for_byte() {
    let Some(mut assets) = archives() else {
        return;
    };
    let tables = assets.list_prefix("DBFilesClient\\");
    assert!(
        tables.len() > 100,
        "the archives answered with {} tables, which is not a 1.12 install",
        tables.len()
    );

    let mut checked = 0usize;
    let mut empty = Vec::new();
    for path in tables {
        let Ok(raw) = assets.read(&path) else {
            continue;
        };
        // **`DBFilesClient\` carries entries that are not tables.**
        // `CharacterCreateCameras.dbc` is zero bytes in a 1.12.1 install — a
        // name the client asks for and gets nothing from. It is not a parse
        // failure to be tolerated inside the reader: a table with no header is
        // not a table, and a caller opening one for editing has to be told.
        if raw.is_empty() {
            assert!(DbcFile::parse(&raw).is_err());
            empty.push(path);
            continue;
        }
        let table =
            DbcFile::parse(&raw).unwrap_or_else(|e| panic!("{path} did not parse: {e}"));
        let written = table.write();
        assert_eq!(
            written.len(),
            raw.len(),
            "{path}: wrote {} bytes for a {}-byte file",
            written.len(),
            raw.len()
        );
        assert!(
            written == raw,
            "{path}: the bytes differ at {:?}",
            written.iter().zip(&raw).position(|(a, b)| a != b)
        );
        checked += 1;
    }
    assert!(checked > 100, "only {checked} tables were read");
    eprintln!("{checked} tables written back byte for byte, {} empty: {empty:?}", empty.len());
}

#[test]
fn a_synthetic_table_writes_back_byte_for_byte() {
    let raw = synthetic();
    assert_eq!(DbcFile::parse(&raw).unwrap().write(), raw);
}

#[test]
fn the_header_is_read_as_it_was_written() {
    let table = DbcFile::parse(&synthetic()).unwrap();
    assert_eq!(table.record_count(), 3);
    assert_eq!(table.field_count(), 2);
    assert_eq!(table.record_size(), 8);
    assert_eq!(table.u32_at(1, 0), Some(20));
    assert_eq!(table.string_at(0, 1).as_deref(), Some("first"));
    assert_eq!(table.string_at(1, 1).as_deref(), Some("second"));
    // Offset 0 is the empty string, which is how a table says nothing.
    assert_eq!(table.string_at(2, 1).as_deref(), Some(""));
}

#[test]
fn a_field_write_changes_that_field_and_nothing_else() {
    let mut table = DbcFile::parse(&synthetic()).unwrap();
    let before = table.write();
    // A value whose every byte differs from the 20 that was there, so that a
    // writer touching too few bytes is visible as well as one touching too many.
    assert!(table.set_u32(1, 0, 0xDEAD_BEEF));
    let after = table.write();
    assert_eq!(table.u32_at(1, 0), Some(0xDEAD_BEEF));
    assert_eq!(after.len(), before.len());
    let differing: Vec<usize> = before
        .iter()
        .zip(&after)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(at, _)| at)
        .collect();
    // Four bytes, all inside record 1's first field: 20 header + 8 record.
    assert_eq!(differing, vec![28, 29, 30, 31]);
}

#[test]
fn a_write_to_a_field_that_is_not_there_is_refused() {
    let mut table = DbcFile::parse(&synthetic()).unwrap();
    assert!(!table.set_u32(3, 0, 1), "record 3 does not exist");
    assert!(!table.set_u32(0, 2, 1), "field 2 does not exist");
    assert!(!table.set_string(9, 1, "x"));
    assert_eq!(table.write(), synthetic(), "a refused write changed bytes");
}

#[test]
fn a_new_string_is_appended_and_every_other_offset_stands() {
    let mut table = DbcFile::parse(&synthetic()).unwrap();
    let was = table.string_size();
    assert!(table.set_string(0, 1, "a longer name than the one it had"));

    assert_eq!(table.string_at(0, 1).as_deref(), Some("a longer name than the one it had"));
    // The point of appending: the other rows still read what they read before.
    assert_eq!(table.string_at(1, 1).as_deref(), Some("second"));
    assert_eq!(table.string_at(2, 1).as_deref(), Some(""));
    assert_eq!(table.u32_at(0, 1), Some(was as u32));
    assert_eq!(table.string_size(), was + "a longer name than the one it had".len() + 1);

    // …and it survives a round trip through the bytes.
    let again = DbcFile::parse(&table.write()).unwrap();
    assert_eq!(again, table);
}

#[test]
fn the_same_text_twice_is_shared_rather_than_appended() {
    let mut table = DbcFile::parse(&synthetic()).unwrap();
    assert!(table.set_string(2, 1, "first"));
    assert_eq!(
        table.string_size(),
        DbcFile::parse(&synthetic()).unwrap().string_size(),
        "an existing string was appended again"
    );
    assert_eq!(table.u32_at(2, 1), Some(1), "it did not point at the old copy");

    let was = table.string_size();
    assert!(table.set_string(2, 1, "new"));
    assert!(table.set_string(1, 1, "new"));
    assert_eq!(table.string_size(), was + 4, "the second write appended again");
    assert_eq!(table.u32_at(1, 1), table.u32_at(2, 1));
}

#[test]
fn a_prefix_is_not_mistaken_for_a_string() {
    let mut table = DbcFile::parse(&synthetic()).unwrap();
    // "sec" is inside "second" but is not a string of its own, so it must be
    // appended rather than found at an offset that would read "second".
    assert!(table.set_string(0, 1, "sec"));
    assert_eq!(table.string_at(0, 1).as_deref(), Some("sec"));
    assert_eq!(table.string_at(1, 1).as_deref(), Some("second"));
}

#[test]
fn clearing_a_string_points_it_at_offset_zero() {
    let mut table = DbcFile::parse(&synthetic()).unwrap();
    let was = table.string_size();
    assert!(table.set_string(0, 1, ""));
    assert_eq!(table.u32_at(0, 1), Some(0));
    assert_eq!(table.string_size(), was, "the empty string was appended");
}

#[test]
fn a_record_narrower_than_a_field_is_carried_but_not_read() {
    // CharBaseInfo.dbc's shape: 41 records of two bytes, one field.
    let mut raw = Vec::new();
    raw.extend_from_slice(b"WDBC");
    raw.extend_from_slice(&2u32.to_le_bytes());
    raw.extend_from_slice(&1u32.to_le_bytes());
    raw.extend_from_slice(&2u32.to_le_bytes());
    raw.extend_from_slice(&1u32.to_le_bytes());
    raw.extend_from_slice(&[1, 2, 3, 4, 0]);

    let mut table = DbcFile::parse(&raw).unwrap();
    assert_eq!(table.record_bytes(0), Some(&[1u8, 2][..]));
    assert_eq!(table.record_bytes(1), Some(&[3u8, 4][..]));
    assert_eq!(table.u32_at(0, 0), None, "a four-byte read of a two-byte record");
    assert!(!table.set_u32(0, 0, 7));
    assert!(table.set_record_bytes(1, &[9, 9]));
    assert_eq!(table.record_bytes(1), Some(&[9u8, 9][..]));
    assert_eq!(DbcFile::parse(&table.write()).unwrap(), table);
}

#[test]
fn a_pushed_record_is_counted_and_readable() {
    let mut table = DbcFile::parse(&synthetic()).unwrap();
    let mut row = Vec::new();
    row.extend_from_slice(&40u32.to_le_bytes());
    row.extend_from_slice(&0u32.to_le_bytes());
    let at = table.push_record(&row).unwrap();

    assert_eq!(at, 3);
    assert_eq!(table.record_count(), 4);
    assert_eq!(table.u32_at(3, 0), Some(40));
    assert_eq!(table.row_of(40), Some(3));
    assert_eq!(table.max_id(), 40);
    assert_eq!(
        DbcFile::parse(&table.write()).unwrap(),
        table,
        "the new record did not survive the bytes"
    );
    assert!(table.push_record(&[0; 3]).is_none(), "a short record was taken");
}

#[test]
fn a_file_that_is_not_a_table_is_refused() {
    assert!(DbcFile::parse(b"").is_err());
    assert!(DbcFile::parse(b"WDBCxxxx").is_err());
    // A header claiming more records than the file holds.
    let mut raw = Vec::new();
    raw.extend_from_slice(b"WDBC");
    raw.extend_from_slice(&1000u32.to_le_bytes());
    raw.extend_from_slice(&1u32.to_le_bytes());
    raw.extend_from_slice(&4u32.to_le_bytes());
    raw.extend_from_slice(&0u32.to_le_bytes());
    assert!(DbcFile::parse(&raw).is_err());
}

#[test]
fn a_stated_string_block_longer_than_the_file_still_round_trips() {
    // A truncated table: the header says more string bytes than are there. The
    // reader keeps what is there and the writer reproduces the file as read,
    // header word included.
    let mut raw = synthetic();
    let stated = u32::from_le_bytes([raw[16], raw[17], raw[18], raw[19]]);
    raw[16..20].copy_from_slice(&(stated + 64).to_le_bytes());
    let table = DbcFile::parse(&raw).unwrap();
    assert_eq!(table.write(), raw);
}

#[test]
fn bytes_after_the_string_block_are_kept() {
    let mut raw = synthetic();
    raw.extend_from_slice(b"trailing");
    assert_eq!(DbcFile::parse(&raw).unwrap().write(), raw);
}

/// The undo stack over a table, which is the half a panel cannot be tested on.
mod stack {
    use super::*;
    use crate::dbc::cell;
    use crate::undo::History;

    /// One field edit, applied, undone and redone.
    #[test]
    fn a_field_edit_inverts_exactly() {
        let mut table = DbcFile::parse(&synthetic()).unwrap();
        let before = table.write();

        let mut history = History::new();
        let edit = Cell::new(&table, 1, 0, 0xBEEF).unwrap();
        edit.apply(&mut table);
        history.record_cell("Test", edit);

        assert_eq!(table.u32_at(1, 0), Some(0xBEEF));
        let change = history.undo().expect("one entry");
        change.revert_table("Test", &mut table);
        assert_eq!(table.write(), before, "the undo did not restore the bytes");

        let change = history.redo().expect("one entry");
        change.apply_table("Test", &mut table);
        assert_eq!(table.u32_at(1, 0), Some(0xBEEF));
    }

    /// **A run of edits to one field is one entry**, which is what typing into a
    /// box or dragging a number produces.
    #[test]
    fn a_run_of_edits_to_one_field_is_one_entry() {
        let mut table = DbcFile::parse(&synthetic()).unwrap();
        let before = table.write();
        let mut history = History::new();

        for (at, value) in [21u32, 22, 23, 24].iter().enumerate() {
            history.begin_gesture("Edit Id", "Test 1 field 0", at as f64 * 0.1);
            let edit = Cell::new(&table, 1, 0, *value).unwrap();
            edit.apply(&mut table);
            history.record_cell("Test", edit);
            history.end();
        }
        assert_eq!(table.u32_at(1, 0), Some(24));

        let change = history.undo().expect("one entry, not four");
        change.revert_table("Test", &mut table);
        assert_eq!(
            table.write(),
            before,
            "one undo did not take the whole run back"
        );
        assert!(history.undo().is_none(), "the run was more than one entry");
    }

    /// …and two fields inside one gesture are one entry naming both.
    #[test]
    fn two_fields_in_one_change_both_come_back() {
        let mut table = DbcFile::parse(&synthetic()).unwrap();
        let before = table.write();
        let mut history = History::new();

        history.begin("Edit row");
        for field in [0usize, 1] {
            let edit = Cell::new(&table, 0, field, 77).unwrap();
            edit.apply(&mut table);
            history.record_cell("Test", edit);
        }
        history.end();

        let change = history.undo().expect("one entry");
        assert_eq!(change.tables(), vec!["Test".to_string()]);
        assert_eq!(change.records("Test"), vec![0]);
        change.revert_table("Test", &mut table);
        assert_eq!(table.write(), before);
    }

    /// A string edit undoes to the **old offset**, which still points at the old
    /// text: the appended bytes stay in the block and nothing reads them.
    #[test]
    fn a_string_edit_undoes_to_the_text_it_had() {
        let mut table = DbcFile::parse(&synthetic()).unwrap();
        let mut history = History::new();

        let edit = cell::set_text(&mut table, 0, 1, "Fireball").unwrap();
        history.record_cell("Test", edit);
        assert_eq!(table.string_at(0, 1).as_deref(), Some("Fireball"));

        let change = history.undo().expect("one entry");
        change.revert_table("Test", &mut table);
        assert_eq!(table.string_at(0, 1).as_deref(), Some("first"));
        // The other rows never moved, which is the whole point of appending.
        assert_eq!(table.string_at(1, 1).as_deref(), Some("second"));
    }

    /// A write of the value that is already there is not an undo step.
    ///
    /// A panel writes its widget's value back whenever the widget reports a
    /// change, and a drag that ends where it started reports one. Without this
    /// the stack fills with entries that do nothing, and `Ctrl+Z` stops
    /// appearing to work.
    #[test]
    fn a_write_that_changes_nothing_is_not_an_entry() {
        let table = DbcFile::parse(&synthetic()).unwrap();
        let mut history = History::new();
        let edit = Cell::new(&table, 1, 0, 20).unwrap();
        assert!(!edit.moves());
        history.record_cell("Test", edit);
        assert!(history.undo().is_none(), "an empty edit reached the stack");
    }

    /// A change naming a table nobody has open reverts nothing and does not
    /// panic — the editor's own case when a table was closed since.
    #[test]
    fn a_change_about_another_table_is_left_alone() {
        let mut table = DbcFile::parse(&synthetic()).unwrap();
        let before = table.write();
        let mut history = History::new();
        let edit = Cell::new(&table, 1, 0, 99).unwrap();
        edit.apply(&mut table);
        history.record_cell("Other", edit);

        let change = history.undo().expect("one entry");
        change.revert_table("Test", &mut table);
        assert_ne!(table.write(), before, "it reverted the wrong table");
        change.revert_table("Other", &mut table);
        assert_eq!(table.write(), before);
    }

    /// **A discard forgets the table's entries and keeps everybody else's.**
    ///
    /// An entry that was only about the discarded table goes; one that touched
    /// two tables keeps the other table's half; the redo branch is cleared of
    /// it too, so a redo after a discard cannot write the old value back into
    /// a table that no longer has it.
    #[test]
    fn a_discard_forgets_one_table_on_both_branches() {
        let mut table = DbcFile::parse(&synthetic()).unwrap();
        let mut other = DbcFile::parse(&synthetic()).unwrap();
        let mut history = History::new();

        // Three entries: one on `Test`, one on `Other`, one on both.
        let edit = Cell::new(&table, 0, 0, 11).unwrap();
        edit.apply(&mut table);
        history.record_cell("Test", edit);
        let edit = Cell::new(&other, 0, 0, 12).unwrap();
        edit.apply(&mut other);
        history.record_cell("Other", edit);
        history.begin("Both");
        let edit = Cell::new(&table, 1, 0, 13).unwrap();
        edit.apply(&mut table);
        history.record_cell("Test", edit);
        let edit = Cell::new(&other, 1, 0, 14).unwrap();
        edit.apply(&mut other);
        history.record_cell("Other", edit);
        history.end();
        // …and one undone, so the redo branch has something on it.
        let undone = history.undo().expect("the joint entry");
        assert_eq!(undone.label, "Both");
        assert_eq!(history.depth_done(), 2);

        assert_eq!(history.forget_table("Test"), 1, "the entry that was only about Test");
        assert_eq!(history.depth_done(), 1, "the Other entry stays");
        let kept = history.next_redo().expect("the joint entry stays on the redo branch");
        assert_eq!(kept.tables(), vec!["Other".to_string()], "with only its Other half");
        assert!(kept.records("Test").is_empty());

        let change = history.undo().expect("the Other entry");
        assert_eq!(change.tables(), vec!["Other".to_string()]);
        assert_eq!(history.forget_table("Nobody"), 0);
    }
}

/// **An edited table is still a table the client can read.**
///
/// The round trip says an *unedited* table writes back byte for byte, which is
/// the check that the container keeps what it does not understand. This is the
/// other half: after an edit, the bytes are read back through the reader the
/// renderer actually uses — `vale_assets::tables::dbc::Dbc` — rather than
/// through this crate's own, so a writer that produced something only its own
/// parser accepts would fail here.
#[test]
fn an_edited_spell_reads_back_through_the_client_reader() {
    let Some(mut assets) = archives() else {
        return;
    };
    let Ok(raw) = assets.read(r"DBFilesClient\Spell.dbc") else {
        return;
    };
    let mut table = DbcFile::parse(&raw).unwrap();

    // Fireball, rank 1. Its name is field 120 and its mana cost field 32 —
    // `vale_assets::tables::spellbook::spell_fields`, which is where those
    // were measured.
    let record = table.row_of(133).expect("Spell.dbc has no 133");
    assert_eq!(table.string_at(record, 120).as_deref(), Some("Fireball"));
    let was = table.u32_at(record, 32).unwrap();

    assert!(table.set_u32(record, 32, was + 7));
    assert!(table.set_string(record, 120, "Fireball (edited)"));
    let written = table.write();

    let read = vale_assets::tables::dbc::Dbc::parse(&written)
        .expect("the edited table did not parse");
    assert_eq!(read.record_count, table.record_count());
    assert_eq!(read.field_count, table.field_count());
    assert_eq!(read.u32_at(record, 32), Some(was + 7));
    assert_eq!(
        read.string_at(record, 120).as_deref(),
        Some("Fireball (edited)")
    );
    // …and the row after it is untouched, which is what the append-only block
    // is for: a longer name must not have moved anybody else's string.
    let next = record + 1;
    assert_eq!(
        read.string_at(next, 120),
        DbcFile::parse(&raw).unwrap().string_at(next, 120)
    );
}
