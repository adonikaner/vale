//! `ItemSet.dbc`: which items make up a set, and which bonuses the set grants
//! at how many pieces.
//!
//! An item names its set by id (`ItemPrototype::ItemSet`, sent in
//! `SMSG_ITEM_QUERY_SINGLE_RESPONSE`). The 1.12.1 client then draws a block
//! under the item's spells:
//!
//! ```text
//!                                          a blank line
//! The Gladiator (2/5)                      ITEM_SET_NAME, gold: owned/total
//! Requires Leatherworking (300)            only for a set with a skill
//!   Brutal Gauntlets                       each piece, light yellow if owned,
//!   Brutal Hauberk                         grey if not; a piece whose name
//!   …                                      is not yet known is left out
//!                                          a blank line
//! Set: +10 Defense.                        ITEM_SET_BONUS, green: active
//! (3) Set: …                               ITEM_SET_BONUS_GRAY, grey
//! ```
//!
//! The bonuses are drawn in threshold order, and 82 of the 172 rows store them
//! in another order, so the order is this module's rule rather than the
//! file's. A piece counts as owned when the character has that item equipped in
//! one of the 19 worn slots and it is not broken. A bonus is active when the
//! owned count reaches its threshold and the character meets the set's skill
//! requirement; four sets have one (Bloodvine Garb, Primal Batskin, Blood Tiger
//! Harness and The Darksoul, each at 300).
//!
//! Columns, measured with `vale dbc ItemSet`: 45 fields; 0 id, 1..8 name (8
//! locales), 9 locale flags, 10..26 the items (17), 27..34 the bonus spells
//! (8), 35..42 each bonus's threshold, 43 the required skill line, 44 its rank.
//! Row 1 is "The Gladiator": items 11729, 11726, 11728, 11731, 11730; spells
//! 7514, 9761, 7597, 9140 at thresholds 3, 2, 5, 4.

use std::collections::HashMap;

use super::dbc::Dbc;

mod fields {
    pub const ID: usize = 0;
    pub const NAME: usize = 1;
    pub const ITEMS: usize = 10;
    pub const ITEM_COUNT: usize = 17;
    pub const SPELLS: usize = 27;
    pub const THRESHOLDS: usize = 35;
    pub const BONUS_COUNT: usize = 8;
    pub const REQUIRED_SKILL: usize = 43;
    pub const REQUIRED_SKILL_RANK: usize = 44;
}

/// One bonus: the spell it grants and the pieces it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetBonus {
    pub spell: u32,
    pub threshold: u32,
}

/// One `ItemSet.dbc` row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemSet {
    pub id: u32,
    pub name: String,
    /// The pieces, in the file's column order with empty columns removed.
    pub items: Vec<u32>,
    /// The bonuses with a spell, in threshold order. Equal thresholds keep the
    /// file's column order.
    pub bonuses: Vec<SetBonus>,
    /// The `SkillLine.dbc` row the bonuses need, 0 for none, and the rank.
    pub required_skill: u32,
    pub required_skill_rank: u32,
}

impl ItemSet {
    /// Which pieces are owned, one flag per entry of [`Self::items`].
    ///
    /// `equipped` is the entries in the worn slots, already without broken
    /// items. A piece is owned when its entry is among them.
    pub fn owned(&self, equipped: &[u32]) -> Vec<bool> {
        self.items
            .iter()
            .map(|item| equipped.contains(item))
            .collect()
    }

    /// Whether the character meets the set's skill requirement, given its
    /// rank in the skill (`None` for a character without the skill). A set
    /// without a requirement is always met.
    pub fn skill_met(&self, rank: Option<i32>) -> bool {
        if self.required_skill == 0 {
            return true;
        }
        rank.is_some_and(|rank| rank >= self.required_skill_rank as i32)
    }

    /// Which bonuses are active, one flag per entry of [`Self::bonuses`].
    pub fn active(&self, owned: usize, skill_met: bool) -> Vec<bool> {
        self.bonuses
            .iter()
            .map(|bonus| skill_met && owned as u32 >= bonus.threshold)
            .collect()
    }
}

/// `ItemSet.dbc`, by id.
#[derive(Debug, Clone, Default)]
pub struct ItemSets(HashMap<u32, ItemSet>);

impl ItemSets {
    /// Parse, tolerating an absent or damaged file. Without the table an item
    /// in a set draws no set block.
    pub fn parse(bytes: &[u8]) -> ItemSets {
        let mut sets = HashMap::new();
        let Ok(table) = Dbc::parse(bytes) else {
            return ItemSets(sets);
        };
        for record in 0..table.record_count {
            let Some(id) = table.u32_at(record, fields::ID) else {
                continue;
            };
            let field = |index: usize| table.u32_at(record, index).unwrap_or(0);
            let items = (0..fields::ITEM_COUNT)
                .map(|slot| field(fields::ITEMS + slot))
                .filter(|item| *item != 0)
                .collect();
            let mut bonuses: Vec<SetBonus> = (0..fields::BONUS_COUNT)
                .map(|slot| SetBonus {
                    spell: field(fields::SPELLS + slot),
                    threshold: field(fields::THRESHOLDS + slot),
                })
                .filter(|bonus| bonus.spell != 0)
                .collect();
            // Stable, so equal thresholds keep their column order.
            bonuses.sort_by_key(|bonus| bonus.threshold);
            sets.insert(
                id,
                ItemSet {
                    id,
                    name: table.string_at(record, fields::NAME).unwrap_or_default(),
                    items,
                    bonuses,
                    required_skill: field(fields::REQUIRED_SKILL),
                    required_skill_rank: field(fields::REQUIRED_SKILL_RANK),
                },
            );
        }
        ItemSets(sets)
    }

    pub fn get(&self, id: u32) -> Option<&ItemSet> {
        self.0.get(&id)
    }

    /// Every set, in id order.
    pub fn in_order(&self) -> Vec<&ItemSet> {
        let mut sets: Vec<&ItemSet> = self.0.values().collect();
        sets.sort_by_key(|set| set.id);
        sets
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// The Gladiator as the file stores it: five pieces and four bonuses
    /// whose thresholds are out of order.
    fn gladiator() -> ItemSets {
        let strings = b"\0The Gladiator\0";
        let mut row = vec![0u32; 45];
        row[0] = 1;
        row[1] = 1;
        row[10..15].copy_from_slice(&[11729, 11726, 11728, 11731, 11730]);
        row[27..31].copy_from_slice(&[7514, 9761, 7597, 9140]);
        row[35..39].copy_from_slice(&[3, 2, 5, 4]);
        ItemSets::parse(&dbc(&[row], 45, strings))
    }

    #[test]
    fn a_set_is_its_pieces_and_its_bonuses_in_threshold_order() {
        let sets = gladiator();
        let set = sets.get(1).expect("row 1");
        assert_eq!(set.name, "The Gladiator");
        assert_eq!(set.items, vec![11729, 11726, 11728, 11731, 11730]);
        let thresholds: Vec<u32> = set.bonuses.iter().map(|b| b.threshold).collect();
        assert_eq!(thresholds, vec![2, 3, 4, 5]);
        assert_eq!(set.bonuses[0].spell, 9761);
    }

    /// Three of five pieces worn activates the two- and three-piece bonuses
    /// and leaves the other two grey.
    #[test]
    fn three_pieces_activate_the_bonuses_up_to_three() {
        let sets = gladiator();
        let set = sets.get(1).unwrap();
        let owned = set.owned(&[11726, 11731, 11730, 6948]);
        assert_eq!(owned, vec![false, true, false, true, true]);
        let count = owned.iter().filter(|o| **o).count();
        assert_eq!(set.active(count, true), vec![true, true, false, false]);
        assert_eq!(set.active(1, true), vec![false, false, false, false]);
    }

    /// A set with a skill requirement grants nothing below the rank, however
    /// many pieces are worn.
    #[test]
    fn a_skill_gated_set_needs_the_rank() {
        let set = ItemSet {
            required_skill: 197,
            required_skill_rank: 300,
            bonuses: vec![SetBonus { spell: 1, threshold: 2 }],
            ..ItemSet::default()
        };
        assert!(!set.skill_met(None));
        assert!(!set.skill_met(Some(299)));
        assert!(set.skill_met(Some(300)));
        assert_eq!(set.active(3, set.skill_met(Some(250))), vec![false]);
        assert!(ItemSet::default().skill_met(None), "no requirement is met");
    }

    #[test]
    fn a_missing_table_is_empty() {
        assert!(ItemSets::parse(&[]).is_empty());
    }
}
