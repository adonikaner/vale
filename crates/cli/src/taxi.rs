//! `vale taxi`: checks the flight map with no session.
//!
//! ```text
//! vale taxi         the three tables against each other, and the six checks
//! vale taxi 2       trace one node: its map, where it lands, and every
//!                      flight a character standing there could take
//! ```
//!
//! As with [`crate::areatrigger`], nothing about the flight map is sent by the
//! server except a node id and a 256-bit mask. Which nodes are drawn, where on
//! the parchment, which route the lines take and what the tooltip quotes are
//! all computed by [`vale_assets::tables::taxi`] from three shipped tables.
//! A column read at the wrong index therefore raises no error and does not
//! look wrong: it produces a map with buttons in the wrong places and routes
//! that differ from the ones the server flies. Nothing shows the fault until a
//! character pays for a flight and lands in the wrong zone.
//!
//! ## The checks
//!
//! Five are shape checks and one is a join between the tables' geometry.
//!
//! * Every node names a map that `Map.dbc` ships. This is a cheap check that
//!   field 1 is the map: the neighbouring columns are a float and an id, and
//!   either would read as map ids in the millions.
//! * Every path's `from` and `to` name a node the table has. Reading
//!   `TaxiPath`'s columns shifted by one gives node ids that are really costs,
//!   and they resolve to nothing.
//! * Every path's waypoints begin at its `from` node and end at its `to` node,
//!   within a few yards. This is the strongest check and the only one that
//!   compares the two tables' geometry. It pins `TaxiNodes` fields 2..4,
//!   `TaxiPathNode` fields 4..6 and `TaxiPath`'s `from`/`to` at once, because
//!   reading any one of them wrong puts the first waypoint of some path half a
//!   continent from the node it leaves. See
//!   [`vale_assets::tables::taxi::TaxiPath::ends`], which exists for this
//!   check.
//! * Every node projects into the unit square through its own continent's
//!   `WorldMapContinent` box. A wrong box column puts every button off the
//!   parchment; a crossed axis puts them all on the diagonal.
//! * The map art exists for every continent a node stands on:
//!   `Interface\TaxiFrame\TAXIMAP<map>.blp`, a path the client composes rather
//!   than one any file states.
//! * Every node is served by at least one side, and the two sides partition the
//!   graph as the four generic mount ids say they do.
//!
//! ## Building the map at every node
//!
//! The survey then opens the map at every node in the game, for both sides,
//! with every node discovered (85 nodes x 2), and reports what the search
//! found. Two numbers matter: no node may be its own neighbour, and the
//! deepest route in the game is a specific number that a broken relaxation
//! turns into 1 or 84.
//!
//! ## How the server loads the tables
//!
//! The last section reads the three tables as vmangos loads them. It reports
//! the structural findings of `vale_edit::dbc::taxi::check`, the waypoints
//! that name a path id `TaxiPath.dbc` does not have (below its largest id), the
//! nodes whose mount columns the server and the
//! client assign to different sides (the server reads field 14 as Horde and
//! field 15 as Alliance; the client reads the values), and which flight paths
//! have a path in the opposite direction.

use std::collections::HashMap;

use vale_assets::tables::taxi::{Board, NodeKind, TaxiMask, TaxiTables, Team};
use vale_config::Config;

use crate::common::open_assets;

/// How far a path's first waypoint may stand from the node it leaves before it
/// is reported.
///
/// It is not zero because a flight master stands at the node and the spline
/// starts on the pad beside them, so the two are several yards apart in the
/// shipped data. Twenty yards accepts every shipped row and is two orders of
/// magnitude tighter than any field-index error, which moves a point across the
/// continent.
const ENDPOINT_TOLERANCE: f32 = 20.0;

pub fn cmd_taxi(cfg: &Config, node: Option<u32>) -> Result<(), String> {
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

    match node {
        Some(id) => trace(&tables, &maps, id),
        None => {
            survey(&tables, &maps, &mut assets)?;
            server(&nodes, &paths, &path_nodes)
        }
    }
}

/// Reports the three tables as vmangos loads them and as the editor edits
/// them: the structural checks (`vale_edit::dbc::taxi::check`), the points
/// that name no path, the two readings of the mount columns, and which flights
/// have a path back.
fn server(nodes: &[u8], paths: &[u8], path_nodes: &[u8]) -> Result<(), String> {
    use vale_edit::dbc::{taxi, DbcFile};
    let open = |bytes: &[u8], name: &str| DbcFile::parse(bytes).map_err(|e| format!("{name}: {e}"));
    let (nodes, paths, points) = (
        open(nodes, "TaxiNodes")?,
        open(paths, "TaxiPath")?,
        open(path_nodes, "TaxiPathNode")?,
    );
    println!("\n  as the server loads them, and as the editor keeps them:");
    let findings = taxi::check(&nodes, &paths, &points);
    println!(
        "    structure: {} finding(s): gaps in a path's point indices, points past the \
         largest path id, paths naming no node, flights with under two points, duplicate \
         paths  [should be 0]",
        findings.len()
    );
    for finding in findings.iter().take(8) {
        println!("               {finding}");
    }
    println!(
        "    unused:    {} point(s) name a path TaxiPath.dbc does not have, below its \
         largest id, which nothing flies",
        taxi::unused_points(&paths, &points)
    );

    let every = taxi::nodes(&nodes);
    let differ: Vec<String> = every
        .iter()
        .filter(|node| {
            let as_read = vale_assets::tables::taxi::TaxiNode {
                id: node.id,
                map: node.map,
                pos: node.at,
                name: node.name.clone(),
                mounts: node.mounts,
            };
            let client = |team| vale_assets::tables::taxi::serves(&as_read, Some(team));
            (node.mounts[0] != 0) != client(Team::Horde)
                || (node.mounts[1] != 0) != client(Team::Alliance)
        })
        .map(|node| format!("{} {:?} {:?}", node.id, node.name, node.mounts))
        .collect();
    println!(
        "    sides:     {} of {} nodes are offered to different sides by the server \
         (field 14 Horde, 15 Alliance, when non-zero) and the client (by value)",
        differ.len(),
        every.len()
    );
    for line in differ.iter().take(16) {
        println!("               {line}");
    }

    let transport: std::collections::HashSet<u32> = every
        .iter()
        .filter(|node| node.mounts == [0, 0])
        .map(|node| node.id)
        .collect();
    let all = taxi::paths(&paths);
    let flights: Vec<&taxi::Path> = all
        .iter()
        .filter(|path| !transport.contains(&path.from) && !transport.contains(&path.to))
        .collect();
    let one_way: Vec<String> = flights
        .iter()
        .filter(|path| !all.iter().any(|back| back.from == path.to && back.to == path.from))
        .map(|path| format!("path {} ({} -> {})", path.id, path.from, path.to))
        .collect();
    println!(
        "    returns:   {} of {} flight paths have a path the other way",
        flights.len() - one_way.len(),
        flights.len()
    );
    for line in one_way.iter().take(8) {
        println!("               {line} has none");
    }
    Ok(())
}

/// Whether a node has no flight master: it names no mount for either side.
/// These nodes are the endpoints of the boat and zeppelin routes, which share
/// `TaxiPath.dbc` with the flights but not their geometry conventions.
fn is_transport(node: &vale_assets::tables::taxi::TaxiNode) -> bool {
    node.mounts == [0, 0]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// A mask with every node discovered: the mask a GM with `IsTaxiCheater` gets.
/// With it the survey reports on the tables rather than on one character.
fn everything() -> TaxiMask {
    TaxiMask([u32::MAX; 8])
}

fn survey(
    tables: &TaxiTables,
    maps: &HashMap<u32, String>,
    assets: &mut vale_assets::Assets,
) -> Result<(), String> {
    let nodes: Vec<_> = {
        let mut nodes: Vec<_> = tables.nodes().collect();
        nodes.sort_by_key(|node| node.id);
        nodes
    };
    let paths: Vec<_> = {
        let mut paths: Vec<_> = tables.paths().collect();
        paths.sort_by_key(|path| path.id);
        paths
    };
    let mut node_maps: Vec<u32> = nodes.iter().map(|node| node.map).collect();
    node_maps.sort_unstable();
    node_maps.dedup();
    println!(
        "TaxiNodes.dbc: {} nodes over {} map(s) — TaxiPath.dbc: {} paths",
        nodes.len(),
        node_maps.len(),
        paths.len()
    );

    // ---- the maps ---------------------------------------------------------
    let unknown: Vec<u32> = node_maps
        .iter()
        .copied()
        .filter(|map| !maps.contains_key(map))
        .collect();
    println!(
        "\n  maps:      {} of {} named maps are in Map.dbc{}",
        node_maps.len() - unknown.len(),
        node_maps.len(),
        match unknown.is_empty() {
            true => String::new(),
            false => format!(" — {unknown:?} are not, so their nodes are unreachable"),
        }
    );

    // ---- the graph --------------------------------------------------------
    let dangling: Vec<u32> = paths
        .iter()
        .filter(|path| tables.node(path.from).is_none() || tables.node(path.to).is_none())
        .map(|path| path.id)
        .collect();
    println!(
        "  edges:     {} of {} paths name two nodes that exist  [dangling: {}]",
        paths.len() - dangling.len(),
        paths.len(),
        dangling.len()
    );

    // ---- the join between path waypoints and node positions ----------------
    let mut off: Vec<(f32, u32, String)> = Vec::new();
    let mut transport_off = 0usize;
    let mut no_waypoints = 0usize;
    let mut cross_map = 0usize;
    for path in &paths {
        let (Some(from), Some(to)) = (tables.node(path.from), tables.node(path.to)) else {
            continue;
        };
        if path.length >= vale_assets::tables::taxi::MISSING_PATH_LENGTH {
            no_waypoints += 1;
            continue;
        }
        // A path across two maps cannot be measured: its waypoints are in one
        // map's coordinates and one of its nodes is in the other's. Two rows
        // are like this, both in Alterac Valley: 313 and 383 run between
        // Frostwolf Keep inside the battleground and Undercity outside it, so
        // the Undercity end computes as 3,058 yards away, which is
        // meaningless.
        if from.map != to.map {
            cross_map += 1;
            continue;
        }
        for (waypoint, node) in [(path.ends.0, from), (path.ends.1, to)] {
            let apart = distance(waypoint, node.pos);
            if apart <= ENDPOINT_TOLERANCE {
                continue;
            }
            // The boats and zeppelins are in this table too, and every long
            // end found here belongs to one. A `TaxiPath` whose endpoint names
            // no mount is a transport route, run by a script (`SendTaxiPath`)
            // rather than by a flight master, and its nodes are placeholders:
            // "Generic, World target for Zeppelin Paths" stands 17 km from the
            // spline that names it. These are counted separately rather than
            // hidden. A real failure is a flight path in this list, and there
            // are none.
            match is_transport(from) || is_transport(to) {
                true => transport_off += 1,
                false => off.push((apart, path.id, node.name.clone())),
            }
        }
    }
    off.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!(
        "  waypoints: {} of {} paths have them; {} flight end(s) further than \
         {ENDPOINT_TOLERANCE:.0} yards from the node they name  [should be 0]",
        paths.len() - no_waypoints,
        paths.len(),
        off.len()
    );
    println!(
        "               …plus {transport_off} on transport paths, whose nodes are placeholders \
         — the boats and the zeppelins"
    );
    println!(
        "               …and {cross_map} path(s) span two maps, where the comparison means nothing"
    );
    for (apart, path, name) in off.iter().take(5) {
        println!("               path {path} is {apart:.0} yards from {name:?}");
    }

    // ---- the projection ----------------------------------------------------
    let mut outside = Vec::new();
    let mut projected = 0usize;
    for node in &nodes {
        let Some(bounds) = tables.taxi_box(node.map) else {
            continue;
        };
        let [x, y] = bounds.project(node.pos);
        projected += 1;
        if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
            outside.push((node.id, node.name.as_str(), x, y));
        }
    }
    println!(
        "  parchment: {} of {} nodes on a mapped continent land inside the taxi box",
        projected - outside.len(),
        projected
    );
    for (id, name, x, y) in outside.iter().take(8) {
        println!("               node {id} {name:?} at ({x:.3}, {y:.3}) — off the parchment");
    }

    // ---- the art -----------------------------------------------------------
    for map in &node_maps {
        if tables.taxi_box(*map).is_none() {
            continue;
        }
        let path = vale_assets::tables::taxi::map_art(*map);
        let bytes = assets.read(&path).map(|raw| raw.len());
        println!(
            "  art:       {path} — {}",
            match bytes {
                Ok(len) => format!("{len} bytes"),
                Err(_) => "MISSING".to_string(),
            }
        );
    }

    // ---- the two sides ------------------------------------------------------
    let served = |team: Team| {
        nodes
            .iter()
            .filter(|node| vale_assets::tables::taxi::serves(node, Some(team)))
            .count()
    };
    let neither = nodes
        .iter()
        .filter(|node| {
            !vale_assets::tables::taxi::serves(node, Some(Team::Alliance))
                && !vale_assets::tables::taxi::serves(node, Some(Team::Horde))
        })
        .count();
    println!(
        "\n  sides:     Alliance may use {}, Horde {} of {} — {neither} are refused by both \
         [should be 0]",
        served(Team::Alliance),
        served(Team::Horde),
        nodes.len()
    );

    // ---- the map, built at every node ---------------------------------------
    println!("\n  the map, opened at every node in the game with everything discovered:");
    let mask = everything();
    for team in [Team::Alliance, Team::Horde] {
        let mut drawn = 0usize;
        let mut reachable = 0usize;
        let mut deepest = (0usize, 0u32, 0u32);
        let mut self_neighbour = 0usize;
        let mut opened = 0usize;
        for node in &nodes {
            let board = Board::build(tables, node.id, &mask, Some(team));
            if board.rows.is_empty() {
                continue;
            }
            opened += 1;
            drawn += board
                .rows
                .iter()
                .filter(|row| row.kind != NodeKind::None)
                .count();
            for (index, row) in board.rows.iter().enumerate() {
                if row.kind == NodeKind::Reachable {
                    reachable += 1;
                }
                let hops = board.hops(index + 1);
                if hops > deepest.0 {
                    deepest = (hops, node.id, row.node);
                }
                // A hop in the current node's own route means the seed was
                // relaxed against itself.
                if row.node == node.id && hops > 0 {
                    self_neighbour += 1;
                }
            }
        }
        println!(
            "    {team:?}: {opened} nodes open a map; {drawn} buttons drawn, {reachable} \
             flyable; deepest route {} hops ({} -> {}); {self_neighbour} self-routes [should be 0]",
            deepest.0,
            name_of(tables, deepest.1),
            name_of(tables, deepest.2),
        );
    }
    Ok(())
}

fn name_of(tables: &TaxiTables, node: u32) -> String {
    tables
        .node(node)
        .map_or_else(|| format!("node {node}"), |node| node.name.clone())
}

/// One node, and every flight a character standing at it could take.
fn trace(tables: &TaxiTables, maps: &HashMap<u32, String>, id: u32) -> Result<(), String> {
    let node = tables
        .node(id)
        .ok_or_else(|| format!("no taxi node {id}; ids run 1..87 with gaps"))?;
    println!(
        "node {id} {:?} on map {} ({})",
        node.name,
        node.map,
        maps.get(&node.map).map_or("?", String::as_str)
    );
    println!(
        "  world position [{:.1}, {:.1}, {:.1}], mounts {:?}",
        node.pos[0], node.pos[1], node.pos[2], node.mounts
    );
    match tables.taxi_box(node.map) {
        Some(bounds) => {
            let [x, y] = bounds.project(node.pos);
            println!("  on the parchment: ({x:.3}, {y:.3}) from the bottom left");
        }
        None => println!("  on the parchment: no WorldMapContinent row — this map has no map art"),
    }
    println!(
        "  served by: {}",
        match (
            vale_assets::tables::taxi::serves(node, Some(Team::Alliance)),
            vale_assets::tables::taxi::serves(node, Some(Team::Horde)),
        ) {
            (true, true) => "both sides",
            (true, false) => "the Alliance",
            (false, true) => "the Horde",
            (false, false) => "neither side",
        }
    );

    let mask = everything();
    for team in [Team::Alliance, Team::Horde] {
        let board = Board::build(tables, id, &mask, Some(team));
        println!(
            "\n  {team:?}, with every node discovered — {} drawn:",
            board
                .rows
                .iter()
                .filter(|row| row.kind != NodeKind::None)
                .count()
        );
        for (index, row) in board.rows.iter().enumerate() {
            if row.kind == NodeKind::None {
                continue;
            }
            let route: Vec<String> = row.route.iter().map(|id| id.to_string()).collect();
            println!(
                "    {:>2} {:<9} {:<38} {:>6} copper  {:>7.0} yards  via {}",
                index + 1,
                row.kind.word(),
                row.name,
                board.cost(index + 1),
                row.distance,
                match route.is_empty() {
                    true => "-".to_string(),
                    false => route.join(" -> "),
                }
            );
        }
    }
    Ok(())
}
