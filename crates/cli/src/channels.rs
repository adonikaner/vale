//! `vale channels` — the six chat channels, and which of them a place is
//! in.
//!
//! The rule is `assets::tables::channels`; this prints the table decoded,
//! the row the city channels are named after, the areas that allow trade,
//! and — with an area id — what a character standing there is joined to.
//! `vale channels 87` is Goldshire: General and LocalDefense for Elwynn
//! Forest and no Trade; a Trade District id adds `Trade - City`.

use crate::common::*;
use vale_assets::tables::channels::{flags, joined_bit};
use vale_config::Config;

pub fn cmd_channels(cfg: &Config, area: Option<u32>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let channels = tables
        .chat_channels()
        .ok_or("ChatChannels.dbc did not parse")?;
    let areas = tables.areas().ok_or("AreaTable.dbc did not parse")?;

    println!("ChatChannels.dbc: {} rows", channels.rows().len());
    for row in channels.rows() {
        let mut named = Vec::new();
        for (bit, name) in [
            (flags::INITIAL, "INITIAL"),
            (flags::ZONE_DEP, "ZONE_DEP"),
            (flags::GLOBAL, "GLOBAL"),
            (flags::TRADE, "TRADE"),
            (flags::CITY_ONLY, "CITY_ONLY"),
            (flags::CITY_ONLY2, "CITY_ONLY2"),
            (flags::DEFENSE, "DEFENSE"),
            (flags::GUILD_REQ, "GUILD_REQ"),
            (flags::LFG, "LFG"),
        ] {
            if row.has(bit) {
                named.push(name);
            }
        }
        println!(
            "  {:>2}  {:<24} {:<18} flags {:#07x} [{}]",
            row.id,
            row.pattern,
            row.shortcut,
            row.flags,
            named.join(" ")
        );
    }
    let mask = channels.default_joined();
    let initial: Vec<String> = channels
        .rows()
        .iter()
        .filter(|row| mask & joined_bit(row.id) != 0)
        .map(|row| row.shortcut.clone())
        .collect();
    println!("  joined on entering the world (ZONECHANNELS {mask}): {}", initial.join(", "));

    match areas.city_name() {
        Some(city) => println!("the city channels are named after: {city:?}"),
        None => println!("no area carries AREA_FLAG_CITY: the city channels cannot be named"),
    }
    let trading: Vec<String> = {
        let mut rows: Vec<_> = areas
            .zones()
            .flat_map(|zone| {
                std::iter::once(zone).chain(areas.iter().filter(move |a| a.parent == zone.id))
            })
            .filter(|area| area.allows_trade())
            .map(|area| format!("{} ({})", area.name, area.id))
            .collect();
        rows.sort();
        rows.dedup();
        rows
    };
    println!("{} areas allow trade; the first ten:", trading.len());
    for name in trading.iter().take(10) {
        println!("  {name}");
    }

    if let Some(area) = area {
        let here = areas.get(area).ok_or_else(|| format!("area {area} is not in AreaTable"))?;
        let zone = areas.zone_of(area).map_or("?", |zone| zone.name.as_str());
        println!(
            "\narea {area}: {} in {zone}, flags {:#x}, trade {}",
            here.name,
            here.flags,
            if here.allows_trade() { "allowed" } else { "not allowed" }
        );
        for channel in channels.wanted(areas, area, mask, true) {
            println!("  {:>2}  {}", channel.id, channel.name);
        }
    }
    Ok(())
}
