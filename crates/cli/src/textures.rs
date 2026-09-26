//! `vale textures` — a tile's layers, alpha maps and BLP decode.

use crate::common::*;
use vale_assets::{world::adt::Adt, adt_path};
use vale_config::Config;

/// Dump raw MCNK header fields straight from the file.
///
/// Diagnostic tool: field *order* inside the MCNK header is the easiest thing
/// to get wrong (vmangos declares the position as `zpos, xpos, ypos`, not
/// x/y/z), and a wrong guess shows up as plausible-looking but subtly broken
/// terrain. Printing the bytes settles it.
/// Dump one tile's texture layers, alpha maps and BLP formats.
///
/// The point is to check the two things that cannot be unit-tested against
/// invented bytes: that the alpha-map stride is being measured correctly on
/// real 1.12 files, and that every texture the tile references actually decodes.
pub fn cmd_textures(cfg: &Config, map: &str, x: u32, y: u32) -> Result<(), String> {
    use vale_assets::world::adt::{layer_flags, ALPHA_LEN, CHUNK_SIZE};
    use vale_assets::world::blp;

    let mut assets = open_assets(cfg)?;
    let raw = assets.read(&adt_path(map, x, y)).map_err(|e| e.to_string())?;
    let adt = Adt::parse(&raw).map_err(|e| e.to_string())?;

    println!("{}", adt_path(map, x, y));
    println!("  {} textures referenced by MTEX", adt.texture_names.len());

    let layered = adt.chunks.iter().filter(|c| c.layers.len() > 1).count();
    println!(
        "  {} of {} chunks have more than one layer",
        layered,
        adt.chunks.len()
    );

    // What the renderer's draw-call count actually depends on. A chunk's GL state
    // is its ground-texture set and nothing else now that the blend maps are one
    // atlas per tile, so chunks sharing a set share a draw — and this is the
    // number that says by how much. Printed rather than assumed: a tileset where
    // every chunk picked a different combination would group to nothing.
    let mesh = adt.to_mesh();
    let drawn: u32 = mesh.draws.iter().map(|d| d.chunks).sum();
    println!(
        "  {} chunks with geometry -> {} draw groups ({:.1}x fewer calls)",
        drawn,
        mesh.draws.len(),
        drawn as f32 / mesh.draws.len().max(1) as f32,
    );
    let mut by_size: Vec<_> = mesh.draws.iter().map(|d| d.chunks).collect();
    by_size.sort_unstable_by(|a, b| b.cmp(a));
    println!(
        "    biggest groups: {}",
        by_size
            .iter()
            .take(8)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Is the atlas lossless? Every chunk's blend map now reaches the shader
    // through two indirections that did not exist before — a cell in a
    // 1024x1024 texture, and a UV computed by `atlas_uv` — and getting either
    // wrong blends a chunk with some *other* chunk's map. That renders as
    // slightly wrong ground, not as an error, which is exactly the kind of bug
    // this command exists to catch. So: read every chunk's map back out of the
    // atlas the way the shader will, and compare with the map itself.
    {
        use vale_assets::world::adt::{atlas_uv, ATLAS_SIDE};
        let atlas = adt.alpha_atlas();
        let mut checked = 0u32;
        let mut wrong = 0u32;
        for (i, chunk) in adt.chunks.iter().enumerate() {
            for channel in 0..3usize {
                let Some(map) = chunk.alphas.get(channel + 1) else {
                    continue;
                };
                if map.len() != ALPHA_LEN {
                    continue;
                }
                // Texel centres, so the comparison is against the value the
                // shader's bilinear sample resolves to exactly.
                for ty in 0..64usize {
                    for tx in 0..64usize {
                        let u = (tx as f32 + 0.5) / 64.0;
                        let v = (ty as f32 + 0.5) / 64.0;
                        let [au, av] = atlas_uv(i, u, v);
                        let px = (au * ATLAS_SIDE as f32).floor() as usize;
                        let py = (av * ATLAS_SIDE as f32).floor() as usize;
                        let got = atlas[(py * ATLAS_SIDE + px) * 4 + channel];
                        checked += 1;
                        if got != map[ty * 64 + tx] {
                            wrong += 1;
                        }
                    }
                }
            }
        }
        println!(
            "  alpha atlas round trip: {checked} texels read back through atlas_uv, {wrong} wrong"
        );
    }

    // **The baked shadow, and whether its bits came out in the right order.**
    //
    // `MCSH` is 1.12's only shadow — nothing is cast at runtime — and it is 512
    // packed bits per chunk with nothing in the file saying which end of a byte
    // texel 0 is at. The wrong answer is not a failure: it mirrors every group of
    // eight texels inside its own byte, which is a shadow with a comb through it
    // at a period of 8 of 64 texels **along one axis only**.
    //
    // So the check is anisotropy. A baked shadow is a blob with an outline, and
    // an outline has no reason to prefer an axis, so the density of horizontal
    // edges and of vertical edges should be within a few per cent of each other.
    // Unpacked backwards the horizontal density multiplies while the vertical one
    // does not move at all — the vertical neighbours are in different bytes and
    // are unaffected by the order within one. Both are printed, and the second
    // pair is the same map deliberately unpacked the other way round, so the run
    // says which of the two orders this file is in rather than asserting it.
    {
        let flagged = adt
            .chunks
            .iter()
            .filter(|c| c.flags & vale_assets::world::adt::mcnk_flags::HAS_MCSH != 0)
            .count();
        let carried = adt.chunks.iter().filter(|c| !c.shadow.is_empty()).count();
        let shadowed: u64 = adt
            .chunks
            .iter()
            .flat_map(|c| c.shadow.iter())
            .filter(|&&v| v != 0)
            .count() as u64;
        let texels = (carried * ALPHA_LEN) as u64;
        println!(
            "  MCSH: {flagged} chunks declare a shadow, {carried} carry one, \
             {:.1}% of their {texels} texels are in shadow",
            100.0 * shadowed as f64 / texels.max(1) as f64,
        );

        // Edge density along each axis, over every chunk that has a map. The
        // last row and column are skipped: they are `fix_alpha_edge`'s copies of
        // their neighbours and so carry no edge by construction.
        let density = |maps: &[Vec<u8>]| -> (f64, f64) {
            let (mut h, mut v, mut n) = (0u64, 0u64, 0u64);
            for map in maps {
                for y in 0..62usize {
                    for x in 0..62usize {
                        let at = |x: usize, y: usize| map[y * 64 + x] != 0;
                        h += u64::from(at(x, y) != at(x + 1, y));
                        v += u64::from(at(x, y) != at(x, y + 1));
                        n += 1;
                    }
                }
            }
            let n = n.max(1) as f64;
            (h as f64 / n, v as f64 / n)
        };
        let shipped: Vec<Vec<u8>> = adt
            .chunks
            .iter()
            .filter(|c| c.shadow.len() == ALPHA_LEN)
            .map(|c| c.shadow.clone())
            .collect();
        // The same maps with each byte's eight texels reversed — which is
        // exactly what the other bit order produces, and needs no second decode.
        let mirrored: Vec<Vec<u8>> = shipped
            .iter()
            .map(|map| {
                let mut out = map.clone();
                for byte in out.chunks_exact_mut(8) {
                    byte.reverse();
                }
                out
            })
            .collect();
        if !shipped.is_empty() {
            let (h, v) = density(&shipped);
            let (mh, mv) = density(&mirrored);
            println!(
                "    edge density  as read {h:.4} across / {v:.4} down ({:.2}x), \
                 bytes reversed {mh:.4} / {mv:.4} ({:.2}x)",
                h / v.max(1e-9),
                mh / mv.max(1e-9),
            );
        }
    }

    // Does the blend run continuously across a chunk boundary?
    //
    // This is what says the "fix alpha map" duplication in `decode_alpha_maps`
    // is right rather than merely conventional. A 64x64 alpha map holds 63x63 of
    // blend — the pre-Cataclysm client puts texel 62's centre on the chunk's far
    // edge and never reads row or column 63 — so the check is against something
    // this client did not produce: the *neighbouring* chunk's first column, which
    // the ground has to be continuous with. Layers are matched by texture id, not
    // by index, because two chunks rarely list their textures in the same order.
    //
    // Column 62 wins on every tileset tried, and not narrowly. Measured on the
    // raw maps, before the fix was written:
    //
    //     Westfall (30, 50)   col62 27.55   col63 51.98
    //     Duskwood (34, 51)   col62 12.90   col63 42.06
    //     Elwynn   (32, 48)   col62 15.36   col63 53.64
    //
    // Drawing column 63 instead paints a seam on the far edge of all 256 chunks
    // in a tile, which reads as a checkerboard rather than as an error. What is
    // printed below is the shipped path's residual — the first column of those
    // pairs. If the fix is ever undone it jumps back to the second, so this is a
    // regression detector and not just a record.
    {
        let near = |a: f32, b: f32| (a - b).abs() < 0.5;
        let mut edge = 0.0f64;
        let mut count = 0u64;
        for a in &adt.chunks {
            // The chunk one step in the +u direction: u runs along -y, which is
            // the direction the mesh's column index runs.
            let Some(b) = adt.chunks.iter().find(|b| {
                near(b.position[0], a.position[0])
                    && near(b.position[1], a.position[1] - CHUNK_SIZE)
            }) else {
                continue;
            };
            for (i, la) in a.layers.iter().enumerate().skip(1) {
                let Some(j) = b.layers.iter().position(|lb| lb.texture_id == la.texture_id) else {
                    continue;
                };
                let (Some(ma), Some(mb)) = (a.alphas.get(i), b.alphas.get(j)) else {
                    continue;
                };
                if j == 0 || ma.len() != ALPHA_LEN || mb.len() != ALPHA_LEN {
                    continue;
                }
                for t in 0..64usize {
                    // The chunk's far edge, which after the fix is columns 62
                    // and 63 alike, against the value the neighbour puts on the
                    // same ground.
                    edge += (ma[t * 64 + 63] as f64 - mb[t * 64] as f64).abs();
                    count += 1;
                }
            }
        }
        println!(
            "  chunk seam continuity: {:.2} of 255 over {count} texels, against the \
             neighbouring chunk's own first column",
            edge / count.max(1) as f64,
        );
    }

    // A few chunks in full, so the flags and offsets can be eyeballed.
    for chunk in adt.chunks.iter().filter(|c| c.layers.len() > 2).take(3) {
        println!("\n  chunk ({}, {})", chunk.index_x, chunk.index_y);
        for (i, layer) in chunk.layers.iter().enumerate() {
            let alpha = chunk.alphas.get(i);
            let (min, max, mean) = match alpha {
                Some(a) if !a.is_empty() => {
                    let sum: u32 = a.iter().map(|&v| v as u32).sum();
                    (
                        *a.iter().min().expect("non-empty"),
                        *a.iter().max().expect("non-empty"),
                        sum / a.len() as u32,
                    )
                }
                _ => (0, 0, 0),
            };
            println!(
                "    layer {i}: tex {:<3} flags {:#06x}{}{}  mcalOfs {:<6} alpha min/max/mean {min}/{max}/{mean}",
                layer.texture_id,
                layer.flags,
                if layer.flags & layer_flags::USE_ALPHA_MAP != 0 { " alpha" } else { "" },
                if layer.flags & layer_flags::ALPHA_COMPRESSED != 0 { " rle" } else { "" },
                layer.alpha_offset,
            );
            println!("             {}", adt.texture_names.get(layer.texture_id as usize).map(String::as_str).unwrap_or("<missing>"));
            if alpha.map(Vec::len) != Some(ALPHA_LEN) {
                println!("             !! alpha map is not {ALPHA_LEN} bytes");
            }
        }
    }

    // Decode every texture the tile uses. A tile that renders needs all of
    // them, so anything that fails here is a hole in the terrain.
    println!("\n  decoding {} textures:", adt.texture_names.len());
    let mut failures = 0;
    let mut formats: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for name in &adt.texture_names {
        match assets.read(name).map_err(|e| e.to_string()).and_then(|b| {
            let format = describe_blp(&b);
            blp::decode_mipped(&b).map(|t| (t, format)).map_err(|e| e.to_string())
        }) {
            Ok((tex, format)) => {
                *formats.entry(format).or_insert(0) += 1;
                if tex.rgba.len() != (tex.width * tex.height * 4) as usize {
                    println!("    !! {name}: {}x{} but {} bytes", tex.width, tex.height, tex.rgba.len());
                    failures += 1;
                }
                // **The mip chain, counted the way the uploader counts it.** A
                // ground texture repeats eight times across a 33-yard chunk, so
                // it is the most heavily minified thing on screen and the first
                // to alias — and `models::model_image` cuts the chain at the
                // first level whose byte count is not exactly its own size,
                // silently. A chain that stops early is therefore invisible
                // until the ground shimmers, which is not a difference anyone
                // can see in a still. So it is measured here instead: a WxH
                // texture wants `log2(max(w, h)) + 1` levels, down to 1x1.
                let want = 32 - tex.width.max(tex.height).leading_zeros();
                let usable = tex
                    .levels()
                    .enumerate()
                    .take_while(|(level, data)| {
                        let (w, h) = ((tex.width >> level).max(1), (tex.height >> level).max(1));
                        data.len() == (w * h * 4) as usize
                    })
                    .count() as u32;
                if usable < want {
                    println!(
                        "    !! {name}: {}x{} has {usable} usable mip levels, wants {want}",
                        tex.width, tex.height
                    );
                    failures += 1;
                }
            }
            Err(e) => {
                println!("    !! {name}: {e}");
                failures += 1;
            }
        }
    }
    for (format, n) in &formats {
        println!("    {n:>4} x {format}");
    }
    println!(
        "  {} decoded, {failures} failed",
        adt.texture_names.len() - failures
    );

    // **The gloss masks the sun's sheen rides on**, which are a *different
    // file*: `X_s.blp` beside `X.blp`, the same picture with an alpha plane.
    // The renderer loads the `_s` variant wherever there is one and forces the
    // rest matte, so what this reports is how much of a tile can shine at all
    // — and the check that says the naming rule is right is simply that the
    // archives answer to it.
    //
    // **The mean is the number to watch.** The mask multiplies a near-white
    // band-9 highlight, so a mask that read 1 everywhere would bleach the
    // ground; the reference's runs about a tenth, which is what makes the
    // sheen a highlight instead of a wash.
    println!("\n  specular masks (the `_s` variant of each, if any):");
    let (mut masked, mut total_mean) = (0usize, 0.0f64);
    for name in &adt.texture_names {
        let specular = vale_assets::world::adt::specular_texture(name);
        let Ok(raw) = assets.read(&specular) else {
            println!("    --   no _s variant: {name}");
            continue;
        };
        match blp::decode(&raw) {
            Ok(tex) => {
                let alpha: Vec<u8> = tex.rgba.chunks_exact(4).map(|t| t[3]).collect();
                let mean = f64::from(alpha.iter().map(|&a| u32::from(a)).sum::<u32>())
                    / alpha.len().max(1) as f64;
                let lit = alpha.iter().filter(|&&a| a >= 128).count() * 100 / alpha.len().max(1);
                masked += 1;
                total_mean += mean;
                println!(
                    "    {:>4.0}  {lit:>3}% over half  {}x{}  {}",
                    mean, tex.width, tex.height, specular
                );
            }
            Err(e) => println!("    !! {specular}: {e}"),
        }
    }
    println!(
        "  {masked} of {} tilesets carry a mask, mean alpha {:.0}/255",
        adt.texture_names.len(),
        if masked == 0 { 0.0 } else { total_mean / masked as f64 }
    );
    Ok(())
}

/// **What `MCLY`'s flags actually say**, censused over every tile of a map.
///
/// The record carries a 32-bit flag word and this project reads two bits of it:
/// `USE_ALPHA_MAP` (0x100) and `ALPHA_COMPRESSED` (0x200). The bits under those
/// are the layer's own **texture animation** — a direction, a speed and a
/// switch — which is what makes lava crawl and a waterfall run, and which
/// nothing in this project read or wrote until this census said what was there.
///
/// It is a census rather than a copied table because the layout has to be
/// pinned from the data. Two properties do that, and both are printed:
///
/// * **`0x100` is exactly the layers that have an alpha map.** Layer 0 never
///   carries one and every layer above it does, so a bit that agrees with the
///   layer index on every record in a map is a bit whose meaning is known — and
///   it fixes where the *low* byte ends.
/// * **the animation bits are `0x01`..`0x40` and nothing between them is
///   unused.** A direction of three bits runs 0..7 and a speed of three bits
///   runs 0..7; a layout off by one would show a direction that never exceeds 3
///   or a speed that is always even.
pub fn cmd_layer_flags(cfg: &Config, map: Option<&str>) -> Result<(), String> {
    use vale_assets::world::adt::layer_flags;
    use vale_assets::world::wdt::Wdt;
    use std::collections::BTreeMap;

    let mut assets = open_assets(cfg)?;
    let map = map.unwrap_or("Azeroth");
    let wdt = assets
        .read(&vale_assets::wdt_path(map))
        .map_err(|e| e.to_string())
        .and_then(|raw| Wdt::parse(&raw).map_err(|e| e.to_string()))?;
    let tiles = wdt.existing_tiles();
    println!("{map}: {} tiles", tiles.len());

    let mut layers = 0u64;
    let mut base_layers = 0u64;
    let mut bits: BTreeMap<u32, u64> = BTreeMap::new();
    // Does 0x100 agree with "this is not layer 0"?
    let (mut agrees, mut disagrees) = (0u64, 0u64);
    let mut animated: BTreeMap<(u32, u32), u64> = BTreeMap::new();
    let mut by_texture: BTreeMap<String, u64> = BTreeMap::new();
    let mut where_animated: Vec<(u32, u32, u64)> = Vec::new();
    let mut read = 0usize;

    for (x, y) in &tiles {
        let Ok(raw) = assets.read(&adt_path(map, *x, *y)) else {
            continue;
        };
        let Ok(adt) = Adt::parse(&raw) else { continue };
        read += 1;
        let here = animated.values().sum::<u64>();
        for chunk in &adt.chunks {
            for (index, layer) in chunk.layers.iter().enumerate() {
                layers += 1;
                if index == 0 {
                    base_layers += 1;
                }
                for bit in 0..32u32 {
                    if layer.flags & (1 << bit) != 0 {
                        *bits.entry(1 << bit).or_default() += 1;
                    }
                }
                let has_map = layer.flags & layer_flags::USE_ALPHA_MAP != 0;
                match has_map == (index > 0) {
                    true => agrees += 1,
                    false => disagrees += 1,
                }
                if layer.flags & layer_flags::ANIMATION_ENABLED != 0 {
                    let turn = layer.flags & layer_flags::ANIMATION_ROTATION;
                    let rate = (layer.flags & layer_flags::ANIMATION_SPEED)
                        >> layer_flags::ANIMATION_SPEED_SHIFT;
                    *animated.entry((turn, rate)).or_default() += 1;
                    if let Some(name) = adt.texture_names.get(layer.texture_id as usize) {
                        *by_texture.entry(name.clone()).or_default() += 1;
                    }
                }
            }
        }
        let gained = animated.values().sum::<u64>() - here;
        if gained > 0 {
            where_animated.push((*x, *y, gained));
        }
    }

    println!("  {read} tiles read, {layers} MCLY records, {base_layers} of them a base");
    println!("\n  every bit that is ever set:");
    for (bit, count) in &bits {
        let named = match *bit {
            b if b & layer_flags::ANIMATION_ROTATION != 0 => "animation direction",
            b if b & layer_flags::ANIMATION_SPEED != 0 => "animation speed",
            layer_flags::ANIMATION_ENABLED => "animation on",
            layer_flags::OVERBRIGHT => "overbright",
            layer_flags::USE_ALPHA_MAP => "has an alpha map",
            layer_flags::ALPHA_COMPRESSED => "alpha map is RLE",
            _ => "unknown",
        };
        println!(
            "    0x{bit:04x}  {count:>7} records  {:>5.1}%  {named}",
            *count as f64 * 100.0 / layers.max(1) as f64
        );
    }
    println!(
        "\n  0x100 against the layer index: {agrees} agree, {disagrees} disagree\
         \n    (it must be set on every layer but the base and on no base, which is\
         \n     what pins the low byte: a wrong split would move the count)"
    );

    let total: u64 = animated.values().sum();
    println!("\n  {total} layers have the animation switch on, by (direction, speed):");
    for ((turn, rate), count) in &animated {
        println!(
            "    direction {turn} ({:>3.0}°)  speed {rate}  {count:>6} records",
            *turn as f32 * 45.0
        );
    }
    let mut ranked: Vec<_> = by_texture.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!("\n  …and what they are painted with:");
    for (name, count) in ranked.iter().take(16) {
        println!("    {count:>6}  {name}");
    }
    if ranked.len() > 16 {
        println!("    … and {} more", ranked.len() - 16);
    }
    // **And which tiles**, so the rule can be looked at rather than believed:
    // a scrolling layer is invisible in a count and obvious in a window, and
    // without this the only way to find one is to fly the map.
    println!("
  …and which tiles they are on:");
    for (x, y, count) in &where_animated {
        println!("    {map} {x} {y}   {count} layer(s)");
    }
    Ok(())
}
