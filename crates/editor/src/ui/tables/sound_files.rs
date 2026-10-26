//! The Files section of a `SoundEntries` row's form, drawn in place of the
//! twenty File and Weight columns, the Play and Import sound… buttons at the
//! head of the form, and the dialog that chooses the file for one slot.

use std::sync::Arc;

use bevy_egui::egui;

use crate::tables::Modal;
use crate::tables::sound_files::{self, Slot, TABLE};
use crate::tables::sounds::Listen;
use crate::ui::tables::Workspace;
use crate::ui::widgets::models::pager;
use crate::ui::widgets::theme;
use super::modals::{PICKER_PAGE, PICKER_WIDTH};
use vale_assets::tables::sound::fields::entry;
use vale_edit::sound::Format;

/// The format and length of one file a row names, as the line under the
/// file shows them.
#[derive(Debug, Clone)]
struct FileFacts {
    format: Option<Format>,
    seconds: Option<f32>,
}

/// The format and length of the file at archive path `path`, read from the
/// project or else the archives once per `files_revision` and kept in egui's
/// memory. `None` when neither holds the file. The length is read from the
/// file's headers, so a long track is not decoded.
fn facts(ui: &egui::Ui, work: &Workspace<'_>, path: &str) -> Option<Arc<FileFacts>> {
    let key = egui::Id::new(("sound-file-facts", path.to_ascii_lowercase(), work.session.files_revision));
    if let Some(known) = ui.data(|data| data.get_temp::<Option<Arc<FileFacts>>>(key)) {
        return known;
    }
    let bytes = work.session.project.read(path).or_else(|| (work.assets.reader())(path));
    let facts = bytes.map(|bytes| {
        Arc::new(FileFacts { format: Format::of(&bytes), seconds: vale_edit::sound::seconds_of(&bytes) })
    });
    ui.data_mut(|data| data.insert_temp(key, facts.clone()));
    facts
}

/// The format and length as one line: `WAV, 16-bit PCM, mono, 22,050 Hz ·
/// 0.84 s`.
fn facts_words(facts: &FileFacts) -> String {
    let format = facts.format.map_or("not a WAV or MP3".to_string(), |format| format.words());
    match facts.seconds {
        Some(seconds) => format!("{format} \u{b7} {}", crate::ui::import::sounds::seconds_words(seconds)),
        None => format,
    }
}

/// A button that plays the file at `path` once on `channel`, or stops it
/// while it plays.
fn play_file(ui: &mut egui::Ui, work: &mut Workspace<'_>, path: &str, channel: vale_client::sound::mixer::Channel) {
    let on = work.browser.hearing_file.as_deref().is_some_and(|heard| heard.eq_ignore_ascii_case(path));
    let mark = match on {
        true => "\u{25a0}",
        false => "\u{25b6}",
    };
    if ui.small_button(mark).on_hover_text(format!("{} {path}", if on { "Stop" } else { "Play" })).clicked() {
        work.browser.listen = Some(match on {
            true => Listen::Stop,
            false => Listen::File { path: path.to_string(), channel },
        });
    }
}

/// The buttons at the head of a `SoundEntries` row's form. Play plays one
/// file of the row, picked by weight, at the row's Volume, on the channel
/// [`sound_files::channel_of_type`] gives. Import sound… opens the Import
/// Sound dialog with no field to set.
pub(super) fn head_buttons(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    let Some(table) = work.session.table(TABLE) else { return };
    let Some(id) = table.u32_at(record, 0) else { return };
    let channel = sound_files::channel_of_type(table.u32_at(record, entry::KIND).unwrap_or(0));
    let any = !sound_files::slots(table, record).is_empty();
    let heard = crate::tables::sounds::Heard { table: TABLE, id, night: false };
    let on = work.browser.hearing == Some(heard);
    let text = match on {
        true => "\u{25a0} Stop",
        false => "\u{25b6} Play",
    };
    if ui
        .add_enabled(any, egui::Button::new(text))
        .on_hover_text("Plays one of the row's files, picked by weight, at the row's Volume, on the mixer channel its SoundType selects.")
        .on_disabled_hover_text("Every File column of this row is empty.")
        .clicked()
    {
        work.browser.listen = Some(match on {
            true => Listen::Stop,
            false => Listen::Play { heard, entry: id, channel },
        });
    }
    if ui
        .button("Import sound\u{2026}")
        .on_hover_text("Opens the Import Sound dialog. The WAV or MP3 files are written into the project, a new SoundEntries row holds them, and the row opens here.")
        .clicked()
    {
        work.asks.push(crate::ui::shell::asks::ShellAsk::ImportSound { target: None });
    }
}

/// Open the file dialog on `slot` of the row at `record`.
fn choose(work: &mut Workspace<'_>, record: usize, slot: usize) {
    work.browser.modal = Some(Modal::SoundFile { record, slot });
    work.browser.pick_query.clear();
    work.browser.pick_page = 0;
    work.browser.pick_focus = true;
}

/// The Files section of the row at `record`, drawn in place of the schema's
/// Files section. Each slot that holds a file is a line with a play button,
/// the file name, its Weight, its probability of being picked, Change… and
/// Remove, and a second line with its format and length. + Add file… opens
/// the file dialog on the first empty slot.
pub(super) fn files_block(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    let Some(table) = work.session.table(TABLE) else { return };
    let slots = sound_files::slots(table, record);
    let directory = sound_files::directory(table, record);
    let free = sound_files::free_slot(table, record);
    let channel = sound_files::channel_of_type(table.u32_at(record, entry::KIND).unwrap_or(0));
    let title = match slots.len() {
        0 => "Files".to_string(),
        n => format!("Files ({n})"),
    };
    egui::CollapsingHeader::new(egui::RichText::new(title).size(14.0).strong())
        .id_salt("sound-files")
        .default_open(true)
        .show(ui, |ui| {
            match (slots.len(), directory.is_empty()) {
                (0, _) => theme::note(ui, "Every File column is empty, so the row plays nothing."),
                (_, true) => theme::note(ui, "Each play picks one file by weight. Directory is empty, so each File column holds a whole archive path."),
                (_, false) => theme::note(ui, format!("Each play picks one file by weight. Every file is in the row's Directory, {directory}.")),
            }
            let mut remove: Option<usize> = None;
            let mut change: Option<usize> = None;
            for slot in &slots {
                let path = sound_files::joined(&directory, &slot.file);
                let known = facts(ui, work, &path);
                ui.horizontal(|ui| {
                    play_file(ui, work, &path, channel);
                    // The name in a fixed-width, left-aligned column, so the
                    // Weight fields of the lines are under each other.
                    let width = 220.0_f32.min(ui.available_width() * 0.4);
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, 22.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(width);
                            ui.add(egui::Label::new(egui::RichText::new(&slot.file).color(theme::INK)).truncate())
                                .on_hover_text(&path);
                        },
                    );
                    weight(ui, work, record, slot);
                    ui.label(
                        theme::number(format!("{:.0}%", sound_files::chance(&slots, slot.weight) * 100.0))
                            .color(theme::INK_DIM),
                    )
                    .on_hover_text("This file's Weight over the sum of the row's weights: the share of plays that pick it.");
                    if ui
                        .small_button("Change\u{2026}")
                        .on_hover_text("Opens the file dialog to replace this slot's file. The Weight is kept.")
                        .clicked()
                    {
                        change = Some(slot.at);
                    }
                    if ui
                        .small_button("Remove")
                        .on_hover_text("Clears this slot's File and Weight. The files and weights of the later slots move up one slot.")
                        .clicked()
                    {
                        remove = Some(slot.at);
                    }
                });
                let (words, colour) = match known.as_deref() {
                    Some(facts) => (facts_words(facts), theme::INK_FAINT),
                    None => (format!("{path} is in neither the archives nor the project"), theme::WARN),
                };
                ui.indent(("sound-file-facts", slot.at), |ui| {
                    ui.label(egui::RichText::new(words).small().color(colour));
                });
            }
            ui.horizontal(|ui| {
                let add = ui
                    .add_enabled(free.is_some(), egui::Button::new("+ Add file\u{2026}"))
                    .on_hover_text("Opens the file dialog on the first empty slot: the WAV and MP3 files of the row's Directory, in the archives and the project.")
                    .on_disabled_hover_text("All ten File columns hold a file.");
                if let (true, Some(slot)) = (add.clicked(), free) {
                    choose(work, record, slot);
                }
            });
            if let Some(slot) = change {
                choose(work, record, slot);
            }
            if let Some(slot) = remove {
                sound_files::remove_file(work.session, record, slot);
            }
        });
}

/// A slot's Weight column as a number field, written through
/// `tables::fields::set_field`.
fn weight(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize, slot: &Slot) {
    let mut value = slot.weight;
    if ui
        .add_sized(egui::vec2(52.0, 22.0), egui::DragValue::new(&mut value).range(0..=1000))
        .on_hover_text("The Weight column: this file's share of the picks, against the sum of the row's weights.")
        .changed()
    {
        crate::tables::fields::set_field(
            work.session,
            TABLE,
            record,
            entry::FREQ + slot.at,
            value,
            &format!("Edit Weight {}", slot.at + 1),
            work.now,
        );
    }
}

/// The WAV and MP3 files under `prefix` in the archives and the project,
/// listed once per `files_revision` and kept in egui's memory.
fn sound_files_under(ui: &egui::Ui, work: &Workspace<'_>, prefix: &str) -> Arc<Vec<String>> {
    let key = egui::Id::new(("sound-file-list", prefix.to_string(), work.session.files_revision));
    if let Some(known) = ui.data(|data| data.get_temp::<Arc<Vec<String>>>(key)) {
        return known;
    }
    let listed: Vec<String> = work
        .session
        .listed(work.assets, prefix)
        .into_iter()
        .filter(|path| crate::import::sounds::is_sound_file(std::path::Path::new(path)))
        .collect();
    let listed = Arc::new(listed);
    ui.data_mut(|data| data.insert_temp(key, Arc::clone(&listed)));
    listed
}

/// The file dialog for `slot` of the row at `record`. When another slot
/// holds a file, it lists the WAV and MP3 files in the row's Directory;
/// otherwise it lists every one under `Sound\`. The list is searched and
/// paged, and each line has a play button. Returns whether the dialog closes.
pub(super) fn pick_file(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize, slot: usize) -> bool {
    let Some(table) = work.session.table(TABLE) else { return true };
    let directory = sound_files::directory(table, record);
    let channel = sound_files::channel_of_type(table.u32_at(record, entry::KIND).unwrap_or(0));
    let alone = sound_files::slots(table, record).iter().all(|other| other.at == slot);
    let prefix = match (alone, directory.is_empty()) {
        (false, false) => format!("{}\\", directory.to_ascii_lowercase()),
        _ => "sound\\".to_string(),
    };
    let listed = sound_files_under(ui, work, &prefix);
    let mut close = false;
    let mut picked: Option<String> = None;
    let response = egui::Modal::new(egui::Id::new("sound-file-pick")).show(ui.ctx(), |ui| {
        ui.set_width(PICKER_WIDTH + 160.0);
        ui.label(egui::RichText::new(format!("SoundEntries File {}", slot + 1)).strong().size(14.0));
        theme::note(
            ui,
            match alone {
                true => "No other slot holds a file, so a file from any folder can be chosen. The row's Directory becomes that file's folder.".to_string(),
                false => format!("Every file of a row is in its Directory, so this lists the WAV and MP3 files in {directory}."),
            },
        );
        let search = ui.add(
            egui::TextEdit::singleline(&mut work.browser.pick_query)
                .hint_text("file or folder")
                .desired_width(PICKER_WIDTH + 160.0),
        );
        if work.browser.pick_focus {
            search.request_focus();
            work.browser.pick_focus = false;
        }
        if search.changed() {
            work.browser.pick_page = 0;
        }
        let query = work.browser.pick_query.trim().to_ascii_lowercase();
        let hits: Vec<&String> = listed.iter().filter(|path| path[prefix.len().min(path.len())..].contains(&query)).collect();
        let range = pager(ui, &mut work.browser.pick_page, hits.len(), PICKER_PAGE);
        let page = &hits[range];
        egui::ScrollArea::vertical()
            .id_salt(("sound-file-list", work.browser.pick_page))
            .max_height(380.0)
            .auto_shrink([false, true])
            .show_rows(ui, 24.0, page.len(), |ui, rows| {
                for at in rows {
                    let path = page[at];
                    ui.horizontal(|ui| {
                        play_file(ui, work, path, channel);
                        let shown = path.get(prefix.len()..).unwrap_or(path);
                        if ui.selectable_label(false, shown).on_hover_text(path.as_str()).clicked() {
                            picked = Some(path.clone());
                        }
                    });
                }
            });
        if hits.is_empty() {
            theme::note(ui, "No WAV or MP3 file matches the search.");
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                close = true;
            }
            ui.label(egui::RichText::new("Esc closes").small().color(theme::INK_FAINT));
        });
    });
    if let Some(path) = picked {
        if let Err(why) = sound_files::put_file(work.session, record, slot, &path) {
            work.session.status = why;
        }
        close = true;
    }
    close || response.should_close()
}
