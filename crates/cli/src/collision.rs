//! `vale collision` — the solid half of those buildings.

use crate::common::*;
use vale_assets::{world::adt::Adt, adt_path};
use vale_config::Config;

/// What a tile is *solid*: the collision hull of every building on it, checked
/// against the same `MODF` boxes `cmd_wmos` uses and then against itself.
///
/// The hull is not the geometry that is drawn — see [`vale_assets::world::collision`]
/// — so none of `vale wmos`' numbers say anything about it, and every way it
/// can be wrong is quiet:
///
/// * a hull built from the draw list instead of `MOPY` is missing the invisible
///   collision triangles and carries the decorative ones, which reads as *a
///   building you can walk into through one particular wall*;
/// * a hull placed through the wrong matrix is a solid building somewhere near
///   the visible one, which reads as an invisible wall in the street;
/// * a grid that loses triangles is a hole in a floor, which reads as *falling
///   through the world in one spot*.
///
/// So there are four measurements here, and each is against something this
/// project did not produce or did not use to build the thing being checked:
/// the `MOPY` census against the file's own triangle count, containment against
/// Blizzard's `MODF` box, the floor query against the hull's own triangles, and
/// the wall query against the hull's own vertical faces.
///
/// ## …and `vale collision <map> <tx> <ty> <x> <y>` asks about **one point**
///
/// Everything above is a sweep, and a sweep answers "is the tile right?" — it
/// cannot answer *"why is this creature standing in the air?"*, which is the
/// question a bug report actually arrives as. The server states a z for every
/// unit and the client draws it there; when the two disagree with the ground,
/// the thing to know is what surfaces exist under that exact spot and how far
/// each is from the height in question. This prints the terrain, every building
/// and doodad hull that answers there with its own name and height, and the
/// number `Standing::floor` would come out with. It was built for a report of
/// NPCs appearing to float.
pub fn cmd_collision(
    cfg: &Config,
    map: &str,
    x: u32,
    y: u32,
    at: Option<(f32, f32, Option<f32>)>,
) -> Result<(), String> {
    use vale_assets::world::adt::{placement_matrix, placement_to_world};
    use vale_assets::world::collision::{
        transform_point, Collider, CollisionWorld, Solid, SolidId, PLAYER_RADIUS, PROBE_HEIGHTS,
        STEP_UP,
    };
    use vale_assets::world::wmo;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    let mut assets = open_assets(cfg)?;
    let raw = assets.read(&adt_path(map, x, y)).map_err(|e| e.to_string())?;
    let adt = Adt::parse(&raw).map_err(|e| e.to_string())?;

    println!("{}", adt_path(map, x, y));
    println!(
        "  {} WMO placements of {} distinct buildings",
        adt.wmos.len(),
        adt.wmo_names.len()
    );

    let distinct: BTreeSet<String> = adt
        .wmos
        .iter()
        .filter_map(|w| adt.wmo_names.get(w.name_id as usize).cloned())
        .collect();
    let mut models: BTreeMap<String, wmo::WmoModel> = BTreeMap::new();
    for path in &distinct {
        match wmo::load(&mut assets, path) {
            Ok((model, _)) => {
                models.insert(path.clone(), model);
            }
            Err(e) => println!("    !! {path}: {e}"),
        }
    }

    // 1. The census. A hull at 100% of the declared triangles means the
    //    `DETAIL` term was dropped; one at 0% means `MOPY` was never read. Both
    //    produce a client that runs.
    let declared: usize = models.values().map(|m| m.collision.declared_triangles).sum();
    let solid: usize = models.values().map(|m| m.collision.triangle_count()).sum();
    let hull_verts: usize = models.values().map(|m| m.collision.positions.len()).sum();
    let drawn: usize = models.values().map(wmo::WmoModel::triangle_count).sum();
    println!(
        "  hull: {} of {} buildings built — {solid} solid of {declared} declared ({:.0}%), \
         {hull_verts} verts",
        models.len(),
        distinct.len(),
        100.0 * solid as f32 / declared.max(1) as f32
    );
    println!("    the drawn geometry is {drawn} triangles, which is a different set");

    // Is that the *flag* byte? `MOPY` is a flag and a material index side by
    // side, and reading the wrong one of the two still yields a hull with a
    // plausible-looking share of the triangles in it — 8 of 256 material ids
    // have 0x20 set, and a building whose materials happen to land there comes
    // out solid. What tells them apart is the shape of the histogram: flags are
    // a handful of values dominated by RENDER (0x20), material indices are a
    // dense run from 0 upwards with no such peak.
    let mut histogram: BTreeMap<u8, usize> = BTreeMap::new();
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
            for &flags in &group.triangle_flags {
                *histogram.entry(flags).or_default() += 1;
            }
        }
    }
    let flagged: usize = histogram.values().sum();
    let mut common: Vec<(u8, usize)> = histogram.iter().map(|(&f, &n)| (f, n)).collect();
    common.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    let shown: Vec<String> = common
        .iter()
        .take(6)
        .map(|(f, n)| format!("0x{f:02X}:{:.0}%", 100.0 * *n as f32 / flagged.max(1) as f32))
        .collect();
    println!(
        "    MOPY flag bytes: {} distinct over {flagged} triangles — {}",
        histogram.len(),
        shown.join("  ")
    );
    for (path, model) in models.iter() {
        println!(
            "    {:.<46} {:6} solid of {:6} declared, {:5} verts",
            path.rsplit('\\').next().unwrap_or(path),
            model.collision.triangle_count(),
            model.collision.declared_triangles,
            model.collision.positions.len(),
        );
    }

    // Place every hull, exactly as the renderer will.
    let world = CollisionWorld::new();
    let mut placed: Vec<(String, Arc<Collider>)> = Vec::new();
    let mut contained = 0usize;
    let mut checked = 0usize;
    let mut worst_overshoot = 0.0f32;
    let mut place_time = std::time::Duration::ZERO;
    for placement in &adt.wmos {
        let Some(name) = adt.wmo_names.get(placement.name_id as usize) else {
            continue;
        };
        let Some(model) = models.get(name) else { continue };
        if model.collision.is_empty() {
            continue;
        }
        let position = placement_to_world(placement.position);
        let matrix = placement_matrix(position, placement.rotation, 1.0);
        // Timed because this is the one part of collision that runs on the
        // renderer's main thread — the hull is decoded on the loader thread with
        // the geometry, but placing it needs the `MODF` matrix, which is not
        // known until the tile spawns it. A frame is 16 ms.
        let started = std::time::Instant::now();
        let collider = Arc::new(Collider::place(&model.collision, &matrix));
        place_time += started.elapsed();

        // 2. Containment. `MODF`'s box is the `MOHD` box with its eight corners
        //    transformed, computed by Blizzard's tools from the same geometry —
        //    so a correctly placed hull sits inside it, and a hull placed
        //    through a wrong matrix cannot. One-sided on purpose: the box is
        //    *looser* than the geometry at any yaw off a right angle, so
        //    "fills it" is not a claim this can make (`vale wmos` makes it,
        //    against the corner-transformed box).
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for corner in 0..8usize {
            let local = [
                model.local_bounds[corner & 1][0],
                model.local_bounds[(corner >> 1) & 1][1],
                model.local_bounds[(corner >> 2) & 1][2],
            ];
            let world = transform_point(&matrix, local);
            for axis in 0..3 {
                lo[axis] = lo[axis].min(world[axis]);
                hi[axis] = hi[axis].max(world[axis]);
            }
        }
        let mut overshoot = 0.0f32;
        for v in collider.positions() {
            for axis in 0..3 {
                overshoot = overshoot.max(lo[axis] - v[axis]).max(v[axis] - hi[axis]);
            }
        }
        checked += 1;
        if overshoot < 1.0 {
            contained += 1;
        }
        worst_overshoot = worst_overshoot.max(overshoot);

        world.insert(
            0,
            (x, y),
            SolidId::placement(placement.unique_id),
            Solid::Building,
            Arc::clone(&collider),
        );
        placed.push((name.clone(), collider));
    }
    println!("  containment against MODF's own boxes, {checked} placements:");
    println!("    {contained} sit inside the box their placement declares, to within 1y");
    println!("    worst overshoot {worst_overshoot:.2}y  <- what the placement matrix controls");
    println!(
        "    {:.1} ms to place all {checked}, which is what a tile load costs the frame",
        place_time.as_secs_f32() * 1000.0
    );

    // 3. The floor query, against the hull's own triangles. Every solid
    //    triangle's centroid has, by construction, a surface at its own height
    //    — so `floor` must find one there, at or above it. A triangle the grid
    //    dropped is a hole in a floor, and is invisible from every other angle.
    let mut sampled = 0usize;
    let mut found = 0usize;
    let mut below = 0usize;
    for (_, collider) in &placed {
        let total = collider.triangle_count();
        // Every triangle on a house, a sample on a city.
        let stride = (total / 2000).max(1);
        for triangle in (0..total).step_by(stride) {
            let t = collider.triangle(triangle);
            let centre = [
                (t[0][0] + t[1][0] + t[2][0]) / 3.0,
                (t[0][1] + t[1][1] + t[2][1]) / 3.0,
                (t[0][2] + t[1][2] + t[2][2]) / 3.0,
            ];
            // A wall has no XY area and is correctly no floor.
            if collider.triangle_is_wall(triangle) {
                continue;
            }
            sampled += 1;
            match collider.floor(centre[0], centre[1], centre[2] + 0.01) {
                Some(z) if z >= centre[2] - 0.01 => found += 1,
                Some(_) => below += 1,
                None => {}
            }
        }
    }
    println!("  floor query against the hull's own {sampled} sampled floor triangles:");
    println!(
        "    {found} answer at the triangle's own height, {below} lower, {} not at all",
        sampled - found - below
    );

    // 4. The wall query. A vertical face, approached head-on from a yard out at
    //    a height the probes reach, must stop the stride. This is the half that
    //    decides whether the character can walk through a house.
    let mut walls = 0usize;
    let mut blocked = 0usize;
    for (_, collider) in &placed {
        let total = collider.triangle_count();
        let stride = (total / 2000).max(1);
        for triangle in (0..total).step_by(stride) {
            if !collider.triangle_is_wall(triangle) {
                continue;
            }
            let t = collider.triangle(triangle);
            let n = collider.triangle_normal(triangle);
            let centre = [
                (t[0][0] + t[1][0] + t[2][0]) / 3.0,
                (t[0][1] + t[1][1] + t[2][1]) / 3.0,
                (t[0][2] + t[1][2] + t[2][2]) / 3.0,
            ];
            // Stand so the lowest probe is exactly at the triangle's centre —
            // a two-foot-high wall panel is genuinely walked over, and counting
            // it as a miss would be measuring the probe heights, not the query.
            let feet = centre[2] - PROBE_HEIGHTS[0];
            let reach = 1.0 + PLAYER_RADIUS;
            let from = [centre[0] + n[0] * reach, centre[1] + n[1] * reach, feet];
            let to = [centre[0], centre[1], feet];
            walls += 1;
            let end = world.step(0, from, to);
            let travelled = ((end[0] - from[0]).powi(2) + (end[1] - from[1]).powi(2)).sqrt();
            if travelled < reach - 0.01 {
                blocked += 1;
            }
        }
    }
    println!("  wall query against the hull's own {walls} sampled vertical faces:");
    println!("    {blocked} stop a stride walked straight into them");

    // 5. What collision is actually *for*: standing on the building rather than
    //    on the ground under it. Sampled over each placement's footprint,
    //    because "Stormwind is above the terrain" is the whole point and no
    //    other number here says it.
    let mut samples = 0usize;
    let mut covered = 0usize;
    let mut walk_on = 0usize;
    let mut highest = 0.0f32;
    for (_, collider) in &placed {
        let b = collider.bounds();
        for row in 0..16 {
            for col in 0..16 {
                let px = b[0][0] + (b[1][0] - b[0][0]) * (col as f32 + 0.5) / 16.0;
                let py = b[0][1] + (b[1][1] - b[0][1]) * (row as f32 + 0.5) / 16.0;
                let Some(ground) = adt.height_at(px, py) else {
                    continue;
                };
                samples += 1;
                // Anything solid over this point at all — the deck of a bridge
                // fifty yards up counts, and for Stormwind most of it is.
                if let Some(z) = world.floor(0, px, py, ground + 1000.0) {
                    if z > ground + 0.05 {
                        covered += 1;
                        highest = highest.max(z - ground);
                    }
                }
                // And the same query the mover makes while standing on the
                // terrain: is there a surface close enough to step onto?
                if world
                    .floor(0, px, py, ground + STEP_UP)
                    .is_some_and(|z| z > ground + 0.05)
                {
                    walk_on += 1;
                }
            }
        }
    }
    println!("  standing, sampled over {samples} points of the placements' footprints:");
    println!(
        "    {covered} have a building surface above the terrain ({:.0}%), up to {highest:.1}y above it",
        100.0 * covered as f32 / samples.max(1) as f32
    );
    println!("    {walk_on} of those are within a step of the ground, so walked onto directly");

    // ------------------------------------------------------------------
    // 6. The doodads — the trees, the fences and the crates, plus the
    //    furniture standing inside the buildings above.
    //
    //    A different block from a WMO's and a simpler rule (no `MOPY`, every
    //    triangle solid), but the same three ways to be quietly wrong — and one
    //    more that is only a doodad's: there are a *thousand* placements to a
    //    tile rather than a dozen, so the cost of a query is now a measurement
    //    and not an assumption. That is the last number printed here.
    // ------------------------------------------------------------------
    use vale_assets::world::m2::M2;

    // The hull and, beside it, the model's **own declared bounding box** — which
    // Blizzard's tools computed from the same geometry and which nothing here
    // used to build the hull. That makes it the M2's answer to `MODF`'s box: a
    // hull put through a wrong matrix cannot sit inside it.
    type Hull = (vale_assets::world::collision::CollisionMesh, [[f32; 3]; 2]);
    let doodads = adt.placed_doodads();
    let distinct_m2: BTreeSet<String> = doodads.iter().map(|d| d.path.clone()).collect();
    let mut hulls: BTreeMap<String, Hull> = BTreeMap::new();
    let mut unreadable = 0usize;
    for path in &distinct_m2 {
        match assets.read(path).ok().and_then(|b| M2::parse(&b).ok()) {
            Some(m) => {
                hulls.insert(path.clone(), (m.collision, m.bounds));
            }
            None => unreadable += 1,
        }
    }
    let solid_models = hulls.values().filter(|(h, _)| !h.is_empty()).count();
    let m2_triangles: usize = hulls.values().map(|(h, _)| h.triangle_count()).sum();
    println!(
        "  doodads: {} placements of {} models, {unreadable} unreadable",
        doodads.len(),
        distinct_m2.len()
    );
    println!(
        "    {solid_models} of {} models carry a hull at all ({m2_triangles} triangles across them)",
        hulls.len()
    );
    println!(
        "    the other {} are what the game means by walk-through: grass, birds, flames",
        hulls.len() - solid_models
    );

    // Place them, into the same world the buildings are already in, checking
    // each against the model's own box as it goes.
    let mut m2_placed: Vec<(String, Arc<Collider>)> = Vec::new();
    let mut m2_place_time = std::time::Duration::ZERO;
    let mut m2_contained = 0usize;
    let mut m2_worst = 0.0f32;
    for placement in &doodads {
        let Some((hull, declared)) = hulls.get(&placement.path) else {
            continue;
        };
        if hull.is_empty() {
            continue;
        }
        let started = std::time::Instant::now();
        let collider = Arc::new(Collider::place(hull, &placement.matrix));
        m2_place_time += started.elapsed();

        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for corner in 0..8usize {
            let local = [
                declared[corner & 1][0],
                declared[(corner >> 1) & 1][1],
                declared[(corner >> 2) & 1][2],
            ];
            let world = transform_point(&placement.matrix, local);
            for axis in 0..3 {
                lo[axis] = lo[axis].min(world[axis]);
                hi[axis] = hi[axis].max(world[axis]);
            }
        }
        let mut overshoot = 0.0f32;
        for v in collider.positions() {
            for axis in 0..3 {
                overshoot = overshoot.max(lo[axis] - v[axis]).max(v[axis] - hi[axis]);
            }
        }
        if overshoot < 1.0 {
            m2_contained += 1;
        }
        m2_worst = m2_worst.max(overshoot);

        world.insert(
            0,
            (x, y),
            SolidId::placement(placement.unique_id),
            Solid::Doodad,
            Arc::clone(&collider),
        );
        m2_placed.push((placement.path.clone(), collider));
    }

    // And the furniture, which appears in no `MDDF` at all — it is named by the
    // `MODD` set the `MODF` row selects, and it shares its building's unique id,
    // which is exactly why [`SolidId`] carries a spawn ordinal beside it.
    let mut furniture = 0usize;
    let mut furniture_solid = 0usize;
    let mut furniture_time = std::time::Duration::ZERO;
    for placement in &adt.wmos {
        let Some(name) = adt.wmo_names.get(placement.name_id as usize) else {
            continue;
        };
        let Some(model) = models.get(name) else { continue };
        let Some(set) = model.doodad_sets.get(placement.doodad_set as usize) else {
            continue;
        };
        let start = (set.start as usize).min(model.doodads.len());
        let end = (start + set.count as usize).min(model.doodads.len());
        let position = placement_to_world(placement.position);
        let matrix = placement_matrix(position, placement.rotation, 1.0);
        for (ordinal, spawn) in model.doodads[start..end].iter().enumerate() {
            if !spawn.drawable() {
                continue;
            }
            furniture += 1;
            let (hull, _) = match hulls.entry(spawn.path.clone()) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(e) => {
                    let parsed = assets
                        .read(e.key())
                        .ok()
                        .and_then(|b| M2::parse(&b).ok())
                        .map(|m| (m.collision, m.bounds))
                        .unwrap_or_default();
                    e.insert(parsed)
                }
            };
            if hull.is_empty() {
                continue;
            }
            furniture_solid += 1;
            let placed = wmo::mul4(&matrix, &spawn.matrix);
            let started = std::time::Instant::now();
            let collider = Arc::new(Collider::place(hull, &placed));
            furniture_time += started.elapsed();
            world.insert(
                0,
                (x, y),
                SolidId::spawn(placement.unique_id, ordinal as u32),
                Solid::Doodad,
                Arc::clone(&collider),
            );
            m2_placed.push((spawn.path.clone(), collider));
        }
    }

    let (held, held_triangles) = world.counts();
    println!(
        "    {} of {} placements are solid, plus {furniture_solid} of {furniture} interior spawns",
        m2_placed.len() - furniture_solid,
        doodads.len()
    );
    // What a tile load pays the frame. Spread over the doodad pass's spawn
    // budget rather than taken at once, and against a building's own hull —
    // 27.6 ms for Stormwind's five placements — this is the smaller half.
    println!(
        "    {:.1} ms to place the {} outdoor hulls and {:.1} ms the {furniture_solid} indoor; \
         the world now holds {held} hulls, {held_triangles} triangles",
        m2_place_time.as_secs_f32() * 1000.0,
        m2_placed.len() - furniture_solid,
        furniture_time.as_secs_f32() * 1000.0,
    );
    // The check that says the furniture is not being deduplicated away: three
    // thousand chairs sharing their building's unique id used to collapse to
    // one chair per building, and nothing about the picture said so.
    println!(
        "    held = {} building + {} doodad + {furniture_solid} furniture = {}",
        placed.len(),
        m2_placed.len() - furniture_solid,
        placed.len() + m2_placed.len()
    );
    // The M2's answer to the `MODF` containment check above, and the only thing
    // here that would notice a hull placed through a wrong matrix: the box is
    // Blizzard's, computed from the same geometry, and no part of building the
    // hull consulted it.
    println!(
        "    {m2_contained} of {} sit inside the box their own model declares, worst overshoot {m2_worst:.2}y",
        m2_placed.len() - furniture_solid
    );

    // The same two queries as above, against the doodads' own triangles.
    let mut sampled = 0usize;
    let mut found = 0usize;
    let mut walls = 0usize;
    let mut blocked = 0usize;
    for (_, collider) in &m2_placed {
        for triangle in 0..collider.triangle_count() {
            let t = collider.triangle(triangle);
            let centre = [
                (t[0][0] + t[1][0] + t[2][0]) / 3.0,
                (t[0][1] + t[1][1] + t[2][1]) / 3.0,
                (t[0][2] + t[1][2] + t[2][2]) / 3.0,
            ];
            if collider.triangle_is_wall(triangle) {
                let n = collider.triangle_normal(triangle);
                let feet = centre[2] - PROBE_HEIGHTS[0];
                let reach = 1.0 + PLAYER_RADIUS;
                let from = [centre[0] + n[0] * reach, centre[1] + n[1] * reach, feet];
                walls += 1;
                let end = world.step(0, from, [centre[0], centre[1], feet]);
                let travelled = ((end[0] - from[0]).powi(2) + (end[1] - from[1]).powi(2)).sqrt();
                if travelled < reach - 0.01 {
                    blocked += 1;
                }
                continue;
            }
            sampled += 1;
            if collider
                .floor(centre[0], centre[1], centre[2] + 0.01)
                .is_some_and(|z| z >= centre[2] - 0.01)
            {
                found += 1;
            }
        }
    }
    println!("  the doodad hulls against themselves:");
    println!("    {found} of {sampled} flat triangles answer the floor query at their own height");
    println!("    {blocked} of {walls} vertical faces stop a stride walked into them");

    // **What a query now costs, with every hull on the tile loaded.** This is
    // the number the whole design turns on: the mover asks both of these twenty
    // times a second on the session thread, and a linear pass over three
    // thousand hulls is a different proposition from one over forty. Sampled
    // over the tile so most of it is the ordinary case — nothing nearby.
    let mut queries = 0usize;
    let started = std::time::Instant::now();
    for row in 0..40 {
        for col in 0..40 {
            let px = 1600.0 + 533.333_3 * (32.0 - x as f32) + 533.333_3 * (row as f32 / 40.0);
            let py = 1600.0 + 533.333_3 * (32.0 - y as f32) + 533.333_3 * (col as f32 / 40.0);
            let z = adt.height_at(px, py).unwrap_or(0.0);
            world.floor(0, px, py, z + STEP_UP);
            world.step(0, [px, py, z], [px + 0.4, py + 0.4, z]);
            queries += 1;
        }
    }
    let elapsed = started.elapsed();
    println!(
        "  {queries} floor+step pairs over the tile in {:.1} ms — {:.0} us each, against a 50 ms mover tick",
        elapsed.as_secs_f32() * 1000.0,
        elapsed.as_secs_f32() * 1.0e6 / queries as f32
    );

    if let Some((px, py, pz)) = at {
        point(&adt, &placed, &m2_placed, &world, px, py, pz);
    }

    Ok(())
}

/// The two path separators a game-data path can use, so a listing prints the
/// file's own name rather than the whole `World\\…` chain.
const SEP: [char; 2] = ['\\', '/'];

/// **What is under one point** — the whole stack, named.
///
/// Asked for by a coordinate rather than by a sweep, because that is the shape
/// the question arrives in: a creature the server puts at some z, drawn in the
/// air. Every candidate surface is listed with its own height and its distance
/// from the point, so *"the server has it standing on the abbey's step"* and
/// *"the server has it standing on nothing"* are two different read-outs
/// instead of one number.
///
/// **Give it the z as well and it answers the report directly.** A hull is
/// asked for the surface under *that* height, which is what a unit standing
/// there is on — without it the query takes whatever the hull's topmost
/// surface is, and for a building that is the roof rather than the floor the
/// creature is on. The tail line is the gap: how far the unit is off whatever
/// is beneath it, which is the number the bug report is about.
fn point(
    adt: &Adt,
    buildings: &[(String, std::sync::Arc<vale_assets::world::collision::Collider>)],
    doodads: &[(String, std::sync::Arc<vale_assets::world::collision::Collider>)],
    world: &vale_assets::world::collision::CollisionWorld,
    px: f32,
    py: f32,
    pz: Option<f32>,
) {
    use vale_assets::world::collision::STEP_UP;

    println!();
    match pz {
        Some(z) => println!("  under ({px:.2}, {py:.2}, {z:.3}):"),
        None => println!("  under ({px:.2}, {py:.2}):"),
    }
    let ground = adt.height_at(px, py);
    let gap = |h: f32| match pz {
        Some(z) => format!("{:+.3} below the point", z - h),
        None => String::new(),
    };
    match ground {
        Some(g) => println!("    terrain                                        {g:9.3}   {}", gap(g)),
        None => println!("    terrain                                          (a hole, or off this tile)"),
    }

    // With a z, the surface a unit standing *there* is on — a step above the
    // feet, which is `Standing::floor`'s own rule. Without one, anything the
    // tile could hold, so nothing is filtered out.
    let (_, hi) = adt.height_range();
    let ceiling = pz.map_or(hi + 100.0, |z| z + STEP_UP);
    let mut found = 0usize;
    for (what, list) in [("building", buildings), ("doodad", doodads)] {
        for (name, collider) in list {
            let Some(z) = collider.floor(px, py, ceiling) else {
                continue;
            };
            found += 1;
            let short = name.rsplit(SEP).next().unwrap_or(name);
            println!("    {what:<9} {short:<36} {z:9.3}   {}", gap(z));
        }
    }
    if found == 0 {
        println!("    no building or doodad hull answers here — this is bare ground");
    }

    // **…and what is merely *drawn* nearby, hull or no hull.** A creature the
    // server has put in the air is often standing on a crate, a rock or a
    // haystack the game means to be walked through — visible, solid-looking and
    // carrying no `BoundingTriangles` at all — so "no hull answers" is only half
    // an answer. Within five yards in plan, which is about as far as a model can
    // be from its own origin and still be underfoot.
    const NEAR: f32 = 5.0;
    let mut nearby = 0usize;
    for placement in &adt.doodads {
        let world = vale_assets::world::adt::placement_to_world(placement.position);
        let (dx, dy) = (world[0] - px, world[1] - py);
        if dx.hypot(dy) > NEAR {
            continue;
        }
        let name = adt
            .model_names
            .get(placement.name_id as usize)
            .map(String::as_str)
            .unwrap_or("(unnamed)");
        let short = name.rsplit(SEP).next().unwrap_or(name);
        nearby += 1;
        println!(
            "    drawn      {short:<36} {:9.3}   {:.1}y away, scale {:.2}",
            world[2],
            dx.hypot(dy),
            placement.scale
        );
    }
    if nearby == 0 {
        println!("    nothing is drawn within {NEAR:.0} yards of it either");
    }

    // **…and whether a character standing there is under open sky**, which is
    // the question a mount asks: the building floor a yard over the feet, its
    // group's `MOGP` flags, and the server's rule over both
    // (`wmo::outdoors_at`). Only with a z, since the answer is about a floor
    // and a floor is chosen by height.
    if let Some(z) = pz {
        let probe = z + 1.0;
        let floor = world.building_floor(0, px, py, probe);
        let outdoors = vale_assets::world::wmo::outdoors_at(floor, ground, probe);
        match floor {
            Some((h, flags)) => println!(
                "    open sky: {} — building floor at {h:.3}, group flags {flags:#010x} (0x8000 {})",
                if outdoors { "yes, a mount may be summoned" } else { "no, this is indoors" },
                if flags & vale_assets::world::wmo::group_flags::OUTDOOR != 0 { "set" } else { "clear" },
            ),
            None => println!("    open sky: yes — no building floor under the probe"),
        }
        // **What the question costs**, because the area tracker asks it every
        // frame. Timed over the same world the answer came from; the result is
        // kept so the loop cannot be optimised away.
        const ASKS: u32 = 100_000;
        let started = std::time::Instant::now();
        let mut open = 0u32;
        for _ in 0..ASKS {
            let floor = world.building_floor(0, px, py, std::hint::black_box(probe));
            open += u32::from(vale_assets::world::wmo::outdoors_at(floor, ground, probe));
        }
        let each = started.elapsed().as_secs_f64() * 1e6 / f64::from(ASKS);
        println!("    asked {ASKS} times: {each:.3} µs each ({open} answered open sky)");
    }

    // …and what the client would actually stand a character on, which is the
    // terrain unless a hull answers within a step above the feet.
    if let Some(g) = ground {
        let solid = world.floor(0, px, py, g + STEP_UP);
        match solid {
            Some(z) => println!("    the mover, standing on the terrain, is put at {z:9.3} (a hull within a step)"),
            None => println!("    the mover, standing on the terrain, is put at {g:9.3} (nothing within a step)"),
        }
    }
}
