//! `vale book` — **the spellbook's own pages**, built from the archives.
//!
//! `vale spellbook` says what a spell is to a *button*. This says what the
//! *panel* is: which tabs a class has, in the client's own order, and which
//! spells land on each — the arithmetic `SpellBookFrame.lua` runs and the one
//! thing about the panel that nothing else in this harness can check.
//!
//! The order is the whole point. `GetSpellTabInfo(i)` hands the interface an
//! offset into one flat array and it slices twelve at a time out of it, so a
//! book whose runs are a row out draws real spells on the wrong pages — and
//! that is the failure mode this command exists to make visible, because it
//! looks *plausible* on screen. See [`vale_assets::tables::book`], where the two
//! comparators are transcribed with the addresses they came from.
//!
//! **A class rather than a character**, because the check must not need a
//! server. The stand-in for "what this character knows" is
//! [`vale_assets::tables::skills::Skills::class_abilities`] — every spell whose own
//! `SkillLineAbility` row names the class — which is a superset of any real
//! character's book and runs through exactly the same sort. `vale book Mage`
//! prints one class page by page.
//!
//! **One thing it therefore cannot show, and it is not a gap in the rule.** The
//! professions and the shared weapon skills are left out of the stand-in,
//! because their `SkillLineAbility` rows carry no class mask at all — what keeps
//! them off a warrior's pages is that he has not learned them, which is a
//! session fact. `vale-client --audit --script "ToggleSpellBook('spell')"` is
//! the check that has them.
//!
//! The General tab **is** here, and its arrival is the tab gate: the stand-in is
//! derived from the ability table, so every spell in it has a skill line by
//! construction, but a line the character's own `SkillRaceClassInfo` row refuses
//! sends its spells to General anyway. A warrior's `Shoot Bow`, `Throw`, `Block`
//! and `Parry` are on that page for exactly the reason the real client puts them
//! there. See [`vale_assets::tables::skills`].

use crate::common::{open_assets, open_display_tables};
use vale_assets::tables::book::{Spellbook, SPELLS_PER_PAGE};
use vale_assets::interface::strings::{Strings, GLOBAL_STRINGS};
use vale_config::Config;

/// The eight classes 1.12 ships, by the id the server sends. Named here rather
/// than read from `ChrClasses.dbc` because what this command needs is the *mask
/// bit*, and the bit is the id — reading the table would add a lookup and no
/// fact.
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

/// A human race id, for the mask filter. Any race would do for the survey — the
/// race masks separate racials, not class pages — and picking one keeps the
/// output comparable between runs.
const RACE_HUMAN: u8 = 1;

pub fn cmd_book(cfg: &Config, class: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let strings = Strings::parse(&assets.read(GLOBAL_STRINGS).unwrap_or_default());
    let spells = tables
        .spellbook()
        .ok_or("Spell.dbc is not in the archive chain")?;
    let skills = tables.skills();

    println!("== the three tables the pages come out of ==");
    match skills {
        Some(s) => {
            println!(
                "  SkillLine.dbc: {} lines;  SkillLineAbility.dbc: {} spells have one",
                s.line_count(),
                s.ability_count()
            );
            match s.tab_counts() {
                Some((lines, open)) => println!(
                    "  SkillRaceClassInfo.dbc: {lines} lines gated, {open} of them may open a tab"
                ),
                None => println!(
                    "  SkillRaceClassInfo.dbc absent — no gate, so every weapon skill and \
                     profession opens a page of its own"
                ),
            }
        }
        None => println!("  absent — every spell falls to General, so the book is one long tab"),
    }
    println!(
        "  {GLOBAL_STRINGS}: GENERAL = {:?}",
        strings.get(vale_assets::tables::book::GENERAL_NAME_KEY)
    );

    let Some(skills) = skills else {
        return Ok(());
    };

    match class {
        Some(name) => one(name, spells, skills, &strings),
        None => survey(spells, skills, &strings),
    }
}

/// Every class's book, one line each — the check that the sort holds up over
/// nine different shapes rather than the one that was worked by hand.
fn survey(
    spells: &vale_assets::tables::spellbook::Spells,
    skills: &vale_assets::tables::skills::Skills,
    strings: &Strings,
) -> Result<(), String> {
    println!();
    println!("== every class's pages, in the client's own tab order ==");
    for (id, name) in CLASSES {
        let learnable = skills.class_abilities(RACE_HUMAN, id);
        let book = Spellbook::build(&learnable, RACE_HUMAN, id, spells, Some(skills));
        let tabs: Vec<String> = (1..=book.tabs.len())
            .map(|i| {
                let tab = book.tab(i).expect("in range");
                format!("{} {}", book.tab_name(i, |k| strings.get(k).map(str::to_string)), tab.count)
            })
            .collect();
        println!(
            "  {name:8} {:4} spells   {}",
            book.spells.len(),
            tabs.join(" | ")
        );
        check(&book, skills, id);
    }
    println!();
    println!("  the count after each tab name is how many spells are on it;");
    println!("  General is first by rule and the rest are alphabetical — see assets::book.");
    Ok(())
}

/// One class, page by page.
fn one(
    class: &str,
    spells: &vale_assets::tables::spellbook::Spells,
    skills: &vale_assets::tables::skills::Skills,
    strings: &Strings,
) -> Result<(), String> {
    let (id, name) = CLASSES
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(class))
        .copied()
        .ok_or_else(|| {
            format!(
                "unknown class {class:?}; try one of: {}",
                CLASSES
                    .iter()
                    .map(|(_, n)| *n)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    let learnable = skills.class_abilities(RACE_HUMAN, id);
    let book = Spellbook::build(&learnable, RACE_HUMAN, id, spells, Some(skills));

    println!();
    println!("== {name}: {} spells over {} tabs ==", book.spells.len(), book.tabs.len());
    for index in 1..=book.tabs.len() {
        let tab = book.tab(index).expect("in range");
        println!();
        println!(
            "  tab {index}  {}  skill line {}  offset {}  {} spells over {} page(s)",
            book.tab_name(index, |k| strings.get(k).map(str::to_string)),
            tab.skill_line,
            tab.offset,
            tab.count,
            tab.pages()
        );
        println!("        icon {}", tab.icon.as_deref().unwrap_or("(none)"));
        for page in 1..=tab.pages() {
            // The panel's own arithmetic, transcribed from
            // `SpellBook_GetSpellID`: a button's index, plus the tab's offset,
            // plus twelve per page turned — **and the bound**, which is the
            // second half of the same rule. `SpellButton_UpdateButton` hides a
            // button whose `id > offset + numSpells`, so the last page of a tab
            // is short rather than borrowing from the next one. Without it a
            // page of eleven spells shows a twelfth that belongs to another tab,
            // which is a real spell in the wrong place.
            let first = tab.offset + SPELLS_PER_PAGE * (page - 1);
            let shown: Vec<String> = (1..=SPELLS_PER_PAGE)
                .filter(|button| button + first <= tab.offset + tab.count)
                .filter_map(|button| book.spell(button + first))
                .map(|info| info.label())
                .collect();
            if shown.is_empty() {
                continue;
            }
            println!("        page {page}: {}", shown.join(", "));
        }
    }
    check(&book, skills, id);
    Ok(())
}

/// **The three invariants the panel's arithmetic stands on**, checked rather
/// than assumed: the offsets tile the flat list with no gap and no overlap,
/// every spell really is under the tab its own skill line names, and **every tab
/// past General is a *class* skill line**.
///
/// The first two failing is a book that draws the right spells on the wrong
/// pages, which is exactly the kind of wrong that looks right.
///
/// The third is the cross-check on the tab gate, and it is worth having because
/// it reads a **different column from the one the gate is**: the gate is
/// `SkillRaceClassInfo.flags` bit 7 and this is `SkillLine.categoryId`, two
/// tables apart. A run of this command before the gate was read printed six
/// extra tabs per class — `Bows`, `Guns`, `Crossbows`, `Thrown`, `Defense`,
/// `Dual Wield` — every one of them category 6, and nothing said so.
fn check(book: &Spellbook, skills: &vale_assets::tables::skills::Skills, class: u8) {
    let mut expected = 0usize;
    for (index, tab) in book.tabs.iter().enumerate() {
        if let Some(line) = skills.line(tab.skill_line) {
            if line.category != vale_assets::tables::skills::CATEGORY_CLASS {
                println!(
                    "        BAD  tab {} is {line}, category {} — not a class page",
                    index + 1,
                    line.category
                );
            }
        }
        if tab.offset != expected {
            println!(
                "        BAD  tab {} ({}) starts at {} where the run before it ends at {expected}",
                index + 1,
                tab.skill_line,
                tab.offset
            );
        }
        expected += tab.count;
        // …and the run really holds that tab's spells. Re-derived from the same
        // lookup the build used, which sounds circular and is not: what it
        // catches is the *arithmetic* between the two — a tab whose offset
        // points into its neighbour's run.
        for slot in 1..=tab.count {
            let Some(info) = book.spell(tab.offset + slot) else {
                println!("        BAD  tab {} slot {slot} is past the end", index + 1);
                continue;
            };
            let line = skills.line_of(info.id, RACE_HUMAN, class);
            if line != tab.skill_line {
                println!(
                    "        BAD  {} is on line {line} and sits under tab {} (line {})",
                    info.label(),
                    index + 1,
                    tab.skill_line
                );
            }
        }
    }
    if expected != book.spells.len() {
        println!(
            "        BAD  the tabs account for {expected} spells and the list holds {}",
            book.spells.len()
        );
    }
}
