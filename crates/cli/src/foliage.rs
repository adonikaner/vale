//! `vale foliage`: the ground effects. Reports the table, the models, and what
//! one tile plants.
//!
//! The command has two forms, which check different parts.
//!
//! `vale foliage` checks the table against the archives: how many of
//! `GroundEffectTexture`'s 12,742 rows plant anything, how many distinct models
//! they name between them, and how many of those models are files that exist.
//! 32 of the 443 names are in no archive in the chain. A client that drew 411
//! of 443 would look the same as one that drew all of them, so only this count
//! shows the missing ones.
//!
//! `vale foliage <Map> <x> <y>` checks one tile's ground: how its 16,384
//! detail cells divide into suppressed, bare and planted, how many tufts the
//! tile grows at the default `frillDensity`, and which models over which
//! textures. It also runs the two placement checks that cannot be made from
//! the table alone:
//!
//! * every tuft stands inside the chunk that planted it. This checks the cell
//!   axes, which run along decreasing world x and y; with them reversed a
//!   chunk's grass grows on its neighbour.
//! * every tuft stands on the drawn ground, sampled again through
//!   `Adt::height_at`, the height a character walks on, so a difference
//!   between the two interpolations is reported.
//!
//! It also reports the distinct textures per chunk, which is how many draw
//! calls a chunk's merged foliage comes to: 1 on `Azeroth_34_51`, up to 2 on
//! `Azeroth_32_48` and up to 3 on `Kalimdor_39_30`.

use crate::common::*;
use vale_assets::adt_path;
use vale_assets::tables::dbc::dbc_path;
use vale_assets::tables::foliage::GroundEffects;
use vale_assets::world::adt::{Adt, CHUNK_SIZE};
use vale_assets::world::foliage::{ChunkFoliage, CELLS_PER_CHUNK, DEFAULT_FRILL_DENSITY};
use vale_assets::world::m2::M2;
use vale_config::Config;
use std::collections::{BTreeMap, BTreeSet};

/// The whole table, against the archives.
pub fn cmd_foliage(cfg: &Config) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let effects = GroundEffects::load(|table| assets.read(&dbc_path(table)).ok());
    if effects.is_empty() {
        return Err("no GroundEffectTexture/GroundEffectDoodad in the chain".to_string());
    }

    println!(
        "ground effects: {} texture rows plant something, naming {} models",
        effects.effect_count(),
        effects.models().len()
    );

    let mut present = 0usize;
    let mut missing: Vec<&str> = Vec::new();
    let mut textures: BTreeMap<String, usize> = BTreeMap::new();
    let mut batches = BTreeMap::new();
    for path in effects.models() {
        let Ok(raw) = assets.read(path) else {
            missing.push(path);
            continue;
        };
        present += 1;
        let Ok(model) = M2::parse(&raw) else {
            println!("  {path}: will not parse");
            continue;
        };
        *batches.entry(model.batches.len()).or_insert(0usize) += 1;
        for texture in &model.textures {
            if !texture.file_name.is_empty() {
                *textures.entry(texture.file_name.clone()).or_insert(0) += 1;
            }
        }
    }

    println!(
        "  {present}/{} are files in the archive; {} are named by the table and in no archive",
        effects.models().len(),
        missing.len()
    );
    for path in missing.iter().take(6) {
        println!("    absent: {path}");
    }
    if missing.len() > 6 {
        println!("    …and {} more", missing.len() - 6);
    }

    // A detail doodad that is one batch can be concatenated into its chunk's
    // mesh without splitting it. Every model being one batch is why a chunk's
    // foliage is one draw call per texture.
    let mut line: Vec<String> = batches
        .iter()
        .map(|(n, count)| format!("{count} of {n}"))
        .collect();
    println!("  batches per model: {}", line.join(", "));
    line = textures
        .iter()
        .rev()
        .take(4)
        .map(|(name, count)| format!("{} x{count}", short(name)))
        .collect();
    println!(
        "  {} distinct textures over them; the busiest: {}",
        textures.len(),
        line.join(", ")
    );

    Ok(())
}

/// One tile's own ground: the cell census, the population, and the two
/// placement checks.
pub fn cmd_foliage_tile(cfg: &Config, map: &str, x: u32, y: u32) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let effects = GroundEffects::load(|table| assets.read(&dbc_path(table)).ok());
    if effects.is_empty() {
        return Err("no GroundEffectTexture/GroundEffectDoodad in the chain".to_string());
    }
    let raw = assets.read(&adt_path(map, x, y)).map_err(|e| e.to_string())?;
    let adt = Adt::parse(&raw).map_err(|e| e.to_string())?;

    println!("{}", adt_path(map, x, y));

    let (mut suppressed, mut bare, mut planted) = (0usize, 0usize, 0usize);
    for chunk in &adt.chunks {
        for row in 0..8 {
            for col in 0..8 {
                match chunk.detail_layer_at(row, col) {
                    None => suppressed += 1,
                    Some(layer) if effects.effect(layer.effect_id).is_some() => planted += 1,
                    Some(_) => bare += 1,
                }
            }
        }
    }
    let cells = adt.chunks.len() * CELLS_PER_CHUNK;
    println!(
        "  {cells} detail cells: {suppressed} suppressed by the header, {bare} over a texture \
         that plants nothing, {planted} planted"
    );

    // The plans, then the tufts, in the order the renderer makes them: the
    // plan is kept from the parse and the tufts are made from it.
    let mut plans = 0usize;
    let mut tufts = Vec::new();
    let mut per_chunk_textures: BTreeMap<usize, usize> = BTreeMap::new();
    let mut models: BTreeMap<u16, usize> = BTreeMap::new();
    let mut off_chunk = 0usize;
    let mut off_ground = 0usize;
    let mut worst = 0.0f32;

    // The model -> texture join, read once: a chunk's draw-call count is the
    // number of distinct textures over the models it plants, not of models.
    let mut texture_of: BTreeMap<u16, String> = BTreeMap::new();
    for (index, path) in effects.models().iter().enumerate() {
        let texture = assets
            .read(path)
            .ok()
            .and_then(|raw| M2::parse(&raw).ok())
            .and_then(|m| m.textures.first().map(|t| t.file_name.clone()))
            .unwrap_or_default();
        texture_of.insert(index as u16, texture);
    }

    for chunk in &adt.chunks {
        let Some(plan) = ChunkFoliage::plan(chunk, &effects) else {
            continue;
        };
        plans += 1;
        let before = tufts.len();
        // At the client's default `frillDensity`; a session reads `Config.wtf`.
        plan.grow(&effects, DEFAULT_FRILL_DENSITY, &mut tufts);

        let mut here: BTreeSet<&str> = BTreeSet::new();
        for tuft in &tufts[before..] {
            *models.entry(tuft.model).or_insert(0) += 1;
            here.insert(texture_of.get(&tuft.model).map_or("", String::as_str));

            // Inside the chunk that planted it.
            let dx = chunk.position[0] - tuft.position[0];
            let dy = chunk.position[1] - tuft.position[1];
            if !(0.0..CHUNK_SIZE).contains(&dx) || !(0.0..CHUNK_SIZE).contains(&dy) {
                off_chunk += 1;
            }
            // …and on the ground a character walks on, asked of the whole tile
            // rather than of the chunk's own copy of the height field.
            match adt.height_at(tuft.position[0], tuft.position[1]) {
                Some(z) => worst = worst.max((z - tuft.position[2]).abs()),
                None => off_ground += 1,
            }
        }
        *per_chunk_textures.entry(here.len()).or_insert(0) += 1;
    }

    println!(
        "  {plans}/{} chunks plant anything, {} tufts over the tile at frillDensity {DEFAULT_FRILL_DENSITY} ({:.0} per planted chunk)",
        adt.chunks.len(),
        tufts.len(),
        tufts.len() as f32 / plans.max(1) as f32
    );
    println!(
        "  {} distinct models: {}",
        models.len(),
        models
            .iter()
            .map(|(index, count)| format!("{} x{count}", short(effects.model(*index))))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  distinct textures per planted chunk: {}  <- draw calls per chunk once merged",
        per_chunk_textures
            .iter()
            .map(|(n, count)| format!("{count} chunks of {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  placement: {off_chunk} outside their own chunk, {off_ground} off the drawn ground, \
         worst height disagreement {worst:.4}y"
    );
    if off_chunk > 0 || off_ground > 0 {
        return Err("tufts landed off their chunk or off the ground".to_string());
    }
    Ok(())
}

/// The last path element, which is all that fits on a line.
fn short(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}
