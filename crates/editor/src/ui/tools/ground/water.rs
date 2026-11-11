//! The water tool's toolbar and panel. The state is
//! [`crate::tools::ground::water`].

use bevy_egui::egui;

use crate::ui::widgets::theme::{self, bar_group, bar_keys, bar_segment, bar_setting, field, property};

/// The water tool's toolbar over the viewport:
///
/// ```text
/// Surface [Flat | Sloped]  Keys
/// ```
///
/// Flat or Sloped decides what a stroke writes and what Space takes, so it
/// is the bar's [`theme::bar_group`]. The heights are in the inspector. See
/// [`theme::toolbar`].
pub(crate) fn toolbar(
    ctx: &egui::Context,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    water: &mut crate::tools::ground::water::Water,
) -> Option<egui::Rect> {
    theme::toolbar(ctx, "water-toolbar", viewport, corner, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            bar_setting(ui, "Surface", |ui| {
                bar_group(ui, |ui| {
                    if bar_segment(ui, !water.sloped, "Flat")
                        .on_hover_text("A stroke writes one height everywhere.")
                        .clicked()
                    {
                        water.sloped = false;
                    }
                    if bar_segment(ui, water.sloped, "Sloped")
                        .on_hover_text(
                            "A stroke writes a surface that falls from a start to an end, and is \
                             flat beyond either.",
                        )
                        .clicked()
                    {
                        water.sloped = true;
                    }
                });
            });
            bar_keys(ui, match water.sloped {
                false => &FLAT_KEYS,
                true => &SLOPED_KEYS,
            });
        });
    })
}

/// The tool's mouse and keyboard controls while the surface is flat, for the
/// toolbar's Keys button.
const FLAT_KEYS: [(&str, &str); 7] = [
    ("Left button", "Flood"),
    ("Shift + left button", "Drain"),
    ("Alt + left button", "Write the two flags to wet cells, level unchanged"),
    ("Ctrl + left button", "Update depth from the ground below"),
    ("Ctrl + wheel", "Radius"),
    ("Space", "Take the height from the water under the pointer"),
    ("Ctrl + Space", "Take the height from the ground"),
];

/// The same while the surface is sloped, where Space sets an end.
const SLOPED_KEYS: [(&str, &str); 8] = [
    ("Left button", "Flood"),
    ("Shift + left button", "Drain"),
    ("Alt + left button", "Write the two flags to wet cells, level unchanged"),
    ("Ctrl + left button", "Update depth from the ground below"),
    ("Ctrl + wheel", "Radius"),
    ("Space", "Set the start at the water under the pointer"),
    ("Shift + Space", "Set the end"),
    ("Ctrl with either", "Take the height from the ground"),
];

/// `n` cells, in words.
fn cells(n: i32) -> String {
    match n {
        1 => "1 cell".to_string(),
        n => format!("{n} cells"),
    }
}

/// The water tool: what is under the pointer, and the level the brush writes.
///
/// The level is the main setting because a wrong value is invisible: a
/// surface written a yard below the ground is a pool nobody can see. The only
/// way to tell before committing is to compare the level with the ground under
/// it, so the panel prints both and the difference between them.
pub(in crate::ui::tools) fn water(ui: &mut egui::Ui, water: &mut crate::tools::ground::water::Water) {
    use crate::tools::ground::water;

    theme::heading(ui, "Under the Pointer");
    match water.ground {
        Some(ground) => {
            property(ui, "Ground", "", |ui| {
                theme::value(ui, format!("{ground:.2}"));
            });
            match water.surface {
                Some((kind, level)) => {
                    property(ui, "Surface", "", |ui| {
                        theme::value(ui, format!("{level:.2}"));
                    });
                    // One line: the liquid, its depth, and the cell's flags.
                    let mut words = format!("{:?} \u{b7} {:.2} yd deep", kind, (level - ground).max(0.0));
                    if let Some(flags) = water.flags {
                        words.push_str(" \u{b7} ");
                        words.push_str(&water::flag_words(flags));
                    }
                    theme::note(ui, words);
                }
                None => theme::note(ui, "No water"),
            }
        }
        None => theme::note(ui, "Pointer is not over an open tile"),
    }

    // Flat or Sloped is chosen on the toolbar; these are that choice's
    // heights.
    theme::heading(ui, "Surface");
    match water.sloped {
        false => {
            property(ui, "Height", "", |ui| {
                field(
                    ui,
                    egui::DragValue::new(&mut water.brush.level)
                        .speed(0.1)
                        .range(water::LEVEL)
                        .fixed_decimals(2),
                );
            });
        }
        true => slope_rows(ui, water),
    }
    // The surface relative to the ground under the pointer. A negative
    // difference is printed as "below the ground" in the error colour,
    // because that is the mistake this panel is meant to catch.
    let here = water
        .point
        .map(|point| water.brush.level_at(point.x, point.y))
        .unwrap_or(water.brush.level);
    if let (Some(ground), false) = (water.ground, water.sloped && water.brush.slope.is_none()) {
        let over = here - ground;
        ui.label(
            egui::RichText::new(match over >= 0.0 {
                true => format!("{over:.2} yd above the ground at the pointer"),
                false => format!("{:.2} yd below the ground at the pointer", -over),
            })
            .size(theme::SMALL)
            .color(match over >= 0.0 {
                true => theme::INK_DIM,
                false => theme::BAD,
            }),
        );
    }

    theme::heading(ui, "Liquid");
    property(ui, "Type", "", |ui| {
        theme::choice(ui, "water-liquid", &mut water.brush.kind, &water::KINDS, |a, b| a == b);
    });
    // The cell's two flags. A stroke writes them on every cell it wets; with
    // alt held, it writes them on the wet cells under the brush without
    // changing their level. The census is at
    // `vale_edit::adt::liquid::FISHABLE`.
    for (bit, label, hint) in [
        (
            vale_edit::adt::liquid::FISHABLE,
            "Fishable",
            "Set on nearly every sea and lake cell in the shipped maps.",
        ),
        (
            vale_edit::adt::liquid::FATIGUE,
            "Deep water",
            "Fatigue flag: a swimmer in these cells takes fatigue damage. The \
             shipped maps set it on open sea only. vmangos reads this bit and \
             no other.",
        ),
    ] {
        let mut on = water.brush.cell_flags & bit != 0;
        if ui.checkbox(&mut on, label).on_hover_text(hint).changed() {
            water.brush.cell_flags = match on {
                true => water.brush.cell_flags | bit,
                false => water.brush.cell_flags & !bit,
            };
        }
    }

    theme::heading_about(ui, "Brush", "Playtest movement reads the same MCLQ data.");
    property(ui, "Radius", "A cell is 4.17 yd, the same grid as the ground.", |ui| {
        field(
            ui,
            egui::DragValue::new(&mut water.brush.radius)
                .speed(0.25)
                .range(water::RADIUS)
                .suffix(" yd"),
        );
    });
    if water.wet != 0 {
        theme::note(
            ui,
            match water.wet > 0 {
                true => format!("{} flooded this session", cells(water.wet)),
                false => format!("{} drained this session", cells(-water.wet)),
            },
        );
    }
}

/// The two ends of a sloped surface: each one's height, or that it is not
/// set, and how far the surface falls between them.
///
/// Clear is laid out first, from the row's right end, and the height takes
/// what is left of the row.
fn slope_rows(ui: &mut egui::Ui, water: &mut crate::tools::ground::water::Water) {
    for (at, label) in [(0, "Start"), (1, "End")] {
        property(ui, label, "", |ui| match &mut water.ends[at] {
            Some(end) => {
                let mut clear = false;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    clear = ui.small_button("Clear").clicked();
                    field(
                        ui,
                        egui::DragValue::new(&mut end[2])
                            .speed(0.1)
                            .range(crate::tools::ground::water::LEVEL)
                            .fixed_decimals(2),
                    );
                });
                if clear {
                    water.ends[at] = None;
                }
            }
            None => {
                ui.label(egui::RichText::new("Not set").color(theme::INK_FAINT));
            }
        });
    }
    if let [Some(start), Some(end)] = water.ends {
        let run = ((end[0] - start[0]).powi(2) + (end[1] - start[1]).powi(2)).sqrt();
        let fall = start[2] - end[2];
        theme::note(
            ui,
            match fall >= 0.0 {
                true => format!("Falls {fall:.1} yd over {run:.0} yd"),
                false => format!("Rises {:.1} yd over {run:.0} yd", -fall),
            },
        );
    }
}

/// The water tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, _: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    water(ui, &mut editing.water);
}
