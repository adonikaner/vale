//! `vale minimap` — **the little round map's three inputs, checked with no
//! window**: which picture covers which tile, how far the view reaches, and
//! which way up the pictures are.
//!
//! ```text
//! vale minimap                 the whole index, every map, the zoom table
//! vale minimap Azeroth         one map: its pictures against its own tiles
//! vale minimap Azeroth 32 48   …one tile, decoded — and the orientation check
//! vale minimap wmo             every WMO's pictures against its groups' grids
//! vale minimap <path>.wmo      one WMO: its groups, grids and orientation vote
//! ```
//!
//! ## The orientation is the check that matters, and it is measurable
//!
//! Everything else here is arithmetic that fails loudly. Which corner of a
//! minimap BLP is the tile's **north-west** is not: get it wrong and the map
//! still draws, still moves with the character, and is simply mirrored — which
//! is the class of fault this project has been bitten by repeatedly, and the one
//! a still picture cannot settle because a stranger's coastline looks like a
//! coastline either way.
//!
//! So it is measured against something the same tile already states in a
//! different file: **its water**. `MCLQ` says which of a tile's 16x16 chunks
//! carry a liquid; the minimap picture is 256x256, which is exactly 16x16 texels
//! per chunk; and water is the one thing on a minimap that is unmistakably a
//! colour. The check bins the picture's blue-dominant texels per chunk and
//! correlates against `MCLQ`'s own grid — **in the assumed orientation and in
//! the three wrong ones** — and prints all four. If the assumed one is not the
//! best of the four by a wide margin, the mapping is backwards.
//!
//! It only says something on a tile that has both water and land, and **one
//! tile is an anecdote** — a stream is not enough blue to separate the readings
//! and a dark ocean under a cliff can beat them. So the map form *votes*, over
//! twenty-five coastline tiles, on two counts: which reading wins the most tiles
//! and which has the highest mean agreement. Both continents answer the same
//! way — **Azeroth north-west first by 18.8 points and 18 of 25 tiles, Kalimdor
//! by 9.7 and 12 of 25**.

use vale_assets::world::adt::{Adt, TILE_SIZE};
use vale_assets::tables::minimap::{self, MinimapTiles, MD5_TRANSLATE, ZOOM_LEVELS};
use vale_assets::world::wdt::Wdt;
use vale_assets::Assets;
use vale_config::Config;

use crate::common::open_assets;

/// Both halves of the census, and the tile trace.
pub fn cmd_minimap(cfg: &Config, map: Option<&str>, tile: Option<(u32, u32)>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let raw = assets
        .read(MD5_TRANSLATE)
        .map_err(|e| format!("{MD5_TRANSLATE}: {e}"))?;
    let index = MinimapTiles::parse(&raw);
    println!("{MD5_TRANSLATE}  ({} KB)", raw.len() / 1024);
    println!(
        "  {} tile picture(s) over {} map director(ies)",
        index.len(),
        index.directories().len()
    );
    println!(
        "  {} WMO picture(s) over {} WMO(s)",
        index.interior_len(),
        index.interior_stems().len()
    );
    match (map, tile) {
        (Some(map), Some((x, y))) => one_tile(&mut assets, &index, map, x, y),
        (Some("wmo"), None) => every_interior(&mut assets, &index),
        (Some(path), None) if path.to_ascii_lowercase().ends_with(".wmo") => {
            one_interior(&mut assets, &index, path)
        }
        (Some(map), None) => one_map(&mut assets, &index, map),
        _ => whole_index(&mut assets, &index),
    }
}

/// The archive path of the WMO a `.trs` stem names.
fn wmo_path_of(stem: &str) -> String {
    format!("World\\{stem}.wmo")
}

/// **Every WMO the index has pictures for, against the grid its groups give.**
///
/// For each WMO: the cells [`minimap::group_tiles`] produces for its groups,
/// against the cells the index names. A named cell the grid does not produce
/// is a fault in the grid rule, and is what this prints. A produced cell
/// with no picture is common (a cell over empty space is not baked) and is
/// only counted. It also counts which group flags carry pictures.
fn every_interior(assets: &mut Assets, index: &MinimapTiles) -> Result<(), String> {
    let mut wmos = 0usize;
    let mut named_total = 0usize;
    let mut stray_total = 0usize;
    let mut unresolved = 0usize;
    let mut missing_wmo = Vec::new();
    // Group-flag census over groups that carry at least one picture.
    let (mut pictured, mut pictured_exterior, mut pictured_unreachable) = (0usize, 0usize, 0usize);
    let mut exterior_groups = 0usize;
    let mut exterior_outdoor = 0usize;
    for stem in index.interior_stems() {
        let path = wmo_path_of(&stem);
        let Some(model) = minimap::InteriorModel::load(&path, |p| assets.read(p).ok()) else {
            missing_wmo.push(path);
            continue;
        };
        wmos += 1;
        let named = index.interior_cells(&stem);
        named_total += named.len();
        let mut produced = std::collections::BTreeSet::new();
        for (group, header) in model.groups.iter().enumerate() {
            let Some(header) = header else { continue };
            if header.flags & vale_assets::world::wmo::group_flags::EXTERIOR != 0 {
                exterior_groups += 1;
                if header.flags & vale_assets::world::wmo::group_flags::OUTDOOR != 0 {
                    exterior_outdoor += 1;
                }
            }
            for tile in minimap::group_tiles(&header.bounds) {
                produced.insert((group as u32, tile.x, tile.y));
            }
        }
        let mut strays = Vec::new();
        let mut groups_with_pictures = std::collections::BTreeSet::new();
        for cell in &named {
            groups_with_pictures.insert(cell.0);
            if !produced.contains(cell) {
                strays.push(*cell);
            }
            if index
                .interior_texture(&stem, cell.0, cell.1, cell.2)
                .is_none_or(|p| !assets.exists(&p))
            {
                unresolved += 1;
            }
        }
        for group in groups_with_pictures {
            pictured += 1;
            if let Some(Some(header)) = model.groups.get(group as usize) {
                if header.flags & vale_assets::world::wmo::group_flags::EXTERIOR != 0 {
                    pictured_exterior += 1;
                }
                if header.flags & vale_assets::world::wmo::group_flags::UNREACHABLE != 0 {
                    pictured_unreachable += 1;
                }
            }
        }
        stray_total += strays.len();
        if !strays.is_empty() {
            println!(
                "  ** {stem}: {} of {} named cell(s) are not on the grid, first {:?}",
                strays.len(),
                named.len(),
                strays[0]
            );
        }
    }
    println!("  {wmos} WMO(s) read, {} not in the archives", missing_wmo.len());
    for path in missing_wmo.iter().take(5) {
        println!("    missing {path}");
    }
    println!(
        "  {} of {named_total} named cell(s) are on the grid the groups give",
        named_total - stray_total
    );
    println!("  {} of {named_total} named picture(s) resolve", named_total - unresolved);
    println!(
        "  {pictured} group(s) carry pictures: {pictured_exterior} EXTERIOR (0x8), \
         {pictured_unreachable} UNREACHABLE (0x80)"
    );
    println!("  {exterior_groups} EXTERIOR group(s) in these WMOs, {exterior_outdoor} of them OUTDOOR (0x8000)");
    Ok(())
}

/// **One WMO**: its groups, their grids against the index, and the
/// orientation vote over every picture it has.
///
/// The orientation is measured as the terrain's is, against something the
/// same group states in another form: its floor. The group's upward-facing
/// triangles are drawn into the cell's texel grid, and the picture's opaque
/// texels are compared with that mask under four readings. The assumed one is
/// texel column 0 at the cell's least `x` and row 0 at its greatest `y`.
fn one_interior(assets: &mut Assets, index: &MinimapTiles, path: &str) -> Result<(), String> {
    let model = minimap::InteriorModel::load(path, |p| assets.read(p).ok())
        .ok_or_else(|| format!("{path} is not in the archives"))?;
    let stem = model.stem.clone();
    println!("{path}");
    println!("  stem {stem}, {} group(s), {} portal(s)", model.groups.len(), model.portals.portals.len());
    let named = index.interior_cells(&stem);
    println!("  {} picture(s) named", named.len());
    let mut wins = [0usize; 4];
    let mut totals = [0f32; 4];
    let mut voted = 0usize;
    for (group, header) in model.groups.iter().enumerate() {
        let Some(header) = header else {
            println!("    group {group:3}: ** not read");
            continue;
        };
        let tiles = minimap::group_tiles(&header.bounds);
        let have = tiles
            .iter()
            .filter(|t| index.interior_texture(&stem, group as u32, t.x, t.y).is_some())
            .count();
        let [min, max] = header.bounds;
        println!(
            "    group {group:3}: flags {:#07x}  {:6.1} x {:6.1}y  grid {} cell(s), {have} pictured",
            header.flags,
            max[0] - min[0],
            max[1] - min[1],
            tiles.len()
        );
        if have == 0 {
            continue;
        }
        let Ok(raw) = assets.read(&vale_assets::world::wmo::group_path(path, group as u32)) else {
            continue;
        };
        let Ok(geometry) = vale_assets::world::wmo::WmoGroup::parse(&raw) else {
            continue;
        };
        for tile in &tiles {
            let Some(picture) = index.interior_texture(&stem, group as u32, tile.x, tile.y) else {
                continue;
            };
            let Ok(raw) = assets.read(&picture) else { continue };
            let Ok(blp) = vale_assets::world::blp::decode(&raw) else { continue };
            let Some(scores) = score_interior(&blp, &geometry, tile) else {
                continue;
            };
            voted += 1;
            for (slot, score) in scores.iter().enumerate() {
                totals[slot] += score;
            }
            let best = (0..4).max_by(|a, b| scores[*a].total_cmp(&scores[*b])).unwrap_or(0);
            wins[best] += 1;
        }
    }
    if voted == 0 {
        println!("  no picture with enough floor and enough empty space to vote on");
        return Ok(());
    }
    println!("  orientation, voted over {voted} picture(s), opaque texels against the floor:");
    for (slot, name) in INTERIOR_READINGS.iter().enumerate() {
        println!(
            "    {name:<34} won {:>3}, {:.1}% mean agreement",
            wins[slot],
            totals[slot] / voted as f32 * 100.0
        );
    }
    Ok(())
}

/// The four readings [`score_interior`] scores, in its order.
const INTERIOR_READINGS: [&str; 4] = [
    "assumed (column +x, row -y)",
    "mirrored in x",
    "mirrored in y",
    "transposed (column +y, row -x)",
];

/// **How well a WMO picture's opaque texels agree with its group's floor**,
/// under four readings; `None` for a picture with no transparent texel or no
/// opaque one, or a cell whose floor mask is all one thing.
fn score_interior(
    blp: &vale_assets::world::blp::Blp,
    group: &vale_assets::world::wmo::WmoGroup,
    tile: &minimap::InteriorTile,
) -> Option<[f32; 4]> {
    const GRID: usize = 32;
    let (w, h) = (blp.width as usize, blp.height as usize);
    if w < GRID || h < GRID {
        return None;
    }
    // The picture, binned to GRID x GRID: a cell is opaque when most of its
    // texels are.
    let mut opaque = vec![0u32; GRID * GRID];
    for py in 0..h {
        for px in 0..w {
            if blp.rgba[(py * w + px) * 4 + 3] >= 128 {
                opaque[(py * GRID / h) * GRID + px * GRID / w] += 1;
            }
        }
    }
    let per_cell = (w / GRID * (h / GRID)) as u32;
    let picture: Vec<bool> = opaque.iter().map(|n| *n * 2 > per_cell).collect();
    let solid = picture.iter().filter(|b| **b).count();
    if solid == 0 || solid == picture.len() {
        return None;
    }
    // The floor, in the same grid over the cell's rectangle: every triangle
    // whose normal points up, sampled at the centre of each grid cell. `floor`
    // is indexed `[i][j]` with `i` along +x and `j` along +y.
    let [lo, hi] = tile.rect;
    let mut floor = vec![false; GRID * GRID];
    for tri in group.indices.chunks_exact(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| group.positions.get(usize::from(i)).copied());
        let (Some(a), Some(b), Some(c)) = (a, b, c) else { continue };
        let normal_z = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        let area = normal_z.abs();
        if area < 1e-6 {
            continue;
        }
        let tri_lo = [a[0].min(b[0]).min(c[0]), a[1].min(b[1]).min(c[1])];
        let tri_hi = [a[0].max(b[0]).max(c[0]), a[1].max(b[1]).max(c[1])];
        for i in 0..GRID {
            let x = lo[0] + (i as f32 + 0.5) / GRID as f32 * (hi[0] - lo[0]);
            if x < tri_lo[0] || x > tri_hi[0] {
                continue;
            }
            for j in 0..GRID {
                let y = lo[1] + (j as f32 + 0.5) / GRID as f32 * (hi[1] - lo[1]);
                if y < tri_lo[1] || y > tri_hi[1] {
                    continue;
                }
                let side = |p: [f32; 3], q: [f32; 3]| (q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0]);
                let (s1, s2, s3) = (side(a, b), side(b, c), side(c, a));
                if (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0) || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0) {
                    floor[i * GRID + j] = true;
                }
            }
        }
    }
    let covered = floor.iter().filter(|b| **b).count();
    if covered == 0 || covered == floor.len() {
        return None;
    }
    // Where floor cell (i, j) lands in the picture's (row, column) under each
    // reading. Assumed: column grows with x, row grows as y falls.
    let last = GRID - 1;
    let readings: [fn(usize, usize, usize) -> (usize, usize); 4] = [
        |i, j, last| (last - j, i),
        |i, j, last| (last - j, last - i),
        |i, j, _| (j, i),
        |i, j, last| (last - i, j),
    ];
    let mut scores = [0f32; 4];
    for (slot, reading) in readings.into_iter().enumerate() {
        let agree = (0..GRID)
            .flat_map(|i| (0..GRID).map(move |j| (i, j)))
            .filter(|(i, j)| {
                let (row, col) = reading(*i, *j, last);
                floor[i * GRID + j] == picture[row * GRID + col]
            })
            .count();
        scores[slot] = agree as f32 / (GRID * GRID) as f32;
    }
    Some(scores)
}

/// **How far the view reaches**, printed at every level of both of the client's
/// tables — see [`vale_assets::tables::minimap`].
fn zoom_table() {
    println!("  zoom radii:");
    println!("    level   outdoor      indoor");
    for zoom in 0..ZOOM_LEVELS {
        let out = minimap::radius_yards(zoom, false);
        let inside = minimap::radius_yards(zoom, true);
        let star = if zoom == minimap::DEFAULT_ZOOM { " <- minimapZoom default" } else { "" };
        println!("      {zoom}     {out:7.2}y    {inside:7.2}y{star}");
    }
    // The number that bounds the draw: the widest view is narrower than a tile,
    // so a frame is never more than four pictures.
    let widest = minimap::radius_yards(0, false) * 2.0;
    println!(
        "    widest view {widest:.2}y against a {TILE_SIZE:.2}y tile -> at most 2x2 pictures"
    );
}

/// Every directory the index names, and whether the pictures it names are there.
fn whole_index(assets: &mut Assets, index: &MinimapTiles) -> Result<(), String> {
    zoom_table();
    println!("  directories:");
    let mut missing_total = 0usize;
    let mut checked_total = 0usize;
    for directory in index.directories() {
        let tiles = index.tiles_of(&directory);
        // **Every referenced picture, not a sample.** The whole point of an
        // index keyed by MD5 is that a name cannot be checked by eye, and an
        // archive lookup is a hash — 20,000 of them is a second.
        let missing = tiles
            .iter()
            .filter(|(col, row)| {
                index
                    .texture(&directory, *col, *row)
                    .is_none_or(|path| assets.read(&path).is_err())
            })
            .count();
        missing_total += missing;
        checked_total += tiles.len();
        let note = if missing == 0 {
            String::new()
        } else {
            format!("  ** {missing} not in the archives")
        };
        println!("    {directory:<24} {:>5} picture(s){note}", tiles.len());
    }
    println!(
        "  {} of {checked_total} referenced picture(s) resolve",
        checked_total - missing_total
    );
    if missing_total > 0 {
        println!("  ** a referenced picture the chain cannot answer draws black");
    }
    Ok(())
}

/// One map: the index's tiles against the ones the `WDT` says the map has.
///
/// The two are **not** required to agree and the difference is the interesting
/// number. A tile with terrain and no picture is a black square on the minimap
/// (the client's own `MINIMAPCHUNKNOTFOUND`); a picture with no terrain is
/// harmless and common, because the packer draws a border of empty tiles.
fn one_map(assets: &mut Assets, index: &MinimapTiles, map: &str) -> Result<(), String> {
    let pictures = index.tiles_of(map);
    if pictures.is_empty() {
        return Err(format!(
            "no minimap pictures for {map:?} — the index names: {}",
            index.directories().join(", ")
        ));
    }
    let wdt = assets
        .read(&format!(r"World\Maps\{map}\{map}.wdt"))
        .map_err(|e| e.to_string())
        .and_then(|raw| Wdt::parse(&raw).map_err(|e| e.to_string()))?;
    let terrain: Vec<(u32, u32)> = wdt.existing_tiles();
    let have: std::collections::BTreeSet<(u32, u32)> = pictures.iter().copied().collect();
    let blank: Vec<(u32, u32)> = terrain
        .iter()
        .copied()
        .filter(|tile| !have.contains(tile))
        .collect();
    println!("map {map}:");
    println!("  {} picture(s), {} tile(s) with terrain", pictures.len(), terrain.len());
    println!(
        "  {} of {} terrain tile(s) have a picture",
        terrain.len() - blank.len(),
        terrain.len()
    );
    if let Some(first) = blank.first() {
        println!(
            "  {} would draw black; first is {:?}",
            blank.len(),
            first
        );
    }
    // **The orientation check, over every tile with a coastline on it.**
    //
    // One tile is an anecdote — see [`orientation`], which says why. Run across
    // a map it is a vote, and the four readings separate: the right one wins on
    // most tiles and by a wide average, and a wrong one wins on a scattering of
    // tiles whose water is dark or whose land is blue-grey rock.
    let mut wins = [0usize; 4];
    let mut totals = [0f32; 4];
    let mut voted = 0usize;
    for tile in terrain.iter().filter(|tile| have.contains(tile)) {
        if voted >= WATER_SCAN {
            break;
        }
        let Ok(raw) = assets.read(&adt_path(map, tile.0, tile.1)) else {
            continue;
        };
        let Ok(adt) = Adt::parse(&raw) else { continue };
        let wet = wet_chunks(&adt).iter().filter(|w| **w).count();
        // A tile that is nearly all one thing agrees with every reading, and a
        // tile whose water is a stream cannot separate them at all.
        if !(48..=208).contains(&wet) {
            continue;
        }
        let Some(path) = index.texture(map, tile.0, tile.1) else {
            continue;
        };
        let Ok(picture) = assets.read(&path) else { continue };
        let Ok(blp) = vale_assets::world::blp::decode(&picture) else {
            continue;
        };
        let Some(scores) = score_readings(&blp, &adt) else {
            continue;
        };
        voted += 1;
        for (slot, score) in scores.iter().enumerate() {
            totals[slot] += score;
        }
        let best = (0..4).max_by(|a, b| scores[*a].total_cmp(&scores[*b])).unwrap_or(0);
        wins[best] += 1;
    }
    if voted == 0 {
        println!("  no tile with enough coastline to check the orientation against");
        return Ok(());
    }
    println!("  orientation, voted over {voted} tile(s) with a coastline:");
    for (slot, name) in READING_NAMES.iter().enumerate() {
        println!(
            "    {name:<26} won {:>3} tile(s), {:.1}% mean agreement",
            wins[slot],
            totals[slot] / voted as f32 * 100.0
        );
    }
    // **Both counts, and neither alone.** The mean is the stronger statistic —
    // it uses every chunk of every tile — but it can be dragged by one lopsided
    // tile; the win count is robust and coarse. A reading that leads on both is
    // the answer, and one that leads on only one is a check that has not
    // separated them, which is a different thing from being wrong.
    let assumed_mean = totals[0] / voted as f32;
    let best_wrong = totals[1..].iter().map(|t| t / voted as f32).fold(0.0f32, f32::max);
    let most_wins = wins.iter().skip(1).all(|w| wins[0] > *w);
    if most_wins && assumed_mean > best_wrong {
        println!(
            "  north-west first, on both counts — {:.1} points of mean agreement clear",
            (assumed_mean - best_wrong) * 100.0
        );
    } else if most_wins || assumed_mean > best_wrong {
        println!("  north-west first leads on one count and not the other — not separated here");
    } else {
        println!("  ** the assumed reading loses on both counts — the mapping is backwards");
    }
    Ok(())
}

/// How many coastline tiles to vote over. Each is an ADT parse and a BLP
/// decode; twenty-five is a second and is already a landslide.
const WATER_SCAN: usize = 25;

/// The four ways the same picture can be read, in the order [`score_readings`]
/// returns them.
const READING_NAMES: [&str; 4] = [
    "assumed (north-west first)",
    "mirrored east-west",
    "mirrored north-south",
    "transposed",
];

/// One tile: its picture, and the orientation check — see the module comment.
fn one_tile(
    assets: &mut Assets,
    index: &MinimapTiles,
    map: &str,
    x: u32,
    y: u32,
) -> Result<(), String> {
    let path = index
        .texture(map, x, y)
        .ok_or_else(|| format!("the index has no picture for {map} ({x}, {y})"))?;
    println!("{map} ({x}, {y}) -> {path}");
    let raw = assets.read(&path).map_err(|e| e.to_string())?;
    println!("  {}", crate::common::describe_blp(&raw));
    let blp = vale_assets::world::blp::decode(&raw).map_err(|e| e.to_string())?;
    println!("  decoded {}x{}", blp.width, blp.height);

    // Where this tile lands when the character is standing in the middle of it,
    // which is the one geometric round trip worth printing.
    let centre = vale_assets::world::terrain::tile_centre(x, y);
    let radius = minimap::radius_yards(minimap::DEFAULT_ZOOM, false);
    let view = minimap::tiles_in_view(centre[0], centre[1], radius);
    println!(
        "  standing at its centre at zoom {} ({radius:.1}y): {} picture(s) in view",
        minimap::DEFAULT_ZOOM,
        view.len()
    );
    for placed in &view {
        let [l, t, r, b] = placed.rect;
        println!(
            "    ({}, {}) at {l:+.3}..{r:+.3} x {t:+.3}..{b:+.3} of the widget",
            placed.tile.0, placed.tile.1
        );
    }

    let adt_raw = assets
        .read(&adt_path(map, x, y))
        .map_err(|e| format!("{}: {e}", adt_path(map, x, y)))?;
    let adt = Adt::parse(&adt_raw).map_err(|e| e.to_string())?;
    orientation(&blp, &adt);
    Ok(())
}

/// The 16x16 grid of "this chunk carries a liquid", in the ADT's own chunk
/// order: index `row * 16 + col`, from the tile's north-west corner.
fn wet_chunks(adt: &Adt) -> Vec<bool> {
    let mut wet = vec![false; 256];
    for (index, chunk) in adt.chunks.iter().enumerate().take(256) {
        wet[index] = !chunk.liquids.is_empty();
    }
    wet
}

/// **Which way up the picture is**, correlated against the tile's own `MCLQ`.
///
/// The score is the fraction of the 256 chunks where "the picture is blue here"
/// agrees with "`MCLQ` says there is water here". A perfect agreement is not
/// expected and is not the point — a shore is a gradient and a deep lake under
/// a cliff is dark rather than blue. What is expected is a **gap**: the right
/// orientation should beat all three wrong ones, because a mirrored coastline
/// lands its water on the tile's dry half.
fn orientation(blp: &vale_assets::world::blp::Blp, adt: &Adt) {
    let wet_count = wet_chunks(adt).iter().filter(|w| **w).count();
    println!("  MCLQ: {wet_count} of 256 chunks carry a liquid");
    let Some(scored) = score_readings(blp, adt) else {
        println!("  ** no orientation check: the tile is all one thing, or the picture is too small");
        return;
    };
    println!("  picture water against MCLQ, four readings of the same file:");
    for (name, score) in READING_NAMES.iter().zip(scored.iter()) {
        println!("    {name:<26} {:.1}% agree", score * 100.0);
    }
    let assumed = scored[0];
    let best_wrong = scored[1..].iter().copied().fold(0.0f32, f32::max);
    let margin = (assumed - best_wrong) * 100.0;
    // **A margin under ten points is no answer**, and saying so is the whole
    // difference between a check and a coin toss. A tile whose water is a
    // stream — Elwynn's 42 wet chunks of 256 — cannot separate the four
    // readings, because a chunk that is one-eighth river averages no bluer than
    // a chunk of shaded forest, and the four scores land within six points of
    // each other in whatever order the noise puts them. **A single tile is an
    // anecdote either way**; `vale minimap <Map>` votes over twenty-five of
    // them and is the form that settles it.
    const DECISIVE: f32 = 10.0;
    if margin >= DECISIVE {
        println!("  this tile says north-west first, by {margin:.1} points");
    } else if margin <= -DECISIVE {
        println!("  ** this tile says a wrong reading, by {:.1} points", -margin);
    } else {
        println!(
            "  this tile says nothing: {margin:+.1} points between the best two. \
             Run `vale minimap <Map>` for the vote"
        );
    }
}

/// **How well each of the four readings of a picture agrees with the tile's own
/// `MCLQ`** — the arithmetic behind [`orientation`], shared with the vote.
///
/// `None` for a tile that is all water or all land, or a picture too small to
/// bin, both of which agree with everything.
fn score_readings(blp: &vale_assets::world::blp::Blp, adt: &Adt) -> Option<[f32; 4]> {
    let wet = wet_chunks(adt);
    let wet_count = wet.iter().filter(|w| **w).count();
    if wet_count == 0 || wet_count == 256 {
        return None;
    }
    let (w, h) = (blp.width as usize, blp.height as usize);
    if w < 16 || h < 16 {
        return None;
    }
    // **How blue a chunk is, ranked rather than thresholded.** Minimap water
    // runs from a near-black deep ocean to a pale shallow, and no fixed cut
    // separates it from wet rock — but *within one tile* the wettest chunks are
    // reliably the bluest. So the score is `blue - (red + green) / 2` averaged
    // over the chunk's texels, and the prediction is "the N bluest chunks are
    // the wet ones", with N taken from `MCLQ` itself. Nothing to tune, and the
    // calibration is the same for all four readings, so it cannot favour one.
    let mut score = vec![0f32; 256];
    let mut total = vec![0f32; 256];
    for py in 0..h {
        for px in 0..w {
            let cell = (py * 16 / h) * 16 + (px * 16 / w);
            let texel = &blp.rgba[(py * w + px) * 4..][..4];
            let blueness = f32::from(texel[2]) - (f32::from(texel[0]) + f32::from(texel[1])) / 2.0;
            score[cell] += blueness;
            total[cell] += 1.0;
        }
    }
    let mut ranked: Vec<(usize, f32)> = (0..256)
        .map(|cell| (cell, score[cell] / total[cell].max(1.0)))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut wet_texels = vec![false; 256];
    for (cell, _) in ranked.iter().take(wet_count) {
        wet_texels[*cell] = true;
    }
    // The four readings of the same picture: where the assumed `(row, col)`
    // lands in each. Same order as [`READING_NAMES`].
    let readings: [fn(usize, usize) -> usize; 4] = [
        |row, col| row * 16 + col,
        |row, col| row * 16 + (15 - col),
        |row, col| (15 - row) * 16 + col,
        |row, col| col * 16 + row,
    ];
    let mut scored = [0f32; 4];
    for (slot, map_cell) in readings.into_iter().enumerate() {
        let agree = (0..16)
            .flat_map(|row| (0..16).map(move |col| (row, col)))
            .filter(|(row, col)| wet[row * 16 + col] == wet_texels[map_cell(*row, *col)])
            .count();
        scored[slot] = agree as f32 / 256.0;
    }
    Some(scored)
}

fn adt_path(map: &str, x: u32, y: u32) -> String {
    format!(r"World\Maps\{map}\{map}_{x}_{y}.adt")
}
