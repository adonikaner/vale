//! `vale wmos` — every WMO a tile places, against its MODF record.

use crate::common::*;
use vale_assets::{world::adt::Adt, adt_path};
use vale_config::Config;

/// Every WMO placed on one tile: read the root and its groups, place them, and
/// check the result against the tile's own `MODF` bounding boxes.
///
/// That last check is the point of the command. `MODF` carries a world-space
/// axis-aligned box for each placement, computed by Blizzard's own tools from
/// the geometry it is placing. Transforming the group vertices by
/// [`adt::placement_matrix`] and comparing the box that falls out is therefore a
/// direct test of *both* the parser and the placement matrix, against data this
/// project did not produce — a building rotated the wrong way, or read in the
/// wrong axis order, misses by tens of yards and cannot look right by accident.
pub fn cmd_wmos(cfg: &Config, map: &str, x: u32, y: u32) -> Result<(), String> {
    use vale_assets::world::adt::{placement_matrix, placement_to_world};
    use vale_assets::world::collision::transform_point;
    use vale_assets::world::wmo;
    use std::collections::{BTreeMap, BTreeSet};

    let mut assets = open_assets(cfg)?;
    let raw = assets.read(&adt_path(map, x, y)).map_err(|e| e.to_string())?;
    let adt = Adt::parse(&raw).map_err(|e| e.to_string())?;

    println!("{}", adt_path(map, x, y));
    println!(
        "  {} WMO placements of {} distinct buildings",
        adt.wmos.len(),
        adt.wmo_names.len()
    );

    // Decode each distinct WMO once — a village re-uses the same three houses.
    let mut models: BTreeMap<String, wmo::WmoModel> = BTreeMap::new();
    let mut textures: BTreeSet<String> = BTreeSet::new();
    let mut doodad_models: BTreeSet<String> = BTreeSet::new();
    let mut failures = 0usize;

    let distinct: BTreeSet<String> = adt
        .wmos
        .iter()
        .filter_map(|w| adt.wmo_names.get(w.name_id as usize).cloned())
        .collect();
    for path in &distinct {
        match wmo::load(&mut assets, path) {
            Ok((model, dropped)) => {
                for name in &dropped {
                    println!("    !! {name}");
                }
                textures.extend(model.textures.iter().cloned());
                doodad_models.extend(
                    model
                        .doodads
                        .iter()
                        .filter(|d| d.drawable())
                        .map(|d| d.path.clone()),
                );
                models.insert(path.clone(), model);
            }
            Err(e) => {
                println!("    !! {path}: {e}");
                failures += 1;
            }
        }
    }

    let vertices: usize = models.values().map(|m| m.positions.len()).sum();
    let triangles: usize = models.values().map(wmo::WmoModel::triangle_count).sum();
    let groups: usize = models.values().map(|m| m.groups.len()).sum();
    let draws: usize = models.values().map(|m| m.draws.len()).sum();
    println!(
        "  {} of {} decoded: {groups} groups, {vertices} verts, {triangles} tris, {draws} batches",
        models.len(),
        distinct.len()
    );
    if failures > 0 {
        println!("  !! {failures} failed to decode");
    }

    // What those batches cost to *draw*, which is not the same number.
    //
    // The renderer gives every batch its own mesh and its own material, so a
    // batch is a draw call and no two of them can ever be merged — a building
    // whose batch list is long is therefore expensive in exact proportion to
    // that list, whatever is on screen. Stormwind is one WMO with 3,042 of them.
    //
    // But a batch is a *material* change only when the material actually
    // differs, and a WMO names a few dozen textures across thousands of batches.
    // So this reports what the list would collapse to if batches sharing a
    // material were concatenated into one mesh — per group, which keeps a room
    // cullable, and per model, which does not. The gap between the two is the
    // price of keeping the groups separate, and it is worth knowing before
    // paying it.
    let key = |d: &wmo::WmoDraw| (d.texture, d.blend, d.unlit, d.two_sided, d.light);
    let mut per_group: BTreeSet<(&str, u32, _)> = BTreeSet::new();
    let mut per_model: BTreeSet<(&str, _)> = BTreeSet::new();
    let mut blended = 0usize;
    for (path, model) in &models {
        for draw in &model.draws {
            per_group.insert((path.as_str(), draw.group, key(draw)));
            per_model.insert((path.as_str(), key(draw)));
            // Modes 2 and up are translucent: they land in the sorted
            // back-to-front phase, where they neither batch nor occlude.
            if draw.blend >= 2 {
                blended += 1;
            }
        }
    }
    println!(
        "  draw calls: {draws} as batches, {} merged per group, {} merged per model ({blended} translucent)",
        per_group.len(),
        per_model.len()
    );

    let missing: Vec<&String> = textures.iter().filter(|t| assets.read(t).is_err()).collect();
    if missing.is_empty() {
        println!("  all {} WMO textures resolve in the archive chain", textures.len());
    } else {
        println!(
            "  !! {} of {} WMO textures missing, e.g. {:?}",
            missing.len(),
            textures.len(),
            &missing[..missing.len().min(3)]
        );
    }

    let bad_doodads = doodad_models
        .iter()
        .filter(|p| {
            assets
                .read(p)
                .and_then(|raw| vale_assets::world::m2::M2::parse(&raw))
                .is_err()
        })
        .count();
    println!(
        "  {} distinct interior doodad models, {bad_doodads} that will not decode",
        doodad_models.len()
    );

    // **How much of the furniture moves**, which is the number that decides
    // whether a building's doodads can be posed at all.
    //
    // An animated `MODD` spawn is a *skinned* draw — re-propagated, re-extracted
    // and re-skinned every frame — where a still one is extracted once and then
    // free. So the question is not "does this model carry a skeleton" but "how
    // many spawns in one building do", and the answer decides the mechanism
    // rather than merely being interesting: a handful can be posed per instance,
    // and thousands cannot.
    //
    // Counted over the **spawns** rather than the models, and the two are very
    // different: one animated model placed two hundred times is two hundred
    // skinned draws.
    {
        let mut animated_models: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let mut spawns = 0usize;
        let mut animated_spawns = 0usize;
        for model in models.values() {
            for doodad in model.doodads.iter().filter(|d| d.drawable()) {
                spawns += 1;
                let entry = animated_models.entry(doodad.path.clone());
                let (count, sequences) = match entry {
                    std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::btree_map::Entry::Vacant(e) => {
                        let sequences = assets
                            .read(&doodad.path)
                            .and_then(|raw| vale_assets::world::m2::M2::parse(&raw))
                            .ok()
                            .map(|m2| m2.skeleton.map_or(0, |s| s.sequences.len()))
                            .unwrap_or(0);
                        e.insert((0, sequences))
                    }
                };
                *count += 1;
                if *sequences > 0 {
                    animated_spawns += 1;
                }
            }
        }
        let animated: Vec<(&String, &(usize, usize))> = {
            let mut rows: Vec<_> = animated_models
                .iter()
                .filter(|(_, (_, sequences))| *sequences > 0)
                .collect();
            rows.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
            rows
        };
        println!(
            "  MODD animation: {} of {} distinct models carry sequences, over {animated_spawns} of {spawns} spawns",
            animated.len(),
            animated_models.len()
        );
        for (path, (count, sequences)) in animated.iter().take(12) {
            let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
            println!("      x{count:<5} {sequences} seq  {name}");
        }
        if animated.len() > 12 {
            println!("      … and {} more", animated.len() - 12);
        }
    }

    // **Where the inside of each building is**, which is what an *entity* needs
    // and a `MODD` spawn does not: the furniture arrives with its own baked
    // light, and a player walks in and out of the same door.
    //
    // The rule is `MOGP`'s `INDOOR` bit and the box the group's own geometry
    // covers ([`wmo::WmoModel::interior_bounds`]), and it is checkable without a
    // renderer because the building already contains a few thousand points that
    // are known to be indoors: **its own `MODD` spawns.** A furniture spawn is
    // under a roof by definition, so the share of them landing inside an indoor
    // box is a direct measure of whether those boxes are the rooms. It is not
    // expected to be 100% — a `MODD` set includes lamp-posts and market stalls
    // standing in a courtyard — but a low number would mean the boxes are
    // somewhere else entirely, which is the failure that would otherwise show up
    // as characters lit by torchlight in the street.
    {
        let mut spawns = 0usize;
        let mut inside = 0usize;
        let mut indoor_groups = 0usize;
        let mut lit = Vec::new();
        // **The `MODR` split, and the geometric test to check it against.**
        // Which light a spawn is under is the groups' own reference lists
        // crossed with their flags (`WmoDoodad::exterior_lit`) — the mechanism
        // the renderer uses — and the box test is an independent measurement of
        // the same fact, so the two agreeing is what says the refs are being
        // read right rather than merely being read.
        let mut sunlit = 0usize;
        let mut sunlit_inside_a_box = 0usize;
        let mut sunlit_peak_sum = 0u64;
        let mut baked_peak_sum = 0u64;
        let mut baked = 0usize;
        let mut sunlit_models: std::collections::BTreeMap<String, usize> = Default::default();
        for (path, model) in &models {
            let rooms = model.interior_bounds();
            indoor_groups += rooms.len();
            for spawn in &model.doodads {
                spawns += 1;
                // The spawn's origin, which is where it stands: its matrix is
                // already WMO-local, the same space the group boxes are in.
                let in_a_room = rooms.iter().any(|b| wmo::box_contains(b, spawn.position));
                if in_a_room {
                    inside += 1;
                }
                let peak = spawn.light().map_or(0, |l| {
                    (l.iter().fold(0.0f32, |a, &c| a.max(c)) * 255.0).round() as u64
                });
                if spawn.exterior_lit {
                    sunlit += 1;
                    sunlit_peak_sum += peak;
                    sunlit_inside_a_box += in_a_room as usize;
                    let name = spawn.path.rsplit('\\').next().unwrap_or(&spawn.path);
                    *sunlit_models.entry(name.to_ascii_lowercase()).or_default() += 1;
                } else {
                    baked += 1;
                    baked_peak_sum += peak;
                }
            }
            let light = model.room_light(0);
            lit.push((
                path.rsplit('\\').next().unwrap_or(path).to_string(),
                rooms.len(),
                light,
            ));
        }
        println!(
            "  interiors: {indoor_groups} of {groups} groups are rooms; {inside} of {spawns} \
             MODD spawns stand inside one"
        );
        // The street furniture, named: the count, how dim the bake it is no
        // longer drawn by was, and how far the two tests agree. `MODR` says
        // where a spawn is *referenced*; the boxes say where it *stands* — a
        // sun-lit spawn inside a box is a lamp just inside a market awning, so
        // a few are expected, and all of them would mean the flags are being
        // read backwards.
        if sunlit > 0 {
            let mut names: Vec<(usize, &str)> = sunlit_models
                .iter()
                .map(|(name, count)| (*count, name.as_str()))
                .collect();
            names.sort_unstable_by(|a, b| b.cmp(a));
            let sample: Vec<String> = names
                .iter()
                .take(10)
                .map(|(count, name)| format!("{name} x{count}"))
                .collect();
            println!(
                "  MODR: {sunlit} spawns referenced only by exterior groups are sun-lit \
                 ({sunlit_inside_a_box} of them inside an indoor box); \
                 bake peak mean {} against {} indoors",
                sunlit_peak_sum / sunlit as u64,
                baked_peak_sum / baked.max(1) as u64,
            );
            println!("    {}", sample.join("  "));
        }
        // And what an entity standing in one is lit by — the mean of those
        // spawns' own lights, which is the light the renderer re-dresses to.
        lit.sort_by_key(|(_, rooms, _)| std::cmp::Reverse(*rooms));
        for (name, rooms, light) in lit.iter().take(4) {
            let byte = |c: f32| (c * 255.0).round() as u32;
            println!(
                "    {name:.<46} {rooms:3} rooms  room light {}/{}/{}",
                byte(light[0]),
                byte(light[1]),
                byte(light[2]),
            );
        }
    }

    // **`MODD`'s own colour, which is what a chair inside a room is lit by.**
    //
    // A `MODD` spawn is not lit by the sun — it is standing under a roof — and
    // it carries no vertex colours either, because it is an ordinary M2 shared
    // with the trees outside. What it has instead is one baked colour per
    // *spawn*, sampled where the client's tools put it, and this is the
    // measurement that says whether that field is worth reading: a table of
    // zeroes would mean the room's own `MOHD` ambient is the only answer
    // available.
    //
    // The distinct-value count is the other half of it — not because a colour
    // is a material any more (it rides on each instance as its `MeshTag`; see
    // `models::RoomLight`), but because it is what that decision was made on:
    // 2,249 distinct lights over one tile is what a material dimension cannot
    // afford and a per-instance u32 does not notice.
    {
        let mut all: Vec<(String, u32)> = Vec::new();
        for path in &distinct {
            let Ok(raw) = assets.read(path) else { continue };
            let Ok(root) = wmo::WmoRoot::parse(&raw) else {
                continue;
            };
            all.extend(
                root.doodads
                    .iter()
                    .filter(|d| d.drawable())
                    .map(|d| (d.path.clone(), d.colour)),
            );
        }
        let spawns = all.len();
        let mut coloured = 0usize;
        let mut opaque = 0usize;
        let mut sum = 0u64;
        let mut distinct_colours = std::collections::BTreeSet::new();
        for &(_, colour) in &all {
            if colour & 0x00FF_FFFF != 0 {
                coloured += 1;
                distinct_colours.insert(colour);
                let [b, g, r] = [colour & 0xFF, (colour >> 8) & 0xFF, (colour >> 16) & 0xFF];
                sum += u64::from(r.max(g).max(b));
            }
            if colour >> 24 != 0 {
                opaque += 1;
            }
        }
        if spawns > 0 {
            println!(
                "  MODD colour: {coloured} of {spawns} spawns carry one, {} distinct, \
                 peak channel mean {}, {opaque} with a non-zero alpha byte",
                distinct_colours.len(),
                sum / coloured.max(1) as u64,
            );
            // **And what the renderer pays for it**, which used to be this
            // check's hard question and is now its receipt. The colour rides on
            // each instance as its `MeshTag` (`models::RoomLight`), so however
            // many lights the tools baked, the furniture's materials are one
            // per (model, room-lit) — the pair count below is what that
            // *replaces*, and it is printed so the decision stays visible: a
            // material per pair was 3,037 for this tile's furniture alone.
            {
                let mut pairs = std::collections::BTreeSet::new();
                for (model, colour) in &all {
                    pairs.insert((model.as_str(), *colour));
                }
                let models: std::collections::BTreeSet<&str> =
                    all.iter().map(|(m, _)| m.as_str()).collect();
                println!(
                    "    as instance tags: {} room-lit materials (one per model), \
                     against the {} (model, light) pairs a material per colour would cost",
                    models.len(),
                    pairs.len(),
                );
            }
        }
    }

    // MOCV — the light inside the buildings.
    //
    // Reported because `WmoGroup::shaded_colours` is the one part of this format
    // transcribed from a renderer rather than derived from the file, and its
    // interior branch has a gain term whose scale depends on whether the client
    // worked in bytes or in normalised floats. The raw alpha histogram is what
    // settles it: in byte space the gain is `1 + a/64`, which at the top of the
    // range is a factor of five and would saturate every lit vertex to white.
    let mut lit_groups = 0usize;
    let mut all_groups = 0usize;
    let mut alpha_buckets = [0usize; 4];
    let mut raw_vertices = 0usize;
    for path in &distinct {
        let Ok(raw) = assets.read(path) else { continue };
        let Ok(root) = wmo::WmoRoot::parse(&raw) else {
            continue;
        };
        for index in 0..root.group_count {
            let parsed = assets
                .read(&wmo::group_path(path, index))
                .and_then(|b| wmo::WmoGroup::parse(&b));
            let Ok(group) = parsed else { continue };
            all_groups += 1;
            if group.colours.is_empty() {
                continue;
            }
            lit_groups += 1;
            for c in &group.colours {
                raw_vertices += 1;
                alpha_buckets[match (c[3] * 255.0).round() as u32 {
                    0 => 0,
                    1..=63 => 1,
                    64..=191 => 2,
                    _ => 3,
                }] += 1;
            }
        }
    }

    if raw_vertices == 0 {
        println!("  MOCV: none of {all_groups} groups carries vertex colours");
    } else {
        let pct = |n: usize| 100.0 * n as f32 / raw_vertices as f32;
        println!(
            "  MOCV: {lit_groups} of {all_groups} groups carry vertex colours, {raw_vertices} verts"
        );
        println!(
            "    raw alpha:  0: {:.0}%   1-63: {:.0}%   64-191: {:.0}%   192-255: {:.0}%",
            pct(alpha_buckets[0]),
            pct(alpha_buckets[1]),
            pct(alpha_buckets[2]),
            pct(alpha_buckets[3])
        );

        // What the fixup actually produced. A room should come out mid-range:
        // all-zero means the ambient swallowed it, all-255 means the gain did.
        let mut lit = 0usize;
        let mut sum = 0u64;
        let mut peak = 0u8;
        let mut saturated = 0usize;
        for model in models.values() {
            for c in &model.colours {
                let level = c[0].max(c[1]).max(c[2]);
                if level == 0 {
                    continue;
                }
                lit += 1;
                sum += u64::from(level);
                peak = peak.max(level);
                if level == 255 {
                    saturated += 1;
                }
            }
        }
        if lit > 0 {
            println!(
                "    shaded peak channel: mean {}, max {peak}, saturated {:.1}%",
                sum / lit as u64,
                100.0 * saturated as f32 / lit as f32
            );
        }
        println!(
            "    ambient: {:?}",
            models
                .values()
                .map(|m| m.ambient.map(|c| (c * 255.0).round() as u8))
                .next()
                .unwrap_or([0; 3])
        );
    }

    // **Which light each batch is drawn by**, which is the question the MOCV
    // counts above do not answer — and which is per *batch* rather than per
    // group, because the file states a seam between the two.
    //
    // Two independent things are checked here and neither is a matter of taste:
    //
    // * **the second exterior bit.** The client's mask is `0x48`
    //   (`wmo::group_flags::EXTERIOR_LIT`), and the line below counts how many
    //   groups carry only `0x40` — every one of which this client used to draw
    //   at its bake with no sun on it, which is a street lit as though it were
    //   a cellar.
    // * **the batch sections.** `MOGP` `+0x28`/`+0x2A` claim to partition the
    //   batch list, and if the offsets were wrong the sections would land
    //   uncorrelated with the flags. So the census prints the crossing: an
    //   *interior*-section batch belongs in a group the flags call indoor, and
    //   an exterior-section one in a group they call outdoor. Noise there is
    //   the offsets being wrong; a clean split is the layout pinned by the
    //   files rather than by a reference.
    let mut modes = [0usize; 4];
    let mut ext_only = 0usize;
    let mut lit_only = 0usize;
    let mut lit_only_baked = 0usize;
    let mut laws = [0usize; 3];
    // (section, is the group interior by the 0x48 mask) -> batches.
    let mut cross = [[0usize; 2]; 3];
    for model in models.values() {
        for group in &model.groups {
            let a = group.flags & wmo::group_flags::EXTERIOR != 0;
            let b = group.flags & wmo::group_flags::EXTERIOR_LIT != 0;
            modes[usize::from(group.indoor) * 2 + usize::from(a || b)] += 1;
            ext_only += usize::from(a && !b);
            // **The bit's teeth.** Reclassifying a group only changes a picture
            // where the group has a bake to be drawn at instead of the sun, so
            // this is the number that says whether the correction is visible or
            // merely right: a `0x40`-only group with no `MOCV` fell through to
            // the sun anyway.
            lit_only += usize::from(b && !a);
            lit_only_baked +=
                usize::from(b && !a && group.flags & wmo::group_flags::HAS_VERTEX_COLOUR != 0);
        }
        for draw in &model.draws {
            laws[match draw.light {
                wmo::BatchLight::Sun => 0,
                wmo::BatchLight::Bake => 1,
                wmo::BatchLight::Blend => 2,
            }] += 1;
        }
        for group in &model.groups {
            let interior = group.flags & 0x48 == 0;
            let start = group.draw_start as usize;
            for draw in &model.draws[start..start + group.draw_count as usize] {
                if draw.liquid.is_some() {
                    continue;
                }
                cross[draw.section as usize][usize::from(interior)] += 1;
            }
        }
    }
    println!(
        "  lit by: {} batches by the sun, {} by their baked MOCV, {} faded between the two",
        laws[0], laws[1], laws[2]
    );
    println!(
        "    groups: {} neither flag, {} exterior, {} indoor only, {} both — of the exteriors, {ext_only} by 0x08 alone and {lit_only} by 0x40 alone ({lit_only_baked} of those carrying MOCV)",
        modes[0], modes[1], modes[2], modes[3]
    );
    println!(
        "    batch sections (outdoor group / indoor group): trans {}/{}, int {}/{}, ext {}/{}",
        cross[0][0], cross[0][1], cross[1][0], cross[1][1], cross[2][0], cross[2][1]
    );

    // **The open-sky bit, `0x8000`, against the other two.** It is what the
    // client's `IsOutdoors` and the server's `IsOutdoorWMO` both read to decide
    // whether a mount may be summoned (`wmo::group_flags::OUTDOOR`), and it is
    // a third opinion beside `INDOOR` and the `0x48` lighting mask. The
    // crossing is printed because the whole report was that the client had
    // been reading one of the other two for this question.
    let mut open = [[0usize; 2]; 2]; // [open sky][indoor bit]
    let mut open_lit = [0usize; 2]; // open sky, by 0x48 lit
    for model in models.values() {
        for group in &model.groups {
            let sky = group.flags & wmo::group_flags::OUTDOOR != 0;
            open[usize::from(sky)][usize::from(group.indoor)] += 1;
            if sky {
                open_lit[usize::from(group.flags & 0x48 != 0)] += 1;
            }
        }
    }
    println!(
        "    open sky (0x8000): {} groups — {} of them also INDOOR, {} lit by the sun (0x48), {} not; {} groups without it, {} of those INDOOR",
        open[1][0] + open[1][1],
        open[1][1],
        open_lit[1],
        open_lit[0],
        open[0][0] + open[0][1],
        open[0][1],
    );

    // **The groups that carry both bits, by the names the file gives them.**
    // `INDOOR` (0x2000) is the portal system's bit and `0x48` is the lighting
    // one, and a group can carry both — which is a room for culling and the
    // open air for light. Whether that is really what those groups *are* is not
    // something a flag census can answer, but `MOGN` names every group, so the
    // building says so itself. Printed rather than counted, because a name is
    // the evidence here.
    let mut both: Vec<&str> = Vec::new();
    for model in models.values() {
        for group in &model.groups {
            if group.indoor && group.flags & 0x48 != 0 {
                both.push(group.name.as_str());
            }
        }
    }
    if !both.is_empty() {
        both.sort_unstable();
        both.dedup();
        println!(
            "    INDOOR *and* exterior-lit, by their own MOGN names ({} distinct): {:?}",
            both.len(),
            &both[..both.len().min(40)]
        );
    }

    // **…and the alpha channel each section reads, which is the interpretation
    // rather than the layout.** The counts above are a measurement: they say the
    // file partitions its batches and that the partition lines up with the group
    // flags. What that partition *means* is a separate claim, and the claim is
    // that one byte carries two different payloads — an emissive mask on an
    // interior batch (sparse: mostly zero, a few hearths high) and a blend
    // weight on a transition one (a ramp: spread across the range, and reaching
    // the top, because a doorway has vertices that are wholly outdoors).
    //
    // Those two distributions look nothing alike, so printing them apart is the
    // check. If the transition column came out 90% zero like the interior one,
    // the reading would be wrong and the fade would be a fade to nothing.
    let mut hist = [[0usize; 4]; 3];
    for model in models.values() {
        for draw in &model.draws {
            if draw.liquid.is_some() {
                continue;
            }
            let start = draw.index_start as usize;
            for &i in &model.indices[start..start + draw.index_count as usize] {
                let a = model.colours[i as usize][3];
                let bucket = match a {
                    0 => 0,
                    1..=63 => 1,
                    64..=191 => 2,
                    _ => 3,
                };
                hist[draw.section as usize][bucket] += 1;
            }
        }
    }
    for (name, row) in ["trans", "int  ", "ext  "].iter().zip(&hist) {
        let total: usize = row.iter().sum();
        if total == 0 {
            continue;
        }
        let pc = |n: usize| n * 100 / total;
        println!(
            "    {name} alpha: 0 {:>3}%   1-63 {:>3}%   64-191 {:>3}%   192-255 {:>3}%   ({total} refs)",
            pc(row[0]),
            pc(row[1]),
            pc(row[2]),
            pc(row[3])
        );
    }
    // **A room with no `MOCV` is the case that decides the rule.** `vertex_lit`
    // wants the colours to be there, so such a group falls through to the sun —
    // which reaches it through its own ceiling. Counted per model because "a
    // room lit like a field" is only visible in the buildings that have one.
    for (path, model) in &models {
        let dark: usize = model
            .groups
            .iter()
            .filter(|g| g.indoor && !g.vertex_lit)
            .count();
        if dark > 0 {
            println!(
                "    {:.<45} {dark} of {} rooms carry no MOCV",
                path.rsplit('\\').next().unwrap_or(path.as_str()),
                model.groups.iter().filter(|g| g.indoor).count()
            );
        }
    }

    // The measurement. `MODF`'s box is world-space and axis-aligned, so the
    // transformed geometry has to sit inside it — and fill it, since Blizzard
    // derived the box from the same vertices.
    let mut checked = 0usize;
    let mut agree = 0usize;
    let mut worst_horizontal = 0.0f32;
    let mut worst: Option<(f32, String)> = None;
    for placement in &adt.wmos {
        let Some(name) = adt.wmo_names.get(placement.name_id as usize) else {
            continue;
        };
        let Some(model) = models.get(name) else { continue };
        if model.positions.is_empty() {
            continue;
        }

        let position = placement_to_world(placement.position);
        let matrix = placement_matrix(position, placement.rotation, 1.0);

        // The eight corners of the WMO's own `MOHD` box, transformed. Not the
        // vertices: `MODF`'s box is itself a corner-transformed box — which is
        // why it is *looser* than the geometry at any yaw that is not a
        // multiple of 90 degrees, by an amount that grows with the building.
        // Comparing against the tight vertex box instead reports a 14-yard
        // "error" on a correctly placed church.
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        let mut extend = |box_: [[f32; 3]; 2], transform: &[f32; 16]| {
            for corner in 0..8usize {
                let local = [
                    box_[corner & 1][0],
                    box_[(corner >> 1) & 1][1],
                    box_[(corner >> 2) & 1][2],
                ];
                let world = transform_point(transform, local);
                for axis in 0..3 {
                    lo[axis] = lo[axis].min(world[axis]);
                    hi[axis] = hi[axis].max(world[axis]);
                }
            }
        };
        // Only the building. The interior doodads are deliberately *not* folded
        // in: a doodad of `duskwood_humantwostory` sticks five yards out of its
        // placement's `MODF` box, so the box is the `MOHD` box and nothing else.
        extend(model.local_bounds, &matrix);

        // MODF's corners are in the same (westward, up, northward) frame as its
        // position, so converting both and re-sorting gives the world box.
        let a = placement_to_world(placement.bounds_lower);
        let b = placement_to_world(placement.bounds_upper);
        let want_lo = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let want_hi = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];

        let axis_error = |i: usize| (lo[i] - want_lo[i]).abs().max((hi[i] - want_hi[i]).abs());
        let error = (0..3).map(axis_error).fold(0.0f32, f32::max);
        // The horizontal error on its own, because that is the number the yaw
        // controls: a placement rotated the wrong way — or built without the
        // 180-degree frame term — misses in x and y, and cannot miss only in z.
        let horizontal = axis_error(0).max(axis_error(1));

        checked += 1;
        if error < 1.0 {
            agree += 1;
        }
        worst_horizontal = worst_horizontal.max(horizontal);
        if std::env::var("WMO_DEBUG").is_ok() {
            println!(
                "    rot {:7.1?} err {error:7.2}\n      got  {:8.1?} .. {:8.1?}\n      want {:8.1?} .. {:8.1?}",
                placement.rotation, lo, hi, want_lo, want_hi
            );
        }
        if worst.as_ref().is_none_or(|(w, _)| error > *w) {
            worst = Some((error, name.clone()));
        }
    }

    println!("  placement check against MODF bounds, {checked} placements:");
    println!("    {agree} agree on all six faces to within 1y");
    println!("    worst horizontal error {worst_horizontal:.2}y  <- what the yaw controls");
    if let Some((error, name)) = &worst {
        println!(
            "    worst error on any face {error:.2}y  {}",
            name.rsplit('\\').next().unwrap_or(name)
        );
    }

    for (path, model) in models.iter().take(5) {
        let sets: Vec<String> = model
            .doodad_sets
            .iter()
            .map(|s| format!("{}({})", s.name, s.count))
            .collect();
        println!(
            "    {:.<46} {:3} groups {:5} tris  sets: {}",
            path.rsplit('\\').next().unwrap_or(path),
            model.groups.len(),
            model.triangle_count(),
            if sets.is_empty() { "-".to_string() } else { sets.join(" ") }
        );
        if std::env::var("WMO_DEBUG").is_ok() {
            println!(
                "      MOHD box {:7.1?}..{:7.1?}  drawn {:7.1?}..{:7.1?}",
                model.local_bounds[0], model.local_bounds[1], model.bounds[0], model.bounds[1]
            );
        }
    }
    Ok(())
}
