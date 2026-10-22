//! The water tool's panel. The state is [`crate::tools::ground::water`].

use bevy_egui::egui;

use crate::ui::widgets::theme;

/// The water tool: what is under the pointer, and the level the brush writes.
///
/// The level is the main setting because a wrong value is invisible: a
/// surface written a yard below the ground is a pool nobody can see. The only
/// way to tell before committing is to compare the level with the ground under
/// it, so the panel prints both and the difference between them.
pub(in crate::ui::tools) fn water(ui: &mut egui::Ui, water: &mut crate::tools::ground::water::Water) {
    use crate::tools::ground::water;

    theme::heading(ui, "Under the pointer");
    match water.ground {
        Some(ground) => {
            theme::row(ui, "ground", |ui| {
                ui.label(theme::number(format!("{ground:.2}")));
            });
            match water.surface {
                Some((kind, level)) => {
                    theme::row(ui, "surface", |ui| {
                        ui.label(theme::number(format!("{level:.2}")));
                    });
                    theme::note(
                        ui,
                        format!("{:?} · {:.2} yd deep", kind, (level - ground).max(0.0)),
                    );
                    if let Some(flags) = water.flags {
                        theme::note(ui, water::flag_words(flags));
                    }
                }
                None => theme::note(ui, "no water"),
            }
        }
        None => theme::note(ui, "pointer is not over an open tile"),
    }

    ui.add_space(4.0);
    theme::heading(ui, "Surface");
    theme::segmented(ui, &mut water.sloped, &[("Flat", false), ("Sloped", true)], |a, b| a == b);
    match water.sloped {
        false => {
            theme::row(ui, "height", |ui| {
                ui.add(
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
    // difference is printed as "under the ground" in the warning colour,
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
    match water.sloped {
        false => {
            theme::note(ui, "space takes the level from the water under the pointer");
            theme::note(ui, "ctrl + space takes it from the ground");
        }
        true => {
            theme::note(ui, "space sets the start, shift + space the end");
            theme::note(ui, "height from the water at the pointer, or with ctrl the ground");
            theme::note(ui, "flat beyond either end");
        }
    }

    ui.add_space(4.0);
    theme::heading(ui, "Liquid");
    theme::segmented(ui, &mut water.brush.kind, &water::KINDS, |a, b| a == b);
    // The cell's two flags. A stroke writes them on every cell it wets; with
    // alt held, it writes them on the wet cells under the brush without
    // changing their level. The census is at
    // `vale_edit::adt::liquid::FISHABLE`.
    for (bit, label, hint) in [
        (
            vale_edit::adt::liquid::FISHABLE,
            "fishable",
            "Set on nearly every sea and lake cell in the shipped maps.",
        ),
        (
            vale_edit::adt::liquid::FATIGUE,
            "deep water",
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

    ui.add_space(4.0);
    theme::heading(ui, "Brush");
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut water.brush.radius)
                .speed(0.25)
                .range(water::RADIUS)
                .suffix(" yd"),
        );
    });
    theme::note(ui, "a cell is 4.17 yd, the same grid as the ground");
    if water.wet != 0 {
        theme::note(
            ui,
            match water.wet > 0 {
                true => format!("{} cell(s) flooded this session", water.wet),
                false => format!("{} cell(s) drained this session", -water.wet),
            },
        );
    }

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button floods · shift + left drains");
    theme::note(
        ui,
        "alt + left writes the two flags to wet cells, level unchanged",
    );
    theme::note(ui, "ctrl + left updates depth from the ground below");
    theme::note(ui, "ctrl + wheel resizes");
    theme::note(ui, "playtest movement reads the same MCLQ data");
}

/// The two ends of a sloped surface: each one's height, or that it is not
/// set, and how far the surface falls between them.
fn slope_rows(ui: &mut egui::Ui, water: &mut crate::tools::ground::water::Water) {
    for (at, label) in [(0, "start"), (1, "end")] {
        theme::row(ui, label, |ui| match &mut water.ends[at] {
            Some(end) => {
                ui.add(
                    egui::DragValue::new(&mut end[2])
                        .speed(0.1)
                        .range(crate::tools::ground::water::LEVEL)
                        .fixed_decimals(2),
                );
                if ui.small_button("Clear").clicked() {
                    water.ends[at] = None;
                }
            }
            None => {
                ui.label(egui::RichText::new("not set").color(theme::INK_FAINT));
            }
        });
    }
    if let [Some(start), Some(end)] = water.ends {
        let run = ((end[0] - start[0]).powi(2) + (end[1] - start[1]).powi(2)).sqrt();
        let fall = start[2] - end[2];
        theme::note(
            ui,
            match fall >= 0.0 {
                true => format!("falls {fall:.1} yd over {run:.0} yd"),
                false => format!("rises {:.1} yd over {run:.0} yd", -fall),
            },
        );
    }
}
