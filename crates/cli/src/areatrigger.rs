//! `vale triggers` — **the volumes an instance portal is made of**, checked
//! with no session.
//!
//! Nothing about area triggers crosses the wire in the direction that could be
//! checked against a reply: the client reads
//! [`vale_assets::tables::areatrigger`], decides it is inside one, and sends four
//! bytes the server either acts on or silently drops. So a column read wrong
//! does not error and does not produce a wrong-looking number — it produces a
//! **portal that does nothing**, which is indistinguishable from the bug that
//! subject started as. This is what makes it loud.
//!
//! ```text
//! vale triggers        the whole table, the partition, and the four checks
//! vale triggers 78     trace one: its volume, its map, its tile, its area
//! ```
//!
//! ## The four checks are shape checks
//!
//! None of them needs to know where the Deadmines are. Each is a property the
//! table must have if it is being read as a table at all, and each fails loudly
//! for a specific misreading:
//!
//! * **Every map is one `Map.dbc` ships.** The map column is field 1, right
//!   beside the id at 0 and the x at 2, and reading either neighbour instead
//!   gives map ids in the thousands or the low millions.
//! * **Every position lands on a tile its own map actually has.** This is the
//!   coordinate check, and it is the strong one: fields 2, 3 and 4 are three
//!   floats in a row, and any rotation of them puts triggers off the edge of
//!   the world in numbers. Maps whose tile grid is full or empty cannot
//!   discriminate and are excluded, exactly as `vale light` excludes them.
//! * **The sphere/box partition is clean.** `radius > 0` is the whole
//!   discriminator the client uses, so a row with both, or a box
//!   with a zero extent, would mean the two blocks are not where they are
//!   thought to be.
//! * **A probe just inside each volume is inside and one just outside is
//!   outside** — run through the same [`vale_assets::tables::areatrigger`] the
//!   session calls, and for a box run along the box's **own** axes. That last
//!   part is the point: a containment test that dropped the yaw passes every
//!   axis-aligned probe and fails only the turned ones, and the table has
//!   enough turned boxes to say so.
//!
//! ## …and the table is sorted by map, which is load-bearing in the reference
//!
//! The reference partitions by scanning for the first row of the current map and
//! stopping at the first row past it. This client groups explicitly instead and
//! does not depend on the order, but the order is reported anyway, because it is
//! the invariant the *reference* has and a file that broke it would mean this
//! reading of the table is wrong somewhere else.

use crate::common::open_assets;
use vale_config::Config;
use vale_assets::tables::areatrigger::{AreaTrigger, AreaTriggers};
use vale_assets::world::terrain::tile_for_position;

/// Entry point for both forms.
pub fn cmd_triggers(cfg: &Config, id: Option<u32>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let raw = assets
        .read(&vale_assets::tables::dbc::dbc_path("AreaTrigger"))
        .map_err(|e| format!("AreaTrigger.dbc: {e}"))?;
    let table = AreaTriggers::load(&raw).map_err(|e| format!("AreaTrigger.dbc: {e}"))?;

    let maps = assets
        .read(&vale_assets::tables::dbc::dbc_path("Map"))
        .ok()
        .and_then(|raw| vale_assets::tables::dbc::map_directories(&raw).ok())
        .unwrap_or_default();

    match id {
        Some(id) => trace(&table, &maps, cfg, id),
        None => survey(&table, &maps, &mut assets),
    }
}

/// A yard offset along the trigger's own local x, turned into world axes.
///
/// The whole reason the probe is written this way: for a box with a yaw, an
/// offset along **world** x is not an offset along the box's long side, so a
/// probe built that way tests nothing about the rotation.
fn along_local_x(trigger: &AreaTrigger, distance: f32) -> [f32; 3] {
    let (sin, cos) = trigger.yaw.sin_cos();
    [
        trigger.position[0] + distance * cos,
        trigger.position[1] + distance * sin,
        trigger.position[2],
    ]
}

/// Half the trigger's reach along its own local x — the radius for a sphere,
/// half the declared length for a box.
fn reach(trigger: &AreaTrigger) -> f32 {
    if trigger.is_sphere() {
        trigger.radius
    } else {
        trigger.extent[0] / 2.0
    }
}

fn survey(
    table: &AreaTriggers,
    maps: &std::collections::HashMap<u32, String>,
    assets: &mut vale_assets::Assets,
) -> Result<(), String> {
    let rows = table.rows();
    let spheres = rows.iter().filter(|t| t.is_sphere()).count();
    let boxes = rows.len() - spheres;
    println!(
        "AreaTrigger.dbc: {} rows over {} maps — {spheres} spheres, {boxes} boxes",
        rows.len(),
        {
            let mut seen: Vec<u32> = rows.iter().map(|t| t.map).collect();
            seen.sort_unstable();
            seen.dedup();
            seen.len()
        }
    );

    // ---- the partition ----------------------------------------------------
    //
    // **The interesting number here is `both`, and it is not zero.** 38 rows
    // carry a real radius *and* an extent, which is why the discriminator has
    // to be the client's own `radius > 0` rather than "does it have an extent":
    // read the other way round, thirty spheres of up to twenty yards become
    // boxes a third of a yard across that nobody could stand in, and those
    // triggers simply never fire.
    //
    // The split matters too, and the second half of it was a surprise. Thirty
    // of the 38 are **vestigial** — a uniform cube of 0.2778 or 0.3333 yards,
    // authoring leftovers rather than volumes. The other **eight carry a
    // genuine box beside a genuine radius**, up to 9.8 x 17.9 x 27.9 against a
    // 10-yard sphere (row 2221, Stratholme), so the two shapes in that row
    // really do disagree and the client silently takes the sphere. Nothing to
    // fix — but it is the difference between "the box column is noise" and
    // "eight triggers in the game are a different shape than the file's other
    // half says", and only the first of those was true before this printed.
    const VESTIGIAL: f32 = 1.0;
    let extents: Vec<f32> = rows
        .iter()
        .filter(|t| t.is_sphere())
        .map(|t| t.extent.iter().copied().fold(0.0f32, f32::max))
        .filter(|widest| *widest > 0.0)
        .collect();
    let both = extents.len();
    let vestigial = extents.iter().filter(|w| **w < VESTIGIAL).count();
    let widest = extents.iter().copied().fold(0.0f32, f32::max);
    let flat = rows
        .iter()
        .filter(|t| !t.is_sphere() && t.extent.contains(&0.0))
        .count();
    let turned = rows.iter().filter(|t| !t.is_sphere() && t.yaw != 0.0).count();
    println!(
        "\n  partition: {both} sphere(s) also carry an extent — {vestigial} vestigial (under a \
         yard), {} substantial, widest side {widest:.2} yards.",
        both - vestigial
    );
    println!("             The discriminator is the radius, so all {both} are spheres and the box is ignored.");
    println!("             {flat} box(es) have a zero side  [should be 0]");
    println!("             {turned} of the {boxes} boxes are turned, which is what makes the probe below mean anything");

    // ---- the order --------------------------------------------------------
    let sorted = rows.windows(2).all(|w| w[0].map <= w[1].map);
    println!(
        "  order:     sorted by map: {}  [the reference partitions by scanning, so it must be]",
        if sorted { "yes" } else { "NO" }
    );

    // ---- the maps ---------------------------------------------------------
    let unknown: Vec<u32> = {
        let mut bad: Vec<u32> = rows
            .iter()
            .map(|t| t.map)
            .filter(|m| !maps.contains_key(m))
            .collect();
        bad.sort_unstable();
        bad.dedup();
        bad
    };
    // Three rows, on maps 24 and 28, which `Map.dbc` does not ship at all —
    // so they are unreachable rather than misread, and the count is reported
    // rather than treated as a failure. What would be a failure is *many* of
    // them, or ids in the thousands, which is what reading a neighbouring
    // column produces.
    println!(
        "  maps:      {} row(s) name a map Map.dbc does not have{} — unreachable, not misread",
        rows.iter().filter(|t| !maps.contains_key(&t.map)).count(),
        if unknown.is_empty() {
            String::new()
        } else {
            format!(" ({unknown:?})")
        }
    );

    // ---- the probe --------------------------------------------------------
    let mut inside_ok = 0usize;
    let mut outside_ok = 0usize;
    let mut probed = 0usize;
    for trigger in rows {
        let half = reach(trigger);
        if half <= 0.0 {
            continue;
        }
        probed += 1;
        // 0.9 and 1.1 rather than 0.99 and 1.01: the extents are stored as f32
        // and a probe on the face is a coin toss, which would report a failure
        // that is arithmetic rather than a misread column.
        inside_ok += usize::from(
            table.holds(trigger.id, trigger.map, along_local_x(trigger, half * 0.9)),
        );
        outside_ok += usize::from(
            !table.holds(trigger.id, trigger.map, along_local_x(trigger, half * 1.1)),
        );
    }
    println!(
        "  probe:     {inside_ok}/{probed} contain a point at 0.9 of their own reach, \
         {outside_ok}/{probed} reject one at 1.1"
    );

    // ---- the positions ----------------------------------------------------
    //
    // Excluded exactly as `vale light` excludes them: a map with no ADT
    // tiles can answer nothing, and one whose grid is full answers "yes" to any
    // position at all.
    println!("\n  per map, how many centres land on one of that map's own tiles:");
    let mut map_ids: Vec<u32> = maps.keys().copied().collect();
    map_ids.sort_unstable();
    let (mut on_tile, mut checked) = (0usize, 0usize);
    for map in map_ids {
        let here: Vec<&AreaTrigger> = table.on_map(map).collect();
        if here.is_empty() {
            continue;
        }
        let directory = &maps[&map];
        let wdt = assets
            .read(&format!("World\\Maps\\{directory}\\{directory}.wdt"))
            .ok()
            .and_then(|raw| vale_assets::world::wdt::Wdt::parse(&raw).ok());
        let Some(wdt) = wdt else {
            println!("  {map:4} {directory:<22} {:4} triggers   no WDT", here.len());
            continue;
        };
        let tiles = wdt.tile_count();
        let misses: Vec<u32> = here
            .iter()
            .filter(|t| {
                let (tx, ty) = tile_for_position(t.position[0], t.position[1]);
                !wdt.has_tile[ty as usize][tx as usize]
            })
            .map(|t| t.id)
            .collect();
        let hits = here.len() - misses.len();
        // A map whose grid is empty or full cannot discriminate.
        let discriminates = tiles > 0 && tiles < 64 * 64;
        if discriminates {
            on_tile += hits;
            checked += here.len();
        }
        println!(
            "  {map:4} {directory:<22} {:4} triggers  {hits:4} on a tile  of {tiles} tiles{}{}",
            here.len(),
            if discriminates { "" } else { "   (cannot discriminate)" },
            // Named rather than counted: a miss is either a trigger over a hole
            // in the map or a coordinate read wrong, and only the id tells you
            // which without a second run.
            if discriminates && !misses.is_empty() {
                format!("   off-tile: {misses:?}")
            } else {
                String::new()
            }
        );
    }
    if checked > 0 {
        println!(
            "\n  {on_tile} of {checked} centres on discriminating maps land on a tile that map ships \
             ({:.0}%)",
            100.0 * on_tile as f32 / checked as f32
        );
    }
    Ok(())
}

fn trace(
    table: &AreaTriggers,
    maps: &std::collections::HashMap<u32, String>,
    cfg: &Config,
    id: u32,
) -> Result<(), String> {
    let Some(trigger) = table.rows().iter().find(|t| t.id == id) else {
        return Err(format!("no area trigger with id {id}"));
    };
    let unknown = String::from("?");
    let directory = maps.get(&trigger.map).unwrap_or(&unknown);
    println!("area trigger {id}  on map {} ({directory})", trigger.map);
    println!(
        "  centre  {:.2}, {:.2}, {:.2}",
        trigger.position[0], trigger.position[1], trigger.position[2]
    );
    if trigger.is_sphere() {
        println!("  sphere  radius {:.2} yards", trigger.radius);
    } else {
        println!(
            "  box     {:.2} x {:.2} x {:.2} yards, yaw {:.3} rad ({:.1} deg)",
            trigger.extent[0],
            trigger.extent[1],
            trigger.extent[2],
            trigger.yaw,
            trigger.yaw.to_degrees()
        );
    }
    let (tx, ty) = tile_for_position(trigger.position[0], trigger.position[1]);
    println!("  tile    {directory} {tx} {ty}");

    // What the *ground* under it is called, which is the one thing here that
    // reads a second file — and the readable confirmation that the coordinates
    // are being taken in the right frame.
    let terrain = vale_assets::MapTerrain::open(&cfg.gamedata).ok();
    let area = terrain
        .as_ref()
        .and_then(|t| t.area_at(trigger.map, trigger.position[0], trigger.position[1]));
    match area {
        Some(area) => println!("  area    {area}"),
        None => println!("  area    (no tile loaded there)"),
    }
    // **How far the volume is from the open ground**, which decides whether it
    // is reachable by anything with terrain-only collision. A dungeon entrance
    // is usually in a cave or a building and sits tens of yards *under* the
    // terrain surface — the Deadmines' is 26 below — so `vale live`, which
    // has no WMO hulls, cannot walk into one and the renderer is the only thing
    // that can. Printed here so that is a number rather than a surprise.
    if let Some(ground) = terrain
        .as_ref()
        .and_then(|t| t.height_at(trigger.map, trigger.position[0], trigger.position[1]))
    {
        let drop = trigger.position[2] - ground;
        println!(
            "  ground  terrain is at {ground:.2} — the centre is {:.2} yards {} it{}",
            drop.abs(),
            if drop >= 0.0 { "above" } else { "below" },
            if drop.abs() > reach(trigger) {
                "   (out of reach on terrain alone: indoors, or in a cave)"
            } else {
                "   (reachable walking on the terrain)"
            }
        );
    }

    // The two probes this trigger contributes to the survey's fourth check.
    let half = reach(trigger);
    println!(
        "  probe   0.9 of reach inside: {}   1.1 outside: {}",
        table.holds(trigger.id, trigger.map, along_local_x(trigger, half * 0.9)),
        !table.holds(trigger.id, trigger.map, along_local_x(trigger, half * 1.1))
    );
    println!(
        "\n  standing in it sends CMSG_AREATRIGGER with {id} — once, on entry; see \
         vale_protocol::play::areatrigger"
    );
    Ok(())
}
