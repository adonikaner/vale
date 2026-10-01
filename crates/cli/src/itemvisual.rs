//! `vale itemvisual`: the glows and flames on held items, checked against the
//! archives with no window and no server.
//!
//! An item visual is an `ItemVisuals.dbc` row of five `ItemVisualEffects.dbc`
//! ids, each naming a model that hangs on the held item's own attachment point
//! of the same index. `ItemDisplayInfo` field 22 gives an item its own visual,
//! and `SpellItemEnchantment` field 22 gives one to an enchantment. The rules
//! are in [`vale_assets::tables::itemvisual`]; this command runs them over the
//! whole of each table.
//!
//! ## Forms
//!
//! * `vale itemvisual`: the census. Every visual with its five slots, how many
//!   display rows and enchantments name it, every effect model and whether the
//!   archives hold it, and the slot ids and column values that name no row.
//! * `vale itemvisual <display id> [enchantment id]`: one held item. The
//!   visual the dressing chooses, with the enchantment in the permanent slot
//!   when one is given, each model it hangs, and whether the item's own model
//!   carries the point each one hangs on.

use std::collections::BTreeMap;

use crate::common::{open_assets, open_display_tables};
use vale_assets::tables::dbc::DisplayTables;
use vale_assets::tables::item::Slot;
use vale_assets::tables::itemvisual::{self, SLOTS};
use vale_assets::world::m2::{attach, M2};
use vale_assets::Assets;
use vale_config::Config;

pub fn cmd_itemvisual(
    cfg: &Config,
    first: Option<&str>,
    second: Option<&str>,
) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    match first {
        None => census(&mut assets, &tables),
        Some(display) => {
            let enchantment = second.map(parse_id).transpose()?.unwrap_or(0);
            one_item(&mut assets, &tables, parse_id(display)?, enchantment)
        }
    }
}

fn parse_id(text: &str) -> Result<u32, String> {
    text.parse()
        .map_err(|_| format!("expected a numeric id, got {text:?}"))
}

fn census(assets: &mut Assets, tables: &DisplayTables) -> Result<(), String> {
    let visuals = tables.item_visuals();
    let items = tables.items().ok_or("ItemDisplayInfo.dbc is not in the archives")?;
    let enchantments = tables.enchantments();

    // Who names each visual: display rows by field 22, enchantments by theirs.
    let mut by_display: BTreeMap<u32, usize> = BTreeMap::new();
    let mut display_rows = 0usize;
    let mut without_model = 0usize;
    for id in items.ids() {
        let visual = items.item_visual(id);
        if visual == 0 {
            continue;
        }
        display_rows += 1;
        *by_display.entry(visual).or_default() += 1;
        let has_model = items
            .appearance(id, 0)
            .is_some_and(|look| look.has_geometry());
        if !has_model {
            without_model += 1;
        }
    }
    let mut by_enchantment: BTreeMap<u32, usize> = BTreeMap::new();
    for id in 1..=u16::MAX as u32 {
        let visual = enchantments.item_visual(id);
        if visual != 0 {
            *by_enchantment.entry(visual).or_default() += 1;
        }
    }

    println!(
        "ItemVisuals.dbc: {} rows; ItemVisualEffects.dbc: {} rows",
        visuals.len(),
        visuals.effect_ids().len()
    );
    println!("  visual  slots 0..4                          displays  enchantments");
    for id in visuals.ids() {
        let slots = visuals.slots(id).unwrap_or([0; SLOTS]);
        let shown: Vec<String> = slots
            .iter()
            .map(|effect| match (*effect, visuals.model(*effect)) {
                (0, _) => "-".to_string(),
                (effect, Some(_)) => effect.to_string(),
                (effect, None) => format!("{effect}?"),
            })
            .collect();
        println!(
            "  {id:>6}  {:<34}  {:>8}  {:>12}",
            shown.join(" "),
            by_display.get(&id).copied().unwrap_or(0),
            by_enchantment.get(&id).copied().unwrap_or(0),
        );
    }
    println!("  (an id followed by ? names no ItemVisualEffects row, and hangs nothing)");

    println!("\n  effect models:");
    let mut missing = 0usize;
    for effect in visuals.effect_ids() {
        let stored = visuals.model(effect).unwrap_or_default();
        if stored.is_empty() || stored.ends_with('\\') {
            println!("    {effect:>4}  {stored:<46} names no file");
            continue;
        }
        let path = vale_assets::world::m2::model_path(stored);
        let there = assets.exists(&path);
        if !there {
            missing += 1;
        }
        println!(
            "    {effect:>4}  {path:<46} {}",
            if there { "in the archives" } else { "NOT in the archives" }
        );
    }

    let unknown_display: Vec<String> = by_display
        .iter()
        .filter(|(visual, _)| !visuals.contains(**visual))
        .map(|(visual, count)| format!("{visual} ({count} rows)"))
        .collect();
    let unknown_enchantment: Vec<String> = by_enchantment
        .iter()
        .filter(|(visual, _)| !visuals.contains(**visual))
        .map(|(visual, count)| format!("{visual} ({count} rows)"))
        .collect();
    println!(
        "\n  ItemDisplayInfo rows naming a visual: {display_rows}, of which {without_model} have no model to hang it on"
    );
    println!(
        "  SpellItemEnchantment rows naming a visual: {}",
        enchantments.with_visual()
    );
    println!(
        "  display values naming no ItemVisuals row: {}",
        list_or_none(&unknown_display)
    );
    println!(
        "  enchantment values naming no ItemVisuals row: {}",
        list_or_none(&unknown_enchantment)
    );
    println!("  effect models not in the archives: {missing}");
    Ok(())
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".into()
    } else {
        items.join(", ")
    }
}

fn one_item(
    assets: &mut Assets,
    tables: &DisplayTables,
    display: u32,
    enchantment: u32,
) -> Result<(), String> {
    let items = tables.items().ok_or("ItemDisplayInfo.dbc is not in the archives")?;
    let look = items
        .appearance(display, 0)
        .ok_or_else(|| format!("display {display} is not an ItemDisplayInfo row"))?;
    let own = items.item_visual(display);
    let mut enchantments = [0u32; 7];
    enchantments[0] = enchantment;
    let visual = itemvisual::choose(
        own,
        &enchantments,
        tables.item_visuals(),
        tables.enchantments(),
    );
    println!("display {display}: model {:?}", look.models[0]);
    println!(
        "  own visual (field 22): {own}{}",
        if tables.item_visuals().contains(own) {
            ""
        } else if own == 0 {
            " (none)"
        } else {
            " (names no ItemVisuals row)"
        }
    );
    if enchantment != 0 {
        println!(
            "  enchantment {enchantment} {:?} in the permanent slot: visual {}",
            tables.enchantments().name(enchantment as i32).unwrap_or("?"),
            tables.enchantments().item_visual(enchantment)
        );
    }
    println!("  drawn: visual {visual}");

    // The held model the effects hang on, as the dressing resolves it for the
    // main hand.
    let Some(held) = look.attachment_at(Slot::MainHand, 0, attach::HAND_RIGHT, 0, 0) else {
        println!("  no model to hang it on: nothing is drawn");
        return Ok(());
    };
    let points = assets
        .read(&held.path)
        .ok()
        .and_then(|bytes| M2::parse(&bytes).ok())
        .map(|model| model.attachments);
    match &points {
        Some(points) => println!(
            "  {}: {} attachment points ({})",
            held.path,
            points.len(),
            points
                .iter()
                .map(|p| format!(
                    "{}@bone {} ({:.2}, {:.2}, {:.2})",
                    p.id, p.bone, p.position[0], p.position[1], p.position[2]
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        None => println!("  {}: will not read", held.path),
    }
    for effect in tables.item_visuals().effects(visual) {
        let carried = points
            .as_ref()
            .is_some_and(|points| points.iter().any(|p| p.id == effect.point));
        println!(
            "    point {}  {:<46} {}, {}",
            effect.point,
            effect.path,
            if assets.exists(&effect.path) { "in the archives" } else { "NOT in the archives" },
            if carried { "point carried" } else { "point NOT carried: dropped" }
        );
    }
    Ok(())
}
