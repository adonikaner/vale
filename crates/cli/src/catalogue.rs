//! `vale catalogue` — the archives' own listing of placeable models, and
//! what in it actually opens.
//!
//! The world editor's model picker is this listing, so what the listing says is
//! what a person is offered. Two things about it are not obvious from reading
//! `list_prefix` and both cost a round:
//!
//! * **the listing names files the archives do not hold.** `(listfile)` carries
//!   the pre-1.0 `.mdx` name beside the `.m2` the archive actually has, so a
//!   picker that offers the listing verbatim offers a path that resolves to
//!   nothing. Choosing one placed a record the file was perfectly happy with,
//!   showed no preview, drew nothing, and then appeared the moment anything made
//!   the tile be read again — because reading an `MDDF` goes through
//!   `m2::model_path`, which is the `.mdx` -> `.m2` swap the picker was not
//!   making.
//! * **most of the `.wmo` paths are not buildings.** A `.wmo` ending `_NNN` is
//!   one of a root's groups; Stormwind is one root and some three hundred rooms.
//!
//! With a prefix it lists what is under it, folder by folder, which is the shape
//! the picker draws.

use crate::common::*;
use vale_config::Config;
use std::collections::BTreeMap;

pub fn cmd_catalogue(cfg: &Config, prefix: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let listed = assets.list_prefix("world\\");
    println!("World\\ holds {} listed files", listed.len());

    let models: Vec<&String> = listed
        .iter()
        .filter(|p| p.ends_with(".m2") || p.ends_with(".mdx") || p.ends_with(".mdl"))
        .collect();
    let wmos: Vec<&String> = listed.iter().filter(|p| p.ends_with(".wmo")).collect();

    // What each extension contributes, and what is left once the `.mdx` and
    // `.mdl` names are resolved the way every reader in this project resolves
    // them.
    let mut by_ext: BTreeMap<&str, usize> = BTreeMap::new();
    for path in &models {
        let ext = path.rsplit('.').next().unwrap_or("");
        *by_ext.entry(ext).or_default() += 1;
    }
    println!("  models:");
    for (ext, count) in &by_ext {
        println!("    .{ext:<4} {count:>6}");
    }

    let mut resolved: Vec<String> = models
        .iter()
        .map(|p| vale_assets::world::m2::model_path(p))
        .collect();
    resolved.sort();
    resolved.dedup();
    println!(
        "    {} distinct once .mdx and .mdl are resolved to .m2",
        resolved.len()
    );

    // **The number the editor's picker is about.** A listed path that does not
    // open is a row somebody can choose and get nothing from.
    let raw_missing = models.iter().filter(|p| !assets.exists(p)).count();
    let resolved_missing = resolved.iter().filter(|p| !assets.exists(p)).count();
    println!(
        "    {raw_missing} of {} listed model paths do not open; \
         {resolved_missing} of {} do not once resolved",
        models.len(),
        resolved.len()
    );
    if raw_missing > 0 {
        let missing: Vec<String> = models
            .iter()
            .filter(|p| !assets.exists(p))
            .take(4)
            .map(|p| p.to_string())
            .collect();
        for path in missing {
            let swapped = vale_assets::world::m2::model_path(&path);
            let opens = assets.exists(&swapped);
            println!(
                "      {path}  ->  {swapped} {}",
                match opens {
                    true => "(which does open)",
                    false => "(which does not either)",
                }
            );
        }
    }

    let roots: Vec<&&String> = wmos.iter().filter(|p| !is_group(p)).collect();
    println!(
        "  buildings: {} .wmo paths, {} of them roots ({} are a root's own groups)",
        wmos.len(),
        roots.len(),
        wmos.len() - roots.len()
    );

    // The folders, which is the shape the picker draws rather than one list of
    // several thousand names.
    match prefix {
        None => {
            println!("\n  top-level folders under World\\ (models / buildings):");
            let mut folders: BTreeMap<String, (usize, usize)> = BTreeMap::new();
            for path in &resolved {
                folders.entry(first(path)).or_default().0 += 1;
            }
            for path in &roots {
                folders.entry(first(path)).or_default().1 += 1;
            }
            for (folder, (models, buildings)) in folders {
                println!("    {folder:<28} {models:>5}  {buildings:>5}");
            }
            println!("\n  `vale catalogue <folder>` lists one.");
        }
        Some(prefix) => {
            let needle = prefix.replace('/', "\\").to_ascii_lowercase();
            let under = |path: &String| path.strip_prefix("world\\").is_some_and(|rest| {
                rest.starts_with(&needle) || path.starts_with(&needle)
            });
            println!("\n  under {prefix}:");
            let mut shown = 0;
            for path in resolved.iter().filter(|p| under(p)) {
                println!("    {path}");
                shown += 1;
                if shown >= 200 {
                    println!("    …");
                    break;
                }
            }
            for path in roots.iter().filter(|p| under(p)) {
                println!("    {path}");
                shown += 1;
                if shown >= 400 {
                    println!("    …");
                    break;
                }
            }
            if shown == 0 {
                println!("    nothing");
            }
        }
    }
    Ok(())
}

/// The first path segment under `world\`, which is what the picker's top level
/// is.
fn first(path: &str) -> String {
    let rest = path.strip_prefix("world\\").unwrap_or(path);
    rest.split('\\').next().unwrap_or(rest).to_string()
}

/// Whether a `.wmo` path names one of a building's groups rather than its root.
///
/// The editor's own rule, restated here because this is the check on it:
/// `stormwind.wmo` is the building and `stormwind_042.wmo` is one of its rooms.
fn is_group(path: &str) -> bool {
    let stem = path.trim_end_matches(".wmo");
    let tail = &stem[stem.len().saturating_sub(3)..];
    stem.len() > 4
        && tail.len() == 3
        && tail.bytes().all(|b| b.is_ascii_digit())
        && stem.as_bytes()[stem.len() - 4] == b'_'
}
