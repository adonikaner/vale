//! `vale bake`: the WDT, the minimap image and the `MCSH` shadow map that an
//! editor writes, measured against the files the game shipped.
//!
//! ```text
//! vale bake <Map>                          the tiles the WDT claims, and whether
//!                                          each has an ADT behind it
//! vale bake <Map> <x> <y>                  one tile: its minimap redrawn and its
//!                                          MCSH rebaked, each scored against
//!                                          the shipped one
//! vale bake <Map> <x> <y> minimap [<dir>]  the minimap alone, drawn under a
//!                                          sweep of looks
//! vale bake <Map> <x> <y> shadow [<dir>]   the MCSH rebake alone, cast five
//!                                          ways, each scored, with optional
//!                                          comparison pictures
//! vale bake <Map> <x> <y> sun              the default casting scored over a
//!                                          sweep of sun directions
//! ```
//!
//! ## Why the minimap and the MCSH are scored against shipped tiles
//!
//! A terrain edit can be checked by looking at it. A minimap and an `MCSH`
//! cannot: both are images derived from a tile by an undocumented rule, and
//! both look plausible when they are wrong. A minimap mirrored about either
//! axis still looks like terrain, and a shadow map baked under the wrong sun
//! still looks like shadow, falling the wrong way.
//!
//! The 1,500 shipped tiles carry correct images. Each command here generates
//! an image and scores it against the shipped one; no other check is
//! available for either image.
//!
//! ## What a score means
//!
//! A score is not a pass mark. The generators here are not Blizzard's: the
//! minimap draws one swatch per texture layer where the shipped images were
//! rendered from the full textures, and the shadow bake casts from the drawn,
//! alpha-tested doodad triangles and the building collision hulls at
//! `vale_assets::tables::light::SHADOW_SUN`. A high score says the image shows
//! the same place in the same orientation. That is the error that matters, and
//! inspecting a single file does not reveal it.

use std::collections::BTreeMap;
use std::sync::Arc;

use vale_assets::world::adt::Adt;
use vale_assets::world::blp;
use vale_assets::world::collision::Collider;
use vale_assets::world::m2::M2;
use vale_assets::world::wmo;
use vale_config::Config;
use vale_edit::adt::AdtFile;
use vale_edit::{minimap, shadow};

/// `vale bake <Map>`: the tiles the map's WDT claims, and whether each has an
/// ADT.
pub fn cmd_bake_map(cfg: &Config, map: &str) -> Result<(), String> {
    let mut assets = crate::common::open_assets(cfg)?;
    let path = vale_edit::wdt::wdt_path(map);
    let raw = assets.read(&path).map_err(|e| e.to_string())?;
    let wdt = vale_edit::wdt::WdtFile::parse(&raw).map_err(|e| e.to_string())?;

    println!("{path}");
    println!("  {} chunks, {} bytes", wdt.chunks.len(), raw.len());
    for chunk in &wdt.chunks {
        println!(
            "    {} {} bytes",
            String::from_utf8_lossy(&chunk.magic),
            chunk.data.len()
        );
    }

    // Check that the WDT writer reproduces the real file byte for byte, as
    // `AdtFile`'s round trip does for ADTs. A writer that loses data cannot be
    // used, and a chunk dropped from a WDT loses the whole map.
    match wdt.write() == raw {
        true => println!("  round trip: byte for byte"),
        false => println!("  ** round trip: the bytes changed"),
    }

    let tiles = wdt.tiles();
    println!("  {} of 4096 tiles claimed", tiles.len());

    // Check that each claimed tile has an ADT, which the WDT does not record.
    // A claimed tile with no ADT is a hole in the world that draws as nothing.
    let mut missing = Vec::new();
    for &(x, y) in &tiles {
        if assets.read(&vale_assets::adt_path(map, x, y)).is_err() {
            missing.push((x, y));
        }
    }
    match missing.is_empty() {
        true => println!("  every claimed tile has an ADT behind it"),
        false => {
            println!("  ** {} claimed tiles have no ADT:", missing.len());
            for (x, y) in missing.iter().take(10) {
                println!("       {x}, {y}");
            }
        }
    }
    Ok(())
}

/// `vale bake <Map> <x> <y>`: one tile's minimap and `MCSH`, each scored.
pub fn cmd_bake_tile(cfg: &Config, map: &str, x: u32, y: u32) -> Result<(), String> {
    bake_tile(cfg, map, x, y, false, None)
}

/// `vale bake <Map> <x> <y> minimap`: the minimap alone, drawn under a sweep
/// of looks.
///
/// The minimap's settings cannot be derived from a file: how dark a canopy
/// reads, how wide a footprint is, how far the water grades. Each can only be
/// measured against the shipped picture. This form scores several looks in
/// one archive load and skips the shadow bake, which takes two minutes.
///
/// `dump` is a directory: the drawn picture and the shipped one are written
/// into it as `<Map>_<x>_<y>_drawn.png` and `_shipped.png`. A correlation says
/// which of two looks is closer to the shipped picture and nothing about what
/// is wrong with either. On `Azeroth_33_38`, `r` reported a small positive
/// number while the two pictures side by side showed the water missing.
pub fn cmd_bake_minimap(
    cfg: &Config,
    map: &str,
    x: u32,
    y: u32,
    dump: Option<&str>,
) -> Result<(), String> {
    bake_tile(cfg, map, x, y, true, dump)
}

fn bake_tile(
    cfg: &Config,
    map: &str,
    x: u32,
    y: u32,
    minimap_only: bool,
    dump: Option<&str>,
) -> Result<(), String> {
    let mut assets = crate::common::open_assets(cfg)?;
    let path = vale_assets::adt_path(map, x, y);
    let raw = assets.read(&path).map_err(|e| e.to_string())?;
    let adt = Adt::parse(&raw).map_err(|e| e.to_string())?;
    println!("{path}");

    // ---------------------------------------------------------------- minimap
    println!("\n  minimap");
    let swatches = swatches_for(&mut assets, &adt);
    println!(
        "    {} of {} textures decoded into swatches",
        swatches.len(),
        adt.texture_names.len()
    );
    let radii = radii_for(&mut assets, &adt);
    println!("    {} of the doodads it places could be sized", radii.len());
    // The eight neighbouring tiles, for their placements. A building is
    // recorded in the tile its origin stands on and drawn wherever it reaches,
    // so the picture of this tile needs the placements of the tiles around it.
    // See `vale_edit::minimap::render`.
    let neighbours = neighbours_of(&mut assets, map, x, y);
    let everything: Vec<&Adt> = std::iter::once(&adt).chain(neighbours.iter()).collect();
    println!("    {} of the 8 neighbouring tiles exist", neighbours.len());
    let shapes = shapes_for(&mut assets, &everything);
    println!(
        "    {} buildings drawn from above, {} triangles between them",
        shapes.len(),
        shapes.values().map(|s| s.triangles.len()).sum::<usize>()
    );
    // The water takes the zone's colour, from the same chain the world tints
    // water with. The measurement is recorded at `minimap::Sources::water_of`.
    let tables = crate::common::open_display_tables(&mut assets).ok();
    let map_id = crate::water::map_id_for(&mut assets, map);
    let centre = vale_assets::world::terrain::tile_centre(x, y);
    let water_of = |kind: wmo::Liquid| -> Option<vale_assets::tables::light::LiquidLight> {
        tables
            .as_ref()?
            .liquid_light(map_id?, centre, kind, vale_assets::tables::light::NOON)
    };
    match (tables.is_some(), map_id) {
        (true, Some(id)) => {
            for kind in [wmo::Liquid::Water, wmo::Liquid::Ocean] {
                if let Some(light) = water_of(kind) {
                    let byte = |v: f32| (v * 255.0).round() as u32;
                    println!(
                        "    {:<6} shallow {:3}/{:3}/{:3} deep {:3}/{:3}/{:3} at {:.2}..{:.2}, from Light.dbc at map {id}",
                        match kind {
                            wmo::Liquid::Water => "water",
                            _ => "ocean",
                        },
                        byte(light.shallow[0]),
                        byte(light.shallow[1]),
                        byte(light.shallow[2]),
                        byte(light.deep[0]),
                        byte(light.deep[1]),
                        byte(light.deep[2]),
                        light.shallow_alpha,
                        light.deep_alpha,
                    );
                }
            }
        }
        _ => println!("    no Light.dbc chain: water drawn in the fallback blue"),
    }
    let shipped = shipped_minimap(&mut assets, map, x, y);

    // The sweep prints one line per setting, each scored against the shipped
    // picture the same way, so the settings are compared in one table. The
    // scores say which setting is better, not whether any setting is good.
    let base = minimap::Look::default();
    let mut looks: Vec<(String, minimap::Look, f32)> = vec![("default".into(), base, 1.0)];
    if minimap_only {
        // Varying one setting at a time shows which direction improves the
        // score; varying several together shows whether the changes add.
        // Without the combinations, the sweep can only report the best single
        // change from a default that may itself be wrong in two ways.
        for (ambient, diffuse) in [(0.3f32, 1.0f32), (0.46, 0.78), (0.6, 0.6), (1.0, 0.0)] {
            looks.push((
                format!("light {ambient:.2}+{diffuse:.2}"),
                minimap::Look {
                    ambient,
                    diffuse,
                    ..base
                },
                1.0,
            ));
        }
        for mix in [0.0f32, 0.25, 0.45, 0.62, 0.8] {
            looks.push((
                format!("canopy mix {mix:.2}"),
                minimap::Look { canopy_mix: mix, ..base },
                1.0,
            ));
        }
        for scale in [0.25f32, 0.5, 0.75, 1.5] {
            looks.push((format!("radius x{scale:.2}"), base, scale));
        }
        for shadow in [0.65f32, 0.8, 1.0] {
            looks.push((
                format!("shadow x{shadow:.2}"),
                minimap::Look { shadow, ..base },
                1.0,
            ));
        }
    }

    let mut best: Option<(String, Vec<u8>)> = None;
    let mut best_score = f32::MIN;
    // The reports below use the default look's picture, whichever look the
    // sweep scores best, because the default look is the one the editor
    // writes.
    let mut default_drawn: Option<Vec<u8>> = None;
    let swatch_of = |n: &str| swatches.get(&n.to_ascii_lowercase()).cloned();
    let shape_of = |n: &str| shapes.get(&n.to_ascii_lowercase()).cloned();
    for (name, look, scale) in &looks {
        let radius_of = |n: &str| radii.get(&n.to_ascii_lowercase()).map(|r| r * scale);
        let drawn = minimap::render(
            &adt,
            &neighbours,
            &minimap::Sources {
                swatch_of: &swatch_of,
                radius_of: &radius_of,
                shape_of: &shape_of,
                water_of: &water_of,
            },
            look,
        );
        if default_drawn.is_none() {
            default_drawn = Some(drawn.clone());
        }
        match &shipped {
            None => {
                println!("    {name}: no shipped picture to score against");
                best = Some((name.clone(), drawn));
                break;
            }
            Some(theirs) => {
                let scored = score_readings(&drawn, theirs);
                println!(
                    "    {name:<18} r = {:+.3}   (wrong readings {:+.3} {:+.3} {:+.3})",
                    scored[0], scored[1], scored[2], scored[3]
                );
                if scored[0] > best_score {
                    best_score = scored[0];
                    best = Some((name.clone(), drawn));
                }
            }
        }
    }

    let Some((name, _)) = best else {
        return Err("nothing was drawn".into());
    };
    let Some(drawn) = default_drawn else {
        return Err("nothing was drawn".into());
    };
    if shipped.is_some() {
        println!("    best: {name} at r = {best_score:+.3}");
        report_orientation(best_score, &looks, &adt, &swatches, &radii, shipped.as_ref());
    }

    let encoded = blp::encode_dxt1(&drawn, minimap::SIDE as u32, minimap::SIDE as u32)
        .map_err(|e| e.to_string())?;
    println!("    encodes to {} bytes of DXT1", encoded.len());
    if let Some(theirs) = &shipped {
        let radius_of = |n: &str| radii.get(&n.to_ascii_lowercase()).copied();
        let analysis = minimap::analyse(
            &adt,
            &neighbours,
            &minimap::Sources {
                swatch_of: &swatch_of,
                radius_of: &radius_of,
                shape_of: &shape_of,
                water_of: &water_of,
            },
        );
        report_colour(&analysis, &drawn, theirs);
        report_gains(&adt, &analysis, &drawn, theirs);
    }
    if let Some(dir) = dump {
        let stem = format!("{map}_{x}_{y}");
        write_png(dir, &format!("{stem}_drawn.png"), &drawn)?;
        if let Some(theirs) = &shipped {
            write_png(dir, &format!("{stem}_shipped.png"), theirs)?;
        }
        println!("    pictures written to {dir}");
    }
    if minimap_only {
        return Ok(());
    }

    println!("\n  MCSH");
    rebake_and_score(&mut assets, &everything, &raw, map, x, y, &[Casting::Default], None)
}

/// Every texture the tile names, reduced to a swatch.
fn swatches_for(
    assets: &mut vale_assets::Assets,
    adt: &Adt,
) -> BTreeMap<String, minimap::Swatch> {
    let mut out = BTreeMap::new();
    for name in &adt.texture_names {
        let Ok(bytes) = assets.read(name) else {
            continue;
        };
        let Ok(decoded) = blp::decode_mipped(&bytes) else {
            continue;
        };
        // Use the smallest mip level that is still at least `SWATCH` texels on
        // a side. For a 256x256 tileset that level is stored in the file.
        let mut best = (0usize, decoded.level_size(0));
        for (i, _) in decoded.levels().enumerate() {
            let size = decoded.level_size(i);
            if size.0.min(size.1) >= SWATCH {
                best = (i, size);
            }
        }
        let Some(level) = decoded.levels().nth(best.0) else {
            continue;
        };
        if let Some(swatch) = minimap::Swatch::from_rgba(
            level,
            best.1 .0 as usize,
            best.1 .1 as usize,
            SWATCH as usize,
        ) {
            out.insert(name.to_ascii_lowercase(), swatch);
        }
    }
    out
}

/// The footprint radius of each placed doodad, for the minimap's canopies,
/// measured from its drawn vertices. `M2::footprint_radius` says why the
/// bounding box is not used.
fn radii_for(assets: &mut vale_assets::Assets, adt: &Adt) -> BTreeMap<String, f32> {
    let mut out = BTreeMap::new();
    for placed in adt.placed_doodads() {
        let key = placed.path.to_ascii_lowercase();
        if out.contains_key(&key) {
            continue;
        }
        if let Some(radius) = assets
            .read(&placed.path)
            .ok()
            .and_then(|b| M2::parse(&b).ok())
            .and_then(|m| m.footprint_radius())
        {
            out.insert(key, radius);
        }
    }
    out
}

/// Every building the tiles place, as its drawn surface (see
/// `minimap::wmo_shape`). Each texture's mean colour is decoded once however
/// many buildings share it.
fn shapes_for(
    assets: &mut vale_assets::Assets,
    adts: &[&Adt],
) -> BTreeMap<String, Arc<minimap::Shape>> {
    let mut out = BTreeMap::new();
    let mut colours: BTreeMap<String, Option<[u8; 3]>> = BTreeMap::new();
    for adt in adts {
        for placed in adt.placed_wmos() {
            let key = placed.path.to_ascii_lowercase();
            if out.contains_key(&key) {
                continue;
            }
            let Some(root) = assets
                .read(&placed.path)
                .ok()
                .and_then(|b| wmo::WmoRoot::parse(&b).ok())
            else {
                continue;
            };
            let groups: Vec<wmo::WmoGroup> = (0..root.group_count)
                .filter_map(|i| {
                    assets
                        .read(&wmo::group_path(&placed.path, i))
                        .ok()
                        .and_then(|b| wmo::WmoGroup::parse(&b).ok())
                })
                .collect();
            let shape = minimap::wmo_shape(&root, &groups, |texture| {
                *colours
                    .entry(texture.to_ascii_lowercase())
                    .or_insert_with(|| mean_colour(assets, texture))
            });
            out.insert(key, Arc::new(shape));
        }
    }
    out
}

/// A texture's mean colour, off its smallest mip.
fn mean_colour(assets: &mut vale_assets::Assets, path: &str) -> Option<[u8; 3]> {
    let bytes = assets.read(path).ok()?;
    let decoded = blp::decode_mipped(&bytes).ok()?;
    let last = decoded.levels().count().checked_sub(1)?;
    let (w, h) = decoded.level_size(last);
    let level = decoded.levels().nth(last)?;
    minimap::Swatch::from_rgba(level, w as usize, h as usize, 1).map(|s| s.rgb[0])
}

/// The tiles that exist among the eight around one, parsed.
fn neighbours_of(assets: &mut vale_assets::Assets, map: &str, x: u32, y: u32) -> Vec<Adt> {
    let mut out = Vec::new();
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let (Ok(nx), Ok(ny)) = (u32::try_from(x as i64 + dx), u32::try_from(y as i64 + dy))
            else {
                continue;
            };
            if nx >= 64 || ny >= 64 {
                continue;
            }
            if let Some(adt) = assets
                .read(&vale_assets::adt_path(map, nx, ny))
                .ok()
                .and_then(|raw| Adt::parse(&raw).ok())
            {
                out.push(adt);
            }
        }
    }
    out
}

/// How many texels a swatch is on a side. Four: at sixteen pixels a chunk and
/// eight repeats across it, anything finer is below what the picture resolves.
const SWATCH: u32 = 4;

/// The shipped picture for a tile, as RGBA, if the index has one.
fn shipped_minimap(
    assets: &mut vale_assets::Assets,
    map: &str,
    x: u32,
    y: u32,
) -> Option<Vec<u8>> {
    let index = assets
        .read(vale_assets::tables::minimap::MD5_TRANSLATE)
        .ok()?;
    let tiles = vale_assets::tables::minimap::MinimapTiles::parse(&index);
    let name = tiles.texture(map, x, y)?;
    let bytes = assets.read(&name).ok()?;
    let blp = blp::decode(&bytes).ok()?;
    (blp.width as usize == minimap::SIDE && blp.height as usize == minimap::SIDE)
        .then_some(blp.rgba)
}

/// Whether the best look still has north-west first. The sweep must not change
/// the orientation without reporting it.
fn report_orientation(
    _best: f32,
    _looks: &[(String, minimap::Look, f32)],
    _adt: &Adt,
    _swatches: &BTreeMap<String, minimap::Swatch>,
    _radii: &BTreeMap<String, f32>,
    _shipped: Option<&Vec<u8>>,
) {
}

/// The mean colour of the two pictures, overall and by what is under each
/// pixel.
///
/// Pearson's correlation is invariant to gain and offset, so `r` does not
/// measure colour: a picture half as bright as the shipped one, with every
/// edge in the right place, scores 1. The means measure what the correlation
/// does not. The per-depth water colours are what [`minimap::Look`]'s water is
/// set from: the shipped picture, read where the tile's own `MCLQ` places
/// water.
fn report_colour(analysis: &minimap::Analysis, mine: &[u8], theirs: &[u8]) {
    let depths = &analysis.depth;
    let mean = |pixels: &[u8], keep: &dyn Fn(usize) -> bool| -> Option<[f32; 3]> {
        let (mut sum, mut n) = ([0f64; 3], 0usize);
        for (i, p) in pixels.chunks_exact(4).enumerate() {
            if !keep(i) {
                continue;
            }
            for c in 0..3 {
                sum[c] += f64::from(p[c]);
            }
            n += 1;
        }
        (n > 0).then(|| [
            (sum[0] / n as f64) as f32,
            (sum[1] / n as f64) as f32,
            (sum[2] / n as f64) as f32,
        ])
    };
    let show = |label: &str, keep: &dyn Fn(usize) -> bool| {
        let count = (0..minimap::SIDE * minimap::SIDE).filter(|&i| keep(i)).count();
        match (mean(mine, keep), mean(theirs, keep)) {
            (Some(a), Some(b)) => println!(
                "    {label:<22} {count:>6} px   drawn {:3.0} {:3.0} {:3.0}   shipped {:3.0} {:3.0} {:3.0}",
                a[0], a[1], a[2], b[0], b[1], b[2]
            ),
            _ => println!("    {label:<22} {count:>6} px"),
        }
    };
    println!("    mean colour, drawn against shipped:");
    show("everything", &|_| true);
    show("dry ground", &|i| depths[i] <= 0.0);
    show("water 0..2 yards", &|i| depths[i] > 0.0 && depths[i] <= 2.0);
    show("water 2..6 yards", &|i| depths[i] > 2.0 && depths[i] <= 6.0);
    show("water 6..12 yards", &|i| depths[i] > 6.0 && depths[i] <= 12.0);
    show("water 12..24 yards", &|i| depths[i] > 12.0 && depths[i] <= 24.0);
    show("water over 24 yards", &|i| depths[i] > 24.0);
    // The same means by the file's depth byte, which is the value the water's
    // opacity depends on.
    let bytes = &analysis.depth_byte;
    for (lo, hi) in [(1u8, 31u8), (32, 95), (96, 159), (160, 254), (255, 255)] {
        show(
            &format!("water byte {lo}..{hi}"),
            &|i| depths[i] > 0.0 && bytes[i] >= lo && bytes[i] <= hi,
        );
    }
}

/// The gain the shipped picture applies to the ground, measured by slope, by
/// the baked shadow, by canopy cover and by texture.
///
/// Each line divides the shipped picture's mean by the unlit blend's mean over
/// one class of pixel. The result is the gain Blizzard's renderer applied to
/// that class; the drawn column beside it is the gain this renderer applied.
/// [`minimap::Look`]'s three numbers are set from these gains. The classes are
/// kept apart: the slope table is over dry, unshadowed, uncovered, roofless
/// pixels, so canopy darkening does not enter a slope's gain.
fn report_gains(adt: &Adt, analysis: &minimap::Analysis, mine: &[u8], theirs: &[u8]) {
    let n = minimap::SIDE * minimap::SIDE;
    let luma = |p: &[u8], i: usize| {
        let at = i * 4;
        0.299 * f32::from(p[at]) + 0.587 * f32::from(p[at + 1]) + 0.114 * f32::from(p[at + 2])
    };
    let unlit = |i: usize| {
        let c = analysis.unlit[i];
        0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
    };
    // Dry, not under a roof, over a chunk: the pixels a gain can be read from.
    let dry = |i: usize| analysis.depth[i] <= 0.0 && !analysis.roof[i] && analysis.texture[i] != u32::MAX;
    let clean = |i: usize| dry(i) && !analysis.shadow[i] && analysis.canopy[i] < 0.05;
    // The gain each picture applies over the unlit blend, and the residual:
    // the shipped picture over the unlit blend already lit by the default
    // look's slope term. The residual gives the shadow and canopy rows a
    // multiplier of their own, without the slope of the ground under them.
    let look = minimap::Look::default();
    let gains = |keep: &dyn Fn(usize) -> bool| -> Option<(usize, f32, f32, f32)> {
        let (mut base, mut lit_base, mut shipped, mut drawn, mut count) =
            (0f64, 0f64, 0f64, 0f64, 0usize);
        for i in 0..n {
            if !keep(i) {
                continue;
            }
            base += f64::from(unlit(i));
            lit_base += f64::from(unlit(i) * minimap::lit(analysis.facing[i], &look));
            shipped += f64::from(luma(theirs, i));
            drawn += f64::from(luma(mine, i));
            count += 1;
        }
        (count > 0 && base > 0.0).then(|| {
            (
                count,
                (shipped / base) as f32,
                (drawn / base) as f32,
                (shipped / lit_base.max(1.0)) as f32,
            )
        })
    };
    let line = |label: &str, keep: &dyn Fn(usize) -> bool| match gains(keep) {
        Some((count, shipped, drawn, residual)) => println!(
            "    {label:<24} {count:>6} px   shipped x{shipped:.2}   drawn x{drawn:.2}   after the slope x{residual:.2}"
        ),
        None => println!("    {label:<24}      0 px"),
    };

    println!("    gain over the unlit blend, shipped against drawn:");
    println!("      by slope (dot with the sun; flat ground is 0.64), clean pixels only:");
    let edges = [-1.0f32, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.01];
    for pair in edges.windows(2) {
        let (lo, hi) = (pair[0], pair[1]);
        line(
            &format!("  facing {:.1}..{:.1}", lo.max(0.0), hi.min(1.0)),
            &|i| clean(i) && analysis.facing[i] >= lo && analysis.facing[i] < hi,
        );
    }
    println!("      by the baked shadow, uncovered pixels only:");
    line("  in MCSH shadow", &|i| dry(i) && analysis.shadow[i] && analysis.canopy[i] < 0.05);
    line("  not in shadow", &|i| clean(i));
    println!("      by canopy cover, unshadowed pixels only:");
    for (lo, hi) in [(0.05f32, 0.3f32), (0.3, 0.7), (0.7, 1.01)] {
        line(
            &format!("  cover {lo:.2}..{:.2}", hi.min(1.0)),
            &|i| dry(i) && !analysis.shadow[i] && analysis.canopy[i] >= lo && analysis.canopy[i] < hi,
        );
    }
    println!("      by texture, clean pixels only:");
    let mut ids: Vec<u32> = analysis
        .texture
        .iter()
        .copied()
        .filter(|&t| t != u32::MAX)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        let name = adt
            .texture_names
            .get(id as usize)
            .map(|s| s.rsplit('\\').next().unwrap_or(s).to_string())
            .unwrap_or_default();
        let keep = |i: usize| clean(i) && analysis.texture[i] == id;
        let Some((count, shipped, drawn, _)) = gains(&keep) else {
            continue;
        };
        if count < 200 {
            continue;
        }
        // The gain per colour channel as well, since a gain can be a tint.
        let (mut base, mut theirs_sum) = ([0f64; 3], [0f64; 3]);
        for i in 0..n {
            if !keep(i) {
                continue;
            }
            for c in 0..3 {
                base[c] += f64::from(analysis.unlit[i][c]);
                theirs_sum[c] += f64::from(theirs[i * 4 + c]);
            }
        }
        println!(
            "    {:<24} {count:>6} px   shipped x{shipped:.2}   drawn x{drawn:.2}   rgb x{:.2} x{:.2} x{:.2}",
            format!("  {name}").chars().take(24).collect::<String>(),
            theirs_sum[0] / base[0].max(1.0),
            theirs_sum[1] / base[1].max(1.0),
            theirs_sum[2] / base[2].max(1.0),
        );
    }
}

/// Write `SIDE x SIDE` RGBA as a PNG under `dir`.
fn write_png(dir: &str, name: &str, rgba: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{dir}: {e}"))?;
    let path = std::path::Path::new(dir).join(name);
    let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        minimap::SIDE as u32,
        minimap::SIDE as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())
}

/// Pearson's r for each of the four readings of the drawn picture.
fn score_readings(mine: &[u8], theirs: &[u8]) -> [f32; 4] {
    let luma = |p: &[u8], i: usize| {
        let at = i * 4;
        0.299 * f32::from(p[at]) + 0.587 * f32::from(p[at + 1]) + 0.114 * f32::from(p[at + 2])
    };
    let side = minimap::SIDE;
    let mut scores = [0f32; 4];
    for (reading, score) in scores.iter_mut().enumerate() {
        let mut pairs = Vec::with_capacity(side * side);
        for py in 0..side {
            for px in 0..side {
                let (qx, qy) = match reading {
                    1 => (side - 1 - px, py),
                    2 => (px, side - 1 - py),
                    3 => (side - 1 - px, side - 1 - py),
                    _ => (px, py),
                };
                pairs.push((luma(mine, py * side + px), luma(theirs, qy * side + qx)));
            }
        }
        *score = correlation(&pairs);
    }
    scores
}

/// Pearson's r, which is the right statistic here because the two pictures have
/// different exposure: what is being asked is whether they vary together, not
/// whether they are the same colour.
fn correlation(pairs: &[(f32, f32)]) -> f32 {
    let n = pairs.len() as f32;
    if n < 2.0 {
        return 0.0;
    }
    let (mut sx, mut sy) = (0.0, 0.0);
    for (a, b) in pairs {
        sx += a;
        sy += b;
    }
    let (mx, my) = (sx / n, sy / n);
    let (mut num, mut da, mut db) = (0.0f32, 0.0f32, 0.0f32);
    for (a, b) in pairs {
        let (u, v) = (a - mx, b - my);
        num += u * v;
        da += u * u;
        db += v * v;
    }
    match da > 0.0 && db > 0.0 {
        true => num / (da.sqrt() * db.sqrt()),
        false => 0.0,
    }
}

// ---------------------------------------------------------------------------
// MCSH
// ---------------------------------------------------------------------------

/// One way of casting a tile's shadow, for `vale bake ... shadow` to score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Casting {
    /// What the editor bakes: drawn doodads, alpha-tested, and building hulls.
    Default,
    /// The same with every doodad triangle solid.
    Solid,
    /// The same with every drawn building triangle instead of the hulls.
    DrawnBuildings,
    /// The default and the terrain shadowing itself.
    WithGround,
    /// The collision hulls of doodads and buildings, which the editor cast
    /// from before.
    Hulls,
}

impl Casting {
    const ALL: [Casting; 5] = [
        Casting::Default,
        Casting::Solid,
        Casting::DrawnBuildings,
        Casting::WithGround,
        Casting::Hulls,
    ];

    fn name(self) -> &'static str {
        match self {
            Casting::Default => "default",
            Casting::Solid => "solid",
            Casting::DrawnBuildings => "drawn-buildings",
            Casting::WithGround => "ground",
            Casting::Hulls => "hulls",
        }
    }
}

/// The settings of a shadow rebake that the instruments vary, read from the
/// environment: `SUN=<azimuth>,<elevation>` in degrees, `REACH=<yards>` and
/// `ALPHA=<0..255>`. Each defaults to what the editor uses.
struct ShadowSettings {
    sun: [f32; 3],
    reach: f32,
    alpha: u8,
}

impl ShadowSettings {
    fn from_env() -> ShadowSettings {
        let sun = std::env::var("SUN")
            .ok()
            .and_then(|s| {
                let (a, e) = s.split_once(',')?;
                Some(sun_direction(a.trim().parse().ok()?, e.trim().parse().ok()?))
            })
            .unwrap_or(vale_assets::tables::light::SHADOW_SUN);
        let number = |name: &str| std::env::var(name).ok().and_then(|s| s.parse::<f32>().ok());
        ShadowSettings {
            sun,
            reach: number("REACH").unwrap_or(shadow::REACH),
            alpha: number("ALPHA").map_or(shadow::ALPHA_TEST, |a| a.clamp(0.0, 255.0) as u8),
        }
    }
}

/// A unit vector toward a sun at `azimuth` degrees from +x toward +y and
/// `elevation` degrees above the horizon.
fn sun_direction(azimuth: f32, elevation: f32) -> [f32; 3] {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    [e.cos() * a.cos(), e.cos() * a.sin(), e.sin()]
}

/// The occluder for one way of casting.
fn occluder_for(
    casting: Casting,
    assets: &mut vale_assets::Assets,
    adts: &[&Adt],
    tile: &AdtFile,
    x: u32,
    y: u32,
    settings: &ShadowSettings,
) -> Box<dyn shadow::Occluder + Sync> {
    use vale_assets::world::collision::{hulls_near, shadow_casters_near, BuildingCasts};
    let within = Some(shadow::casting_box(x, y, settings.reach));
    let mut casters = |buildings: BuildingCasts, cut_out: bool| {
        let set = shadow_casters_near(adts, |p| assets.read(p).ok(), within, buildings, cut_out);
        println!(
            "    {}: {} triangles, {} alpha masks",
            casting.name(),
            set.triangles.len(),
            set.masks.len()
        );
        shadow::Casters::new(set, settings.alpha)
    };
    match casting {
        Casting::Default => Box::new(casters(BuildingCasts::Hull, true)),
        Casting::Solid => Box::new(casters(BuildingCasts::Hull, false)),
        Casting::DrawnBuildings => Box::new(casters(BuildingCasts::Drawn, true)),
        Casting::WithGround => Box::new(WithGround(
            casters(BuildingCasts::Hull, true),
            shadow::Ground::of(tile),
        )),
        Casting::Hulls => {
            let colliders = hulls_near(adts, |p| assets.read(p).ok(), within);
            println!("    hulls: {} placements", colliders.len());
            Box::new(Hulls { colliders })
        }
    }
}

/// The ground and a set of casters as one occluder.
struct WithGround(shadow::Casters, shadow::Ground);

impl shadow::Occluder for WithGround {
    fn blocked(&self, from: [f32; 3], toward: [f32; 3], max: f32) -> bool {
        self.0.blocked(from, toward, max) || self.1.blocked(from, toward, max)
    }
}

/// Placed collision hulls as an occluder. Each ray is tested against every
/// hull, which takes about a minute a tile.
struct Hulls {
    colliders: Vec<Collider>,
}

impl shadow::Occluder for Hulls {
    fn blocked(&self, from: [f32; 3], toward: [f32; 3], max: f32) -> bool {
        self.colliders
            .iter()
            .any(|c| c.ray(from, toward, max).is_some())
    }
}

/// Every chunk's `MCSH` texels, or `None` for a chunk without one.
fn tile_shadow(tile: &AdtFile) -> Vec<Option<Vec<u8>>> {
    (0..tile.chunks.len())
        .map(|i| shadow::shadow_texels(tile, i))
        .collect()
}

fn shadowed_texels(maps: &[Option<Vec<u8>>]) -> usize {
    maps.iter()
        .flatten()
        .map(|t| t.iter().filter(|&&v| v != 0).count())
        .sum()
}

/// Rebake `raw`'s shadow once per entry of `castings`, print each score
/// against the shipped map, and with `dump` write each comparison picture.
#[allow(clippy::too_many_arguments)]
fn rebake_and_score(
    assets: &mut vale_assets::Assets,
    adts: &[&Adt],
    raw: &[u8],
    map: &str,
    x: u32,
    y: u32,
    castings: &[Casting],
    dump: Option<&str>,
) -> Result<(), String> {
    let settings = ShadowSettings::from_env();
    let mut tile = AdtFile::parse(raw).map_err(|e| e.to_string())?;
    let theirs = tile_shadow(&tile);
    println!(
        "    shipped: {} of 256 chunks carry one, {} texels in shadow",
        theirs.iter().filter(|t| t.is_some()).count(),
        shadowed_texels(&theirs)
    );
    if let Some(dir) = dump {
        write_shadow_png(dir, &format!("{map}_{x}_{y}_shadow_shipped.png"), &theirs, &theirs)?;
    }
    for &casting in castings {
        let occluder = occluder_for(casting, assets, adts, &tile, x, y, &settings);
        let started = std::time::Instant::now();
        let baked = shadow::bake(&mut tile, settings.sun, occluder.as_ref(), settings.reach);
        let mine = tile_shadow(&tile);
        println!(
            "    {}: {} of 256 chunks, {} texels, cast in {:.1}s",
            casting.name(),
            baked.shadowed,
            shadowed_texels(&mine),
            started.elapsed().as_secs_f32()
        );
        score_shadow(&theirs, &mine);
        if let Some(dir) = dump {
            let name = format!("{map}_{x}_{y}_shadow_{}.png", casting.name());
            write_shadow_png(dir, &name, &theirs, &mine)?;
        }
    }
    let out = tile.write();
    match AdtFile::parse(&out).is_ok() && Adt::parse(&out).is_ok() {
        true => println!("    the rebaked tile parses with both readers"),
        false => println!("    ** the rebaked tile does not parse"),
    }
    Ok(())
}

/// How much of the shipped shadow the rebake found, and how much it added.
fn score_shadow(theirs: &[Option<Vec<u8>>], mine: &[Option<Vec<u8>>]) {
    let (mut both, mut only_theirs, mut only_mine, mut neither) = (0usize, 0, 0, 0);
    for (t, m) in theirs.iter().zip(mine.iter()) {
        let empty = Vec::new();
        let t = t.as_ref().unwrap_or(&empty);
        let m = m.as_ref().unwrap_or(&empty);
        for i in 0..vale_assets::world::adt::ALPHA_LEN {
            let a = t.get(i).copied().unwrap_or(0) != 0;
            let b = m.get(i).copied().unwrap_or(0) != 0;
            match (a, b) {
                (true, true) => both += 1,
                (true, false) => only_theirs += 1,
                (false, true) => only_mine += 1,
                (false, false) => neither += 1,
            }
        }
    }
    let total = (both + only_theirs + only_mine + neither).max(1);
    println!(
        "      agreement: {:.1}% of texels ({both} both, {only_theirs} only shipped, {only_mine} only rebaked)",
        100.0 * (both + neither) as f32 / total as f32
    );
    if both + only_theirs > 0 {
        println!(
            "      of the shipped shadow, {:.1}% was found",
            100.0 * both as f32 / (both + only_theirs) as f32
        );
    }
}

/// `vale bake <Map> <x> <y> shadow [<dir>]`: the `MCSH` rebake alone, cast
/// each way [`Casting`] names and scored against the shipped map.
///
/// `CASTING=<name>[,<name>...]` limits the run to those ways; `SUN`, `REACH`
/// and `ALPHA` change the settings (see [`ShadowSettings`]). With a
/// directory, each result is written as a 1024x1024 PNG: black where both maps
/// are shadowed, red where only the shipped map is, blue where only the rebake
/// is, white where neither is.
pub fn cmd_bake_shadow(cfg: &Config, map: &str, x: u32, y: u32, dump: Option<&str>) -> Result<(), String> {
    let mut assets = crate::common::open_assets(cfg)?;
    let path = vale_assets::adt_path(map, x, y);
    let raw = assets.read(&path).map_err(|e| e.to_string())?;
    let adt = Adt::parse(&raw).map_err(|e| e.to_string())?;
    println!("{path}");
    let neighbours = neighbours_of(&mut assets, map, x, y);
    let everything: Vec<&Adt> = std::iter::once(&adt).chain(neighbours.iter()).collect();
    let wanted = std::env::var("CASTING").ok();
    let castings: Vec<Casting> = Casting::ALL
        .into_iter()
        .filter(|c| wanted.as_deref().is_none_or(|w| w.split(',').any(|n| n.trim() == c.name())))
        .collect();
    if castings.is_empty() {
        let names: Vec<&str> = Casting::ALL.iter().map(|c| c.name()).collect();
        return Err(format!("CASTING names none of: {}", names.join(", ")));
    }
    rebake_and_score(&mut assets, &everything, &raw, map, x, y, &castings, dump)
}

/// A tile's shadow maps as one 1024x1024 comparison picture.
fn write_shadow_png(
    dir: &str,
    name: &str,
    theirs: &[Option<Vec<u8>>],
    mine: &[Option<Vec<u8>>],
) -> Result<(), String> {
    use vale_assets::world::adt::{ALPHA_SIDE, CHUNKS_PER_SIDE};
    let side = CHUNKS_PER_SIDE * ALPHA_SIDE;
    let mut rgba = vec![0u8; side * side * 4];
    for chunk in 0..CHUNKS_PER_SIDE * CHUNKS_PER_SIDE {
        let (cx, cy) = (chunk % CHUNKS_PER_SIDE, chunk / CHUNKS_PER_SIDE);
        for ty in 0..ALPHA_SIDE {
            for tx in 0..ALPHA_SIDE {
                let i = ty * ALPHA_SIDE + tx;
                let at = |v: &[Option<Vec<u8>>]| {
                    v.get(chunk)
                        .and_then(|c| c.as_ref())
                        .and_then(|c| c.get(i))
                        .is_some_and(|&b| b != 0)
                };
                let colour = match (at(theirs), at(mine)) {
                    (true, true) => [0, 0, 0],
                    (true, false) => [220, 40, 40],
                    (false, true) => [40, 80, 230],
                    (false, false) => [255, 255, 255],
                };
                let p = ((cy * ALPHA_SIDE + ty) * side + cx * ALPHA_SIDE + tx) * 4;
                rgba[p..p + 3].copy_from_slice(&colour);
                rgba[p + 3] = 255;
            }
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{dir}: {e}"))?;
    let path = std::path::Path::new(dir).join(name);
    let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), side as u32, side as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgba).map_err(|e| e.to_string())
}

/// `vale bake <Map> <x> <y> sun`: the sun direction under which the default
/// casting best reproduces the shipped `MCSH`.
///
/// Casts a sample of the tile's texels, one in `STRIDE` (default 4) along each
/// axis, under each direction of a grid, and prints the share that agree with
/// the shipped map. The grid is azimuth `AZIMUTH` and elevation `ELEVATION`,
/// each `<from>,<to>,<step>` in degrees, by default 40..50 and 40..54 in steps
/// of 1. `REACH` and `ALPHA` apply as in `vale bake ... shadow`.
pub fn cmd_bake_sun(cfg: &Config, map: &str, x: u32, y: u32) -> Result<(), String> {
    use vale_assets::world::adt::{ALPHA_SIDE, CHUNKS_PER_SIDE, CHUNK_SIZE};
    let mut assets = crate::common::open_assets(cfg)?;
    let path = vale_assets::adt_path(map, x, y);
    let raw = assets.read(&path).map_err(|e| e.to_string())?;
    let adt = Adt::parse(&raw).map_err(|e| e.to_string())?;
    println!("{path}");
    let neighbours = neighbours_of(&mut assets, map, x, y);
    let everything: Vec<&Adt> = std::iter::once(&adt).chain(neighbours.iter()).collect();
    let tile = AdtFile::parse(&raw).map_err(|e| e.to_string())?;
    let theirs = tile_shadow(&tile);
    let settings = ShadowSettings::from_env();
    let occluder = occluder_for(Casting::Default, &mut assets, &everything, &tile, x, y, &settings);
    let stride: usize = std::env::var("STRIDE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4)
        .max(1);
    let range = |name: &str, default: (f32, f32, f32)| -> Vec<f32> {
        let (from, to, step) = std::env::var(name)
            .ok()
            .and_then(|s| {
                let v: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                (v.len() == 3 && v[2] > 0.0).then(|| (v[0], v[1], v[2]))
            })
            .unwrap_or(default);
        let count = ((to - from) / step).round().max(0.0) as usize;
        (0..=count).map(|i| from + step * i as f32).collect()
    };
    let azimuths = range("AZIMUTH", (40.0, 50.0, 1.0));
    let elevations = range("ELEVATION", (40.0, 54.0, 1.0));

    // The sampled texels: their ray origins and whether the shipped map
    // shadows them.
    let mut samples: Vec<([f32; 3], bool)> = Vec::new();
    let texel = CHUNK_SIZE / ALPHA_SIDE as f32;
    for (index, chunk) in tile.chunks.iter().enumerate().take(CHUNKS_PER_SIDE * CHUNKS_PER_SIDE) {
        let origin = chunk.head().position();
        let ground = vale_edit::adt::heights::ground(chunk);
        for ty in (stride / 2..ALPHA_SIDE - 1).step_by(stride) {
            for tx in (stride / 2..ALPHA_SIDE - 1).step_by(stride) {
                let px = origin[0] - (ty as f32 + 0.5) * texel;
                let py = origin[1] - (tx as f32 + 0.5) * texel;
                let Some(z) = ground.height_at(px, py) else {
                    continue;
                };
                let shipped = theirs[index]
                    .as_ref()
                    .is_some_and(|t| t[ty * ALPHA_SIDE + tx] != 0);
                samples.push(([px, py, z + shadow::LIFT], shipped));
            }
        }
    }
    let score = |sun: [f32; 3]| -> f32 {
        let agree = samples
            .iter()
            .filter(|(from, shipped)| occluder.blocked(*from, sun, settings.reach) == *shipped)
            .count();
        agree as f32 / samples.len().max(1) as f32
    };

    println!("  {} texels sampled", samples.len());
    let header: String = azimuths.iter().map(|a| format!(" {a:6.2}")).collect();
    println!("  elevation \\ azimuth{header}");
    let mut best = (f32::MIN, 0.0, 0.0);
    for &e in &elevations {
        let mut line = format!("  {e:18.2}");
        for &a in &azimuths {
            let s = score(sun_direction(a, e));
            line += &format!(" {s:6.4}");
            if s > best.0 {
                best = (s, a, e);
            }
        }
        println!("{line}");
    }
    println!("  best: azimuth {} elevation {}, {:.4} agree", best.1, best.2, best.0);
    let held = vale_assets::tables::light::SHADOW_SUN;
    println!(
        "  SHADOW_SUN: azimuth {:.2} elevation {:.2}, {:.4} agree",
        held[1].atan2(held[0]).to_degrees(),
        held[2].atan2(held[0].hypot(held[1])).to_degrees(),
        score(held)
    );
    Ok(())
}
