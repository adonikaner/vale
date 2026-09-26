//! **What a pet *is*, in four tables the wire never mentions.**
//!
//! Everything on the pet paper doll that is not a unit field comes from here,
//! and none of it is derivable from anything the server sends: the happiness
//! bands, the damage percentage under them, the loyalty rate, the family's name
//! and icon, and the diet.
//!
//! ```text
//! PetPersonality.dbc  the three happiness bands, and the damage and loyalty
//!                     each band is worth — GetPetHappiness
//! CreatureFamily.dbc  the family's name, its icon, and its food mask —
//!                     UnitCreatureFamily, GetPetIcon, GetPetFoodTypes
//! ItemPetFood.dbc     …and what the bits of that mask are called
//! PetLoyalty.dbc      the six loyalty levels, by name — GetPetLoyalty
//! StableSlotPrices.dbc  what the next stable slot costs —
//!                     GetNextStableSlotCost
//! ```
//!
//! ## The field indices, and how each was pinned
//!
//! The offsets in the doc comments are byte offsets into a DBC record, so
//! `+0xN` is field `N/4`.
//!
//! **`GetPetHappiness`** is the whole of the rule:
//!
//! ```text
//! the creature template's +0x24     indexes PetPersonality.dbc
//! a row it has no record for        falls back to id 1, "Personality: Standard"
//! UNIT_FIELD_POWER5                 the happiness value
//! the three thresholds, fields 10..12   …counted, giving a level of 0..3
//! the damage, fields 13..15         …times 100
//! the loyalty rate, fields 16..18
//! ```
//!
//! The shipped table has two rows and the
//! standard one reads exactly what a player of 1.12 remembers: thresholds
//! **0 / 333,000 / 666,000**, damage **75% / 100% / 125%**, loyalty
//! **-10 / +5 / +20**.
//!
//! **The family and the personality** are two adjacent
//! fields of the creature template (`+0x1c` and `+0x24`) — pinned from the far
//! end rather than by counting: the first indexes a table whose *name* is at
//! field 8, which is `CreatureFamily.dbc` and nothing else, and the second
//! indexes the file named above.
//!
//! **And vmangos sends a literal zero in the personality slot** — its own
//! comment calls the field reserved and says "clients expect it as 0" — so
//! against this server the fallback *is* the behaviour. That is why the
//! fallback is [`Personalities::happiness`]'s own first line rather than a
//! caller's `unwrap_or`: the ordinary path is the one nothing takes.
//!
//! ## The stable's prices are the fifth table, and the row is the slot
//!
//! `StableSlotPrices.dbc` is an id and a price and nothing else, and the id
//! *is* the slot being bought: row 1 is the first bought slot at 500 copper and
//! row 2 the second at 50,000. The server charges from the same row
//! (`HandleBuyStableSlot` looks up `m_stableSlots + 1`), so a client that reads
//! it agrees with the money that is taken. Nothing about the stable crosses the
//! wire as a price.
//!
//! ## …and the two the food list is made of
//!
//! `GetPetFoodTypes` walks **every** `ItemPetFood.dbc` row and
//! keeps the ones whose `1 << (id - 1)` is in the family's mask
//! (`CreatureFamily.dbc` field 7, read at `+0x1c`). So the answer is a *list*
//! and its order is the food table's rather than the mask's, which is what
//! `BuildListString` then joins with commas.

use std::collections::HashMap;

use crate::tables::dbc::Dbc;

/// `PetPersonality.dbc` field indices — see the module comment for the
/// addresses each was read at.
mod personality_fields {
    /// `+0x28`, `+0x2c`, `+0x30`: the happiness value each band starts at.
    pub const THRESHOLDS: usize = 10;
    /// `+0x34`..`+0x3c`: the damage multiplier in each band.
    pub const DAMAGE: usize = 13;
    /// `+0x40`..`+0x48`: the loyalty gained (or lost) per tick in each band.
    pub const LOYALTY: usize = 16;
}

/// `CreatureFamily.dbc` field indices.
mod family_fields {
    /// `+0x14` and `+0x18` — the family's two skill lines (`Pet - Wolf`, and
    /// the shared pet line), which is where a pet spell's `SkillLineAbility`
    /// row is looked up: the client tries each in turn. Read for the craft
    /// window's training-point cost.
    pub const SKILL_LINES: usize = 5;
    /// `+0x1c` — a bitmask of `ItemPetFood.dbc` ids, `1 << (id - 1)`.
    pub const FOOD_MASK: usize = 7;
    /// `+0x20`, localised — "Wolf", "Cat", "Boar".
    pub const NAME: usize = 8;
    /// `+0x44`, **not** localised — `Interface\Icons\Ability_Hunter_Pet_Wolf`.
    pub const ICON: usize = 17;
}

/// The name field of `ItemPetFood.dbc` and `PetLoyalty.dbc`, both of which are
/// an id and a localised name and nothing else — `+0x4` in each.
const NAME_FIELD: usize = 1;

/// The price field of `StableSlotPrices.dbc`, whose two fields are the slot id
/// and the cost in copper.
const PRICE_FIELD: usize = 1;

/// **The id the client falls back to** when the creature template names a
/// personality this table has no row for — the client reads `table[1]`
/// rather than returning.
///
/// It is not a defensive default: vmangos sends 0 in that slot for every
/// creature in the game, so this is the row every pet in a live session
/// actually uses.
pub const DEFAULT_PERSONALITY: u32 = 1;

/// How many happiness bands there are. Three, and a pet below the first is
/// level 0 — which `PetFrame_SetHappiness` draws no icon for, since its
/// `if` chain starts at 1.
pub const BANDS: usize = 3;

/// The damage percentage a pet below the first band does, and the loyalty it
/// gains — `100.0` and `0.0`, for both the no-pet case and the level-0 one.
pub const NO_BAND: (f32, f32) = (100.0, 0.0);

/// One row of `PetPersonality.dbc`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Personality {
    /// The happiness value each band begins at, ascending.
    pub thresholds: [i32; BANDS],
    /// …the damage multiplier in each, as the file stores it (0.75, 1.0, 1.25).
    pub damage: [f32; BANDS],
    /// …and the loyalty rate, which is negative in the lowest band.
    pub loyalty: [f32; BANDS],
}

impl Personality {
    /// **Which band `happiness` is in** — 0 for below the first, 3 for the top.
    ///
    /// The client's loop exactly: count the thresholds the value is not below,
    /// stopping at three. Signed, because the client's comparison is signed.
    pub fn band(&self, happiness: i32) -> usize {
        self.thresholds
            .iter()
            .take_while(|threshold| happiness >= **threshold)
            .count()
    }
}

/// One row of `CreatureFamily.dbc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    pub name: String,
    /// The icon path, whole — this column is a path and not an icon *name*, so
    /// nothing has to be prefixed onto it.
    pub icon: String,
    /// A mask of `1 << (food id - 1)`.
    pub food_mask: u32,
    /// The two skill lines a pet of this family learns under — see
    /// [`family_fields::SKILL_LINES`]. Zero where the file states none.
    pub skill_lines: [u32; 2],
}

/// **The four tables, together**, because every read here needs at least two of
/// them and the panel that asks needs all four.
#[derive(Debug, Clone, Default)]
pub struct PetTables {
    personalities: HashMap<u32, Personality>,
    families: HashMap<u32, Family>,
    /// `ItemPetFood.dbc` in **file order**, which is the order the list comes
    /// out in — see the module comment.
    foods: Vec<(u32, String)>,
    loyalties: HashMap<u32, String>,
    /// `StableSlotPrices.dbc`, by slot — see [`Self::stable_slot_cost`].
    stable_prices: HashMap<u32, u32>,
}

impl PetTables {
    /// Build from the four files. **Any of them may be absent**, and each
    /// absence is its own degradation rather than an error: no
    /// `PetPersonality.dbc` is a happiness icon that never shows, no
    /// `CreatureFamily.dbc` is a pet with no family name and no icon, no
    /// `ItemPetFood.dbc` is an empty diet line, and no `PetLoyalty.dbc` is a
    /// blank loyalty label. All four are `Optional` in the DBC chain for that
    /// reason.
    pub fn parse(
        personality: &[u8],
        family: &[u8],
        food: &[u8],
        loyalty: &[u8],
        stable_prices: &[u8],
    ) -> PetTables {
        let mut tables = PetTables::default();
        if let Ok(dbc) = Dbc::parse(personality) {
            for row in 0..dbc.record_count {
                let Some(id) = dbc.u32_at(row, 0) else { continue };
                let three = |base: usize| {
                    [0, 1, 2].map(|i| dbc.f32_at(row, base + i).unwrap_or(0.0))
                };
                tables.personalities.insert(
                    id,
                    Personality {
                        thresholds: [0, 1, 2].map(|i| {
                            dbc.u32_at(row, personality_fields::THRESHOLDS + i)
                                .unwrap_or(0) as i32
                        }),
                        damage: three(personality_fields::DAMAGE),
                        loyalty: three(personality_fields::LOYALTY),
                    },
                );
            }
        }
        if let Ok(dbc) = Dbc::parse(family) {
            for row in 0..dbc.record_count {
                let Some(id) = dbc.u32_at(row, 0) else { continue };
                tables.families.insert(
                    id,
                    Family {
                        name: dbc.string_at(row, family_fields::NAME).unwrap_or_default(),
                        icon: dbc.string_at(row, family_fields::ICON).unwrap_or_default(),
                        food_mask: dbc.u32_at(row, family_fields::FOOD_MASK).unwrap_or(0),
                        skill_lines: [0, 1].map(|i| {
                            dbc.u32_at(row, family_fields::SKILL_LINES + i).unwrap_or(0)
                        }),
                    },
                );
            }
        }
        if let Ok(dbc) = Dbc::parse(food) {
            for row in 0..dbc.record_count {
                let Some(id) = dbc.u32_at(row, 0) else { continue };
                let name = dbc.string_at(row, NAME_FIELD).unwrap_or_default();
                if !name.is_empty() {
                    tables.foods.push((id, name));
                }
            }
        }
        if let Ok(dbc) = Dbc::parse(loyalty) {
            for row in 0..dbc.record_count {
                let Some(id) = dbc.u32_at(row, 0) else { continue };
                let name = dbc.string_at(row, NAME_FIELD).unwrap_or_default();
                if !name.is_empty() {
                    tables.loyalties.insert(id, name);
                }
            }
        }
        if let Ok(dbc) = Dbc::parse(stable_prices) {
            for row in 0..dbc.record_count {
                let Some(id) = dbc.u32_at(row, 0) else { continue };
                let Some(price) = dbc.u32_at(row, PRICE_FIELD) else {
                    continue;
                };
                tables.stable_prices.insert(id, price);
            }
        }
        tables
    }

    /// **`GetNextStableSlotCost()`** — what the slot after `bought` costs, in
    /// copper.
    ///
    /// The row is the slot being bought, so a character with none yet asks for
    /// row 1. **Zero once every slot is bought**, which is not a price but the
    /// value `PetStable_Update` puts into `MoneyFrame_Update` on the frame
    /// before it hides the whole purchase block — the panel's own test is
    /// `GetNumStableSlots() == NUM_PET_STABLE_SLOTS`, not the cost.
    pub fn stable_slot_cost(&self, bought: u32) -> u32 {
        self.stable_prices
            .get(&bought.saturating_add(1))
            .copied()
            .unwrap_or(0)
    }

    /// Every priced slot, ascending — for `vale pet`'s census.
    pub fn stable_prices(&self) -> Vec<(u32, u32)> {
        let mut all: Vec<(u32, u32)> = self.stable_prices.iter().map(|(a, b)| (*a, *b)).collect();
        all.sort_by_key(|(slot, _)| *slot);
        all
    }

    /// **`GetPetHappiness`'s three answers** — the band, the damage percentage
    /// and the loyalty rate.
    ///
    /// `None` when there is no personality row to read at all, which is
    /// `GetPetHappiness`' own nil and what makes `PetFrame_SetHappiness` hide
    /// the icon.
    ///
    /// A pet below the first threshold answers `(0, 100.0, 0.0)` rather than
    /// reading the table, as the client does — and note that the
    /// damage is **already multiplied by 100** here, because
    /// `PET_DAMAGE_PERCENTAGE` is `"Causes %d%% of normal damage"` and the file
    /// stores 0.75.
    pub fn happiness(&self, personality: u32, happiness: i32) -> Option<(u32, f32, f32)> {
        let row = self
            .personalities
            .get(&personality)
            .or_else(|| self.personalities.get(&DEFAULT_PERSONALITY))?;
        let band = row.band(happiness);
        if band == 0 {
            return Some((0, NO_BAND.0, NO_BAND.1));
        }
        Some((
            band as u32,
            row.damage[band - 1] * 100.0,
            row.loyalty[band - 1],
        ))
    }

    pub fn family(&self, id: u32) -> Option<&Family> {
        self.families.get(&id)
    }

    /// **What a family may be fed** — every `ItemPetFood.dbc` name whose bit is
    /// in the mask, in the food table's own order.
    pub fn foods_for(&self, family: u32) -> Vec<&str> {
        let Some(mask) = self.families.get(&family).map(|f| f.food_mask) else {
            return Vec::new();
        };
        self.foods
            .iter()
            .filter(|(id, _)| id.checked_sub(1).is_some_and(|bit| mask & (1 << bit) != 0))
            .map(|(_, name)| name.as_str())
            .collect()
    }

    /// The loyalty level's name — "(Loyalty Level 3) Submissive". `None` for
    /// level 0, which is a pet that has none yet.
    pub fn loyalty(&self, level: u32) -> Option<&str> {
        self.loyalties.get(&level).map(String::as_str)
    }

    /// What each check reports: the five row counts.
    pub fn counts(&self) -> (usize, usize, usize, usize, usize) {
        (
            self.personalities.len(),
            self.families.len(),
            self.foods.len(),
            self.loyalties.len(),
            self.stable_prices.len(),
        )
    }

    /// Every family, for `vale pet`'s census — by id, ascending.
    pub fn families(&self) -> Vec<(u32, &Family)> {
        let mut all: Vec<(u32, &Family)> = self.families.iter().map(|(id, f)| (*id, f)).collect();
        all.sort_by_key(|(id, _)| *id);
        all
    }

    /// …and every personality, on the same terms.
    pub fn personalities(&self) -> Vec<(u32, &Personality)> {
        let mut all: Vec<(u32, &Personality)> =
            self.personalities.iter().map(|(id, p)| (*id, p)).collect();
        all.sort_by_key(|(id, _)| *id);
        all
    }

    pub fn foods(&self) -> &[(u32, String)] {
        &self.foods
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// The shipped `PetPersonality.dbc` row 1, "Personality: Standard", which
    /// is the one every pet in a live session uses.
    fn standard() -> PetTables {
        let mut row = vec![0u32; 19];
        row[0] = 1;
        row[personality_fields::THRESHOLDS] = 0;
        row[personality_fields::THRESHOLDS + 1] = 333_000;
        row[personality_fields::THRESHOLDS + 2] = 666_000;
        row[personality_fields::DAMAGE] = 0.75f32.to_bits();
        row[personality_fields::DAMAGE + 1] = 1.0f32.to_bits();
        row[personality_fields::DAMAGE + 2] = 1.25f32.to_bits();
        row[personality_fields::LOYALTY] = (-10.0f32).to_bits();
        row[personality_fields::LOYALTY + 1] = 5.0f32.to_bits();
        row[personality_fields::LOYALTY + 2] = 20.0f32.to_bits();
        PetTables::parse(&dbc(&[row], 19, b"\0"), &[], &[], &[], &[])
    }

    /// **The three bands are the numbers a player of 1.12 remembers**, and the
    /// damage comes out as a percentage rather than as the file's multiplier.
    #[test]
    fn the_standard_personality_is_75_100_and_125_per_cent() {
        let tables = standard();
        // The lowest band starts at zero, so every non-negative happiness is in
        // one — the level-0 arm is only reachable by a negative value.
        assert_eq!(tables.happiness(1, 0), Some((1, 75.0, -10.0)));
        assert_eq!(tables.happiness(1, 332_999), Some((1, 75.0, -10.0)));
        assert_eq!(tables.happiness(1, 333_000), Some((2, 100.0, 5.0)));
        assert_eq!(tables.happiness(1, 666_000), Some((3, 125.0, 20.0)));
        assert_eq!(tables.happiness(1, 1_050_000), Some((3, 125.0, 20.0)));
        assert_eq!(tables.happiness(1, -1), Some((0, 100.0, 0.0)));
    }

    /// **A personality the table has no row for falls back to id 1** — and it
    /// is the ordinary path rather than the exception,
    /// because vmangos sends a literal zero in that slot for every creature.
    #[test]
    fn an_unknown_personality_reads_the_standard_row() {
        let tables = standard();
        assert_eq!(tables.happiness(0, 400_000), Some((2, 100.0, 5.0)));
        assert_eq!(tables.happiness(9999, 400_000), Some((2, 100.0, 5.0)));
        // …and with no table at all there is nothing to fall back *to*, which
        // is `GetPetHappiness`' own nil.
        assert_eq!(PetTables::default().happiness(1, 400_000), None);
    }

    /// **The diet is a mask over the food table, and the order is the food
    /// table's** — the client's loop walks every row and tests its bit, rather
    /// than walking the mask.
    #[test]
    fn a_family_is_fed_what_its_mask_names_in_the_foods_own_order() {
        let family = |id: u32, mask: u32, name: u32| {
            let mut row = vec![0u32; 18];
            row[0] = id;
            row[family_fields::FOOD_MASK] = mask;
            row[family_fields::NAME] = name;
            row[family_fields::ICON] = name;
            row
        };
        let food = |id: u32, name: u32| {
            let mut row = vec![0u32; 10];
            row[0] = id;
            row[NAME_FIELD] = name;
            row
        };
        let tables = PetTables::parse(
            &[],
            // Mask 0b101 — food ids 1 and 3.
            &dbc(&[family(1, 0b101, 1)], 18, b"\0Wolf\0"),
            &dbc(
                &[food(1, 1), food(2, 6), food(3, 11)],
                10,
                b"\0Meat\0Fish\0Cheese\0",
            ),
            &[],
            &[],
        );
        assert_eq!(tables.foods_for(1), vec!["Meat", "Cheese"]);
        // …and a family nothing knows is fed nothing rather than everything.
        assert!(tables.foods_for(99).is_empty());
        assert_eq!(tables.family(1).map(|f| f.name.as_str()), Some("Wolf"));
    }

    /// **The row is the slot being bought**, so a character with none yet asks
    /// for row 1 — an off-by-one here reads the 5-gold price to somebody who
    /// owes 5 silver, and `PetStablePurchaseButton` then greys out for a
    /// character who can afford it.
    #[test]
    fn the_next_slot_costs_the_row_after_the_ones_already_bought() {
        use crate::tables::dbc::testing::dbc;
        let price = |slot: u32, copper: u32| vec![slot, copper];
        let tables = PetTables::parse(
            &[],
            &[],
            &[],
            &[],
            // The shipped table: 500 copper, then 50,000.
            &dbc(&[price(1, 500), price(2, 50_000)], 2, b" "),
        );
        assert_eq!(tables.stable_slot_cost(0), 500, "the first slot");
        assert_eq!(tables.stable_slot_cost(1), 50_000, "the second");
        // **Zero once both are bought** — not a price, and the panel hides the
        // whole purchase block on the slot count rather than on this.
        assert_eq!(tables.stable_slot_cost(2), 0);
        assert_eq!(tables.stable_slot_cost(u32::MAX), 0, "and it does not wrap");
        assert_eq!(tables.stable_prices(), vec![(1, 500), (2, 50_000)]);
    }

    /// A missing `StableSlotPrices.dbc` prices every slot at nothing, which is
    /// the stated degradation rather than an error.
    #[test]
    fn an_absent_price_table_costs_nothing() {
        let tables = PetTables::parse(&[], &[], &[], &[], &[]);
        assert_eq!(tables.stable_slot_cost(0), 0);
        assert!(tables.stable_prices().is_empty());
    }
}
