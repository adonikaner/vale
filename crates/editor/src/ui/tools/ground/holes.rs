//! The holes tool's panel. The state is [`crate::tools::ground::holes`].

use bevy_egui::egui;

use crate::session::EditSession;
use crate::ui::widgets::theme;

/// The hole tool, which has no settings at all.
///
/// A hole is one of sixteen fixed squares per chunk: there is no radius,
/// strength, falloff or shape to choose. The panel shows what the pointer is
/// over and what the tile already has, which the square outline in the
/// viewport does not show.
pub(in crate::ui::tools) fn holes(ui: &mut egui::Ui, holes: &crate::tools::ground::holes::Holes, session: &EditSession) {
    theme::heading(ui, "Square");
    match holes.at {
        Some(at) => {
            theme::row(ui, "tile", |ui| {
                ui.label(theme::number(format!("{},{}", at.tile.0, at.tile.1)));
            });
            theme::row(ui, "chunk", |ui| {
                ui.label(theme::number(at.chunk.to_string()));
            });
            theme::row(ui, "square", |ui| {
                // Row and column of the 4x4 grid are printed beside the bit,
                // because "row 2, column 1" can be matched to the square
                // outlined on screen and "square 9" cannot.
                ui.label(theme::number(format!(
                    "{} · row {} col {}",
                    at.bit,
                    at.bit / 4,
                    at.bit % 4
                )));
            });
            ui.label(
                egui::RichText::new(match holes.already {
                    true => "already cut — shift-click fills it",
                    false => "solid — click cuts it",
                })
                .size(theme::SMALL)
                .color(match holes.already {
                    true => theme::WARN,
                    false => theme::INK_DIM,
                }),
            );
        }
        None => theme::note(ui, "pointer is not over an open tile"),
    }

    ui.add_space(4.0);
    theme::heading(ui, "Tile");
    // What the map already has, not only what this session did. A tile of
    // Azeroth ships with holes wherever a cave mouth or a doorway is, and the
    // count tells an untouched tile from an edited one.
    match holes.at.and_then(|at| session.tiles.get(&at.tile)) {
        Some(tile) => theme::row(ui, "cut", |ui| {
            ui.label(theme::number(format!(
                "{} of {}",
                vale_edit::adt::holes::cut_in(tile),
                tile.chunks.len() * 16
            )));
        }),
        None => theme::note(ui, "no tile open under the pointer"),
    }
    if holes.cut != 0 {
        theme::note(
            ui,
            match holes.cut > 0 {
                true => format!("{} cut this session", holes.cut),
                false => format!("{} filled this session", -holes.cut),
            },
        );
    }

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button cuts · drag cuts every square it crosses");
    theme::note(ui, "shift + left button fills");
    theme::note(
        ui,
        "16 squares per chunk; the file has no finer unit",
    );
}

/// The hole tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, .. } = subject;
    holes(ui, &editing.holes, session);
}
