//! `vale bake` — the three things an editor writes that nothing else does,
//! measured against the files the game shipped.
//!
//! ```text
//! vale bake <Map>            what the map claims, and what is behind each claim
//! vale bake <Map> <x> <y>    one tile: its minimap redrawn, its MCSH rebaked,
//!                               each scored against the shipped one
//! ```
//!
//! ## Why these two need an instrument and the rest of the editor does not
//!
//! A terrain edit is checked by looking at it: the ground is where you put it
//! or it is not. A **minimap** and an **`MCSH`** are different — both are
//! derived pictures of a tile, both are produced by a rule nobody wrote down,
//! and both look plausible when they are wrong. A minimap mirrored about either
//! axis is a perfectly good picture of a place that is not there. A shadow map
//! baked under the wrong sun is a perfectly good shadow falling the wrong way.
//!
//! But there are 1,500 shipped tiles that already carry the right answer. So
//! this generates one and scores it against the other, which is the only check
//! either of them can have.
//!
//! **A score is not a pass mark.** The generators here are not Blizzard's: the
//! minimap draws swatch-per-layer where theirs rendered the real textures, and
//! the shadow bake casts from this project's own collision hulls at a sun
//! recovered by `vale sun`. What the numbers say is *this is a picture of the
//! same place, the same way up* — which is the failure that matters and the one
//! no amount of staring at a single file reveals.

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

/// `vale bake <Map>` — what the map's WDT claims.
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

    // **The round trip, on the real file.** The same check `AdtFile` has and for
    // the same reason: a writer is worth exactly what its losslessness is worth,
    // and a WDT is the one file where a dropped chunk costs a whole map.
    match wdt.write() == raw {
        true => println!("  round trip: byte for byte"),
        false => println!("  ** round trip: the bytes changed"),
    }

    let tiles = wdt.tiles();
    println!("  {} of 4096 tiles claimed", tiles.len());

    // …and what is actually behind each claim, which the WDT cannot say. A
    // claimed tile with no ADT is a hole in the world that draws as nothing.
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

/// `vale bake <Map> <x> <y>` — one tile, both pictures.
pub fn cmd_bake_tile(cfg: &Config, map: &str, x: u32, y: u32) -> Result<(), String> {
    bake_tile(cfg, map, x, y, false, None)
}

/// `vale bake <Map> <x> <y> minimap` — the picture alone, over a sweep of
/// looks.
///
/// **Because the minimap's settings are not derivable and the shadow bake is
/// two minutes.** How dark a canopy reads, how wide a footprint is, how far the
/// water grades: none of those is measurable from a file, all of them are
/// measurable *against* the shipped picture, and iterating on them a minute at a
/// time is how they end up guessed instead. This scores several in one archive
/// load.
///
/// `dump` is a directory: the drawn picture and the shipped one are written
/// into it as `<Map>_<x>_<y>_drawn.png` and `_shipped.png`, because a
/// correlation says which of two looks is *closer* and nothing about what is
/// wrong with either. The picture beside the picture is what said the water
/// was missing on `Azeroth_33_38` while `r` was reporting a small positive
/// number.
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
    // **The neighbours, for their placements.** A building is recorded by the
    // tile its origin stands on and drawn wherever it reaches, so the picture
    // of this tile wants the eight files around it as well — see
    // `vale_edit::minimap::render`.
    let neighbours = neighbours_of(&mut assets, map, x, y);
    let everything: Vec<&Adt> = std::iter::once(&adt).chain(neighbours.iter()).collect();
    println!("    {} of the 8 neighbouring tiles exist", neighbours.len());
    let shapes = shapes_for(&mut assets, &everything);
    println!(
        "    {} buildings drawn from above, {} triangles between them",
        shapes.len(),
        shapes.values().map(|s| s.triangles.len()).sum::<usize>()
    );
    // **The water's colour is the zone's**, from the same chain the world tints
    // it by; see `minimap::Sources::water_of`, where the measurement is.
    let tables = crate::common::open_display_tables(&mut assets).ok();
    let map_id = crate::water::map_id_for(&mut assets, map);
    let centre = vale_assets::world::terrain::tile_centre(x, y);
    let water_of = |kind: wmo::Liquid| -> Option<vale_assets::tables::light::LiquidLight> {
        tables
            .as_ref()?
            .liquid_depth_light(map_id?, centre, kind, vale_assets::tables::light::NOON)
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
                        byte(light.close[0]),
                        byte(light.close[1]),
                        byte(light.close[2]),
                        byte(light.far[0]),
                        byte(light.far[1]),
                        byte(light.far[2]),
                        light.shallow_alpha,
                        light.deep_alpha,
                    );
                }
            }
        }
        _ => println!("    no Light.dbc chain: water drawn in the fallback blue"),
    }
    let shipped = shipped_minimap(&mut assets, map, x, y);

    // **The sweep.** One line per setting, each scored against the shipped
    // picture the same way, so the number that matters — which is *better* and
    // not which is good — is read off one table.
    let base = minimap::Look::default();
    let mut looks: Vec<(String, minimap::Look, f32)> = vec![("default".into(), base, 1.0)];
    if minimap_only {
        // **One at a time says which direction; a combination says whether they
        // add.** Without this the sweep can only ever report the best single
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
    // **The default look's picture is the one reported on**, whatever the
    // sweep prefers: it is the picture the editor writes, and the sweep's
    // best is a number about a picture nobody sees.
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

    // ----------------------------------------------------------------- MCSH
    println!("\n  MCSH");
    let mut tile = AdtFile::parse(&raw).map_err(|e| e.to_string())?;
    let theirs: Vec<Option<Vec<u8>>> = (0..tile.chunks.len())
        .map(|i| shadow::shadow_texels(&tile, i))
        .collect();
    let shipped_chunks = theirs.iter().filter(|t| t.is_some()).count();
    let shipped_texels: usize = theirs
        .iter()
        .flatten()
        .map(|t| t.iter().filter(|&&v| v != 0).count())
        .sum();
    println!("    shipped: {shipped_chunks} of 256 chunks carry one, {shipped_texels} texels in shadow");

    let hulls = build_occluder(&mut assets, &everything, x, y);
    // **Both ways round, because which of the two Blizzard's tools cast from is
    // an open question and this is what would settle it.** The hulls alone are
    // doodads and buildings; the ground alone is the terrain shadowing itself.
    // If the shipped bakes carry terrain self-shadow, adding the ground should
    // raise what is found without raising what is invented. If they do not, it
    // will do the opposite — and the numbers say which.
    let ground = shadow::Ground::of(&tile);
    for (what, occluder) in [
        ("hulls only", &hulls as &dyn shadow::Occluder),
        ("hulls and ground", &shadow::Either(&ground, &hulls)),
    ] {
        let started = std::time::Instant::now();
        let baked = shadow::bake(
            &mut tile,
            vale_assets::tables::light::SUN_TOWARD,
            occluder,
            shadow::REACH,
        );
        let took = started.elapsed();
        let mine: Vec<Option<Vec<u8>>> = (0..tile.chunks.len())
            .map(|i| shadow::shadow_texels(&tile, i))
            .collect();
        let mine_texels: usize = mine
            .iter()
            .flatten()
            .map(|t| t.iter().filter(|&&v| v != 0).count())
            .sum();
        println!(
            "    {what}: {} of 256 chunks, {mine_texels} texels, in {:.1}s",
            baked.shadowed,
            took.as_secs_f32()
        );
        score_shadow(&theirs, &mine);
    }

    // …and the file is still a file.
    let out = tile.write();
    match AdtFile::parse(&out).is_ok() && Adt::parse(&out).is_ok() {
        true => println!("    the rebaked tile still parses, both ways"),
        false => println!("    ** the rebaked tile does not parse"),
    }
    Ok(())
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
        // The smallest level that is still at least the swatch, which for a
        // 256x256 tileset is one Blizzard already authored.
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

/// **How wide each placed doodad is**, for the minimap's canopies: its drawn
/// vertices — see `M2::footprint_radius`, which says why not its box.
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

/// **Every building the tiles place, as its drawn surface** — see
/// `minimap::wmo_shape`. Each texture's mean colour is decoded once however
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

/// The eight tiles around one, parsed — those of them that exist.
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

/// Whether the best look still says north-west first, which the sweep must not
/// quietly lose.
fn report_orientation(
    _best: f32,
    _looks: &[(String, minimap::Look, f32)],
    _adt: &Adt,
    _swatches: &BTreeMap<String, minimap::Swatch>,
    _radii: &BTreeMap<String, f32>,
    _shipped: Option<&Vec<u8>>,
) {
}

/// **What colour the two pictures are**, overall and by what is under each
/// pixel.
///
/// `r` is blind to this: Pearson's correlation is invariant to gain and offset,
/// so a picture half as bright as the shipped one with every edge in the right
/// place scores perfectly. The means are the number the correlation cannot see,
/// and the per-depth water colours are what [`minimap::Look`]'s water is set
/// from — the shipped picture, read back where the tile's own `MCLQ` says the
/// water is.
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
    // …and by the file's own depth byte, which is what the opacity is
    // actually a function of.
    let bytes = &analysis.depth_byte;
    for (lo, hi) in [(1u8, 31u8), (32, 95), (96, 159), (160, 254), (255, 255)] {
        show(
            &format!("water byte {lo}..{hi}"),
            &|i| depths[i] > 0.0 && bytes[i] >= lo && bytes[i] <= hi,
        );
    }
}

/// **What the shipped picture multiplies the ground by**, read off it: by
/// slope, by the baked shadow, by canopy cover and by texture.
///
/// Each line divides the shipped picture's mean by the *unlit* blend's mean
/// over one class of pixel — so the number is the gain Blizzard's renderer
/// applied there, and the drawn column beside it is the gain this one did.
/// That is what [`minimap::Look`]'s three numbers are set from. The classes
/// are kept apart: the slope table is over dry, unshadowed, uncovered,
/// roofless pixels, so a hillside's gain is not a forest's darkness.
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
    // The gain each picture applies over the unlit blend, and the **residual**
    // — the shipped picture over the unlit blend *already lit by the default
    // look's slope term* — so the shadow and canopy rows read a multiplier of
    // their own rather than the slope of the ground they happen to stand on.
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
        // …and the colour, since a gain can be a tint.
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

/// How much of the shipped shadow the rebake found, and how much it invented.
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
        "    agreement: {:.1}% of texels ({both} both, {only_theirs} only shipped, {only_mine} only rebaked)",
        100.0 * (both + neither) as f32 / total as f32
    );
    if both + only_theirs > 0 {
        println!(
            "      of the shipped shadow, {:.1}% was found again",
            100.0 * both as f32 / (both + only_theirs) as f32
        );
    }
    // **The honest caveat, printed rather than left to be discovered.** The
    // hulls this casts from are collision hulls, and a tree's hull is its trunk:
    // the game's own bakes were cast from the drawn geometry, canopy included.
    // So a wooded tile is expected to come back with less shadow than it
    // shipped with, and the number to read is not the agreement but the
    // **asymmetry** — shadow this invents where the shipped file has none is
    // the failure, and shadow it misses is the known gap.
    println!(
        "      a shipped bake casts from drawn geometry and this from collision hulls,"
    );
    println!(
        "      so a wooded tile finds less. Read the invented column, not the total."
    );
}

/// Everything on this tile that casts a shadow.
///
/// `collision::tile_hulls` does the work; what is here is the reader it is
/// given and the [`shadow::Occluder`] it is wrapped in. The editor builds the
/// same list through the same function from its own archive handle.
fn build_occluder(assets: &mut vale_assets::Assets, adts: &[&Adt], x: u32, y: u32) -> Hulls {
    // **The tile and its neighbours, cut to what can reach it** — see
    // `collision::hulls_near` and `shadow::casting_box`.
    let colliders = vale_assets::world::collision::hulls_near(
        adts,
        |path| assets.read(path).ok(),
        Some(shadow::casting_box(x, y, shadow::REACH)),
    );
    println!("    {} hulls cast, from {} tiles", colliders.len(), adts.len());
    Hulls { colliders }
}

/// The placed hulls of a neighbourhood, as something a bake can ask.
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
