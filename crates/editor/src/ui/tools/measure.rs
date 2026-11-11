//! The Measure tool's panel. The state is [`crate::tools::measure`].
//!
//! ```text
//! Between Points      distance, horizontal, rise, slope and facing, then
//!                     Copy Facing and Clear Points
//! First Point         X, Y and Z, the two Copy buttons, and what is at the
//!                     point
//! Second Point        the same, without the tile and area where they are
//!                     the first point's
//! Under the Pointer   the same rows, with no Copy buttons
//! ```
//!
//! The tool has no mode and no switch, so its toolbar over the viewport is
//! the Keys button alone ([`theme::toolbar_keys`] with [`KEYS`]).

use bevy_egui::egui;

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::ui::widgets::theme::{self, property};

/// The tool's mouse and keyboard controls, for the toolbar's Keys button.
pub(crate) const KEYS: [(&str, &str); 3] = [
    ("Left button", "Keep the first point, then the second"),
    ("A third click", "Start over from a new first point"),
    ("Escape", "Clear both points"),
];

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

    theme::heading_about(ui, "Between Points", "Measuring changes nothing in the project.");
    let span = measuring.span();
    match span {
        Some(span) => {
            value_row(ui, "Distance", "", format!("{:.2} yd", span.distance));
            value_row(
                ui,
                "Horizontal",
                "The distance in the x-y plane, ignoring height.",
                format!("{:.2} yd", span.horizontal),
            );
            value_row(ui, "Rise", "", format!("{:+.2} yd", span.rise));
            text_row(
                ui,
                "Slope",
                "The angle of the straight line between the points, not of the \
                 ground between them. A character cannot climb ground steeper \
                 than about 50 degrees.",
                format!("{:+.1}\u{b0}", span.slope),
                slope_colour(span.slope.abs()),
            );
            value_row(
                ui,
                "Facing",
                "The direction from the first point to the second, in radians from \
                 north (+x) toward west (+y). It is the value a creature or game \
                 object row's orientation column needs for a spawn at the first \
                 point to face the second.",
                format!("{:.4}  ({:.1}\u{b0})", span.facing, span.facing.to_degrees()),
            );
        }
        None => theme::note(ui, "Fewer than two points picked"),
    }
    // Copy Facing is under the rows and not in the Facing row: the value and
    // a button do not fit beside the name at the inspector's narrowest.
    match span {
        Some(span) => {
            ui.add_space(2.0);
            ui.columns(2, |columns| {
                let copy = theme::full_button(&columns[0], "Copy Facing");
                if columns[0].add(copy).on_hover_text("Copies the facing in radians.").clicked() {
                    columns[0].ctx().copy_text(format!("{:.4}", span.facing));
                }
                let clear = theme::full_button(&columns[1], "Clear Points");
                if columns[1].add(clear).clicked() {
                    measuring.clear();
                }
            });
        }
        None if measuring.first.is_some() => {
            ui.add_space(2.0);
            if ui.button("Clear Points").clicked() {
                measuring.clear();
            }
        }
        None => {}
    }

    // The second point's tile and area are left out when they are the first
    // point's, which is the usual case for two points a few yards apart.
    let points = [
        ("First Point", measuring.first, None),
        ("Second Point", measuring.second, measuring.first),
    ];
    for (name, probe, beside) in points {
        theme::heading(ui, name);
        match probe {
            Some(probe) => probe_rows(ui, &probe, session, tables, true, beside.as_ref()),
            None => theme::note(ui, "Not picked"),
        }
    }

    theme::heading(ui, "Under the Pointer");
    match cursor.surface {
        Some(at) => {
            let probe = measure::probe(session, at, cursor.ground);
            probe_rows(ui, &probe, session, tables, false, None);
        }
        None => theme::note(ui, "Pointer is not over an open tile"),
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

/// A short value of the report as a [`property`] row, drawn with
/// [`theme::value`]. `about`, when not empty, is shown when the name or the
/// value is hovered.
fn value_row(ui: &mut egui::Ui, name: &str, about: &str, text: String) {
    property(ui, name, about, |ui| {
        let value = theme::value(ui, text);
        if !about.is_empty() {
            value.on_hover_text(about);
        }
    });
}

/// A value of the report as a [`property`] row, in `colour`, wrapped in the
/// room beside the name.
///
/// For the values that are words: a surface, a tile's file name, an area's
/// name, a liquid's flags. They can be wider than the row at the inspector's
/// narrowest, where [`theme::value`] would cut them. `about` is shown as
/// [`value_row`]'s is.
fn text_row(ui: &mut egui::Ui, name: &str, about: &str, text: String, colour: egui::Color32) {
    property(ui, name, about, |ui| {
        let value = ui.add(egui::Label::new(egui::RichText::new(text).color(colour)).wrap());
        if !about.is_empty() {
            value.on_hover_text(about);
        }
    });
}

/// `text` with its first letter in upper case, for a value the tool's state
/// words in lower case.
fn capitalised(text: &str) -> String {
    let mut letters = text.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// The rows describing one point. `kept` adds the two Copy buttons, which a
/// point under a moving pointer cannot be clicked for. With `beside`, the tile
/// and area rows are left out where they are the same as that point's.
///
/// The position is a row for each of X, Y and Z: the three numbers on one
/// line do not fit beside a name at the inspector's narrowest.
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
    for (name, value) in [("X", at.x), ("Y", at.y), ("Z", at.z)] {
        value_row(ui, name, "", format!("{value:.2}"));
    }
    if kept {
        // The two labels are too long for equal columns at the inspector's
        // narrowest. The row wraps there.
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Copy Position")
                .on_hover_text("x y z, as three numbers separated by spaces.")
                .clicked()
            {
                ui.ctx().copy_text(measure::position_text(at));
            }
            if ui
                .button("Copy .go Command")
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
    let surface = match (probe.on, probe.ground) {
        (On::Ground, _) => "Ground".to_string(),
        (On::Model, Some(ground)) => format!("Model, {:.2} yd above the ground", at.z - ground),
        (On::Model, None) => "Model, no ground below".to_string(),
    };
    text_row(ui, "Surface", "", surface, theme::INK);
    let ground = match (probe.ground, probe.hole, probe.chunk) {
        (Some(z), _, _) => format!("{z:.2} z"),
        (None, true, _) => "Hole in the ground".to_string(),
        (None, false, Some(_)) => "None".to_string(),
        (None, false, None) => "No open tile".to_string(),
    };
    value_row(ui, "Ground", "", ground);
    if beside.is_none_or(|other| other.tile != probe.tile) {
        text_row(
            ui,
            "Tile",
            "",
            format!("{}, {}  {}_{}_{}", probe.tile.0, probe.tile.1, session.map, probe.tile.0, probe.tile.1),
            theme::INK,
        );
    }
    if let Some(chunk) = probe.chunk {
        let (x, y) = vale_assets::world::adt::chunk_xy(chunk);
        value_row(
            ui,
            "Chunk",
            "The MCNK index, then its column and row in the tile's 16x16 grid.",
            format!("{chunk}  ({x}, {y})"),
        );
    }
    if let Some(id) = probe.area.filter(|&id| beside.is_none_or(|other| other.area != Some(id))) {
        let table = tables.and_then(|tables| tables.areas());
        let zone = table.and_then(|table| table.zone_of(id)).filter(|zone| zone.id != id);
        let text = match (id, table.and_then(|table| table.get(id)), zone) {
            (0, _, _) => "None (0)".to_string(),
            (id, Some(area), Some(zone)) => format!("{} ({id}), in {}", area.name, zone.name),
            (id, Some(area), None) => format!("{} ({id})", area.name),
            (id, None, _) => format!("Area {id}"),
        };
        text_row(ui, "Area", "", text, theme::INK);
    }
    if let Some(slope) = probe.slope {
        text_row(
            ui,
            "Slope",
            "The terrain's slope at this point. A character cannot climb ground \
             steeper than about 50 degrees, and the server's navmesh marks such \
             ground as a steep slope.",
            format!("{slope:.1}\u{b0}"),
            slope_colour(slope),
        );
    }
    // The liquid is three rows: its type and level, its depth where the
    // ground under it is known, and its flags. As one value they are wider
    // than the room beside the name at the inspector's default width.
    if let Some((kind, level, flags)) = probe.water {
        let name = capitalised(measure::liquid_name(kind));
        text_row(ui, "Water", "", format!("{name} at {level:.2} z"), theme::INK);
        if let Some(z) = probe.ground {
            value_row(ui, "Water Depth", "", format!("{:.2} yd", level - z));
        }
        let flags = capitalised(&crate::tools::ground::water::flag_words(flags));
        text_row(ui, "Water Flags", "", flags, theme::INK);
    }
    if probe.impassable {
        text_row(
            ui,
            "Flags",
            "MCNK flag 0x02. vmangos reads it when it builds mmaps.",
            "Impassable chunk".to_string(),
            theme::WARN,
        );
    }
}

/// The Measure tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, cursor, tables, .. } = subject;
    measure(ui, &mut editing.measuring, session, cursor, tables);
}
