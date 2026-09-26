//! `vale models` — every M2 a tile places: decode, place, verify.

use crate::common::*;
use vale_assets::{world::adt::Adt, adt_path};
use vale_config::Config;

/// Every model placed on one tile: decode it, place it, and check it landed.
///
/// This is the counterpart of `vale textures` for geometry, and it exists to
/// check the two things unit tests on invented bytes cannot: that real 1.12 M2s
/// parse, and that a placement lands on the terrain it is supposed to stand on.
/// The height check is the load-bearing one — a wrong axis order in
/// `placement_to_world` still produces plausible coordinates, but it puts the
/// doodads hundreds of yards off the ground.
pub fn cmd_models(cfg: &Config, map: &str, x: u32, y: u32) -> Result<(), String> {
    use vale_assets::world::m2::M2;
    use std::collections::{BTreeMap, BTreeSet};

    let mut assets = open_assets(cfg)?;
    let raw = assets.read(&adt_path(map, x, y)).map_err(|e| e.to_string())?;
    let adt = Adt::parse(&raw).map_err(|e| e.to_string())?;

    let doodads = adt.placed_doodads();
    let wmos = adt.placed_wmos();
    println!("{}", adt_path(map, x, y));
    println!(
        "  {} doodad placements of {} distinct models, {} WMO placements of {}",
        doodads.len(),
        adt.model_names.len(),
        wmos.len(),
        adt.wmo_names.len()
    );

    // Decode every distinct model once, and total up what a renderer would have
    // to upload for this tile.
    let mut failures = 0;
    let mut vertices = 0usize;
    let mut triangles = 0usize;
    let mut textures: BTreeSet<String> = BTreeSet::new();
    let mut blends: BTreeMap<u16, usize> = BTreeMap::new();
    let mut radii: BTreeMap<String, f32> = BTreeMap::new();
    // Scenery is drawn instanced and is never posed, so its keyframes are dead
    // weight in the payload. Worth knowing how much of it there is before
    // deciding whether the `model` command should learn to leave them out.
    let mut animated = 0usize;
    let mut keyframes = 0usize;
    // **The number that decides whether animating them is affordable**, and it
    // is the one this used to leave out: a *model* carrying a skeleton costs
    // nothing, and a *placement* of one costs a pose, a joint entity per bone
    // and a skin upload every frame. The two are wildly different counts —
    // windmills and banners are a handful of models placed a handful of times,
    // where the trees are most of the tile.
    let mut animated_names: BTreeSet<String> = BTreeSet::new();

    // **Which models this tile places are lamps**, by the one rule that says so
    // — see `vale_assets::world::glow`, whose whole population this names.
    let mut lamps: BTreeSet<String> = BTreeSet::new();
    let distinct: BTreeSet<String> = doodads.iter().map(|d| d.path.clone()).collect();
    for path in &distinct {
        let bytes = match assets.read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("    !! {path}: {e}");
                failures += 1;
                continue;
            }
        };
        match M2::parse(&bytes) {
            Ok(m) => {
                vertices += m.positions.len();
                triangles += m.triangle_count();
                radii.insert(path.clone(), m.bounding_radius);
                for t in m.texture_paths() {
                    textures.insert(t.to_string());
                }
                for b in m.visible_batches(vale_assets::world::m2::Dress::Creature) {
                    *blends.entry(b.blend).or_insert(0) += 1;
                }
                // **The two shapes a glow batch comes in**, counted apart —
                // see the census printed below, which is what the rule in
                // `assets::world::glow` was widened against.
                if !vale_assets::world::glow::glows(&m).is_empty() {
                    lamps.insert(path.clone());
                }
                if let Some(sk) = m.skeleton.as_ref() {
                    animated += 1;
                    animated_names.insert(path.clone());
                    keyframes += sk
                        .bones
                        .iter()
                        .flat_map(|b| [&b.translation, &b.rotation, &b.scale])
                        .filter_map(|t| t.as_ref())
                        .map(|t| t.times.len())
                        .sum::<usize>();
                }
            }
            Err(e) => {
                println!("    !! {path}: {e}");
                failures += 1;
            }
        }
    }
    println!(
        "  {} of {} models decoded ({vertices} verts, {triangles} tris, {} textures)",
        distinct.len() - failures,
        distinct.len(),
        textures.len()
    );
    println!("  blend modes in use: {blends:?}  (0 opaque, 1 alpha-key, 2 blend, 3 additive)");
    // **What the lamp rule fires on**, which is the measurement that decides
    // whether "an unlit batch is a light" is a rule or a wish. An `unlit`
    // material is the file's own statement that a surface is not lit by
    // anything, which is what a source is — but the flag is cheap and a model
    // may carry it for other reasons, so the population it selects is worth
    // looking at rather than assuming. See `vale_assets::world::glow`.
    println!("  {} of the {} distinct models are lamps:", lamps.len(), distinct.len());
    for name in lamps.iter().take(12) {
        println!("    lamp: {name}");
    }
    let animated_placements = doodads
        .iter()
        .filter(|d| animated_names.contains(&d.path))
        .count();
    println!(
        "  {animated} of them carry a skeleton, over {animated_placements} of the \
         {} placements: {keyframes} keyframes, about {} KB",
        doodads.len(),
        keyframes * 16 / 1024
    );
    for name in &animated_names {
        let n = doodads.iter().filter(|d| &d.path == name).count();
        println!("    {n:5} x {name}");
    }

    // Do the textures the models ask for actually exist?
    let missing: Vec<&String> = textures
        .iter()
        .filter(|t| assets.read(t).is_err())
        .collect();
    if missing.is_empty() {
        println!("  every model texture resolves in the archive chain");
    } else {
        println!("  !! {} model textures missing, e.g. {:?}", missing.len(), &missing[..missing.len().min(3)]);
    }

    // Placement sanity: a doodad should be standing on the ground, not buried
    // or in orbit. Terrain has no height over holes and at the tile margin, so
    // only placements with a height under them are judged.
    let mut checked = 0;
    let mut off_ground = 0;
    let mut worst: Option<(f32, &str)> = None;
    for d in &doodads {
        let Some(ground) = adt.height_at(d.position[0], d.position[1]) else {
            continue;
        };
        checked += 1;
        let delta = d.position[2] - ground;
        if delta.abs() > 15.0 {
            off_ground += 1;
        }
        if worst.is_none_or(|(w, _)| delta.abs() > w.abs()) {
            worst = Some((delta, &d.path));
        }
    }
    println!(
        "  {checked} placements sit over known terrain; {off_ground} more than 15y off it"
    );
    if let Some((delta, path)) = worst {
        println!("    furthest from the ground: {delta:+.1}y  {path}");
    }

    // **The `MCSH` bit under each placement's origin**, which is what picks
    // the per-instance sun scale the client keeps on every placed M2 (2.5 lit
    // / 0.5 shadowed — see `models::sun_scale` in the renderer). The share
    // should read as the zone the same way the ground's own coverage does:
    // high in Duskwood, near zero on a Tanaris tile — a count that tracks the
    // tile's shadow share is what says the lookup samples the ground the
    // placement stands on rather than some other chunk's map.
    let shadowed = doodads
        .iter()
        .filter(|d| adt.shadowed_at(d.position[0], d.position[1]))
        .count();
    println!(
        "  {shadowed} of {} placements stand in the ground's baked MCSH shadow \
         (sun scale 0.5 against 2.5 lit)",
        doodads.len()
    );

    for d in doodads.iter().take(5) {
        println!(
            "    {:.<62} ({:9.1},{:9.1},{:7.1}) scale {:.2} r{:.1}",
            d.path.rsplit('\\').next().unwrap_or(&d.path),
            d.position[0],
            d.position[1],
            d.position[2],
            d.scale,
            radii.get(&d.path).copied().unwrap_or(0.0),
        );
    }
    for w in wmos.iter().take(5) {
        println!(
            "    WMO {:.<58} ({:9.1},{:9.1},{:7.1})",
            w.path.rsplit('\\').next().unwrap_or(&w.path),
            w.position[0],
            w.position[1],
            w.position[2],
        );
    }
    Ok(())
}
