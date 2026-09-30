//! `vale itemset`: the item plate's set block, enchantment lines and random
//! suffixes, checked against the archives with no window and no server.
//!
//! Three DBCs feed the parts of an item plate that the item's template does
//! not: `ItemSet.dbc` (the set block), `SpellItemEnchantment.dbc` (one line per
//! enchantment on the copy in hand) and `ItemRandomProperties.dbc` (the suffix
//! and the enchantments it grants). The rules that read them are in
//! [`vale_assets::tables::itemset`] and [`vale_assets::tables::enchant`]; this
//! command runs the same code over the whole of each table.
//!
//! ## Forms
//!
//! * `vale itemset`: the census. Set rows, the piece-count histogram, the rows
//!   whose bonuses are stored out of threshold order, the sets with a skill
//!   requirement, and for the two enchantment tables the row counts and
//!   whether every enchantment a suffix grants names a row.
//! * `vale itemset <id>`: one set, with its bonuses in the order the plate
//!   draws them and the bonus lines the plate would draw at every owned count.
//! * `vale itemset suffix <id>`: one random property, with the name it gives a
//!   sample item and the enchantment lines it adds.

use crate::common::{open_assets, open_display_tables};
use vale_assets::interface::strings::{substitute_all, Strings, GLOBAL_STRINGS};
use vale_assets::tables::dbc::DisplayTables;
use vale_config::Config;

/// The level set bonus sentences are described at.
const LEVEL: u32 = 60;

pub fn cmd_itemset(cfg: &Config, first: Option<&str>, second: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let strings = Strings::parse(&assets.read(GLOBAL_STRINGS).unwrap_or_default());
    match (first, second) {
        (None, _) => census(&tables),
        (Some("suffix"), Some(id)) => suffix(&tables, &strings, parse_id(id)?),
        (Some(id), _) => one_set(&tables, &strings, parse_id(id)?),
    }
}

fn parse_id(text: &str) -> Result<u32, String> {
    text.parse()
        .map_err(|_| format!("expected a numeric id, got {text:?}"))
}

fn census(tables: &DisplayTables) -> Result<(), String> {
    let sets = tables.item_sets();
    println!("ItemSet.dbc: {} sets", sets.len());
    let mut histogram = std::collections::BTreeMap::new();
    for set in sets.in_order() {
        *histogram.entry(set.items.len()).or_insert(0usize) += 1;
    }
    println!("  pieces per set:");
    for (pieces, count) in &histogram {
        println!("    {pieces:>2} pieces  {count:>3} sets");
    }
    let gated: Vec<String> = sets
        .in_order()
        .into_iter()
        .filter(|set| set.required_skill != 0)
        .map(|set| {
            format!(
                "{} {:?} (skill {} at {})",
                set.id, set.name, set.required_skill, set.required_skill_rank
            )
        })
        .collect();
    println!("  sets with a skill requirement: {}", gated.len());
    for line in gated {
        println!("    {line}");
    }

    let enchantments = tables.enchantments();
    let properties = tables.random_properties();
    println!("\nSpellItemEnchantment.dbc: {} rows", enchantments.len());
    println!("ItemRandomProperties.dbc: {} rows", properties.len());
    let mut named = 0usize;
    let mut granted = 0usize;
    let mut unresolved = Vec::new();
    for id in 1..=u16::MAX as i32 {
        let Some(row) = properties.get(id) else {
            continue;
        };
        if !row.suffix.is_empty() {
            named += 1;
        }
        for enchantment in row.enchantments.iter().filter(|e| **e != 0) {
            granted += 1;
            if enchantments.name(*enchantment as i32).is_none() {
                unresolved.push((id, *enchantment));
            }
        }
    }
    println!("  rows with a suffix: {named}");
    println!(
        "  enchantments granted: {granted}, of which {} name no SpellItemEnchantment row",
        unresolved.len()
    );
    for (id, enchantment) in unresolved.iter().take(10) {
        println!("    property {id} -> enchantment {enchantment}");
    }
    Ok(())
}

fn one_set(tables: &DisplayTables, strings: &Strings, id: u32) -> Result<(), String> {
    let set = tables
        .item_sets()
        .get(id)
        .ok_or_else(|| format!("no ItemSet.dbc row {id}"))?;
    let catalog = tables.spellbook();
    let describe = |spell: u32| -> String {
        let Some(catalog) = catalog else {
            return format!("spell {spell}");
        };
        let Some(info) = catalog.info(spell) else {
            return format!("spell {spell} (no Spell.dbc row)");
        };
        let sentence =
            vale_assets::tables::spelltext::describe(&info, LEVEL, Some(catalog), None);
        if sentence.is_empty() {
            info.name.clone()
        } else {
            sentence
        }
    };
    println!("ItemSet.dbc row {id}: {:?}", set.name);
    println!("  pieces ({}): {:?}", set.items.len(), set.items);
    if set.required_skill != 0 {
        let name = tables
            .skills()
            .and_then(|s| s.line(set.required_skill))
            .map_or_else(|| "?".to_string(), |line| line.name.clone());
        println!(
            "  requires skill {} {name:?} at {}",
            set.required_skill, set.required_skill_rank
        );
    }
    println!("  bonuses in threshold order:");
    for bonus in &set.bonuses {
        println!(
            "    ({}) spell {:>5}  {}",
            bonus.threshold,
            bonus.spell,
            describe(bonus.spell)
        );
    }
    println!("\n  the bonus lines at each owned count, skill met:");
    for owned in 0..=set.items.len() {
        let name = strings
            .get("ITEM_SET_NAME")
            .map(|f| substitute_all(f, &[&set.name, &owned.to_string(), &set.items.len().to_string()]))
            .unwrap_or_default();
        println!("    {name}");
        for (bonus, active) in set.bonuses.iter().zip(set.active(owned, true)) {
            let sentence = describe(bonus.spell);
            let line = if active {
                strings
                    .get("ITEM_SET_BONUS")
                    .map(|f| format!("green  {}", substitute_all(f, &[&sentence])))
            } else {
                strings.get("ITEM_SET_BONUS_GRAY").map(|f| {
                    format!(
                        "grey   {}",
                        substitute_all(f, &[&bonus.threshold.to_string(), &sentence])
                    )
                })
            };
            println!("      {}", line.unwrap_or_default());
        }
    }
    Ok(())
}

fn suffix(tables: &DisplayTables, strings: &Strings, id: u32) -> Result<(), String> {
    let id = i32::try_from(id).map_err(|_| format!("id {id} is out of range"))?;
    let row = tables
        .random_properties()
        .get(id)
        .ok_or_else(|| format!("no ItemRandomProperties.dbc row {id}"))?;
    println!("ItemRandomProperties.dbc row {id}: suffix {:?}", row.suffix);
    if let Some(template) = strings.get("ITEM_SUFFIX_TEMPLATE") {
        println!(
            "  a sample name: {:?}",
            substitute_all(template, &["Bandit's Sword", &row.suffix])
        );
    }
    println!("  enchantment lines, white, in slots 3..:");
    for (slot, enchantment) in row.enchantments.iter().enumerate() {
        if *enchantment == 0 {
            continue;
        }
        let name = tables
            .enchantments()
            .name(*enchantment as i32)
            .unwrap_or("(no SpellItemEnchantment row)");
        println!("    slot {}  {enchantment:>5}  {name}", slot + 3);
    }
    Ok(())
}
