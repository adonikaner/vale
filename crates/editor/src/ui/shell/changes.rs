//! The top bar's Changes button and the window it opens: what is unsaved,
//! Save and Discard, the conflicts left to resolve, and the project's
//! history. The view of `crate::collab::changes`.
//!
//! ## The button
//!
//! `Changes` when every edit of this editor's is in a changelist,
//! `Changes · 12` with the count of those that are not, and `Changes · 2
//! conflicts` in the warning colour while conflicts are listed.
//!
//! ## The window
//!
//! Three tabs. Changes lists what is unsaved, each tile, table and the server
//! rows with Discard, then the description, Save and Discard All; a save
//! puts this editor's edits since the last one in a changelist with the
//! description. Edits a crash left out of the files are offered first, with
//! Restore and Discard. A joined editor's tab has Submit and Discard All
//! instead, since the host saves; its Discard All asks the host for its edits
//! in no changelist and takes them back. Under either, the conflicts, each
//! with Reapply and Keep
//! Current. History lists the changelists newest first, each with Show
//! Details and Revert, and above them the edits each author has not put in
//! one, with Take Back for another author who is not connected. A joined
//! editor reads the host's history, asked for once each time
//! the window opens. Targets shows where the project has been written, read
//! from each target's record
//! (`crate::server::targets::Overview`): the rows applied to this machine's
//! world database, the files written to its `DataDir`, and the releases, with
//! buttons that open the Server panel and Publish.

use crate::collab::changes::Tab;
use crate::collab::{Ask, Collab};
use crate::session::EditSession;
use crate::ui::widgets::theme;
use bevy_egui::egui;

/// The window's width, in points.
const WIDTH: f32 = 420.0;

/// The top bar's button. Answers its response, for the window's anchor.
pub(crate) fn button(ui: &mut egui::Ui, collab: &Collab) -> egui::Response {
    let conflicts = collab.changes.conflicts.len();
    let open = collab.open_edits();
    let text = match (conflicts, open) {
        (0, 0) => egui::RichText::new("Changes"),
        (0, open) => egui::RichText::new(format!("Changes \u{b7} {open}")),
        (1, _) => egui::RichText::new("Changes \u{b7} 1 conflict").color(theme::WARN),
        (n, _) => egui::RichText::new(format!("Changes \u{b7} {n} conflicts")).color(theme::WARN),
    };
    ui.button(text).on_hover_text(
        "What is unsaved, with Save and Discard, and the project's history of changelists, each of which can be reverted.",
    )
}

/// What a press in the window asks the caller to do, since the caller holds
/// what each needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asked {
    Nothing,
    /// Save, as the top bar's Save does.
    Save,
    /// Open the Server panel.
    Server,
    /// Open Publish.
    Publish,
}

/// The window's contents.
pub(crate) fn panel(ui: &mut egui::Ui, collab: &mut Collab, session: &mut EditSession) -> Asked {
    ui.set_width(WIDTH);
    ui.label(egui::RichText::new("Changes").strong().size(14.0).color(theme::INK));
    ui.add_space(4.0);
    let mut tab = collab.changes.tab;
    theme::segmented(
        ui,
        &mut tab,
        &[("Changes", Tab::Changes), ("History", Tab::History), ("Targets", Tab::Targets)],
        |a, b| a == b,
    );
    collab.changes.tab = tab;
    ui.add_space(6.0);
    let mut asked = Asked::Nothing;
    match (tab, collab.is_joined()) {
        (Tab::Changes, true) => submit(ui, collab, session),
        (Tab::Changes, false) => {
            if unsaved(ui, collab, session) {
                asked = Asked::Save;
            }
        }
        (Tab::History, _) => history(ui, collab),
        (Tab::Targets, _) => asked = targets(ui, collab),
    }
    if tab == Tab::Changes && !collab.changes.conflicts.is_empty() {
        conflicts(ui, collab);
    }
    asked
}

/// The Targets tab: where the project has been written, as each target's
/// record says.
fn targets(ui: &mut egui::Ui, collab: &Collab) -> Asked {
    let Some((_, overview)) = collab.changes.targets.as_ref() else {
        theme::waiting(ui, "Reading the records\u{2026}");
        return Asked::Nothing;
    };
    let mut asked = Asked::Nothing;
    theme::heading(ui, "World Database");
    match &overview.database {
        Ok(line) => {
            ui.label(egui::RichText::new(line).color(theme::INK));
        }
        Err(why) => theme::note(ui, why.clone()),
    }
    if overview.applied.is_empty() {
        theme::note(ui, "No rows applied.");
    }
    for (subject, rows) in &overview.applied {
        let line = match rows {
            Some(1) => format!("{subject} \u{b7} 1 row applied"),
            Some(n) => format!("{subject} \u{b7} {n} rows applied"),
            None => format!("{subject} \u{b7} revert file unreadable"),
        };
        ui.label(egui::RichText::new(line).color(theme::INK_DIM));
    }
    if ui.button("Open Server Panel").on_hover_text("Apply, Restore and the server's tiles, per subject.").clicked() {
        asked = Asked::Server;
    }
    theme::heading(ui, "Server DataDir");
    match &overview.data_dir {
        Ok(dir) => {
            ui.label(egui::RichText::new(dir).color(theme::INK));
            theme::note(
                ui,
                format!(
                    "{} tile files and {} DBC files written by this project; {} tiles built",
                    overview.tile_files, overview.dbc_files, overview.tiles_built
                ),
            );
        }
        Err(why) => theme::note(ui, why.clone()),
    }
    theme::heading(ui, "Releases");
    match &overview.last_patch {
        Some((name, Some(edit))) => theme::note(ui, format!("Last patch {name}, made from edit {edit}")),
        Some((name, None)) => theme::note(ui, format!("Last patch {name}")),
        None => theme::note(ui, "No patch published."),
    }
    theme::note(
        ui,
        match (&overview.migration, overview.patches) {
            (Some(id), n) => format!("{} \u{b7} last migration {id}", crate::collab::counted(n, "patch", "patches")),
            (None, n) => format!("{} \u{b7} no migration written", crate::collab::counted(n, "patch", "patches")),
        },
    );
    if ui.button("Publish\u{2026}").on_hover_text("Write a patch: the client archive, the server's files and a migration.").clicked() {
        asked = Asked::Publish;
    }
    asked
}

/// The Changes tab alone, hosting or offline: the recovered edits, what is
/// unsaved with Discard, the description, Save and Discard All. Answers
/// whether Save was pressed.
fn unsaved(ui: &mut egui::Ui, collab: &mut Collab, session: &mut EditSession) -> bool {
    let recovered = collab.changes.recovered.len();
    if recovered > 0 {
        let line = match recovered {
            1 => "1 edit in the project's log is not in its files. The editor closed before it was saved.".to_string(),
            n => format!("{n} edits in the project's log are not in its files. The editor closed before they were saved."),
        };
        ui.label(egui::RichText::new(line).color(theme::WARN));
        ui.horizontal(|ui| {
            if ui.button("Restore").on_hover_text("Apply them again. They are then unsaved, as they were.").clicked() {
                collab.ask = Some(Ask::RestoreRecovered);
            }
            if ui
                .button("Discard")
                .on_hover_text("Take back every unsaved edit, these included, so the project stays as its files hold it.")
                .clicked()
            {
                session.discard_asked.push("all".to_string());
            }
        });
        ui.add_space(6.0);
    }
    let subjects = crate::collab::changes::unsaved_subjects(session);
    theme::heading(ui, &format!("Unsaved ({})", subjects.len()));
    if subjects.is_empty() {
        theme::note(ui, "Nothing is unsaved.");
    }
    egui::ScrollArea::vertical().id_salt("changes-unsaved").max_height(200.0).auto_shrink([false, true]).show(ui, |ui| {
        for (key, name) in &subjects {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(name).color(theme::INK));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("Discard")
                        .on_hover_text(format!("Take back every unsaved edit to {name}, so it is as the project's files hold it."))
                        .clicked()
                    {
                        session.discard_asked.push(key.clone());
                    }
                });
            });
        }
    });
    let offline = collab.offline();
    theme::note(
        ui,
        match offline {
            true => "This project is a copy of a host's, offline. Save writes the copy; the edits are kept and sent \
                     to the host at the next Reconnect.",
            false => "Save writes these into the project's files and puts this editor's edits since the last save \
                      in a changelist with this description, listed under History.",
        },
    );
    if !offline {
        ui.add(
            egui::TextEdit::multiline(&mut collab.changes.description)
                .hint_text("What these edits do")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
    }
    ui.add_space(4.0);
    let any = !subjects.is_empty();
    let mut save = false;
    ui.horizontal(|ui| {
        if ui.add_enabled(any, egui::Button::new("Save")).on_hover_text("Ctrl+S.").clicked() {
            save = true;
        }
        if ui
            .add_enabled(any, egui::Button::new("Discard All"))
            .on_hover_text("Take back every unsaved edit, so the project is as its files hold it.")
            .clicked()
        {
            session.discard_asked.push("all".to_string());
        }
    });
    save
}

/// The Changes tab of a joined editor: what it sent since its last Submit,
/// the description, Submit and Discard All.
fn submit(ui: &mut egui::Ui, collab: &mut Collab, session: &mut EditSession) {
    let line = match collab.open_edits() {
        0 => "No edits sent since the last Submit.".to_string(),
        1 => "1 edit sent since the last Submit.".to_string(),
        n => format!("{n} edits sent since the last Submit."),
    };
    ui.label(egui::RichText::new(line).color(theme::INK));
    theme::note(
        ui,
        "The host saves the project. Submit asks the host to put the edits sent since the last Submit in a \
         changelist, with the description, listed under History.",
    );
    ui.add_space(4.0);
    ui.add(
        egui::TextEdit::multiline(&mut collab.changes.description)
            .hint_text("What these edits do")
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    ui.add_space(4.0);
    let any = collab.open_edits() > 0;
    ui.horizontal(|ui| {
        if theme::primary(ui, "Submit").clicked() {
            collab.ask = Some(Ask::Submit);
        }
        if ui
            .add_enabled(any, egui::Button::new("Discard All"))
            .on_hover_text(
                "Take back every edit of this editor's that is in no changelist, so the project is as the host's \
                 files hold it. The edits taken back reach the other editors.",
            )
            .on_disabled_hover_text("No edits sent since the last Submit.")
            .clicked()
        {
            session.discard_asked.push("all".to_string());
        }
    });
}

/// The conflicts, each with its two choices.
fn conflicts(ui: &mut egui::Ui, collab: &mut Collab) {
    theme::heading(ui, &format!("Conflicts ({})", collab.changes.conflicts.len()));
    theme::note(
        ui,
        "Edits made since changed these subjects, so the edits below were not made. Reapply makes them over the \
         current version. Keep Current drops them.",
    );
    let mut ask = None;
    for (at, conflict) in collab.changes.conflicts.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&conflict.subject).color(theme::INK));
                let steps = match conflict.len() {
                    1 => "1 edit".to_string(),
                    n => format!("{n} edits"),
                };
                theme::note(ui, format!("{} \u{b7} {steps}", conflict.source));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Keep Current").on_hover_text("Keep the current version and drop these edits.").clicked() {
                    ask = Some(Ask::KeepCurrent(at));
                }
                if ui
                    .small_button("Reapply")
                    .on_hover_text("Make these edits over the current version, so the subject ends as the edits leave it.")
                    .clicked()
                {
                    ask = Some(Ask::Reapply(at));
                }
            });
        });
    }
    if ask.is_some() {
        collab.ask = ask;
    }
}

/// The History tab.
fn history(ui: &mut egui::Ui, collab: &mut Collab) {
    let joined = collab.is_joined();
    if joined && !collab.changes.asked {
        collab.ask = Some(Ask::History);
    }
    if collab.offline() {
        theme::note(ui, "This project is a copy of a host's, and its history is the host's. Reconnect to see it.");
        return;
    }
    let (changelists, open) = match (joined, collab.log.as_ref(), collab.changes.history.as_ref()) {
        (true, _, Some((_, list))) => (list.clone(), Vec::new()),
        (true, _, None) => {
            theme::waiting(ui, "Asking the host for its history\u{2026}");
            return;
        }
        (false, Some(log), _) => (log.changelists().to_vec(), log.open_counts()),
        (false, None, _) => {
            theme::note(ui, "This project has no log.");
            return;
        }
    };
    if !open.is_empty() {
        theme::heading(ui, "Not Submitted");
        for (author, name, count) in &open {
            let edits = crate::collab::counted(*count, "edit", "edits");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{name} \u{b7} {edits}")).color(theme::INK_DIM));
                // Another author's unsubmitted edits hold what they touch
                // until they submit. One who is not here can have them taken
                // back. See `crate::collab::changes::take_back`.
                if collab.is_me(author) {
                    return;
                }
                let here = collab.connected(author);
                if ui
                    .add_enabled(!here, egui::Button::new("Take Back").small())
                    .on_hover_text(format!(
                        "Close {name}'s unsubmitted edits into a changelist and undo them as an edit of \
                         this editor's. Until then they hold what they touch, and saves leave them out."
                    ))
                    .on_disabled_hover_text(format!("{name} is in the session and submits or undoes these edits."))
                    .clicked()
                {
                    collab.ask = Some(Ask::TakeBack(*author));
                }
            });
        }
    }
    theme::heading(ui, &format!("Changelists ({})", changelists.len()));
    if changelists.is_empty() {
        theme::note(ui, "No changelist has been submitted.");
        return;
    }
    let detail = collab.changes.detail.clone();
    let mut ask = None;
    egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
        for changelist in changelists.iter().rev() {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let description = match changelist.description.is_empty() {
                        true => "No description",
                        false => changelist.description.as_str(),
                    };
                    ui.label(egui::RichText::new(description).color(theme::INK));
                    let edits = crate::collab::counted(changelist.whole_edits, "edit", "edits");
                    theme::note(
                        ui,
                        format!(
                            "{} \u{b7} {} \u{b7} {} \u{b7} {edits}",
                            changelist.number,
                            changelist.author.name,
                            vale_collab::log::date(changelist.time)
                        ),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("Revert")
                        .on_hover_text(
                            "Make the inverse of this changelist's edits as new edits of this editor's. What other \
                             edits changed since is listed as conflicts on the Changes tab.",
                        )
                        .clicked()
                    {
                        ask = Some(Ask::Revert(changelist.number));
                    }
                    let shown = detail.as_ref().is_some_and(|(number, _)| *number == changelist.number);
                    let text = match shown {
                        true => "Hide Details",
                        false => "Show Details",
                    };
                    if ui.small_button(text).on_hover_text("What each edit of this changelist changed.").clicked() {
                        ask = Some(Ask::Detail(changelist.number));
                    }
                });
            });
            if let Some((_, lines)) = detail.as_ref().filter(|(number, _)| *number == changelist.number) {
                match lines {
                    Some(lines) => {
                        for line in lines {
                            let indented = line.starts_with(' ');
                            let text = egui::RichText::new(line.trim_start()).size(theme::SMALL);
                            ui.horizontal(|ui| {
                                ui.add_space(if indented { 18.0 } else { 6.0 });
                                ui.label(match indented {
                                    true => text.color(theme::INK_FAINT),
                                    false => text.color(theme::INK_DIM),
                                });
                            });
                        }
                    }
                    None => theme::waiting(ui, "Asking the host for the edits\u{2026}"),
                }
            }
            ui.add_space(2.0);
        }
    });
    if let Some(ask) = ask {
        if matches!(ask, Ask::Revert(_)) {
            collab.changes.tab = Tab::Changes;
        }
        collab.ask = Some(ask);
    }
}
