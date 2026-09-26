//! **What heading a quest goes under in the log** — `QuestSort.dbc`, and the
//! sign bit that says when it is the answer at all.
//!
//! The quest log is the only panel in the game whose rows are *grouped by the
//! client*. Nothing on the wire says a quest belongs under "Elwynn Forest":
//! `SMSG_QUEST_QUERY_RESPONSE` carries one signed field, `ZoneOrSort`, and the
//! whole rule is what its sign means.
//!
//! ```text
//! ZoneOrSort > 0   an AreaTable id       -> AreaTable.dbc[11]   "Elwynn Forest"
//! ZoneOrSort < 0   a QuestSort id, negated -> QuestSort.dbc[1]  "Blacksmithing"
//! ZoneOrSort = 0   no heading at all
//! ```
//!
//! The two tables are indexed at different field offsets, and swapping them
//! would produce a heading rather than an error. `GetQuestLogTitle`'s header
//! arm is two branches:
//!
//! ```text
//! zoneOrSort > 0   checked against AreaTable's max id
//!                  [row + locale*4 + 0x2c]   field 11 — the area name
//! zoneOrSort < 0   negated, checked against QuestSort's max id
//!                  [row + locale*4 + 0x4]    field 1 — the sort name
//! ```
//!
//! Field 11 is the same column [`crate::tables::area`] already reads and states its own
//! corroboration for. Field 1 here is corroborated against the shipped file the
//! same way: row 0 is id 1 with its name offset resolving to "Epic", row 1 is
//! id 21 resolving to "REUSE - old wailing caverns" — a table of 34 rows whose
//! ids are sparse, which is why it is a map rather than an index.
//!
//! ## The headings are sorted by their *name*
//!
//! The log's builder sorts the distinct headings with a comparator that
//! resolves both sides to a 256-byte name buffer before comparing — so
//! the order on screen is alphabetical by what is drawn, not by id and not by
//! the order the quests were taken in. A zone id of 0 sorts first: the
//! comparator returns -1 outright for it, before either name is built.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

mod fields {
    /// The localised name — the first of the eight locale columns.
    pub const NAME: usize = 1;
}

/// `QuestSort.dbc`, reduced to the one column anything asks for.
#[derive(Debug, Clone, Default)]
pub struct QuestSorts {
    by_id: HashMap<u32, String>,
}

impl QuestSorts {
    /// Parse the table. An absent or damaged file is an empty map rather than
    /// an error: the cost is that the handful of profession and "Epic" headings
    /// draw blank while every zone heading — which is nearly all of them — is
    /// unaffected.
    pub fn parse(raw: &[u8]) -> QuestSorts {
        let Ok(dbc) = Dbc::parse(raw) else {
            return QuestSorts::default();
        };
        let mut by_id = HashMap::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            let Some(id) = dbc.u32_at(record, 0) else {
                continue;
            };
            let name = dbc.string_at(record, fields::NAME).unwrap_or_default();
            by_id.insert(id, name);
        }
        QuestSorts { by_id }
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// The name of one sort row, by its **positive** id.
    pub fn name(&self, id: u32) -> Option<&str> {
        self.by_id.get(&id).map(String::as_str)
    }
}

/// **The heading a `ZoneOrSort` names**, joining the two tables by its sign.
///
/// The one function this module exists for; see the module comment for the two
/// addresses. `None` is a quest the log cannot file — which the reference
/// draws as a heading with an empty name rather than dropping the quest, so
/// callers should keep the row.
pub fn heading<'a>(
    zone_or_sort: i32,
    areas: Option<&'a crate::tables::area::Areas>,
    sorts: &'a QuestSorts,
) -> Option<&'a str> {
    match zone_or_sort {
        0 => None,
        id if id > 0 => areas?.get(id as u32).map(|area| area.name.as_str()),
        // **Negated, not masked.** The field is a plain `int32` and the sort
        // ids are small; reading it as a `u32` and masking would turn sort 1
        // into area 4,294,967,295 and answer nothing at all, silently.
        id => sorts.name(id.unsigned_abs()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sign is the whole rule, and it decides *which table*, so getting it
    /// wrong reads a plausible heading out of the other one.
    #[test]
    fn the_sign_decides_which_table_answers() {
        let sorts = QuestSorts {
            by_id: [(1, "Epic".to_string()), (101, "Tailoring".to_string())]
                .into_iter()
                .collect(),
        };
        assert_eq!(heading(-1, None, &sorts), Some("Epic"));
        assert_eq!(heading(-101, None, &sorts), Some("Tailoring"));
        // A positive id is an area, and with no `AreaTable` there is no answer
        // — which is a blank heading rather than "Epic".
        assert_eq!(heading(1, None, &sorts), None);
        assert_eq!(heading(0, None, &sorts), None, "0 is no heading at all");
        // …and the negation does not go through a mask.
        assert_eq!(heading(-2, None, &sorts), None);
    }

    /// A missing file is an empty table, not a parse failure — the same
    /// documented degradation every optional DBC in this crate takes.
    #[test]
    fn an_absent_table_is_empty_rather_than_an_error() {
        assert!(QuestSorts::parse(&[]).is_empty());
        assert_eq!(QuestSorts::parse(b"not a dbc").len(), 0);
    }
}
