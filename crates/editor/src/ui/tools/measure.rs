//! The Measure tool's panel. The state is [`crate::tools::measure`].

use bevy_egui::egui;

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::ui::widgets::theme;

/// The measuring tool: the span between the two kept points, the points,
/// and what is under the pointer. See [`crate::tools::measure`].
///
/// The live pointer rows are last because their height changes as the
/// pointer moves on and off the world. Placed above the other rows, they
/// would move everything below them each time the height changed.
pub(super) fn measure(
    ui: &mut egui::Ui,
    measuring: &mut crate::tools::measure::Measuring,
    session: &mut EditSession,
    cursor: &Cursor,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
) {
    use crate::tools::measure;

    theme::note(
        ui,
        "Click a first point, then a second to measure between them. A third \
         click starts over; Escape clears. Changes nothing.",
    );

    ui.add_space(4.0);
    theme::heading(ui, "Between points");
    match measuring.span() {
        Some(span) => {
            theme::row(ui, "distance", |ui| {
                ui.label(theme::number(format!("{:.2} yd", span.distance)));
            });
            theme::row(ui, "horizontal", |ui| {
                ui.label(theme::number(format!("{:.2} yd", span.horizontal)))
                    .on_hover_text("The distance in the x-y plane, ignoring height.");
            });
            theme::row(ui, "rise", |ui| {
                ui.label(theme::number(format!("{:+.2} yd", span.rise)));
            });
            theme::row(ui, "slope", |ui| {
                ui.label(theme::number(format!("{:+.1}°", span.slope)).color(slope_colour(span.slope.abs())))
                    .on_hover_text(
                        "The angle of the straight line between the points, not of the \
                         ground between them. A character cannot climb ground steeper \
                         than about 50 degrees.",
                    );
            });
            theme::row(ui, "facing", |ui| {
                ui.label(theme::number(format!(
                    "{:.4}  ({:.1}°)",
                    span.facing,
                    span.facing.to_degrees()
                )))
                .on_hover_text(
                    "The direction from the first point to the second, in radians from \
                     north (+x) toward west (+y). It is the value a creature or game \
                     object row's orientation column needs for a spawn at the first \
                     point to face the second.",
                );
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(format!("{:.4}", span.facing));
                }
            });
        }
        None => theme::note(ui, "pick two points to measure between them"),
    }
    if measuring.first.is_some() {
        ui.add_space(2.0);
        if ui.button("Clear points").clicked() {
            measuring.clear();
        }
    }

    // The second point's tile and area are left out when they are the first
    // point's, which is the usual case for two points a few yards apart.
    let points = [
        ("First point", measuring.first, None),
        ("Second point", measuring.second, measuring.first),
    ];
    for (name, probe, beside) in points {
        ui.add_space(6.0);
        theme::heading(ui, name);
        match probe {
            Some(probe) => probe_rows(ui, &probe, session, tables, true, beside.as_ref()),
            None => theme::note(ui, "not picked"),
        }
    }

    ui.add_space(6.0);
    theme::heading(ui, "Under the pointer");
    match cursor.surface {
        Some(at) => {
            let probe = measure::probe(session, at, cursor.ground);
            probe_rows(ui, &probe, session, tables, false, None);
        }
        None => theme::note(ui, "pointer is not over an open tile"),
    }
}

/// The colour a slope in degrees is drawn in: warning past 35, bad past the
/// climbing limit.
fn slope_colour(degrees: f32) -> egui::Color32 {
    match degrees {
        d if d > crate::tools::measure::CLIMB_LIMIT => theme::BAD,
        d if d > 35.0 => theme::WARN,
        _ => theme::INK,
    }
}

/// The rows describing one point. `kept` adds the two Copy buttons, which a
/// point under a moving pointer cannot be clicked for. With `beside`, the tile
/// and area rows are left out where they are the same as that point's.
fn probe_rows(
    ui: &mut egui::Ui,
    probe: &crate::tools::measure::Probe,
    session: &EditSession,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    kept: bool,
    beside: Option<&crate::tools::measure::Probe>,
) {
    use crate::tools::measure::{self, On};

    let at = probe.at;
    theme::row(ui, "position", |ui| {
        ui.label(theme::number(format!("{:.2} {:.2} {:.2}", at.x, at.y, at.z)));
    });
    if kept {
        theme::row(ui, "", |ui| {
            if ui
                .small_button("Copy position")
                .on_hover_text("x y z, as three numbers separated by spaces.")
                .clicked()
            {
                ui.ctx().copy_text(measure::position_text(at));
            }
            if ui
                .small_button("Copy .go command")
                .on_hover_text(
                    ".go xyz x y z map: vmangos' GM command to teleport to this point, \
                     for the playtest's chat line.",
                )
                .clicked()
            {
                ui.ctx().copy_text(measure::go_command(at, session.map_id));
            }
        });
    }
    theme::row(ui, "surface", |ui| {
        let text = match (probe.on, probe.ground) {
            (On::Ground, _) => "ground".to_string(),
            (On::Model, Some(ground)) => format!("model, {:.2} yd above the ground", at.z - ground),
            (On::Model, None) => "model, no ground below".to_string(),
        };
        ui.label(egui::RichText::new(text).color(theme::INK));
    });
    theme::row(ui, "ground", |ui| {
        let text = match (probe.ground, probe.hole, probe.chunk) {
            (Some(z), _, _) => format!("{z:.2} z"),
            (None, true, _) => "a hole in the ground".to_string(),
            (None, false, Some(_)) => "none".to_string(),
            (None, false, None) => "no open tile".to_string(),
        };
        ui.label(theme::number(text));
    });
    if beside.is_none_or(|other| other.tile != probe.tile) {
        theme::row(ui, "tile", |ui| {
            ui.label(theme::number(format!(
                "{}, {}  {}_{}_{}",
                probe.tile.0, probe.tile.1, session.map, probe.tile.0, probe.tile.1
            )));
        });
    }
    if let Some(chunk) = probe.chunk {
        theme::row(ui, "chunk", |ui| {
            ui.label(theme::number(format!("{chunk}  ({}, {})", chunk % 16, chunk / 16)))
                .on_hover_text("The MCNK index, then its column and row in the tile's 16x16 grid.");
        });
    }
    if let Some(id) = probe.area.filter(|&id| beside.is_none_or(|other| other.area != Some(id))) {
        let table = tables.and_then(|tables| tables.areas());
        let zone = table.and_then(|table| table.zone_of(id)).filter(|zone| zone.id != id);
        let text = match (id, table.and_then(|table| table.get(id)), zone) {
            (0, _, _) => "none (0)".to_string(),
            (id, Some(area), Some(zone)) => format!("{} ({id}), in {}", area.name, zone.name),
            (id, Some(area), None) => format!("{} ({id})", area.name),
            (id, None, _) => format!("area {id}"),
        };
        theme::row(ui, "area", |ui| {
            ui.label(egui::RichText::new(text).color(theme::INK));
        });
    }
    if let Some(slope) = probe.slope {
        theme::row(ui, "slope", |ui| {
            ui.label(theme::number(format!("{slope:.1}°")).color(slope_colour(slope)))
                .on_hover_text(
                    "The terrain's slope at this point. A character cannot climb ground \
                     steeper than about 50 degrees, and the server's navmesh marks such \
                     ground as a steep slope.",
                );
        });
    }
    if let Some((kind, level, flags)) = probe.water {
        let flags = crate::tools::ground::water::flag_words(flags);
        let name = measure::liquid_name(kind);
        let text = match probe.ground {
            Some(z) => format!("{name} at {level:.2} z, {:.2} yd deep; {flags}", level - z),
            None => format!("{name} at {level:.2} z; {flags}"),
        };
        theme::row(ui, "water", |ui| {
            ui.label(egui::RichText::new(text).color(theme::INK));
        });
    }
    if probe.impassable {
        theme::row(ui, "flags", |ui| {
            ui.label(egui::RichText::new("impassable chunk").color(theme::WARN))
                .on_hover_text("MCNK flag 0x02. vmangos reads it when it builds mmaps.");
        });
    }
}
