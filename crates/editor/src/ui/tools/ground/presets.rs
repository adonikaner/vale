//! The brush preset grid and drop-down the terrain and texture panels share.
//! The presets themselves are [`crate::prefs::brushes`].
//!
//! The grid draws each preset as an icon over its name, as WoWEdit's brush
//! windows do. The icon shows what the preset does: its mode for the terrain
//! brush, and its shape, spray or eraser for the texture brush. A click
//! chooses a preset; the right button opens Copy, Rename and Delete. While the
//! brush differs from the chosen preset, the preset's name ends in an
//! asterisk and is drawn in the warning colour, and Save and Discard are
//! enabled.

use bevy_egui::egui;

use crate::prefs::brushes::{Ask, Paint, Sculpt};
use crate::ui::widgets::theme;
use vale_edit::ops::brush::{Mode, Shape};

/// What a preset's icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Icon {
    Raise,
    Lower,
    Flatten,
    Smooth,
    Noise,
    Blur,
    Terrace,
    Erode,
    Mountain,
    Crevice,
    Round,
    Square,
    Diamond,
    Speckle,
    Erase,
}

impl Icon {
    /// The icon of a terrain preset: its mode.
    pub(super) fn of_sculpt(settings: &Sculpt) -> Icon {
        match settings.mode {
            Mode::Raise => Icon::Raise,
            Mode::Lower => Icon::Lower,
            Mode::Flatten { .. } => Icon::Flatten,
            Mode::Smooth => Icon::Smooth,
            Mode::Noise => Icon::Noise,
            Mode::Blur => Icon::Blur,
            Mode::Terrace { .. } => Icon::Terrace,
            Mode::Erode => Icon::Erode,
            Mode::Mountain => Icon::Mountain,
            Mode::Crevice => Icon::Crevice,
        }
    }

    /// The icon of a texture preset: Erase, a spray, or its shape.
    pub(super) fn of_paint(settings: &Paint) -> Icon {
        match (settings.erase, settings.density < 1.0, settings.shape) {
            (true, ..) => Icon::Erase,
            (false, true, _) => Icon::Speckle,
            (false, false, Shape::Circle) => Icon::Round,
            (false, false, Shape::Square) => Icon::Square,
            (false, false, Shape::Diamond) => Icon::Diamond,
        }
    }

    /// Paint the icon centred in `rect`.
    fn paint(self, painter: &egui::Painter, rect: egui::Rect, colour: egui::Color32) {
        let c = rect.center();
        let r = rect.width().min(rect.height()) * 0.38;
        match self {
            Icon::Raise | Icon::Round => {
                painter.circle_filled(c, r, colour);
            }
            Icon::Lower => {
                painter.circle_stroke(c, r * 0.85, egui::Stroke::new(2.5, colour));
            }
            Icon::Flatten => {
                let points = vec![
                    egui::pos2(c.x - r * 1.2, c.y + r * 0.6),
                    egui::pos2(c.x - r * 0.5, c.y - r * 0.4),
                    egui::pos2(c.x + r * 0.5, c.y - r * 0.4),
                    egui::pos2(c.x + r * 1.2, c.y + r * 0.6),
                ];
                painter.add(egui::Shape::convex_polygon(points, colour, egui::Stroke::NONE));
            }
            Icon::Smooth => {
                // A soft dot: rings fading outward from a solid middle.
                for (scale, alpha) in [(1.0, 0.25), (0.8, 0.45), (0.6, 0.7), (0.4, 1.0)] {
                    painter.circle_filled(c, r * scale, colour.gamma_multiply(alpha));
                }
            }
            Icon::Noise => {
                for row in [-0.4, 0.4] {
                    let y = c.y + r * row;
                    let points: Vec<egui::Pos2> = (0..=6)
                        .map(|i| {
                            let x = c.x - r * 1.1 + i as f32 * r * 2.2 / 6.0;
                            let up = match i % 2 {
                                0 => -0.25,
                                _ => 0.25,
                            };
                            egui::pos2(x, y + r * up)
                        })
                        .collect();
                    painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, colour)));
                }
            }
            Icon::Blur => {
                // A jagged line above and the same line smoothed below it.
                let wave = |row: f32, sharp: bool| -> Vec<egui::Pos2> {
                    (0..=12)
                        .map(|i| {
                            let s = i as f32 / 12.0;
                            let x = c.x - r * 1.1 + s * r * 2.2;
                            let up = match sharp {
                                true => (s * 18.0).sin() * 0.22 + (s * 5.0).sin() * 0.12,
                                false => (s * 5.0).sin() * 0.12,
                            };
                            egui::pos2(x, c.y + r * (row + up))
                        })
                        .collect()
                };
                painter.add(egui::Shape::line(wave(-0.45, true), egui::Stroke::new(1.5, colour.gamma_multiply(0.5))));
                painter.add(egui::Shape::line(wave(0.45, false), egui::Stroke::new(2.0, colour)));
            }
            Icon::Terrace => {
                // Three steps rising to the right.
                let mut points = vec![egui::pos2(c.x - r * 1.1, c.y + r * 0.8)];
                for k in 0..3 {
                    let x = c.x - r * 1.1 + (k as f32 + 1.0) * r * 2.2 / 3.0;
                    let y = c.y + r * (0.8 - k as f32 * 0.6);
                    points.push(egui::pos2(x - r * 0.25, y));
                    points.push(egui::pos2(x, y - r * 0.6));
                }
                points.push(egui::pos2(c.x + r * 1.1, c.y - r * 1.0));
                painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, colour)));
            }
            Icon::Erode => {
                // A slope with gullies running down it.
                let slope = [egui::pos2(c.x - r * 1.1, c.y - r * 0.8), egui::pos2(c.x + r * 1.1, c.y + r * 0.8)];
                painter.line_segment(slope, egui::Stroke::new(2.0, colour));
                for k in 0..3 {
                    let s = 0.2 + k as f32 * 0.3;
                    let on = slope[0] + (slope[1] - slope[0]) * s;
                    painter.line_segment(
                        [on, on + egui::vec2(r * 0.15, r * 0.55)],
                        egui::Stroke::new(1.5, colour.gamma_multiply(0.7)),
                    );
                }
            }
            Icon::Mountain => {
                let points = vec![
                    egui::pos2(c.x - r * 1.2, c.y + r * 0.8),
                    egui::pos2(c.x - r * 0.35, c.y - r * 0.3),
                    egui::pos2(c.x - r * 0.05, c.y),
                    egui::pos2(c.x + r * 0.35, c.y - r * 0.9),
                    egui::pos2(c.x + r * 1.2, c.y + r * 0.8),
                ];
                painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, colour)));
            }
            Icon::Crevice => {
                // Level ground with a raised rim either side of a V.
                let points = vec![
                    egui::pos2(c.x - r * 1.2, c.y - r * 0.1),
                    egui::pos2(c.x - r * 0.6, c.y - r * 0.4),
                    egui::pos2(c.x, c.y + r * 0.9),
                    egui::pos2(c.x + r * 0.6, c.y - r * 0.4),
                    egui::pos2(c.x + r * 1.2, c.y - r * 0.1),
                ];
                painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, colour)));
            }
            Icon::Square => {
                painter.rect_filled(egui::Rect::from_center_size(c, egui::Vec2::splat(r * 1.7)), 1.0, colour);
            }
            Icon::Diamond => {
                let points = vec![
                    egui::pos2(c.x, c.y - r),
                    egui::pos2(c.x + r, c.y),
                    egui::pos2(c.x, c.y + r),
                    egui::pos2(c.x - r, c.y),
                ];
                painter.add(egui::Shape::convex_polygon(points, colour, egui::Stroke::NONE));
            }
            Icon::Speckle => {
                for (dx, dy) in [(-0.6, -0.5), (0.5, -0.6), (0.0, 0.0), (-0.5, 0.6), (0.6, 0.5)] {
                    painter.circle_filled(egui::pos2(c.x + r * dx, c.y + r * dy), r * 0.28, colour);
                }
            }
            Icon::Erase => {
                painter.circle_stroke(c, r * 0.85, egui::Stroke::new(2.0, theme::BAD));
                let d = r * 0.5;
                painter.line_segment(
                    [egui::pos2(c.x - d, c.y + d), egui::pos2(c.x + d, c.y - d)],
                    egui::Stroke::new(2.0, theme::BAD),
                );
            }
        }
    }
}

/// The narrowest and the height of one cell of the grid, in points. A row
/// holds as many cells of at least this width as fit, and they share the
/// row's width equally, so the grid's right edge is the panel's.
const CELL: egui::Vec2 = egui::vec2(62.0, 58.0);

/// The space between two cells, in points.
const CELL_GAP: f32 = 4.0;

/// The fill of a cell that is neither chosen nor hovered: one step off the
/// panel, so the grid reads as a set of tiles to press.
const CELL_FILL: egui::Color32 = egui::Color32::from_rgb(0x24, 0x29, 0x32);

/// The presets as a grid of icons over names, with the chosen one marked,
/// and marked as modified when `modified`. A click chooses a preset; the right
/// button opens Copy, Rename and Delete. Answers what was asked.
pub(super) fn grid(
    ui: &mut egui::Ui,
    id: &str,
    presets: &[(String, Icon)],
    chosen: Option<usize>,
    modified: bool,
) -> Option<Ask> {
    let mut asked = None;
    let room = ui.available_width();
    let across = (((room + CELL_GAP) / (CELL.x + CELL_GAP)).floor() as usize).max(1);
    let cell = egui::vec2(((room - CELL_GAP * (across - 1) as f32) / across as f32).floor(), CELL.y);
    // The cell width is rounded down, so `across` cells and their gaps never
    // exceed the row and the layout wraps after exactly that many.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(CELL_GAP, CELL_GAP);
        for (at, (name, icon)) in presets.iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(cell, egui::Sense::click());
            let on = chosen == Some(at);
            let painter = ui.painter();
            match (on, response.hovered()) {
                (true, _) => {
                    painter.rect_filled(rect, 3.0, theme::ACCENT_SUNK);
                    painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
                }
                (false, true) => {
                    painter.rect_filled(rect, 3.0, theme::RAISED_HOT);
                }
                (false, false) => {
                    painter.rect_filled(rect, 3.0, CELL_FILL);
                }
            }
            let picture = egui::Rect::from_min_size(rect.min + egui::vec2(0.0, 4.0), egui::vec2(rect.width(), 24.0));
            icon.paint(painter, picture, if on { theme::INK } else { theme::INK_DIM });
            let (shown, colour) = match (on, on && modified) {
                (_, true) => (format!("{name} *"), theme::WARN),
                (true, false) => (name.clone(), theme::INK),
                (false, _) => (name.clone(), theme::INK_DIM),
            };
            let mut job = egui::text::LayoutJob::single_section(
                shown,
                egui::TextFormat::simple(egui::FontId::proportional(theme::SMALL), colour),
            );
            // Up to two lines, cut with an ellipsis after that.
            job.wrap = egui::text::TextWrapping {
                max_width: rect.width() - 6.0,
                max_rows: 2,
                break_anywhere: false,
                overflow_character: Some('\u{2026}'),
            };
            job.halign = egui::Align::Center;
            let galley = painter.layout_job(job);
            painter.galley(egui::pos2(rect.center().x, picture.bottom() + 2.0), galley, theme::INK);
            let response = response.on_hover_text(name.as_str());
            if response.clicked() {
                asked = Some(Ask::Choose(at));
            }
            response.context_menu(|ui| {
                if ui.button("Copy").on_hover_text("Make a copy of this brush after it, and choose the copy.").clicked() {
                    asked = Some(Ask::Copy(at));
                    ui.close();
                }
                // The new name is a draft kept while the menu is open.
                let draft_id = egui::Id::new((id, "rename", at));
                let mut draft = ui.data(|data| data.get_temp::<String>(draft_id)).unwrap_or_else(|| name.clone());
                ui.horizontal(|ui| {
                    let edit = ui.add(egui::TextEdit::singleline(&mut draft).desired_width(120.0));
                    let entered = edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if ui.button("Rename").clicked() || entered {
                        asked = Some(Ask::Rename(at, draft.clone()));
                        ui.close();
                    }
                });
                match asked {
                    Some(Ask::Rename(..)) => ui.data_mut(|data| data.remove::<String>(draft_id)),
                    _ => ui.data_mut(|data| {
                        data.insert_temp(draft_id, draft);
                    }),
                }
                ui.separator();
                if ui.button(egui::RichText::new("Delete").color(theme::BAD)).clicked() {
                    asked = Some(Ask::Delete(at));
                    ui.close();
                }
            });
        }
    });
    asked
}

/// The section a brush's inspector opens with: the grid, the Save, Discard
/// and Save as New buttons, and how presets are edited. Answers what was
/// asked.
pub(super) fn section(
    ui: &mut egui::Ui,
    id: &str,
    presets: &[(String, Icon)],
    chosen: Option<usize>,
    modified: bool,
) -> Option<Ask> {
    theme::heading_about(
        ui,
        "Brushes",
        "Click a brush to use it. Right-click a brush to copy, rename or delete it.\n\n\
         A change to the settings below is kept for this session. Save writes it into the \
         chosen brush, Discard drops it, and choosing another brush drops it.",
    );
    let mut asked = grid(ui, id, presets, chosen, modified);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(modified, egui::Button::new("Save"))
            .on_hover_text("Write the brush's current settings into the chosen preset.")
            .on_disabled_hover_text("The brush's settings are the chosen preset's.")
            .clicked()
        {
            asked = Some(Ask::Save);
        }
        if ui
            .add_enabled(modified, egui::Button::new("Discard"))
            .on_hover_text("Set the brush back to the chosen preset's settings.")
            .on_disabled_hover_text("The brush's settings are the chosen preset's.")
            .clicked()
        {
            asked = Some(Ask::Discard);
        }
        if ui
            .button("Save as New")
            .on_hover_text("Save the brush's current settings as a new preset, and choose it.")
            .clicked()
        {
            asked = Some(Ask::New);
        }
    });
    asked
}

/// The Brush Settings section under the grid, as a heading that folds
/// ([`theme::fold`]). Open at first; egui keeps whether it is open under
/// `id`, so it stays as it was left.
pub(super) fn settings(ui: &mut egui::Ui, id: &str, body: impl FnOnce(&mut egui::Ui)) {
    theme::fold(ui, id, "Brush Settings", body);
}

/// The toolbar's Brush drop-down: the presets by name, the chosen one with an
/// asterisk while `modified`. Answers the one chosen this frame.
pub(super) fn picker(ui: &mut egui::Ui, id: &str, names: &[&str], chosen: Option<usize>, modified: bool) -> Option<usize> {
    let mut picked = None;
    let name = chosen.and_then(|at| names.get(at)).copied().unwrap_or("None");
    let shown = match modified {
        true => egui::RichText::new(format!("{name} *")).color(theme::WARN),
        false => egui::RichText::new(name),
    };
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(110.0)
        .truncate()
        .show_ui(ui, |ui| {
            for (at, name) in names.iter().enumerate() {
                if ui.selectable_label(chosen == Some(at), *name).clicked() {
                    picked = Some(at);
                }
            }
        })
        .response
        .on_hover_text(
            "The brush preset. An asterisk means the brush has changes not saved into it; the \
             inspector saves or discards them, and copies, renames and deletes presets.",
        );
    picked
}
