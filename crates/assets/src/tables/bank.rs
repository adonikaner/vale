//! **What a bank bag slot costs** — `BankBagSlotPrices.dbc`, the one table
//! the bank window reads, and the rule that turns it into `GetNumBankSlots`'s
//! second answer.
//!
//! ```text
//! BankBagSlotPrices.dbc   12 records x 2 fields: [0] the slot being bought, [1] copper
//! ```
//!
//! Measured with `vale dbc BankBagSlotPrices` and checked by `vale bank`.
//! The row is the slot being bought, so a character with none pays row 1
//! (10 silver) and one with five pays row 6 — the same lookup
//! `HandleBuyBankSlotOpcode` makes with `GetBankBagSlotCount() + 1`, so the
//! price the panel prints is the price the server takes.
//!
//! ## Twelve rows, six slots
//!
//! Measured: rows 1..6 are 10 silver, 1, 10, 25, 50 and 100 gold; rows 7..12
//! are all 999,999,999 copper, the file's own way of saying *never*. The
//! character has six: `PLAYER_FIELD_BANKBAG_SLOT_1` is six guids wide,
//! `BankFrame.lua`'s `NUM_BANKBAGSLOTS` is 6, and
//! `BANK_SLOT_BAG_END - BANK_SLOT_BAG_START` is 6. So `full` — the second
//! value of `GetNumBankSlots()`, which hides the purchase frame — is *six
//! bought* and not *no row left*; a client that read the table's length
//! would price a seventh slot at 99,999 gold and offer it, and the fields
//! could not hold it. Rows 7..12 are recorded as what the file says and
//! never reached.

use crate::tables::dbc::Dbc;

/// The bag slots a bank has — see the module note on why this is not the
/// table's row count. The same six as
/// `vale_protocol::play::items::BANK_BAG_SLOTS`, stated here because this
/// crate does not depend on that one.
pub const BANK_BAG_SLOTS: u32 = 6;

/// The price column.
const PRICE_FIELD: usize = 1;

/// `BankBagSlotPrices.dbc`, by the slot being bought.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BankPrices {
    /// `(slot, copper)`, ascending by slot.
    rows: Vec<(u32, u32)>,
}

impl BankPrices {
    /// Parse the table. An absent or unreadable file is an empty table, whose
    /// degradation is a purchase button priced at nothing and a server that
    /// refuses the click — stated rather than an error, like every optional
    /// table in the chain.
    pub fn parse(raw: &[u8]) -> BankPrices {
        let mut rows = Vec::new();
        if let Ok(dbc) = Dbc::parse(raw) {
            for row in 0..dbc.record_count {
                let Some(slot) = dbc.u32_at(row, 0) else { continue };
                let Some(price) = dbc.u32_at(row, PRICE_FIELD) else { continue };
                rows.push((slot, price));
            }
        }
        rows.sort_by_key(|(slot, _)| *slot);
        BankPrices { rows }
    }

    /// **`GetBankSlotCost(numSlots)`** — what the slot after `bought` costs,
    /// in copper, or `None` past the table. The panel calls it with
    /// `GetNumBankSlots()`'s first answer and hands the number straight to
    /// `MoneyFrame_Update`, so a slot with no row answers 0 there.
    pub fn cost(&self, bought: u32) -> Option<u32> {
        let next = bought.checked_add(1)?;
        self.rows
            .iter()
            .find(|(slot, _)| *slot == next)
            .map(|(_, price)| *price)
    }

    /// **`GetNumBankSlots()`'s second answer**: whether every slot the fields
    /// can hold is bought — see the module note.
    pub fn full(bought: u32) -> bool {
        bought >= BANK_BAG_SLOTS
    }

    /// Every priced slot, ascending — for `vale bank`'s census.
    pub fn ladder(&self) -> &[(u32, u32)] {
        &self.rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    fn prices() -> BankPrices {
        BankPrices::parse(&dbc(
            &[vec![2, 10_000], vec![1, 1_000], vec![3, 100_000], vec![7, 1]],
            2,
            b" ",
        ))
    }

    #[test]
    fn the_row_is_the_slot_being_bought() {
        let table = prices();
        assert_eq!(table.cost(0), Some(1_000), "a character with none pays row 1");
        assert_eq!(table.cost(1), Some(10_000));
        assert_eq!(table.cost(2), Some(100_000));
        assert_eq!(table.cost(3), None, "no row 4 in this table");
        assert_eq!(table.ladder(), &[(1, 1_000), (2, 10_000), (3, 100_000), (7, 1)]);
    }

    #[test]
    fn full_is_six_bought_and_not_the_end_of_the_table() {
        assert!(!BankPrices::full(5));
        assert!(BankPrices::full(6));
        assert!(BankPrices::full(7));
        assert_eq!(BankPrices::parse(b"junk").cost(0), None);
    }
}
