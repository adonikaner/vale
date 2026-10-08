//! What it costs to repair a worn item, out of the two shipped tables that
//! say so, and what a merchant pays for an item, which subtracts that cost.
//!
//! `CMSG_REPAIR_ITEM` carries two guids and no money: the server prices the
//! job itself. The client prices it too, because the interface asks before
//! the packet is sent: `MerchantRepairAllButton`'s `OnEnter` calls
//! `GetRepairAllCost()` and puts the answer on the plate. The sell price an
//! item's tooltip shows at a vendor is computed the same way, on the client.
//!
//! ```text
//! DurabilityCosts.dbc    300 rows   itemLevel -> 29 multipliers
//! DurabilityQuality.dbc   14 rows   (quality + 1) * 2 -> a float
//! ```
//!
//! ## The repair arithmetic, as `Player::DurabilityRepair` computes it
//!
//! ```text
//! lost       = maxDurability - durability
//! column     = subclass          for a weapon (class 2)
//!              subclass + 21     for armour   (class 4)
//!              0                 for anything else
//! costs      = uint32(lost * DurabilityCosts[itemLevel][column] * quality_mod)
//! costs      = uint32(costs * discount + 0.5)
//! costs      = max(costs, 1)                       // never free once worn
//! ```
//!
//! The row is looked up by the item's level, not by its id
//! (`sDurabilityCostsStore.LookupEntry(ditemProto->ItemLevel)`), so the table
//! is a curve over item level with a column per subclass, and an item whose
//! level has no row cannot be priced. The quality row is `(quality + 1) * 2`,
//! an arithmetic index rather than the quality itself, which is why
//! `DurabilityQuality.dbc` has fourteen rows for seven qualities.
//!
//! ## The discount is not applied
//!
//! The server multiplies by `GetReputationPriceDiscount(unit)`: 1.0, 0.95, 0.9
//! or 0.8 by standing with the vendor's faction. [`RepairCosts::cost`] takes
//! the discount as an argument and every caller passes [`NO_DISCOUNT`], so a
//! friendly or better player is quoted a few percent high. The packet still
//! charges the server's price; only the number on the tooltip differs.
//!
//! ## The sell price
//!
//! [`sell_price`] is what the 1.12.1 client shows on an item's tooltip while a
//! merchant window is open, and what vmangos' `HandleSellItemOpcode` pays: the
//! template's `SellPrice`, prorated by the charges left when the item is used
//! up by more than one charge, less the repair cost (and 1 copper when the
//! repair costs as much or more), times the stack.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

/// `ITEM_CLASS_WEAPON`, whose subclass indexes the cost columns directly.
const CLASS_WEAPON: u32 = 2;
/// `ITEM_CLASS_ARMOR`, whose subclasses start after the twenty-one weapon ones.
const CLASS_ARMOR: u32 = 4;
/// The first armour column. vmangos' `ItemSubClassToDurabilityMultiplierId`
/// returns `ItemSubClass + 21` for armour.
const ARMOR_COLUMN_BASE: usize = 21;
/// How many multiplier columns a `DurabilityCosts` row has, after the id.
const COLUMNS: usize = 29;

/// No reputation discount. The module comment says why every caller in this
/// repo passes it.
pub const NO_DISCOUNT: f32 = 1.0;

/// What a repair is priced against: everything the arithmetic reads, from the
/// item object (the two durabilities) and its template (the other four).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Worn {
    /// `ITEM_FIELD_DURABILITY`.
    pub durability: u32,
    /// `ITEM_FIELD_MAXDURABILITY`. Zero means the item cannot wear out (a
    /// potion, a reagent, a bag), and it is priced at nothing.
    pub max_durability: u32,
    /// The template's `ItemLevel`, which is the row key and not the item's id.
    pub item_level: u32,
    /// The template's `Quality`, which becomes `(quality + 1) * 2`.
    pub quality: u32,
    /// The template's class and subclass, which choose the column.
    pub class: u32,
    pub subclass: u32,
}

impl Worn {
    /// How much durability is missing. Zero for a whole item and for one that
    /// cannot wear out, so no caller has to test both.
    pub fn lost(&self) -> u32 {
        self.max_durability.saturating_sub(self.durability)
    }
}

/// Which of the twenty-nine columns an item is priced in, as vmangos'
/// `ItemSubClassToDurabilityMultiplierId` chooses it.
///
/// Anything that is neither a weapon nor armour falls to column 0, because
/// vmangos returns 0 for it; this is not a guard added here. Only an item that
/// has durability and is neither would reach it, and 1.12 has none.
pub fn cost_column(class: u32, subclass: u32) -> usize {
    match class {
        CLASS_WEAPON => subclass as usize,
        CLASS_ARMOR => subclass as usize + ARMOR_COLUMN_BASE,
        _ => 0,
    }
}

/// The two tables, read.
#[derive(Debug, Default)]
pub struct RepairCosts {
    /// By item level. Each row is the 29 multipliers after the id.
    costs: HashMap<u32, Vec<u32>>,
    /// By the row id the quality arithmetic produces, not by quality.
    quality: HashMap<u32, f32>,
}

impl RepairCosts {
    /// Read both. Either table missing has the same visible effect:
    /// an unpriceable item contributes nothing to `GetRepairAllCost`, so the
    /// tooltip shows a smaller sum or none. The repair itself still works,
    /// because the server computes the money it charges.
    pub fn parse(costs: &[u8], quality: &[u8]) -> RepairCosts {
        let mut out = RepairCosts::default();

        if let Ok(dbc) = Dbc::parse(costs) {
            // A row is the id and 29 multipliers. A shorter row is a layout
            // this code does not know, and reading it would price every item
            // off the wrong column.
            if dbc.field_count as usize >= COLUMNS + 1 {
                for record in 0..dbc.record_count {
                    let Some(level) = dbc.u32_at(record, 0) else { continue };
                    let row: Vec<u32> =
                        (1..=COLUMNS).map(|field| dbc.u32_at(record, field).unwrap_or(0)).collect();
                    out.costs.insert(level, row);
                }
            }
        }

        if let Ok(dbc) = Dbc::parse(quality) {
            for record in 0..dbc.record_count {
                let (Some(id), Some(modifier)) =
                    (dbc.u32_at(record, 0), dbc.f32_at(record, 1))
                else {
                    continue;
                };
                out.quality.insert(id, modifier);
            }
        }
        out
    }

    /// Whether anything was read at all — what a caller tests before quoting a
    /// zero as a price rather than as an absence.
    pub fn is_empty(&self) -> bool {
        self.costs.is_empty() || self.quality.is_empty()
    }

    /// What one item costs to repair, or `None` when it cannot be priced.
    ///
    /// `None` and `Some(0)` are different answers and both happen. An item with
    /// no durability, or a whole one, is `Some(0)`: there is nothing to pay. An
    /// item whose level has no `DurabilityCosts` row, or whose quality has no
    /// modifier, is `None`, because the row needed to price it is missing.
    /// vmangos logs an error and charges nothing in that case; a sum over a bag
    /// must not count it as free without saying so.
    pub fn cost(&self, item: Worn, discount: f32) -> Option<u32> {
        let lost = item.lost();
        if lost == 0 {
            return Some(0);
        }
        let row = self.costs.get(&item.item_level)?;
        let multiplier = *row.get(cost_column(item.class, item.subclass))?;
        // vmangos indexes the quality table by `(Quality + 1) * 2`, which is
        // why the table is twice as long as the quality enum.
        let quality = *self.quality.get(&((item.quality + 1) * 2))?;
        let costs = (lost * multiplier) as f32 * quality;
        let costs = (costs as u32) as f32 * discount + 0.5;
        // A worn item costs at least 1 copper. vmangos adds this for the
        // artifact quality, whose modifier rounds a real cost to nothing.
        Some((costs as u32).max(1))
    }

    /// The repair cost of a set of items: the sum, and whether every item in it
    /// could be priced.
    ///
    /// The interface reads both from one call,
    /// `local repairAllCost, canRepair = GetRepairAllCost()`. A sum that
    /// dropped an unpriceable item without reporting it would give a wrong
    /// cost while still answering yes to the second value.
    pub fn total<I: IntoIterator<Item = Worn>>(
        &self,
        items: I,
        discount: f32,
    ) -> (u32, bool) {
        let mut total = 0;
        let mut all_priced = true;
        for item in items {
            match self.cost(item, discount) {
                Some(cost) => total += cost,
                None => all_priced = false,
            }
        }
        (total, all_priced)
    }
}

/// The copper a merchant pays for one carried item; see the module comment.
///
/// `charges` is the copy's `ITEM_FIELD_SPELL_CHARGES[0]` and the template's
/// first spell charges, which prorate the price when the template's is below
/// -1 (an item used up by its charges, with more than one). `repair` is
/// [`RepairCosts::cost`] for the copy, 0 for an item that cannot wear out.
/// A template with no sell price answers 0, which the tooltip shows as "No
/// sell price".
pub fn sell_price(template_price: u32, charges: (i32, i32), repair: u32, stack: u32) -> u32 {
    if template_price == 0 {
        return 0;
    }
    let (left, total) = charges;
    let mut price = if total < -1 {
        (i64::from(template_price) * i64::from(left) / i64::from(total)).max(0) as u32
    } else {
        template_price
    };
    if repair >= price {
        price = 1;
    } else {
        price -= repair;
    }
    if stack > 1 {
        price = price.saturating_mul(stack);
    }
    price
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sell_price_prorates_charges_subtracts_repair_and_multiplies_the_stack() {
        // A stack of five, nothing worn.
        assert_eq!(sell_price(10, (0, 0), 0, 5), 50);
        // A mana gem with two of three charges left.
        assert_eq!(sell_price(300, (-2, -3), 0, 1), 200);
        // A single-use item is not prorated.
        assert_eq!(sell_price(25, (-1, -1), 0, 1), 25);
        // A worn sword: the repair comes off, and never below 1 copper.
        assert_eq!(sell_price(1000, (0, 0), 300, 1), 700);
        assert_eq!(sell_price(1000, (0, 0), 1000, 1), 1);
        assert_eq!(sell_price(1000, (0, 0), 5000, 1), 1);
        // No sell price at all.
        assert_eq!(sell_price(0, (0, 0), 0, 3), 0);
    }

    /// The column rule. A wrong index here would price a sword as a shirt and
    /// still produce a plausible number.
    #[test]
    fn the_column_is_the_subclass_and_armour_starts_at_twenty_one() {
        // A one-handed sword is weapon subclass 7.
        assert_eq!(cost_column(CLASS_WEAPON, 7), 7);
        // Mail is armour subclass 3, which is column 24.
        assert_eq!(cost_column(CLASS_ARMOR, 3), 24);
        // The twenty-one weapon columns and eight armour columns fill the row
        // exactly, which lets the shape be checked.
        assert_eq!(cost_column(CLASS_ARMOR, 7), COLUMNS - 1);
        // Anything else falls to 0, as in vmangos.
        assert_eq!(cost_column(0, 9), 0);
        assert_eq!(cost_column(15, 4), 0);
    }

    /// A hand-built pair of tables, so the arithmetic is checked without an
    /// archive: one item level, one quality.
    fn tables() -> RepairCosts {
        let mut costs = RepairCosts::default();
        let mut row = vec![0; COLUMNS];
        row[7] = 100; // a one-handed sword
        costs.costs.insert(40, row);
        // (Quality 2 + 1) * 2 = 6 — "uncommon", at nine tenths.
        costs.quality.insert(6, 0.9);
        costs
    }

    fn sword(durability: u32, max: u32) -> Worn {
        Worn {
            durability,
            max_durability: max,
            item_level: 40,
            quality: 2,
            class: CLASS_WEAPON,
            subclass: 7,
        }
    }

    /// The whole arithmetic, in vmangos' order. The order matters because the
    /// value is truncated twice.
    #[test]
    fn a_worn_sword_is_priced_off_its_level_quality_and_subclass() {
        let tables = tables();
        // 10 lost * 100 * 0.9 = 900, discount 1, + 0.5, truncated.
        assert_eq!(tables.cost(sword(50, 60), NO_DISCOUNT), Some(900));
        // A whole item is free rather than unpriceable.
        assert_eq!(tables.cost(sword(60, 60), NO_DISCOUNT), Some(0));
        // So is one that cannot wear out.
        assert_eq!(tables.cost(sword(0, 0), NO_DISCOUNT), Some(0));
        // The discount is applied after the first truncation, as in vmangos.
        assert_eq!(tables.cost(sword(50, 60), 0.9), Some(810));
    }

    /// A worn item costs at least 1 copper. This is the artifact-quality rule,
    /// and it is why one point of lost durability still costs a copper.
    #[test]
    fn a_price_that_rounds_to_nothing_is_still_a_copper() {
        let mut tables = tables();
        tables.quality.insert(6, 0.0001);
        assert_eq!(tables.cost(sword(59, 60), NO_DISCOUNT), Some(1));
    }

    /// An item the tables cannot price is `None`, not zero, and the total for a
    /// bag containing one reports it. That report is the `canRepair` value of
    /// `GetRepairAllCost`.
    #[test]
    fn an_unpriceable_item_is_not_a_free_one() {
        let tables = tables();
        let unknown_level = Worn { item_level: 41, ..sword(50, 60) };
        assert_eq!(tables.cost(unknown_level, NO_DISCOUNT), None);
        let unknown_quality = Worn { quality: 4, ..sword(50, 60) };
        assert_eq!(tables.cost(unknown_quality, NO_DISCOUNT), None);

        // The sum drops it and reports that it did.
        let (total, all_priced) =
            tables.total([sword(50, 60), unknown_level], NO_DISCOUNT);
        assert_eq!(total, 900);
        assert!(!all_priced);
        // A bag of items it can price reports that too.
        let (total, all_priced) = tables.total([sword(50, 60), sword(60, 60)], NO_DISCOUNT);
        assert_eq!((total, all_priced), (900, true));
    }

    /// Without both tables nothing is usable. See [`RepairCosts::parse`].
    #[test]
    fn either_table_missing_is_the_same_degradation() {
        assert!(RepairCosts::parse(&[], &[]).is_empty());
        assert!(tables().costs.len() == 1 && !tables().is_empty());
    }
}
