//! `vale item` — a garment's components, regions and geosets.

use crate::character::character_model_geosets;
use crate::common::*;
use vale_config::Config;

/// Check equipment against the archive: the textures a display id names, and
/// the geosets it asks the wearer to draw.
///
/// The same shape as `vale npc` and `vale char`, and for the same reason:
/// a piece of armour whose texture does not resolve is **not** an error. It
/// paints nothing, and the character wears the bare skin that was already there
/// — a naked forearm under a sleeve that is in the archive and was not looked
/// for. So the questions are:
///
/// 1. **Does every component texture resolve?** Over the whole table, with the
///    gender fallback chain — and counted per suffix, because a client that
///    tried only `_M` would silently lose every unisex piece and the total would
///    still look plausible.
/// 2. **Which geoset groups does the data actually use?** Printed per slot, so
///    the mapping in `item::Slot::geoset_groups` can be compared against what
///    the rows contain rather than trusted.
/// 3. **How much of the wardrobe is geometry rather than paint?** Shoulders,
///    helmets and weapons are attached *models*, not textures, and counting them
///    separately is what keeps a slot's absence from reading as a missing file.
///    `vale attach` is where they are checked against the archive.
pub fn cmd_item(cfg: &Config, display_id: Option<u32>) -> Result<(), String> {
    use vale_assets::world::blp;
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::tables::item::{Component, ItemDisplays};
    use std::collections::BTreeMap;

    let mut assets = open_assets(cfg)?;
    let raw = assets
        .read(&dbc_path("ItemDisplayInfo"))
        .map_err(|e| e.to_string())?;
    let table = ItemDisplays::parse(&raw).map_err(|e| e.to_string())?;
    println!("ItemDisplayInfo.dbc: {} display ids", table.len());

    // One id in full, for a worked example behind the numbers.
    if let Some(id) = display_id {
        let Some(look) = table.appearance(id, 0) else {
            return Err(format!("display id {id} is not in ItemDisplayInfo"));
        };
        println!("\n  display {id}");
        for (i, component) in Component::ALL.iter().enumerate() {
            let Some(name) = table.texture_name(id, i) else {
                continue;
            };
            let region = component.region();
            let path = vale_assets::tables::item::texture_candidates(*component, &name, 0)
                .into_iter()
                .find(|p| assets.exists(p));
            println!(
                "    {:<16} {:>3},{:<3} {:>3}x{:<3}  {}",
                format!("{component:?}"),
                region.x,
                region.y,
                region.width,
                region.height,
                path.unwrap_or_else(|| format!("!! {name} (no suffix resolves)")),
            );
        }
        println!("    geoset groups {:?}", look.geoset_groups);
        if look.has_geometry() {
            println!(
                "    attached models {:?} (a weapon is drawn only while it is out \
                 of its sheath)",
                look.models
            );
        }
        return Ok(());
    }

    // The whole table, both genders, against the archive.
    let mut resolved = 0u64;
    let mut missing = 0u64;
    let mut by_suffix: BTreeMap<char, u64> = BTreeMap::new();
    let mut undecodable = 0u64;
    let mut with_models = 0u64;
    let mut sizes: BTreeMap<String, u64> = BTreeMap::new();
    // Decoding all ~30k rows' textures is minutes of work for a number that does
    // not change; decode a sample and *existence*-check the rest, and say so.
    let mut decoded = 0u64;

    let ids = table.ids();
    for (n, id) in ids.iter().enumerate() {
        let Some(look) = table.appearance(*id, 0) else {
            continue;
        };
        if look.has_geometry() {
            with_models += 1;
        }
        for (i, component) in Component::ALL.iter().enumerate() {
            let Some(name) = table.texture_name(*id, i) else {
                continue;
            };
            let found = vale_assets::tables::item::texture_candidates(*component, &name, 0)
                .into_iter()
                .find(|p| assets.exists(p));
            match found {
                Some(path) => {
                    resolved += 1;
                    let suffix = path
                        .rsplit('_')
                        .next()
                        .and_then(|s| s.chars().next())
                        .unwrap_or('?');
                    *by_suffix.entry(suffix).or_insert(0) += 1;
                    // Every hundredth, decoded in full — these are ordinary
                    // BLPs, and the point is that the *paths* are right.
                    if n % 100 == 0 {
                        match assets.read(&path).ok().and_then(|b| blp::decode(&b).ok()) {
                            Some(image) => {
                                decoded += 1;
                                *sizes
                                    .entry(format!("{}x{}", image.width, image.height))
                                    .or_insert(0) += 1;
                                let region = component.region();
                                if image.width != region.width || image.height != region.height {
                                    println!(
                                        "    !! {path} is {}x{} but {component:?} is {}x{}",
                                        image.width, image.height, region.width, region.height
                                    );
                                }
                            }
                            None => {
                                println!("    !! will not decode: {path}");
                                undecodable += 1;
                            }
                        }
                    }
                }
                None => {
                    if missing < 6 {
                        println!("    !! no file for {name} ({component:?})");
                    }
                    missing += 1;
                }
            }
        }
    }

    println!(
        "  {resolved} component textures resolve, {missing} name a file no suffix finds"
    );
    for (suffix, n) in &by_suffix {
        println!("    {n:>6} x _{suffix}");
    }
    println!("  {decoded} decoded as a sample, {undecodable} will not decode");
    for (size, n) in &sizes {
        println!("    {n:>6} x {size}");
    }
    println!(
        "  {with_models} display ids carry an attached model (shoulders, helms, \
         weapons) — geometry rather than paint; `vale attach` checks those"
    );

    // **Which columns a garment fills, from the files rather than from a
    // reference.** A row's eight texture columns are not one piece of armour:
    // they are the whole *set*, copied into every row of it, and the slot picks
    // the ones that belong to the item being worn. What says which is which is
    // the name — `Cloth_B_03RedPhoenix_Sleeve_AU` is a sleeve filling arm-upper
    // — so this counts (piece, component) pairs over the whole table and prints
    // the matrix `Slot::components` was written from. A pair that appears here
    // and is missing there would be a garment painting nothing.
    let mut matrix: BTreeMap<String, BTreeMap<&'static str, u64>> = BTreeMap::new();
    for id in &ids {
        for (i, component) in Component::ALL.iter().enumerate() {
            let Some(name) = table.texture_name(*id, i) else {
                continue;
            };
            // `<family>_<piece>_<component>`: the last token names the column
            // and the one before it names the garment.
            let piece = name
                .rsplit('_')
                .nth(1)
                .unwrap_or("?")
                .to_ascii_lowercase();
            *matrix
                .entry(piece)
                .or_default()
                .entry(match component {
                    Component::ArmUpper => "AU",
                    Component::ArmLower => "AL",
                    Component::Hand => "HA",
                    Component::TorsoUpper => "TU",
                    Component::TorsoLower => "TL",
                    Component::LegUpper => "LU",
                    Component::LegLower => "LL",
                    Component::Foot => "FO",
                })
                .or_insert(0) += 1;
        }
    }
    println!("\n  which components each garment fills, by name:");
    for (piece, columns) in &matrix {
        let total: u64 = columns.values().sum();
        if total < 50 {
            continue; // a handful of one-off names, not a garment kind
        }
        let cells: Vec<String> = columns.iter().map(|(c, n)| format!("{c} {n}")).collect();
        println!("    {piece:<10} {}", cells.join("  "));
    }

    // **Where `geosetGroup` starts, argued from the columns rather than from a
    // reference.** Both readings of this table add up to its 23 fields — one
    // icon at 5 with the triple at 6..8, or two icons at 5..6 with the triple at
    // 7..9 — so the arithmetic cannot settle it and only the data can. Reading
    // it one high loses every item's first variant and reads `flags` as its
    // third, which draws a character in plate boots with bare feet and warns
    // about nothing.
    //
    // Two questions, printed side by side. Is column 6 a *name*? And which
    // column moves when the garment changes?
    let mut distinct: [std::collections::HashSet<u32>; 8] = Default::default();
    let mut largest = [0u32; 8];
    let mut nonzero = [0u64; 8];
    let mut by_piece: BTreeMap<String, [BTreeMap<u32, u64>; 3]> = BTreeMap::new();
    for id in &ids {
        for (i, field) in (4..12).enumerate() {
            if let Some((number, _)) = table.raw_field(*id, field) {
                if number != 0 {
                    nonzero[i] += 1;
                }
                distinct[i].insert(number);
                largest[i] = largest[i].max(number);
            }
        }
        // Which garment this row is, by the same rule the component matrix uses:
        // the second-to-last token of a component texture names the piece.
        let piece = (0..8)
            .find_map(|i| table.texture_name(*id, i))
            .and_then(|n| n.rsplit('_').nth(1).map(str::to_ascii_lowercase));
        let (Some(piece), Some(look)) = (piece, table.appearance(*id, 0)) else {
            continue;
        };
        let columns = by_piece.entry(piece).or_default();
        for (column, value) in columns.iter_mut().zip(look.geoset_groups) {
            *column.entry(value).or_insert(0) += 1;
        }
    }
    // A string column holds thousands of distinct six-figure offsets; a variant
    // column holds a handful of single digits. That is the difference between
    // `inventoryIcon` and `geosetGroup[0]`, and it is not close.
    println!("\n  fields 4..11, by the shape of what they hold:");
    for (i, field) in (4..12).enumerate() {
        println!(
            "    [{field:>2}]  {:>6} non-zero, {:>6} distinct, largest {}",
            nonzero[i],
            distinct[i].len(),
            largest[i],
        );
    }
    println!("\n  geosetGroup columns, by garment (0 = the group's default):");
    for (piece, columns) in &by_piece {
        let total: u64 = columns[0].values().sum();
        if total < 50 {
            continue;
        }
        let cells: Vec<String> = columns
            .iter()
            .map(|values| {
                let varied: Vec<String> = values
                    .iter()
                    .filter(|(v, _)| **v != 0)
                    .map(|(v, n)| format!("{v}x{n}"))
                    .collect();
                if varied.is_empty() {
                    "-".to_string()
                } else {
                    varied.join(",")
                }
            })
            .collect();
        println!("    {piece:<10} {}", cells.join("   |   "));
    }

    // **Which group each column fills, picked by the models rather than by a
    // reference.** `Slot::geoset_groups` is transcribed, and the table it comes
    // from (`CharComponentTextureLayouts`) is a later expansion's — so the
    // honest check is the one the facial-hair column order got: for every
    // candidate group, count how many of the wardrobe's rows would name a geoset
    // that **no character model carries**, and see which group leaves none.
    //
    // A geoset the model lacks does not fail. It draws nothing, which is a bare
    // foot under a plate boot — the exact symptom this whole section exists for.
    let available: std::collections::BTreeSet<u16> = {
        let mut assets = open_assets(cfg)?;
        let tables = open_display_tables(&mut assets)?;
        character_model_geosets(&mut assets, &tables)
            .values()
            .flatten()
            .copied()
            .collect()
    };
    println!("\n  which group can hold each column, by what the models carry:");
    for (piece, columns) in &by_piece {
        let total: u64 = columns[0].values().sum();
        if total < 50 {
            continue;
        }
        for (column, values) in columns.iter().enumerate() {
            let asked: u64 = values.iter().filter(|(v, _)| **v != 0).map(|(_, n)| n).sum();
            if asked == 0 {
                continue;
            }
            // The cape counts from x02; every other group from x01.
            let fits: Vec<String> = (4..=15u16)
                .filter_map(|group| {
                    let base = if group == 15 { 2 } else { 1 };
                    let missed: u64 = values
                        .iter()
                        .filter(|(v, _)| **v != 0)
                        .filter(|(v, _)| !available.contains(&(group * 100 + **v as u16 + base)))
                        .map(|(_, n)| n)
                        .sum();
                    (missed == 0).then(|| group.to_string())
                })
                .collect();
            println!(
                "    {piece:<10} column {column}  {asked:>5} rows ask for it   fits group(s) {}",
                if fits.is_empty() { "none".to_string() } else { fits.join(", ") }
            );
        }
    }
    Ok(())
}
