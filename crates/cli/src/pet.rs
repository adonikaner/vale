//! `vale pet` — **what a pet is**, checked against the archives with no
//! window and no server.
//!
//! The pet panel's whole content is a client-side rule over five DBCs, and none
//! of it crosses the wire: the happiness bands, the damage percentage under
//! them, the loyalty rate, the family's name and icon, the diet, and what a
//! stable slot costs. See [`vale_assets::tables::pet`].
//!
//! **A rule like that does not fail when it is wrong.** It draws a plausible
//! happiness icon off the wrong band, or a wolf with a cat's diet, which is why
//! this command prints the tables rather than a pass/fail.
//!
//! ## What the census reports
//!
//! * **The five row counts**, which is the cheapest thing that says a file
//!   changed shape: 2 personalities, 23 families, 8 foods, 6 loyalty levels,
//!   2 stable slot prices.
//! * **Every family against the archives** — its name, its icon (checked for a
//!   real BLP), and the diet its mask resolves to. A mask bit with no
//!   `ItemPetFood.dbc` row behind it is a family with a food nobody can name.
//! * **The happiness bands, traced at each of their edges**, which is the one
//!   arithmetic here: the level, the damage percentage the tooltip prints, and
//!   the loyalty rate whose *sign* chooses between "Gaining Loyalty" and
//!   "Losing Loyalty".
//! * **What the stable's two slots cost**, which is the one number
//!   `PetStable.lua` does arithmetic on: `GetNextStableSlotCost()` goes
//!   straight into `MoneyFrame_Update` and is compared against `GetMoney()` to
//!   decide whether the purchase button is enabled. The row is the slot being
//!   bought, so the answer for a character with none yet is row 1 — see
//!   [`vale_protocol::play::stable`], which is the packet half.
//!
//! ## …and one family traced
//!
//! `vale pet Wolf` (or `vale pet 1`) prints that family's mask bit by
//! bit, so a diet that reads wrong can be attributed to the mask or to the food
//! table rather than to the join.

use crate::common::*;
use vale_config::Config;

/// The happiness values worth printing a row for: either side of each band's
/// edge, plus the two ends.
///
/// **Either side, not on it**, because the comparison is `>=` and an off-by-one
/// reading of it is invisible at the threshold itself.
const PROBES: [i32; 8] = [-1, 0, 332_999, 333_000, 665_999, 666_000, 1_050_000, 1_050_001];

/// Copper as the three coins, which is what `MoneyFrame_Update` draws it as —
/// printed here so a price can be read against the number a player remembers
/// (5 silver, then 5 gold) rather than against 500 and 50,000.
fn money(copper: u32) -> String {
    let (gold, silver, copper) = (copper / 10_000, (copper / 100) % 100, copper % 100);
    let mut out = String::new();
    if gold > 0 {
        out.push_str(&format!("{gold}g "));
    }
    if gold > 0 || silver > 0 {
        out.push_str(&format!("{silver}s "));
    }
    out.push_str(&format!("{copper}c"));
    out
}

pub fn cmd_pet(cfg: &Config, which: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let pet = tables.pet();
    let (personalities, families, foods, loyalties, prices) = pet.counts();

    println!("== the five tables a pet is made of ==");
    println!(
        "  PetPersonality.dbc: {personalities} rows;  CreatureFamily.dbc: {families};  \
         ItemPetFood.dbc: {foods};  PetLoyalty.dbc: {loyalties};  StableSlotPrices.dbc: {prices}"
    );
    if personalities == 0 || families == 0 {
        return Err(
            "PetPersonality.dbc or CreatureFamily.dbc is not in the archive chain".to_string(),
        );
    }

    println!();
    println!("== the loyalty levels, which are a SetText argument and nothing else ==");
    for level in 1..=8 {
        match pet.loyalty(level) {
            Some(name) => println!("  {level}  {name}"),
            None => println!("  {level}  —"),
        }
    }

    println!();
    println!("== the stable, whose slots are the one thing here with a price ==");
    // **`bought` rather than `slot`**: the answer for a character who owns
    // `bought` slots is the row `bought + 1`, which is the off-by-one that
    // would grey the purchase button out for somebody who can afford it.
    for bought in 0..=u32::from(vale_protocol::play::stable::MAX_STABLE_SLOTS) {
        let cost = pet.stable_slot_cost(bought);
        let owned = match bought {
            0 => "none bought".to_string(),
            n => format!("{n} bought"),
        };
        match cost {
            0 => println!("  {owned}: nothing left to buy"),
            copper => println!("  {owned}: next slot costs {}", money(copper)),
        }
    }
    for (slot, copper) in pet.stable_prices() {
        println!("    StableSlotPrices.dbc row {slot} = {copper} copper");
    }

    if let Some(which) = which {
        return one(pet, which);
    }

    println!();
    println!("== the happiness bands, at each edge ==");
    for (id, row) in pet.personalities() {
        println!(
            "  personality {id}: thresholds {:?}, damage {:?}, loyalty {:?}",
            row.thresholds, row.damage, row.loyalty
        );
        for happiness in PROBES {
            let Some((level, damage, loyalty)) = pet.happiness(id, happiness) else {
                continue;
            };
            // The sign is the whole of the third answer's use — see
            // `PetFrame_SetHappiness`.
            let word = match loyalty {
                l if l < 0.0 => "Losing Loyalty",
                l if l > 0.0 => "Gaining Loyalty",
                _ => "—",
            };
            println!("      {happiness:>9}  level {level}  {damage:>5.0}% damage  {word}");
        }
    }
    // **The one every pet in a live session uses**, because vmangos sends a
    // zero personality for every creature in the game.
    println!(
        "  a personality nothing knows falls back to {}: {:?}",
        vale_assets::tables::pet::DEFAULT_PERSONALITY,
        pet.happiness(0, 400_000)
    );

    println!();
    println!("== every family: its name, its icon, and what it eats ==");
    let mut iconless = 0usize;
    let mut missing_art = Vec::new();
    let mut dietless = Vec::new();
    for (id, family) in pet.families() {
        let diet = pet.foods_for(id);
        if family.icon.is_empty() {
            iconless += 1;
        } else if assets.read(&format!("{}.blp", family.icon)).is_err() {
            missing_art.push(family.icon.clone());
        }
        if diet.is_empty() {
            dietless.push(family.name.clone());
        }
        println!(
            "  {id:>3}  {:<14} mask {:#06x}  {:<28}  {}",
            family.name,
            family.food_mask,
            diet.join(", "),
            family.icon
        );
    }
    println!();
    println!("  {iconless} family(s) name no icon; {} name one the archives do not have", missing_art.len());
    for icon in &missing_art {
        println!("    MISSING {icon}");
    }
    println!("  {} family(s) can be fed nothing: {}", dietless.len(), dietless.join(", "));
    Ok(())
}

/// One family, mask bit by mask bit.
fn one(pet: &vale_assets::tables::pet::PetTables, which: &str) -> Result<(), String> {
    let wanted = which.parse::<u32>().ok();
    let Some((id, family)) = pet.families().into_iter().find(|(id, family)| {
        wanted == Some(*id) || family.name.eq_ignore_ascii_case(which)
    }) else {
        return Err(format!(
            "no creature family named {which}; try one of: {}",
            pet.families()
                .iter()
                .map(|(_, f)| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };

    println!();
    println!("== family {id}: {} ==", family.name);
    println!("  icon       {}", family.icon);
    println!("  food mask  {:#010x}", family.food_mask);
    println!("  the mask, bit by bit — `1 << (foodId - 1)`:");
    for (food, name) in pet.foods() {
        let bit = food.checked_sub(1).map(|b| 1u32 << b).unwrap_or(0);
        let set = family.food_mask & bit != 0;
        println!(
            "    food {food:>2} {name:<10} bit {bit:#06x}  {}",
            if set { "yes" } else { "—" }
        );
    }
    println!("  diet: {}", pet.foods_for(id).join(", "));
    Ok(())
}
