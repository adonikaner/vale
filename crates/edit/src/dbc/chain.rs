//! Copying a row, and copying a row with everything it points at.
//!
//! ## Why a deep copy and not a row copy
//!
//! Forty-seven spells name Fireball's `SpellVisual`. Editing one of its kits in
//! place edits all forty-seven, and a row copy of the visual does not help: the
//! copy still names the same kits, and the kits the same effects. What a person
//! wants when they say "let me change this spell's visual" is a visual of their
//! own, with kits of their own, with effects of their own, every reference
//! between them rewired to the copies, and the original left as it was. That is
//! [`clone_chain`].
//!
//! ## The schema says what is a reference; this decides nothing about a spell
//!
//! Which columns of `SpellVisual` point at `SpellVisualKit` is a rule about what
//! the file means and lives in `vale_assets::tables::schema`. This walks the
//! `Kind::Reference` columns the schema states, follows those whose target is
//! in the caller's `deep` list, and copies the rest of the row as bytes. Point it
//! at a different table with a schema and it copies that instead.
//!
//! A reference the target table has no row for (`SpellVisualKit` names 27 sound
//! ids `SoundEntries` does not carry) is left as it is. It was dangling before
//! the copy and it is dangling after, and inventing a row for it would be a
//! change to the file nobody asked for.
//!
//! ## What comes back is the edits, already applied
//!
//! The tables are written as the walk goes, because a kit's new id has to
//! exist before the visual's column can be pointed at it. The rows and cells
//! that did it come back in the order they were made, which is the order
//! `undo::Change` applies them in and the reverse of the order it reverts them
//! in, so one deep copy is one entry on the stack.

use super::{cell, Cell, DbcFile, Row};
use vale_assets::tables::schema::{self, Kind};
use std::collections::HashMap;

/// What a copy produced: the edits for the stack, and the id map.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Cloned {
    /// Every record added, with its table, in the order it was added.
    pub rows: Vec<(String, Row)>,
    /// Every field rewired, with its table, in the order it was written. A
    /// cloned row's reference columns are written after the row is in, so each
    /// cell's `before` is the original's id and `after` the copy's.
    pub cells: Vec<(String, Cell)>,
    /// `(table, original id, copy id)` for every row copied, root first.
    pub ids: Vec<(String, u32, u32)>,
}

impl Cloned {
    /// The copy's id for a row that was copied, or `None`.
    pub fn new_id(&self, table: &str, old: u32) -> Option<u32> {
        self.ids
            .iter()
            .find(|(had, from, _)| had == table && *from == old)
            .map(|(_, _, to)| *to)
    }

    /// How many rows of `table` were copied.
    pub fn count(&self, table: &str) -> usize {
        self.ids.iter().filter(|(had, _, _)| had == table).count()
    }
}

/// Copy one row of `table` to a new id, appending it. The copy is the
/// original's bytes with field 0 replaced, so every column, including the ones
/// no schema names, comes across.
///
/// `suffix`, when given, is appended to the row's label column (the first
/// `Kind::Text` column its schema has) so the copy can be told from the
/// original in a list: `Fireball (copy)`. A table with no schema or no text
/// column gets no suffix.
///
/// `None` when the table is not open or has no row with that id.
pub fn clone_row(
    tables: &mut HashMap<String, DbcFile>,
    table: &str,
    id: u32,
    suffix: Option<&str>,
) -> Option<Cloned> {
    let mut out = Cloned::default();
    copy_row(tables, table, id, suffix, &mut out)?;
    Some(out)
}

/// Copy a row and, recursively, every row it references in a table named in
/// `deep`, rewiring each reference to its copy.
///
/// A row referenced twice (the same kit in two slots, the same effect on two
/// kits) is copied once and both references are rewired to the one copy, which
/// keeps the copy's shape the same as the original's.
///
/// `suffix` is applied to every copied row's label column, on [`clone_row`]'s
/// terms. `None` when the root table has no such row, or no schema: a table
/// with no schema has no references to follow, and a copy of its row alone is
/// [`clone_row`]'s job.
pub fn clone_chain(
    tables: &mut HashMap<String, DbcFile>,
    table: &str,
    id: u32,
    deep: &[&str],
    suffix: Option<&str>,
) -> Option<Cloned> {
    schema::for_table(table)?;
    let mut out = Cloned::default();
    copy_deep(tables, table, id, deep, suffix, &mut out)?;
    Some(out)
}

/// One row copied, its edits pushed onto `out`. Returns the copy's id and
/// record index.
fn copy_row(
    tables: &mut HashMap<String, DbcFile>,
    table: &str,
    id: u32,
    suffix: Option<&str>,
    out: &mut Cloned,
) -> Option<(u32, usize)> {
    let file = tables.get_mut(table)?;
    let from = file.row_of(id)?;
    let mut bytes = file.record_bytes(from)?.to_vec();
    let new_id = file.max_id().checked_add(1)?;
    bytes[..super::file::FIELD_LEN].copy_from_slice(&new_id.to_le_bytes());
    let at = file.push_record(&bytes)?;
    out.rows.push((table.to_string(), Row::added(file, at)?));
    out.ids.push((table.to_string(), id, new_id));

    if let (Some(suffix), Some(field)) = (suffix, label_field(table)) {
        if let Some(name) = file.string_at(at, field) {
            let renamed = match name.is_empty() {
                true => String::new(),
                false => format!("{name}{suffix}"),
            };
            if renamed != name {
                if let Some(edit) = cell::set_text(file, at, field, &renamed) {
                    out.cells.push((table.to_string(), edit));
                }
            }
        }
    }
    Some((new_id, at))
}

/// [`copy_row`], then every reference column into a `deep` table followed and
/// rewired. Memoised on `out.ids`, so a row referenced twice is copied once.
fn copy_deep(
    tables: &mut HashMap<String, DbcFile>,
    table: &str,
    id: u32,
    deep: &[&str],
    suffix: Option<&str>,
    out: &mut Cloned,
) -> Option<u32> {
    if let Some(done) = out.new_id(table, id) {
        return Some(done);
    }
    let (new_id, at) = copy_row(tables, table, id, suffix, out)?;
    let Some(schema) = schema::for_table(table) else {
        return Some(new_id);
    };
    for column in schema.columns.iter() {
        let Kind::Reference(target) = column.kind else {
            continue;
        };
        if !deep.contains(&target) {
            continue;
        }
        let Some(old) = tables.get(table).and_then(|file| file.u32_at(at, column.field)) else {
            continue;
        };
        // Zero and -1 are the two spellings of "nothing"; a target the table
        // does not carry is left dangling as it was.
        if old == 0 || old == u32::MAX {
            continue;
        }
        if tables.get(target).and_then(|file| file.row_of(old)).is_none() {
            continue;
        }
        let Some(copied) = copy_deep(tables, target, old, deep, suffix, out) else {
            continue;
        };
        let file = tables.get_mut(table)?;
        if let Some(edit) = Cell::new(file, at, column.field, copied) {
            edit.apply(file);
            out.cells.push((table.to_string(), edit));
        }
    }
    Some(new_id)
}

/// The column a row of `table` is named by: its schema's first text column.
fn label_field(table: &str) -> Option<usize> {
    schema::for_table(table)?
        .columns
        .iter()
        .find(|column| column.kind == Kind::Text)
        .map(|column| column.field)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of `fields` words per record, records given as rows of words.
    fn table(fields: u32, rows: &[&[u32]], strings: &[u8]) -> DbcFile {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&fields.to_le_bytes());
        out.extend_from_slice(&(fields * 4).to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for row in rows {
            assert_eq!(row.len() as u32, fields);
            for word in row.iter() {
                out.extend_from_slice(&word.to_le_bytes());
            }
        }
        out.extend_from_slice(strings);
        DbcFile::parse(&out).unwrap()
    }

    /// The three chain tables at their real widths, small enough to reason
    /// about: one visual whose precast and cast slots name the same kit, an
    /// impact kit of its own, and effects shared between the kits.
    fn chain() -> HashMap<String, DbcFile> {
        let mut tables = HashMap::new();
        // SpellVisual: 16 fields. Slots 1 and 2 name kit 10, slot 3 kit 11, the
        // missile model (7) effect 100.
        let mut visual = vec![0u32; 16];
        visual[0] = 5;
        visual[1] = 10;
        visual[2] = 10;
        visual[3] = 11;
        visual[7] = 100;
        visual[10] = 7; // a sound, which is not deep
        tables.insert("SpellVisual".to_string(), table(16, &[&visual], b"\0"));
        // SpellVisualKit: 35 fields. Kit 10 hangs effect 100 on the head and
        // 101 on the chest; kit 11 hangs 100 on the base and names effect 999,
        // which does not exist.
        let mut kit10 = vec![0u32; 35];
        kit10[0] = 10;
        kit10[3] = 100;
        kit10[4] = 101;
        let mut kit11 = vec![0u32; 35];
        kit11[0] = 11;
        kit11[5] = 100;
        kit11[6] = 999;
        tables.insert("SpellVisualKit".to_string(), table(35, &[&kit10, &kit11], b"\0"));
        // SpellVisualEffectName: 5 fields; field 1 is the name.
        let strings = b"\0glow\0burst\0";
        let effect100 = [100u32, 1, 0, 0, 0];
        let effect101 = [101u32, 6, 0, 0, 0];
        tables.insert(
            "SpellVisualEffectName".to_string(),
            table(5, &[&effect100, &effect101], strings),
        );
        tables
    }

    #[test]
    fn a_row_copy_is_the_same_bytes_under_a_new_id() {
        let mut tables = chain();
        let done = clone_row(&mut tables, "SpellVisualKit", 10, None).unwrap();
        assert_eq!(done.ids, vec![("SpellVisualKit".to_string(), 10, 12)]);
        let kits = &tables["SpellVisualKit"];
        assert_eq!(kits.record_count(), 3);
        assert_eq!(kits.u32_at(2, 0), Some(12));
        assert_eq!(kits.u32_at(2, 3), Some(100), "the copy still names the same effect");
        assert_eq!(done.rows.len(), 1);
        assert!(done.cells.is_empty(), "no label column, so nothing to rename");
    }

    #[test]
    fn a_deep_copy_rewires_every_reference_and_copies_a_shared_row_once() {
        let mut tables = chain();
        let done = clone_chain(
            &mut tables,
            "SpellVisual",
            5,
            &["SpellVisualKit", "SpellVisualEffectName"],
            Some(" (copy)"),
        )
        .unwrap();
        assert_eq!(done.count("SpellVisual"), 1);
        assert_eq!(done.count("SpellVisualKit"), 2, "kit 10 is in two slots and is copied once");
        assert_eq!(done.count("SpellVisualEffectName"), 2, "effect 100 is on both kits and the missile");

        let visual = &tables["SpellVisual"];
        let copy = visual.row_of(6).expect("the visual's copy is id 6");
        let kit_a = visual.u32_at(copy, 1).unwrap();
        assert_eq!(visual.u32_at(copy, 2), Some(kit_a), "both slots name the one copy");
        assert_ne!(kit_a, 10);
        let kit_b = visual.u32_at(copy, 3).unwrap();
        assert_ne!(kit_b, 11);
        assert_ne!(kit_a, kit_b);
        assert_eq!(visual.u32_at(copy, 10), Some(7), "a sound is not deep and is kept");
        let missile = visual.u32_at(copy, 7).unwrap();
        assert_eq!(done.new_id("SpellVisualEffectName", 100), Some(missile));

        let kits = &tables["SpellVisualKit"];
        let a = kits.row_of(kit_a).unwrap();
        assert_eq!(kits.u32_at(a, 3), Some(missile), "the head effect is the copy the missile is");
        assert_eq!(kits.u32_at(a, 4), done.new_id("SpellVisualEffectName", 101));
        let b = kits.row_of(kit_b).unwrap();
        assert_eq!(kits.u32_at(b, 5), Some(missile));
        assert_eq!(kits.u32_at(b, 6), Some(999), "a dangling reference stays dangling");

        let effects = &tables["SpellVisualEffectName"];
        let glow = effects.row_of(missile).unwrap();
        assert_eq!(effects.string_at(glow, 1).as_deref(), Some("glow (copy)"));
        // The originals are untouched.
        assert_eq!(visual.u32_at(0, 1), Some(10));
        assert_eq!(effects.string_at(0, 1).as_deref(), Some("glow"));
    }

    #[test]
    fn the_edits_come_back_in_an_order_the_stack_can_replay() {
        let mut tables = chain();
        let before = tables.clone();
        let done = clone_chain(
            &mut tables,
            "SpellVisual",
            5,
            &["SpellVisualKit", "SpellVisualEffectName"],
            None,
        )
        .unwrap();
        let mut change = crate::undo::Change::new("Clone chain");
        for (table, row) in done.rows {
            change.absorb_row(&table, row);
        }
        for (table, cell) in done.cells {
            change.absorb_cell(&table, cell);
        }
        for name in change.tables() {
            change.revert_table(&name, tables.get_mut(&name).unwrap());
        }
        assert_eq!(tables, before, "reverting the copy leaves the tables as they were");
        for name in change.tables() {
            change.apply_table(&name, tables.get_mut(&name).unwrap());
        }
        assert_eq!(tables["SpellVisual"].record_count(), 2);
        assert_eq!(tables["SpellVisualKit"].record_count(), 4);
        assert_eq!(tables["SpellVisualEffectName"].record_count(), 4);
        assert_eq!(tables["SpellVisual"].u32_at(1, 1), tables["SpellVisual"].u32_at(1, 2));
    }

    #[test]
    fn a_missing_row_or_an_unschemad_table_copies_nothing() {
        let mut tables = chain();
        assert!(clone_row(&mut tables, "SpellVisual", 99, None).is_none());
        assert!(clone_chain(&mut tables, "NoSuchTable", 1, &[], None).is_none());
        assert_eq!(tables, chain());
    }
}
