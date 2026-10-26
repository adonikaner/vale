//! The model browser a model field opens: the models in the archives and the
//! project by folder, Starred and Recent, with the chosen one previewed.

use bevy_egui::egui;

use crate::tables::MODEL_FOLDERS;
use crate::tables;
use crate::ui::tables::Workspace;
use crate::ui::tables::appearance::FileField;
use crate::ui::widgets::models::{model_row, pager, MODEL_ROW};
use crate::ui::widgets::theme;
use super::{PICKER_PAGE, PICKER_WIDTH};

/// The height of the model browser's list: what is left of a 720-tall window
/// after the preview pane under it.
const MODEL_LIST_HEIGHT: f32 = 250.0;

/// The two entries after `MODEL_FOLDERS` on the browser's folder row: the
/// starred models and the recently used ones.
const STARRED_FOLDER: usize = MODEL_FOLDERS.len();
const RECENT_FOLDER: usize = MODEL_FOLDERS.len() + 1;

/// A model path as the display tables write it: with `.mdx`, as every
/// shipped row of `CreatureModelData` and `GameObjectDisplayInfo` does. The
/// readers resolve it to the `.m2` beside it.
fn as_mdx(path: &str) -> String {
    match path.len().checked_sub(3).filter(|&at| path[at..].eq_ignore_ascii_case(".m2")) {
        Some(at) => format!("{}.mdx", &path[..at]),
        None => path.to_string(),
    }
}

/// The model browser for `field` of `record` in the open table
/// (`table_name`): the models by folder, Starred and Recent, with the
/// chosen one previewed. Answers whether the dialog closes.
pub(super) fn models(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table_name: String,
    record: usize,
    field: usize,
) -> bool {
    let mut close = false;
    // An item's model field names a file in one of the folders under
    // `Item\ObjectComponents\`, without the folder, so the browser
    // offers those folders alone. See `ui::tables::appearance::item_model_path`.
    let item = crate::ui::tables::appearance::file_field(&table_name, field) == Some(FileField::ItemModel);
    // Every model the archives and the project list, read again when
    // the project's files change. See `EditSession::listed`.
    let revision = work.session.files_revision;
    if work.browser.models.is_none() || work.browser.models_for != Some(revision) {
        work.browser.models_for = Some(revision);
        work.browser.models = Some(
            work.session
                .listed(work.assets, "")
                .into_iter()
                .filter(|path| path.ends_with(".m2"))
                .collect(),
        );
    }
    // The dialog opens with the field's current model previewed.
    // Otherwise it opened with no preview pane and Use disabled,
    // reading "nothing chosen", even though the row already named a
    // model. The preview is set here rather than where the dialog is
    // opened, because there are two such places: the form's `browse…`
    // and `--browse`.
    if work.browser.model_preview.is_none() {
        let current = work
            .session
            .table(&table_name)
            .and_then(|table| table.string_at(record, field))
            .filter(|declared| !declared.trim().is_empty())
            .and_then(|declared| match item {
                true => crate::ui::tables::appearance::item_model_path(work.assets, &declared)
                    .map(|(_, path)| path.to_ascii_lowercase()),
                false => Some(vale_assets::world::m2::model_path(&declared).to_ascii_lowercase()),
            });
        work.browser.model_preview = current;
    }
    let mut picked: Option<String> = None;
    // A lighter backdrop than a picker's, because the stage behind the
    // dialog is part of it: pressing a row previews that model on the
    // stage, and a dark backdrop would hide the preview.
    let response = egui::Modal::new(egui::Id::new("data-models"))
        .backdrop_color(egui::Color32::from_black_alpha(60))
        .show(ui.ctx(), |ui| {
            ui.set_width(PICKER_WIDTH + 200.0);
            ui.label(egui::RichText::new("Model").strong().size(14.0));
            // The archives' folders, then the person's two lists,
            // starred and recent; see `crate::prefs::favourites`. An item's
            // model has its own folders and is offered no others.
            if !item {
                ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
                    let mut options: Vec<(&str, usize)> = MODEL_FOLDERS
                        .iter()
                        .enumerate()
                        .map(|(at, (name, _))| (*name, at))
                        .collect();
                    options.push(("Starred", STARRED_FOLDER));
                    options.push(("Recent", RECENT_FOLDER));
                    let before = work.browser.model_folder;
                    theme::segmented(ui, &mut work.browser.model_folder, &options, |a, b| a == b);
                    if before != work.browser.model_folder {
                        work.browser.model_page = 0;
                    }
                });
            }
            let box_ = ui.add(
                egui::TextEdit::singleline(&mut work.browser.model_query)
                    .hint_text("words in the model path")
                    .desired_width(ui.available_width()),
            );
            if box_.changed() {
                work.browser.model_page = 0;
            }
            if work.browser.pick_focus {
                box_.request_focus();
                work.browser.pick_focus = false;
            }
            let folder = match item {
                true => "item\\objectcomponents\\",
                false => MODEL_FOLDERS
                    .get(work.browser.model_folder)
                    .map(|(_, prefix)| *prefix)
                    .unwrap_or(""),
            };
            let words: Vec<String> = work
                .browser
                .model_query
                .to_ascii_lowercase()
                .split_whitespace()
                .map(str::to_string)
                .collect();
            let kind = crate::prefs::favourites::Kind::Effect;
            let source: Vec<String> = match work.browser.model_folder {
                STARRED_FOLDER if !item => work.favourites.starred(kind),
                RECENT_FOLDER if !item => work.favourites.recent(kind),
                _ => work
                    .browser
                    .models
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .filter(|path| path.starts_with(folder))
                    .cloned()
                    .collect(),
            };
            let hits: Vec<String> = source
                .into_iter()
                .filter(|path| words.iter().all(|word| path.contains(word.as_str())))
                .collect();
            if hits.is_empty() && words.is_empty() {
                theme::note(
                    ui,
                    match work.browser.model_folder {
                        STARRED_FOLDER => "no starred models: click the star on a row to add one",
                        RECENT_FOLDER => "no recent models: models chosen with Use are listed here",
                        _ => "no models under this folder",
                    },
                );
            }
            let range = pager(ui, &mut work.browser.model_page, hits.len(), PICKER_PAGE);
            let page = work.browser.model_page;
            let previewing = work.browser.model_preview.clone();
            let rows = &hits[range];
            let spell = work.session.spellings(work.assets);
            let portraits = &mut *work.portraits;
            let favourites = &mut *work.favourites;
            let browser = &mut *work.browser;
            egui::ScrollArea::vertical()
                .id_salt(("models", page))
                // Shorter than the previous 380, because the preview
                // pane sits under the list: at 380 the two together
                // pushed the dialog's buttons below the bottom of a
                // 720-tall window.
                .max_height(MODEL_LIST_HEIGHT)
                .auto_shrink([false, true])
                .show_rows(ui, MODEL_ROW, rows.len(), |ui, range| {
                    for at in range {
                        let path = &rows[at];
                        let lit = previewing.as_deref() == Some(path.as_str());
                        let hit = model_row(ui, portraits, favourites, kind, path, spell.of(path), lit);
                        if hit.starred {
                            favourites.toggle(kind, path);
                        }
                        if hit.double {
                            picked = Some(path.clone());
                        } else if hit.clicked {
                            browser.model_preview = Some(path.clone());
                        }
                    }
                });
            // The previewed model, under the list. The stage behind
            // the dialog also shows it, which is why the backdrop is
            // lighter; this pane shows it without looking past the
            // dialog. It is the same pane the placement pickers use.
            if let Some(path) = work.browser.model_preview.clone() {
                ui.add_space(4.0);
                // An item's model is drawn wearing the row's skin for
                // it, which a bare item model does not carry.
                let key = match item {
                    true => crate::ui::tables::appearance::item_model_key(work, &table_name, record, field, &path),
                    false => path,
                };
                crate::ui::widgets::preview::preview_pane(ui, work.portraits, &key);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let chosen = work.browser.model_preview.clone();
                let written = chosen.as_deref().map(crate::ui::tables::appearance::item_model_name);
                let name = match item {
                    true => written.as_deref(),
                    false => chosen.as_deref().map(tables::labels::basename),
                }
                .unwrap_or("nothing chosen");
                if ui
                    .add_enabled(
                        chosen.is_some(),
                        egui::Button::new(egui::RichText::new(format!("Use {name}")).color(
                            egui::Color32::from_rgb(0x0C, 0x16, 0x1E),
                        ))
                        .fill(theme::ACCENT),
                    )
                    .on_hover_text("Set the field to the previewed model's path.")
                    .clicked()
                {
                    picked = chosen;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                ui.label(
                    egui::RichText::new(
                        "click previews the model on the stage · double-click or Use chooses it",
                    )
                    .small()
                    .color(theme::INK_FAINT),
                );
            });
        });
    if let Some(path) = picked {
        work.favourites.used(crate::prefs::favourites::Kind::Effect, &path);
        // Written as the archives spell it, as the shipped rows are.
        let path = work.session.spellings(work.assets).of(&path).to_string();
        let written = match item {
            true => crate::ui::tables::appearance::item_model_name(&path),
            false => match crate::ui::tables::appearance::file_field(&table_name, field) {
                Some(FileField::Model) => as_mdx(&path),
                _ => path,
            },
        };
        tables::fields::set_text(
            work.session,
            &table_name,
            record,
            field,
            &written,
            "Edit Model",
            work.now,
        );
        work.browser.forget_buffers();
        close = true;
    }
    if response.should_close() {
        close = true;
    }
    if close {
        work.browser.model_preview = None;
    }
    close
}
