//! **What it costs to make a worn item whole again**, out of the two shipped
//! tables that say so — and nothing about the wire.
//!
//! `CMSG_REPAIR_ITEM` carries two guids and no money at all: the server prices
//! the job itself. So why is the price here? Because the *interface* asks for it
//! before the packet is sent — `MerchantRepairAllButton`'s `OnEnter` calls
//! `GetRepairAllCost()` and puts the answer on the plate — and a client that
//! could not price a repair would show an empty tooltip on a button that
//! silently takes your money. The same split every rule in this crate follows:
//! the price is a rule in a file, not an answer from a server.
//!
//! ```text
//! DurabilityCosts.dbc    300 rows   itemLevel -> 29 multipliers
//! DurabilityQuality.dbc   14 rows   (quality + 1) * 2 -> a float
//! ```
//!
//! ## The arithmetic, and every step of it is `Player::DurabilityRepair`'s
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
//! Two of those are worth stating rather than reading past. **The row is looked
//! up by the item's *level*, not by its id** — `sDurabilityCostsStore.LookupEntry
//! (ditemProto->ItemLevel)` — so the table is a curve over item level with a
//! column per subclass, and an item whose level has no row cannot be priced at
//! all. And **the quality row is `(quality + 1) * 2`**, an arithmetic index
//! rather than the quality itself, which is why `DurabilityQuality.dbc` has
//! fourteen rows for seven qualities.
//!
//! ## The discount is the one input this client does not have
//!
//! The server multiplies by `GetReputationPriceDiscount(unit)` — 1.0, 0.95, 0.9
//! or 0.8 by standing with the vendor's faction. Nothing here tracks reputation,
//! so [`RepairCosts::cost`] takes the discount as an argument and every caller
//! passes [`NO_DISCOUNT`]. **A stated over-estimate**: a friendly player is
//! quoted a few percent high, the packet still charges the true price, and the
//! only visible consequence is the number on a tooltip. Getting it *right* wants
//! `SMSG_INITIALIZE_FACTIONS` and the whole reputation subject, which this
//! client has none of.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

/// `ITEM_CLASS_WEAPON`, whose subclass indexes the cost columns directly.
const CLASS_WEAPON: u32 = 2;
/// `ITEM_CLASS_ARMOR`, whose subclasses start after the twenty-one weapon ones.
const CLASS_ARMOR: u32 = 4;
/// Where the armour columns begin — `ItemSubClassToDurabilityMultiplierId`'s
/// own `ItemSubClass + 21`.
const ARMOR_COLUMN_BASE: usize = 21;
/// How many multiplier columns a `DurabilityCosts` row has, after the id.
const COLUMNS: usize = 29;

/// **No reputation discount** — see the module comment on why every caller in
/// this repo passes it.
pub const NO_DISCOUNT: f32 = 1.0;

/// What a repair is priced against: everything the arithmetic reads, from the
/// item object (the two durabilities) and its template (the other four).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Worn {
    /// `ITEM_FIELD_DURABILITY`.
    pub durability: u32,
    /// `ITEM_FIELD_MAXDURABILITY`. **Zero means the thing cannot wear out at
    /// all** — a potion, a reagent, a bag — and is priced at nothing.
    pub max_durability: u32,
    /// The template's `ItemLevel`, which is the row key and not the item's id.
    pub item_level: u32,
    /// …its `Quality`, which becomes `(quality + 1) * 2`.
    pub quality: u32,
    /// …and its class and subclass, which choose the column.
    pub class: u32,
    pub subclass: u32,
}

impl Worn {
    /// How much durability is missing. Zero for a whole item **and** for one
    /// that cannot wear out, which is why nothing else has to test both.
    pub fn lost(&self) -> u32 {
        self.max_durability.saturating_sub(self.durability)
    }
}

/// **Which of the twenty-nine columns an item is priced in** —
/// `ItemSubClassToDurabilityMultiplierId`, transcribed.
///
/// Anything that is neither a weapon nor armour falls to column 0, which is the
/// reference's own `return 0` rather than a guard invented here: it is reached
/// by items that have durability and are neither, of which 1.12 has none.
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
    /// By item **level**. Each row is the 29 multipliers after the id.
    costs: HashMap<u32, Vec<u32>>,
    /// By the row id the quality arithmetic produces, not by quality.
    quality: HashMap<u32, f32>,
}

impl RepairCosts {
    /// Read both. **Either absence is the same degradation** and it is a
    /// visible one: an unpriceable item contributes nothing to
    /// `GetRepairAllCost`, so the tooltip shows a smaller sum or none at all —
    /// and the repair itself still works, because the money is the server's
    /// arithmetic and not this one.
    pub fn parse(costs: &[u8], quality: &[u8]) -> RepairCosts {
        let mut out = RepairCosts::default();

        if let Ok(dbc) = Dbc::parse(costs) {
            // `id + 29` — a row that is shorter is a table this code does not
            // know, and reading it would price every item off the wrong column.
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

    /// **What one item costs to repair**, or `None` when it cannot be priced.
    ///
    /// `None` and `Some(0)` are different answers and both happen: an item with
    /// no durability, or a whole one, is `Some(0)` — there is nothing to pay —
    /// while an item whose level has no `DurabilityCosts` row, or whose quality
    /// has no modifier, is `None`, because the table this client would need to
    /// price it is not there. The reference logs an error and charges nothing in
    /// that case; a sum over a bag must not silently count it as free.
    pub fn cost(&self, item: Worn, discount: f32) -> Option<u32> {
        let lost = item.lost();
        if lost == 0 {
            return Some(0);
        }
        let row = self.costs.get(&item.item_level)?;
        let multiplier = *row.get(cost_column(item.class, item.subclass))?;
        // `(Quality + 1) * 2` — the reference's own index, and the reason the
        // table is twice as long as the quality enum.
        let quality = *self.quality.get(&((item.quality + 1) * 2))?;
        let costs = (lost * multiplier) as f32 * quality;
        let costs = (costs as u32) as f32 * discount + 0.5;
        // **Never free once it is worn** — the reference's own fix for the
        // artifact quality, whose modifier rounds a real cost to nothing.
        Some((costs as u32).max(1))
    }

    /// …and over a whole bag: the sum, and **whether anything in it could not
    /// be priced**.
    ///
    /// Both halves, because the interface asks two questions of one call —
    /// `local repairAllCost, canRepair = GetRepairAllCost()` — and a sum that
    /// silently dropped an unpriceable item would answer the first one wrongly
    /// while still saying yes to the second.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The column rule, which is the one place a wrong index would price a sword
    /// as a shirt and produce a plausible number.
    #[test]
    fn the_column_is_the_subclass_and_armour_starts_at_twenty_one() {
        // A one-handed sword is weapon subclass 7.
        assert_eq!(cost_column(CLASS_WEAPON, 7), 7);
        // Mail is armour subclass 3, which is column 24.
        assert_eq!(cost_column(CLASS_ARMOR, 3), 24);
        // …and the twenty-one weapon columns and eight armour ones fill the row
        // exactly, which is what makes the shape checkable at all.
        assert_eq!(cost_column(CLASS_ARMOR, 7), COLUMNS - 1);
        // Anything else falls to 0 — the reference's own `return 0`.
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

    /// **The whole arithmetic, in the order the reference does it** — and the
    /// order matters, because the truncation happens twice.
    #[test]
    fn a_worn_sword_is_priced_off_its_level_quality_and_subclass() {
        let tables = tables();
        // 10 lost * 100 * 0.9 = 900, discount 1, + 0.5, truncated.
        assert_eq!(tables.cost(sword(50, 60), NO_DISCOUNT), Some(900));
        // A whole item is free rather than unpriceable.
        assert_eq!(tables.cost(sword(60, 60), NO_DISCOUNT), Some(0));
        // …and so is one that cannot wear out at all.
        assert_eq!(tables.cost(sword(0, 0), NO_DISCOUNT), Some(0));
        // A discount is applied *after* the first truncation, which is the
        // reference's own order.
        assert_eq!(tables.cost(sword(50, 60), 0.9), Some(810));
    }

    /// **Never free once it is worn.** The artifact-quality fix, and the reason
    /// a one-point scratch on a trinket still costs a copper.
    #[test]
    fn a_price_that_rounds_to_nothing_is_still_a_copper() {
        let mut tables = tables();
        tables.quality.insert(6, 0.0001);
        assert_eq!(tables.cost(sword(59, 60), NO_DISCOUNT), Some(1));
    }

    /// **An item the tables cannot price is `None`, not zero**, and a bag
    /// containing one says so — which is the `canRepair` half of
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
        // …and a bag of things it *can* price says so.
        let (total, all_priced) = tables.total([sword(50, 60), sword(60, 60)], NO_DISCOUNT);
        assert_eq!((total, all_priced), (900, true));
    }

    /// Both tables or nothing usable — see [`RepairCosts::parse`].
    #[test]
    fn either_table_missing_is_the_same_degradation() {
        assert!(RepairCosts::parse(&[], &[]).is_empty());
        assert!(tables().costs.len() == 1 && !tables().is_empty());
    }
}
