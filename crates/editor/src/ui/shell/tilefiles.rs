//! The *Server tile files* dialog: the files in the live `DataDir`'s `maps\`,
//! `vmaps\` and `mmaps\` that a decision is wanted on, and the buttons that
//! act on them. Opened from the Server panel's *Server tiles* section.
//!
//! A regeneration writes and replaces tile files and removes none (see
//! `vale_mangos::datadir`'s *Nothing is removed by a run*). Every removal and
//! every restoration of a tile file is made here, after a confirmation that
//! names how many files it touches. The dialog has four sections:
//!
//! ```text
//! No longer produced     files a run marked stale: Remove
//! Written by projects    each project's runs, the open one included, and
//!                        whether its folder still exists: Put back originals
//! Removed server files   the server's own files removed here, kept in
//!                        vale-editor\originals\: Put back
//! Check                  a search of the DataDir for files nothing in the
//!                        open project uses: Remove
//! ```
//!
//! A file the editor has never written is the server's own. Remove moves it
//! into `vale-editor\originals\` instead of deleting it, so every removal of
//! such a file can be put back. See `vale_mangos::datadir::ledger`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use bevy_egui::egui;

use crate::ui::widgets::theme;
use vale_mangos::datadir::Ledger;

/// The ledger's name for files the editor removed that it had not written.
const SERVER_OWN: &str = "-";

/// The dialog's state, held across frames in [`super::popover::Popovers`].
#[derive(Default)]
pub(crate) struct TileFiles {
    pub(crate) open: bool,
    /// The `DataDir` and its ledger, read when the dialog opens and after
    /// each action, or why they could not be.
    ledger: Option<Result<(PathBuf, Ledger), String>>,
    /// Projects that have a folder under `Edit\`, read with the ledger.
    folders: Vec<String>,
    /// The stale files ticked for removal.
    stale_chosen: BTreeSet<String>,
    /// The check's search, while it runs.
    checking: Option<Task<Result<Vec<(String, String)>, String>>>,
    /// The check's result.
    found: Option<Result<Vec<(String, String)>, String>>,
    found_chosen: BTreeSet<String>,
    /// The action waiting for its confirmation.
    asked: Option<Asked>,
    /// What the last action did.
    said: String,
}

/// An action that needs a confirmation.
#[derive(Clone, PartialEq, Eq)]
enum Asked {
    Remove(Vec<String>),
    PutBack(String),
}

impl TileFiles {
    /// Open the dialog and read the ledger afresh.
    pub(crate) fn show(&mut self) {
        self.open = true;
        self.ledger = None;
        self.asked = None;
        self.said.clear();
    }

    fn read(&mut self, server: &crate::server::settings::ServerSettings, assets: &vale_client::assets::GameAssets) {
        self.ledger = Some(
            crate::server::datadir::data_dir(server).and_then(|dir| Ledger::open(&dir).map(|ledger| (dir, ledger))),
        );
        self.folders = vale_edit::project::projects(&assets.root);
        let stale: BTreeSet<String> = match &self.ledger {
            Some(Ok((_, ledger))) => ledger.stale_files().into_iter().map(|(key, _)| key).collect(),
            _ => BTreeSet::new(),
        };
        self.stale_chosen.retain(|key| stale.contains(key));
    }
}

/// Draw the dialog when it is open. Returns the rectangle it covers, the
/// whole viewport while it is up, for [`crate::ui::Viewport`]'s floating list.
pub(crate) fn draw(
    ctx: &egui::Context,
    state: &mut TileFiles,
    session: &mut crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    server: &crate::server::settings::ServerSettings,
    queue: &crate::server::queue::ServerQueue,
    standings: &mut crate::server::sync::Standings,
) -> Option<egui::Rect> {
    if !state.open {
        return None;
    }
    if state.ledger.is_none() {
        state.read(server, assets);
    }
    if let Some(task) = state.checking.as_mut() {
        if let Some(found) = block_on(future::poll_once(task)) {
            state.checking = None;
            state.found_chosen.clear();
            state.found = Some(found);
        }
    }
    let busy = queue.busy();
    let response = egui::Modal::new(egui::Id::new("server-tile-files")).show(ctx, |ui| {
        ui.set_width(620.0);
        ui.label(egui::RichText::new("Server tile files").strong().size(14.0).color(theme::INK));
        let (dir, ledger) = match &state.ledger {
            Some(Ok((dir, ledger))) => (dir.clone(), ledger.clone()),
            Some(Err(why)) => {
                theme::note(ui, format!("The server's DataDir could not be read: {why}"));
                if ui.button("Close").clicked() {
                    state.open = false;
                }
                return;
            }
            None => return,
        };
        theme::note(
            ui,
            format!(
                "{}. A regeneration writes and replaces these files and never removes one. \
                 Removing and putting back are done here.",
                dir.display()
            ),
        );
        if busy {
            ui.label(egui::RichText::new("A server write is running; the buttons are off until it ends.").color(theme::WARN));
        }
        egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
            no_longer_produced(ui, state, &ledger, busy);
            written_by_projects(ui, state, &ledger, &session.project.name, busy);
            removed_server_files(ui, state, &ledger, busy);
            check(ui, state, session, assets, &dir, busy);
        });
        confirm(ui, state, standings);
        if !state.said.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&state.said).small().color(theme::INK_DIM));
        }
        ui.add_space(8.0);
        if ui.button("Close").clicked() {
            state.open = false;
        }
    });
    if response.should_close() {
        state.open = false;
    }
    if !state.open {
        state.asked = None;
    }
    Some(ctx.viewport_rect())
}

/// A list of files, each with a tick box and its reason.
fn file_list(ui: &mut egui::Ui, id: &str, files: &[(String, String)], chosen: &mut BTreeSet<String>) {
    egui::Grid::new(id).num_columns(2).spacing([8.0, 2.0]).striped(true).show(ui, |ui| {
        for (key, why) in files {
            let mut on = chosen.contains(key);
            if ui.checkbox(&mut on, key.as_str()).changed() {
                match on {
                    true => chosen.insert(key.clone()),
                    false => chosen.remove(key),
                };
            }
            ui.label(egui::RichText::new(why).small().color(theme::INK_DIM));
            ui.end_row();
        }
    });
}

/// Select all and none, then Remove for the ticked files.
fn remove_row(ui: &mut egui::Ui, files: &[(String, String)], chosen: &mut BTreeSet<String>, busy: bool) -> Option<Asked> {
    let mut asked = None;
    ui.horizontal(|ui| {
        if ui.small_button("All").clicked() {
            chosen.extend(files.iter().map(|(key, _)| key.clone()));
        }
        if ui.small_button("None").clicked() {
            chosen.clear();
        }
        if ui
            .add_enabled(!busy && !chosen.is_empty(), egui::Button::new(format!("Remove ({})", chosen.len())))
            .on_hover_text(
                "Remove the ticked files from the DataDir, after a confirmation. A file the \
                 editor did not write is moved to vale-editor\\originals\\ and can be put back.",
            )
            .clicked()
        {
            asked = Some(Asked::Remove(chosen.iter().cloned().collect()));
        }
    });
    asked
}

fn no_longer_produced(ui: &mut egui::Ui, state: &mut TileFiles, ledger: &Ledger, busy: bool) {
    ui.add_space(8.0);
    theme::heading(ui, "No longer produced");
    let stale = ledger.stale_files();
    if stale.is_empty() {
        theme::note(ui, "No file. A regeneration lists here each file it no longer produces.");
        return;
    }
    theme::note(
        ui,
        "Files a regeneration no longer produces. The server still reads each one while it is \
         there, and the navmesh generator stitches a .map into the tiles beside it. After \
         removing a .map, regenerate its four neighbours (Server files in Edit Map).",
    );
    file_list(ui, "tile-files-stale", &stale, &mut state.stale_chosen);
    if let Some(asked) = remove_row(ui, &stale, &mut state.stale_chosen, busy) {
        state.asked = Some(asked);
    }
}

fn written_by_projects(ui: &mut egui::Ui, state: &mut TileFiles, ledger: &Ledger, open: &str, busy: bool) {
    ui.add_space(10.0);
    theme::heading(ui, "Written by projects");
    let projects = ledger.projects();
    if projects.is_empty() {
        theme::note(ui, "No project's regeneration has written a file here.");
        return;
    }
    theme::note(
        ui,
        "Each project's regenerations, with the files whose last write was that project's. \
         Put back originals replaces each such file with the copy kept before the editor \
         first wrote it, and removes the files that did not exist before.",
    );
    egui::Grid::new("tile-files-projects").num_columns(3).spacing([12.0, 4.0]).show(ui, |ui| {
        for (name, files) in &projects {
            let mark = match (name == open, state.folders.iter().any(|f| f == name)) {
                (true, _) => " (open)",
                (false, true) => "",
                (false, false) => " (deleted)",
            };
            ui.label(format!("{name}{mark}"));
            ui.label(egui::RichText::new(format!("{files} file(s)")).color(theme::INK_DIM));
            if ui
                .add_enabled(!busy, egui::Button::new("Put back originals"))
                .on_hover_text("Restore the DataDir as it was before this project's regenerations, after a confirmation.")
                .clicked()
            {
                state.asked = Some(Asked::PutBack(name.clone()));
            }
            ui.end_row();
        }
    });
}

fn removed_server_files(ui: &mut egui::Ui, state: &mut TileFiles, ledger: &Ledger, busy: bool) {
    let removed = ledger.written_by(SERVER_OWN);
    if removed.is_empty() {
        return;
    }
    ui.add_space(10.0);
    theme::heading(ui, "Removed server files");
    ui.horizontal(|ui| {
        ui.label(format!("{} file(s) the server had before the editor, removed here", removed.len()));
        if ui
            .add_enabled(!busy, egui::Button::new("Put back"))
            .on_hover_text("Move them back from vale-editor\\originals\\, after a confirmation.")
            .clicked()
        {
            state.asked = Some(Asked::PutBack(SERVER_OWN.to_string()));
        }
    });
}

fn check(
    ui: &mut egui::Ui,
    state: &mut TileFiles,
    session: &crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    dir: &std::path::Path,
    busy: bool,
) {
    ui.add_space(10.0);
    theme::heading(ui, "Check");
    theme::note(
        ui,
        "Search the DataDir for files nothing in the open project uses: every file of a map \
         Map.dbc does not list, a .map or .mmtile for a tile the map's WDT does not have, and \
         a model file no tile, tree or game object names. A .vmtile outside the WDT is not \
         listed: the map tools write one for every tile a model's bounds reach.",
    );
    ui.horizontal(|ui| {
        let running = state.checking.is_some();
        if ui.add_enabled(!running, egui::Button::new("Check")).clicked() {
            let reader = crate::server::datadir::TileReader::new(&session.project, assets);
            let dir = dir.to_path_buf();
            state.checking = Some(AsyncComputeTaskPool::get().spawn(async move { reader.orphans(&dir) }));
        }
        if running {
            ui.add(egui::Spinner::new().size(14.0).color(theme::ACCENT));
        }
    });
    let Some(found) = &state.found else {
        return;
    };
    match found {
        Err(why) => theme::note(ui, format!("The check could not finish: {why}")),
        Ok(files) if files.is_empty() => theme::note(ui, "Nothing found."),
        Ok(files) => {
            let files = files.clone();
            file_list(ui, "tile-files-found", &files, &mut state.found_chosen);
            if let Some(asked) = remove_row(ui, &files, &mut state.found_chosen, busy) {
                state.asked = Some(asked);
            }
        }
    }
}

/// The confirmation for the action waiting on one, and the action.
fn confirm(ui: &mut egui::Ui, state: &mut TileFiles, standings: &mut crate::server::sync::Standings) {
    let Some(asked) = state.asked.clone() else {
        return;
    };
    let Some(Ok((_, ledger))) = &mut state.ledger else {
        state.asked = None;
        return;
    };
    ui.add_space(10.0);
    let question = match &asked {
        Asked::Remove(keys) => {
            let own = keys.iter().filter(|key| !ledger.files.contains_key(*key)).count();
            format!(
                "Remove {} file(s) from the DataDir? {} the editor wrote are deleted; {own} the \
                 server had before the editor are moved to vale-editor\\originals\\.",
                keys.len(),
                keys.len() - own
            )
        }
        Asked::PutBack(project) => {
            let files = ledger.written_by(project);
            let kept = files.iter().filter(|(_, original)| *original).count();
            let who = match project.as_str() {
                SERVER_OWN => "the server's own removed files".to_string(),
                name => format!("{name}'s files"),
            };
            format!(
                "Put back {who}? {kept} file(s) are replaced by their kept originals and {} \
                 that did not exist before are removed.",
                files.len() - kept
            )
        }
    };
    ui.label(egui::RichText::new(question).color(theme::WARN));
    let (mut go, mut cancel) = (false, false);
    ui.horizontal(|ui| {
        let verb = match &asked {
            Asked::Remove(_) => "Remove",
            Asked::PutBack(_) => "Put back",
        };
        go = ui.button(egui::RichText::new(verb).color(theme::BAD)).clicked();
        cancel = ui.button("Cancel").clicked();
    });
    if go {
        state.said = match &asked {
            Asked::Remove(keys) => match ledger.remove(keys) {
                Ok(gone) => format!("removed {} file(s)", gone.len()),
                Err(e) => format!("removal stopped: {e}"),
            },
            Asked::PutBack(project) => match ledger.put_back(project) {
                Ok(back) => {
                    let mut said = format!(
                        "put back {} original(s), removed {} file(s)",
                        back.restored.len(),
                        back.removed.len()
                    );
                    if let Some((_, why)) = back.left.first() {
                        said.push_str(&format!("; left {}: {why}", back.left.len()));
                    }
                    said
                }
                Err(e) => format!("put back stopped: {e}"),
            },
        };
        if let Err(e) = ledger.compact() {
            state.said.push_str(&format!("; the ledger could not be rewritten: {e}"));
        }
        state.ledger = None;
        state.found = None;
        state.stale_chosen.clear();
        standings.forget();
    }
    if go || cancel {
        state.asked = None;
    }
}
