//! The holes tool's panel. The state is [`crate::tools::ground::holes`].

use bevy_egui::egui;

use crate::session::EditSession;
use crate::ui::widgets::theme::{self, property};

/// The tool's mouse and keyboard controls, for the Square heading's info
/// mark. The tool has no toolbar over the viewport to hold them.
const KEYS: &str = "Left button: cut\n\
                    Drag: cut every square it crosses\n\
                    Shift + left button: fill\n\n\
                    16 squares per chunk; the file has no finer unit.";

/// The hole tool, which has no settings.
///
/// A hole is one of sixteen fixed squares per chunk: there is no radius,
/// strength, falloff or shape to choose. The panel shows what the pointer is
/// over and what the tile already has, which the square outline in the
/// viewport does not show.
pub(in crate::ui::tools) fn holes(ui: &mut egui::Ui, holes: &crate::tools::ground::holes::Holes, session: &EditSession) {
    theme::heading_about(ui, "Square", KEYS);
    match holes.at {
        Some(at) => {
            property(ui, "Tile", "", |ui| {
                theme::value(ui, format!("{},{}", at.tile.0, at.tile.1));
            });
            property(ui, "Chunk", "", |ui| {
                theme::value(ui, at.chunk.to_string());
            });
            property(ui, "Square", "", |ui| {
                // Row and column of the 4x4 grid are printed beside the bit,
                // because "row 2, column 1" can be matched to the square
                // outlined on screen and "square 9" cannot.
                theme::value(ui, format!("{} \u{b7} row {} col {}", at.bit, at.bit / 4, at.bit % 4));
            });
            ui.label(
                egui::RichText::new(match holes.already {
                    true => "Already cut. Shift + click fills it.",
                    false => "Solid. Click cuts it.",
                })
                .size(theme::SMALL)
                .color(match holes.already {
                    true => theme::WARN,
                    false => theme::INK_DIM,
                }),
            );
        }
        None => theme::note(ui, "Pointer is not over an open tile"),
    }

    theme::heading(ui, "Tile");
    // What the map already has, not only what this session did. A tile of
    // Azeroth ships with holes wherever a cave mouth or a doorway is, and the
    // count tells an untouched tile from an edited one.
    match holes.at.and_then(|at| session.tiles.get(&at.tile)) {
        Some(tile) => property(ui, "Cut", "", |ui| {
            theme::value(
                ui,
                format!("{} of {}", vale_edit::adt::holes::cut_in(tile), tile.chunks.len() * 16),
            );
        }),
        None => theme::note(ui, "No tile open under the pointer"),
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
}

/// The hole tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, .. } = subject;
    holes(ui, &editing.holes, session);
}
