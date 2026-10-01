//! `ItemVisuals.dbc` and `ItemVisualEffects.dbc`: the glows and flames on a
//! held item.
//!
//! An item visual has five effect slots. Each slot names an `ItemVisualEffects`
//! row, and each of those names one model under `Spells\Enchantments\`. The
//! model in slot `n` hangs on attachment point `n` of the held item's own
//! model, not on a point of the wearer. Weapon models carry points 0..4 along
//! the blade (`Axe_2H_Horde_D_01.m2`: 0.33 to 1.08 yards from the grip), so a
//! visual that fills all five slots runs the length of the weapon, and one
//! that fills only slot 3 is a single model near the tip.
//!
//! Two columns name a visual: field 22 of `ItemDisplayInfo`, the item's own,
//! and field 22 of `SpellItemEnchantment`, an enchantment's. [`choose`] is the
//! rule between them.
//!
//! Only a held item takes a visual. The 1.12.1 client passes the display's
//! visual when it builds the model for a weapon, a shield or an off-hand item,
//! and passes none when it builds a helm or a pauldron. Eight display rows
//! carry a visual and name no model, four of them cloaks, and draw nothing.
//!
//! Columns, measured with `vale dbc`:
//!
//! ```text
//! ItemVisuals        6 fields   0 id, 1..5 ItemVisualEffects id per slot
//!                    34 rows    row 25: 45 in all five slots
//!                               row 32: 51 in slot 3 only
//! ItemVisualEffects  2 fields   0 id, 1 model path (.mdx)
//!                    35 rows    row 45: Spells\Enchantments\RedFlame_Low.mdx
//! ```
//!
//! Row 28 of `ItemVisuals` holds 90148992 in slot 0 and 455344256 in slot 3,
//! which name no `ItemVisualEffects` row, and row 61 of `ItemVisualEffects`
//! names the bare directory. Both are read as an empty slot. `vale itemvisual`
//! checks the tables against the archives.

use std::collections::HashMap;

use super::dbc::Dbc;
use super::enchant::Enchantments;

/// How many effect slots an item visual has.
pub const SLOTS: usize = 5;

mod visual_fields {
    pub const ID: usize = 0;
    pub const EFFECT: usize = 1;
}

mod effect_fields {
    pub const ID: usize = 0;
    pub const MODEL: usize = 1;
}

/// One model an item visual hangs on a held item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemEffect {
    /// The attachment id on the held item's model: the slot's index.
    pub point: u32,
    /// The archive path, with `.mdx` read as `.m2`.
    pub path: String,
}

/// `ItemVisuals.dbc` joined to `ItemVisualEffects.dbc`.
#[derive(Debug, Clone, Default)]
pub struct ItemVisuals {
    slots: HashMap<u32, [u32; SLOTS]>,
    models: HashMap<u32, String>,
}

impl ItemVisuals {
    /// Parse both tables, tolerating an absent or damaged file. Without
    /// `ItemVisuals` no item has a visual; without `ItemVisualEffects` every
    /// slot is empty.
    pub fn parse(visuals: &[u8], effects: &[u8]) -> ItemVisuals {
        let mut slots = HashMap::new();
        if let Ok(table) = Dbc::parse(visuals) {
            for record in 0..table.record_count {
                let Some(id) = table.u32_at(record, visual_fields::ID) else {
                    continue;
                };
                let mut row = [0; SLOTS];
                for (slot, effect) in row.iter_mut().enumerate() {
                    *effect = table
                        .u32_at(record, visual_fields::EFFECT + slot)
                        .unwrap_or(0);
                }
                slots.insert(id, row);
            }
        }
        let mut models = HashMap::new();
        if let Ok(table) = Dbc::parse(effects) {
            for record in 0..table.record_count {
                let (Some(id), Some(path)) = (
                    table.u32_at(record, effect_fields::ID),
                    table.string_at(record, effect_fields::MODEL),
                ) else {
                    continue;
                };
                models.insert(id, path);
            }
        }
        ItemVisuals { slots, models }
    }

    /// Whether `id` is an `ItemVisuals` row. This is the test [`choose`]
    /// applies to an item's own visual.
    pub fn contains(&self, id: u32) -> bool {
        self.slots.contains_key(&id)
    }

    /// The five `ItemVisualEffects` ids of one row, as stored.
    pub fn slots(&self, id: u32) -> Option<[u32; SLOTS]> {
        self.slots.get(&id).copied()
    }

    /// The model path an `ItemVisualEffects` row names, as stored (`.mdx`).
    pub fn model(&self, effect: u32) -> Option<&str> {
        self.models.get(&effect).map(String::as_str)
    }

    /// The models visual `id` hangs, one per filled slot, in slot order.
    ///
    /// A slot is empty when it is zero, when it names no `ItemVisualEffects`
    /// row, or when that row names a directory rather than a file. Visual 0
    /// and a visual that is not a row have no models.
    pub fn effects(&self, id: u32) -> Vec<ItemEffect> {
        let Some(row) = self.slots.get(&id) else {
            return Vec::new();
        };
        row.iter()
            .enumerate()
            .filter_map(|(slot, effect)| {
                let path = self.models.get(effect)?;
                if path.is_empty() || path.ends_with('\\') {
                    return None;
                }
                Some(ItemEffect {
                    point: slot as u32,
                    path: crate::world::m2::model_path(path),
                })
            })
            .collect()
    }

    /// Every `ItemVisuals` id, sorted.
    pub fn ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.slots.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Every `ItemVisualEffects` id, sorted.
    pub fn effect_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.models.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// How many `ItemVisuals` rows there are.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// The visual a held item draws: its display's own, or one of its
/// enchantments'.
///
/// `own` is field 22 of the item's `ItemDisplayInfo` row. `enchantments` are
/// the item's enchantment ids in slot order: slot 0 is the permanent
/// enchantment and slot 1 the temporary one. The 1.12.1 client applies three
/// rules, in order:
///
/// 1. When `own` names an `ItemVisuals` row, the item keeps it, and no
///    enchantment changes it. A Fiery War Axe with a Crusader enchantment
///    still burns.
/// 2. Otherwise the first enchantment, in slot order, whose row names a
///    visual gives it. The value is taken as stored and not checked against
///    `ItemVisuals`, so an enchantment whose column names no row draws
///    nothing.
/// 3. Otherwise the item has no visual, and 0 is returned.
///
/// The client applies the same rule to an item it holds an object for and to
/// another player's visible item, whose update fields carry the enchantment
/// ids.
pub fn choose(
    own: u32,
    enchantments: &[u32],
    visuals: &ItemVisuals,
    table: &Enchantments,
) -> u32 {
    if visuals.contains(own) {
        return own;
    }
    enchantments
        .iter()
        .map(|id| table.item_visual(*id))
        .find(|visual| *visual != 0)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// A string block holding `names`, and the offset of each.
    fn string_block(names: &[&str]) -> (Vec<u8>, Vec<u32>) {
        let mut block = vec![0u8];
        let mut offsets = Vec::new();
        for name in names {
            offsets.push(block.len() as u32);
            block.extend_from_slice(name.as_bytes());
            block.push(0);
        }
        (block, offsets)
    }

    /// Visual 25 fills all five slots with effect 45; visual 32 fills slot 3
    /// with effect 51; visual 28 names two effects that are not rows; visual
    /// 61 names effect 61, whose path is the bare directory.
    fn visuals() -> ItemVisuals {
        let rows = vec![
            vec![25, 45, 45, 45, 45, 45],
            vec![32, 0, 0, 0, 51, 0],
            vec![28, 90148992, 0, 0, 455344256, 47],
            vec![61, 0, 0, 0, 61, 0],
        ];
        let (strings, at) = string_block(&[
            "Spells\\Enchantments\\RedFlame_Low.mdx",
            "Spells\\Enchantments\\Shaman_Fire.mdx",
            "Spells\\Enchantments\\Sparkle_A.mdx",
            "Spells\\Enchantments\\",
        ]);
        let effects = vec![
            vec![45, at[0]],
            vec![51, at[1]],
            vec![47, at[2]],
            vec![61, at[3]],
        ];
        ItemVisuals::parse(&dbc(&rows, 6, b"\0"), &dbc(&effects, 2, &strings))
    }

    /// An enchantment table where 1900 (Crusader) names visual 103, 5
    /// (Flametongue 1) names visual 32, and 20 (a stat enchantment) names none.
    fn enchantments() -> Enchantments {
        let (strings, at) = string_block(&["Crusader", "Flametongue 1", "+5 Stamina"]);
        let row = |id: u32, name: u32, visual: u32| {
            let mut row = vec![0u32; 24];
            row[0] = id;
            row[13] = name;
            row[22] = visual;
            row
        };
        let rows = [row(1900, at[0], 103), row(5, at[1], 32), row(20, at[2], 0)];
        Enchantments::parse(&dbc(&rows, 24, &strings))
    }

    /// Slot `n` hangs on point `n`, and the path is read as `.m2`.
    #[test]
    fn each_filled_slot_hangs_on_the_point_of_its_index() {
        let table = visuals();
        let all = table.effects(25);
        assert_eq!(all.len(), 5);
        assert_eq!(
            all.iter().map(|e| e.point).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        assert_eq!(all[0].path, "Spells\\Enchantments\\RedFlame_Low.m2");
        let tip = table.effects(32);
        assert_eq!(
            tip,
            [ItemEffect {
                point: 3,
                path: "Spells\\Enchantments\\Shaman_Fire.m2".into()
            }]
        );
    }

    /// An effect id that is not a row, and a row naming only the directory,
    /// are empty slots; a visual that is not a row has no models.
    #[test]
    fn a_slot_that_names_no_model_is_empty() {
        let table = visuals();
        let sparkle = table.effects(28);
        assert_eq!(sparkle.len(), 1);
        assert_eq!(sparkle[0].point, 4);
        assert!(table.effects(61).is_empty());
        assert!(table.effects(0).is_empty());
        assert!(table.effects(u32::MAX).is_empty());
    }

    /// The display's own visual wins when it is a row, whatever the
    /// enchantments say.
    #[test]
    fn an_items_own_visual_is_kept_over_an_enchantment() {
        assert_eq!(choose(25, &[1900, 0], &visuals(), &enchantments()), 25);
    }

    /// Without an own visual, the first enchantment in slot order that names
    /// one gives it, and an enchantment with none is passed over.
    #[test]
    fn otherwise_the_first_enchantment_with_a_visual_gives_it() {
        let (table, names) = (visuals(), enchantments());
        assert_eq!(choose(0, &[1900, 5], &table, &names), 103);
        assert_eq!(choose(0, &[20, 5], &table, &names), 32);
        assert_eq!(choose(0, &[0, 5, 1900], &table, &names), 32);
        // An own visual that is not a row does not count as one.
        assert_eq!(choose(u32::MAX, &[5], &table, &names), 32);
        assert_eq!(choose(0, &[20, 0], &table, &names), 0);
        assert_eq!(choose(0, &[], &table, &names), 0);
    }

    /// No files are no rows rather than a failure.
    #[test]
    fn missing_tables_are_empty() {
        let table = ItemVisuals::parse(&[], &[]);
        assert!(table.is_empty());
        assert!(table.effects(25).is_empty());
    }
}
