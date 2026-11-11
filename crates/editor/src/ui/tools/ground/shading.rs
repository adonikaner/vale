//! The shading brush's panel. The state is [`crate::tools::ground::shading`].

use bevy_egui::egui;

use crate::tools::ground::terrain::{FALLOFFS, SHAPES};
use crate::ui::widgets::theme::{self, field, property};

/// The brush's mouse and keyboard controls, for the Brush Settings heading's
/// info mark. The tool has no toolbar over the viewport to hold them.
const KEYS: &str = "Left button: paint\n\
                    Ctrl + wheel: radius\n\
                    Shift + wheel: strength\n\
                    Alt + wheel: core\n\
                    1 to 5: mode\n\
                    6 to 0: falloff\n\
                    Shift + 1 to 3: shape";

/// The shading brush: `MCCV`, the light painted onto the ground's vertices.
///
/// Colour and brightness are separate controls because `MCCV`'s midpoint is
/// 1.0 rather than 0.5. White at brightness one is the value that leaves a
/// vertex unchanged, and every other value departs from it. A single 0..1
/// swatch could not express "keep the hue and halve the light", which is the
/// most common shading edit.
///
/// The sections are the height brush's: Brush Settings with the mode, radius,
/// strength, core, falloff and shape as rows, then the section only this
/// brush has, Colour.
///
/// See `crate::tools::ground::shading` for the deviation this brush is part of.
pub(in crate::ui::tools) fn shading(ui: &mut egui::Ui, shading: &mut crate::tools::ground::shading::Shading) {
    use crate::tools::ground::shading;

    theme::heading_about(ui, "Brush Settings", KEYS);
    property(ui, "Mode", "", |ui| {
        theme::choice(ui, "shading-mode", &mut shading.brush.mode, &shading::MODES, |a, b| a == b);
    });
    property(ui, "Radius", "", |ui| {
        field(
            ui,
            egui::DragValue::new(&mut shading.brush.radius)
                .speed(0.5)
                .range(shading::RADIUS)
                .suffix(" yd"),
        );
    });
    property(ui, "Strength", "", |ui| {
        field(
            ui,
            egui::DragValue::new(&mut shading.brush.strength)
                .speed(0.05)
                .range(shading::RATE)
                .suffix("/s"),
        );
    });
    property(ui, "Core", "", |ui| {
        field(
            ui,
            egui::DragValue::new(&mut shading.brush.core)
                .speed(0.01)
                .range(crate::tools::CORE)
                .fixed_decimals(2),
        );
    });
    property(ui, "Falloff", "", |ui| {
        theme::choice(ui, "shading-falloff", &mut shading.brush.falloff, &FALLOFFS, |a, b| a == b);
    });
    property(ui, "Shape", "", |ui| {
        theme::choice(ui, "shading-shape", &mut shading.brush.shape, &SHAPES, |a, b| a == b);
    });

    theme::heading_about(
        ui,
        "Colour",
        "MCCV is a per-vertex colour multiplier; the reference client does not \
         read it. A chunk painted back to neutral drops its MCCV data, so an \
         undone tile is written back byte for byte.",
    );
    // Only `Paint` reads the colour. The other four modes aim at values they
    // decide themselves (white, black, the mean, neutral), so a picker beside
    // them would do nothing. The height brush hides its scale row for the
    // same reason.
    match shading.brush.mode {
        vale_edit::ops::shade::Shade::Paint => {
            property(ui, "Tint", "", |ui| {
                ui.color_edit_button_rgb(&mut shading.picked);
            });
        }
        _ => theme::note(ui, "This mode sets its own colour."),
    }
    property(ui, "Brightness", "", |ui| {
        field(
            ui,
            egui::DragValue::new(&mut shading.brightness)
                .speed(0.01)
                .range(0.0..=vale_edit::ops::shade::SHADE_MAX)
                .fixed_decimals(2)
                .suffix("x"),
        );
    });
    // The combined value in the file's own units, which is what the region
    // stores and what is compared between two chunks.
    let aim = shading.target();
    property(ui, "Target", "1.00 leaves a vertex unchanged.", |ui| {
        theme::value(ui, format!("{:.2}, {:.2}, {:.2}", aim[0], aim[1], aim[2]));
    });
    if ui.button("Reset to Neutral").clicked() {
        shading.picked = [1.0, 1.0, 1.0];
        shading.brightness = 1.0;
    }
}

/// The shading brush's inspector panel. The drawing tablet's settings are in
/// the Settings window; see `crate::ui::shell::settings`.
pub(crate) fn inspect(ui: &mut egui::Ui, _: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    shading(ui, &mut editing.shading);
}
