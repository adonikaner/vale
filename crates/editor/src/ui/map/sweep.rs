//! The sweep tool's panel. The state is [`crate::map::sweep`].

use bevy_egui::egui;

use crate::session::EditSession;
use crate::ui::widgets::theme;

/// The sweep tool: a find-and-replace over every tile of the map.
///
/// Find and Replace walk the tiles the same way; only Replace writes. Both are
/// offered because a path is easy to misspell, and a misspelled path reports
/// "no tiles matched", which looks the same as "nothing to do". Find is
/// expected to be pressed first, and it prints the count that decides whether
/// to replace.
///
/// The paths are typed rather than picked. A picker over the archives' listing
/// would offer 5,816 models and 621 tilesets and would be the wrong list for
/// both fields: the find path should be one this map uses, which the listing
/// does not say, and the replacement can be any path. The Chunk list's rows
/// and the placement inspector both print the full path, so the find path can
/// be copied from there.
pub(in crate::ui) fn sweep(
    ui: &mut egui::Ui,
    sweep: &mut crate::map::sweep::Sweep,
    session: &EditSession,
    assets: &vale_client::assets::GameAssets,
) {
    use crate::map::sweep::{Subject, SUBJECTS};

    theme::heading(ui, "Map");
    theme::note(
        ui,
        format!(
            "{}, {} tiles. Tiles reference textures and models by index into a \
             path list, so replacing a path changes nothing else: no chunk is \
             repainted and no placement moves.",
            session.map,
            session.claimed.len()
        ),
    );

    ui.add_space(4.0);
    let running = sweep.running.is_some();
    ui.add_enabled_ui(!running, |ui| {
        theme::segmented(ui, &mut sweep.subject, &SUBJECTS, |a, b| a == b);
        ui.add_space(4.0);
        theme::heading(ui, "Find path");
        ui.add(
            egui::TextEdit::singleline(&mut sweep.from)
                .hint_text(sweep.subject.root())
                .desired_width(f32::INFINITY),
        );
        theme::heading(ui, "Replace with");
        ui.add(
            egui::TextEdit::singleline(&mut sweep.to)
                .hint_text(match sweep.subject {
                    Subject::Texture => r"Tileset\Barrens\BarrensDirt01.blp",
                    Subject::Model => r"World\Azeroth\Elwynn\PassiveDoodads\Trees\ElwynnTree01.m2",
                    Subject::Building => r"World\wmo\Azeroth\Buildings\Human_Farm\Farm.wmo",
                })
                .desired_width(f32::INFINITY),
        );
    });

    ui.add_space(6.0);
    let named = !sweep.from.trim().is_empty();
    ui.horizontal(|ui| {
        let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if ui
            .add_enabled(
                !running && named,
                egui::Button::new("Find").min_size(egui::vec2(half, 24.0)),
            )
            .on_hover_text(
                "Count how many tiles name this path, and how many times, \
                 without changing anything.",
            )
            .on_disabled_hover_text("Enter a path to find.")
            .clicked()
        {
            sweep.find(session, assets);
        }
        if ui
            .add_enabled(
                !running && named && !sweep.to.trim().is_empty(),
                egui::Button::new("Replace").min_size(egui::vec2(half, 24.0)),
            )
            .on_hover_text(
                "Replace the path on every tile of this map, as one undo entry. \
                 Each changed tile is written to the project folder.",
            )
            .on_disabled_hover_text("Enter both paths.")
            .clicked()
        {
            sweep.replace(session, assets);
        }
    });

    if let Some(run) = &sweep.running {
        ui.add_space(4.0);
        let (done, total) = (run.done(), run.total());
        theme::progress(
            ui,
            done as f32 / total.max(1) as f32,
            "Sweep",
            &format!("{done} / {total} tiles"),
            ui.available_width(),
        );
    }

    if let Some(report) = &sweep.last {
        ui.add_space(6.0);
        theme::heading(ui, "Last sweep");
        theme::note(
            ui,
            match report.tiles {
                0 => format!(
                    "{} — not found in any tile of this map, {} scanned.",
                    report.what, report.scanned
                ),
                tiles => format!(
                    "{} — {tiles} tile(s), {} occurrence(s), {} scanned.",
                    report.what, report.names, report.scanned
                ),
            },
        );
    }

    ui.add_space(6.0);
    theme::note(
        ui,
        "Replace does not merge duplicates: replacing A with B on a tile that \
         already lists B leaves two B entries. Merging would renumber the list \
         and every record that indexes it.",
    );
}

/// The sweep's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, assets, .. } = subject;
    sweep(ui, &mut editing.sweep, session, assets);
}
