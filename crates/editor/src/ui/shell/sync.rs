//! The Server panel: every server operation for this project. It shows what
//! the project changes, how much of that is in the database, and the four
//! operations that act on it.
//!
//! ## Why all subjects share one panel
//!
//! Several subjects store their edits as rows in vmangos' database rather than
//! as bytes in a file: the client tables (spells, flight nodes and skill line
//! abilities), creatures, objects, items, quests, loot, vendors and trainers,
//! and behaviour. Each subject used to keep these operations in its own panel.
//! The result was three names for one operation
//! (*Apply to the server*, *Apply*, and a checkbox in a third window for
//! spells), three names for its inverse (*Put the rows back*, *Revert*, *Put
//! the server back*), and one panel that applied on every save while the other
//! two waited for a button that was not visible where the edit was made.
//!
//! The operations now live here, opened from the bar's *Server…* button. Every
//! subject gets the same block, in the same order, with the same words:
//!
//! ```text
//! ┌ <subject>  N row(s) changed · M applied      [Discard] [Put back] [Apply] ┐
//! │ <a refusal, a trouble, or an unsaved table, when there is one>            │
//! └───────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The subject's name carries the rest in its hover text: the tables it
//! writes, the file the statements are in, and what it takes for an applied
//! change to be live. A block with changes not yet applied is outlined amber.
//!
//! The subject panels show which rows this project changes and what state
//! they are in, and direct a person to this panel for the operations. See
//! [`crate::ui::subjects::items`] and [`crate::ui::tools::spawns::creatures`].
//!
//! The subjects, their standing and the four operations are in
//! [`crate::server::sync`]; this file draws them.

use bevy_egui::egui;

use crate::server::dbcs;
use crate::server::sync::{apply, discard, put_back, Half, Standing, Work};
use crate::ui::widgets::theme;

/// Draws the whole section: the *Apply on save* switch, then one block per
/// subject.
pub fn project(ui: &mut egui::Ui, work: &mut Work<'_>) {
    theme::heading(ui, "Server operations");
    let have_a_database = work.server.resolve().is_some();

    // One switch covers every subject, as its label says. See
    // `ServerSettings::apply_on_save`.
    if ui
        .add_enabled(
            have_a_database,
            egui::Checkbox::new(&mut work.server.apply_on_save, "Apply on save"),
        )
        .on_hover_text(
            "Every save also runs the SQL it writes against the world database, for every \
             subject. A save made with the panels open over a playtest also sends .reload \
             for each table that has one. When off, a save only writes the SQL files.",
        )
        .on_disabled_hover_text("No world database is set.")
        .changed()
    {
        work.server.save(&work.assets.root);
    }
    theme::note(
        ui,
        "A save always writes the SQL statements into the project folder. This setting \
         decides whether the save also runs them against the database.",
    );

    ui.add_space(4.0);
    for half in Half::ALL {
        block(ui, work, half, have_a_database);
        ui.add_space(4.0);
    }
}

/// Draws one subject's block. The module comment shows its layout.
fn block(ui: &mut egui::Ui, work: &mut Work<'_>, half: Half, have_a_database: bool) {
    let standing = work
        .standings
        .of(half, work.session, work.assets, work.now)
        .clone();
    // The line is amber while changed rows are not applied, and plain once the
    // database holds them all. A warning colour that is always shown stops
    // being read as a warning.
    let outstanding = standing.changed > 0 && standing.outstanding > 0;
    let colour = match outstanding {
        false => theme::INK_DIM,
        true => theme::WARN,
    };
    let stroke = match outstanding {
        false => theme::LINE,
        true => theme::WARN,
    };
    egui::Frame::default()
        .fill(theme::SUNK)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            block_body(ui, work, half, have_a_database, &standing, colour);
        });
}

/// The inside of one subject's block: the name, the state and the buttons on
/// one line, and under them, when there is one, the plan's error, each
/// refusal, the server DBC files' line and the unsaved-table note.
fn block_body(
    ui: &mut egui::Ui,
    work: &mut Work<'_>,
    half: Half,
    have_a_database: bool,
    standing: &Standing,
    colour: egui::Color32,
) {
    let about = format!(
        "{}\n\nA save writes the statements to {}; {} holds the statements that revert \
         every applied row.\n\n{}",
        half.tables(),
        half.file(),
        half.revert_file(),
        half.going_live()
    );
    // The client-table block also stands for the DBC files the server reads
    // from DataDir\5875\dbc, which Apply copies and Put back restores.
    let files = match half {
        Half::Tables => Some(work.standings.files(work.session, work.server, work.now).clone()),
        _ => None,
    };
    let (files_outstanding, files_saved) = match &files {
        Some(Ok(files)) => (files.outstanding(), files.saved),
        _ => (false, 0),
    };
    // Apply and Put back are disabled while a write is running, rather than
    // queueing behind it. A second press while the first is still running is
    // almost always a repeat of the same press.
    let busy = work.queue.busy();
    let mut said: Option<Result<String, String>> = None;
    egui::Sides::new().shrink_left().wrap().show(
        ui,
        |ui| {
            ui.label(egui::RichText::new(half.name()).strong().size(13.0).color(theme::INK))
                .on_hover_text(&about);
            ui.label(egui::RichText::new(standing.line()).small().color(colour));
        },
        |ui| {
            buttons(
                ui,
                work,
                half,
                have_a_database && (standing.appliable() || files_outstanding) && !busy,
                have_a_database && (standing.applied > 0 || files_saved > 0) && !busy,
                have_a_database,
                busy,
                standing.changed > 0,
                &mut said,
            );
        },
    );
    if let Some(trouble) = &standing.trouble {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
    }
    for refused in &standing.refused {
        ui.label(egui::RichText::new(refused).small().color(theme::BAD));
    }
    // The client-table block also stands for the DBC files the server reads
    // from DataDir\5875\dbc, which Apply copies and Put back restores.
    match &files {
        Some(Ok(files)) => {
            let colour = match files.outstanding() {
                true => theme::WARN,
                false => theme::INK_DIM,
            };
            ui.label(egui::RichText::new(files.line()).small().color(colour))
                .on_hover_text(format!(
                    "The client tables vmangos reads as DBC files: {}. Apply copies the \
                     project's versions into DataDir\\5875\\dbc, first saving the server's \
                     originals under the project's {}; Restore copies the originals back.",
                    match files.carried.is_empty() {
                        true => "none in this project".to_string(),
                        false => files.carried.join(", "),
                    },
                    dbcs::BEFORE_DIR
                ));
        }
        Some(Err(e)) if !dbcs::carried(&work.session.project).is_empty() => {
            ui.label(
                egui::RichText::new(format!("server DBC files: {e}"))
                    .small()
                    .color(theme::BAD),
            );
        }
        _ => {}
    }
    if standing.unsaved {
        ui.label(
            egui::RichText::new(
                "A table was edited since the last save and is not in the counts above. \
                 Apply saves it into the project first.",
            )
            .small()
            .color(theme::WARN),
        );
    }

    if let Some(said) = said {
        // Any of these presses makes the numbers above stale. See
        // [`crate::server::sync::Standings`]: the risk of that cache is a
        // held answer that outlives the press that changed it.
        work.standings.forget();
        work.session.status = match said {
            Ok(line) => {
                bevy::prelude::info!("server: {line}");
                line
            }
            Err(e) => {
                bevy::prelude::warn!("server: {e}");
                format!("{}: {e}", half.name().to_lowercase())
            }
        };
    }
}

/// A block's three buttons, laid out from the right: Apply, Put back, then
/// Discard or the note that the subject is edited as a file. A press stores
/// its result in `said`.
#[allow(clippy::too_many_arguments)]
fn buttons(
    ui: &mut egui::Ui,
    work: &mut Work<'_>,
    half: Half,
    can_apply: bool,
    can_put_back: bool,
    have_a_database: bool,
    busy: bool,
    changed: bool,
    said: &mut Option<Result<String, String>>,
) {
    {
        if ui
            .add_enabled(can_apply, egui::Button::new("Apply"))
            .on_hover_text(format!(
                "Write {} and run it against the world database, after writing the \
                 statements that put every row back into {}.{}",
                half.file(),
                half.revert_file(),
                half.order_note()
            ))
            .on_disabled_hover_text(match (have_a_database, busy) {
                (false, _) => "No world database is set.",
                (true, true) => "A server write is running.",
                (true, false) => "This project changes no row of these tables.",
            })
            .clicked()
        {
            *said = Some(apply(half, work));
        }
        if ui
            .add_enabled(can_put_back, egui::Button::new("Restore"))
            .on_hover_text(format!(
                "Run {}, which restores every row this project has applied to its value \
                 before the project first changed it, then discard the revert record.{}",
                half.revert_file(),
                half.order_note()
            ))
            .on_disabled_hover_text(match (have_a_database, busy) {
                (false, _) => "No world database is set.",
                (true, true) => "A server write is running.",
                (true, false) => "This project has applied nothing.",
            })
            .clicked()
        {
            *said = Some(put_back(half, work));
        }
        if half.discardable() {
            if ui
                .add_enabled(changed, egui::Button::new("Discard"))
                .on_hover_text(
                    "Discard every edit this project holds for these tables. The database \
                     is not changed and the revert record is kept: Restore still works, \
                     and the next Apply restores the applied rows first.",
                )
                .on_disabled_hover_text("This project changes no row of these tables.")
                .clicked()
            {
                *said = Some(Ok(discard(half, work.session)));
            }
        } else {
            ui.label(
                egui::RichText::new("edited as a file")
                    .small()
                    .color(theme::INK_FAINT),
            )
            .on_hover_text(
                "Client table edits are stored in this project's DBC files, such as \
                 Spell.dbc, not as rows, so there is nothing to discard here. To revert a \
                 spell edit, use Undo or Discard in the Spells workspace.",
            );
        }
    }
}
