//! The preview pane: one model's rendered picture, at the inspector's width.

use bevy_egui::egui;

use crate::ui::widgets::theme;

/// The large preview of one model, with its heading and a button that resets
/// it to the resting view.
///
/// Shared by the placement pickers and the spell workspace's model browser,
/// because the pane is the same under both. `crate::preview::portraits::paint_large`
/// does the drawing.
pub(in crate::ui) fn preview_pane(
    ui: &mut egui::Ui,
    portraits: &mut crate::preview::portraits::Portraits,
    path: &str,
) {
    preview_pane_at(ui, portraits, path, PREVIEW_MAX);
}

/// [`preview_pane`] with a size cap chosen by the caller.
///
/// [`PREVIEW_MAX`] is the inspector's cap, set by the list under the pane; see
/// its own note. A panel with nothing under the pane has no such limit and
/// passes the space it has. The item workspace's appearance column holds one
/// picture and its details and is resizable, so the picture grows with the
/// column.
pub(in crate::ui) fn preview_pane_at(
    ui: &mut egui::Ui,
    portraits: &mut crate::preview::portraits::Portraits,
    path: &str,
    largest: f32,
) {
    let pane = crate::preview::portraits::pane_id(ui);
    ui.horizontal(|ui| {
        theme::heading(ui, "Preview");
        if portraits.large_turned(pane) {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("reset")
                    .on_hover_text("Reset the preview to the camera angle the list rows use.")
                    .clicked()
                {
                    portraits.reset_large_turn(pane);
                }
            });
        }
    });
    // Square, and never wider than the panel it is in: the camera renders at
    // one aspect, so a box of another shape would stretch the model.
    let side = ui.available_width().min(largest);
    ui.vertical_centered(|ui| {
        crate::preview::portraits::paint_large(ui, portraits, pane, path, side);
    });
}

/// The largest the preview pane is drawn, in points.
///
/// 170: four times a row's picture and about a third of the inspector's
/// height. At 220 the list under the pane shows a single row. The pane is for
/// judging the model already chosen, and taking the list's space makes choosing
/// the next one harder. A wide panel does not enlarge it, for the same reason.
pub(super) const PREVIEW_MAX: f32 = 170.0;
