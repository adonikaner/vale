//! `vale mcnk` — raw MCNK headers and height grids.

use crate::common::*;
use vale_assets::adt_path;
use vale_config::Config;

pub fn cmd_mcnk(cfg: &Config, map: &str, x: u32, y: u32) -> Result<(), String> {
    use vale_assets::world::chunk::{f32_at, u32_at, ChunkReader};

    let mut assets = open_assets(cfg)?;
    let raw = assets.read(&adt_path(map, x, y)).map_err(|e| e.to_string())?;

    println!("{}", adt_path(map, x, y));
    println!(
        "  expected tile world origin: x(north) {:.1}, y(west) {:.1}",
        (32.0 - y as f32) * 533.333_33,
        (32.0 - x as f32) * 533.333_33
    );

    for (n, c) in ChunkReader::new(&raw)
        .filter(|c| c.is(b"MCNK"))
        .enumerate()
        .take(3)
    {
        let d = c.data;
        println!("\n  MCNK #{n}  (payload {} bytes)", d.len());
        println!("    ix={} iy={} nLayers={}", u32_at(d, 0x04), u32_at(d, 0x08), u32_at(d, 0x0C));
        println!("    areaid={} holes={:#06x}", u32_at(d, 0x34), u32_at(d, 0x3C) & 0xFFFF);
        println!("    offsMCVT={} offsMCNR={} offsMCLY={}", u32_at(d, 0x14), u32_at(d, 0x18), u32_at(d, 0x1C));
        println!(
            "    floats @0x68={:.2}  @0x6C={:.2}  @0x70={:.2}",
            f32_at(d, 0x68),
            f32_at(d, 0x6C),
            f32_at(d, 0x70)
        );
        // First few MCVT samples, straight from the declared offset.
        let ofs = u32_at(d, 0x14) as usize;
        for base in [ofs, ofs.saturating_sub(8)] {
            if base + 12 <= d.len() {
                let magic = &d[base..base + 4];
                println!(
                    "    at offsMCVT{}: magic {:?} size {}",
                    if base == ofs { "" } else { "-8" },
                    String::from_utf8_lossy(magic),
                    u32_at(d, base + 4)
                );
            }
        }

        if n == 0 {
            // 145 samples at stride 17: outer row r at r*17, inner row r at
            // r*17+9. Print both grids so a bad interleave is visible by eye —
            // neighbouring values should differ by yards, not hundreds.
            let hbase = ofs.saturating_sub(8) + 8;
            let h = |i: usize| f32_at(d, hbase + i * 4);
            println!("    outer 9x9 (absolute):");
            for r in 0..9 {
                let row: Vec<String> =
                    (0..9).map(|c| format!("{:7.1}", f32_at(d, 0x70) + h(r * 17 + c))).collect();
                println!("      {}", row.join(""));
            }
            println!("    inner 8x8 (absolute):");
            for r in 0..8 {
                let row: Vec<String> = (0..8)
                    .map(|c| format!("{:7.1}", f32_at(d, 0x70) + h(r * 17 + 9 + c)))
                    .collect();
                println!("      {}", row.join(""));
            }
        }
    }
    Ok(())
}
