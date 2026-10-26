//! The sounds an item makes when it is picked up and put down:
//! `ItemGroupSounds.dbc`, 24 rows of 5 fields, reached from the item's own
//! `ItemDisplayInfo` row.
//!
//! ```text
//! ItemDisplayInfo.dbc  field 11   groupSoundIndex  ->  a row of this table
//! ItemGroupSounds.dbc  field 1    pickUp           ->  a SoundEntries id
//!                      field 2    putDown          ->  a SoundEntries id
//! ```
//!
//! The 1.12.1 client plays field 1 when an item is picked up onto the cursor
//! and when it is looted, and field 2 when it is put down off the cursor.
//!
//! ## Rows 1 to 6 name no sound
//!
//! Rows 1 to 6 hold 273, 274 and 275, which are not rows of `SoundEntries`.
//! Rows 7 to 24 hold 1183 to 1221, each a `SoundEntries` row named `PickUp*`
//! or `PutDown*` in `Sound\interface\PickUp\`. Over the 29,604
//! `ItemDisplayInfo` rows, `groupSoundIndex` holds 0 or one of 3 to 24, and
//! 8,274 rows hold 7, `PickUpCloth_Leather`. 40 rows hold 3 to 6 and reach no
//! sound; no row holds 1 or 2. [`ItemGroupSounds::pick_up`] answers the id
//! the row holds whether or not it resolves; the caller checks the bank.

use crate::tables::dbc::Dbc;

/// Field indices in `ItemGroupSounds.dbc`, all five, so the schema can be
/// checked against every one.
#[allow(dead_code)]
pub(crate) mod fields {
    /// The row id, which is what `ItemDisplayInfo`'s `groupSoundIndex` holds.
    pub const ID: usize = 0;
    /// The sound of picking the item up: onto the cursor, or from a corpse.
    pub(crate) const PICK_UP: usize = 1;
    /// The sound of putting it down.
    pub(crate) const PUT_DOWN: usize = 2;
    /// A third sound column, set on rows 1, 2, 4, 5 and 6 to 273, 274 or 275,
    /// none of which is a `SoundEntries` row. The 1.12.1 client does not
    /// read it, so what it is for is not established.
    pub const THIRD: usize = 3;
    /// Zero in every row of the shipped table.
    pub(crate) const UNUSED: usize = 4;
    pub const COUNT: usize = 5;
}

/// `ItemGroupSounds.dbc`, indexed by its own row id.
///
/// A lookup by id is a scan of the 24 rows rather than a map. It cannot be an
/// index, because the ids start at 1 and a `groupSoundIndex` of 0 means the
/// item makes no sound.
#[derive(Default)]
pub struct ItemGroupSounds {
    dbc: Option<Dbc>,
}

/// One of the two columns the client plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemSound {
    /// Field 1: the item picked up.
    PickUp,
    /// Field 2: the item put down.
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

    /// The `SoundEntries` id the row `group` holds for `which`, or `None` for a
    /// group of 0 (no sound), a group with no row, or a column that holds 0, as
    /// one of the two does in each of rows 2 to 6.
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

    /// A table of `rows`, in the shipped file's layout of five fields.
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

    /// A row's two columns are found by its id, and a group of 0 answers no
    /// sound rather than the first row's.
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
        // An id that is not a `SoundEntries` row is answered as the row holds
        // it; this table does not read `SoundEntries`.
        assert_eq!(table.pick_up(1), Some(273));
        // No group, no row, and an empty column.
        assert_eq!(table.pick_up(0), None);
        assert_eq!(table.pick_up(9), None);
        assert_eq!(table.put_down(1).is_some(), true);
    }

    /// An absent table answers no sound and does not panic, as every optional
    /// table in this crate does.
    #[test]
    fn an_absent_table_is_silence() {
        let table = ItemGroupSounds::parse(&[]);
        assert!(table.is_empty());
        assert_eq!(table.pick_up(7), None);
    }
}
