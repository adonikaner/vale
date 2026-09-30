//! `SpellItemEnchantment.dbc` and `ItemRandomProperties.dbc`: what an
//! enchantment on an item is called, and what a random suffix is called and
//! adds.
//!
//! An item object carries seven enchantment slots in `ITEM_FIELD_ENCHANTMENT`
//! (see `vale_protocol::play::items::ItemEnchant`). Each non-empty slot is one
//! line on the item's plate, and the line is the enchantment row's name
//! verbatim: "+5 Stamina", "Frostbrand 3", "Deadly Poison". The plate adds a
//! remaining time to a slot the server started a clock on and a charge count to
//! a slot with charges; both rules are here.
//!
//! A random suffix is an `ItemRandomProperties.dbc` row. Its name follows the
//! item's own through `ITEM_SUFFIX_TEMPLATE` ("%s %s"), and its enchantments are
//! the ones the server writes into slots 3..5 of the item it creates, so a
//! carried "of the Bear" item draws its stamina and strength lines through the
//! enchantment slots like any other enchantment.
//!
//! Columns, measured with `vale dbc`:
//!
//! ```text
//! SpellItemEnchantment  24 fields   0 id, 1..3 effect, 4..6 points min,
//!                                   7..9 points max, 10..12 effect argument,
//!                                   13..20 name (8 locales), 21 locale flags,
//!                                   22 item visual, 23 flags
//!                       row 1   "Rockbiter 3"     row 15  "Reinforced Armor +8"
//! ItemRandomProperties  16 fields   0 id, 1 internal name, 2..6 enchantment,
//!                                   7..14 suffix (8 locales), 15 locale flags
//!                       row 5   "of Intellect"    enchantment 79 "+1 Intellect"
//! ```
//!
//! Enchantment columns 5 and 6 of `ItemRandomProperties` are zero on all 2,012
//! rows, and column 4 is set on 22.

use std::collections::HashMap;

use super::dbc::Dbc;

mod enchantment_fields {
    pub const ID: usize = 0;
    pub const NAME: usize = 13;
}

mod property_fields {
    pub const ID: usize = 0;
    pub const ENCHANTMENT: usize = 2;
    pub const ENCHANTMENT_COUNT: usize = 5;
    pub const SUFFIX: usize = 7;
}

/// `SpellItemEnchantment.dbc`, as an id to its name.
#[derive(Debug, Clone, Default)]
pub struct Enchantments(HashMap<u32, String>);

impl Enchantments {
    /// Parse, tolerating an absent or damaged file. Without the table an item
    /// draws no enchantment lines.
    pub fn parse(bytes: &[u8]) -> Enchantments {
        let mut names = HashMap::new();
        if let Ok(table) = Dbc::parse(bytes) {
            for record in 0..table.record_count {
                let (Some(id), Some(name)) = (
                    table.u32_at(record, enchantment_fields::ID),
                    table.string_at(record, enchantment_fields::NAME),
                ) else {
                    continue;
                };
                names.insert(id, name);
            }
        }
        Enchantments(names)
    }

    /// The name an enchantment id draws as. The 1.12.1 client looks up the
    /// absolute value, so a negative id names the same row; see [`ink`] for
    /// what the sign changes.
    pub fn name(&self, id: i32) -> Option<&str> {
        self.0.get(&id.unsigned_abs()).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One `ItemRandomProperties.dbc` row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RandomProperty {
    /// The suffix, "of the Bear". Empty on a row with no name.
    pub suffix: String,
    /// The `SpellItemEnchantment.dbc` rows the suffix grants, zero for an
    /// unused column.
    pub enchantments: [u32; property_fields::ENCHANTMENT_COUNT],
}

/// `ItemRandomProperties.dbc`, by id.
#[derive(Debug, Clone, Default)]
pub struct RandomProperties(HashMap<u32, RandomProperty>);

impl RandomProperties {
    /// Parse, tolerating an absent or damaged file. Without the table an item
    /// keeps its plain name.
    pub fn parse(bytes: &[u8]) -> RandomProperties {
        let mut rows = HashMap::new();
        if let Ok(table) = Dbc::parse(bytes) {
            for record in 0..table.record_count {
                let Some(id) = table.u32_at(record, property_fields::ID) else {
                    continue;
                };
                let mut enchantments = [0; property_fields::ENCHANTMENT_COUNT];
                for (slot, enchantment) in enchantments.iter_mut().enumerate() {
                    *enchantment = table
                        .u32_at(record, property_fields::ENCHANTMENT + slot)
                        .unwrap_or(0);
                }
                rows.insert(
                    id,
                    RandomProperty {
                        suffix: table
                            .string_at(record, property_fields::SUFFIX)
                            .unwrap_or_default(),
                        enchantments,
                    },
                );
            }
        }
        RandomProperties(rows)
    }

    /// One row. The item field is signed; a negative or zero id names no row
    /// in 1.12, whose table has no negative ids.
    pub fn get(&self, id: i32) -> Option<&RandomProperty> {
        u32::try_from(id).ok().and_then(|id| self.0.get(&id))
    }

    /// The suffix a random property id adds to an item's name, or `None` for
    /// no row or an empty suffix. The name is composed through
    /// `ITEM_SUFFIX_TEMPLATE`.
    pub fn suffix(&self, id: i32) -> Option<&str> {
        self.get(id)
            .map(|row| row.suffix.as_str())
            .filter(|suffix| !suffix.is_empty())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The colour an enchantment line is drawn in.
///
/// The 1.12.1 client draws slots 0 (permanent) and 1 (temporary) in green, or
/// in red when the id is negative, and every other slot in white. The random
/// property's enchantments are in slots 3..5, so a suffix's stat lines are
/// white like the item's own stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnchantInk {
    Green,
    Red,
    White,
}

/// The colour of the line for `id` in `slot`. See [`EnchantInk`].
pub fn ink(slot: usize, id: i32) -> EnchantInk {
    if slot >= 2 {
        EnchantInk::White
    } else if id > 0 {
        EnchantInk::Green
    } else {
        EnchantInk::Red
    }
}

/// The unit a remaining time is printed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    Days,
    Hours,
    Minutes,
    Seconds,
}

impl TimeUnit {
    /// The `GlobalStrings.lua` key suffix for this unit: a remaining time is
    /// printed through `<KEY>_DAYS`, `<KEY>_HOURS`, `<KEY>_MIN` or `<KEY>_SEC`
    /// (`ITEM_ENCHANT_TIME_LEFT_MIN`, `ITEM_DURATION_HOURS`).
    pub fn key_suffix(self) -> &'static str {
        match self {
            TimeUnit::Days => "_DAYS",
            TimeUnit::Hours => "_HOURS",
            TimeUnit::Minutes => "_MIN",
            TimeUnit::Seconds => "_SEC",
        }
    }
}

const SECOND_MS: u32 = 1_000;
const MINUTE_MS: u32 = 60 * SECOND_MS;
const HOUR_MS: u32 = 60 * MINUTE_MS;
const DAY_MS: u32 = 24 * HOUR_MS;

/// A remaining time as the 1.12.1 client prints it: the largest unit the time
/// reaches, and a count in that unit.
///
/// With `round_up`, days, hours and minutes are rounded up, so 28 minutes and
/// 10 seconds is "29 min"; seconds are always truncated. Without it every unit
/// is truncated. The item plate prints both an enchantment's time and an
/// item's own duration with `round_up` set.
pub fn time_left(ms: u32, round_up: bool) -> (TimeUnit, u32) {
    let count = |unit: u32| {
        if round_up {
            (ms - 1) / unit + 1
        } else {
            ms / unit
        }
    };
    if ms >= DAY_MS {
        (TimeUnit::Days, count(DAY_MS))
    } else if ms >= HOUR_MS {
        (TimeUnit::Hours, count(HOUR_MS))
    } else if ms >= MINUTE_MS {
        (TimeUnit::Minutes, count(MINUTE_MS))
    } else {
        (TimeUnit::Seconds, ms / SECOND_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    fn row(id: u32, name_offset: u32, width: usize, name_field: usize) -> Vec<u32> {
        let mut row = vec![0u32; width];
        row[0] = id;
        row[name_field] = name_offset;
        row
    }

    /// An enchantment is named by its row, and a negative id names the same
    /// row.
    #[test]
    fn an_enchantment_id_names_its_row_by_absolute_value() {
        let strings = b"\0+5 Stamina\0Frostbrand 3\0";
        let rows = [row(66, 1, 24, 13), row(3, 12, 24, 13)];
        let table = Enchantments::parse(&dbc(&rows, 24, strings));
        assert_eq!(table.name(66), Some("+5 Stamina"));
        assert_eq!(table.name(-66), Some("+5 Stamina"));
        assert_eq!(table.name(3), Some("Frostbrand 3"));
        assert_eq!(table.name(4), None);
        assert_eq!(table.len(), 2);
    }

    /// A random property names its suffix and lists its enchantments, and an
    /// id of zero or below names nothing.
    #[test]
    fn a_random_property_is_a_suffix_and_its_enchantments() {
        let strings = b"\0of the Bear\0";
        let mut bear = row(1100, 0, 16, 7);
        bear[7] = 1;
        bear[2] = 84;
        bear[3] = 70;
        let table = RandomProperties::parse(&dbc(&[bear], 16, strings));
        assert_eq!(table.suffix(1100), Some("of the Bear"));
        assert_eq!(table.get(1100).unwrap().enchantments, [84, 70, 0, 0, 0]);
        assert_eq!(table.suffix(0), None);
        assert_eq!(table.suffix(-1100), None);
    }

    /// No file is no rows rather than a failure.
    #[test]
    fn missing_tables_are_empty() {
        assert!(Enchantments::parse(&[]).is_empty());
        assert!(RandomProperties::parse(&[]).is_empty());
    }

    /// Slots 0 and 1 are green, or red for a negative id; every other slot is
    /// white.
    #[test]
    fn the_first_two_slots_take_the_enchantment_colours() {
        assert_eq!(ink(0, 20), EnchantInk::Green);
        assert_eq!(ink(1, 3), EnchantInk::Green);
        assert_eq!(ink(1, -3), EnchantInk::Red);
        assert_eq!(ink(3, 84), EnchantInk::White);
        assert_eq!(ink(5, 70), EnchantInk::White);
    }

    /// The largest unit reached, rounded up above a minute and truncated
    /// below one.
    #[test]
    fn a_remaining_time_takes_the_largest_unit_it_reaches() {
        assert_eq!(time_left(28 * MINUTE_MS + 10_000, true), (TimeUnit::Minutes, 29));
        assert_eq!(time_left(28 * MINUTE_MS + 10_000, false), (TimeUnit::Minutes, 28));
        assert_eq!(time_left(30 * MINUTE_MS, true), (TimeUnit::Minutes, 30));
        assert_eq!(time_left(59_900, true), (TimeUnit::Seconds, 59));
        assert_eq!(time_left(HOUR_MS, true), (TimeUnit::Hours, 1));
        assert_eq!(time_left(HOUR_MS + 1, true), (TimeUnit::Hours, 2));
        assert_eq!(time_left(3 * DAY_MS, true), (TimeUnit::Days, 3));
        assert_eq!(TimeUnit::Minutes.key_suffix(), "_MIN");
    }
}
