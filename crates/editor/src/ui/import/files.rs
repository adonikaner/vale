//! The Game Files part of the Assets workspace: every file the archives
//! list, as a tree of folders, with what each holds, a preview, and Export
//! and Replace. The state is [`crate::import::files`].
//!
//! ## The part's three columns, as in the Project Files part
//!
//! ```text
//! left       which file    the archives' folders as a tree, each opening in
//!                          place under its parent, with the files in each;
//!                          or with words in the search box the paths across
//!                          every folder that hold them all
//! middle     what it is    its path, type and size, a texture's dimensions
//!                          and compression, a model's skeleton and
//!                          sequences, a sound's format, length and the
//!                          SoundEntries rows that hold it, whether the
//!                          project replaces it, and what can be done to it;
//!                          with no file chosen, the chosen folder and
//!                          Export Folder
//! inspector  what it is like  the texture or the model drawn, or the sound's
//!                          waveform and a button that plays it
//! ```
//!
//! Names and paths are drawn as the archives spell them, and looked up in
//! lower case. See `crate::session::files::Spellings`.
//!
//! A table (`.dbc`) is listed and exported as a file. Its rows are edited in
//! the table editor, which Open in Table Editor opens on it
//! ([`crate::ui::shell::asks::ShellAsk::ShowTable`]).
//!
//! The details and the inspector are the Project Files part's own
//! ([`super::assets::content_rows`], [`super::assets::preview`]), reading the
//! archives' copy ([`super::assets::Whose::Archives`]).

use std::sync::Arc;

use bevy_egui::egui;

use super::assets::{
    basename, content_of, content_rows, export_buttons, picture, preview, sequences, size_words, Whose, Work,
};
use super::sounds;
use crate::import::assets::{Ask as ImportAsk, Own};
use crate::import::files::{folder_of, Ask};
use crate::session::files::Spellings;
use crate::ui::shell::asks::ShellAsk;
use crate::ui::subjects::listing;
use crate::ui::widgets::folders::{folder_row, split_folder, INDENT};
use crate::ui::widgets::theme;
use vale_edit::manifest::Kind;

/// What the top of the archives is called when it is the chosen folder.
const ROOT: &str = "Archives";

/// Draw the part into the region the panels left.
pub(crate) fn draw(ui: &mut egui::Ui, mut work: Work<'_>) {
    listing::frame(ui, &mut work, "game-files", list, details);
}

/// What a file is, by its extension, in its row and its details.
fn type_words(vpath: &str) -> String {
    match Kind::of(vpath) {
        Kind::Tile => "map tile".to_string(),
        Kind::Table => "client table".to_string(),
        Kind::Model => "model".to_string(),
        Kind::Minimap => "minimap picture".to_string(),
        Kind::Texture => "texture".to_string(),
        Kind::Sound => "sound".to_string(),
        Kind::Wdt => "map tile index".to_string(),
        _ => match vpath.rsplit_once('.') {
            Some((_, extension)) => format!("{} file", extension.to_ascii_uppercase()),
            None => "file".to_string(),
        },
    }
}

/// A folder's subfolders, each with how many files are under it, and its
/// own files, kept in egui's memory. The archives' listing does not change
/// while the editor runs, so a folder is split once.
fn split(ui: &egui::Ui, catalogue: &[String], here: &str) -> Arc<(Vec<(String, usize)>, Vec<String>)> {
    let key = egui::Id::new(("game-files-split", here.to_string()));
    if let Some(known) = ui.data(|data| data.get_temp::<Arc<(Vec<(String, usize)>, Vec<String>)>>(key)) {
        return known;
    }
    let split = Arc::new(split_folder(catalogue, here));
    ui.data_mut(|data| data.insert_temp(key, Arc::clone(&split)));
    split
}

/// One row of the tree.
#[derive(Debug, Clone)]
enum Row {
    /// A folder: its lower-case path ending in `\`, how many files are under
    /// it, and whether it is open.
    Folder { path: String, count: usize, open: bool, depth: usize },
    /// A file, by its lower-case path.
    File { path: String, depth: usize },
}

/// The tree's rows: each folder, and under an open one its folders and files
/// one level in. Built again when a folder opens or closes, and kept in
/// egui's memory under `GameFiles::open_revision` meanwhile.
fn tree(ui: &egui::Ui, catalogue: &[String], open: &std::collections::BTreeSet<String>, revision: u64) -> Arc<Vec<Row>> {
    let key = egui::Id::new(("game-files-tree", revision));
    if let Some(known) = ui.data(|data| data.get_temp::<Arc<Vec<Row>>>(key)) {
        return known;
    }
    fn walk(ui: &egui::Ui, catalogue: &[String], open: &std::collections::BTreeSet<String>, here: &str, depth: usize, rows: &mut Vec<Row>) {
        let split = split(ui, catalogue, here);
        for (name, count) in &split.0 {
            let path = format!("{here}{name}\\");
            let is_open = open.contains(&path);
            rows.push(Row::Folder { path: path.clone(), count: *count, open: is_open, depth });
            if is_open {
                walk(ui, catalogue, open, &path, depth + 1, rows);
            }
        }
        rows.extend(split.1.iter().map(|path| Row::File { path: path.clone(), depth }));
    }
    let mut rows = Vec::new();
    walk(ui, catalogue, open, "", 0, &mut rows);
    let rows = Arc::new(rows);
    ui.data_mut(|data| data.insert_temp(key, Arc::clone(&rows)));
    rows
}

/// The paths that hold every word of `query`, kept in egui's memory for that
/// query: the listing is too long to search again every frame.
fn found(ui: &egui::Ui, catalogue: &[String], query: &str) -> Arc<Vec<String>> {
    let key = egui::Id::new(("game-files-found", query.to_string()));
    if let Some(known) = ui.data(|data| data.get_temp::<Arc<Vec<String>>>(key)) {
        return known;
    }
    let words: Vec<String> = query.to_ascii_lowercase().split_whitespace().map(str::to_string).collect();
    let hits: Vec<String> = catalogue
        .iter()
        .filter(|path| words.iter().all(|word| path.contains(word.as_str())))
        .cloned()
        .collect();
    let hits = Arc::new(hits);
    ui.data_mut(|data| data.insert_temp(key, Arc::clone(&hits)));
    hits
}

/// One file's row, indented `depth` levels: its picture, its name, and its
/// type, after its folder in a search's hits (`with_folder`), with a note
/// when the project replaces it. Answers whether it was clicked.
fn file_row(
    ui: &mut egui::Ui,
    work: &mut Work<'_>,
    spell: &Spellings,
    vpath: &str,
    replaced: bool,
    depth: usize,
    with_folder: bool,
) -> bool {
    let chosen = work.files.selected.as_deref() == Some(vpath);
    let folder = spell.of(folder_of(vpath)).trim_end_matches('\\');
    let mut sub = match with_folder && !folder.is_empty() {
        true => format!("{folder} \u{b7} {}", type_words(vpath)),
        false => type_words(vpath),
    };
    if replaced {
        sub.push_str(" \u{b7} the project replaces it");
    }
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(depth as f32 * INDENT);
        let shape = theme::list_row(
            ui,
            theme::ListRow {
                title: spell.name_of(vpath),
                sub: &sub,
                trailing: "",
                tint: match replaced {
                    true => theme::ACCENT,
                    false => theme::INK,
                },
                picture: true,
            },
            chosen,
        );
        let own = Own { vpath: vpath.to_string(), kind: Kind::of(vpath), bytes: 0 };
        picture(ui, work, &own, shape.picture);
        clicked = shape.response.clicked();
    });
    clicked
}

/// The left column: the part strip, the search, and the tree, or the
/// search's hits.
fn list(ui: &mut egui::Ui, work: &mut Work<'_>) {
    if let Some(part) = crate::ui::shell::rail::parts(ui, crate::tools::Tool::GameFiles) {
        work.asks.push(ShellAsk::Part(part));
    }
    let catalogue = work.files.catalogue(work.game);
    let own = work.files.own(work.session);
    let spell = work.session.spellings(work.game);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Game Files").strong().size(15.0).color(theme::INK));
        ui.label(theme::number(format!("{} files", catalogue.len())));
    })
    .response
    .on_hover_text(
        "Every file the archive chain lists, by archive path. A file the project also has is \
         read from the project instead; it is marked, and Project Files lists it.",
    );
    ui.add_space(4.0);
    if catalogue.is_empty() {
        theme::note(ui, "The archives list no files. Check that Data\\ holds the 1.12.1 MPQ archives.");
        return;
    }
    let width = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(&mut work.files.query)
            .hint_text("words in the path, in every folder")
            .desired_width(width),
    );
    let height = listing::ROW_HEIGHT;
    let mut clicked: Option<String> = None;
    let query = work.files.query.trim().to_string();
    if !query.is_empty() {
        let hits = found(ui, &catalogue, &query);
        ui.label(egui::RichText::new(format!("{} found", hits.len())).small().color(theme::INK_FAINT));
        ui.add_space(4.0);
        theme::list_area(ui, None, height).show_rows(ui, height, hits.len(), |ui, range| {
            for vpath in &hits[range] {
                if file_row(ui, work, &spell, vpath, own.contains(vpath), 0, true) {
                    clicked = Some(vpath.clone());
                }
            }
        });
    } else {
        let rows = tree(ui, &catalogue, &work.files.open, work.files.open_revision);
        let mut toggled: Option<String> = None;
        // Scrolled to the chosen file once, after Show in Game Files or
        // `--game-file` opened the folders it is in.
        let at = match std::mem::take(&mut work.files.reveal) {
            true => rows
                .iter()
                .position(|row| matches!(row, Row::File { path, .. } if Some(path) == work.files.selected.as_ref())),
            false => None,
        };
        ui.add_space(4.0);
        // One list of rows of one height, so only the rows on screen are
        // drawn and only their files ask for a picture: `Interface\Icons\`
        // alone holds thousands.
        theme::list_area(ui, at, height).show_rows(ui, height, rows.len(), |ui, range| {
            for row in &rows[range] {
                match row {
                    Row::Folder { path, count, open, depth } => {
                        let chosen = work.files.selected.is_none() && work.files.folder == *path;
                        let response = folder_row(ui, spell.name_of(path), *count, *depth, *open, height);
                        if chosen {
                            ui.painter().rect_stroke(
                                response.rect,
                                egui::CornerRadius::same(3),
                                egui::Stroke::new(1.0, theme::ACCENT),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if response.clicked() {
                            toggled = Some(path.clone());
                        }
                    }
                    Row::File { path, depth } => {
                        if file_row(ui, work, &spell, path, own.contains(path), *depth, false) {
                            clicked = Some(path.clone());
                        }
                    }
                }
            }
        });
        if let Some(folder) = toggled {
            work.files.toggle(&folder);
            work.files.folder = folder;
            work.files.selected = None;
        }
    }
    if clicked.is_some() {
        work.files.selected = clicked;
    }
}

/// The middle: the chosen file's path, what it is and holds, and what can be
/// done to it; with no file chosen, the chosen folder and Export Folder.
fn details(ui: &mut egui::Ui, work: &mut Work<'_>) {
    let Some(vpath) = work.files.selected.clone() else {
        folder_details(ui, work);
        return;
    };
    let spell = work.session.spellings(work.game);
    // Shown, copied, exported and replaced as the archives spell it.
    let spelled = spell.of(&vpath).to_string();
    let kind = Kind::of(&vpath);
    let replaced = work.files.own(work.session).contains(&vpath);
    let size = shipped_size(ui, work, &vpath);
    let content = content_of(ui, work, Whose::Archives, kind, &vpath);
    ui.label(egui::RichText::new(basename(&spelled)).strong().size(17.0).color(theme::INK));
    ui.label(egui::RichText::new(&spelled).small().color(theme::INK_DIM));
    ui.add_space(6.0);
    egui::Grid::new("game-file-details")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Type").color(theme::INK_DIM));
            let mut word = type_words(&vpath);
            word[..1].make_ascii_uppercase();
            ui.label(word);
            ui.end_row();
            ui.label(egui::RichText::new("Size").color(theme::INK_DIM));
            match size {
                Some(bytes) => ui.label(size_words(bytes)),
                None => ui.label(egui::RichText::new("does not read").color(theme::BAD)),
            };
            ui.end_row();
            content_rows(ui, work, &content);
            ui.label(egui::RichText::new("Project").color(theme::INK_DIM));
            match replaced {
                true => ui
                    .label(egui::RichText::new("replaces this file").color(theme::ACCENT))
                    .on_hover_text(
                        "The game reads the project's file at this path. The details and Export are the \
                         archives' file; the picture and Play are the project's.",
                    ),
                false => ui.label("does not replace this file"),
            };
            ui.end_row();
        });
    if kind == Kind::Table {
        ui.add_space(4.0);
        theme::note(
            ui,
            "A client table. Its rows are edited in the table editor, and a save writes the \
             project's copy of this file.",
        );
    }
    sequences(ui, work, &content);
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if kind == Kind::Sound {
            sounds::play_button(ui, work.browser, &vpath);
        }
        if kind == Kind::Table {
            let table = crate::tables::tabs::table_named(basename(&vpath).trim_end_matches(".dbc"));
            if ui
                .add_enabled(table.is_some(), egui::Button::new("Open in Table Editor"))
                .on_hover_text("Open this table in the workspace that edits it, or in Tables when none does.")
                .on_disabled_hover_text("The table editor does not know this table's name.")
                .clicked()
            {
                if let Some(table) = table {
                    work.asks.push(ShellAsk::ShowTable(table));
                }
            }
        }
        export_buttons(ui, work, &spelled, false);
        if matches!(kind, Kind::Texture | Kind::Minimap | Kind::Model | Kind::Sound)
            && ui
                .button("Replace\u{2026}")
                .on_hover_text(match kind {
                    Kind::Model => {
                        "Choose a glTF 2.0 model and open Import Model on it, at this path. The project's \
                         file is then read in place of this one."
                    }
                    Kind::Sound => {
                        "Choose a WAV or MP3 and open Import Sound on it, at this path. The project's file \
                         is then read in place of this one, by every SoundEntries row that holds it."
                    }
                    _ => {
                        "Choose a PNG, TGA or BLP picture and open Import Texture on it, at this path. The \
                         project's file is then read in place of this one."
                    }
                })
                .clicked()
        {
            work.assets.ask = Some(ImportAsk::Replace(spelled.clone()));
        }
        if replaced
            && ui
                .button("Show in Project Files")
                .on_hover_text("Open the project's file at this path in the Project Files part.")
                .clicked()
        {
            work.assets.selected = work
                .assets
                .files(work.session)
                .iter()
                .find(|own| own.vpath.eq_ignore_ascii_case(&vpath))
                .map(|own| own.vpath.clone());
            work.asks.push(ShellAsk::Part(crate::tools::Tool::Assets));
        }
        if ui
            .button("Copy Path")
            .on_hover_text("The path as the archives spell it, as a table field or a model names it.")
            .clicked()
        {
            ui.ctx().copy_text(spelled.clone());
            work.session.status = format!("{spelled} is on the clipboard");
        }
    });
}

/// The size of the archives' copy of `vpath`, kept in egui's memory.
fn shipped_size(ui: &egui::Ui, work: &Work<'_>, vpath: &str) -> Option<u64> {
    let key = egui::Id::new(("game-file-size", vpath.to_string()));
    if let Some(known) = ui.data(|data| data.get_temp::<Option<u64>>(key)) {
        return known;
    }
    let size = crate::import::files::shipped(work.game, vpath).map(|bytes| bytes.len() as u64);
    ui.data_mut(|data| data.insert_temp(key, size));
    size
}

/// The chosen folder, with no file chosen: how much is under it, and Export
/// Folder, with the bar of the export running.
fn folder_details(ui: &mut egui::Ui, work: &mut Work<'_>) {
    let folder = work.files.folder.clone();
    let catalogue = work.files.catalogue(work.game);
    let under = catalogue.iter().filter(|path| path.starts_with(&folder)).count();
    let spell = work.session.spellings(work.game);
    let name = match folder.is_empty() {
        true => ROOT.to_string(),
        false => spell.of(&folder).trim_end_matches('\\').to_string(),
    };
    theme::heading(ui, &name);
    theme::note(
        ui,
        format!(
            "{under} files under this folder. Click a folder on the left to open it and choose it, or a \
             file to see it, export it or replace it."
        ),
    );
    ui.add_space(6.0);
    ui.checkbox(&mut work.files.as_png, "Write BLP textures as PNG").on_hover_text(
        "Each BLP texture is written as an 8-bit RGBA PNG of its largest mip, under the same name with \
         .png. Every other file is written byte for byte.",
    );
    let running = work.files.exporting.clone();
    if ui
        .add_enabled(running.is_none() && under > 0, egui::Button::new("Export Folder\u{2026}"))
        .on_hover_text(format!(
            "Choose a directory and write the {under} files under {name} into it at their whole archive \
             paths, so each file's way back in is its own path. The archives' files are written, not the \
             project's."
        ))
        .on_disabled_hover_text(match running {
            Some(_) => "A folder export is running.",
            None => "No file is under this folder.",
        })
        .clicked()
    {
        work.files.ask = Some(Ask::ExportFolder { folder: folder.clone() });
    }
    if let Some((label, progress)) = running {
        ui.add_space(4.0);
        ui.add(egui::ProgressBar::new(progress.fraction()).text(format!(
            "{label}: {} of {}",
            progress.done(),
            progress.total()
        )));
        ui.ctx().request_repaint();
    }
}

/// The part's third column, in the shell's inspector: the chosen file,
/// drawn.
fn inspector(ui: &mut egui::Ui, mut work: Work<'_>) {
    let work = &mut work;
    let Some(vpath) = work.files.selected.clone() else {
        theme::note(ui, "Choose a file to see it.");
        return;
    };
    preview(ui, work, Whose::Archives, &vpath);
}

/// The Game Files part's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, assets, now, .. } = subject;
    inspector(ui, super::assets::work(session, editing, assets, now));
}

/// The Game Files part in the middle of a window.
pub(crate) fn workspace(ui: &mut egui::Ui, _: crate::tools::Tool, bench: crate::ui::workspace::Bench<'_, '_, '_>) {
    let crate::ui::workspace::Bench { session, editing, assets, now, .. } = bench;
    draw(ui, super::assets::work(session, editing, assets, now));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file's type is named by what it is to the game, and any other by its
    /// extension.
    #[test]
    fn a_file_type_is_named_by_its_extension() {
        assert_eq!(type_words("dbfilesclient\\spell.dbc"), "client table");
        assert_eq!(type_words("world\\maps\\azeroth\\azeroth_32_48.adt"), "map tile");
        assert_eq!(type_words("textures\\minimap\\azeroth\\map32_48.blp"), "minimap picture");
        assert_eq!(type_words("interface\\framexml\\ui.xsd"), "XSD file");
        assert_eq!(type_words("readme"), "file");
    }
}
