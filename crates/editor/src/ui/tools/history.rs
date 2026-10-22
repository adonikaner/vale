//! The undo and redo row at the top of an editing tool's panel. The history
//! itself is `EditSession::history`.

use bevy_egui::egui;

use crate::session::EditSession;
use crate::ui::widgets::theme;

/// What undo would take back, and what redo would put in.
pub(super) fn history(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    assets: &vale_client::assets::GameAssets,
) {
    let undo = session.history.next_undo().map(|c| c.label.clone());
    let redo = session.history.next_redo().map(|c| c.label.clone());
    ui.horizontal(|ui| {
        let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if ui
            .add_enabled(
                undo.is_some(),
                egui::Button::new("Undo").min_size(egui::vec2(half, 22.0)),
            )
            .on_hover_text(match &undo {
                Some(label) => format!("Ctrl+Z — {label}"),
                None => "Ctrl+Z".to_string(),
            })
            .clicked()
        {
            session.undo(assets);
        }
        if ui
            .add_enabled(
                redo.is_some(),
                egui::Button::new("Redo").min_size(egui::vec2(half, 22.0)),
            )
            .on_hover_text(match &redo {
                Some(label) => format!("Ctrl+Y — {label}"),
                None => "Ctrl+Y".to_string(),
            })
            .clicked()
        {
            session.redo(assets);
        }
    });
    let depth = session.history.depth_done();
    theme::note(
        ui,
        match (&undo, depth) {
            (Some(label), 1) => format!("{label} — 1 step to undo"),
            (Some(label), n) => format!("{label} — {n} steps to undo"),
            (None, _) => "nothing to undo".to_string(),
        },
    );
}
