//! The Server panel: every server operation for this project. It shows what
//! the project changes, how much of that is in the database, and the four
//! operations that act on it.
//!
//! ## Why all subjects share one panel
//!
//! Several subjects store their edits as rows in vmangos' database rather than
//! as bytes in a file: the client tables (spells, flight nodes and skill line
//! abilities), creatures, objects, items, quests, loot, vendors and trainers,
//! and behaviour. Panels of their own per subject gave three names for one
//! operation ("Apply to the server", "Apply", and a checkbox in a third window
//! for spells), three names for its inverse ("Put the rows back", "Revert",
//! "Put the server back"), and one panel that applied on every save while the
//! other two waited for a button that was not visible where the edit was made.
//!
//! The operations are here, opened from the bar's Server… button. Every
//! subject with something to act on gets the same block, in the same order,
//! with the same words:
//!
//! ```text
//! ┌ <subject>                              [Drop Changes] [Restore] [Apply] ┐
//! │ N rows changed, M applied                                               │
//! │ <a refusal, a trouble, or an unsaved table, when there is one>          │
//! └─────────────────────────────────────────────────────────────────────────┘
//! No changes: <every other subject, by name, on one line>
//! ```
//!
//! The subject's name carries the rest in its hover text: the tables it
//! writes, the file the statements are in, and what it takes for an applied
//! change to be live. A block with changes not yet applied is outlined amber.
//!
//! A subject with no change, no applied row, no refusal and no error has no
//! block. All three of its buttons would be disabled, and a project that
//! edits creatures drew sixteen such blocks around the one that mattered.
//! Those subjects are named on one line under the blocks, each with the same
//! hover text as a block's name.
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

/// Draws the whole section: the Apply on save switch, then one block per
/// subject that has anything to act on, then the line naming the rest. In a
/// shared session the switch, Apply and Restore are disabled
/// when `gate` says this editor may not write to its server. Drop Changes is
/// an edit like any other and is sent to the other editors. See
/// `crate::collab::ServerGate`.
pub(crate) fn project(ui: &mut egui::Ui, work: &mut Work<'_>, gate: &crate::collab::ServerGate) {
    theme::heading(ui, "Server Operations");
    let have_a_database = work.server.resolve().is_some();
    match (&gate.refused, &gate.host) {
        (Some(refused), _) => theme::note(ui, refused.clone()),
        (None, Some(host)) => theme::note(
            ui,
            format!("Apply and Restore write to the database in Edit\\server.txt. They do not change {host}'s database."),
        ),
        (None, None) => {}
    }

    // One switch covers every subject, as its label says. See
    // `ServerSettings::apply_on_save`.
    if ui
        .add_enabled(
            have_a_database && gate.writes(),
            egui::Checkbox::new(&mut work.server.apply_on_save, "Apply on save"),
        )
        .on_hover_text(
            "A save always writes the SQL statements into the project folder. On: every save \
             also runs them against the world database, for every subject, and a save made \
             with the panels open over a playtest also sends .reload for each table that has \
             one. Off: a save only writes the SQL files.",
        )
        .on_disabled_hover_text(gate.refused.clone().unwrap_or_else(|| "No world database is set.".to_string()))
        .changed()
    {
        work.server.save(&work.assets.root);
    }

    ui.add_space(4.0);
    let mut quiet: Vec<Half> = Vec::new();
    for half in Half::ALL {
        if is_quiet(work, half) {
            quiet.push(half);
            continue;
        }
        block(ui, work, half, have_a_database, gate);
        ui.add_space(4.0);
    }
    if !quiet.is_empty() {
        unchanged(ui, &quiet, quiet.len() == Half::ALL.len());
    }
}

/// Whether a subject has nothing to act on, so it is named on the last line
/// and has no block: [`Standing::quiet`], and for the client tables also no
/// server DBC file to copy or put back and no error reading them.
fn is_quiet(work: &mut Work<'_>, half: Half) -> bool {
    if !work.standings.of(half, work.session, work.assets, work.now).quiet() {
        return false;
    }
    match half {
        Half::Tables => match work.standings.files(work.session, work.server, work.now) {
            Ok(files) => !files.outstanding() && files.saved == 0 && files.carried.is_empty(),
            Err(_) => dbcs::carried(&work.session.project).is_empty(),
        },
        _ => true,
    }
}

/// The line under the blocks that names the subjects with nothing to act on.
/// `all` is whether that is every subject.
fn unchanged(ui: &mut egui::Ui, quiet: &[Half], all: bool) {
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let lead = match all {
            true => "This project changes no server row:",
            false => "No changes:",
        };
        ui.label(egui::RichText::new(lead).small().color(theme::INK_FAINT));
        for (at, half) in quiet.iter().enumerate() {
            let name = match at + 1 == quiet.len() {
                true => half.name().to_string(),
                false => format!("{},", half.name()),
            };
            ui.label(egui::RichText::new(name).small().color(theme::INK_DIM))
                .on_hover_text(about(*half));
        }
    });
}

/// A subject's hover text: the tables it writes, the files its statements
/// are in, and what it takes for an applied change to be live.
fn about(half: Half) -> String {
    format!(
        "{}\n\nA save writes the statements to {}; {} holds the statements that revert \
         every applied row.\n\n{}",
        half.tables(),
        half.file(),
        half.revert_file(),
        half.going_live()
    )
}

/// Draws one subject's block. The module comment shows its layout.
fn block(ui: &mut egui::Ui, work: &mut Work<'_>, half: Half, have_a_database: bool, gate: &crate::collab::ServerGate) {
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
            block_body(ui, work, half, have_a_database, gate, &standing, colour);
        });
}

/// The inside of one subject's block: the name and the buttons on one line,
/// the state under them, and then, when there is one, the plan's error, each
/// refusal, the server DBC files' line and the unsaved-table note.
fn block_body(
    ui: &mut egui::Ui,
    work: &mut Work<'_>,
    half: Half,
    have_a_database: bool,
    gate: &crate::collab::ServerGate,
    standing: &Standing,
    colour: egui::Color32,
) {
    let about = about(half);
    // The client-table block also stands for the DBC files the server reads
    // from DataDir\5875\dbc, which Apply copies and Restore puts back.
    let files = match half {
        Half::Tables => Some(work.standings.files(work.session, work.server, work.now).clone()),
        _ => None,
    };
    let (files_outstanding, files_saved) = match &files {
        Some(Ok(files)) => (files.outstanding(), files.saved),
        _ => (false, 0),
    };
    // Apply and Restore are disabled while a write is running, rather than
    // queueing behind it. A second press while the first is still running is
    // almost always a repeat of the same press.
    let busy = work.queue.busy();
    let mut said: Option<Result<String, String>> = None;
    egui::Sides::new().shrink_left().wrap().show(
        ui,
        |ui| {
            ui.label(egui::RichText::new(half.name()).size(theme::BODY).color(theme::INK))
                .on_hover_text(&about);
        },
        |ui| {
            buttons(
                ui,
                work,
                half,
                have_a_database && (standing.appliable() || files_outstanding) && !busy,
                have_a_database && (standing.applied > 0 || files_saved > 0) && !busy,
                have_a_database,
                gate,
                busy,
                standing.changed > 0,
                &mut said,
            );
        },
    );
    // The state has a line of its own under the name and the buttons. Beside
    // them it had about 220 points, and its longest wording wrapped to four
    // lines.
    ui.label(egui::RichText::new(standing.line()).small().color(colour));
    if let Some(trouble) = &standing.trouble {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
    }
    for refused in &standing.refused {
        ui.label(egui::RichText::new(refused).small().color(theme::BAD));
    }
    // The client-table block also stands for the DBC files the server reads
    // from DataDir\5875\dbc, which Apply copies and Restore puts back.
    match &files {
        Some(Ok(files)) => {
            let colour = match files.outstanding() {
                true => theme::WARN,
                false => theme::INK_DIM,
            };
            ui.label(egui::RichText::new(files.line()).small().color(colour))
                .on_hover_text(format!(
                    "The client tables vmangos reads as DBC files: {}. Apply copies the \
                     project's versions into DataDir\\5875\\dbc, first keeping the server's \
                     originals in DataDir\\vale-editor\\originals; Restore copies the \
                     originals back.",
                    match files.carried.is_empty() {
                        true => "none in this project".to_string(),
                        false => files.carried.join(", "),
                    }
                ));
        }
        Some(Err(e)) if !dbcs::carried(&work.session.project).is_empty() => {
            ui.label(
                egui::RichText::new(format!("Server DBC files: {e}"))
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

/// A block's three buttons, laid out from the right: Apply, Restore, then
/// Drop Changes or the note that the subject is edited as a file. A press stores
/// its result in `said`.
#[allow(clippy::too_many_arguments)]
fn buttons(
    ui: &mut egui::Ui,
    work: &mut Work<'_>,
    half: Half,
    can_apply: bool,
    can_put_back: bool,
    have_a_database: bool,
    gate: &crate::collab::ServerGate,
    busy: bool,
    changed: bool,
    said: &mut Option<Result<String, String>>,
) {
    {
        if ui
            .add_enabled(can_apply && gate.writes(), egui::Button::new("Apply"))
            .on_hover_text(format!(
                "Write {} and run it against the world database, after writing the \
                 statements that put every row back into {}.{}",
                half.file(),
                half.revert_file(),
                half.order_note()
            ))
            .on_disabled_hover_text(match (gate.refused.as_deref(), have_a_database, busy) {
                (Some(refused), _, _) => refused,
                (None, false, _) => "No world database is set.",
                (None, true, true) => "A server write is running.",
                (None, true, false) => "This project changes no row of these tables.",
            })
            .clicked()
        {
            *said = Some(apply(half, work));
        }
        if ui
            .add_enabled(can_put_back && gate.writes(), egui::Button::new("Restore"))
            .on_hover_text(format!(
                "Run {}, which restores every row this project has applied to its value \
                 before the project first changed it, then discard the revert record.{}",
                half.revert_file(),
                half.order_note()
            ))
            .on_disabled_hover_text(match (gate.refused.as_deref(), have_a_database, busy) {
                (Some(refused), _, _) => refused,
                (None, false, _) => "No world database is set.",
                (None, true, true) => "A server write is running.",
                (None, true, false) => "This project has applied nothing.",
            })
            .clicked()
        {
            *said = Some(put_back(half, work));
        }
        if half.discardable() {
            if ui
                .add_enabled(changed, egui::Button::new("Drop Changes"))
                .on_hover_text(
                    "Remove every change this project holds for these tables, saved or not, as \
                     one edit that Undo takes back. The database is not changed and the revert \
                     record is kept: Restore still works, and the next Apply restores the applied \
                     rows first. To take back only what is unsaved, use Discard in the Changes \
                     window.",
                )
                .on_disabled_hover_text("This project changes no row of these tables.")
                .clicked()
            {
                *said = Some(Ok(discard(half, work.session)));
            }
        } else {
            ui.label(
                egui::RichText::new("Edited as a file")
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
