//! `vale skills` — what the skills panel would draw, with no window and no
//! server.
//!
//! The check behind `GetNumSkillLines`/`GetSkillLineInfo`, and it exists for the
//! reason [`crate::reputation`]'s does: the wire carries 128 `(id, value, max,
//! bonus)` triplets in whatever order the server wrote them, and **everything
//! else the panel shows is a client-side rule** — which lines are listed at all,
//! what they are called, what heading they go under, and what order any of it is
//! in. A rule like that does not fail when it is wrong; it draws a plausible
//! list with the wrong things on it.
//!
//! ## What the census reports
//!
//! * **The eight headings and the column they sort by**, which is
//!   `SkillLineCategory.dbc` field 10 and nothing else — not the name and not
//!   the id. If that order is wrong the panel is wrong in a way that looks
//!   deliberate.
//! * **The three flags**, counted over `SkillRaceClassInfo.dbc`: how many rows
//!   say *always listed*, how many say *never*, and how many say *at level*.
//!   Those three decide whether a line the character has never used is on the
//!   panel, and they are the only thing that does.
//! * **What each class sees at level 60 with every line at its cap**, which is
//!   the end-to-end check. A warrior must see Arms, Fury and Protection under
//!   Class Skills and every weapon he can hold under Weapon Skills; a mage must
//!   see neither.
//!
//! ## …and one class traced
//!
//! `vale skills Warrior` prints that class's whole list the way the panel
//! would, heading by heading, with the rank arithmetic beside each bar.

use crate::common::*;
use vale_config::Config;

/// `(race, class, name)` — the class list, with a race that can be it.
///
/// **Human wherever it can be**, so the race column is held still and the class
/// column is the only thing moving; the two that a human cannot be take the race
/// that is closest to hand. A skill line's own row is selected by *both* masks,
/// so this is the pair the answer depends on and neither half can be left out.
const CLASSES: [(u8, u8, &str); 9] = [
    (1, 1, "Warrior"),
    (1, 2, "Paladin"),
    (1, 3, "Hunter"),
    (1, 4, "Rogue"),
    (1, 5, "Priest"),
    (2, 7, "Shaman"),
    (1, 8, "Mage"),
    (1, 9, "Warlock"),
    (4, 11, "Druid"),
];

/// The level the census pretends the character is. **60**, because two of the
/// three listing flags are level-gated and a level-1 character would report a
/// shorter list that is also correct — and the longer one is the one that
/// exercises the rule.
const LEVEL: u32 = 60;

pub fn cmd_skills(cfg: &Config, target: Option<&str>) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::tables::skills::{SkillEntry, SkillList, Skills};

    let mut assets = open_assets(cfg)?;
    let mut read = |table: &str| assets.read(&dbc_path(table)).unwrap_or_default();
    let ability = read("SkillLineAbility");
    let line = read("SkillLine");
    let icon = read("SpellIcon");
    let race_class = read("SkillRaceClassInfo");
    let category = read("SkillLineCategory");
    let skills = Skills::parse(&ability, &line, &icon, &race_class)
        .ok_or("SkillLineAbility.dbc or SkillLine.dbc would not parse")?
        .with_categories(&category);

    // **Every line at its own cap**, which is what makes the list the longest it
    // can be. The panel's `value == 0` test is what the flags gate on, so a
    // census built from an empty character would only ever exercise one of the
    // three branches.
    let everything: Vec<SkillEntry> = skills
        .line_ids()
        .into_iter()
        .map(|id| SkillEntry {
            id,
            // **Step 1**, so the census exercises the Unlearn branch: a step of
            // zero would report every row un-abandonable whatever the flag says.
            step: 1,
            value: 300,
            rank: 300,
            max_rank: 300,
            modifier: 0,
        })
        .collect();

    let list_for = |race: u8, class: u8| {
        let mut list = SkillList::default();
        list.build(&skills, race, class, LEVEL, &everything);
        list
    };

    // --- one class traced ---
    if let Some(target) = target {
        let Some(&(race, class, name)) = CLASSES
            .iter()
            .find(|(_, _, n)| n.eq_ignore_ascii_case(target))
        else {
            return Err(format!(
                "unknown class {target:?}; try one of: {}",
                CLASSES.map(|(_, _, n)| n).join(", ")
            ));
        };
        let list = list_for(race, class);
        println!("{name} (race {race}, class {class}) at level {LEVEL}\n");
        println!("  {} rows, {} displayed", list.rows().len(), list.len());
        for index in 1..=list.len() {
            let Some(row) = list.row(index) else { continue };
            if row.is_header {
                println!("\n  [{}]", row.name);
                continue;
            }
            println!(
                "    {:<28} {:>4}/{:<4} {:<6} line {}",
                row.name,
                row.rank,
                row.max_rank,
                if row.unstarted { "unused" } else { "" },
                row.line
            );
        }
        return Ok(());
    }

    // --- the census ---
    println!("SkillLine.dbc: {} lines", skills.line_count());
    println!("  headings, in the order the panel puts them:");
    for category in skills.categories_in_order() {
        println!("    sort {:>2}  {:<3} {}", category.sort, category.id, category.name);
    }
    let (always, never, at_level) = skills.flag_census();
    println!(
        "  SkillRaceClassInfo: {always} rows say always-listed, {never} never, \
         {at_level} at a level"
    );

    println!("\nwhat each class sees at level {LEVEL}:");
    for (race, class, name) in CLASSES {
        let list = list_for(race, class);
        let headings: Vec<String> = (1..=list.len())
            .filter_map(|i| list.row(i))
            .filter(|row| row.is_header)
            .map(|row| {
                let under = list
                    .rows()
                    .iter()
                    .filter(|other| !other.is_header && other.category == row.category)
                    .count();
                format!("{} {under}", row.name)
            })
            .collect();
        println!("  {name:<8} {:>3} lines   {}", list.len(), headings.join(", "));
    }
    Ok(())
}
