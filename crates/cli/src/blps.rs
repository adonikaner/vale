//! `vale blps` — the header shapes of every BLP the archives list, counted.
//!
//! The texture importer writes BLP files the 1.12.1 client has to read, and a
//! header shape the shipped textures never use is a shape nothing shows that
//! client reads. So the importer offers the shapes this census finds, and the
//! census is the check that they are still the shapes the archives carry.
//!
//! Each row is `compression, alphaDepth, alphaType, hasMips`, with how many
//! files have it and one path that does; then the same for the sizes. With a prefix it counts the files
//! under it alone.

use crate::common::*;
use vale_config::Config;
use std::collections::BTreeMap;

pub fn cmd_blps(cfg: &Config, prefix: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let prefix = prefix.unwrap_or("").to_ascii_lowercase();
    let listed: Vec<String> = assets
        .list_prefix(&prefix)
        .into_iter()
        .filter(|path| path.to_ascii_lowercase().ends_with(".blp"))
        .collect();
    println!("{} BLP files listed under {:?}", listed.len(), prefix);

    let mut shapes: BTreeMap<(u8, u8, u8, u8), (usize, String)> = BTreeMap::new();
    let mut unreadable = 0usize;
    let mut empty = 0usize;
    let mut not_blp2: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut not_powers = 0usize;
    let mut sizes: BTreeMap<(u32, u32), (usize, String)> = BTreeMap::new();
    for path in &listed {
        let Ok(bytes) = assets.read(path) else {
            unreadable += 1;
            continue;
        };
        if bytes.is_empty() {
            empty += 1;
            continue;
        }
        if bytes.len() < 0x14 || &bytes[0..4] != b"BLP2" {
            let magic = String::from_utf8_lossy(&bytes[..bytes.len().min(4)]).into_owned();
            let entry = not_blp2.entry(magic).or_insert((0, path.clone()));
            entry.0 += 1;
            continue;
        }
        let shape = (bytes[0x08], bytes[0x09], bytes[0x0A], bytes[0x0B]);
        let width = u32::from_le_bytes([bytes[0x0C], bytes[0x0D], bytes[0x0E], bytes[0x0F]]);
        let height = u32::from_le_bytes([bytes[0x10], bytes[0x11], bytes[0x12], bytes[0x13]]);
        if !width.is_power_of_two() || !height.is_power_of_two() {
            not_powers += 1;
        }
        let entry = shapes.entry(shape).or_insert((0, path.clone()));
        entry.0 += 1;
        let entry = sizes.entry((width, height)).or_insert((0, path.clone()));
        entry.0 += 1;
    }

    println!("  compression alphaDepth alphaType hasMips   files  e.g.");
    for ((compression, depth, kind, mips), (count, example)) in &shapes {
        let word = match (compression, depth, kind) {
            (1, ..) => "palettized",
            (2, 0 | 1, _) => "DXT1",
            (2, _, 7) => "DXT5",
            (2, ..) => "DXT3",
            (3, ..) => "raw BGRA",
            _ => "other",
        };
        println!(
            "  {compression:>11} {depth:>10} {kind:>9} {mips:>7} {count:>7}  {word:<10} {example}"
        );
    }
    println!("  width x height   files  e.g.");
    for ((width, height), (count, example)) in &sizes {
        println!("  {:>14} {count:>7}  {example}", format!("{width}x{height}"));
    }
    println!("  {unreadable} could not be read from the archives");
    println!("  {empty} read as zero bytes");
    for (magic, (count, example)) in &not_blp2 {
        println!("  {count} start with {magic:?}, not BLP2, e.g. {example}");
    }
    println!("  {not_powers} have a side that is not a power of two");
    Ok(())
}
