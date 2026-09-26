//! `vale tile` — one ADT: its chunks, its heights and what it places.

use crate::common::*;
use vale_assets::{world::adt::Adt, adt_path};
use vale_config::Config;

pub fn cmd_tile(cfg: &Config, map: &str, x: u32, y: u32) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let path = adt_path(map, x, y);
    let raw = assets.read(&path).map_err(|e| e.to_string())?;
    println!("{path}  ({} KB)", raw.len() / 1024);

    let adt = Adt::parse(&raw).map_err(|e| e.to_string())?;
    let (lo, hi) = adt.height_range();
    let mesh = adt.to_mesh();

    println!("  version        {}", adt.version);
    println!("  map chunks     {}", adt.chunks.len());
    println!("  height range   {lo:.1} .. {hi:.1}");
    println!("  textures       {}", adt.texture_names.len());
    println!("  models (M2)    {}", adt.model_names.len());
    println!("  WMOs           {}", adt.wmo_names.len());
    println!("  doodad placed  {}", adt.doodads.len());
    println!("  WMO placed     {}", adt.wmos.len());
    println!(
        "  mesh           {} verts, {} tris",
        mesh.positions.len(),
        mesh.triangle_count()
    );

    for name in adt.texture_names.iter().take(5) {
        println!("    tex: {name}");
    }
    if adt.texture_names.len() > 5 {
        println!("    ... and {} more", adt.texture_names.len() - 5);
    }
    Ok(())
}
