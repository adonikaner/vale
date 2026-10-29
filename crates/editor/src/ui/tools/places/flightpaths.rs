//! The flight path tool's controls, in two places.
//!
//! ```text
//! viewport toolbar   what a click in the world does: Select, New Node,
//!                    Connect, Add Points, and under the armed one its own
//!                    settings and what the next click does; Show Transports
//!                    and Show Unselected, which decide which paths are drawn
//! inspector          the map's summary and flight map, then the properties
//!                    of one selection: the node, the path or the point,
//!                    chosen with a breadcrumb when more than one is selected
//! ```
//!
//! A mode and its settings decide what the next click in the world makes, so
//! they are drawn over the world, beside the pointer that uses them, and only
//! the armed mode's settings are shown. The inspector edits what is already
//! selected.
//!
//! Each selection is drawn as named sections of property rows: the name in a
//! fixed column, the value or its control after it. A section ends with one
//! line saying how to change what it shows in the viewport. A node's paths and
//! a path's points are tables, one row each, and a click on a row selects it.
//!
//! Selecting a point also selects its path, and a path keeps the node it was
//! reached from, so up to three selections are held at once. The inspector
//! shows one of them: the most specific when the selection changes, or the
//! one picked in the breadcrumb. Which one is shown is drawing state kept in
//! egui's memory; the tool's own selection is unchanged by it.
//!
//! Every number here writes through `crate::tables::fields::set_fields` under a
//! gesture key, so a value dragged through forty steps is one undo entry. Every
//! operation that adds or removes a row goes through `vale_edit::dbc::taxi`
//! and is one undo entry. See `crate::tools::places::flightpaths` for the pointer.

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tables;
use crate::tools::places::flightpaths::{self, Armed, Flightpaths, Route};
use crate::ui::widgets::reference;
use crate::ui::widgets::theme::{self, property};
use vale_assets::tables::taxi::{node_fields as nf, path_fields as pf, path_node_fields as wf};
use vale_edit::dbc::taxi;

/// What the panel is drawn from.
pub(crate) struct Subject<'a> {
    pub(crate) session: &'a mut EditSession,
    pub(crate) flights: &'a mut Flightpaths,
    pub(crate) assets: &'a vale_client::assets::GameAssets,
    pub(crate) now: f64,
    /// The top bar's Server… popover, which holds Apply and Put back.
    pub(crate) server_panel: &'a mut crate::ui::shell::popover::Popover,
    /// Where the Fly To buttons send their requests. See
    /// [`crate::camera::FlyTo`].
    pub(crate) fly: &'a mut bevy::prelude::Messages<crate::camera::FlyTo>,
}

/// The size of a selection's title.
const TITLE: f32 = 17.0;

/// How tall the path's height profile is drawn.
const PROFILE_HEIGHT: f32 = 110.0;

/// The height of one row of a table.
const TABLE_ROW: f32 = 20.0;

/// Which of the three selections the inspector shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Node,
    Path,
    Point,
}

/// The selection the inspector last drew and the level it showed, kept in
/// egui's memory. A change of selection shows the most specific level again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Focus {
    selection: (Option<u32>, Option<u32>, Option<u32>),
    level: Level,
}

pub(crate) fn draw(ui: &mut egui::Ui, subject: Subject<'_>) {
    let Subject {
        session,
        flights,
        assets,
        now,
        server_panel,
        fly,
    } = subject;
    for table in taxi::TABLES {
        if !session.open_table(assets, table) {
            theme::note(ui, format!("Opening {table}.dbc\u{2026}"));
            return;
        }
    }

    overview(ui, session, flights, assets);
    ui.add_space(8.0);
    let level = focus(ui, flights);

    egui::ScrollArea::vertical()
        .id_salt("flightpaths")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            match (level, flights.node, flights.path, flights.point) {
                (Some(Level::Point), _, _, Some(id)) => point_card(ui, session, flights, fly, id, now),
                (Some(Level::Path), _, Some(id), _) => path_card(ui, session, flights, fly, id, now),
                (Some(Level::Node), Some(id), _, _) => node_card(ui, session, flights, fly, id, now),
                _ => nothing_selected(ui),
            }
            ui.add_space(16.0);
            server_footer(ui, server_panel);
            ui.add_space(12.0);
        });
}

/// The map's counts, the three tables' validation, and the flight map, in one
/// sunk block at the top of the panel.
fn overview(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    assets: &vale_client::assets::GameAssets,
) {
    egui::Frame::new()
        .fill(theme::SUNK)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let flights_here = flights.routes.iter().filter(|route| !route.transport).count();
            let transports = flights.routes.len() - flights_here;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                count(ui, flights.nodes.len(), "nodes");
                ui.label(egui::RichText::new("\u{b7}").color(theme::INK_FAINT));
                count(ui, flights_here, "flight paths");
                ui.label(egui::RichText::new("\u{b7}").color(theme::INK_FAINT));
                count(ui, transports, "transport paths");
            });
            if !flights.findings.is_empty() {
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!("Validation: {} warning(s)", flights.findings.len())).color(theme::WARN),
                )
                .id_salt(("flightpaths", "findings"))
                .default_open(false)
                .show(ui, |ui| {
                    for finding in &flights.findings {
                        ui.label(egui::RichText::new(finding.to_string()).size(theme::SMALL).color(theme::WARN));
                    }
                })
                .header_response
                .on_hover_text("Problems the server reports when it loads the three tables.");
            }
            ui.add_space(2.0);
            flight_map_line(ui, session, flights, assets);
        });
}

/// A number and what it counts, the number in the body colour.
fn count(ui: &mut egui::Ui, n: usize, what: &str) {
    ui.label(theme::number(n.to_string()));
    ui.label(egui::RichText::new(what).size(theme::SMALL).color(theme::INK_DIM));
}

/// Whether the client opens a flight map on this map, with its size, its
/// problems and the commands that make, fit or redraw it.
fn flight_map_line(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    assets: &vale_client::assets::GameAssets,
) {
    use vale_edit::flightmap;
    for table in [flightmap::TABLE, flightpaths::WORLD_MAP_AREA] {
        if !session.open_table(assets, table) {
            theme::note(ui, format!("Opening {table}.dbc\u{2026}"));
            return;
        }
    }
    let Some(state) = flightpaths::flight_map(session, flights) else {
        return;
    };
    let mut make = false;
    let mut redraw: Option<flightmap::FlightMap> = None;
    match &state {
        flightpaths::FlightMap::Shipped => {
            property(
                ui,
                "Flight Map",
                "This map is a world map continent. Its flight map is part of the game data and is not edited here.",
                |ui| value(ui, "Standard"),
            );
        }
        flightpaths::FlightMap::Missing => {
            property(
                ui,
                "Flight Map",
                &format!(
                    "WorldMapContinent.dbc has no row for map {}. The 1.12.1 client opens no flight map \
                     here, so a flight master on this map offers no flights.",
                    flights.map
                ),
                |ui| {
                    let colour = match flights.nodes.is_empty() {
                        true => theme::INK,
                        false => theme::WARN,
                    };
                    ui.label(egui::RichText::new("None").color(colour));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        make = ui
                            .small_button("Create")
                            .on_hover_text(
                                "Add a WorldMapContinent.dbc row for this map, with a square box over its \
                                 tiles and nodes, and draw Interface\\TaxiFrame\\TAXIMAP<map>.blp from its \
                                 minimap tiles. Client only: the server reads neither.",
                            )
                            .clicked();
                    });
                },
            );
        }
        flightpaths::FlightMap::Made { flight, outside } => {
            let side = flight.taxi_box[2] - flight.taxi_box[0];
            property(
                ui,
                "Flight Map",
                "This map's WorldMapContinent.dbc row. The client draws only the nodes inside its box.",
                |ui| {
                    value(ui, format!("Custom, {side:.0} yd square"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("\u{2026}", |ui| {
                            if ui
                                .button("Refit Flight Map")
                                .on_hover_text("Reset the box to cover this map's tiles and nodes, and redraw TAXIMAP<map>.blp.")
                                .clicked()
                            {
                                make = true;
                                ui.close();
                            }
                            if ui
                                .button("Redraw Image")
                                .on_hover_text("Redraw TAXIMAP<map>.blp from the current minimap tiles, with the same box.")
                                .clicked()
                            {
                                redraw = Some(*flight);
                                ui.close();
                            }
                        })
                        .response
                        .on_hover_text("Flight map commands");
                    });
                },
            );
            if !outside.is_empty() {
                let ids: Vec<String> = outside.iter().map(u32::to_string).collect();
                warn_line(ui, &format!("Outside the flight map: node {}", ids.join(", ")))
                    .on_hover_text("The client does not draw a node outside the flight map's box. Refit the flight map to include it.");
            }
            if !flightpaths::has_picture(assets, flights) {
                warn_line(ui, "Flight map image missing")
                    .on_hover_text("The client draws the flight map's nodes on a blank background. Redraw Image draws it.");
            }
        }
    }
    if make {
        session.status = flightpaths::make_flight_map(session, assets, flights);
    }
    if let Some(flight) = redraw {
        session.status = flightpaths::draw_flight_map(session, assets, flights, &flight);
    }
}

/// One small line in the warning colour.
fn warn_line(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.label(egui::RichText::new(text).size(theme::SMALL).color(theme::WARN))
}

/// The level the inspector shows, and the breadcrumb that chooses it when more
/// than one level is selected.
fn focus(ui: &mut egui::Ui, flights: &mut Flightpaths) -> Option<Level> {
    let selection = (flights.node, flights.path, flights.point);
    let held: Vec<Level> = [
        (selection.0.is_some(), Level::Node),
        (selection.1.is_some(), Level::Path),
        (selection.2.is_some(), Level::Point),
    ]
    .into_iter()
    .filter_map(|(on, level)| on.then_some(level))
    .collect();
    let deepest = *held.last()?;
    let id = egui::Id::new(("flightpaths", "focus"));
    let kept: Option<Focus> = ui.data(|data| data.get_temp(id));
    let mut level = match kept {
        Some(kept) if kept.selection == selection && held.contains(&kept.level) => kept.level,
        _ => deepest,
    };
    if held.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (at, crumb) in held.iter().enumerate() {
                if at > 0 {
                    ui.label(egui::RichText::new("\u{203a}").color(theme::INK_FAINT));
                }
                let name = match crumb {
                    Level::Node => short(&selection.0.map(|id| flights.name(id)).unwrap_or_default(), 22),
                    Level::Path => format!("Path {}", selection.1.unwrap_or(0)),
                    Level::Point => match selection.2.and_then(|id| flights.point(id)) {
                        Some((_, point)) => format!("Point {}", point.index),
                        None => "Point".to_string(),
                    },
                };
                if ui
                    .add(egui::Button::selectable(level == *crumb, egui::RichText::new(name).size(theme::SMALL)))
                    .clicked()
                {
                    level = *crumb;
                }
            }
        });
        ui.add_space(6.0);
    }
    ui.data_mut(|data| data.insert_temp(id, Focus { selection, level }));
    Some(level)
}

/// A name cut to `most` characters, with an ellipsis when cut.
fn short(name: &str, most: usize) -> String {
    match name.chars().count() > most {
        true => format!("{}\u{2026}", name.chars().take(most - 1).collect::<String>()),
        false => name.to_string(),
    }
}

/// The selection's title: one line, cut with an ellipsis to the panel's width.
fn title(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).strong().size(TITLE).color(theme::INK)).truncate());
}

/// A property's value that is not edited here.
fn value(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.add(egui::Label::new(egui::RichText::new(text.into()).color(theme::INK)).truncate())
}

/// A property's value in another colour.
fn value_in(ui: &mut egui::Ui, text: impl Into<String>, colour: egui::Color32) -> egui::Response {
    ui.add(egui::Label::new(egui::RichText::new(text.into()).color(colour)).truncate())
}

/// The panel with nothing selected: what each toolbar mode does, and the key
/// to the node colours.
fn nothing_selected(ui: &mut egui::Ui) {
    title(ui, "No Selection");
    theme::note(ui, "Click a node, a path or a point in the viewport to show its properties here.");
    theme::heading(ui, "Viewport Toolbar");
    for (mode, what) in [
        ("Select", "Select nodes, paths and points. Drag to move; Ctrl + drag moves vertically."),
        ("New Node", "Click the ground to create a node."),
        ("Connect", "With a node selected, click another node to create a path to it."),
        ("Add Points", "With a path selected, click near it to insert a point."),
        ("Show Transports", "Also draw the boat and zeppelin paths."),
        ("Show Unselected", "Draw every path. Off, only the selection's paths are drawn."),
    ] {
        property(ui, mode, "", |ui| {
            ui.add(egui::Label::new(egui::RichText::new(what).size(theme::SMALL).color(theme::INK_DIM)).wrap());
        });
    }
    theme::heading(ui, "Legend");
    for (mounts, label) in [
        ([1, 1], "Node available to both factions"),
        ([1, 0], "Node available to the Horde"),
        ([0, 1], "Node available to the Alliance"),
        ([0, 0], "Transport dock: no mount for either faction"),
    ] {
        ui.horizontal(|ui| {
            dot(ui, side_colour(mounts), 5.0);
            ui.label(egui::RichText::new(label).color(theme::INK_DIM));
        });
    }
}

/// The colour the world draws a node with these mounts in, for the panel.
fn side_colour(mounts: [u32; 2]) -> egui::Color32 {
    let colour = flightpaths::side_colour(mounts).to_srgba();
    egui::Color32::from_rgb(
        (colour.red * 255.0) as u8,
        (colour.green * 255.0) as u8,
        (colour.blue * 255.0) as u8,
    )
}

/// A filled circle the height of a line of text.
fn dot(ui: &mut egui::Ui, colour: egui::Color32, radius: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(radius * 2.0 + 4.0, ui.spacing().interact_size.y), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), radius, colour);
}

/// The height of `at` over the ground under it, as a property row. Nothing
/// when the ground there is not loaded.
fn above_ground(ui: &mut egui::Ui, session: &EditSession, at: [f32; 3]) {
    let Some(ground) = crate::tools::placements::doodads::ground_height(session, at[0], at[1]) else {
        return;
    };
    let above = at[2] - ground;
    property(ui, "Above Ground", "Z minus the height of the loaded ground directly below.", |ui| {
        let colour = match above < 0.0 {
            true => theme::WARN,
            false => theme::INK,
        };
        value_in(ui, format!("{above:.1} yd"), colour);
        ui.label(egui::RichText::new(format!("ground Z {ground:.1}")).size(theme::SMALL).color(theme::INK_FAINT));
    });
}

/// A row of command buttons, with a destructive one at the right end.
fn actions(ui: &mut egui::Ui, left: impl FnOnce(&mut egui::Ui), remove: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        left(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), remove);
    });
}

/// A button in the danger colour.
fn remove_button(ui: &mut egui::Ui, text: &str, hover: &str) -> bool {
    ui.button(egui::RichText::new(text).color(theme::BAD))
        .on_hover_text(hover)
        .clicked()
}

/// A copper amount as the game writes it.
pub(crate) fn money(copper: u32) -> String {
    let (gold, silver, copper) = (copper / 10_000, copper / 100 % 100, copper % 100);
    match (gold, silver) {
        (0, 0) => format!("{copper}c"),
        (0, _) => format!("{silver}s {copper}c"),
        _ => format!("{gold}g {silver}s {copper}c"),
    }
}

/// A table's column widths across `width`: the fixed ones as given, and the
/// one given as 0 taking what is left.
fn column_widths(width: f32, widths: &[f32]) -> Vec<f32> {
    let rest = (width - widths.iter().sum::<f32>()).max(0.0);
    widths.iter().map(|&w| if w == 0.0 { rest } else { w }).collect()
}

/// The rectangles of a row's cells, left to right.
fn cells(row: egui::Rect, widths: &[f32]) -> Vec<egui::Rect> {
    let mut x = row.left();
    widths
        .iter()
        .map(|&w| {
            let cell = egui::Rect::from_min_size(egui::pos2(x, row.top()), egui::vec2(w, row.height()));
            x += w;
            cell
        })
        .collect()
}

/// Text in a cell, cut with an ellipsis to the cell's width.
fn cell_text(painter: &egui::Painter, cell: egui::Rect, text: &str, colour: egui::Color32, right: bool) {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat::simple(egui::FontId::proportional(theme::SMALL + 0.5), colour),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((cell.width() - 8.0).max(0.0));
    let galley = painter.layout_job(job);
    let x = match right {
        true => cell.right() - 4.0 - galley.size().x,
        false => cell.left() + 4.0,
    };
    painter.galley(egui::pos2(x, cell.center().y - galley.size().y * 0.5), galley, colour);
}

/// A table's header row: the column names, small and faint, over a rule.
fn table_header(ui: &mut egui::Ui, names: &[(&str, bool)], widths: &[f32]) {
    let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), TABLE_ROW - 2.0), egui::Sense::hover());
    let painter = ui.painter();
    for (cell, (name, right)) in cells(row, &column_widths(row.width(), widths)).into_iter().zip(names) {
        cell_text(painter, cell, name, theme::INK_FAINT, *right);
    }
    painter.hline(row.x_range(), row.bottom(), egui::Stroke::new(1.0, theme::LINE));
}

/// The fill of a table row or cell: the chosen one in the accent, a hovered
/// one raised.
fn row_fill(painter: &egui::Painter, rect: egui::Rect, chosen: bool, hovered: bool) {
    let fill = match (chosen, hovered) {
        (true, _) => theme::ACCENT_SUNK,
        (false, true) => theme::RAISED,
        (false, false) => return,
    };
    painter.rect_filled(rect, 2.0, fill);
}

fn node_card(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    fly: &mut bevy::prelude::Messages<crate::camera::FlyTo>,
    id: u32,
    now: f64,
) {
    let Some(node) = flights.node(id).cloned() else {
        theme::note(ui, format!("Node {id} is not on this map."));
        return;
    };
    title(ui, &node.name);

    theme::heading(ui, "Taxi Node");
    property(ui, "ID", "The TaxiNodes.dbc row ID, which is also the server's taxi_nodes ID.", |ui| {
        value(ui, id.to_string());
    });
    let mut name = node.name.clone();
    property(ui, "Name", "The name the flight map shows for the node.", |ui| {
        let edit = egui::TextEdit::singleline(&mut name).desired_width(ui.available_width()).hint_text("Name");
        if ui.add(edit).changed() {
            tables::fields::set_text(session, taxi::NODES, node.record, nf::NAME, &name, "Rename flight node", now);
            flights.stale();
        }
    });
    property(
        ui,
        "Available To",
        "The factions the server offers the node to: those with a mount set under Mounts.",
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            dot(ui, side_colour(node.mounts), 4.0);
            value(ui, side_word(&node));
        },
    );
    if let Some(differ) = sides_differ(&node) {
        warn_line(ui, &differ);
    }

    theme::heading(ui, "Location");
    let mut at = node.at;
    if theme::xyz(ui, &mut at) {
        flightpaths::move_node(session, flights, id, bevy::math::Vec3::from(at), now);
        flights.stale();
    }
    above_ground(ui, session, at);
    ui.checkbox(&mut flights.carry_ends, "Move path ends with the node").on_hover_text(
        "Moving the node also moves the first point of every path leaving it and the last \
         point of every path arriving at it.",
    );
    theme::note(ui, "To move the node, drag it in the viewport. Ctrl + drag moves it vertically.");

    theme::heading(ui, "Mounts");
    let mut mounts = node.mounts;
    let mut remounted = false;
    for (n, (label, tip)) in [
        (
            "Horde Mount",
            "MountCreatureId 1, a creature_template entry. vmangos offers the node to the Horde \
             only when this is not 0, and flies this creature. 2224 is the wind rider.",
        ),
        (
            "Alliance Mount",
            "MountCreatureId 2. vmangos offers the node to the Alliance only when this is not 0. \
             541 is the gryphon.",
        ),
    ]
    .iter()
    .enumerate()
    {
        property(ui, label, tip, |ui| {
            remounted |= ui.add(egui::DragValue::new(&mut mounts[n]).speed(1.0)).on_hover_text(*tip).changed();
            if mounts[n] == 0 {
                ui.label(egui::RichText::new("None").size(theme::SMALL).color(theme::INK_FAINT));
            }
        });
    }
    if remounted {
        tables::fields::set_fields(
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
    theme::note(
        ui,
        "Creature entries. 0 makes the node unavailable to that faction. Standard mounts: 2224 (Horde), 541 (Alliance).",
    );

    let mut removed = false;
    actions(
        ui,
        |ui| {
            if ui.button("Fly To").on_hover_text("Move the camera to the node.").clicked() {
                fly.write(crate::camera::FlyTo::at(node.at.into()));
            }
            if ui
                .button("Snap to Ground")
                .on_hover_text("Set Z to the ground under the node, where the ground is loaded.")
                .clicked()
            {
                if let Some(ground) = crate::tools::placements::doodads::ground_height(session, at[0], at[1]) {
                    let to = bevy::math::Vec3::new(at[0], at[1], ground);
                    flightpaths::move_node(session, flights, id, to, now);
                    flights.stale();
                }
            }
        },
        |ui| {
            removed = remove_button(
                ui,
                "Remove Node",
                "Remove the node, every path leaving or arriving at it, and their points. One undo entry.",
            );
        },
    );
    if removed {
        match taxi::remove_node(&mut session.tables, id) {
            Ok(done) => {
                flightpaths::record(session, "Remove flight node", done);
                session.status = format!("Node {id} removed");
                flights.node = None;
                flights.path = None;
                flights.point = None;
                flights.stale();
            }
            Err(e) => session.status = e.to_string(),
        }
        return;
    }

    connections(ui, flights, id);
}

/// One node this node has a path to or from: the path each way, as its id,
/// cost and length.
struct Link {
    other: u32,
    out: Option<(u32, u32, f32)>,
    back: Option<(u32, u32, f32)>,
}

/// The nodes this one has paths to or from, as a table: the other node, the
/// cost of the path there and of the path back. A click on a cost selects that
/// path; a click on a name selects that node. A missing direction is named in
/// the warning colour.
fn connections(ui: &mut egui::Ui, flights: &mut Flightpaths, id: u32) {
    let (leaving, arriving) = flights.routes_of(id);
    let mut links: Vec<Link> = Vec::new();
    let mut add = |other: u32, path: (u32, u32, f32), outward: bool| {
        let at = match links.iter().position(|link| link.other == other) {
            Some(at) => at,
            None => {
                links.push(Link { other, out: None, back: None });
                links.len() - 1
            }
        };
        match outward {
            true => links[at].out = Some(path),
            false => links[at].back = Some(path),
        }
    };
    for route in leaving {
        add(route.path.to, (route.path.id, route.path.cost, route.length), true);
    }
    for route in arriving {
        add(route.path.from, (route.path.id, route.path.cost, route.length), false);
    }
    links.sort_by_key(|link| flights.name(link.other).to_lowercase());

    theme::heading(ui, &format!("Paths ({})", links.len()));
    if links.is_empty() {
        theme::note(ui, "No paths. To add one, choose Connect in the viewport toolbar and click another node.");
        return;
    }
    let widths = [0.0, 72.0, 72.0];
    table_header(ui, &[("Node", false), ("Outbound", true), ("Return", true)], &widths);
    let mut chosen_path: Option<u32> = None;
    let mut chosen_node: Option<u32> = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for link in &links {
            let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), TABLE_ROW), egui::Sense::hover());
            let parts = cells(row, &column_widths(row.width(), &widths));
            let name = flights.name(link.other);
            let name_cell = ui.interact(parts[0], ui.id().with(("node", link.other)), egui::Sense::click());
            row_fill(ui.painter(), parts[0], false, name_cell.hovered());
            cell_text(ui.painter(), parts[0], &name, theme::INK, false);
            if name_cell.on_hover_text(format!("Node {}. Click to select it.", link.other)).clicked() {
                chosen_node = Some(link.other);
            }
            for (cell, (path, which)) in parts[1..].iter().zip([(link.out, "out"), (link.back, "back")]) {
                match path {
                    Some((path, cost, length)) => {
                        let response = ui.interact(*cell, ui.id().with((which, link.other)), egui::Sense::click());
                        row_fill(ui.painter(), *cell, flights.path == Some(path), response.hovered());
                        cell_text(ui.painter(), *cell, &money(cost), theme::INK, true);
                        let (from, to) = match which {
                            "out" => (id, link.other),
                            _ => (link.other, id),
                        };
                        if response
                            .on_hover_text(format!(
                                "Path {path}: {} to {}, {length:.0} yd. Click to select it.",
                                flights.name(from),
                                flights.name(to)
                            ))
                            .clicked()
                        {
                            chosen_path = Some(path);
                        }
                    }
                    None => cell_text(ui.painter(), *cell, "None", theme::WARN, true),
                }
            }
        }
    });
    theme::note(ui, "Click a cost to select that path. To add a path, choose Connect in the viewport toolbar.");
    if let Some(path) = chosen_path {
        flights.select_path(path);
    } else if let Some(node) = chosen_node {
        flights.select_node(node);
    }
}

/// Which factions the node is offered to.
fn side_word(node: &taxi::Node) -> &'static str {
    match (node.mounts[0] != 0, node.mounts[1] != 0) {
        (true, true) => "Horde and Alliance",
        (true, false) => "Horde",
        (false, true) => "Alliance",
        (false, false) => "None (transport dock)",
    }
}

/// When the server and the client's flight map read the node's two mount
/// columns differently, a line that says so.
fn sides_differ(node: &taxi::Node) -> Option<String> {
    let line = sides(node);
    line.starts_with("The server").then_some(line)
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

/// One point as the path's profile and points table draw it.
struct Sample {
    id: u32,
    index: u32,
    at: [f32; 3],
    action_flag: u32,
    /// Yards along the path from its first point, across the ground.
    along: f32,
    ground: Option<f32>,
    /// On another map than the open one.
    away: bool,
}

/// A route's points with their distance along the path and the ground under
/// each.
fn samples(session: &EditSession, flights: &Flightpaths, route: &Route) -> Vec<Sample> {
    let mut samples: Vec<Sample> = Vec::with_capacity(route.points.len());
    let mut along = 0.0;
    let mut last: Option<[f32; 3]> = None;
    for point in &route.points {
        let away = point.map != flights.map;
        if let (Some(last), false) = (last, away) {
            along += (point.at[0] - last[0]).hypot(point.at[1] - last[1]);
        }
        if !away {
            last = Some(point.at);
        }
        samples.push(Sample {
            id: point.id,
            index: point.index,
            at: point.at,
            action_flag: point.action_flag,
            along,
            ground: match away {
                true => None,
                false => crate::tools::placements::doodads::ground_height(session, point.at[0], point.at[1]),
            },
            away,
        });
    }
    samples
}

fn path_card(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    fly: &mut bevy::prelude::Messages<crate::camera::FlyTo>,
    id: u32,
    now: f64,
) {
    let Some(route) = flights.route(id).cloned() else {
        return;
    };
    title(ui, &format!("{} to {}", flights.name(route.path.from), flights.name(route.path.to)));
    let back = flights
        .routes
        .iter()
        .find(|other| other.path.from == route.path.to && other.path.to == route.path.from)
        .map(|other| other.path.id);

    theme::heading(ui, "Taxi Path");
    property(ui, "ID", "The TaxiPath.dbc row ID.", |ui| {
        value(ui, id.to_string());
    });
    for (name, end) in [("From", route.path.from), ("To", route.path.to)] {
        property(ui, name, "", |ui| {
            if reference::link(ui, &flights.name(end), theme::INK, &format!("Node {end}. Click to select it and move the camera to it.")) {
                flights.select_node(end);
                if let Some(node) = flights.node(end) {
                    let at = node.at;
                    fly.write(crate::camera::FlyTo::at(at.into()));
                }
            }
        });
    }
    property(
        ui,
        "Type",
        "A transport path has no mount at either end. Boats and zeppelins use these rows.",
        |ui| {
            value(ui, match route.transport {
                true => "Transport",
                false => "Flight",
            })
        },
    );
    let mut cost = route.path.cost;
    property(ui, "Cost", "Copper, before the reputation discount the server applies.", |ui| {
        if ui.add(egui::DragValue::new(&mut cost).speed(10.0).suffix(" c")).changed() {
            tables::fields::set_fields(
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
        ui.label(egui::RichText::new(money(cost)).size(theme::SMALL).color(theme::INK_DIM));
    });
    property(ui, "Length", "Measured point to point, including height, over the points on this map.", |ui| {
        value(ui, format!("{:.0} yd", route.length));
    });
    property(ui, "Return Path", "The path from this path's end node back to its start node.", |ui| match back {
        Some(back) => {
            if reference::link(ui, &format!("Path {back}"), theme::INK, "Click to select the return path.") {
                flights.select_path(back);
            }
        }
        None => {
            value_in(ui, "None", theme::WARN);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("Create")
                    .on_hover_text("Create the path in the opposite direction through the same points, at the same cost.")
                    .clicked()
                {
                    match taxi::reverse_path(&mut session.tables, id) {
                        Ok(done) => {
                            let made = done.made;
                            flightpaths::record(session, "Create return path", done);
                            session.status = format!("Path {} created", made.unwrap_or(0));
                            flights.path = made;
                            flights.point = None;
                            flights.stale();
                        }
                        Err(e) => session.status = e.to_string(),
                    }
                }
            });
        }
    });

    let mut removed = false;
    actions(
        ui,
        |ui| {
            if ui
                .button("Fly To")
                .on_hover_text("Frame the whole path. From that far back the fog can hide the ground; FOG on the view bar turns it off.")
                .clicked()
            {
                if let Some(flight) = flights.path_flight(id) {
                    fly.write(flight);
                }
            }
            if ui
                .button("Snap Ends")
                .on_hover_text("Move the first point to the node the path leaves and the last to the node it reaches.")
                .clicked()
            {
                snap_ends(session, flights, &route, now);
            }
        },
        |ui| {
            removed = remove_button(ui, "Remove Path", "Remove this path and its points. The return path is kept.");
        },
    );
    if removed {
        match taxi::remove_path(&mut session.tables, id) {
            Ok(done) => {
                flightpaths::record(session, "Remove flight path", done);
                session.status = format!("Path {id} removed");
                flights.path = None;
                flights.point = None;
                flights.stale();
            }
            Err(e) => session.status = e.to_string(),
        }
        return;
    }

    let samples = samples(session, flights, &route);
    theme::heading(ui, "Height Profile");
    profile(ui, flights, &samples);
    theme::note(ui, "Line: the path's height. Shaded: the ground below it. Click a point to select it.");

    theme::heading(ui, &format!("Points ({})", samples.len()));
    points_table(ui, flights, &samples);
    theme::note(
        ui,
        "To move a point, drag it in the viewport; Ctrl + drag moves it vertically. To insert one, \
         choose Add Points in the viewport toolbar. Delete removes the selected point.",
    );
}

/// The path's points as a table: index, X, Y, Z and action. A Z under the
/// ground is in the warning colour. A click on a row selects the point.
fn points_table(ui: &mut egui::Ui, flights: &mut Flightpaths, samples: &[Sample]) {
    let share = ((ui.available_width() - 28.0 - 64.0) / 3.0).max(0.0);
    let widths = [28.0, share, share, share, 64.0];
    table_header(
        ui,
        &[("#", false), ("X", true), ("Y", true), ("Z", true), ("Action", false)],
        &widths,
    );
    let mut chosen: Option<u32> = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for s in samples {
            let (row, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), TABLE_ROW), egui::Sense::click());
            let painter = ui.painter();
            row_fill(painter, row, flights.point == Some(s.id), response.hovered());
            let parts = cells(row, &column_widths(row.width(), &widths));
            let below = s.ground.is_some_and(|ground| s.at[2] < ground);
            let action = ACTIONS
                .iter()
                .find(|(_, flag)| *flag == s.action_flag)
                .map_or_else(|| s.action_flag.to_string(), |(name, _)| name.to_string());
            let dim = match s.away {
                true => theme::WARN,
                false => theme::INK_DIM,
            };
            cell_text(painter, parts[0], &s.index.to_string(), dim, false);
            cell_text(painter, parts[1], &format!("{:.1}", s.at[0]), theme::INK, true);
            cell_text(painter, parts[2], &format!("{:.1}", s.at[1]), theme::INK, true);
            cell_text(
                painter,
                parts[3],
                &format!("{:.1}", s.at[2]),
                match below {
                    true => theme::WARN,
                    false => theme::INK,
                },
                true,
            );
            cell_text(painter, parts[4], &action, theme::INK_DIM, false);
            let response = match (s.away, below) {
                (true, _) => response.on_hover_text("On another map."),
                (false, true) => response.on_hover_text("Below the ground."),
                (false, false) => response,
            };
            if response.clicked() {
                chosen = Some(s.id);
            }
        }
    });
    if let Some(id) = chosen {
        flights.select_point(id);
    }
}

/// The path's points as a height profile: the ground under each point filled
/// in, the path's line over it, a dot for each point. Hovering names the point
/// nearest the pointer; a click selects it.
fn profile(ui: &mut egui::Ui, flights: &mut Flightpaths, samples: &[Sample]) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), PROFILE_HEIGHT), egui::Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, theme::SUNK);
    painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
    if samples.is_empty() {
        return;
    }
    let plot = egui::Rect::from_min_max(rect.min + egui::vec2(8.0, 16.0), rect.max - egui::vec2(8.0, 8.0));
    let heights = samples.iter().flat_map(|s| std::iter::once(s.at[2]).chain(s.ground));
    let (mut low, mut high) = heights.fold((f32::MAX, f32::MIN), |(lo, hi), h| (lo.min(h), hi.max(h)));
    if high - low < 20.0 {
        let middle = (high + low) * 0.5;
        low = middle - 10.0;
        high = middle + 10.0;
    }
    let total = samples.last().map_or(0.0, |s| s.along);
    let x_of = |at: usize, s: &Sample| match total > 0.0 {
        true => plot.left() + s.along / total * plot.width(),
        false => plot.left() + at as f32 / (samples.len().max(2) - 1) as f32 * plot.width(),
    };
    let y_of = |h: f32| plot.bottom() - (h - low) / (high - low) * plot.height();

    // The ground, as a band from the bottom of the plot up to the ground under
    // each pair of neighbouring points.
    for (at, pair) in samples.windows(2).enumerate() {
        if let (Some(a), Some(b)) = (pair[0].ground, pair[1].ground) {
            let (xa, xb) = (x_of(at, &pair[0]), x_of(at + 1, &pair[1]));
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(xa, y_of(a)),
                    egui::pos2(xb, y_of(b)),
                    egui::pos2(xb, rect.bottom() - 1.0),
                    egui::pos2(xa, rect.bottom() - 1.0),
                ],
                theme::RAISED_HOT,
                egui::Stroke::NONE,
            ));
        }
    }
    let line: Vec<egui::Pos2> = samples.iter().enumerate().map(|(at, s)| egui::pos2(x_of(at, s), y_of(s.at[2]))).collect();
    painter.add(egui::Shape::line(line.clone(), egui::Stroke::new(1.5, theme::ACCENT)));

    let hovered = response.hover_pos().and_then(|pointer| {
        line.iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (a.x - pointer.x).abs().total_cmp(&(b.x - pointer.x).abs()))
            .map(|(at, _)| at)
    });
    if let Some(at) = samples.iter().position(|s| Some(s.id) == flights.point) {
        painter.vline(line[at].x, plot.top()..=rect.bottom(), egui::Stroke::new(1.0, theme::LINE));
    }
    for (at, (s, pos)) in samples.iter().zip(&line).enumerate() {
        let chosen = Some(s.id) == flights.point;
        let colour = match (chosen, s.away) {
            (true, _) => theme::ACCENT,
            (false, true) => theme::WARN,
            (false, false) => theme::INK_DIM,
        };
        let radius = match (chosen, Some(at) == hovered) {
            (true, _) => 5.0,
            (false, true) => 4.0,
            (false, false) => 2.5,
        };
        painter.circle_filled(*pos, radius, colour);
        if chosen {
            painter.circle_stroke(*pos, radius, egui::Stroke::new(1.5, theme::INK));
        }
    }
    let scale = egui::FontId::proportional(theme::SMALL - 1.0);
    painter.text(rect.left_top() + egui::vec2(6.0, 3.0), egui::Align2::LEFT_TOP, format!("Z {high:.0}"), scale, theme::INK_FAINT);

    if let Some(at) = hovered {
        let s = &samples[at];
        let above = match (s.away, s.ground) {
            (true, _) => ", on another map".to_string(),
            (false, Some(ground)) => format!(", {:.0} yd above ground", s.at[2] - ground),
            (false, None) => String::new(),
        };
        let response = response.clone().on_hover_text_at_pointer(format!("Point {}: Z {:.0}{above}", s.index, s.at[2]));
        if response.clicked() {
            flights.select_point(s.id);
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
        tables::fields::set_fields(
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
        0 => "Neither end's node is on this map".to_string(),
        n => format!("{n} end(s) moved to their nodes"),
    };
    flights.stale();
}

/// What a point's ActionFlag tells the client to do there.
const ACTIONS: [(&str, u32); 3] = [("Fly", 0), ("Teleport", 1), ("Stop", 2)];

fn point_card(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    fly: &mut bevy::prelude::Messages<crate::camera::FlyTo>,
    id: u32,
    now: f64,
) {
    let Some((route, point)) = flights.point(id).map(|(r, p)| (r.clone(), *p)) else {
        return;
    };
    let at_index = route.points.iter().position(|p| p.id == id).unwrap_or(0);
    title(ui, &format!("Point {}", point.index));

    theme::heading(ui, "Path Point");
    let mut step: Option<u32> = None;
    property(ui, "Index", "The point's place in the path, counted from 0.", |ui| {
        value(ui, format!("{} (last is {})", point.index, route.points.len().saturating_sub(1)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let next = route.points.get(at_index + 1).map(|p| p.id);
            if ui.add_enabled(next.is_some(), egui::Button::new("\u{203a}").small()).on_hover_text("Next point").clicked() {
                step = next;
            }
            let previous = at_index.checked_sub(1).and_then(|at| route.points.get(at)).map(|p| p.id);
            if ui.add_enabled(previous.is_some(), egui::Button::new("\u{2039}").small()).on_hover_text("Previous point").clicked() {
                step = previous;
            }
        });
    });
    property(ui, "ID", "The TaxiPathNode.dbc row ID.", |ui| {
        value(ui, id.to_string());
    });
    property(ui, "Path", "", |ui| {
        let words = format!(
            "Path {}: {} to {}",
            route.path.id,
            flights.name(route.path.from),
            flights.name(route.path.to)
        );
        if reference::link(ui, &words, theme::INK, "Click to select the path.") {
            flights.point = None;
        }
    });
    property(ui, "Map", "The map the point is on. A path that crosses maps has points on each.", |ui| {
        let colour = match point.map == flights.map {
            true => theme::INK,
            false => theme::WARN,
        };
        value_in(ui, point.map.to_string(), colour);
    });
    if let Some(next) = step {
        flights.select_point(next);
        return;
    }
    if flights.point.is_none() {
        return;
    }

    theme::heading(ui, "Location");
    let mut at = point.at;
    if theme::xyz(ui, &mut at) {
        flightpaths::move_point(session, flights, id, bevy::math::Vec3::from(at), now);
        flights.stale();
    }
    above_ground(ui, session, at);
    theme::note(ui, "To move the point, drag it in the viewport. Ctrl + drag moves it vertically.");

    theme::heading(ui, "Action");
    let (mut flag, mut delay) = (point.action_flag, point.delay);
    let mut changed = false;
    property(
        ui,
        "Type",
        "ActionFlag: 0 for a point flown through, 1 a teleport to the next point, 2 a stop. Standard flights use 0.",
        |ui| {
            if ACTIONS.iter().any(|(_, value)| *value == flag) {
                let before = flag;
                theme::segmented(ui, &mut flag, &ACTIONS, |a, b| a == b);
                changed |= flag != before;
            } else {
                changed |= ui.add(egui::DragValue::new(&mut flag).range(0..=2)).changed();
            }
        },
    );
    property(ui, "Delay", "Seconds the flight waits at a Stop point.", |ui| {
        changed |= ui
            .add_enabled(flag == 2, egui::DragValue::new(&mut delay).suffix(" s"))
            .on_disabled_hover_text("Used only when Type is Stop.")
            .changed();
    });
    theme::note(
        ui,
        match flag {
            0 => "Fly: the flight passes through the point.",
            1 => "Teleport: the flight jumps from this point to the next.",
            _ => "Stop: the flight halts here for the delay.",
        },
    );
    if changed {
        tables::fields::set_fields(
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

    let mut removed = false;
    actions(
        ui,
        |ui| {
            if ui.button("Fly To").on_hover_text("Move the camera to the point.").clicked() {
                fly.write(crate::camera::FlyTo::at(point.at.into()));
            }
        },
        |ui| {
            removed = ui
                .add_enabled(route.points.len() > 2, egui::Button::new(egui::RichText::new("Remove Point").color(theme::BAD)))
                .on_hover_text("Remove this point. Each later point's index moves down by one.")
                .on_disabled_hover_text("A path needs at least two points.")
                .clicked();
        },
    );
    if removed {
        let line = flightpaths::remove_selected_point(session, flights);
        session.status = line;
    }
}

/// Where the edits reach the server, as one line and a button.
fn server_footer(ui: &mut egui::Ui, server_panel: &mut crate::ui::shell::popover::Popover) {
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Server restart required after Apply").size(theme::SMALL).color(theme::INK_FAINT))
            .on_hover_text(
                "A node is a row of taxi_nodes. Paths and their points are TaxiPath.dbc and \
                 TaxiPathNode.dbc, which the server reads from DataDir\\5875\\dbc. Apply on the \
                 Server panel writes both, and so does a save with Apply on save on. vmangos \
                 reads all three at startup only: restart the server after applying.",
            );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("Server\u{2026}")
                .on_hover_text(
                    "Open the Server panel. Apply in its Client tables block writes the nodes as \
                     taxi_nodes rows and copies TaxiPath.dbc and TaxiPathNode.dbc into \
                     DataDir\\5875\\dbc; Restore copies the server's original files back.",
                )
                .clicked()
            {
                server_panel.show();
            }
        });
    });
}

/// What a click in the world does, as the toolbar draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Select,
    NewNode,
    Connect,
    AddPoints,
}

impl Mode {
    fn of(armed: Armed) -> Mode {
        match armed {
            Armed::Nothing => Mode::Select,
            Armed::NewNode => Mode::NewNode,
            Armed::Connect { .. } | Armed::Draw { .. } => Mode::Connect,
            Armed::AddPoints => Mode::AddPoints,
        }
    }
}

/// The toolbar over the top of the viewport while the flight path tool is
/// chosen: the four modes, the armed mode's settings and what the next click
/// does, and whether the transport paths are drawn. It sits to the right of
/// `corner`, the Select and Measure buttons, in the same row. Answers its
/// rectangle, which the shell adds to the areas a press over the world does
/// not reach.
pub(crate) fn strip(
    ctx: &egui::Context,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    flights: &mut Flightpaths,
) -> Option<egui::Rect> {
    theme::toolbar(ctx, "flightpaths-strip", viewport, corner, |ui| {
        modes(ui, flights);
        armed_settings(ui, flights);
    })
}

/// The four modes, and the two switches that decide which paths are drawn.
fn modes(ui: &mut egui::Ui, flights: &mut Flightpaths) {
    let mode = Mode::of(flights.armed);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let button = |ui: &mut egui::Ui, on: bool, enabled: bool, text: &str| {
            ui.add_enabled(enabled, egui::Button::selectable(on, text).min_size(egui::vec2(0.0, 26.0)))
        };
        if button(ui, mode == Mode::Select, true, "Select")
            .on_hover_text("Click a node, a path or a point to select it. Drag to move; Ctrl + drag moves vertically.")
            .clicked()
        {
            flights.disarm();
        }
        if button(ui, mode == Mode::NewNode, true, "New Node")
            .on_hover_text(
                "Click the ground to create a node there, available to both factions with mounts \
                 2224 (Horde) and 541 (Alliance). The server uses a node only when a flight master \
                 stands near it.",
            )
            .clicked()
        {
            flights.disarm();
            flights.armed = Armed::NewNode;
        }
        let connect = button(ui, mode == Mode::Connect, flights.node.is_some() || mode == Mode::Connect, "Connect")
            .on_hover_text(
                "Click another node to create a path from the selected node to it, with points every \
                 120 yards at the clearance above the ground, or drawn by hand.",
            )
            .on_disabled_hover_text("Select the start node first.");
        if connect.clicked() && mode != Mode::Connect {
            if let Some(from) = flights.node {
                flights.disarm();
                flights.arm_connect(from);
            }
        }
        if button(ui, mode == Mode::AddPoints, flights.path.is_some() || mode == Mode::AddPoints, "Add Points")
            .on_hover_text(
                "Click the ground to insert a point into the selected path, in the leg nearest the \
                 click, at the clearance above the ground or the leg's own height, whichever is higher.",
            )
            .on_disabled_hover_text("Select a path first.")
            .clicked()
        {
            flights.disarm();
            flights.armed = Armed::AddPoints;
        }
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
        let transports = flights.transports;
        if button(ui, transports, true, "Show Transports")
            .on_hover_text(
                "Also draw the boat and zeppelin paths. They are paths whose end nodes have no \
                 mount for either faction, and they share TaxiPath.dbc with the flights.",
            )
            .clicked()
        {
            flights.transports = !transports;
        }
        let unselected = flights.unselected;
        if button(ui, unselected, true, "Show Unselected")
            .on_hover_text(
                "Draw every path. Off, only the selected path and the selected node's paths \
                 are drawn and can be clicked. Also on the right-click menu.",
            )
            .clicked()
        {
            flights.unselected = !unselected;
        }
    });
}

/// Under an armed mode: its settings and what the next click does.
fn armed_settings(ui: &mut egui::Ui, flights: &mut Flightpaths) {
    let hint = match flights.armed {
        Armed::Nothing => return,
        Armed::NewNode => "Click the ground to place a node.".to_string(),
        Armed::AddPoints => {
            settings_row(ui, |ui| clearance(ui, flights));
            "Click the ground near the selected path to insert a point.".to_string()
        }
        Armed::Connect { from } => {
            settings_row(ui, |ui| {
                clearance(ui, flights);
                ui.separator();
                if ui
                    .checkbox(&mut flights.draw_by_hand, "Draw by Hand")
                    .on_hover_text("Place the path's points one click at a time before choosing the end node.")
                    .changed()
                {
                    flights.arm_connect(from);
                }
                return_path(ui, flights);
            });
            format!("Click the end node for the path from {}.", short(&flights.name(from), 24))
        }
        Armed::Draw { from } => {
            let placed = flights.draft.as_ref().map_or(0, |draft| draft.points.len());
            settings_row(ui, |ui| {
                if let Some(draft) = flights.draft.as_mut() {
                    ui.label(egui::RichText::new("Height").color(theme::INK_DIM));
                    ui.add(egui::DragValue::new(&mut draft.height).speed(1.0).suffix(" yd"))
                        .on_hover_text("The height the next point is placed at. Ctrl + drag in the world sets it too.");
                    ui.separator();
                }
                if ui
                    .checkbox(&mut flights.draw_by_hand, "Draw by Hand")
                    .on_hover_text("Off, the path is created with points every 120 yards when the end node is clicked.")
                    .changed()
                {
                    flights.arm_connect(from);
                }
                return_path(ui, flights);
            });
            format!(
                "From {}: {placed} point(s) placed. Click: place a point \u{b7} Ctrl + drag: set height \u{b7} \
                 Backspace: remove the last \u{b7} Click a node: finish",
                short(&flights.name(from), 20)
            )
        }
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(hint).size(theme::SMALL).color(theme::INK_DIM));
        ui.label(egui::RichText::new("Esc: cancel").size(theme::SMALL).color(theme::INK_FAINT));
    });
}

/// One line of an armed mode's settings, spaced from the modes above it.
fn settings_row(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        contents(ui);
    });
}

/// The clearance a new point is put at.
fn clearance(ui: &mut egui::Ui, flights: &mut Flightpaths) {
    ui.label(egui::RichText::new("Clearance").color(theme::INK_DIM));
    ui.add(egui::DragValue::new(&mut flights.clearance).range(0.0..=500.0).speed(1.0).suffix(" yd"))
        .on_hover_text("How far above the ground a new point is put.");
}

/// Whether Connect makes the path back as well.
fn return_path(ui: &mut egui::Ui, flights: &mut Flightpaths) {
    ui.checkbox(&mut flights.with_return, "Return Path").on_hover_text(
        "Also create the path back: the same points in the opposite order. 270 of the 275 standard \
         flights have one, as a row of its own.",
    );
}

/// The flight path tool's inspector panel. The flight paths are client tables
/// whose rows are places, picked in the viewport like a light, so their form
/// is drawn in the inspector too.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, assets, now, server_panel, .. } = subject;
    draw(
        ui,
        Subject {
            session,
            flights: &mut editing.flightpaths,
            assets,
            now,
            server_panel,
            fly: &mut editing.fly,
        },
    );
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
        // Node 9 has this pair: the Alliance gryphon in the column vmangos
        // reads as the Horde's.
        assert_eq!(
            sides(&node([541, 0])),
            "The server offers it to the Horde; the client's flight map draws it for the Alliance."
        );
        assert!(sides_differ(&node([541, 0])).is_some());
        assert!(sides_differ(&node([2224, 541])).is_none());
    }

    /// Every mode the tool can be armed in is drawn as one of the four.
    #[test]
    fn every_armed_state_is_one_of_the_toolbar_modes() {
        assert_eq!(Mode::of(Armed::Nothing), Mode::Select);
        assert_eq!(Mode::of(Armed::NewNode), Mode::NewNode);
        assert_eq!(Mode::of(Armed::Connect { from: 1 }), Mode::Connect);
        assert_eq!(Mode::of(Armed::Draw { from: 1 }), Mode::Connect);
        assert_eq!(Mode::of(Armed::AddPoints), Mode::AddPoints);
    }

    #[test]
    fn a_long_name_is_cut_with_an_ellipsis() {
        assert_eq!(short("Stormwind", 22), "Stormwind");
        assert_eq!(short("Thorium Point, Searing Gorge", 10), "Thorium P\u{2026}");
    }

    /// The column given as 0 takes the width the fixed columns leave, and the
    /// cells sit edge to edge.
    #[test]
    fn a_tables_columns_fill_its_width() {
        let widths = column_widths(300.0, &[0.0, 72.0, 72.0]);
        assert_eq!(widths, vec![156.0, 72.0, 72.0]);
        let row = egui::Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(300.0, 20.0));
        let parts = cells(row, &widths);
        assert_eq!(parts[0].right(), parts[1].left());
        assert_eq!(parts[2].right(), row.right());
    }
}
