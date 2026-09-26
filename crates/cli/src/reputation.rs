//! `vale reputation` — what the reputation panel would draw, with no window
//! and no server.
//!
//! The check behind `GetNumFactions`/`GetFactionInfo`, and it exists for the
//! reason [`crate::portrait`]'s does: the panel's whole shape is a **client-side
//! rule**. The wire carries 64 pairs of numbers; which rows exist, which are
//! headings, what order they are in and where a bar starts and ends are all
//! decided here — see [`vale_assets::tables::reputation`], whose module note
//! carries the addresses.
//!
//! A rule like that does not fail when it is wrong. It draws fifteen plausible
//! bars with the wrong factions on them, which is why this command prints the
//! list rather than a pass/fail.
//!
//! ## What the census reports
//!
//! * **How many of `Faction.dbc`'s rows carry a `reputationListID`**, which is
//!   the population a character can ever have a bar for — 64 slots, and the file
//!   must not use one twice.
//! * **How many are headings**, which is flag bit 3 and not a shape. Five rows
//!   in the shipped file carry it; if that number moves, the reading of the bit
//!   is what moved.
//! * **What each of the eight playable races sees at level one**, which is the
//!   one end-to-end check: the list is built from `Faction.dbc`'s own defaults
//!   (vmangos' `ReputationMgr::Initialize`) and printed the way the panel would.
//!   A human must see Alliance over four Alliance cities and an orc must see
//!   Horde over four Horde ones — and Stormwind must be a human's *highest*,
//!   because their own capital's base is the one that differs.
//!
//! ## …and one race traced
//!
//! `vale reputation Tauren` prints that race's whole list with the bar
//! arithmetic beside it: the slot, the base, the delta, the standing, and both
//! ends of the band. Every one of those is a number the panel divides, so a
//! wrong band is visible here as a bar that is full at the wrong place.

use crate::common::*;
use vale_config::Config;

/// Every race whose list is worth printing, with the class used to resolve the
/// race/class alternatives.
///
/// **Warrior everywhere**, because it is the one class every race can be — and
/// because no row in the shipped file states a class mask without also stating
/// a race mask, so the class never changes the answer. Printed anyway, so that
/// a file where it did would be visible.
const RACES: [(u8, u8, &str); 8] = [
    (1, 1, "Human"),
    (2, 1, "Orc"),
    (3, 1, "Dwarf"),
    (4, 1, "NightElf"),
    (5, 1, "Undead"),
    (6, 1, "Tauren"),
    (7, 1, "Gnome"),
    (8, 1, "Troll"),
];

pub fn cmd_reputation(cfg: &Config, target: Option<&str>) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::tables::reputation::{flags, Factions, Reputation, SLOTS};

    let mut assets = open_assets(cfg)?;
    let raw = assets.read(&dbc_path("Faction")).map_err(|e| e.to_string())?;
    let factions = Factions::parse(&raw).map_err(|e| e.to_string())?;

    let list_for = |race: u8, class: u8| {
        let mut standing = Reputation::default();
        standing.rebuild(&factions, race, class, &factions.default_states(race, class));
        standing
    };

    // --- one race traced ---
    if let Some(target) = target {
        let Some(&(race, class, name)) = RACES
            .iter()
            .find(|(_, _, n)| n.eq_ignore_ascii_case(target))
        else {
            return Err(format!(
                "unknown race {target:?}; try one of: {}",
                RACES.map(|(_, _, n)| n).join(", ")
            ));
        };
        let standing = list_for(race, class);
        println!("{name} (race {race}, class {class}) — a fresh character's list\n");
        println!(
            "  {} rows built, {} displayed, {} headings",
            standing.rows().len(),
            standing.num_factions(),
            standing.headers().len()
        );
        println!(
            "\n  {:<4} {:<26} {:<4} {:>7} {:>7} {:>7}  {:>7}..{:<7}",
            "#", "faction", "slot", "base", "earned", "standing", "bar", ""
        );
        for index in 1..=standing.num_factions() {
            let Some(info) = standing.info(&factions, index) else {
                continue;
            };
            let slot = standing.reputation_id_at(index);
            let cell = usize::try_from(slot).ok().and_then(|s| standing.slots().get(s));
            if info.is_header {
                println!("  {index:<4} {:<26} {:<4}  <- heading", info.name, "");
                continue;
            }
            println!(
                "  {index:<4} {:<26} {slot:<4} {:>7} {:>7} {:>7}  {:>7}..{:<7}  {}",
                info.name,
                cell.map(|c| c.base).unwrap_or(0),
                cell.map(|c| c.standing).unwrap_or(0),
                info.bar_value,
                info.bar_min,
                info.bar_max,
                // The word is `GlobalStrings.lua`'s and this command has no
                // interface loaded, so the *number* is what is checkable here:
                // 1 is Hated and 8 Exalted.
                format!("standing {}/8", info.standing_id),
            );
            if info.at_war {
                println!("       at war{}", if info.can_toggle_at_war { "" } else { " (forced)" });
            }
        }
        return Ok(());
    }

    // --- the census ---
    println!("Faction.dbc: {} rows", factions.len());

    let mut slots: [Option<u32>; SLOTS] = [None; SLOTS];
    let (mut listed, mut headings, mut collisions, mut parentless) = (0, 0, 0, 0);
    for row in factions.descending() {
        let rep = row.reputation_list_id;
        if rep < 0 || rep as usize >= SLOTS {
            continue;
        }
        listed += 1;
        // **Two factions in one slot is a file this reading cannot be right
        // about**, because the wire is keyed by slot and nothing else.
        if let Some(other) = slots[rep as usize].replace(row.id) {
            collisions += 1;
            println!("  SLOT COLLISION  {rep}: {} and {}", other, row.id);
        }
        if row.flags.iter().any(|f| *f as u8 & flags::HEADER != 0) {
            headings += 1;
            println!("  heading  {:<3} {:<26} slot {rep}", row.id, row.name);
        }
        if row.parent == 0 {
            parentless += 1;
        }
    }
    println!(
        "  {listed} rows carry a reputationListID, {} slots used, {collisions} collisions",
        slots.iter().filter(|s| s.is_some()).count()
    );
    println!("  {headings} are headings (flag bit 3), {parentless} have no parent -> FACTION_OTHER");

    println!("\nwhat each race sees at level one:");
    for (race, class, name) in RACES {
        let standing = list_for(race, class);
        let rows: Vec<String> = (1..=standing.num_factions())
            .filter_map(|i| standing.info(&factions, i))
            .map(|info| {
                if info.is_header {
                    format!("[{}]", info.name)
                } else {
                    format!("{} {}", info.name, info.bar_value)
                }
            })
            .collect();
        println!("  {name:<9} {:>2} lines   {}", rows.len(), rows.join(", "));
    }
    Ok(())
}
