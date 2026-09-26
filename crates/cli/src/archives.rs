//! `vale archives` — open the MPQ chain and report what mounted.

use crate::common::*;
use vale_config::Config;

pub fn cmd_archives(cfg: &Config) -> Result<(), String> {
    let assets = open_assets(cfg)?;
    println!("archives in load order (first hit wins):");
    for (i, name) in assets.archive_names().iter().enumerate() {
        println!("  [{i:2}] {name}");
    }
    Ok(())
}
