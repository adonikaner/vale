//! **The C functions `BankFrame.lua` calls** — three reads, two writes.
//!
//! ```text
//! GetNumBankSlots()              (bought, full) — the bag slots paid for, and whether six are
//! GetBankSlotCost(bought)        copper, straight into MoneyFrame_Update
//! BankButtonIDToInvSlotID(id, isBag)   a button's own id onto the flat inventory numbering
//!
//! PurchaseSlot()                 CloseBankFrame()
//! ```
//!
//! The split every panel keeps: the wire is [`vale_protocol::play::bank`],
//! the window is [`crate::game::npc::bank`], the price and the six-is-full
//! rule are [`vale_assets::tables::bank`], and this file is registration
//! and arguments.
//!
//! ## Everything else the frame reads is the paper doll's
//!
//! Thirty buttons, and not one of them asks a bank question. Each is a
//! `GetInventoryItemTexture("player", id)`, `GetInventoryItemCount`,
//! `IsInventoryItemLocked` and `GameTooltip:SetInventoryItem` over the id
//! `BankButtonIDToInvSlotID` hands it — 40..63 for the squares, 64..69 for
//! the bag slots — and a `PickupContainerItem(BANK_CONTAINER, id)` or
//! `UseContainerItem(BANK_CONTAINER, id)` on a click. All of those are
//! [`super::container`]'s and answer for a bank id since
//! [`vale_protocol::play::items::Inventory::inventory_slot`] learned the
//! two runs. So the bank *draws* off the same reads the character sheet
//! draws off, which is why its contents were never a packet.
//!
//! ## `GetNumBankSlots` is a byte and a rule
//!
//! The first answer is `PLAYER_BYTES_2`'s third byte, read off the inventory
//! snapshot rather than the window, so the panel probe — which opens the
//! frame with no banker — sees the character's real count. The second is
//! `full`, which `UpdateBagSlotStatus` uses to hide the purchase frame, and
//! it is *six bought* rather than the price table's twelve rows — see the
//! table's own module note.
//!
//! ## The three were stubs, and the stubs were not wrong
//!
//! `GetNumBankSlots` answered `(0, nil)` and `GetBankSlotCost` answered 1000,
//! which is exactly a character who has never bought a slot standing at a
//! banker: the panel drew correctly and reported nothing. `BankButtonIDToInvSlotID`
//! was the arithmetic it still is, moved here because the numbering it
//! produces now resolves to something. See [`super::super::api::stubs`].

use super::super::api::Answers;

/// **The reads this module registers**, for the count that measures the gap.
pub const READS: [&str; 4] = [
    "BankButtonIDToInvSlotID",
    "ContainerIDToInventoryID",
    "GetBankSlotCost",
    "GetNumBankSlots",
];

/// **What the bank panel may ask.** Every answer has a default that is a
/// character who has bought nothing, which is what the audit's two stand-ins
/// answer and what the window answers when the archives are shut.
pub trait BankAnswers {
    /// `GetNumBankSlots()`'s first answer — bag slots **bought**, 0..6.
    fn bank_slots_bought(&self) -> u32 {
        0
    }
    /// `GetBankSlotCost(bought)` — copper for the next slot, `0` past the
    /// table, which is what `MoneyFrame_Update` draws for "no price".
    fn bank_slot_cost(&self, bought: u32) -> u32 {
        let _ = bought;
        0
    }
}

impl BankAnswers for super::super::api::Live<'_, '_, '_> {
    fn bank_slots_bought(&self) -> u32 {
        u32::from(self.inventory.carried.bank_bag_slots)
    }

    fn bank_slot_cost(&self, bought: u32) -> u32 {
        self.tables
            .as_deref()
            .and_then(|tables| tables.bank().cost(bought))
            .unwrap_or(0)
    }
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    // `(numSlots, full)` — and `full` is a **1 or nil**, which is what
    // `if ( full ) then` tests: a `false` would also pass, but the reference's
    // second value is a boolean the 5.0 dialect spells this way.
    globals.set(
        "GetNumBankSlots",
        scope.create_function(move |_, ()| {
            let bought = answers.bank_slots_bought();
            let full = vale_assets::tables::bank::BankPrices::full(bought).then_some(1);
            Ok((bought, full))
        })?,
    )?;
    // The panel passes `GetNumBankSlots()`'s first answer back in; a missing
    // argument is a character with none.
    globals.set(
        "GetBankSlotCost",
        scope.create_function(move |_, bought: Option<i64>| {
            let bought = u32::try_from(bought.unwrap_or(0).max(0)).unwrap_or(u32::MAX);
            Ok(answers.bank_slot_cost(bought))
        })?,
    )?;
    // **`BankButtonIDToInvSlotID(buttonID, isBag)` — arithmetic, and nothing
    // else.** The two bases are vmangos' own enum rather than a guess:
    // `BANK_SLOT_ITEM_START = 39` and `BANK_SLOT_BAG_START = 63` (`Player.h`),
    // zero-based on the wire against the interface's one-based slot ids — the
    // same +1 crossing `items::server_inventory_slot` makes the other way.
    // Scoped with its neighbours for tidiness only; it reads no state.
    globals.set(
        "BankButtonIDToInvSlotID",
        scope.create_function(move |_, (id, is_bag): (Option<i64>, mlua::Value)| {
            let base = if super::super::api::to_boolean(Some(&is_bag), false) {
                i64::from(vale_protocol::play::items::SERVER_BANK_BAG_START)
            } else {
                i64::from(vale_protocol::play::items::SERVER_BANK_START)
            };
            Ok(base + id.unwrap_or(0))
        })?,
    )?;
    // **`ContainerIDToInventoryID(bag)`** — the inventory slot a bag hangs in:
    // 1..4 are the belt's `INVSLOT_BAG1..4` (20..23), 5..10 the bank's six,
    // which are the same 64..69 the call above answers for a bank bag. A bag
    // addon asks it to draw each bag's own icon.
    globals.set(
        "ContainerIDToInventoryID",
        scope.create_function(move |_, id: Option<i64>| {
            let id = id.unwrap_or(0);
            Ok(match id {
                1..=4 => 19 + id,
                5..=10 => i64::from(vale_protocol::play::items::SERVER_BANK_BAG_START) + id - 4,
                _ => 0,
            })
        })?,
    )?;
    Ok(())
}

/// The queue the writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::game::npc::bank::BankPress>>>;

/// Register the two writes. Unscoped — they record.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    use crate::game::npc::bank::BankPress as P;
    let globals = lua.globals();
    for (name, press) in [("PurchaseSlot", P::PurchaseSlot), ("CloseBankFrame", P::Close)] {
        let queue = std::rc::Rc::clone(queue);
        let f = lua.create_function(move |_, _: mlua::MultiValue| {
            queue.borrow_mut().push(press);
            Ok(())
        })?;
        globals.set(name, f)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The read list is sorted and each name is registered exactly once — the
    /// same shape every panel's list is checked in.
    #[test]
    fn the_read_list_is_sorted_and_unique() {
        let mut sorted = super::READS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.as_slice(), super::READS.as_slice());
    }
}
