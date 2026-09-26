//! `vale maps` — every map that has a WDT, with its tile count.

use crate::common::*;
use vale_assets::world::wdt::Wdt;
use vale_config::Config;

pub fn cmd_maps(cfg: &Config) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let wdts: Vec<String> = assets
        .list_prefix("world\\maps\\")
        .into_iter()
        .filter(|p| p.ends_with(".wdt"))
        .collect();

    println!("{} map(s) with a WDT:", wdts.len());
    for path in wdts {
        // world\maps\<name>\<name>.wdt
        let name = path
            .rsplit('\\')
            .next()
            .and_then(|f| f.strip_suffix(".wdt"))
            .unwrap_or("?")
            .to_string();

        match assets.read(&path).map_err(|e| e.to_string()).and_then(|b| {
            Wdt::parse(&b).map_err(|e| e.to_string())
        }) {
            Ok(wdt) => {
                let kind = if wdt.is_wmo_only() { "WMO-only" } else { "terrain" };
                println!("  {name:<28} {:>4} tiles  {kind}", wdt.tile_count());
            }
            Err(e) => println!("  {name:<28}   ??  ({e})"),
        }
    }
    Ok(())
}
