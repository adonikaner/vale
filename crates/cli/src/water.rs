//! `vale water` — MLIQ and MCLQ: the liquid standing in a tile.

use crate::common::*;
use vale_assets::{world::adt::Adt, adt_path};
use vale_config::Config;

/// The water, lava and slime standing inside a tile's buildings: `MLIQ`.
///
/// Two things here are not stated in the file and had to be pinned against
/// something else, which is what this command exists to report.
///
/// **The tile size.** `MLIQ` gives a corner and a tile count and no spacing at
/// all, so the grid's extent depends entirely on a constant this project
/// supplies. `LIQUID_TILE_SIZE` is the game's `CHUNKSIZE / 8`, and the check is
/// that every grid then sits inside the bounding box of the group that owns it
/// — a box Blizzard wrote. Double the constant and a font in a chapel spills
/// through its walls; halve it and the moat is a puddle in one corner. Both look
/// like plausible water.
///
/// **Which liquid it is.** The raw `MOGP` type is not the answer — it goes
/// through a four-step fixup (see `WmoGroup::liquid_kind`) in which the
/// water/ocean distinction is carried by a *group flag* and not by the type at
/// all. Nothing in the file cross-checks that, so what is reported instead is
/// the distribution and the textures it resolves to, against the archive.
pub fn cmd_water(cfg: &Config, map: &str, x: u32, y: u32) -> Result<(), String> {
    use vale_assets::world::adt::{placement_matrix, placement_to_world};
    use vale_assets::world::collision::transform_point;
    use vale_assets::world::wmo::{self, Liquid, LIQUID_TILE_SIZE};
    use std::collections::{BTreeMap, BTreeSet};

    let mut assets = open_assets(cfg)?;
    let raw = assets.read(&adt_path(map, x, y)).map_err(|e| e.to_string())?;
    let adt = Adt::parse(&raw).map_err(|e| e.to_string())?;

    println!("{}", adt_path(map, x, y));
    println!("  liquid tile size {LIQUID_TILE_SIZE:.4}y (TILE_SIZE / 16 / 8)");

    let distinct: BTreeSet<String> = adt
        .wmos
        .iter()
        .filter_map(|w| adt.wmo_names.get(w.name_id as usize).cloned())
        .collect();

    let mut groups = 0usize;
    let mut wet_groups = 0usize;
    let mut tiles = 0usize;
    let mut wet = 0usize;
    let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut unresolved = 0usize;
    // The measurement: how far a liquid grid sticks out of the box of the group
    // that owns it, per axis, in yards.
    let mut worst_overhang = f32::MIN;
    let mut worst_named = String::new();
    let mut inside = 0usize;
    let mut rows: Vec<String> = Vec::new();
    // How far each grid's corner sits from a whole number of tiles.
    let mut worst_remainder = 0.0f32;
    let mut aligned_axes = 0usize;

    for path in &distinct {
        let Ok(bytes) = assets.read(path) else { continue };
        let Ok(root) = wmo::WmoRoot::parse(&bytes) else {
            continue;
        };
        for index in 0..root.group_count {
            let parsed = assets
                .read(&wmo::group_path(path, index))
                .and_then(|b| wmo::WmoGroup::parse(&b));
            let Ok(group) = parsed else { continue };
            groups += 1;
            let Some(liquid) = group.liquid.as_ref() else {
                continue;
            };
            wet_groups += 1;
            for axis in 0..2 {
                let steps = liquid.base[axis] / LIQUID_TILE_SIZE;
                worst_remainder = worst_remainder.max((steps - steps.round()).abs());
                aligned_axes += 1;
            }
            tiles += liquid.x_tiles * liquid.y_tiles;
            wet += liquid.wet_tiles();
            match group.liquid_kind(&root) {
                Some(kind) => *kinds.entry(kind_name(kind)).or_default() += 1,
                None => unresolved += 1,
            }

            // Containment. Only the wet vertices: an empty tile's height is
            // whatever the tool left there, and a grid that covers a whole room
            // is mostly empty (see the wet/total ratio above).
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            let mut any = false;
            for ty in 0..liquid.y_tiles {
                for tx in 0..liquid.x_tiles {
                    if !liquid.has_liquid(tx, ty) {
                        continue;
                    }
                    any = true;
                    for (cx, cy) in [(tx, ty), (tx + 1, ty), (tx, ty + 1), (tx + 1, ty + 1)] {
                        let v = liquid.vertex(cx, cy);
                        for axis in 0..3 {
                            lo[axis] = lo[axis].min(v[axis]);
                            hi[axis] = hi[axis].max(v[axis]);
                        }
                    }
                }
            }
            if !any {
                continue;
            }
            let overhang = (0..3)
                .map(|a| (group.bounds[0][a] - lo[a]).max(hi[a] - group.bounds[1][a]))
                .fold(f32::MIN, f32::max);
            if overhang <= 0.5 {
                inside += 1;
            }
            if overhang > worst_overhang {
                worst_overhang = overhang;
                worst_named = format!(
                    "{} group {index} ({}x{})",
                    path.rsplit('\\').next().unwrap_or(path),
                    liquid.x_tiles,
                    liquid.y_tiles
                );
            }
            if std::env::var("WATER_DEBUG").is_ok() {
                println!(
                    "      g{index:3} {:2}x{:<2}  grid {:8.2?}..{:8.2?}
                                 box  {:8.2?}..{:8.2?}",
                    liquid.x_tiles, liquid.y_tiles, lo, hi, group.bounds[0], group.bounds[1]
                );
            }
            if rows.len() < 8 {
                rows.push(format!(
                    "    {:.<40} group {index:3}  {:2}x{:<2} tiles, {:4} wet, {:?}, z {:.1}..{:.1}",
                    path.rsplit('\\').next().unwrap_or(path),
                    liquid.x_tiles,
                    liquid.y_tiles,
                    liquid.wet_tiles(),
                    group.liquid_kind(&root),
                    lo[2],
                    hi[2],
                ));
            }
        }
    }

    println!("  {wet_groups} of {groups} groups carry MLIQ, {tiles} tiles of which {wet} are wet");

    // The check the tile size lives or dies by.
    //
    // Containment in the owning group's box is the obvious one and it is *not*
    // conclusive: a canal's water plane is authored to run under the wall of the
    // room it flows into, so a small overhang is correct and a grid that falls
    // short of the room is correct too. What is conclusive is that every grid's
    // corner is an exact multiple of the tile size — the tool that wrote these
    // files snapped the base to its own grid, so a wrong constant leaves a
    // remainder on every single one. Nothing else in the file states the
    // spacing, and this states it to five figures.
    //
    // **No early return when the buildings are dry**: a tile of open ocean has
    // no wet WMO and everything below this section — the terrain's own `MCLQ`,
    // the light ramp, the textures — is exactly what such a tile is asked about.
    if wet_groups == 0 {
        println!("  (no building on this tile holds liquid)");
    } else {
        println!("  liquids: {kinds:?}{}", if unresolved > 0 {
            format!(", {unresolved} that resolve to none")
        } else {
            String::new()
        });
        for row in &rows {
            println!("{row}");
        }
        println!("  grid alignment — every MLIQ corner is a whole number of tiles from the origin:");
        println!(
            "    worst remainder {:.4} of a tile over {aligned_axes} corners ({} grids x 2 axes)",
            worst_remainder, wet_groups
        );
        println!("    <- a constant wrong by 1% leaves ~0.2 here on a grid 20 tiles out");
        println!("  containment in the owning group's own MOGP box, for context:");
        println!("    {inside} of {wet_groups} grids sit inside it to within 0.5y");
        println!("    worst overhang {worst_overhang:+.2}y  {worst_named}");
        println!("    (an overhang is not an error: water runs under the wall it flows through)");
    }

    // And the textures the client's table names, against the archive.
    // And what the renderer will actually be handed: the surface `assemble`
    // generates for each of those grids, in the same buffers and the same draw
    // list as the masonry. Reported here because the geometry is *generated*
    // rather than read — nothing in the file can be compared against it — so the
    // only checks available are that it exists, that it indexes inside the
    // buffers it was appended to, and that its triangle count is two per wet
    // tile and not two per tile.
    let mut liquid_draws = 0usize;
    let mut liquid_triangles = 0usize;
    let mut out_of_range = 0usize;
    let mut liquid_textures: BTreeSet<String> = BTreeSet::new();
    // The opacity the surface actually carries, which is the number that decides
    // whether the water is visible at all.
    let mut opacity_total = 0u64;
    let mut opacity_count = 0usize;
    let mut opacity_min = u8::MAX;
    let mut opacity_max = u8::MIN;
    // Kept so the world-space pass below can transform the same surface the
    // renderer is handed, rather than re-deriving one from the grids.
    let mut models: BTreeMap<String, wmo::WmoModel> = BTreeMap::new();
    for path in &distinct {
        let Ok((model, _)) = wmo::load(&mut assets, path) else {
            continue;
        };
        let vertices = model.positions.len();
        for draw in &model.draws {
            let Some(name) = draw.texture.and_then(|t| model.textures.get(t as usize)) else {
                continue;
            };
            if !name.starts_with("XTextures") {
                continue;
            }
            liquid_draws += 1;
            liquid_triangles += draw.index_count as usize / 3;
            liquid_textures.insert(name.clone());
            let range = draw.index_start as usize
                ..(draw.index_start + draw.index_count) as usize;
            // A draw whose range does not exist at all counts as out of range
            // too, which an empty slice would silently pass.
            let indices = model.indices.get(range);
            if !indices.is_some_and(|s| s.iter().all(|&i| (i as usize) < vertices)) {
                out_of_range += 1;
            }
            let indices = indices.unwrap_or(&[]);
            if draw.liquid.is_none() {
                continue;
            }
            for &index in indices {
                let Some(colour) = model.colours.get(index as usize) else {
                    continue;
                };
                opacity_total += colour[3] as u64;
                opacity_count += 1;
                opacity_min = opacity_min.min(colour[3]);
                opacity_max = opacity_max.max(colour[3]);
            }
        }
        models.insert(path.clone(), model);
    }
    println!("  assembled surface: {liquid_draws} draws, {liquid_triangles} triangles");
    println!(
        "    {} = 2 per wet tile{}",
        if liquid_triangles == wet * 2 { "which is exactly" } else { "which is NOT" },
        if out_of_range > 0 {
            format!(", and {out_of_range} draws index outside the buffers")
        } else {
            String::new()
        }
    );
    println!("    textures appended to the models: {liquid_textures:?}");

    // **The opacity, and where it comes from.** Every other draw in a WMO takes
    // its alpha from the texel. A liquid draw must not: `lake_a`'s alpha channel
    // is a foam and highlight mask (reported below — mean 54 of 255, three
    // quarters under 64), and a canal blended by it is invisible, which is
    // exactly what Stormwind's were. The opacity is `MLIQ`'s own per-vertex
    // depth byte instead, which is 0 at every dry vertex and rises into the
    // pool. Nothing states the mapping, so what is checkable is that the surface
    // carries a spread of real values rather than one flat number or none.
    if opacity_count > 0 {
        println!(
            "    opacity from MLIQ depth, not the texture: {opacity_min}..{opacity_max} mean {} \
             over {opacity_count} surface vertices",
            opacity_total / opacity_count as u64
        );
    }

    // **The renderer's own admission test.** `client::wmos::batch_draw` drops any
    // draw whose indices reach past the *shortest* of the four vertex buffers,
    // silently, because a batch pointing outside its buffers is a damaged file
    // rather than an error worth taking a building down for. That guard is only
    // as good as the invariant it assumes: the liquid surface pushes a position,
    // a normal, a uv and a colour per vertex, and if any one of the four fell
    // behind, every draw appended after it — the water first — would vanish with
    // no message anywhere. So the lengths are asserted here rather than trusted.
    let ragged: Vec<String> = models
        .iter()
        .filter(|(_, m)| {
            m.normals.len() != m.positions.len()
                || m.uvs.len() != m.positions.len()
                || m.colours.len() != m.positions.len()
        })
        .map(|(path, m)| {
            format!(
                "{}: {} positions, {} normals, {} uvs, {} colours",
                path.rsplit('\\').next().unwrap_or(path),
                m.positions.len(),
                m.normals.len(),
                m.uvs.len(),
                m.colours.len()
            )
        })
        .collect();
    if ragged.is_empty() {
        println!("    all four vertex buffers stay parallel — nothing the renderer would drop");
    } else {
        for row in &ragged {
            println!("    !! ragged buffers, the renderer will drop draws: {row}");
        }
    }

    // Where the surface actually ends up once its building is placed.
    //
    // Every check above this line is in the *model's* own space, which is where
    // a wrong tile size or a mis-read flag byte shows — and none of it says the
    // water is at the height the canal wants it. This is the one number a person
    // standing in the city can compare against: the HUD prints their z, and the
    // canal water they are looking for is a few yards below the street.
    //
    // The transform is the placement's own matrix — the same one the walls go
    // through, which `vale wmos` already lands on the game's `MODF` box to
    // 0.00 yards — so a disagreement here is about the *liquid*, not the
    // building.
    println!("  in world space, through each MODF placement:");
    let mut placements_with_water = 0usize;
    for placement in &adt.wmos {
        let Some(name) = adt.wmo_names.get(placement.name_id as usize) else {
            continue;
        };
        let Some(model) = models.get(name) else { continue };
        let position = placement_to_world(placement.position);
        let matrix = placement_matrix(position, placement.rotation, 1.0);

        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        let mut draws = 0usize;
        for draw in &model.draws {
            let Some(texture) = draw.texture.and_then(|t| model.textures.get(t as usize)) else {
                continue;
            };
            if !texture.starts_with("XTextures") {
                continue;
            }
            draws += 1;
            let range =
                draw.index_start as usize..(draw.index_start + draw.index_count) as usize;
            let Some(indices) = model.indices.get(range) else {
                continue;
            };
            for &index in indices {
                let Some(v) = model.positions.get(index as usize) else {
                    continue;
                };
                let world = transform_point(&matrix, *v);
                for axis in 0..3 {
                    lo[axis] = lo[axis].min(world[axis]);
                    hi[axis] = hi[axis].max(world[axis]);
                }
            }
        }
        if draws == 0 {
            continue;
        }
        placements_with_water += 1;
        println!(
            "    {:.<34} {draws:3} draws  x {:8.1}..{:8.1}  y {:8.1}..{:8.1}  z {:7.1}..{:7.1}",
            name.rsplit('\\').next().unwrap_or(name),
            lo[0],
            hi[0],
            lo[1],
            hi[1],
            lo[2],
            hi[2],
        );
    }
    if placements_with_water == 0 {
        println!("    (none of this tile's placements carries liquid)");
    }

    // ---------------------------------------------------------------------
    // The terrain's own liquid: `MCLQ`, one per MCNK — the moat, the lakes and
    // the ocean, which are not in any building and were invisible for as long
    // as only `MLIQ` was read.
    //
    // Three things are checked here because nothing in the file states them:
    //
    // * **the flag bits are the declaration** — a block per set liquid bit, in
    //   bit order, so blocks-parsed against bits-set is the truncation check;
    // * **the vertex union is picked correctly** — water's byte 0 is a depth
    //   with a shore-and-plateau shape, magma's is half a texture coordinate
    //   spread flat, so the per-kind distribution below is what says a lava
    //   chunk was not read as a lake;
    // * **the frame turn is right** — the file's rows run in decreasing world
    //   x/y and the grid's in increasing, and getting that 180° turn wrong
    //   still draws a plausible lake. What it misplaces is the depths, so the
    //   check is that the surface stands *above* the terrain under it, and by
    //   more where the depth byte is larger.
    {
        use vale_assets::world::adt::mcnk_flags;
        use vale_protocol::state::movement;
        let mut chunks_flagged = 0usize;
        let mut blocks_expected = 0usize;
        let mut blocks_parsed = 0usize;
        let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut wet_tiles = 0usize;
        // **The high nibble of each wet tile's flag**, censused, because
        // nothing in the format states what it means. vmangos's extractor
        // reads `flags & 0x80` as deep water — the fatigue that kills a
        // swimmer — and reads no other bit; the count of each value over the
        // wet tiles is what says whether 0x40 is used at all.
        let mut nibbles: BTreeMap<u8, usize> = BTreeMap::new();
        // Per kind: (byte0 sum, count, min, max) over wet vertices.
        let mut byte0: BTreeMap<&'static str, (u64, usize, u8, u8)> = BTreeMap::new();
        let mut z: BTreeMap<&'static str, (f32, f32)> = BTreeMap::new();
        // Surface height minus terrain height, bucketed by the depth byte.
        let mut lift = [(0f64, 0usize); 3];
        let mut sunk = 0usize;
        let mut ground_samples = 0usize;

        for chunk in &adt.chunks {
            let bits = (chunk.flags & mcnk_flags::LQ_ANY).count_ones() as usize;
            if bits > 0 {
                chunks_flagged += 1;
                blocks_expected += bits;
            }
            for (kind, grid) in &chunk.liquids {
                blocks_parsed += 1;
                *kinds.entry(kind_name(*kind)).or_default() += 1;
                for ty in 0..grid.y_tiles {
                    for tx in 0..grid.x_tiles {
                        if grid.has_liquid(tx, ty) {
                            wet_tiles += 1;
                            let flag = grid.tile_flags.get(ty * grid.x_tiles + tx).copied();
                            *nibbles.entry(flag.unwrap_or(0) & 0xF0).or_default() += 1;
                        }
                    }
                }
                let side = grid.x_tiles + 1;
                for index in 0..side * side {
                    if !grid.vertex_is_wet(index) {
                        continue;
                    }
                    let v = grid.vertex(index % side, index / side);
                    let entry = z.entry(kind_name(*kind)).or_insert((f32::MAX, f32::MIN));
                    entry.0 = entry.0.min(v[2]);
                    entry.1 = entry.1.max(v[2]);
                    let depth = grid.depths.get(index).copied().unwrap_or(0);
                    let stats = byte0.entry(kind_name(*kind)).or_insert((0, 0, u8::MAX, 0));
                    stats.0 += depth as u64;
                    stats.1 += 1;
                    stats.2 = stats.2.min(depth);
                    stats.3 = stats.3.max(depth);

                    if matches!(kind, Liquid::Water | Liquid::Ocean) {
                        if let Some(ground) = adt.height_at(v[0], v[1]) {
                            ground_samples += 1;
                            let d = (v[2] - ground) as f64;
                            if d < -0.5 {
                                sunk += 1;
                            }
                            let bucket = match depth {
                                0..=31 => 0,
                                32..=95 => 1,
                                _ => 2,
                            };
                            lift[bucket].0 += d;
                            lift[bucket].1 += 1;
                        }
                    }
                }
            }
        }

        println!("  terrain liquid (MCLQ), the same tile:");
        if blocks_expected == 0 {
            println!("    no chunk declares a liquid flag — this tile's ground is dry");
        } else {
            println!(
                "    {chunks_flagged} of {} chunks declare liquid; {blocks_parsed} of \
                 {blocks_expected} declared blocks parse{}",
                adt.chunks.len(),
                if blocks_parsed == blocks_expected { "" } else { "  <- truncated MCLQ data" },
            );
            println!("    liquids: {kinds:?}, {wet_tiles} wet tiles");
            println!(
                "      tile flag high nibbles over the wet tiles: {}   <- 0x80 is deep water (fatigue), the one bit vmangos reads",
                nibbles
                    .iter()
                    .map(|(nibble, count)| format!("0x{nibble:02x} x{count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            for (name, (lo, hi)) in &z {
                let stats = byte0.get(name);
                match stats {
                    Some((total, count, min, max)) if *count > 0 => println!(
                        "      {name:6} z {lo:.1}..{hi:.1}   byte0 {min}..{max} mean {} over {count} wet vertices",
                        total / *count as u64
                    ),
                    _ => println!("      {name:6} z {lo:.1}..{hi:.1}"),
                }
            }

            let surface = adt.liquid_surface();
            let triangles: usize = surface.iter().map(|d| d.triangle_count()).sum();
            let parallel = surface
                .iter()
                .all(|d| d.uvs.len() == d.positions.len() && d.colours.len() == d.positions.len());
            println!(
                "    assembled surface: {} draws (one per kind), {triangles} triangles — {} 2 per wet tile{}",
                surface.len(),
                if triangles == wet_tiles * 2 { "exactly" } else { "NOT" },
                if parallel { "" } else { ", RAGGED BUFFERS — the renderer will drop these" },
            );

            // The frame-turn check. A transposed or untranslated grid puts deep
            // water over dry land, which shows here as a surface under its own
            // ground — and the lift must *rise* with the depth byte, because
            // that byte is how far down the bed is.
            if ground_samples > 0 {
                println!(
                    "    the surface stands clear of the ground under it: {} of {ground_samples} \
                     wet vertices sit more than 0.5y sunk",
                    sunk,
                );
                // An all-deep tile (open sea) has nothing in the shallow
                // buckets, and an empty bucket is omitted rather than a NaN.
                let buckets: Vec<String> = [("0..31", lift[0]), ("32..95", lift[1]), ("96..", lift[2])]
                    .into_iter()
                    .filter(|(_, b)| b.1 > 0)
                    .map(|(name, b)| format!("{name}: {:+.1}y", b.0 / b.1 as f64))
                    .collect();
                println!(
                    "      surface minus ground, by depth byte:  {}   <- must rise",
                    buckets.join("   "),
                );
            }

            // **And what the *mover* asks of all this**, which is a different
            // question from what the renderer asks: not "where do I draw a
            // surface" but "how deep is the water over this point", answered by
            // `Mcnk::liquid_at` at a world position rather than by walking the
            // grid. Two things are checked and each has its own failure.
            //
            // The **round trip** is the indexing: every wet tile's centre must
            // come back wet and every dry one dry. `liquid_at` inverts the
            // world-position-to-tile arithmetic `liquid_surface` walks forward,
            // so a transposed or off-by-one inverse still answers a plausible
            // height everywhere and would show up only as a character swimming
            // one tile away from the water.
            //
            // The **depth census** is the rule itself: `MIN_SWIM_DEPTH` against
            // the ground under the same point, which is the one place the swim
            // threshold and the real archives meet. A tile of open ocean should
            // be almost all swimmable and a river tile mostly bank.
            let mut probes = 0usize;
            let mut agreed = 0usize;
            let mut wet_probes = 0usize;
            let mut swimmable = 0usize;
            let mut waded = 0usize;
            let mut deepest = 0.0f32;
            for chunk in &adt.chunks {
                for (_, grid) in &chunk.liquids {
                    for ty in 0..grid.y_tiles {
                        for tx in 0..grid.x_tiles {
                            let v = grid.vertex(tx, ty);
                            let at = (
                                v[0] + wmo::LIQUID_TILE_SIZE * 0.5,
                                v[1] + wmo::LIQUID_TILE_SIZE * 0.5,
                            );
                            probes += 1;
                            let answered = chunk.liquid_at(at.0, at.1);
                            if answered.is_some() == grid.has_liquid(tx, ty) {
                                agreed += 1;
                            }
                            let Some((_, surface)) = answered else { continue };
                            wet_probes += 1;
                            let Some(ground) = adt.height_at(at.0, at.1) else {
                                continue;
                            };
                            let depth = surface - ground;
                            deepest = deepest.max(depth);
                            if depth > movement::MIN_SWIM_DEPTH {
                                swimmable += 1;
                            } else if depth > 0.0 {
                                waded += 1;
                            }
                        }
                    }
                }
            }
            if probes > 0 {
                println!(
                    "    liquid_at round trip: {agreed} of {probes} tile centres agree with \
                     has_liquid{}",
                    if agreed == probes { "" } else { "  <- the lookup is off by a tile" },
                );
                println!(
                    "    what the mover would do here: {swimmable} of {wet_probes} wet centres are \
                     over {:.2}y deep (swim), {waded} shallower (wade), deepest {deepest:.1}y",
                    movement::MIN_SWIM_DEPTH,
                );
            }
        }
    }

    // **What colour the water is, which is not in the water.** See the texture
    // line below: `lake_a` and `ocean_h` carry no colour at all, so the ramp
    // between a shallow and a deep colour comes out of `Light.dbc` ->
    // `LightParams.dbc` -> `LightIntBand.dbc`, and `MLIQ`'s depth byte — which
    // this command already reports as an opacity above — is the *mix* along it.
    // Printed here because a missing light chain is a documented degradation
    // (white water) rather than an error, and the only place it would otherwise
    // show is the window.
    match map_id_for(&mut assets, map) {
        Some(map_id) => {
            let tables = crate::common::open_display_tables(&mut assets)?;
            // **Which of the eighteen bands are the water bands, measured.**
            // `LightIntBand` names nothing; the row arithmetic gives band *b* of
            // every light, and picking the wrong *b* yields a plausible blue —
            // the sky and the fog are blue too. What separates them is that the
            // water bands come in **shallow/deep pairs where the deep one is
            // darker, in every zone the game ships**, and no sky band is paired
            // with the one after it that way. Printed across every default light
            // so the pair is a count and not an eyeball.
            if std::env::var("LIGHT_DEBUG").is_ok() {
                if let Some(light) = tables.light() {
                    let ids = light.default_params();
                    println!("  band survey over {} default lights, noon:", ids.len());
                    for band in 0..18u32 {
                        let mut mean = [0.0f64; 3];
                        let mut darker_than_previous = 0usize;
                        let mut seen = 0usize;
                        for &id in &ids {
                            let Some(c) = light.band(id, band, vale_assets::tables::light::NOON) else {
                                continue;
                            };
                            seen += 1;
                            for k in 0..3 {
                                mean[k] += c[k] as f64;
                            }
                            if band > 0 {
                                if let Some(p) =
                                    light.band(id, band - 1, vale_assets::tables::light::NOON)
                                {
                                    let sum = |v: [f32; 3]| v[0] + v[1] + v[2];
                                    if sum(c) < sum(p) {
                                        darker_than_previous += 1;
                                    }
                                }
                            }
                        }
                        let n = seen.max(1) as f64;
                        println!(
                            "    band {band:2}  mean {:3.0}/{:3.0}/{:3.0}   darker than band {:2}: {darker_than_previous:3}/{seen}",
                            mean[0] / n * 255.0,
                            mean[1] / n * 255.0,
                            mean[2] / n * 255.0,
                            band.saturating_sub(1),
                        );
                    }
                }
            }
            // The tile's own centre, because `Light.dbc` is positional: this
            // tile may stand inside a zone light with water of its own.
            let at = vale_assets::world::terrain::tile_centre(x, y);
            println!(
                "  liquid colour, from Light.dbc at map {map_id}, tile centre ({:.0}, {:.0}), noon:",
                at[0], at[1],
            );
            for kind in [Liquid::Water, Liquid::Ocean, Liquid::Magma, Liquid::Slime] {
                // `None` is not a failure: it is the answer for the two liquids
                // whose own texture carries colour. See `DisplayTables`.
                let Some(light) = tables.liquid_light(map_id, at, kind, vale_assets::tables::light::NOON)
                else {
                    println!(
                        "    {:<6} untinted — its own texture has colour",
                        kind_name(kind)
                    );
                    continue;
                };
                let byte = |v: f32| (v * 255.0).round() as u32;
                let rgb =
                    |c: [f32; 3]| format!("{:3}/{:3}/{:3}", byte(c[0]), byte(c[1]), byte(c[2]));
                // **The colours have no ordering and the alphas do.** A close/far
                // pair may go either way — surveyed over the 19 default lights the
                // far one is brighter in 15 — so the only thing checkable here is
                // the alphas, and it is the check that says `LightParams` fields
                // 5..8 are not transposed: a bank is never more opaque than the
                // middle of the pool.
                let shape = if light.shallow_alpha <= light.deep_alpha {
                    "bank no more opaque than the middle"
                } else {
                    "!! shallow alpha EXCEEDS deep — LightParams fields transposed?"
                };
                println!(
                    "    {:<6} close {}  far {}  alpha {:.2}..{:.2}  <- {shape}",
                    kind_name(kind),
                    rgb(light.close),
                    rgb(light.far),
                    light.shallow_alpha,
                    light.deep_alpha,
                );
            }
            // **…and what the world looks like from *under* that water**, which
            // is `Light.dbc`'s second `LightParams` column and the whole of what
            // the reference does about being submerged. It is checked here
            // rather than in `vale light` because it is a fact about water:
            // the number to look at is the fog, which should close to a few
            // dozen yards where the surface light reaches hundreds, and the
            // dome, which should be the colour of the water rather than of the
            // sky. A column read one field out would answer the *storm* light,
            // which is a plausible grey rather than an error.
            if let Some(light) = tables.light() {
                use vale_assets::tables::light::Weather;
                let byte = |v: f32| (v * 255.0).round() as u32;
                let rgb =
                    |c: [f32; 3]| format!("{:3}/{:3}/{:3}", byte(c[0]), byte(c[1]), byte(c[2]));
                let over = light.atmosphere_in(map_id, at, vale_assets::tables::light::NOON, Weather::Clear);
                let under =
                    light.atmosphere_in(map_id, at, vale_assets::tables::light::NOON, Weather::Underwater);
                println!("  and seen from under it — Light.dbc's clear-underwater column:");
                println!(
                    "    params {:?} over, {:?} under",
                    light.params_for(map_id),
                    light.params_for_weather(map_id, Weather::Underwater),
                );
                println!(
                    "    fog   {:.0}..{:.0}y over   {:.0}..{:.0}y under   <- must close in",
                    over.fog_start, over.fog_end, under.fog_start, under.fog_end,
                );
                println!(
                    "    dome  {} over   {} under   (the horizon stop)",
                    rgb(over.fog()),
                    rgb(under.fog()),
                );
            }
        }
        None => println!("  (map {map:?} is not in Map.dbc, so no light chain to read)"),
    }

    // The alpha depth is the load-bearing part: this renderer draws a WMO batch
    // with the model shader, which returns the texel's own alpha — so whether
    // water is translucent is a property of the file and not a setting.
    //
    // **And the depth is not the value.** `alphaDepth=8` says there are eight
    // bits of alpha in the file; it does not say what is in them, and a surface
    // blended by an alpha of zero is not translucent water, it is nothing at
    // all. The decoded mean is printed for exactly that reason — it is the
    // number the blend multiplies by, and the only one that says whether the
    // surface is visible.
    for kind in Liquid::ALL {
        // **Counted out of the archive rather than taken from the constant**,
        // and then held against it: `LIQUID_FRAMES` is the client's own 30
        // and the renderer indexes the set with it, so a family that ships a
        // different number would animate to a frame nobody has. Probed past 30
        // for the same reason — a count that agreed because it was told to
        // would check nothing.
        let frames = (1..=60).filter(|&f| assets.exists(&kind.texture(f))).count();
        let rate = if frames as u32 == vale_assets::world::wmo::LIQUID_FRAMES {
            format!(
                "{:.0} a second over {:.2}s",
                frames as f32 / kind.flipbook_period(),
                kind.flipbook_period()
            )
        } else {
            format!(
                "<- the client indexes {} of them",
                vale_assets::world::wmo::LIQUID_FRAMES
            )
        };
        let raw = assets.read(&kind.texture(1));
        let first = raw
            .as_ref()
            .map(|b| describe_blp(b))
            .unwrap_or_else(|e| e.to_string());
        let alpha = raw
            .ok()
            .and_then(|b| vale_assets::world::blp::decode(&b).ok())
            .map(|image| {
                let alphas: Vec<u8> = image.rgba.chunks_exact(4).map(|p| p[3]).collect();
                let total: u64 = alphas.iter().map(|&a| a as u64).sum();
                // The share below a quarter is what says "mask" rather than
                // "opacity map": an opacity map for water would sit around a
                // middling value, and this one is three quarters dark with a
                // saturated speckle — foam and glints.
                let faint = alphas.iter().filter(|&&a| a < 64).count();
                // **And the alpha is only half of what the blend deposits — the
                // other half is the colour, and for water there is none.** A
                // surface drawn at alpha `a` moves a pixel by `a * (rgb - bed)`,
                // so a black texture over a stone canal is a shadow whatever its
                // alpha does, which is the failure one step milder than the one
                // the depth byte fixed. `peak` is the number that settles it: the
                // *brightest single texel* in the whole image. `lake_a` reaches
                // 41 of 255 and `ocean_h` 82, greyscale; `lava` and `slime` reach
                // 255 and 174 in colour. Two of these four are masks and two are
                // pictures, and that is why the water's colour has to come from
                // `Light.dbc` — see the ramp printed above.
                let mut channel = [0u64; 3];
                for p in image.rgba.chunks_exact(4) {
                    for c in 0..3 {
                        channel[c] += p[c] as u64;
                    }
                }
                let texels = alphas.len().max(1) as u64;
                let peak = image
                    .rgba
                    .chunks_exact(4)
                    .map(|p| p[0].max(p[1]).max(p[2]))
                    .max()
                    .unwrap_or(0);
                format!(
                    ", decoded alpha {}..{} mean {}, {}% under 64, rgb mean {}/{}/{} peak {peak}",
                    alphas.iter().copied().min().unwrap_or(0),
                    alphas.iter().copied().max().unwrap_or(0),
                    total / texels,
                    faint * 100 / alphas.len().max(1),
                    channel[0] / texels,
                    channel[1] / texels,
                    channel[2] / texels,
                )
            })
            .unwrap_or_default();
        println!(
            "  {:<6} {:<32} {frames} frames, {rate}, {first}{alpha}",
            kind_name(kind),
            kind.texture_pattern()
        );
    }
    Ok(())
}

/// The id `Map.dbc` gives the directory this command was asked for.
///
/// The light chain is keyed by map id and every other terrain command is keyed
/// by directory name, so this is the one place the two meet.
pub(crate) fn map_id_for(assets: &mut vale_assets::archive::Assets, directory: &str) -> Option<u32> {
    use vale_assets::tables::dbc::{dbc_path, map_directories};
    let raw = assets.read(&dbc_path("Map")).ok()?;
    map_directories(&raw)
        .ok()?
        .into_iter()
        .find(|(_, dir)| dir.eq_ignore_ascii_case(directory))
        .map(|(id, _)| id)
}

fn kind_name(kind: vale_assets::world::wmo::Liquid) -> &'static str {
    use vale_assets::world::wmo::Liquid;
    match kind {
        Liquid::Water => "water",
        Liquid::Ocean => "ocean",
        Liquid::Magma => "magma",
        Liquid::Slime => "slime",
    }
}
