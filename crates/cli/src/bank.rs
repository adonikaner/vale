//! `vale bank` — **what a bank bag slot costs, and how the bank's thirty
//! squares are numbered**, checked against the archives with no window and
//! no server.
//!
//! The bank window's whole client-side content is one small table and three
//! numberings: `BankBagSlotPrices.dbc` prices the slot being bought
//! ([`vale_assets::tables::bank`]), and every square is addressed by a
//! Lua inventory id, a bag id and a wire pair that have to agree
//! ([`vale_protocol::play::items`]). None of it crosses the wire as a
//! number, so a wrong price or a slot off by one draws plausibly and is
//! refused by the server with nothing said on this end.
//!
//! ## What it reports
//!
//! * **The ladder**, every row — twelve, priced from 10 silver to 2,500 gold
//!   — with the six the character can hold marked, since `full` is six
//!   bought and not the end of the table.
//! * **`GetNumBankSlots` at every count**, 0..6, with the cost the panel
//!   would print beside it.
//! * **The three numberings crossed both ways** for the first and last of
//!   the squares and of the bag slots: `BankButtonIDToInvSlotID`'s id, the
//!   wire pair `server_inventory_slot` makes of it, the bag id
//!   `bag_id_of_inventory_slot` makes of a bag slot, and the pair
//!   `server_container_slot` makes of a square inside such a bag. Each is
//!   asserted, so a crossing that stops agreeing fails the command rather
//!   than printing a different number.

use crate::common::*;
use vale_config::Config;
use vale_protocol::play::items::{
    bag_id_of_inventory_slot, inventory_slot_of_bag_id, is_bank_position, server_container_slot,
    server_inventory_slot, BANK_BAG_SLOTS, BANK_CONTAINER, BANK_SLOTS, FIRST_BANK_BAG_ID,
    FIRST_BANK_BAG_INVENTORY_SLOT, FIRST_BANK_INVENTORY_SLOT, SERVER_BANK_BAG_START,
    SERVER_BANK_START,
};

pub fn cmd_bank(cfg: &Config) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let prices = tables.bank();

    println!("BankBagSlotPrices.dbc: {} rows", prices.ladder().len());
    for (slot, copper) in prices.ladder() {
        let held = if *slot <= vale_assets::tables::bank::BANK_BAG_SLOTS {
            ""
        } else {
            "  (no field to hold it: never bought)"
        };
        println!("  slot {slot:>2}  {}{held}", money(*copper));
    }

    println!("\nGetNumBankSlots() -> (bought, full), and GetBankSlotCost(bought):");
    for bought in 0..=vale_assets::tables::bank::BANK_BAG_SLOTS {
        let full = vale_assets::tables::bank::BankPrices::full(bought);
        let cost = prices.cost(bought);
        println!(
            "  {bought} bought  full={}  next {}",
            if full { "1" } else { "nil" },
            cost.map_or("(none)".to_string(), money),
        );
    }

    println!("\nthe three numberings:");
    println!("  {} squares: Lua {}..{}, wire slot {}..{}, bag id {BANK_CONTAINER}",
        BANK_SLOTS,
        FIRST_BANK_INVENTORY_SLOT,
        FIRST_BANK_INVENTORY_SLOT + BANK_SLOTS as u32 - 1,
        SERVER_BANK_START,
        SERVER_BANK_START + BANK_SLOTS as u8 - 1,
    );
    println!("  {} bag slots: Lua {}..{}, wire slot {}..{}, bag ids {}..{}",
        BANK_BAG_SLOTS,
        FIRST_BANK_BAG_INVENTORY_SLOT,
        FIRST_BANK_BAG_INVENTORY_SLOT + BANK_BAG_SLOTS as u32 - 1,
        SERVER_BANK_BAG_START,
        SERVER_BANK_BAG_START + BANK_BAG_SLOTS as u8 - 1,
        FIRST_BANK_BAG_ID,
        FIRST_BANK_BAG_ID + BANK_BAG_SLOTS as i32 - 1,
    );

    let mut failed = 0;
    let mut check = |label: &str, ok: bool| {
        println!("  {}  {label}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            failed += 1;
        }
    };
    // A square: button 1 is Lua 40 is wire (255, 39), and the same square by
    // bag id.
    check(
        "square 1: BankButtonIDToInvSlotID(1) = 40 -> (255, 39)",
        server_inventory_slot(FIRST_BANK_INVENTORY_SLOT) == Some((255, SERVER_BANK_START)),
    );
    check(
        "square 24: Lua 63 -> (255, 62), and Lua 64 is not a square",
        server_inventory_slot(63) == Some((255, 62)) && is_bank_position(255, 62),
    );
    check(
        "square 1 by bag id: (BANK_CONTAINER, 1) -> (255, 39)",
        server_container_slot(BANK_CONTAINER, 1) == Some((255, SERVER_BANK_START)),
    );
    check(
        "square 25 does not exist",
        server_container_slot(BANK_CONTAINER, 25).is_none(),
    );
    // A bag slot: button 1 with isBag is Lua 64 is wire (255, 63) is bag id 5.
    check(
        "bag slot 1: BankButtonIDToInvSlotID(1, 1) = 64 -> (255, 63), bag id 5",
        server_inventory_slot(64) == Some((255, SERVER_BANK_BAG_START))
            && bag_id_of_inventory_slot(64) == Some(FIRST_BANK_BAG_ID)
            && inventory_slot_of_bag_id(FIRST_BANK_BAG_ID) == Some(64),
    );
    check(
        "bag slot 6: Lua 69 -> (255, 68), bag id 10; Lua 70 is nothing",
        server_inventory_slot(69) == Some((255, 68))
            && bag_id_of_inventory_slot(69) == Some(10)
            && server_inventory_slot(70).is_none(),
    );
    check(
        "inside bag 5, square 1 -> (63, 0), which IsBankPos accepts",
        server_container_slot(5, 1) == Some((63, 0)) && is_bank_position(63, 0),
    );
    check(
        "inside bag 10, square 36 -> (68, 35); square 37 is nothing",
        server_container_slot(10, 36) == Some((68, 35)) && server_container_slot(10, 37).is_none(),
    );
    check(
        "the gap: Lua 24..39 and bag id 11 are nothing",
        (24..40).all(|id| server_inventory_slot(id).is_none())
            && server_container_slot(11, 1).is_none()
            && bag_id_of_inventory_slot(39).is_none(),
    );
    check(
        "the backpack's last square (255, 38) is not the bank",
        !is_bank_position(255, 38) && !is_bank_position(255, 69),
    );

    if failed > 0 {
        return Err(format!("{failed} numbering checks failed"));
    }
    Ok(())
}

/// Copper as the panel would print it.
fn money(copper: u32) -> String {
    let gold = copper / 10_000;
    let silver = (copper / 100) % 100;
    let bronze = copper % 100;
    match (gold, silver) {
        (0, 0) => format!("{bronze}c"),
        (0, _) => format!("{silver}s {bronze}c"),
        _ => format!("{gold}g {silver}s {bronze}c"),
    }
}
