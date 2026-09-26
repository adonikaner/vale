//! What one table changes against another, row by row, keyed by the id in
//! field 0.
//!
//! ## Why a differing dword is checked against the string block
//!
//! A record's bytes are compared first, and equal bytes are an unchanged
//! row: [`DbcFile::set_string`] only appends to the string block, so a table
//! edited by this crate keeps every offset the original had, and a row with
//! equal bytes has equal fields and equal strings. A table written by
//! another tool may carry a rebuilt string block, in which every string
//! field's offset differs while its text does not. So a dword that differs
//! is read as an offset into each file's block, and a row whose differing
//! dwords all resolve to the same text on both sides is unchanged. A dword
//! that is not a valid offset on either side is a changed value.

use super::DbcFile;
use std::collections::HashMap;

/// How an edited table differs from the one it was edited from, as ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableDiff {
    /// Ids in the edited table and not the original, in record order.
    pub added: Vec<u32>,
    /// Ids in the original and not the edited table, in record order.
    pub removed: Vec<u32>,
    /// Ids in both whose fields differ, in the edited table's record order.
    pub changed: Vec<u32>,
}

/// How many ids a line names before it says how many more there are.
const NAMED: usize = 6;

impl TableDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// `3 rows changed (133, 2000001, 2000002), 1 added (2000003)`, or `no
    /// difference`.
    pub fn line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut named = false;
        for (ids, verb) in [
            (&self.changed, "changed"),
            (&self.added, "added"),
            (&self.removed, "removed"),
        ] {
            if ids.is_empty() {
                continue;
            }
            let count = ids.len();
            let noun = match (named, count) {
                (true, _) => "",
                (false, 1) => "row ",
                (false, _) => "rows ",
            };
            parts.push(format!("{count} {noun}{verb} ({})", list(ids)));
            named = true;
        }
        match parts.is_empty() {
            true => "no difference".to_string(),
            false => parts.join(", "),
        }
    }
}

/// The first few ids, then `and N more`.
fn list(ids: &[u32]) -> String {
    let shown: Vec<String> = ids.iter().take(NAMED).map(|id| id.to_string()).collect();
    match ids.len() > NAMED {
        true => format!("{}, and {} more", shown.join(", "), ids.len() - NAMED),
        false => shown.join(", "),
    }
}

/// Compare `edited` against `original`.
pub fn diff(edited: &DbcFile, original: &DbcFile) -> TableDiff {
    let mut out = TableDiff::default();
    let mut before: HashMap<u32, usize> = HashMap::with_capacity(original.record_count());
    for record in 0..original.record_count() {
        if let Some(id) = original.u32_at(record, 0) {
            // The first record under an id stands for it; a shipped table has
            // no duplicate ids.
            before.entry(id).or_insert(record);
        }
    }
    let mut seen: HashMap<u32, ()> = HashMap::with_capacity(edited.record_count());
    for record in 0..edited.record_count() {
        let Some(id) = edited.u32_at(record, 0) else {
            continue;
        };
        if seen.insert(id, ()).is_some() {
            continue;
        }
        match before.get(&id) {
            None => out.added.push(id),
            Some(&was) => {
                if !same_record(edited, record, original, was) {
                    out.changed.push(id);
                }
            }
        }
    }
    for record in 0..original.record_count() {
        if let Some(id) = original.u32_at(record, 0) {
            if !seen.contains_key(&id) && before.get(&id) == Some(&record) {
                out.removed.push(id);
            }
        }
    }
    out
}

/// Whether two records hold the same values, reading a differing dword as a
/// string offset before calling it a change. See the module comment.
fn same_record(a: &DbcFile, ra: usize, b: &DbcFile, rb: usize) -> bool {
    let (Some(bytes_a), Some(bytes_b)) = (a.record_bytes(ra), b.record_bytes(rb)) else {
        return false;
    };
    if bytes_a == bytes_b {
        return true;
    }
    if bytes_a.len() != bytes_b.len() {
        return false;
    }
    let dwords = bytes_a.len() / 4;
    for field in 0..dwords {
        let (Some(va), Some(vb)) = (a.u32_at(ra, field), b.u32_at(rb, field)) else {
            return false;
        };
        if va == vb {
            continue;
        }
        let (Some(ta), Some(tb)) = (a.text_at(va as usize), b.text_at(vb as usize)) else {
            return false;
        };
        if ta != tb {
            return false;
        }
    }
    // A record whose size is not a whole number of dwords compares its tail
    // by bytes.
    bytes_a[dwords * 4..] == bytes_b[dwords * 4..]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three records of two fields, an id and a string offset, over a string
    /// block, with the block and the rows given so a rebuilt block can be
    /// made.
    fn table(rows: &[(u32, u32)], strings: &[u8]) -> DbcFile {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for (id, at) in rows {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&at.to_le_bytes());
        }
        out.extend_from_slice(strings);
        DbcFile::parse(&out).unwrap()
    }

    const STRINGS: &[u8] = b"\0first\0second\0";

    #[test]
    fn an_unchanged_table_has_no_difference() {
        let a = table(&[(10, 1), (20, 7), (30, 0)], STRINGS);
        let d = diff(&a, &a);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(d.line(), "no difference");
    }

    #[test]
    fn added_removed_and_changed_rows_are_named_by_id() {
        let original = table(&[(10, 1), (20, 7), (30, 0)], STRINGS);
        // 20 now says "first", 30 is gone, 40 is new.
        let edited = table(&[(10, 1), (20, 1), (40, 7)], STRINGS);
        let d = diff(&edited, &original);
        assert_eq!(d.changed, vec![20]);
        assert_eq!(d.added, vec![40]);
        assert_eq!(d.removed, vec![30]);
        assert_eq!(d.line(), "1 row changed (20), 1 added (40), 1 removed (30)");
    }

    /// A string block written in another order moves every offset and no
    /// text; the rows are unchanged.
    #[test]
    fn a_rebuilt_string_block_is_not_a_change() {
        let original = table(&[(10, 1), (20, 7)], STRINGS);
        let rebuilt = table(&[(10, 8), (20, 1)], b"\0second\0first\0");
        assert!(diff(&rebuilt, &original).is_empty());
        // ...and a row pointed at other text is.
        let other = table(&[(10, 8), (20, 8)], b"\0second\0first\0");
        assert_eq!(diff(&other, &original).changed, vec![20]);
    }

    #[test]
    fn a_long_list_of_ids_is_cut_short() {
        let original = table(&[], STRINGS);
        let rows: Vec<(u32, u32)> = (1..=9).map(|id| (id, 0)).collect();
        let edited = table(&rows, STRINGS);
        assert_eq!(
            diff(&edited, &original).line(),
            "9 rows added (1, 2, 3, 4, 5, 6, and 3 more)"
        );
    }
}
