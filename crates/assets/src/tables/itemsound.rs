//! **What an item sounds like when it changes hands** — `ItemGroupSounds.dbc`,
//! twenty-four rows of four sound columns, reached from the item's own
//! `ItemDisplayInfo` row.
//!
//! ```text
//! ItemDisplayInfo.dbc  field 11   groupSoundIndex  ->  a row of this table
//! ItemGroupSounds.dbc  field 1    pickUp           ->  a SoundEntries id
//!                      field 2    putDown          ->  a SoundEntries id
//! ```
//!
//! The client's own player takes `(displayId, column)`: it indexes
//! `ItemDisplayInfo` by display id, takes `[row + 0x2c]` — field 11 — bounds-
//! checks it against this table's length, and plays the given column of the row it
//! finds. The column is **0 at three call sites and 1 at one**: picking an item up onto
//! the cursor and looting one are 0, dropping one off the cursor is 1.
//!
//! ## The columns *are* `SoundEntries` ids, and the six rows that say otherwise
//! are the six that are not used
//!
//! This repo recorded the opposite — "whose columns are **not** `SoundEntries`
//! ids (the file names 273/274/275 and `SoundEntries` has no row between 265
//! and 285)" — and that measurement was true of the rows it was taken from and
//! false of the table. Rows **1..6** name 273/274/275, which resolve to nothing;
//! rows **7..24** name 1183..1221, and every one of those is a row of
//! `SoundEntries` called `PickUp*` or `PutDown*` in `Sound\interface\PickUp\`.
//!
//! What settles which half matters is the *other* table: over all 29,604
//! `ItemDisplayInfo` rows, `groupSoundIndex` holds `{0, 3..24}` and **8,274 of
//! them hold 7** — `PickUpCloth_Leather`. The six dead rows are reachable in
//! principle and named by almost nothing, so a reader that stops at row 1 sees
//! a broken table and a reader that looks at the distribution sees a working
//! one. [`ItemGroupSounds::pick_up`] answers `None` for an unresolvable id
//! rather than playing a wrong one; the caller checks the bank.

use crate::tables::dbc::Dbc;

/// Field indices in `ItemGroupSounds.dbc`. Five in 1.12, and the last two are
/// named because a block that accounts for every field is checkable where one
/// naming the two that are used is not.
#[allow(dead_code)]
mod fields {
    /// The row id, which is what `ItemDisplayInfo`'s `groupSoundIndex` holds.
    pub const ID: usize = 0;
    /// Lifting the item — onto the cursor, or out of a corpse.
    pub const PICK_UP: usize = 1;
    /// …and letting go of it.
    pub const PUT_DOWN: usize = 2;
    /// A third sound, set on five of the twenty-four rows and on none of the
    /// eighteen that anything names. Unplayed here for that reason: there is no
    /// call site in the client that reads column 2, so what it is for is not
    /// established.
    pub const DROP: usize = 3;
    /// Zero in every row of the shipped table.
    pub const UNUSED: usize = 4;
    pub const COUNT: usize = 5;
}

/// `ItemGroupSounds.dbc`, indexed by its own row id.
///
/// Small enough (24 rows) that the lookup is a scan of the parsed table rather
/// than a map — and it has to be a lookup rather than an index, because the ids
/// are 1-based and a `groupSoundIndex` of 0 means "this item makes no sound".
#[derive(Default)]
pub struct ItemGroupSounds {
    dbc: Option<Dbc>,
}

/// One of the two columns anything in the client plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemSound {
    /// Column 1 — the item arriving in your hands.
    PickUp,
    /// Column 2 — the item leaving them.
    PutDown,
}

impl ItemGroupSounds {
    /// Parse, tolerating an absent or damaged table: without it items are
    /// silent, which is this client's behaviour before the table was read at
    /// all.
    pub fn parse(bytes: &[u8]) -> ItemGroupSounds {
        ItemGroupSounds {
            dbc: Dbc::parse(bytes)
                .ok()
                .filter(|dbc| dbc.field_count >= fields::COUNT),
        }
    }

    pub fn len(&self) -> usize {
        self.dbc.as_ref().map_or(0, |dbc| dbc.record_count)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every row id in the table, for the survey.
    pub fn ids(&self) -> Vec<u32> {
        let Some(dbc) = self.dbc.as_ref() else {
            return Vec::new();
        };
        (0..dbc.record_count)
            .filter_map(|record| dbc.u32_at(record, fields::ID))
            .collect()
    }

    /// **The `SoundEntries` id one group makes for one of the two gestures**, or
    /// `None` for a group of 0 (no sound), a group with no row, or a row whose
    /// column is empty — which rows 2..6 all are on one side or the other.
    pub fn sound(&self, group: u32, which: ItemSound) -> Option<u32> {
        if group == 0 {
            return None;
        }
        let dbc = self.dbc.as_ref()?;
        let column = match which {
            ItemSound::PickUp => fields::PICK_UP,
            ItemSound::PutDown => fields::PUT_DOWN,
        };
        (0..dbc.record_count)
            .find(|record| dbc.u32_at(*record, fields::ID) == Some(group))
            .and_then(|record| dbc.u32_at(record, column))
            .filter(|id| *id != 0)
    }

    /// `sound(group, PickUp)`, named for the call site.
    pub fn pick_up(&self, group: u32) -> Option<u32> {
        self.sound(group, ItemSound::PickUp)
    }

    /// `sound(group, PutDown)`, named for the call site.
    pub fn put_down(&self, group: u32) -> Option<u32> {
        self.sound(group, ItemSound::PutDown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built table in the shipped file's own shape: the first row is one
    /// of the six dead ones and the second is a live one.
    fn build(rows: &[[u32; fields::COUNT]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(fields::COUNT as u32).to_le_bytes());
        out.extend_from_slice(&((fields::COUNT * 4) as u32).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for row in rows {
            for field in row {
                out.extend_from_slice(&field.to_le_bytes());
            }
        }
        out.push(0);
        out
    }

    /// The two columns come back by id, and a group of 0 is silence rather than
    /// row zero — which is the difference between "this item makes no noise"
    /// and "every item makes the first row's noise".
    #[test]
    fn a_group_answers_its_two_columns_and_zero_answers_nothing() {
        // Row 1 as shipped (273/274/275, none of which resolve) and row 7,
        // `PickUpCloth_Leather` / `PutDownClothLeather`.
        let table = ItemGroupSounds::parse(&build(&[
            [1, 273, 274, 275, 0],
            [7, 1185, 1202, 0, 0],
        ]));
        assert_eq!(table.len(), 2);
        assert_eq!(table.pick_up(7), Some(1185));
        assert_eq!(table.put_down(7), Some(1202));
        // Present but unresolvable is still what the row says — this table does
        // not know about `SoundEntries` and must not pretend to.
        assert_eq!(table.pick_up(1), Some(273));
        // No group, no row, and an empty column.
        assert_eq!(table.pick_up(0), None);
        assert_eq!(table.pick_up(9), None);
        assert_eq!(table.put_down(1).is_some(), true);
    }

    /// An absent table is silence, not a panic — the same degradation every
    /// optional table in this crate documents.
    #[test]
    fn an_absent_table_is_silence() {
        let table = ItemGroupSounds::parse(&[]);
        assert!(table.is_empty());
        assert_eq!(table.pick_up(7), None);
    }
}
