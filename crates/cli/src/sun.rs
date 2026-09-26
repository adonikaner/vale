//! `vale sun` — where the sun stands, from a tile's baked shadows.

use vale_config::Config;

/// `vale sun <Map> <x> <y>` — the sun's direction, measured from the
/// shadows Blizzard baked.
///
/// `Light.dbc` states the sun's colour at every hour and its direction at none,
/// and the renderer carried a guessed afternoon angle from the WebGL days. But
/// the direction is not actually unstated: every chunk's `MCSH` is the shadow
/// the game's tools baked for a *specific* sun, so the offset between a tall
/// doodad and the shadow it casts is the sun's azimuth, and its height against
/// the shadow's reach is the elevation.
///
/// The instrument is a cross-correlation rather than any per-tree pairing,
/// because the bakes defeat pairing twice over: a forest's shadows merge into
/// one blob, and a dead tree's shadow is lace that flood-fills into dozens of
/// crumbs (Westfall: 2,726 blobs averaging ten texels). So instead: propose a
/// sun — a shadow direction and a reach-per-yard-of-height slope — predict
/// where every tall doodad's canopy shadow would fall under it, and score the
/// proposal by how much of the predicted area actually is in shadow. The
/// baked sun is the proposal that wins, the score against the tile's base
/// shadow rate says how decisively, and the score at the opposite azimuth is
/// the control. The elevation is `atan(1 / slope)` and leans on the canopy
/// sitting where the model's vertices say — so the azimuth is the trustworthy
/// half and the output says so.
///
/// [`vale_assets::tables::light::SUN_TOWARD`] is the constant the renderer aims its
/// light by; this prints the measurement beside it so drift is visible.
pub fn cmd_sun(cfg: &Config, map: &str, x: u32, y: u32) -> Result<(), String> {
    use vale_assets::world::adt::{placement_to_world, Adt, ALPHA_SIDE, CHUNKS_PER_SIDE, CHUNK_SIZE};
    use vale_assets::world::m2::M2;
    use std::collections::BTreeMap;

    let mut assets = crate::common::open_assets(cfg)?;
    let raw = assets
        .read(&vale_assets::adt_path(map, x, y))
        .map_err(|e| e.to_string())?;
    let adt = Adt::parse(&raw).map_err(|e| e.to_string())?;

    // The tile's shadow as one bitmap, in the atlas's own cell layout — a
    // cell's neighbour in the atlas is its neighbour in the world. Texel rows
    // follow the chunk's v (decreasing world x) and columns its u (decreasing
    // world y), the layout `alpha_atlas` assumes and the seam-continuity check
    // in `vale textures` pins.
    let texel = CHUNK_SIZE / ALPHA_SIDE as f32;
    let side = CHUNKS_PER_SIDE * ALPHA_SIDE;
    let mut in_shadow = vec![false; side * side];
    let mut shadowed = 0usize;
    for (i, chunk) in adt.chunks.iter().enumerate() {
        if i >= CHUNKS_PER_SIDE * CHUNKS_PER_SIDE {
            break;
        }
        let (cell_x, cell_y) = (i % CHUNKS_PER_SIDE, i / CHUNKS_PER_SIDE);
        for t in 0..ALPHA_SIDE * ALPHA_SIDE {
            let (tx, ty) = (t % ALPHA_SIDE, t / ALPHA_SIDE);
            if chunk.shadow.get(t).copied().unwrap_or(0) != 0 {
                in_shadow[(cell_y * ALPHA_SIDE + ty) * side + cell_x * ALPHA_SIDE + tx] = true;
                shadowed += 1;
            }
        }
    }
    let base_rate = shadowed as f64 / (side * side) as f64;
    println!("{}", vale_assets::adt_path(map, x, y));
    println!(
        "  {} of {} texels in shadow ({:.1}%)",
        shadowed,
        side * side,
        100.0 * base_rate as f32
    );

    // World -> bitmap. The chunk origins are one uniform grid, so the whole
    // tile is two corner values and a texel size.
    let x0 = adt.chunks.iter().map(|c| c.position[0]).fold(f32::MIN, f32::max);
    let y0 = adt.chunks.iter().map(|c| c.position[1]).fold(f32::MIN, f32::max);

    // What each distinct model is shaped like: canopy top, canopy centre, and
    // horizontal radius, all in model space (+Z up), scaled per placement
    // below. **From the drawn vertices, not the declared bounding box** — the
    // box includes animation swing, which for a tree is ~3x the canopy and for
    // a flying bird is its whole 60-yard flight path, and a "canopy" that big
    // matches every blob on the tile.
    let doodads = adt.placed_doodads();
    let mut shapes: BTreeMap<&str, (f32, f32, f32)> = BTreeMap::new();
    for d in &doodads {
        if shapes.contains_key(d.path.as_str()) {
            continue;
        }
        let Ok(bytes) = assets.read(&d.path) else { continue };
        let Ok(m) = M2::parse(&bytes) else { continue };
        if m.positions.is_empty() {
            continue;
        }
        let (mut top, mut r_xy, mut z_sum) = (f32::MIN, 0.0f32, 0.0f64);
        for p in &m.positions {
            top = top.max(p[2]);
            r_xy = r_xy.max(p[0].hypot(p[1]));
            z_sum += p[2] as f64;
        }
        let centre = (z_sum / m.positions.len() as f64) as f32;
        shapes.insert(d.path.as_str(), (top, centre, r_xy));
    }

    // A building is an exclusion zone: its own shadow reaches its height times
    // the same worst-case factor a doodad's does.
    const REACH: f32 = 2.2; // shadow reach per yard of height — covers elevation down to ~24 deg
    let boxes: Vec<([f32; 2], [f32; 2])> = adt
        .wmos
        .iter()
        .map(|w| {
            let a = placement_to_world(w.bounds_lower);
            let b = placement_to_world(w.bounds_upper);
            let margin = REACH * (a[2] - b[2]).abs();
            (
                [a[0].min(b[0]) - margin, a[1].min(b[1]) - margin],
                [a[0].max(b[0]) + margin, a[1].max(b[1]) + margin],
            )
        })
        .collect();

    // The casters: every tall doodad standing clear of any building's possible
    // shadow, with the canopy centre height that sets its shadow's offset.
    struct Tree {
        x: f32,
        y: f32,
        centre: f32,
        r: f32,
    }
    let mut trees: Vec<Tree> = Vec::new();
    for d in &doodads {
        let Some(&(top, centre, r_xy)) = shapes.get(d.path.as_str()) else {
            continue;
        };
        let (top, centre, r_xy) = (top * d.scale, centre * d.scale, r_xy * d.scale);
        if top < 8.0 {
            continue;
        }
        let (px, py) = (d.position[0], d.position[1]);
        if boxes
            .iter()
            .any(|(lo, hi)| px > lo[0] && px < hi[0] && py > lo[1] && py < hi[1])
        {
            continue;
        }
        trees.push(Tree { x: px, y: py, centre, r: r_xy.clamp(1.5, 12.0) });
    }
    println!(
        "  {} tall doodads standing clear of the {} buildings' shadow zones",
        trees.len(),
        adt.wmos.len()
    );
    if trees.len() < 5 {
        println!("  too few casters to correlate — try another tile");
        return Ok(());
    }

    // How much of the shadow predicted by one proposed sun is really there.
    // `az` is the direction shadows are displaced (away from the sun) and `k`
    // the displacement per yard of canopy-centre height, i.e. 1/tan(elevation).
    let score = |az: f32, k: f32| -> f64 {
        let (dx, dy) = (az.cos(), az.sin());
        let (mut hit, mut total) = (0u64, 0u64);
        for t in &trees {
            let (cx, cy) = (t.x + dx * t.centre * k, t.y + dy * t.centre * k);
            let r = t.r;
            let lo = |c: f32, origin: f32| (((origin - c - r) / texel) - 0.5).ceil().max(0.0);
            let hi = |c: f32, origin: f32| {
                ((((origin - c + r) / texel) - 0.5).floor()).min((side - 1) as f32)
            };
            let (gy0, gy1) = (lo(cx, x0), hi(cx, x0));
            let (gx0, gx1) = (lo(cy, y0), hi(cy, y0));
            if gy0 > gy1 || gx0 > gx1 {
                continue;
            }
            for gy in gy0 as usize..=gy1 as usize {
                let px = x0 - (gy as f32 + 0.5) * texel;
                for gx in gx0 as usize..=gx1 as usize {
                    let py = y0 - (gx as f32 + 0.5) * texel;
                    if (px - cx).hypot(py - cy) <= r {
                        total += 1;
                        hit += u64::from(in_shadow[gy * side + gx]);
                    }
                }
            }
        }
        if total == 0 {
            return 0.0;
        }
        hit as f64 / total as f64
    };

    // Coarse sweep, then a fine one around the winner.
    let mut best = (f64::MIN, 0.0f32, 1.0f32);
    for az_deg in (0..360).step_by(4) {
        for k10 in 4..=30 {
            let (az, k) = ((az_deg as f32).to_radians(), k10 as f32 / 10.0);
            let s = score(az, k);
            if s > best.0 {
                best = (s, az, k);
            }
        }
    }
    let coarse = best;
    for fine_az in -8..=8 {
        for fine_k in -4..=4 {
            let az = coarse.1 + (fine_az as f32 * 0.5).to_radians();
            let k = coarse.2 + fine_k as f32 * 0.025;
            let s = score(az, k);
            if s > best.0 {
                best = (s, az, k);
            }
        }
    }

    let (precision, shadow_az, k) = best;
    let elevation = (1.0 / k).atan();
    let sun_az = shadow_az + std::f32::consts::PI;
    let opposite = score(sun_az, k);
    let deg = |r: f32| r.to_degrees();
    println!(
        "  best fit: shadows displaced {:.1} deg at {:.2} yards per yard of height",
        deg(shadow_az).rem_euclid(360.0),
        k
    );
    println!(
        "    {:.0}% of the predicted shadow is really there, against {:.0}% for the opposite \
         azimuth and a base rate of {:.0}%",
        100.0 * precision,
        100.0 * opposite,
        100.0 * base_rate
    );
    if precision - opposite < 0.05 || precision - base_rate < 0.05 {
        println!(
            "    !! no discrimination — the shadow here is a blanket, and this tile cannot say"
        );
    }
    println!(
        "  so the sun stands at azimuth {:.1} deg, elevation {:.1} deg \
         (azimuth is the trustworthy half; the elevation leans on canopy heights)",
        deg(sun_az).rem_euclid(360.0),
        deg(elevation),
    );
    println!("    (compass is the world's own axes: 0 deg = +X north, 90 deg = +Y west)");

    let toward = [
        elevation.cos() * sun_az.cos(),
        elevation.cos() * sun_az.sin(),
        elevation.sin(),
    ];
    let held = vale_assets::tables::light::SUN_TOWARD;
    let dot = toward
        .iter()
        .zip(held)
        .map(|(a, b)| a * b)
        .sum::<f32>()
        .clamp(-1.0, 1.0);
    println!(
        "  toward the sun: [{:+.3}, {:+.3}, {:+.3}]   SUN_TOWARD holds [{:+.3}, {:+.3}, {:+.3}] \
         — {:.1} deg apart",
        toward[0],
        toward[1],
        toward[2],
        held[0],
        held[1],
        held[2],
        deg(dot.acos()),
    );
    Ok(())
}
