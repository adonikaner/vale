//! The flight path tool's panel: the selected node, the paths leaving and
//! arriving at it, the selected path and its points, and the selected point.
//!
//! Every number here writes through `crate::tools::tables::set_fields` under a
//! gesture key, so a value dragged through forty steps is one undo entry. Every
//! operation that adds or removes a row goes through `vale_edit::dbc::taxi`
//! and is one undo entry. See `crate::tools::flightpaths` for the pointer.

use bevy_egui::egui;

use super::theme;
use crate::session::EditSession;
use crate::tools::flightpaths::{self, Armed, Flightpaths, Route};
use crate::tools::tables;
use vale_assets::tables::taxi::{node_fields as nf, path_fields as pf, path_node_fields as wf};
use vale_edit::dbc::taxi;

/// What the panel is drawn from.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub flights: &'a mut Flightpaths,
    pub assets: &'a vale_client::assets::GameAssets,
    pub now: f64,
    /// The top bar's Server… popover, which holds Apply and Put back.
    pub server_panel: &'a mut super::popover::Popover,
}

pub fn draw(ui: &mut egui::Ui, subject: Subject<'_>) {
    let Subject {
        session,
        flights,
        assets,
        now,
        server_panel,
    } = subject;
    for table in taxi::TABLES {
        if !session.open_table(assets, table) {
            theme::note(ui, format!("opening {table}.dbc\u{2026}"));
            return;
        }
    }

    summary(ui, flights);
    ui.add_space(4.0);
    controls(ui, flights);

    egui::ScrollArea::vertical()
        .id_salt("flightpaths")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Each selected thing is a section that folds, so a node with
            // its paths, a path and a point together do not run the panel
            // off the screen.
            if let Some(id) = flights.node {
                section(ui, &format!("Node {id}"), "node", |ui| {
                    node_block(ui, session, flights, id, now);
                });
            }
            if let Some(id) = flights.path {
                section(ui, &format!("Path {id}"), "path", |ui| {
                    path_block(ui, session, flights, id, now);
                });
            }
            if let Some(id) = flights.point {
                let title = flights
                    .point(id)
                    .map(|(route, point)| format!("Point {} of {}", point.index, route.points.len()))
                    .unwrap_or_else(|| format!("Point {id}"));
                section(ui, &title, "point", |ui| {
                    point_block(ui, session, flights, id, now);
                });
            }
            if flights.node.is_none() && flights.path.is_none() {
                theme::note(
                    ui,
                    "Click a node or a path in the world. Nodes are coloured by the \
                     sides the server offers them to: gold both, red Horde, blue \
                     Alliance, grey neither (a boat or zeppelin end).",
                );
            }
            ui.add_space(6.0);
            theme::heading(ui, "Server");
            if ui
                .button("Server\u{2026}")
                .on_hover_text(
                    "Opens the Server panel. Apply in its Client tables block writes the \
                     nodes as taxi_nodes rows and copies TaxiPath.dbc and TaxiPathNode.dbc \
                     into DataDir\\5875\\dbc; Put back restores the server's own.",
                )
                .clicked()
            {
                server_panel.show();
            }
            theme::note(
                ui,
                "A node is a row of taxi_nodes. Paths and their points are TaxiPath.dbc and \
                 TaxiPathNode.dbc, which the server reads from DataDir\\5875\\dbc. Apply on the \
                 Server panel writes both, and so does a save with Apply on save on. vmangos \
                 reads all three at startup only: restart the server after applying.",
            );
            ui.add_space(12.0);
        });
}

/// One folding section of the panel, open by default. `salt` keeps a section's
/// open state across selections, so folding the node section keeps it folded
/// when another node is clicked.
fn section(ui: &mut egui::Ui, title: &str, salt: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(4.0);
    egui::CollapsingHeader::new(egui::RichText::new(title).strong().color(theme::INK))
        .id_salt(("flightpaths", salt))
        .default_open(true)
        .show(ui, body);
}

/// The counts, and what the three tables' check finds.
fn summary(ui: &mut egui::Ui, flights: &Flightpaths) {
    let paths = flights.routes.iter().filter(|route| !route.transport).count();
    theme::note(
        ui,
        format!(
            "{} nodes and {paths} flight paths on this map, and {} boat or zeppelin routes",
            flights.nodes.len(),
            flights.routes.len() - paths
        ),
    );
    if flights.findings.is_empty() {
        return;
    }
    ui.label(
        egui::RichText::new(format!(
            "{} problem(s) the server's loader would meet:",
            flights.findings.len()
        ))
        .color(theme::WARN),
    );
    for finding in flights.findings.iter().take(6) {
        ui.label(
            egui::RichText::new(finding.to_string())
                .size(theme::SMALL)
                .color(theme::WARN),
        );
    }
}

/// What a click on empty ground does, and the settings for what it makes.
fn controls(ui: &mut egui::Ui, flights: &mut Flightpaths) {
    ui.horizontal_wrapped(|ui| {
        let new_node = flights.armed == Armed::NewNode;
        if ui
            .selectable_label(new_node, "New node")
            .on_hover_text(
                "Armed, a click on the ground makes a node there, offered to both \
                 sides: MountCreatureId 1 is the wind rider 2224 and 2 the gryphon \
                 541. A flight master has to stand near it for the server to use it.",
            )
            .clicked()
        {
            flights.armed = match new_node {
                true => Armed::Nothing,
                false => Armed::NewNode,
            };
        }
        let connecting = matches!(flights.armed, Armed::Connect { .. });
        let connect = ui.add_enabled(
            flights.node.is_some() || connecting,
            egui::Button::selectable(connecting, "Connect"),
        );
        if connect
            .on_hover_text(
                "Armed, a click on another node makes a path from the selected node \
                 to it, with points every 120 yards at the clearance above the ground.",
            )
            .on_disabled_hover_text("Select the node the path starts from first.")
            .clicked()
        {
            flights.armed = match (connecting, flights.node) {
                (false, Some(from)) => Armed::Connect { from },
                _ => Armed::Nothing,
            };
        }
        let adding = flights.armed == Armed::AddPoints;
        let add = ui.add_enabled(
            flights.path.is_some() || adding,
            egui::Button::selectable(adding, "Add points"),
        );
        if add
            .on_hover_text(
                "Armed, a click on the ground puts a point into the selected path, in \
                 the leg nearest the click, at the clearance above the ground or the \
                 leg's own height, whichever is higher.",
            )
            .on_disabled_hover_text("Select a path first.")
            .clicked()
        {
            flights.armed = match adding {
                true => Armed::Nothing,
                false => Armed::AddPoints,
            };
        }
    });
    theme::row(ui, "clearance", |ui| {
        ui.add(
            egui::DragValue::new(&mut flights.clearance)
                .range(0.0..=500.0)
                .speed(1.0)
                .suffix(" yd"),
        )
        .on_hover_text("How far above the ground a new point is put.");
    });
    ui.checkbox(&mut flights.with_return, "Make the path back")
        .on_hover_text(
            "Connect makes the reverse path as well: the same points in the other \
             order. 270 of the 275 shipped flights have one, as a row of its own.",
        );
    ui.checkbox(&mut flights.carry_ends, "Move path ends with a node")
        .on_hover_text(
            "Dragging a node moves the first point of every path leaving it and the \
             last point of every path arriving at it to the node.",
        );
    ui.checkbox(&mut flights.transports, "Show boat and zeppelin routes")
        .on_hover_text(
            "Paths whose ends name no mount on either side are the transports' \
             routes. They share TaxiPath.dbc with the flights.",
        );
    if flights.armed != Armed::Nothing {
        theme::note(ui, "Escape disarms.");
    }
}

/// A copper amount as the game writes it.
pub fn money(copper: u32) -> String {
    let (gold, silver, copper) = (copper / 10_000, copper / 100 % 100, copper % 100);
    match (gold, silver) {
        (0, 0) => format!("{copper}c"),
        (0, _) => format!("{silver}s {copper}c"),
        _ => format!("{gold}g {silver}s {copper}c"),
    }
}

fn node_block(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    id: u32,
    now: f64,
) {
    let Some(node) = flights.node(id).cloned() else {
        theme::note(ui, format!("node {id} is not on this map"));
        return;
    };
    let mut name = node.name.clone();
    theme::row(ui, "name", |ui| {
        if ui.text_edit_singleline(&mut name).changed() {
            tables::set_text(session, taxi::NODES, node.record, nf::NAME, &name, "Rename flight node", now);
            flights.stale();
        }
    });
    let mut at = node.at;
    let mut moved = false;
    for (axis, label) in ["x", "y", "z"].iter().enumerate() {
        theme::row(ui, label, |ui| {
            moved |= ui
                .add(egui::DragValue::new(&mut at[axis]).speed(0.5).suffix(" yd"))
                .changed();
        });
    }
    if moved {
        flightpaths::move_node(session, flights, id, bevy::math::Vec3::from(at), now);
        flights.stale();
    }
    let mut mounts = node.mounts;
    let mut remounted = false;
    for (n, (label, tip)) in [
        (
            "Horde mount",
            "MountCreatureId 1, a creature_template entry. vmangos offers the node to \
             the Horde only when this is not 0, and flies this creature.",
        ),
        (
            "Alliance mount",
            "MountCreatureId 2. vmangos offers the node to the Alliance only when this \
             is not 0.",
        ),
    ]
    .iter()
    .enumerate()
    {
        theme::row(ui, label, |ui| {
            remounted |= ui
                .add(egui::DragValue::new(&mut mounts[n]).speed(1.0))
                .on_hover_text(*tip)
                .changed();
        });
    }
    if remounted {
        tables::set_fields(
            session,
            taxi::NODES,
            node.record,
            &[(nf::MOUNT, mounts[0]), (nf::MOUNT + 1, mounts[1])],
            "Change flight node mounts",
            &format!("TaxiNodes {id} mounts"),
            now,
        );
        flights.stale();
    }
    theme::note(ui, sides(&node));
    ui.horizontal(|ui| {
        if ui.button("Fly to").clicked() {
            flights.fly_to(node.at);
        }
        if ui
            .button("Drop to the ground")
            .on_hover_text("Set z to the ground under the node, where the ground is open.")
            .clicked()
        {
            if let Some(ground) = crate::tools::doodads::ground_height(session, at[0], at[1]) {
                let to = bevy::math::Vec3::new(at[0], at[1], ground);
                flightpaths::move_node(session, flights, id, to, now);
                flights.stale();
            }
        }
        if ui
            .button("Remove node")
            .on_hover_text("Removes the node, every path leaving or arriving at it, and their points.")
            .clicked()
        {
            match taxi::remove_node(&mut session.tables, id) {
                Ok(done) => {
                    flightpaths::record(session, "Remove flight node", done);
                    session.status = format!("node {id} removed");
                    flights.node = None;
                    flights.path = None;
                    flights.point = None;
                    flights.stale();
                }
                Err(e) => session.status = e.to_string(),
            }
        }
    });

    let (leaving, arriving) = flights.routes_of(id);
    let leaving: Vec<Route> = leaving.into_iter().cloned().collect();
    let arriving: Vec<Route> = arriving.into_iter().cloned().collect();
    ui.add_space(4.0);
    theme::heading(ui, &format!("Paths from here ({})", leaving.len()));
    for route in &leaving {
        let back = arriving.iter().any(|other| other.path.from == route.path.to);
        route_row(ui, flights, route, route.path.to, "\u{2192}", back);
    }
    theme::heading(ui, &format!("Paths to here ({})", arriving.len()));
    for route in &arriving {
        let back = leaving.iter().any(|other| other.path.to == route.path.from);
        route_row(ui, flights, route, route.path.from, "\u{2190}", back);
    }
}

/// Which sides the node is offered to, as the server and the client each read
/// its two mount columns.
fn sides(node: &taxi::Node) -> String {
    use vale_assets::tables::taxi::{serves, TaxiNode, Team};
    let words = |horde: bool, alliance: bool| match (horde, alliance) {
        (true, true) => "both sides",
        (true, false) => "the Horde",
        (false, true) => "the Alliance",
        (false, false) => "neither side",
    };
    let server = words(node.mounts[0] != 0, node.mounts[1] != 0);
    let as_read = TaxiNode {
        id: node.id,
        map: node.map,
        pos: node.at,
        name: node.name.clone(),
        mounts: node.mounts,
    };
    let client = words(
        serves(&as_read, Some(Team::Horde)),
        serves(&as_read, Some(Team::Alliance)),
    );
    match server == client {
        true => format!("Offered to {server}."),
        false => format!(
            "The server offers it to {server}; the client's flight map draws it for {client}."
        ),
    }
}

fn route_row(
    ui: &mut egui::Ui,
    flights: &mut Flightpaths,
    route: &Route,
    other: u32,
    arrow: &str,
    back: bool,
) {
    let chosen = flights.path == Some(route.path.id);
    let text = format!(
        "{arrow} {}  \u{b7}  {}  \u{b7}  {:.0} yd",
        flights.name(other),
        money(route.path.cost),
        route.length
    );
    let response = ui.selectable_label(chosen, text).on_hover_text(format!(
        "path {}, {} points{}",
        route.path.id,
        route.points.len(),
        match back {
            true => "",
            false => ". There is no path the other way.",
        }
    ));
    if response.clicked() {
        flights.select_path(route.path.id);
    }
    if !back {
        ui.label(
            egui::RichText::new("   no path back")
                .size(theme::SMALL)
                .color(theme::WARN),
        );
    }
}

fn path_block(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    id: u32,
    now: f64,
) {
    let Some(route) = flights.route(id).cloned() else {
        return;
    };
    ui.horizontal(|ui| {
        for (end, word) in [(route.path.from, "from"), (route.path.to, "to")] {
            ui.label(egui::RichText::new(word).color(theme::INK_DIM));
            if ui.link(flights.name(end)).clicked() {
                flights.select_node(end);
                if let Some(node) = flights.node(end) {
                    let at = node.at;
                    flights.fly_to(at);
                }
            }
        }
    });
    let mut cost = route.path.cost;
    theme::row(ui, "cost", |ui| {
        if ui
            .add(egui::DragValue::new(&mut cost).speed(10.0).suffix(" c"))
            .on_hover_text("Copper, before the reputation discount the server applies.")
            .changed()
        {
            tables::set_fields(
                session,
                taxi::PATHS,
                route.path.record,
                &[(pf::COST, cost)],
                "Change flight cost",
                &format!("TaxiPath {id} cost"),
                now,
            );
            flights.stale();
        }
        ui.label(theme::number(money(cost)));
    });
    theme::row(ui, "length", |ui| {
        ui.label(theme::number(format!(
            "{:.0} yd, {} points",
            route.length,
            route.points.len()
        )));
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Fly to")
            .on_hover_text(
                "Frames the whole path. From that far back the world's fog can hide                  the ground; FOG on the view bar turns it off.",
            )
            .clicked()
        {
            flights.fly_to_path(id);
        }
        let has_back = flights
            .routes
            .iter()
            .any(|other| other.path.from == route.path.to && other.path.to == route.path.from);
        if ui
            .add_enabled(!has_back, egui::Button::new("Make the path back"))
            .on_hover_text("A new path the other way through the same points, at the same cost.")
            .on_disabled_hover_text("There is already a path the other way.")
            .clicked()
        {
            match taxi::reverse_path(&mut session.tables, id) {
                Ok(done) => {
                    let made = done.made;
                    flightpaths::record(session, "Make the path back", done);
                    session.status = format!("path {} made", made.unwrap_or(0));
                    flights.path = made;
                    flights.point = None;
                    flights.stale();
                }
                Err(e) => session.status = e.to_string(),
            }
        }
        if ui
            .button("Snap the ends to the nodes")
            .on_hover_text("Move the first point to the node it leaves and the last to the node it reaches.")
            .clicked()
        {
            snap_ends(session, flights, &route, now);
        }
        if ui
            .button("Remove path")
            .on_hover_text("Removes this path and its points. The path the other way is kept.")
            .clicked()
        {
            match taxi::remove_path(&mut session.tables, id) {
                Ok(done) => {
                    flightpaths::record(session, "Remove flight path", done);
                    session.status = format!("path {id} removed");
                    flights.path = None;
                    flights.point = None;
                    flights.stale();
                }
                Err(e) => session.status = e.to_string(),
            }
        }
    });
    // The points fold away by default: a shipped flight has twenty to sixty,
    // and the selected one has a section of its own under this.
    egui::CollapsingHeader::new(format!("{} points", route.points.len()))
        .id_salt(("flightpaths", "points"))
        .default_open(false)
        .show(ui, |ui| points_list(ui, session, flights, &route));
}

/// Every point of a path, one selectable row each.
fn points_list(ui: &mut egui::Ui, session: &EditSession, flights: &mut Flightpaths, route: &Route) {
    for point in &route.points {
        let chosen = flights.point == Some(point.id);
        let above = crate::tools::doodads::ground_height(session, point.at[0], point.at[1])
            .map(|ground| format!("{:+.0} above the ground", point.at[2] - ground))
            .unwrap_or_default();
        let other_map = match point.map == flights.map {
            true => String::new(),
            false => format!("  map {}", point.map),
        };
        let text = format!(
            "{:>3}  z {:.0}  {above}{other_map}",
            point.index, point.at[2]
        );
        if ui
            .selectable_label(chosen, egui::RichText::new(text).monospace().size(theme::SMALL))
            .clicked()
        {
            flights.select_point(point.id);
        }
    }
}

/// Move a path's first point to its from node and its last point to its to
/// node, as one gesture.
fn snap_ends(session: &mut EditSession, flights: &mut Flightpaths, route: &Route, now: f64) {
    let subject = format!("TaxiPath {} ends", route.path.id);
    let ends = [
        (route.points.first(), route.path.from),
        (route.points.last(), route.path.to),
    ];
    let mut moved = 0;
    for (point, node) in ends {
        let (Some(point), Some(node)) = (point, flights.node(node)) else {
            continue;
        };
        tables::set_fields(
            session,
            taxi::POINTS,
            point.record,
            &[
                (wf::X, node.at[0].to_bits()),
                (wf::Y, node.at[1].to_bits()),
                (wf::Z, node.at[2].to_bits()),
            ],
            "Snap path ends",
            &subject,
            now,
        );
        moved += 1;
    }
    session.status = match moved {
        0 => "neither end's node is on this map".to_string(),
        n => format!("{n} end(s) moved to their nodes"),
    };
    flights.stale();
}

fn point_block(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    id: u32,
    now: f64,
) {
    let Some((route, point)) = flights.point(id).map(|(r, p)| (r.clone(), *p)) else {
        return;
    };
    let mut at = point.at;
    let mut moved = false;
    for (axis, label) in ["x", "y", "z"].iter().enumerate() {
        theme::row(ui, label, |ui| {
            moved |= ui
                .add(egui::DragValue::new(&mut at[axis]).speed(0.5).suffix(" yd"))
                .changed();
        });
    }
    if moved {
        flightpaths::move_point(session, flights, id, bevy::math::Vec3::from(at), now);
        flights.stale();
    }
    if let Some(ground) = crate::tools::doodads::ground_height(session, at[0], at[1]) {
        theme::row(ui, "ground", |ui| {
            ui.label(theme::number(format!(
                "{ground:.1}, {:+.1} yd below the point",
                ground - at[2]
            )));
        });
    }
    let (mut flag, mut delay) = (point.action_flag, point.delay);
    let mut changed = false;
    theme::row(ui, "ActionFlag", |ui| {
        changed |= ui
            .add(egui::DragValue::new(&mut flag).range(0..=2))
            .on_hover_text("1 is a teleport to the next point, 2 a stop. A flight uses 0.")
            .changed();
    });
    theme::row(ui, "Delay", |ui| {
        changed |= ui
            .add(egui::DragValue::new(&mut delay).suffix(" s"))
            .on_hover_text("Seconds waited at a stop. Read only when ActionFlag is 2.")
            .changed();
    });
    if changed {
        tables::set_fields(
            session,
            taxi::POINTS,
            point.record,
            &[(wf::ACTION_FLAG, flag), (wf::DELAY, delay)],
            "Change flight point",
            &format!("TaxiPathNode {id} flags"),
            now,
        );
        flights.stale();
    }
    ui.horizontal(|ui| {
        if ui.button("Fly to").clicked() {
            flights.fly_to(point.at);
        }
        if ui
            .add_enabled(route.points.len() > 2, egui::Button::new("Remove point"))
            .on_hover_text("Delete. The later points move down by one.")
            .on_disabled_hover_text("A path needs at least two points.")
            .clicked()
        {
            let line = flightpaths::remove_selected_point(session, flights);
            session.status = line;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_written_as_the_game_writes_it() {
        assert_eq!(money(0), "0c");
        assert_eq!(money(95), "95c");
        assert_eq!(money(250), "2s 50c");
        assert_eq!(money(12_345), "1g 23s 45c");
    }

    #[test]
    fn the_two_readings_of_a_nodes_side_are_named_when_they_differ() {
        let node = |mounts: [u32; 2]| taxi::Node {
            id: 9,
            record: 0,
            map: 0,
            at: [0.0; 3],
            name: String::new(),
            mounts,
        };
        assert_eq!(sides(&node([0, 541])), "Offered to the Alliance.");
        assert_eq!(sides(&node([2224, 541])), "Offered to both sides.");
        // Node 9's shape: the Alliance gryphon in the column vmangos reads as
        // the Horde's.
        assert_eq!(
            sides(&node([541, 0])),
            "The server offers it to the Horde; the client's flight map draws it for the Alliance."
        );
    }
}
