//! `vale talent` — the three trees a class may spend in, with no window and
//! no server.
//!
//! The check behind `GetNumTalentTabs`, `GetTalentTabInfo`, `GetNumTalents`,
//! `GetTalentInfo` and `GetTalentPrereqs`, and it exists for a sharper version
//! of [`crate::skills`]' reason: **nothing about a talent tree crosses the
//! wire.** The tabs, their order, the tiers, the cells, the arrows between them
//! and even how many points are *spent* are all worked out client-side off two
//! DBCs and the character's known spells — so every one of them fails by drawing
//! a plausible tree with the wrong things in it.
//!
//! ## What the census reports
//!
//! * **Nine classes, three tabs each, in the file's own order**, with the
//!   `orderIndex` column printed beside it. That column is *not* what the client
//!   sorts on, and the mage is the proof: Arcane and Fire both carry 0, so a
//!   sort has a tie and one of the two answers it can give is wrong. See
//!   [`vale_assets::tables::talent`].
//! * **Every tab's picture and parchment against the archives** — the
//!   `SpellIcon.dbc` path for the tab button, and the four
//!   `Interface\TalentFrame\<file>-{TopLeft,TopRight,BottomLeft,BottomRight}`
//!   quarters the tree is drawn on. A missing quarter is a blank panel.
//! * **Every talent's face spell against `Spell.dbc`** — a talent whose rank-1
//!   spell has no row draws an unnamed button with no icon.
//! * **The grid**: how many tiers each tab uses, and whether any two talents
//!   land in the same cell, which would stack two buttons on one square.
//! * **The arrows**: how many talents name a prerequisite, and whether every one
//!   of those points at a real row in the same tab.
//!
//! ## …and one class traced
//!
//! `vale talent Warrior` prints that class's three trees the way the panel
//! lays them out — tier by tier, with each cell's talent, its rank spread and
//! the cell any arrow comes from.

use crate::common::*;
use vale_config::Config;

/// `(class, name)` — the nine, in class-id order.
///
/// No race column, unlike [`crate::skills`]': every shipped `TalentTab` row
/// carries `raceMask` 511, so the race has never yet excluded a tab. It is still
/// passed to the lookup, because the column exists and the client tests it.
const CLASSES: [(u8, &str); 9] = [
    (1, "Warrior"),
    (2, "Paladin"),
    (3, "Hunter"),
    (4, "Rogue"),
    (5, "Priest"),
    (7, "Shaman"),
    (8, "Mage"),
    (9, "Warlock"),
    (11, "Druid"),
];

/// A race every class check is run as. Human — see [`CLASSES`]; the mask has
/// never excluded anything, so holding it still costs nothing and makes the
/// class column the only thing moving.
const RACE: u8 = 1;

/// The four quarters `TalentFrame_Update` builds from the background stem.
const QUARTERS: [&str; 4] = ["TopLeft", "TopRight", "BottomLeft", "BottomRight"];

pub fn cmd_talent(cfg: &Config, target: Option<&str>) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::tables::spellbook::Spells;
    use vale_assets::tables::talent::{Talents, BACKGROUND_DIR, POINTS_PER_TIER};

    let mut assets = open_assets(cfg)?;
    let mut read = |table: &str| assets.read(&dbc_path(table)).unwrap_or_default();
    let talent = read("Talent");
    let talent_tab = read("TalentTab");
    let spell = read("Spell");
    let icons = read("SpellIcon");
    let talents = Talents::parse(&talent, &talent_tab)
        .ok_or("Talent.dbc or TalentTab.dbc would not parse")?;
    // Only the name and the icon are wanted here, but `Spells::parse` is the one
    // door into `Spell.dbc` and the other five tables degrade to nothing on
    // their own terms — see its own doc.
    let catalog = Spells::parse(&spell, &[], &[], &icons, &[], &[], &[]).ok();

    // --- one class traced ---
    if let Some(target) = target {
        let Some(&(class, name)) = CLASSES
            .iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(target))
        else {
            return Err(format!(
                "unknown class {target:?}; try one of: {}",
                CLASSES.map(|(_, n)| n).join(", ")
            ));
        };
        // **An empty known-spell set**, so every rank reads 0 and every button
        // is the untrained one. A tree with points in it is a *session*
        // question and `vale-client --script` is where that is checked; what
        // this traces is the layout, which does not depend on it.
        let tree = vale_assets::tables::talent::TalentTree::build(
            &talents, RACE, class, &[], catalog.as_ref(),
        );
        println!("{name} (class {class}) — {} tabs\n", tree.num_tabs());
        for index in 1..=tree.num_tabs() {
            let Some(tab) = tree.tab(index) else { continue };
            println!(
                "  [{index}] {} ({} talents)  parchment {}{}-*",
                tab.name,
                tab.talents.len(),
                BACKGROUND_DIR,
                tab.background
            );
            println!("      icon {}", or_none(&tab.icon));
            let tiers = tab.talents.iter().map(|t| t.tier).max().unwrap_or(0);
            for tier in 1..=tiers {
                println!(
                    "      tier {tier}  (opens at {} points)",
                    (tier - 1) * POINTS_PER_TIER
                );
                for talent in tab.talents.iter().filter(|t| t.tier == tier) {
                    let arrows: Vec<String> = talent
                        .prereqs
                        .iter()
                        .map(|p| format!("<- {},{}", p.tier, p.column))
                        .collect();
                    println!(
                        "        col {}  {:<30} 0/{}{}{} {}",
                        talent.column,
                        or_none(&talent.name),
                        talent.max_rank,
                        if talent.exceptional { " *" } else { "  " },
                        if talent.meets_prereq { "" } else { " [needs a spell]" },
                        arrows.join(" ")
                    );
                }
            }
            println!();
        }
        return Ok(());
    }

    // --- the census ---
    println!(
        "TalentTab.dbc: {} tabs      Talent.dbc: {} talents",
        talents.tabs().len(),
        talents.talents().len()
    );
    let ranks: usize = talents
        .talents()
        .iter()
        .map(|t| t.max_rank() as usize)
        .sum();
    let exceptional = talents.talents().iter().filter(|t| t.exceptional()).count();
    let with_prereq = talents
        .talents()
        .iter()
        .filter(|t| t.prereq_talent.iter().any(|id| *id != 0))
        .count();
    let with_spell = talents
        .talents()
        .iter()
        .filter(|t| t.required_spell != 0)
        .count();
    println!(
        "  {ranks} ranks in all, {exceptional} exceptional, \
         {with_prereq} with an arrow into them, {with_spell} gated on a spell"
    );

    // **Every arrow lands somewhere, and in the same tab.** A prereq that named
    // a row in another tree would draw a line off the edge of the panel —
    // `TalentFrame_DrawLines` indexes `TALENT_BRANCH_ARRAY` by the *dependency's*
    // tier and column with no check that it belongs here.
    let mut dangling = 0;
    let mut crossed = 0;
    for row in talents.talents() {
        for id in row.prereq_talent.iter().copied().filter(|id| *id != 0) {
            match talents.talent(id) {
                None => dangling += 1,
                Some(dep) if dep.tab != row.tab => crossed += 1,
                Some(_) => {}
            }
        }
    }
    println!("  arrows: {dangling} dangling, {crossed} across tabs (both must be 0)");

    let mut missing_art = 0;
    let mut missing_spell = 0;
    let mut collisions = 0;
    println!("\nwhat each class sees:");
    for (class, name) in CLASSES {
        let tree = vale_assets::tables::talent::TalentTree::build(
            &talents, RACE, class, &[], catalog.as_ref(),
        );
        let mut lines = Vec::new();
        for index in 1..=tree.num_tabs() {
            let Some(tab) = tree.tab(index) else { continue };
            let order = talents
                .tabs()
                .iter()
                .find(|row| row.id == tab.id)
                .map_or(0, |row| row.order);
            let tiers = tab.talents.iter().map(|t| t.tier).max().unwrap_or(0);
            lines.push(format!(
                "{} ({} in {tiers} tiers, order {order})",
                tab.name,
                tab.talents.len()
            ));
            // Art: the tab's own button, and the four parchment quarters.
            for quarter in QUARTERS {
                let path = format!("{BACKGROUND_DIR}{}-{quarter}.blp", tab.background);
                if assets.read(&path).is_err() {
                    println!("    missing parchment {path}");
                    missing_art += 1;
                }
            }
            if tab.icon.is_empty() {
                println!("    tab {} has no icon row", tab.name);
                missing_art += 1;
            }
            // A cell may hold one talent and no more.
            let mut cells: Vec<(u32, u32)> =
                tab.talents.iter().map(|t| (t.tier, t.column)).collect();
            cells.sort_unstable();
            let before = cells.len();
            cells.dedup();
            collisions += before - cells.len();
            for talent in &tab.talents {
                if talent.name.is_empty() {
                    println!("    talent {} has no Spell.dbc row", talent.id);
                    missing_spell += 1;
                }
            }
        }
        println!("  {name:<8} {}", lines.join(",  "));
    }
    println!(
        "\n  {missing_art} missing pictures, {missing_spell} unnamed talents, \
         {collisions} cells with two talents in them (all three must be 0)"
    );
    Ok(())
}

/// An empty string reads as nothing rather than as a blank column.
fn or_none(text: &str) -> &str {
    if text.is_empty() {
        "(none)"
    } else {
        text
    }
}
