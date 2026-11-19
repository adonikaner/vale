//! `vale ships`: the boats and the zeppelins, checked with no session.
//!
//! ```text
//! vale ships        every shipped route: its schedule, its curve, and the
//!                      shape checks over all of them
//! vale ships 302    trace one: every key frame, and the route walked
//!                      minute by minute across whichever maps it touches
//! ```
//!
//! ## Why the routes are checked here
//!
//! A continent transport's position is stated by no packet.
//! `GameObject::GetStationaryX` returns a literal `0.f` for
//! `GAMEOBJECT_TYPE_MO_TRANSPORT`, and that zero is what
//! `UPDATEFLAG_TRANSPORT` puts on the wire. A client with the wrong schedule
//! therefore draws the boat at the map's origin, or far from its route, or not
//! at all, and there is no packet to compare against and no error raised.
//!
//! The checks are shape checks against things this project did not produce,
//! as in `vale taxi`:
//!
//! * Every route resolves: a `taxiPathId` with waypoints, enough of them to
//!   make a curve, and a non-zero period.
//! * Every key frame stands on a map `Map.dbc` ships, and the set of maps a
//!   route touches is the set its waypoints name.
//! * The route is walked end to end and stays on its own waypoints. Every
//!   sample must be within a few yards of the polyline through the frames it
//!   lies between. This fails for a spline evaluated at the wrong index, a
//!   schedule that runs backwards, and a segment fraction outside `0..1`. A
//!   Catmull-Rom curve leaves its polyline only on a corner, so the bound is
//!   generous and still fails on each of those.
//! * The clock is monotonic across the frames and ends at the period.
//! * The hull turns with the curve. The heading is sampled every
//!   [`TURN_MS`], and a step that turns the hull more than [`SNAP_DEGREES`]
//!   is counted. A heading taken from each segment's chord jumps at every
//!   node, by up to 76 degrees on the shipped routes; the curve's tangent
//!   does not. See [`turns`].
//! * The model exists in the archives, which decides whether anything is
//!   drawn.
//! * The bow leads. No file states this rule, and a ship that breaks it
//!   sails backwards and fails nothing else. See [`front_of`].
//!
//! The routes are not read from a config. They are the nine
//! `gameobject_template` rows the server ships, which this client learns only
//! from `SMSG_GAMEOBJECT_QUERY_RESPONSE`. They are written out in [`SHIPPED`]
//! with their template words, so this command runs with no server and no
//! session.

use crate::common::*;
use vale_assets::tables::shiptransport::{MoTransport, Route};
use vale_assets::tables::taxi::TaxiTables;
use vale_config::Config;

/// The nine transports 1.12 ships, with the three template words and the
/// period of each.
///
/// The client does not read this table at run time. A live session learns
/// the three words from `SMSG_GAMEOBJECT_QUERY_RESPONSE`, whose `data[0..2]`
/// for a type 15 template are these numbers. They are here so the arithmetic
/// can be checked against the shipped DBCs with nothing running.
///
/// Taken from the world database's `transports` joined to
/// `gameobject_template`. Each of the nine states `moveSpeed` 30 and
/// `accelRate` 1 except Naxxramas, a platform that never departs.
const SHIPPED: &[(u32, &str, MoTransport, u32)] = &[
    (20808, "TEST Ship", tmpl(241, 30, 1), 350_818),
    (164871, "Zeppelin - Orgrimmar to Undercity", tmpl(302, 30, 1), 356_284),
    (175080, "Zeppelin - Grom'Gol to Orgrimmar", tmpl(285, 30, 1), 303_463),
    (176231, "Proudmore's Treasure", tmpl(292, 30, 1), 329_313),
    (176244, "Moonspray", tmpl(293, 30, 1), 316_251),
    (176310, "Serenity's Shore", tmpl(295, 30, 1), 295_579),
    (176495, "Zeppelin - Grom'Gol to Undercity", tmpl(301, 30, 1), 333_044),
    (177233, "Feathermoon Ferry", tmpl(303, 30, 1), 317_040),
    (181056, "Naxxramas", tmpl(436, 1, 1), 1_208_014),
];

const fn tmpl(taxi_path: u32, move_speed: u32, accel_rate: u32) -> MoTransport {
    MoTransport { taxi_path, move_speed, accel_rate }
}

/// How far a sample may sit from the polyline through its own key frames, in
/// yards.
///
/// A Catmull-Rom curve leaves its control polygon on a corner, and the routes
/// have some sharp ones, so the bound is generous. It tests that the ship is
/// on its route at all: every wrong schedule found so far misses by hundreds
/// of yards or more.
const ON_ROUTE: f32 = 40.0;

/// How often the route is sampled, in milliseconds. Fine enough that a 30 y/s
/// ship moves 15 yards between samples.
const SAMPLE_MS: u32 = 500;

/// How often the heading is sampled, in milliseconds: 3 yards of travel at
/// cruise.
const TURN_MS: u32 = 100;

/// Degrees of yaw in one [`TURN_MS`] step above which the step is counted as
/// a snap. A ship at cruise on the sharpest shipped corner turns less than
/// this in 3 yards.
const SNAP_DEGREES: f32 = 10.0;

pub fn cmd_ships(cfg: &Config, which: Option<u32>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let mut read = |name: &str| {
        assets
            .read(&vale_assets::tables::dbc::dbc_path(name))
            .unwrap_or_default()
    };
    let (nodes, paths, path_nodes, continents) = (
        read("TaxiNodes"),
        read("TaxiPath"),
        read("TaxiPathNode"),
        read("WorldMapContinent"),
    );
    let tables = TaxiTables::parse(&nodes, &paths, &path_nodes, &continents)
        .ok_or("the taxi tables did not load — see TaxiTables::parse, which is all four or none")?;
    let maps = assets
        .read(&vale_assets::tables::dbc::dbc_path("Map"))
        .ok()
        .and_then(|raw| vale_assets::tables::dbc::map_directories(&raw).ok())
        .unwrap_or_default();

    match which {
        Some(id) => trace(&tables, &maps, id),
        None => survey(&tables, &maps, &mut assets),
    }
}

/// Every route, with its schedule and the checks.
fn survey(
    tables: &TaxiTables,
    maps: &std::collections::HashMap<u32, String>,
    assets: &mut vale_assets::Assets,
) -> Result<(), String> {
    println!("the transports 1.12 ships — {} of them\n", SHIPPED.len());

    let (mut resolved, mut off_route, mut bad_map, mut not_monotonic) = (0, 0, 0, 0);
    let mut wrong_period = 0;
    let mut worst_off = 0.0f32;
    let (mut snaps, mut worst_turn) = (0usize, 0.0f32);
    for (entry, name, template, period) in SHIPPED {
        let Some(path) = tables.waypoints(template.taxi_path) else {
            println!("  {entry:>6} {name:<38} !! TaxiPath {} has no waypoints", template.taxi_path);
            continue;
        };
        let Some(route) = Route::build(path, *template) else {
            println!(
                "  {entry:>6} {name:<38} !! {} nodes made no route",
                path.len()
            );
            continue;
        };
        resolved += 1;

        let legs = route.legs();
        let waits: usize = legs
            .iter()
            .map(|leg| leg.stops().iter().filter(|s| s.delay_ms > 0).count())
            .sum();
        let mut route_maps: Vec<u32> = legs.iter().map(|leg| leg.map()).collect();
        route_maps.sort_unstable();
        route_maps.dedup();
        let names: Vec<String> = route_maps
            .iter()
            .map(|m| maps.get(m).cloned().unwrap_or_else(|| format!("map {m}")))
            .collect();
        let drift = route.period() as i64 - *period as i64;
        if drift != 0 {
            wrong_period += 1;
        }
        println!(
            "  {entry:>6} {name:<38} path {:>3}  {} legs, {waits} wait  {:>9} ms  {:>+5}  {}",
            template.taxi_path,
            legs.len(),
            route.period(),
            drift,
            names.join(" + "),
        );

        for leg in legs {
            if !maps.contains_key(&leg.map()) {
                bad_map += 1;
            }
            // The clock only ever runs forwards inside a leg, and its last stop
            // is the leg's own end.
            let mut previous = leg.start_ms();
            for stop in leg.stops() {
                if stop.time_ms < previous {
                    not_monotonic += 1;
                }
                previous = stop.time_ms + stop.delay_ms;
            }
            if leg.end_ms() < previous {
                not_monotonic += 1;
            }
        }
        if legs.last().is_some_and(|leg| leg.end_ms() != route.period()) {
            not_monotonic += 1;
        }

        // The route walked, against the polyline through its own nodes.
        let (missed, worst) = walk(&route);
        off_route += missed;
        worst_off = worst_off.max(worst);
        let (snapped, sharpest) = turns(&route);
        snaps += snapped;
        worst_turn = worst_turn.max(sharpest);
    }

    println!();
    println!("  {resolved} of {} routes resolve", SHIPPED.len());
    println!("  {wrong_period} periods that are not the reference client's own");
    println!("  {bad_map} legs standing on a map Map.dbc does not have");
    println!("  {not_monotonic} routes whose clock is not monotonic or does not end at its period");
    println!(
        "  {off_route} samples further than {ON_ROUTE:.0}y from the polyline through their own nodes, worst {worst_off:.1}y"
    );
    println!(
        "  {snaps} steps of {TURN_MS} ms that turn the hull more than {SNAP_DEGREES:.0} degrees, sharpest {worst_turn:.1}"
    );

    // The models, which decide whether anything is drawn.
    println!();
    println!("  the models, against the archives:");
    let mut backwards = 0;
    for path in [
        "World\\wmo\\transports\\transport_ship\\transportship.wmo",
        "World\\wmo\\transports\\transport_zeppelin\\transport_zeppelin.wmo",
        "World\\wmo\\transports\\BlackCitadel\\BlackCitadel.wmo",
    ] {
        let short = path.rsplit('\\').next().unwrap_or(path);
        match vale_assets::world::wmo::load(assets, path) {
            Ok((model, _)) => {
                let hull: usize = model.collision.indices.len() / 3;
                // What it costs to move the hull, which decides how a moving
                // building is collided with: a ship travels 30 y/s, so its
                // hull is somewhere new every frame, and `Collider::place`
                // transforms and re-indexes every triangle. Ten placements
                // are timed, because the allocator dominates the time of one.
                let matrix = vale_assets::world::collision::object_matrix(
                    [0.0, 0.0, 0.0],
                    0.7,
                    1.0,
                );
                let started = std::time::Instant::now();
                const RUNS: u32 = 10;
                for _ in 0..RUNS {
                    std::hint::black_box(
                        vale_assets::world::collision::Collider::place(&model.collision, &matrix),
                    );
                }
                let each = started.elapsed().as_secs_f32() * 1000.0 / RUNS as f32;
                println!(
                    "    {short:<30} {:>6} draw batches, {hull:>6} solid triangles, {each:>5.2} ms to place",
                    model.draws.len()
                );
                // Which way the model points, which no measurement of the
                // route shows.
                match front_of(&model.positions) {
                    Some((front, taper)) => {
                        let leads = front < 0.0;
                        if !leads {
                            backwards += 1;
                        }
                        println!(
                            "      front at local x {front:+6.1} ({taper}), so `Rz(o)` puts it {} the heading — {}",
                            if leads { "along" } else { "against" },
                            if leads { "bow leads" } else { "!! SAILING BACKWARDS" },
                        );
                    }
                    // Naxxramas is a platform and not a vessel. Its route has
                    // one stop and it never departs, so it has no heading to
                    // lead with.
                    None => println!("      both ends alike: no bow to lead, and it never departs"),
                }
            }
            Err(e) => println!("    {short:<30} !! {e}"),
        }
    }
    println!();
    println!("  {backwards} transport models whose front does not lead");
    Ok(())
}

/// Which end of a transport model is its front, as a local x, with what was
/// compared to decide it.
///
/// No file states this rule, and a model placed the wrong way round sails
/// backwards and fails nothing else, so it is measured. Two facts decide it:
///
/// * the server's stored orientation for a ship is `atan2(dir.y, dir.x) + M_PI`
///   (`ShipTransport::Update`), a half-turn from the direction of travel;
/// * the client places it with `Rz(o)` and nothing else
///   (`collision::object_matrix`). The `Rz(pi)` in `adt::placement_matrix`
///   is an `MDDF`/`MODF` row's own encoding, and a server-placed object does
///   not go through that frame.
///
/// `Rz(o)` therefore sends local `-x` along the heading, and the model's front
/// has to be there. Both shipped hulls have it there: a ship's bowsprit and a
/// zeppelin's nose are each a thin spike off the negative end, and the other
/// end is the broad transom or the tail fins. The measurement is that
/// asymmetry, the width of the outermost tenth at each end, because a
/// bounding box does not show it.
///
/// Answers `(the front's local x, what was compared)`, or `None` for a hull
/// whose two ends are alike. That is Naxxramas, a platform that never departs
/// its one stop and has no heading.
fn front_of(positions: &[[f32; 3]]) -> Option<(f32, String)> {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for v in positions {
        lo = lo.min(v[0]);
        hi = hi.max(v[0]);
    }
    // The width of the outermost tenth at each end, in the across-ship axis.
    let tip = |from: f32, to: f32| {
        let (a, b) = (from.min(to), from.max(to));
        let (mut ylo, mut yhi) = (f32::MAX, f32::MIN);
        for v in positions.iter().filter(|v| v[0] >= a && v[0] <= b) {
            ylo = ylo.min(v[1]);
            yhi = yhi.max(v[1]);
        }
        if ylo > yhi { 0.0 } else { yhi - ylo }
    };
    let tenth = (hi - lo) * 0.1;
    let (front_width, back_width) = (tip(lo, lo + tenth), tip(hi - tenth, hi));
    let (front, narrow, broad) = if front_width < back_width {
        (lo, front_width, back_width)
    } else {
        (hi, back_width, front_width)
    };
    // An end is a bow only when it is at least twice as narrow as the other.
    // The ship's is 34 times narrower and the zeppelin's has no width.
    // Naxxramas is 165.1 against 166.8, a 1% difference that would otherwise
    // be reported as a front.
    if narrow * 2.0 > broad {
        return None;
    }
    Some((front, format!("{narrow:.1}y wide against {broad:.1}y at the other end")))
}

/// Walk a route at [`SAMPLE_MS`] and check each sample against the polyline
/// through the key frames it lies between.
///
/// Answers `(samples that missed, the worst distance)`.
fn walk(route: &Route) -> (usize, f32) {
    let (mut missed, mut worst) = (0usize, 0.0f32);
    let mut progress = 0;
    while progress < route.period() {
        if let Some(at) = route.at(progress) {
            // The nearest segment of the route's own polyline, on the same map.
            let mut best = f32::MAX;
            for leg in route.legs() {
                if leg.map() != at.map {
                    continue;
                }
                for pair in leg.spline().points().windows(2) {
                    best = best.min(point_to_segment(at.pos, pair[0], pair[1]));
                }
            }
            if best.is_finite() {
                worst = worst.max(best);
                if best > ON_ROUTE {
                    missed += 1;
                }
            }
        }
        progress += SAMPLE_MS;
    }
    (missed, worst)
}

/// Walk a route at [`TURN_MS`] and measure how far the heading turns in each
/// step.
///
/// A step that changes map, or moves the ship further than it can travel in
/// the step, is a teleport between legs and is not a turn. Answers `(steps
/// that turn more than [`SNAP_DEGREES`], the largest turn in degrees)`.
fn turns(route: &Route) -> (usize, f32) {
    let (mut snaps, mut worst) = (0usize, 0.0f32);
    let mut previous: Option<vale_assets::tables::shiptransport::Placement> = None;
    let mut progress = 0;
    while progress < route.period() {
        let at = route.at(progress);
        if let (Some(was), Some(now)) = (previous, at) {
            let moved = (0..3).map(|n| (now.pos[n] - was.pos[n]).powi(2)).sum::<f32>().sqrt();
            if was.map == now.map && moved < 50.0 {
                let tau = std::f32::consts::TAU;
                let turn = (now.facing - was.facing).rem_euclid(tau);
                let turn = turn.min(tau - turn).to_degrees();
                worst = worst.max(turn);
                if turn > SNAP_DEGREES {
                    snaps += 1;
                }
            }
        }
        previous = at;
        progress += TURN_MS;
    }
    (snaps, worst)
}

/// One route in full.
fn trace(
    tables: &TaxiTables,
    maps: &std::collections::HashMap<u32, String>,
    which: u32,
) -> Result<(), String> {
    // By entry or by taxi path, whichever the number matches. The entry is
    // what the wire carries and the path id is what the table is keyed by, and
    // the survey prints both.
    let (entry, name, template, period) = SHIPPED
        .iter()
        .find(|(entry, _, t, _)| *entry == which || t.taxi_path == which)
        .ok_or_else(|| format!("no shipped transport with entry or taxi path {which}"))?;

    let path = tables
        .waypoints(template.taxi_path)
        .ok_or_else(|| format!("TaxiPath {} has no waypoints", template.taxi_path))?;
    let route = Route::build(path, *template)
        .ok_or_else(|| format!("TaxiPath {} made no route", template.taxi_path))?;

    println!("{name}  (entry {entry}, taxi path {})", template.taxi_path);
    println!(
        "  {} waypoints -> {} legs, cycle {} ms against the reference's {period} ms, {} y/s at {} y/s^2",
        path.len(),
        route.legs().len(),
        route.period(),
        template.move_speed,
        template.accel_rate,
    );

    for (index, leg) in route.legs().iter().enumerate() {
        let map = maps
            .get(&leg.map())
            .cloned()
            .unwrap_or_else(|| format!("map {}", leg.map()));
        println!();
        println!(
            "  leg {index} on {map}: {} nodes, {:.0}y of curve, {} ms travelling, {} .. {} ms",
            leg.spline().points().len(),
            leg.spline().total(),
            leg.travel_ms(),
            leg.start_ms(),
            leg.end_ms(),
        );
        println!(
            "    {:>8} {:>10} {:>8}   {:>9} {:>9} {:>8}",
            "arrive", "distance", "wait", "x", "y", "z"
        );
        for stop in leg.stops() {
            // Read from the leg's own curve and not through `Route::at`: a
            // leg's last stop is timed at the moment the next leg begins, so
            // the route would answer with the far continent.
            let at = leg.spline().along(stop.dist);
            println!(
                "    {:>8} {:>10.1} {:>8}   {:>9.1} {:>9.1} {:>8.1}",
                stop.time_ms,
                stop.dist,
                stop.delay_ms,
                at[0],
                at[1],
                at[2],
            );
        }
    }

    println!();
    println!("  the route walked, every 15 s:");
    let mut progress = 0;
    while progress < route.period() {
        if let Some(at) = route.at(progress) {
            let map = maps
                .get(&at.map)
                .cloned()
                .unwrap_or_else(|| format!("map {}", at.map));
            println!(
                "    {:>7.1} s  {map:<14} {:>9.1} {:>9.1} {:>8.1}  facing {:>5.2}",
                progress as f32 / 1000.0,
                at.pos[0],
                at.pos[1],
                at.pos[2],
                at.facing,
            );
        }
        progress += 15_000;
    }

    let (missed, worst) = walk(&route);
    println!();
    println!(
        "  {missed} samples further than {ON_ROUTE:.0}y from its own polyline, worst {worst:.1}y"
    );
    let (snaps, sharpest) = turns(&route);
    println!(
        "  {snaps} steps of {TURN_MS} ms that turn the hull more than {SNAP_DEGREES:.0} degrees, sharpest {sharpest:.1}"
    );
    Ok(())
}

/// Distance from a point to a segment, in three dimensions.
fn point_to_segment(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    let t = if len2 < 1.0e-6 {
        0.0
    } else {
        ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2).clamp(0.0, 1.0)
    };
    let near = [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t];
    ((p[0] - near[0]).powi(2) + (p[1] - near[1]).powi(2) + (p[2] - near[2]).powi(2)).sqrt()
}
