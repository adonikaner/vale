//! `vale extract` — any file in the archives, written out or hexdumped.

use crate::common::*;
use vale_config::Config;

/// Pull one file out of the archive chain, as bytes.
///
/// **The one command here that checks nothing**, and it earns its place for the
/// case that keeps coming up: a question whose answer is a file the game
/// ships and no parser in this repo reads. `shaders\vertex\Model2.bls` is the
/// worked example — the ARB program the 1.12 client lights every M2 with, which
/// settles what the exterior lighting curve actually *is* rather than what a
/// renderer would guess. This gets the bytes.
///
/// With no output path it prints a hexdump of the head instead of writing, which
/// is usually enough to tell a text blob from a binary one.
pub fn cmd_extract(cfg: &Config, path: &str, out: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let buf = assets.read(path).map_err(|e| e.to_string())?;
    println!("{path}: {} bytes", buf.len());
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
