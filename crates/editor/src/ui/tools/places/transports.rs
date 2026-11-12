//! The Transports half of the flight path tool: its toolbar over the
//! viewport and its inspector.
//!
//! ```text
//! toolbar     [Flights | Transports]  [Select | Add Points | New Transport]
//!             Show Unselected  Keys; while a leg is being drawn, its height
//!             and Finish and Cancel, and a line naming the next click
//! inspector   every transport and every route with none, as a list; then
//!             the selected one: the ship (name, hull, speed, acceleration,
//!             how long a round takes and whether the server has that
//!             number), the route as one leg per map with a Go To for each,
//!             the stops, a clock that moves the ship's marker, and the
//!             selected point
//! ```
//!
//! The inspector is written around what a person sees in the game. A route
//! is its legs: on each the ship comes into view, sails through the points,
//! waits at the stops and leaves, and it is then moved to the start of the
//! next leg. The two nodes a `TaxiPath` row needs are made with a new
//! transport and are not shown, since a ship goes to neither.
//!
//! Go To on a leg, a stop or the ship opens the map it is on when that is
//! not the open one, with the route still selected, so a route over two maps
//! is edited without choosing each map by hand.
//!
//! Every edit goes through `crate::tools::places::transports`, where a change
//! to the route, the speed or the acceleration also writes the period.

use bevy_egui::egui;

use super::flightpaths::{
    above_ground, cell_text, cells, column_widths, leading_number, remove_button, row_fill, server_footer, settings_row,
    table_header, title, value_in, value_then, warn_line, TABLE_ROW,
};
use crate::preview::portraits::Portraits;
use crate::session::EditSession;
use crate::subjects::displays;
use crate::tables;
use crate::tools::places::flightpaths::{self, Armed, Flightpaths};
use crate::tools::places::transports::{self, Drawing, Go, Sailing, Ship, Transports, HULLS};
use crate::ui::widgets::reference;
use crate::ui::widgets::theme::{self, bar_field, bar_group, bar_keys, bar_segment, bar_setting, bar_switch, field, property};
use vale_client::assets::GameAssets;
use vale_assets::tables::taxi::path_node_fields as wf;
use vale_edit::dbc::taxi;

/// A time into a round as minutes and seconds.
pub(crate) fn clock(ms: f64) -> String {
    let seconds = (ms / 1000.0).floor().max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A whole number with a comma between each three digits.
pub(crate) fn grouped(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A map's name, or its id when `Map.dbc` has none.
fn map_name(session: &EditSession, map: u32) -> String {
    match session.maps.iter().find(|(id, _)| *id == map) {
        Some((_, name)) => name.clone(),
        None => format!("map {map}"),
    }
}

/// What a route is called: its transport's name, or its path id when it has
/// no transport.
fn route_name(transports: &Transports, path: u32) -> String {
    match transports.of_path(path).next() {
        Some(ship) if !ship.template.name.is_empty() => ship.template.name.clone(),
        Some(ship) => format!("Transport {}", ship.template.entry),
        None => format!("Route {path}, no transport"),
    }
}

/// The toolbar while the transports half is shown:
///
/// ```text
/// [Flights | Transports]  [Select | Add Points | New Transport]  Show Unselected  Keys
/// Height  Finish  Cancel
/// Click each place the ship sails through, in order.
/// ```
///
/// The second and third lines are drawn while a leg is being drawn.
pub(super) fn toolbar(ui: &mut egui::Ui, flights: &mut Flightpaths, transports: &mut Transports) {
    let drawing = transports.drawing.as_ref().map(|draft| draft.what);
    let adding = flights.armed == Armed::AddPoints;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        bar_group(ui, |ui| {
            if bar_segment(ui, false, "Flights")
                .on_hover_text("Show the flight nodes and the flight paths between them, in place of the transports.")
                .clicked()
            {
                transports.drawing = None;
                flights.set_mode(false);
            }
            let _ = bar_segment(ui, true, "Transports").on_hover_text("The boats and zeppelins and the routes they sail.");
        });
        bar_group(ui, |ui| {
            if bar_segment(ui, drawing.is_none() && !adding, "Select")
                .on_hover_text("Click a route to select its transport, or a point of the selected route. Drag a point to move it; Ctrl + drag moves it vertically.")
                .clicked()
            {
                transports.drawing = None;
                flights.disarm();
            }
            if ui
                .add_enabled_ui(flights.path.is_some() || adding, |ui| bar_segment(ui, adding, "Add Points"))
                .inner
                .on_hover_text("Click near the selected route to insert a point into the leg nearest the click, at that leg's height.")
                .on_disabled_hover_text("Select a route that has a leg on this map first.")
                .clicked()
            {
                transports.drawing = None;
                flights.disarm();
                flights.armed = Armed::AddPoints;
            }
            if bar_segment(ui, drawing == Some(Drawing::New), "New Transport")
                .on_hover_text(
                    "Draw a new transport's route on this map: click each place the ship sails through, in \
                     order, then Finish. A leg on another map is added afterwards from the transport's card.",
                )
                .clicked()
                && drawing != Some(Drawing::New)
            {
                flights.disarm();
                transports.start_new();
            }
        });
        if bar_switch(ui, flights.unselected, "Show Unselected")
            .on_hover_text("Draw every route. Off, only the selected route is drawn and can be clicked.")
            .clicked()
        {
            flights.unselected = !flights.unselected;
        }
        bar_keys(ui, match (drawing.is_some(), adding) {
            (true, _) => &DRAW_KEYS,
            (false, true) => &ADD_KEYS,
            (false, false) => &SELECT_KEYS,
        });
    });
    if adding {
        theme::note(ui, "Click near the selected route to insert a point.");
    }
    let Some(draft) = transports.drawing.as_mut() else { return };
    let placed = draft.points.len();
    let mut finish = false;
    let mut cancel = false;
    settings_row(ui, |ui| {
        bar_setting(ui, "Height", |ui| {
            let mut height = draft.height.unwrap_or(0.0);
            let known = draft.height.is_some();
            let shown = bar_field(
                ui,
                egui::DragValue::new(&mut height).speed(1.0).custom_formatter(move |z, _| match known {
                    true => format!("{z:.0} yd"),
                    false => "First click".to_string(),
                }),
            )
            .on_hover_text(
                "The height every point of the leg is placed at. A ship's is 0, the sea's level; a zeppelin's \
                 starts 60 yards above the first click.",
            );
            if shown.changed() {
                draft.height = Some(height);
            }
        });
        finish = ui
            .add_enabled(placed >= 2, egui::Button::new("Finish"))
            .on_hover_text("Make the leg from the points placed. Enter does the same.")
            .on_disabled_hover_text("Place at least two points.")
            .clicked();
        cancel = ui.button("Cancel").on_hover_text("Discard the points placed. Escape does the same.").clicked();
    });
    let what = match draft.what {
        Drawing::New => "New transport".to_string(),
        Drawing::Leg { path } => format!("New leg of {}", route_name(transports, path)),
    };
    let draft_points = match placed {
        1 => "1 point placed".to_string(),
        n => format!("{n} points placed"),
    };
    theme::note(
        ui,
        format!("{what}: {draft_points}. Click each place the ship sails through, in order. Shift + click places a stop, where it waits."),
    );
    if finish {
        transports.finish = true;
    }
    if cancel {
        transports.drawing = None;
    }
}

/// The controls with nothing armed.
const SELECT_KEYS: [(&str, &str); 6] = [
    ("Left button", "Select a route, or a point of the selected route"),
    ("Drag", "Move the point"),
    ("Ctrl + drag", "Move it vertically"),
    ("Delete", "Remove the selected point"),
    ("F", "Move the camera to the selection"),
    ("Esc", "Deselect the point, then the route"),
];

/// The controls while Add Points is armed.
const ADD_KEYS: [(&str, &str); 2] = [("Left button", "Insert a point into the selected route"), ("Esc", "Cancel")];

/// The controls while a leg is being drawn.
const DRAW_KEYS: [(&str, &str); 5] = [
    ("Left button", "Place a point the ship sails through"),
    ("Shift + left button", "Place a stop, where the ship waits"),
    ("Backspace", "Remove the last point"),
    ("Enter", "Finish the leg"),
    ("Esc", "Discard it"),
];

/// The inspector while the transports half is shown.
#[allow(clippy::too_many_arguments)]
pub(super) fn panel(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    transports: &mut Transports,
    fly: &mut bevy::prelude::Messages<crate::camera::FlyTo>,
    server_panel: &mut crate::ui::shell::popover::Popover,
    assets: &GameAssets,
    portraits: &mut Portraits,
    now: f64,
) {
    egui::ScrollArea::vertical()
        .id_salt("transports")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if let Some(trouble) = transports.trouble() {
                warn_line(ui, "World database not read").on_hover_text(format!(
                    "{trouble}\n\nThe transports the database holds are not listed, and a round's length cannot \
                     be compared with the server's. The transports this project creates are listed."
                ));
            }
            if let Some(draft) = transports.drawing.as_ref().map(|draft| draft.what) {
                drawing_card(ui, transports, portraits, assets, draft);
                return;
            }
            list(ui, session, flights, transports);
            ui.add_space(8.0);
            let Some(route) = transports.route.and_then(|path| transports.sailing(path)).cloned() else {
                theme::note(ui, "Select a transport in the list or in the viewport, or choose New Transport.");
                return;
            };
            card(ui, session, flights, transports, fly, portraits, assets, &route, now);
            server_footer(ui, server_panel);
            ui.add_space(12.0);
        });
    // The model picker a hull is chosen in. A choice for a ship is written
    // to its template by the picker; one for the New Transport form is kept
    // for it and taken here.
    crate::ui::subjects::displays::window(ui.ctx(), &mut transports.hull_pick, session, assets, portraits, now);
    if let Some(display) = displays::take_kept(&mut transports.hull_pick) {
        transports.set_new_hull(display);
    }
}

/// Every transport and every route with none, with the button that starts a
/// new one. A click selects.
fn list(ui: &mut egui::Ui, session: &EditSession, flights: &mut Flightpaths, transports: &mut Transports) {
    theme::heading_about(
        ui,
        &format!("Transports ({})", transports.ships.len()),
        "Every boat and zeppelin, on every map, with how long one round of its route takes. A faint row has \
         no leg on the open map. Click a row to select it; Go To on its card opens its map.",
    );
    let mut chosen: Option<u32> = None;
    let widths = [0.0, 52.0];
    // The transports first, then the routes with none.
    let (sailed, bare): (Vec<&Sailing>, Vec<&Sailing>) =
        transports.routes.iter().partition(|route| transports.of_path(route.path).next().is_some());
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for route in sailed.into_iter().chain(bare) {
            let (row, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), TABLE_ROW), egui::Sense::click());
            let here = route.legs.iter().any(|leg| leg.map == session.map_id);
            row_fill(ui.painter(), row, transports.route == Some(route.path), response.hovered());
            let parts = cells(row, &column_widths(row.width(), &widths));
            let ship = transports.of_path(route.path).next();
            let troubled = ship.is_some_and(|ship| !ship.in_step() || !ship.findings.is_empty());
            let ink = match (ship.is_some(), here) {
                (false, _) => theme::INK_FAINT,
                (true, true) => theme::INK,
                (true, false) => theme::INK_DIM,
            };
            cell_text(ui.painter(), parts[0], &route_name(transports, route.path), ink, false);
            let round = match ship.and_then(Ship::computed) {
                Some(period) => clock(f64::from(period)),
                None => String::new(),
            };
            let colour = match troubled {
                true => theme::WARN,
                false => theme::INK_DIM,
            };
            cell_text(ui.painter(), parts[1], &round, colour, true);
            let maps: Vec<String> = route.legs.iter().map(|leg| map_name(session, leg.map)).collect();
            let about = match maps.is_empty() {
                true => format!("Path {}. It has no points.", route.path),
                false => format!("Path {}, on {}.", route.path, maps.join(", then ")),
            };
            if response.on_hover_text(about).clicked() {
                chosen = Some(route.path);
            }
        }
    });
    if let Some(path) = chosen {
        transports.select(flights, path);
    }
    ui.add_space(4.0);
    let button = theme::full_button(ui, "New Transport");
    if ui
        .add(button)
        .on_hover_text("Draw a new transport's route on this map. Also on the toolbar.")
        .clicked()
    {
        flights.disarm();
        transports.start_new();
    }
}

/// The inspector while a leg is being drawn: for a new transport, what it
/// will be; then how a leg is drawn.
fn drawing_card(ui: &mut egui::Ui, transports: &mut Transports, portraits: &mut Portraits, assets: &GameAssets, what: Drawing) {
    match what {
        Drawing::New => {
            theme::heading(ui, "New Transport");
            ship_form(ui, transports, portraits, assets);
        }
        Drawing::Leg { path } => {
            theme::heading(ui, "New Leg");
            property(ui, "Transport", "", |ui| {
                theme::value(ui, route_name(transports, path));
            });
        }
    }
    theme::heading(ui, "Drawing the Leg");
    for line in [
        "1. Click each place the ship sails through on this map, in order: where it comes into view, its way to the dock, and its way out again.",
        "2. Hold Shift on the click at a dock. The ship waits there for 60 seconds; the wait can be changed afterwards.",
        "3. Press Finish on the toolbar, or Enter.",
    ] {
        ui.add(egui::Label::new(egui::RichText::new(line).color(theme::INK_DIM)).wrap());
    }
    ui.add_space(4.0);
    theme::note(
        ui,
        "At the end of a leg the ship is moved to the start of the next, and from the last leg to the \
         first. Players lose sight of it first when a leg starts and ends away from the dock.",
    );
}

/// The fields of a new transport: its name, hull, speed and acceleration.
fn ship_form(ui: &mut egui::Ui, transports: &mut Transports, portraits: &mut Portraits, assets: &GameAssets) {
    property(ui, "Name", "The name the game shows for the ship.", |ui| {
        theme::text_field(ui, &mut transports.new_ship.name);
    });
    let display = transports.new_ship.display;
    hull_row(ui, transports, portraits, assets, display, None);
    let new = &mut transports.new_ship;
    property(ui, "Speed", SPEED_ABOUT, |ui| {
        field(ui, egui::DragValue::new(&mut new.speed).range(1..=200).suffix(" yd/s"));
    });
    property(ui, "Acceleration", ACCEL_ABOUT, |ui| {
        field(ui, egui::DragValue::new(&mut new.accel).range(1..=50).suffix(" yd/s\u{b2}"));
    });
}

const HULL_ABOUT: &str = "The model the ship is drawn as: a GameObjectDisplayInfo.dbc row. 3015 is the standard ship and 3031 the zeppelin.";
const SPEED_ABOUT: &str = "Yards a second once under way. Eight of the nine standard transports use 30.";
const ACCEL_ABOUT: &str = "Yards a second gained each second leaving a stop, and lost coming into one. The standard transports use 1.";

/// What a hull is called in its row: Ship or Zeppelin for the two standard
/// hulls, else the model's file name, with the display id after it.
fn hull_words(display: u32, model: Option<&str>) -> String {
    if display == 0 {
        return "None".to_string();
    }
    if let Some((_, name)) = HULLS.iter().find(|(known, _)| *known == display) {
        return format!("{name} ({display})");
    }
    match model.and_then(|path| path.rsplit(['\\', '/']).next()) {
        Some(file) => format!("{file} ({display})"),
        None => format!("Display {display}, no model"),
    }
}

/// A hull's row: the model's picture, what it is called, and the button
/// that opens the model picker. For a `ship` the picker writes the choice to
/// its template's `displayId`; with none it is the New Transport form's
/// hull, kept for the form.
fn hull_row(
    ui: &mut egui::Ui,
    transports: &mut Transports,
    portraits: &mut Portraits,
    assets: &GameAssets,
    display: u32,
    ship: Option<&Ship>,
) {
    let model = transports.hull_model(assets, display);
    property(ui, "Hull", HULL_ABOUT, |ui| {
        let size = ui.spacing().interact_size.y + 4.0;
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
        match model.as_deref() {
            Some(path) => crate::preview::portraits::paint(ui, portraits, path, rect),
            None if display != 0 => {
                ui.painter().rect_filled(rect, 3.0, theme::DEAD);
            }
            None => {}
        }
        let chosen = value_then(ui, hull_words(display, model.as_deref()), theme::INK, |ui| {
            reference::choose_button(
                ui,
                "Choose the model from pictures of every GameObjectDisplayInfo.dbc row that has one. Search \
                 \"transport\" for the hulls the game ships.",
            )
        });
        if chosen {
            transports.pick_hull(ship, display);
        }
    });
}

/// The selected route: its ship, its legs, its stops, the clock, the
/// selected point, and the commands that remove it.
#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    transports: &mut Transports,
    fly: &mut bevy::prelude::Messages<crate::camera::FlyTo>,
    portraits: &mut Portraits,
    assets: &GameAssets,
    route: &Sailing,
    now: f64,
) {
    let ships: Vec<Ship> = transports.of_path(route.path).cloned().collect();
    title(ui, &route_name(transports, route.path));
    let here = route.legs.iter().any(|leg| leg.map == session.map_id);
    if !here && !route.legs.is_empty() {
        let first = &route.legs[0];
        let mut go = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("No leg of this route is on {}.", map_name(session, session.map_id))).color(theme::WARN));
            go = ui
                .small_button(format!("Go To {}", map_name(session, first.map)))
                .on_hover_text("Open that map on the route's first leg. Unsaved tiles of this map are saved first.")
                .clicked();
        });
        if go {
            transports.travel = Some(Go { map: first.map, at: Some(first.at.into()) });
        }
    }

    theme::heading_about(
        ui,
        "Ship",
        "The transport itself: a gameobject_template row of type 15 that names this route. The server makes \
         a ship of every such row when it starts; there is nothing to place in the world.",
    );
    match ships.first() {
        None => {
            theme::note(ui, "This route has no transport.");
            ship_form(ui, transports, portraits, assets);
            ui.add_space(4.0);
            let button = theme::full_button(ui, "Create Transport");
            if ui.add(button).on_hover_text("Make a transport that sails this route. One undo entry.").clicked() {
                session.status = match transports::create(session, transports, route.path, now) {
                    Ok(entry) => format!("Transport {entry} created on path {}", route.path),
                    Err(why) => why,
                };
            }
        }
        Some(_) => {
            for ship in &ships {
                ship_card(ui, session, transports, portraits, assets, ship, ships.len() > 1, now);
            }
        }
    }

    legs(ui, session, flights, transports, route);
    if let Some(ship) = ships.first() {
        stops(ui, session, flights, transports, ship);
        preview(ui, session, transports, ship);
    }
    point(ui, session, flights, fly, route, now);

    ui.add_space(8.0);
    let database = ships.iter().any(|ship| !ship.created());
    let mut removed = false;
    ui.columns(2, |columns| {
        let button = remove_button(&columns[0], "Remove Transport");
        removed = columns[0]
            .add_enabled(!database, button)
            .on_hover_text("Remove the route with its points, and the transport this project created for it. One undo entry.")
            .on_disabled_hover_text(
                "The database holds this transport, and a template row is never deleted. Its route is kept, \
                 since the server stops on a transport whose route has no points.",
            )
            .clicked();
    });
    if removed {
        session.status = transports::remove_route(session, flights, transports, route.path);
    }
}

/// One transport's fields, how long its round takes and whether the server
/// has that number, and what is wrong with its route.
#[allow(clippy::too_many_arguments)]
fn ship_card(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    transports: &mut Transports,
    portraits: &mut Portraits,
    assets: &GameAssets,
    ship: &Ship,
    several: bool,
    now: f64,
) {
    let entry = ship.template.entry;
    if several {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(format!("Transport {entry}")).strong().color(theme::INK));
    }
    let mut changed = false;
    let mut name = ship.template.name.clone();
    property(ui, "Name", "The name the game shows for the ship.", |ui| {
        if theme::text_field(ui, &mut name).changed() {
            transports::set_column(session, ship, "name", vale_mangos::sql::text(&name), now);
            changed = true;
        }
    });
    hull_row(ui, transports, portraits, assets, ship.template.display, Some(ship));
    let mut speed = ship.template.speed;
    property(ui, "Speed", SPEED_ABOUT, |ui| {
        if field(ui, egui::DragValue::new(&mut speed).range(1..=200).suffix(" yd/s")).changed() {
            transports::set_column(session, ship, "data1", speed.to_string(), now);
            changed = true;
        }
    });
    let mut accel = ship.template.accel;
    property(ui, "Acceleration", ACCEL_ABOUT, |ui| {
        if field(ui, egui::DragValue::new(&mut accel).range(1..=50).suffix(" yd/s\u{b2}")).changed() {
            transports::set_column(session, ship, "data2", accel.to_string(), now);
            changed = true;
        }
    });
    property(
        ui,
        "Entry",
        "The gameobject_template entry. A transport this project creates is numbered from 2,000,000.",
        |ui| {
            theme::value(ui, match ship.created() {
                true => format!("{entry}, new in this project"),
                false => entry.to_string(),
            });
        },
    );
    if changed {
        transports.stale();
    }

    let mut update = false;
    property(
        ui,
        "Round Trip",
        "How long the ship takes to sail every leg and wait at every stop once, as the game client works it \
         out from the route, the speed and the acceleration.",
        |ui| match ship.computed() {
            Some(period) => {
                theme::value(ui, clock(f64::from(period))).on_hover_text(format!("{} ms", grouped(period)));
            }
            None => {
                value_in(ui, "None", theme::WARN);
            }
        },
    );
    if let Some(period) = ship.computed() {
        property(
            ui,
            "Server",
            "The server is given the round trip's length as a number (transports.period), and the client \
             works it out. Both find the ship from it, so the server's number must be the client's or the ship \
             is in a different place on each. The editor writes it whenever the route, the speed or the \
             acceleration changes.",
            |ui| match (ship.in_step(), ship.period.server()) {
                (true, _) => {
                    theme::value(ui, "Up to date");
                }
                (false, held) => {
                    let words = match held {
                        Some(held) => format!("Has {}", clock(f64::from(held))),
                        None => "Not set".to_string(),
                    };
                    update = value_then(ui, words, theme::WARN, |ui| {
                        ui.small_button("Update")
                            .on_hover_text(format!("Write {} ms as this transport's period. One undo entry.", grouped(period)))
                            .clicked()
                    });
                }
            },
        );
    }
    for finding in &ship.findings {
        ui.add(egui::Label::new(egui::RichText::new(finding).size(theme::SMALL).color(theme::WARN)).wrap());
    }
    if update {
        session.status = transports::update_period(session, transports, entry, now);
    }
}

/// The route as its legs, one row each, with the commands that go to one,
/// add one on another map and remove one.
fn legs(ui: &mut egui::Ui, session: &mut EditSession, flights: &mut Flightpaths, transports: &mut Transports, route: &Sailing) {
    theme::heading_about(
        ui,
        &format!("Route ({} {})", route.legs.len(), if route.legs.len() == 1 { "leg" } else { "legs" }),
        "A route is one leg for each map it visits. On a leg the ship comes into view at the first point, \
         sails through the others, waits at the stops, and leaves at the last. It is then moved to the start \
         of the next leg, and from the last leg back to the first. Click a leg to go to it; its map is \
         opened when it is not the open one.",
    );
    if route.legs.is_empty() {
        theme::note(ui, "The route has no points.");
    }
    let widths = [22.0, 0.0, 58.0, 50.0];
    if !route.legs.is_empty() {
        table_header(ui, &[("#", false), ("Map", false), ("Points", true), ("Stops", true)], &widths);
    }
    let mut go: Option<Go> = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (n, leg) in route.legs.iter().enumerate() {
            let (row, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), TABLE_ROW), egui::Sense::click());
            let here = leg.map == session.map_id;
            row_fill(ui.painter(), row, here, response.hovered());
            let parts = cells(row, &column_widths(row.width(), &widths));
            cell_text(ui.painter(), parts[0], &(n + 1).to_string(), theme::INK_DIM, false);
            cell_text(ui.painter(), parts[1], &map_name(session, leg.map), theme::INK, false);
            cell_text(ui.painter(), parts[2], &leg.points.len().to_string(), theme::INK_DIM, true);
            let colour = match leg.stops {
                0 => theme::INK_FAINT,
                _ => theme::INK_DIM,
            };
            cell_text(ui.painter(), parts[3], &leg.stops.to_string(), colour, true);
            let about = match here {
                true => "Click to move the camera to this leg.".to_string(),
                false => format!("Click to open {} on this leg. Unsaved tiles of this map are saved first.", map_name(session, leg.map)),
            };
            if response.on_hover_text(about).clicked() {
                go = Some(Go { map: leg.map, at: Some(leg.at.into()) });
            }
        }
    });
    if let Some(go) = go {
        transports.travel = Some(go);
    }

    ui.add_space(4.0);
    let last = route.points.last().map(|point| point.map);
    let mut add: Option<u32> = None;
    let mut remove: Option<usize> = None;
    ui.horizontal(|ui| {
        ui.menu_button("Add Leg On\u{2026}", |ui| {
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (id, name) in &session.maps {
                    let enabled = last != Some(*id);
                    let item = ui
                        .add_enabled(enabled, egui::Button::new(format!("{name} ({id})")))
                        .on_disabled_hover_text("The route's last leg is on this map. Use Add Points to lengthen it.");
                    if item.clicked() {
                        add = Some(*id);
                        ui.close();
                    }
                }
            });
        })
        .response
        .on_hover_text(
            "Open a map and draw the route's next leg there: the same clicks as a new transport's. The leg is \
             added after the last one.",
        );
        ui.add_enabled_ui(route.legs.len() > 1, |ui| {
            ui.menu_button("Remove Leg\u{2026}", |ui| {
                for (n, leg) in route.legs.iter().enumerate() {
                    if ui.button(format!("Leg {}, {}", n + 1, map_name(session, leg.map))).clicked() {
                        remove = Some(n);
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Remove one leg and its points. One undo entry.")
            .on_disabled_hover_text("A route keeps one leg.");
        });
    });
    if let Some(map) = add {
        flights.disarm();
        transports.start_leg(route.path);
        if map != session.map_id {
            transports.travel = Some(Go { map, at: None });
        }
    }
    if let Some(leg) = remove {
        session.status = transports::remove_leg(session, flights, transports, route.path, leg);
    }
}

/// The places the ship waits, in the order of the round. A click goes to
/// one.
fn stops(ui: &mut egui::Ui, session: &EditSession, flights: &mut Flightpaths, transports: &mut Transports, ship: &Ship) {
    theme::heading_about(
        ui,
        &format!("Stops ({})", ship.stops.len()),
        "Where the ship waits, with the time into the round it arrives and leaves. To make a point a stop, \
         select it and tick Ship Waits Here. Click a row to go to the stop; its map is opened when it is not \
         the open one.",
    );
    if ship.stops.is_empty() {
        theme::note(ui, "No stops. Select a point of the route and tick Ship Waits Here.");
        return;
    }
    let at = transports.preview.ms.rem_euclid(f64::from(ship.computed().unwrap_or(1).max(1))) as u32;
    let widths = [52.0, 52.0, 52.0, 0.0];
    table_header(ui, &[("Arrive", true), ("Wait", true), ("Leave", true), ("Map", false)], &widths);
    let mut chosen: Option<transports::Stop> = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for stop in &ship.stops {
            let (row, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), TABLE_ROW), egui::Sense::click());
            let waiting = at >= stop.arrive_ms && at < stop.arrive_ms + stop.wait_ms;
            row_fill(ui.painter(), row, waiting, response.hovered());
            let parts = cells(row, &column_widths(row.width(), &widths));
            cell_text(ui.painter(), parts[0], &clock(f64::from(stop.arrive_ms)), theme::INK, true);
            cell_text(ui.painter(), parts[1], &format!("{} s", stop.wait_ms / 1000), theme::INK_DIM, true);
            cell_text(ui.painter(), parts[2], &clock(f64::from(stop.arrive_ms + stop.wait_ms)), theme::INK, true);
            cell_text(ui.painter(), parts[3], &map_name(session, stop.map), theme::INK_DIM, false);
            if response.on_hover_text("Click to go to this stop and set the clock to the ship's arrival.").clicked() {
                chosen = Some(*stop);
            }
        }
    });
    if let Some(stop) = chosen {
        transports.preview.ms = f64::from(stop.arrive_ms);
        transports.travel = Some(Go { map: stop.map, at: Some(stop.at.into()) });
        if let Some(point) = stop.point.filter(|point| flights.point(*point).is_some()) {
            flights.select_point(point);
        }
    }
}

/// The preview clock's rates: how many times faster than the game it runs.
const RATES: [(&str, f32); 3] = [("1\u{d7}", 1.0), ("10\u{d7}", 10.0), ("60\u{d7}", 60.0)];

/// The clock that moves the ship's marker along its route, and where the
/// ship is at its time.
fn preview(ui: &mut egui::Ui, session: &EditSession, transports: &mut Transports, ship: &Ship) {
    let Some(period) = ship.computed() else { return };
    theme::heading_about(
        ui,
        "Preview",
        "One round of the route. The clock moves the ship's marker in the viewport. It is the editor's own \
         clock: it does not show where the ship is on a running server.",
    );
    let preview = &mut transports.preview;
    let mut at = preview.ms.rem_euclid(f64::from(period));
    ui.horizontal(|ui| {
        let word = match preview.playing {
            true => "Pause",
            false => "Play",
        };
        if ui.button(word).on_hover_text("Run or hold the clock.").clicked() {
            preview.playing = !preview.playing;
        }
        theme::choice(ui, "transports-rate", &mut preview.rate, &RATES, |a, b| a == b)
            .on_hover_text("How many times faster than the game the clock runs.");
    });
    let moved = ui
        .scope(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(60.0);
            let slider = egui::Slider::new(&mut at, 0.0..=f64::from(period))
                .custom_formatter(|ms, _| clock(ms))
                .custom_parser(|text| leading_number(text).map(|seconds| seconds * 1000.0));
            ui.add(slider).on_hover_text("Time into the round. Drag to move the ship along its route.").changed()
        })
        .inner;
    if moved {
        preview.ms = at;
    }
    let placed = ship.placement(preview.ms);
    let mut go: Option<Go> = None;
    property(ui, "Ship Is At", "Where the ship is at the clock's time.", |ui| match placed {
        Some(placed) => {
            let words = format!("{}  {:.0}, {:.0}, {:.0}", map_name(session, placed.map), placed.pos[0], placed.pos[1], placed.pos[2]);
            value_then(ui, words, theme::INK, |ui| {
                if ui
                    .small_button("Go To")
                    .on_hover_text("Move the camera to the ship, opening its map when it is not the open one.")
                    .clicked()
                {
                    go = Some(Go { map: placed.map, at: Some(placed.pos.into()) });
                }
            });
        }
        None => {
            theme::value(ui, "None");
        }
    });
    if go.is_some() {
        transports.travel = go;
    }
}

/// The selected point of the route: whether the ship waits there and for how
/// long, where it is, and the commands that step along the route and remove
/// it.
fn point(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    flights: &mut Flightpaths,
    fly: &mut bevy::prelude::Messages<crate::camera::FlyTo>,
    route: &Sailing,
    now: f64,
) {
    let selected = flights.point.and_then(|id| route.points.iter().position(|point| point.id == id));
    theme::heading_about(
        ui,
        "Point",
        "Click a point of the route in the viewport to select it. Drag it to move it; Ctrl + drag moves it \
         vertically. Add Points on the toolbar inserts one.",
    );
    let Some(at_index) = selected else {
        theme::note(ui, "No point is selected.");
        return;
    };
    let chosen = route.points[at_index];
    let id = chosen.id;
    let leg = route.legs.iter().position(|leg| leg.points.contains(&at_index));
    let handle = route.legs.iter().any(|leg| at_index == leg.points.start || at_index + 1 == leg.points.end);
    let mut step: Option<u32> = None;
    property(ui, "Point", "The point's place in the route, counted from 1.", |ui| {
        let words = match leg {
            Some(leg) => format!("{} of {}, leg {}", at_index + 1, route.points.len(), leg + 1),
            None => format!("{} of {}", at_index + 1, route.points.len()),
        };
        value_then(ui, words, theme::INK, |ui| {
            let next = route.points.get(at_index + 1).filter(|next| next.map == chosen.map).map(|next| next.id);
            if ui.add_enabled(next.is_some(), egui::Button::new("\u{203a}").small()).on_hover_text("Next point").clicked() {
                step = next;
            }
            let previous = at_index.checked_sub(1).map(|at| route.points[at]).filter(|previous| previous.map == chosen.map).map(|previous| previous.id);
            if ui.add_enabled(previous.is_some(), egui::Button::new("\u{2039}").small()).on_hover_text("Previous point").clicked() {
                step = previous;
            }
        });
    });
    if let Some(next) = step {
        flights.select_point(next);
        return;
    }

    if handle {
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Curve handle. The first and last point of a leg are not places the ship goes: they set \
                     the direction it arrives from and leaves in. This one moves with the point beside it \
                     until it is dragged by hand.",
                )
                .size(theme::SMALL)
                .color(theme::INK_DIM),
            )
            .wrap(),
        );
    } else {
        let mut waits = chosen.action_flag == 2;
        let mut delay = chosen.delay;
        let mut changed = false;
        property(ui, "Stop", "Whether the ship stops at this point. A dock is a stop.", |ui| {
            changed |= ui.checkbox(&mut waits, "Ship Waits Here").changed();
        });
        property(ui, "Wait", "Seconds the ship waits at the stop. The standard docks wait 60.", |ui| {
            changed |= ui
                .add_enabled_ui(waits, |ui| field(ui, egui::DragValue::new(&mut delay).range(0..=3600).suffix(" s")))
                .inner
                .on_disabled_hover_text("Used only at a stop.")
                .changed();
        });
        if changed {
            let flag = match (waits, chosen.action_flag) {
                (true, _) => 2,
                (false, 2) => 0,
                (false, other) => other,
            };
            let delay = match (waits, chosen.action_flag == 2, delay) {
                (true, false, 0) => transports::DEFAULT_WAIT,
                (true, _, delay) => delay,
                (false, _, _) => 0,
            };
            tables::fields::set_fields(
                session,
                taxi::POINTS,
                chosen.record,
                &[(wf::ACTION_FLAG, flag), (wf::DELAY, delay)],
                "Change route point",
                &format!("TaxiPathNode {id} stop"),
                now,
            );
            flights.stale();
        }
        if chosen.action_flag & 1 != 0 {
            theme::note(ui, "This point carries the teleport flag: the leg ends here, and the next point starts a new one.");
        }
    }

    let mut place = chosen.at;
    if theme::xyz(ui, &mut place) {
        flightpaths::move_point(session, flights, id, bevy::math::Vec3::from(place), now);
        flights.stale();
    }
    above_ground(ui, session, place);

    let mut removed = false;
    ui.add_space(4.0);
    ui.columns(2, |columns| {
        let button = theme::full_button(&columns[0], "Fly To");
        if columns[0].add(button).on_hover_text("Move the camera to the point. F does the same.").clicked() {
            fly.write(crate::camera::FlyTo::at(chosen.at.into()));
        }
        let button = remove_button(&columns[1], "Remove Point");
        removed = columns[1]
            .add_enabled(route.points.len() > 2, button)
            .on_hover_text("Remove this point from the route. Delete does the same.")
            .on_disabled_hover_text("A route needs at least two points.")
            .clicked();
    });
    if removed {
        session.status = flightpaths::remove_selected_point(session, flights);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_reads_as_minutes_and_seconds() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(59_999.0), "0:59");
        assert_eq!(clock(350_818.0), "5:50");
        assert_eq!(clock(1_208_014.0), "20:08");
    }

    #[test]
    fn a_hull_is_called_by_its_kind_or_its_model() {
        assert_eq!(hull_words(3015, None), "Ship (3015)");
        assert_eq!(hull_words(3031, Some("World\\wmo\\transports\\transport_zeppelin\\transport_zeppelin.wmo")), "Zeppelin (3031)");
        assert_eq!(hull_words(6637, Some("World\\wmo\\transports\\BlackCitadel\\BlackCitadel.wmo")), "BlackCitadel.wmo (6637)");
        assert_eq!(hull_words(9, None), "Display 9, no model");
        assert_eq!(hull_words(0, None), "None");
    }

    #[test]
    fn a_number_is_grouped_in_threes() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(350_818), "350,818");
        assert_eq!(grouped(1_208_014), "1,208,014");
    }
}
