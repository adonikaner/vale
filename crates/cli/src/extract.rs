//! `vale extract` — any file in the archives, written out or hexdumped.

use crate::common::*;
use vale_config::Config;

/// Pull one file out of the archive chain, as bytes.
///
/// The one command here that checks nothing. It is for a question whose
/// answer is a file the game ships and no parser in this repository reads.
/// `shaders\vertex\Model2.bls` is the worked example: the ARB program the
/// 1.12 client lights every M2 with, which settles what the exterior lighting
/// curve is rather than what a renderer would guess. This gets the bytes.
///
/// With no output path it prints a hexdump of the head instead of writing, which
/// is usually enough to tell a text blob from a binary one. For a file that
/// reads as empty, or that an archive deletes, it prints each archive's own
/// entry for the path.
pub fn cmd_extract(cfg: &Config, path: &str, out: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let entries = |assets: &mut vale_assets::archive::Assets| {
        for (name, size, stored, flags) in assets.entries(path) {
            let deleted = match flags & vale_assets::archive::DELETE_MARKER {
                0 => "",
                _ => ", deleted",
            };
            println!("  {name}: {size} bytes, {stored} stored, flags 0x{flags:08X}{deleted}");
        }
    };
    let buf = match assets.read(path) {
        Ok(buf) => buf,
        Err(e) if assets.entries(path).is_empty() => return Err(e.to_string()),
        Err(_) => {
            println!("{path}: not found: the first archive with an entry deletes it");
            entries(&mut assets);
            return Ok(());
        }
    };
    println!("{path}: {} bytes", buf.len());
    if buf.is_empty() {
        entries(&mut assets);
    }
    match out {
        Some(dest) => {
            std::fs::write(dest, &buf).map_err(|e| e.to_string())?;
            println!("  written to {dest}");
        }
        None => {
            for (i, row) in buf.chunks(16).take(16).enumerate() {
                let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
                let ascii: String = row
                    .iter()
                    .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' })
                    .collect();
                println!("  {:04x}  {:<47}  {ascii}", i * 16, hex.join(" "));
            }
        }
    }
    Ok(())
}
