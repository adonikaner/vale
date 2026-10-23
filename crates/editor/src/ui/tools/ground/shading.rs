//! The shading brush's panel. The state is [`crate::tools::ground::shading`].

use bevy_egui::egui;

use crate::tools::ground::terrain::{FALLOFFS, SHAPES};
use crate::ui::widgets::theme;

/// The shading brush: `MCCV`, the light painted onto the ground's vertices.
///
/// Colour and brightness are separate controls because `MCCV`'s midpoint is
/// 1.0 rather than 0.5. White at brightness one is the value that leaves a
/// vertex unchanged, and every other value departs from it. A single 0..1
/// swatch could not express "keep the hue and halve the light", which is the
/// most common shading edit.
///
/// See `crate::tools::ground::shading` for the deviation this brush is part of.
pub(in crate::ui::tools) fn shading(ui: &mut egui::Ui, shading: &mut crate::tools::ground::shading::Shading) {
    use crate::tools::ground::shading;

    theme::heading(ui, "Brush");
    theme::segmented(ui, &mut shading.brush.mode, &shading::MODES, |a, b| a == b);

    ui.add_space(4.0);
    theme::heading(ui, "Colour");
    // Only `Paint` reads the colour. The other four modes aim at values they
    // decide themselves (white, black, the mean, neutral), so a picker beside
    // them would do nothing. The height brush hides its scale row for the
    // same reason.
    match shading.brush.mode {
        vale_edit::ops::shade::Shade::Paint => {
            theme::row(ui, "tint", |ui| {
                ui.color_edit_button_rgb(&mut shading.picked);
            });
        }
        _ => theme::note(ui, "this mode sets its own colour"),
    }
    theme::row(ui, "brightness", |ui| {
        ui.add(
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
    theme::note(
        ui,
        format!(
            "target {:.2}, {:.2}, {:.2}; 1.00 leaves a vertex unchanged",
            aim[0], aim[1], aim[2]
        ),
    );
    if ui.button("Reset to neutral").clicked() {
        shading.picked = [1.0, 1.0, 1.0];
        shading.brightness = 1.0;
    }

    // The same three headings as the height brush, in the same order. With one
    // heading, Shape, over the radius, strength, core, falloff row and shape
    // row, the two unlabelled segmented controls at the bottom read as part of
    // the rows above them.
    ui.add_space(4.0);
    theme::heading(ui, "Size");
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut shading.brush.radius)
                .speed(0.5)
                .range(shading::RADIUS)
                .suffix(" yd"),
        );
    });
    theme::row(ui, "strength", |ui| {
        ui.add(
            egui::DragValue::new(&mut shading.brush.strength)
                .speed(0.05)
                .range(shading::RATE)
                .suffix("/s"),
        );
    });
    theme::row(ui, "core", |ui| {
        ui.add(
            egui::DragValue::new(&mut shading.brush.core)
                .speed(0.01)
                .range(crate::tools::CORE)
                .fixed_decimals(2),
        );
    });

    ui.add_space(4.0);
    theme::heading(ui, "Falloff");
    theme::segmented(ui, &mut shading.brush.falloff, &FALLOFFS, |a, b| a == b);

    ui.add_space(4.0);
    theme::heading(ui, "Shape");
    theme::segmented(ui, &mut shading.brush.shape, &SHAPES, |a, b| a == b);

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button paints");
    theme::note(ui, "ctrl + wheel resizes · shift + wheel the strength");
    theme::note(ui, "alt + wheel the core");
    theme::note(ui, "1 - 5 pick the mode · 6 - 0 the falloff");
    theme::note(ui, "shift + 1 · 2 · 3 pick the shape");

    ui.add_space(6.0);
    theme::heading(ui, "About");
    theme::note(
        ui,
        "MCCV is a per-vertex colour multiplier; the reference client does not \
         read it. A chunk painted back to neutral drops its MCCV data, so an \
         undone tile is written back byte for byte.",
    );
}

/// The shading brush's inspector panel, with the tablet's settings under it.
pub(crate) fn inspect(ui: &mut egui::Ui, _: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    shading(ui, &mut editing.shading);
    crate::ui::tools::tablet::tablet(ui, &mut editing.tablet, &editing.pen);
}
