//! `vale waypoints` — the paths the server's creatures walk, against the
//! ground the client draws.
//!
//! The check the waypoint tool can have without a window. A path is nine
//! numbers per node in a table nothing on the wire ever states, and every way
//! it goes wrong renders plausibly: a node under the terrain, a path on a
//! creature whose movement type never walks one, a point sequence the server
//! is about to renumber. Each of those has an answer here, and three of the
//! four come from crossing the database against the archives, which is the
//! join neither side can make alone.
//!
//! Three forms:
//!
//! ```text
//! vale waypoints              every map with paths, and what is wrong across all of them
//! vale waypoints Azeroth      one map: its paths, their lengths, and every node against the ground
//! vale waypoints 12345        one path traced node by node, with the statements an edit becomes
//! ```
//!
//! Read-only. It runs no statement that changes a row.
//!
//! ## What the numbers mean when they are wrong
//!
//! * **A node on a tile the map does not have** is a creature walking over
//!   nothing. That is a fault whatever the creature is.
//! * **A path on a creature whose `movement_type` walks none** is an edit that
//!   applies, reloads and does nothing. Two of the four types walk a path —
//!   see `path::Walk`.
//! * **Points that are not a dense `1..N`** are points `WaypointManager::Cleanup`
//!   rewrites at the next server start, which moves rows out from under any
//!   undo file that names them.
//! * **A `script_id` with no `creature_movement_scripts` row** costs that one
//!   node at load, silently: the path comes up a point short and nothing on
//!   screen says so.
//! * **A cyclic path with fewer than two nodes** is refused by the generator
//!   (`CyclicMovementGenerator::LoadPath`) and the creature stands still.
//!
//! ## The distance from the terrain is a measurement, not a fault
//!
//! The command reports how far each node is from the ADT height under it, and
//! **does not call any distance wrong**. A first draft did, at a yard under and
//! three over, and flagged 26% of map 0 — because two whole populations are
//! legitimately nowhere near the terrain:
//!
//! * **Flying creatures.** Guid 3344 is a Blackrock Drake circling inside
//!   Blackrock Mountain, 500 yards under the terrain height above it, on a
//!   hand-authored 18-node path. Nothing about it is wrong.
//! * **Anything indoors.** A creature on a building's floor stands on a WMO,
//!   and the terrain under the building is not the surface it is on.
//!
//! So the distance is printed as a distribution, and what it is for is the
//! editor's own paths: a node this tool places by pointing at the ground should
//! be *at* the ground, and a shipped path is the baseline that says what normal
//! looks like. Resolving the surface a node is really on means the WMO and M2
//! hulls `vale collision` walks, which is a different and far more expensive
//! question than this command asks.

use crate::common::open_assets;
use vale_config::Config;
use vale_mangos::conn::{self, Db, Where};
use vale_mangos::creature::RowValue;
use vale_mangos::path::{self, Node, Path, Which};

/// **The bands the distance from the terrain is counted in**, in yards.
///
/// A distribution rather than a threshold, because no distance is a fault on
/// its own — see the module comment and the Blackrock Drake. What the bands are
/// for is the shape: shipped paths are overwhelmingly on the ground, so a run
/// of this over a project's own paths that is not is a run worth reading.
const BANDS: [f32; 4] = [1.0, 5.0, 25.0, 100.0];

/// Every form of the command.
pub fn cmd_waypoints(cfg: &Config, which: Option<&str>) -> Result<(), String> {
    let at = Where::find().ok_or_else(Where::absent)?;
    println!("world database: {}", at.line());
    let mut db = Db::open(&at)?;

    match which {
        None => census(&mut db),
        Some(word) => match word.parse::<u64>() {
            Ok(guid) => one(cfg, &mut db, guid),
            Err(_) => map(cfg, &mut db, word),
        },
    }
}

/// What both tables hold, and the two faults that need no archives to find.
fn census(db: &mut Db) -> Result<(), String> {
    for which in [Which::Spawn, Which::Template] {
        let counts = db.rows(&path::counts_query(which))?;
        let paths = counts.first().and_then(|row| row.integer("paths")).unwrap_or(0);
        let nodes = counts.first().and_then(|row| row.integer("nodes")).unwrap_or(0);
        println!("\n{} — {}", which.table(), which.about());
        println!("  {paths} path(s), {nodes} node(s)");
        if paths > 0 {
            println!("  {:.1} nodes a path on average", nodes as f64 / paths as f64);
        }

        // The query vmangos itself runs at every start. A path that answers it
        // is a path the server renumbers, which moves rows out from under an
        // undo that names them.
        let ragged = db.rows(&path::out_of_order_query(which))?;
        match ragged.len() {
            0 => println!("  every path's points are a dense 1..N"),
            n => {
                println!("  PROBLEM  {n} path(s) have points the server will renumber at its next start:");
                for row in ragged.iter().take(10) {
                    println!("    {} {}", which.key_column(), row.integer("owner").unwrap_or(-1));
                }
            }
        }
    }

    // A per-guid path whose creature is gone is a path `Load` skips with a DB
    // error and nothing else.
    let orphans = db.rows(
        "SELECT COUNT(DISTINCT m.`id`) AS `n` FROM `creature_movement` m \
         WHERE NOT EXISTS (SELECT 1 FROM `creature` c WHERE c.`guid` = m.`id`)",
    )?;
    let orphans = orphans.first().and_then(|row| row.integer("n")).unwrap_or(0);
    println!();
    match orphans {
        0 => println!("  every `creature_movement` path belongs to a creature that exists"),
        n => println!("  PROBLEM  {n} path(s) name a creature guid with no `creature` row"),
    }

    // The one that makes a whole path do nothing. **Both** walking types, which
    // is what a first draft got wrong: type 3 is every flying patrol in the
    // game and every one of them was reported as dead.
    let idle = db.rows(&format!(
        "SELECT COUNT(DISTINCT m.`id`) AS `n` FROM `creature_movement` m \
         JOIN `creature` c ON c.`guid` = m.`id` \
         WHERE c.`movement_type` NOT IN ({}, {})",
        path::WAYPOINT_MOTION_TYPE,
        path::CYCLIC_MOTION_TYPE
    ))?;
    let idle = idle.first().and_then(|row| row.integer("n")).unwrap_or(0);
    match idle {
        0 => println!("  every creature with a path has a movement_type that walks it"),
        n => println!("  PROBLEM  {n} creature(s) have a path and a movement_type that never walks one"),
    }

    // …and the one that makes a *cyclic* path do nothing: the generator refuses
    // anything shorter than two nodes.
    let stub = db.rows(&format!(
        "SELECT COUNT(*) AS `n` FROM (SELECT m.`id` FROM `creature_movement` m \
         JOIN `creature` c ON c.`guid` = m.`id` WHERE c.`movement_type` = {} \
         GROUP BY m.`id` HAVING COUNT(*) < 2) AS `short`",
        path::CYCLIC_MOTION_TYPE
    ))?;
    match stub.first().and_then(|row| row.integer("n")).unwrap_or(0) {
        0 => println!("  every cyclic path has the two nodes its generator needs"),
        n => println!("  PROBLEM  {n} cyclic path(s) have fewer than two nodes and are refused"),
    }

    // …and the one that costs a single node, silently.
    let scripts = db.rows(
        "SELECT COUNT(*) AS `n` FROM `creature_movement` m WHERE m.`script_id` <> 0 \
         AND NOT EXISTS (SELECT 1 FROM `creature_movement_scripts` s WHERE s.`id` = m.`script_id`)",
    );
    match scripts {
        Ok(rows) => match rows.first().and_then(|row| row.integer("n")).unwrap_or(0) {
            0 => println!("  every node's script_id has a `creature_movement_scripts` row"),
            n => println!("  PROBLEM  {n} node(s) name a script the server does not have, and are dropped at load"),
        },
        // The table is not in every schema, and its absence is not this
        // command's failure.
        Err(why) => println!("  (could not check script ids: {why})"),
    }

    println!("\n  nothing was written: this command only reads");
    Ok(())
}

/// One map: every path on it, and every node against the ground.
fn map(cfg: &Config, db: &mut Db, name: &str) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let raw = assets
        .read(&vale_assets::tables::dbc::dbc_path("Map"))
        .map_err(|e| format!("Map.dbc: {e}"))?;
    let directories =
        vale_assets::tables::dbc::map_directories(&raw).map_err(|e| format!("Map.dbc: {e}"))?;
    let map_id = *directories
        .iter()
        .find(|(_, directory)| directory.eq_ignore_ascii_case(name))
        .map(|(id, _)| id)
        .ok_or_else(|| format!("no map directory called {name:?} in Map.dbc"))?;

    let started = std::time::Instant::now();
    let rows = db.rows(&path::paths_on_map_query(map_id))?;
    let read = started.elapsed();
    let paths = collect(&rows);
    println!(
        "{name} (map {map_id}): {} path(s), {} node(s), read in {} ms\n",
        paths.len(),
        rows.len(),
        read.as_millis()
    );
    if paths.is_empty() {
        println!("  nothing was written: this command only reads");
        return Ok(());
    }

    // **Which tiles the map actually has**, which is what separates the one real
    // fault here from the measurement beside it. A node over a tile the WDT does
    // not declare is a creature walking over nothing, whatever it is; a node far
    // from the terrain on a tile that exists may be flying or indoors.
    let raw = assets
        .read(&vale_assets::wdt_path(name))
        .map_err(|e| format!("the WDT: {e}"))?;
    let wdt = vale_assets::world::wdt::Wdt::parse(&raw).map_err(|e| format!("the WDT: {e}"))?;

    // The ground, which is the half of this the database cannot answer.
    // `MapTerrain` reads a tile per node's tile and caches it, so the cost is
    // the tiles a map's paths touch rather than the nodes.
    let terrain = vale_assets::world::terrain::MapTerrain::open(&cfg.gamedata)
        .map_err(|e| format!("the terrain: {e}"))?;

    let mut no_tile: Vec<(u64, u32, u32)> = Vec::new();
    let mut no_height = 0usize;
    // One counter per band, plus one for everything past the last.
    let mut bands = [0usize; BANDS.len() + 1];
    let mut lengths: Vec<(u64, f32, usize)> = Vec::new();
    for path in &paths {
        lengths.push((path.owner, path.loop_length(), path.nodes.len()));
        for node in &path.nodes {
            let (tile_x, tile_y) = vale_assets::tile_for_position(node.x, node.y);
            let held = wdt
                .has_tile
                .get(tile_y as usize)
                .and_then(|row| row.get(tile_x as usize))
                .copied()
                .unwrap_or(false);
            if !held {
                no_tile.push((path.owner, tile_x, tile_y));
                continue;
            }
            match terrain.height_at(map_id, node.x, node.y) {
                // A tile the map has, with no height at that point: a hole in
                // the terrain, which is a real place a creature can stand over.
                None => no_height += 1,
                Some(ground) => {
                    let over = (node.z - ground).abs();
                    let band = BANDS.iter().position(|edge| over <= *edge).unwrap_or(BANDS.len());
                    bands[band] += 1;
                }
            }
        }
    }

    match no_tile.len() {
        0 => println!("  every node is over a tile this map has"),
        n => {
            println!("  PROBLEM  {n} node(s) are over a tile the WDT does not declare:");
            let mut named: Vec<(u64, u32, u32)> = no_tile.clone();
            named.dedup();
            for (owner, x, y) in named.iter().take(5) {
                println!("    guid {owner:>8} over tile {x},{y}");
            }
        }
    }
    if no_height > 0 {
        println!("  {no_height} node(s) are over a hole in the terrain of a tile that exists");
    }

    // **A distribution, not a fault count.** See the module comment: a flying
    // creature and a creature indoors are both legitimately far from the
    // terrain, and a first draft of this called 26% of the map wrong.
    println!("\n  how far each node is from the terrain under it:");
    let counted: usize = bands.iter().sum();
    let mut low = 0.0;
    for (index, count) in bands.iter().enumerate() {
        let label = match BANDS.get(index) {
            Some(high) => format!("{low:>5.0} to {high:>5.0} yards  "),
            None => format!("{:>5.0} yards and beyond", BANDS[BANDS.len() - 1]),
        };
        let share = match counted {
            0 => 0.0,
            total => *count as f64 * 100.0 / total as f64,
        };
        println!("    {label}{count:>6}  {share:>5.1}%");
        low = BANDS.get(index).copied().unwrap_or(low);
    }

    // Which creatures actually walk what is drawn, over both walking types.
    let idle = db.rows(&format!(
        "SELECT m.`id` AS `owner` FROM (SELECT DISTINCT `id` FROM `creature_movement`) m \
         JOIN `creature` c ON c.`guid` = m.`id` \
         WHERE c.`map` = {map_id} AND c.`movement_type` NOT IN ({}, {})",
        path::WAYPOINT_MOTION_TYPE,
        path::CYCLIC_MOTION_TYPE
    ))?;
    println!();
    match idle.len() {
        0 => println!("  every path on this map is walked"),
        n => println!("  PROBLEM  {n} path(s) belong to a creature whose movement_type never walks one"),
    }

    lengths.sort_by(|a, b| b.1.total_cmp(&a.1));
    println!("\n  the ten longest rounds:");
    for (owner, length, nodes) in lengths.iter().take(10) {
        println!("    guid {owner:>8}  {length:>9.1} yards  {nodes:>3} node(s)");
    }
    let total: f32 = lengths.iter().map(|(_, length, _)| length).sum();
    println!(
        "\n  {:.1} yards of path in all, {:.1} nodes a path on average",
        total,
        rows.len() as f64 / paths.len() as f64
    );

    println!("\n  nothing was written: this command only reads");
    Ok(())
}

/// One path traced, and the statements an edit to it becomes.
fn one(cfg: &Config, db: &mut Db, guid: u64) -> Result<(), String> {
    let spawn = db.row(&vale_mangos::creature::spawn_query(guid))?;
    let Some(spawn) = spawn else {
        return Err(format!("no `creature` row with guid {guid}"));
    };
    let entry = spawn.integer("id").unwrap_or(0) as u32;
    let map_id = spawn.integer("map").unwrap_or(0) as u32;
    let movement = spawn.integer("movement_type").unwrap_or(0) as u32;

    let patch = wow_patch();
    let template = db.row(&vale_mangos::creature::winning_template_query(entry, patch))?;
    let name = template
        .as_ref()
        .and_then(|row| row.text("name"))
        .unwrap_or("(no template row at this patch)")
        .to_string();
    println!("\nguid {guid} — {name} (entry {entry}) on map {map_id}");

    // Which of the two tables answers for this creature, which is
    // `GetDefaultPath`'s own order: the spawn's own path first, the template's
    // only when there is none.
    let own = read_path(db, Which::Spawn, guid)?;
    let from_template = read_path(db, Which::Template, u64::from(entry))?;
    let path = match own.nodes.is_empty() {
        false => {
            println!("  path from `creature_movement`, keyed by this guid");
            own
        }
        true if !from_template.nodes.is_empty() => {
            println!("  no path of its own; `creature_movement_template` answers for entry {entry}");
            from_template
        }
        true => {
            println!("  no path in either table");
            return Ok(());
        }
    };

    let walk = path::Walk::of(movement);
    match walk.walks() {
        true => println!("  movement_type {movement} — {}", walk.about()),
        false => println!(
            "  PROBLEM  movement_type {movement} — {}. \
             An edit here applies, reloads and does nothing.",
            walk.about()
        ),
    }
    if path.nodes.len() < walk.needs_nodes() {
        println!(
            "  PROBLEM  {} node(s), and this generator refuses anything shorter than {}",
            path.nodes.len(),
            walk.needs_nodes()
        );
    }

    let terrain = vale_assets::world::terrain::MapTerrain::open(&cfg.gamedata)
        .map_err(|e| format!("the terrain: {e}"))?;

    println!("\n  point         x           y           z    ground     over  wait   facing");
    for (index, node) in path.nodes.iter().enumerate() {
        let ground = terrain.height_at(map_id, node.x, node.y);
        let (ground_text, over) = match ground {
            Some(ground) => (format!("{ground:>9.2}"), format!("{:>+8.2}", node.z - ground)),
            None => ("   off map".to_string(), "        ".to_string()),
        };
        // Three states, not two: applied, stored but inert, and absent. The
        // middle one is a facing on a node with no wait, which the server never
        // applies — see `Node::faces`.
        let facing = match (node.faces(), node.has_orientation()) {
            (true, _) => format!("{:>7.2}", node.orientation),
            (false, true) => format!("{:>6.2}*", node.orientation),
            (false, false) => "      —".to_string(),
        };
        println!(
            "  {:>5}  {:>10.2}  {:>10.2}  {:>10.2}  {ground_text}  {over}  {:>4}  {facing}",
            index + 1,
            node.x,
            node.y,
            node.z,
            node.waittime,
        );
    }

    println!(
        "\n  {} node(s), {:.1} yards round the loop, {} ms waited in all",
        path.nodes.len(),
        path.loop_length(),
        path.total_wait()
    );
    let inert = path.nodes.iter().filter(|node| !node.faces() && node.has_orientation()).count();
    if inert > 0 {
        println!(
            "  * {inert} node(s) hold a facing the server never applies: it needs a waittime as well"
        );
    }
    let scripts = path.script_ids();
    if !scripts.is_empty() {
        println!("  script ids: {scripts:?} — each needs a `creature_movement_scripts` row or its node is dropped");
    }
    let specials = path.path_ids();
    if !specials.is_empty() {
        println!("  special paths: {specials:?} — from `creature_movement_special`");
    }

    // What an edit becomes, which is the half of this the editor writes and the
    // only place it can be read without opening a window.
    println!("\n  the statements a save of this path emits:");
    for line in path::statements(&path) {
        println!("    {line}");
    }

    println!("\n  nothing was written: this command only reads");
    Ok(())
}

/// Every node row for one owner, in point order.
fn read_path(db: &mut Db, which: Which, owner: u64) -> Result<Path, String> {
    let rows = db.rows(&path::path_query(which, owner))?;
    let mut nodes: Vec<(u32, Node)> = rows.iter().filter_map(path::node_from_row).collect();
    nodes.sort_by_key(|(point, _)| *point);
    Ok(Path { which, owner, nodes: nodes.into_iter().map(|(_, node)| node).collect() })
}

/// The rows of a whole map, grouped into paths by owner.
///
/// Ordered by the `point` the table states rather than by the order the rows
/// arrived, so a path with a gap in it still reads in the order the server will
/// walk it.
fn collect(rows: &[vale_mangos::creature::Row]) -> Vec<Path> {
    let mut by_owner: std::collections::BTreeMap<u64, Vec<(u32, Node)>> = std::collections::BTreeMap::new();
    for row in rows {
        let Some(owner) = row.integer("id") else { continue };
        let Some(node) = path::node_from_row(row) else { continue };
        by_owner.entry(owner.max(0) as u64).or_default().push(node);
    }
    by_owner
        .into_iter()
        .map(|(owner, mut nodes)| {
            nodes.sort_by_key(|(point, _)| *point);
            Path { which: Which::Spawn, owner, nodes: nodes.into_iter().map(|(_, node)| node).collect() }
        })
        .collect()
}

/// The content patch the template row is read at — see `vale spawns`, whose
/// copy of this reads the same conf.
fn wow_patch() -> u32 {
    let Ok(at) = std::env::var("VALE_MANGOSD") else {
        return conn::DEFAULT_WOW_PATCH;
    };
    let at = std::path::PathBuf::from(at);
    let conf = match at.is_dir() {
        true => at.join("mangosd.conf"),
        false => at,
    };
    conn::wow_patch_from_conf(conf).unwrap_or(conn::DEFAULT_WOW_PATCH)
}

