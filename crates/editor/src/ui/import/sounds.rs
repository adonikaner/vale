//! The Import Sound dialog, and what the Assets workspace shows of a sound
//! file: its format, its length, its waveform, the `SoundEntries` rows that
//! hold it and a button that plays it. What the import writes, and why, is
//! [`crate::import::sounds`].
//!
//! The shell draws the dialog under any tool, as it draws the texture and
//! model dialogs ([`super::assets::import_dialog`] draws all three). It is
//! opened by the Assets workspace's Import and Re-import, by Import… in a
//! `SoundEntries` reference picker, and by Import sound… at the head of a
//! `SoundEntries` row's form.
//!
//! Its controls, in order: the purpose; the folder, the mono mix and the
//! sample rate; each file with the name it is written under and the format it
//! is written in; and the `SoundEntries` row the import adds.

use std::sync::Arc;

use bevy_egui::egui;

use crate::import::assets::{folder_of, Ask, Assets};
use crate::import::sounds::{rows_naming, SoundImport, SoundPurpose, TABLE};
use crate::session::EditSession;
use crate::tables::browser::Browser;
use crate::tables::sounds::Listen;
use crate::ui::widgets::theme;
use vale_edit::sound::{self, Format};

/// What [`Browser::hearing_file`] holds while the dialog plays its file
/// `index`.
fn heard_key(index: usize) -> String {
    format!("import-sound:{index}")
}

/// The Import Sound dialog, when one is open.
pub(crate) fn sound_dialog(
    ctx: &egui::Context,
    assets: &mut Assets,
    browser: &mut Browser,
    session: &EditSession,
    game: &vale_client::assets::GameAssets,
) {
    let Some(import) = assets.sound.as_mut() else {
        // A preview the dialog started is stopped when the dialog goes.
        if browser.hearing_file.as_deref().is_some_and(|key| key.starts_with("import-sound:")) {
            browser.listen = Some(Listen::Stop);
        }
        return;
    };
    let mut close = false;
    let mut write = false;
    let response = egui::Modal::new(egui::Id::new("import-sound")).show(ctx, |ui| {
        ui.set_width(640.0);
        ui.label(egui::RichText::new("Import Sound").strong().size(15.0));
        let shown = import.files.len().min(3);
        for file in &import.files[..shown] {
            ui.label(egui::RichText::new(file.source.display().to_string()).small().color(theme::INK_DIM));
        }
        if import.files.len() > shown {
            ui.label(
                egui::RichText::new(format!("and {} more", import.files.len() - shown))
                    .small()
                    .color(theme::INK_DIM),
            );
        }
        ui.add_space(6.0);
        // The purpose comes first. An import a table field opened takes its
        // purpose from the field's table and offers no choice.
        match &import.for_field {
            Some(target) => {
                let column = vale_assets::tables::schema::for_table(&target.table)
                    .and_then(|schema| schema.column(target.field))
                    .map_or("the field", |column| column.name);
                ui.label(format!("For {} {column}", target.table));
            }
            None => {
                let mut purpose = import.purpose;
                let options: Vec<(&str, SoundPurpose)> = SoundPurpose::OFFERED.iter().map(|p| (p.name(), *p)).collect();
                ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
                    theme::segmented(ui, &mut purpose, &options, |a, b| a == b);
                });
                if purpose != import.purpose {
                    import.set_purpose(purpose);
                }
            }
        }
        ui.label(egui::RichText::new(import.purpose.hint()).small().color(theme::INK_DIM));
        ui.add_space(6.0);
        egui::Grid::new("import-sound-form")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| form(ui, import));
        ui.add_space(6.0);
        theme::heading(ui, "Files");
        files(ui, import, browser);
        ui.add_space(6.0);
        theme::heading(ui, "SoundEntries row");
        let first = import.vpath(0);
        let playing = rows_playing(ui, session, &first).unwrap_or_default();
        match import.reuses(&playing) {
            true => theme::note(
                ui,
                format!(
                    "SoundEntries {} already holds {first}. The file is written over it and no row \
                     is added.",
                    ids_words(&playing)
                ),
            ),
            false => row(ui, import),
        }
        ui.add_space(6.0);
        match import.files.len() {
            1 => ui.label(format!("Writes {first}")),
            n => ui.label(format!("Writes {n} files into {}", folder_of(&import.folder))),
        };
        let replaced = (0..import.files.len()).map(|f| import.vpath(f)).find(|vpath| session.project.has(vpath));
        let shipped = (0..import.files.len())
            .map(|f| import.vpath(f))
            .find(|vpath| super::assets::in_archives(ui.ctx(), game, vpath));
        if let Some(vpath) = replaced {
            ui.label(
                egui::RichText::new(format!("Replaces the project's file at {vpath}."))
                    .small()
                    .color(theme::WARN),
            );
        } else if let Some(vpath) = shipped {
            ui.label(
                egui::RichText::new(format!(
                    "Overlays the archives' file at {vpath}: every SoundEntries row that holds it plays the import."
                ))
                    .small()
                    .color(theme::WARN),
            );
        }
        let problems = import.problems();
        for problem in &problems {
            ui.label(egui::RichText::new(problem).small().color(theme::BAD));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(problems.is_empty(), egui::Button::new("Import").fill(theme::ACCENT))
                .clicked()
            {
                write = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if response.should_close() {
        close = true;
    }
    if write {
        assets.ask = Some(Ask::ImportSound);
    } else if close {
        assets.sound = None;
    }
}

/// The dialog's form: the folder, the mono mix and the sample rate.
fn form(ui: &mut egui::Ui, import: &mut SoundImport) {
    ui.label(egui::RichText::new("Folder").color(theme::INK_DIM));
    // The field holds only the part of the folder after the purpose's root,
    // as a ground texture's folder field holds the part after `Tileset\`. For
    // Other, and for a folder outside the root copied from a field's row,
    // the field holds the whole folder.
    let root = import.purpose.root();
    let folder = folder_of(&import.folder);
    let under_root = folder.to_ascii_lowercase().starts_with(&root.to_ascii_lowercase());
    match import.purpose != SoundPurpose::Other && under_root {
        true => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(egui::RichText::new(root).color(theme::INK_DIM));
                let mut typed = folder[root.len()..].trim_end_matches('\\').to_string();
                let edited = ui.add(egui::TextEdit::singleline(&mut typed).desired_width(180.0).hint_text("Custom"));
                if edited.changed() {
                    let typed = typed.trim().trim_matches(['\\', '/']);
                    import.folder = match typed.is_empty() {
                        true => root.to_string(),
                        false => format!("{root}{typed}\\"),
                    };
                }
            });
        }
        false => {
            ui.add(egui::TextEdit::singleline(&mut import.folder).desired_width(260.0));
        }
    }
    ui.end_row();
    let converted = (0..import.files.len()).any(|f| !import.kept(f));
    if converted && import.files.iter().any(|f| f.pcm.channels > 1) {
        ui.label(egui::RichText::new("Channels").color(theme::INK_DIM));
        ui.checkbox(&mut import.mono, "Mix to mono").on_hover_text(
            "Mix a stereo file's two channels into one. The game plays a creature's, a spell's or an \
             object's sound from one point in the world, and 6,717 of the 6,845 shipped WAVs are mono.",
        );
        ui.end_row();
    }
    if converted {
        ui.label(egui::RichText::new("Sample rate").color(theme::INK_DIM));
        ui.horizontal(|ui| {
            ui.radio_value(&mut import.rate, sound::RATES[0], "22,050 Hz")
                .on_hover_text("The rate of 6,777 of the 6,845 shipped WAVs.");
            ui.radio_value(&mut import.rate, sound::RATES[1], "44,100 Hz")
                .on_hover_text("The rate of all 194 shipped MP3s, which are the music, and of 68 shipped WAVs.");
        });
        ui.end_row();
    }
}

/// Each file: a button that plays the bytes the import will write, its
/// waveform, the name it is written under, and its length, source format and
/// written format.
fn files(ui: &mut egui::Ui, import: &mut SoundImport, browser: &mut Browser) {
    let mut hear = None;
    egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
        for index in 0..import.files.len() {
            let key = heard_key(index);
            let on = browser.hearing_file.as_deref() == Some(key.as_str());
            ui.horizontal(|ui| {
                if ui
                    .small_button(play_mark(on))
                    .on_hover_text(match on {
                        true => "Stop",
                        false => "Play the file as the import will write it, on the purpose's mixer channel at the row's Volume.",
                    })
                    .clicked()
                {
                    hear = Some((index, on));
                }
                waveform(ui, &import.files[index].peaks, egui::vec2(120.0, 20.0));
                ui.add(egui::TextEdit::singleline(&mut import.files[index].name).desired_width(240.0))
                    .on_hover_text("The name it is written under, without .wav or .mp3.");
            });
            let file = &import.files[index];
            let was = file.format.map_or_else(|| "unknown format".to_string(), |f| f.words());
            let becomes = match import.kept(index) {
                true => "written unchanged".to_string(),
                false if file.format.is_some_and(|f| f.is_shipped_wav()) && import.written_words(index) == was => {
                    "written in the same format".to_string()
                }
                false => format!("written as {}", import.written_words(index)),
            };
            ui.label(
                egui::RichText::new(format!("{} \u{b7} {was} \u{b7} {becomes}", seconds_words(file.pcm.seconds())))
                    .small()
                    .color(theme::INK_DIM),
            );
            ui.add_space(2.0);
        }
    });
    if let Some((index, on)) = hear {
        browser.listen = Some(match on {
            true => Listen::Stop,
            false => Listen::Bytes {
                key: heard_key(index),
                bytes: import.written(index),
                volume: import.entry.volume,
                channel: import.purpose.channel(),
            },
        });
    }
}

/// The `SoundEntries` row the import adds: whether it does, and its values.
fn row(ui: &mut egui::Ui, import: &mut SoundImport) {
    ui.checkbox(&mut import.row, "Add a SoundEntries row").on_hover_text(
        "The game plays a sound file only through a SoundEntries row. The new row's File 1 to \
         File 10 hold the files' names and its Directory their folder, each at Weight 1, so each \
         play picks one file with equal odds. Without a row, the files are written and no row \
         plays them.",
    );
    if !import.row {
        return;
    }
    egui::Grid::new("import-sound-row")
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            let entry = &mut import.entry;
            ui.label(egui::RichText::new("Name").color(theme::INK_DIM));
            ui.add(egui::TextEdit::singleline(&mut entry.name).desired_width(220.0))
                .on_hover_text("The row's Name. The interface's Lua plays a sound by this name, with PlaySound.");
            ui.end_row();
            ui.label(egui::RichText::new("SoundType").color(theme::INK_DIM));
            ui.add(egui::DragValue::new(&mut entry.kind).range(0..=255));
            ui.end_row();
            ui.label(egui::RichText::new("Volume").color(theme::INK_DIM));
            ui.add(egui::DragValue::new(&mut entry.volume).range(0.0..=1.0).speed(0.01).max_decimals(2));
            ui.end_row();
            ui.label(egui::RichText::new("Distances").color(theme::INK_DIM));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut entry.min_distance).range(0.0..=1000.0).speed(0.5).suffix(" yd"))
                    .on_hover_text("Within this distance the sound plays at full volume.");
                ui.label("to");
                ui.add(egui::DragValue::new(&mut entry.cutoff).range(0.0..=1000.0).speed(0.5).suffix(" yd"))
                    .on_hover_text("Beyond this range the sound is not started at all.");
            });
            ui.end_row();
            ui.label(egui::RichText::new("Flags").color(theme::INK_DIM));
            ui.add(egui::DragValue::new(&mut entry.flags).hexadecimal(4, false, true));
            ui.end_row();
        });
    let from = match import.from_field {
        true => "SoundType, Volume, the distances and Flags are copied from a SoundEntries row the field's record names.",
        false => "SoundType, Volume, the distances and Flags are the commonest values among shipped rows of this SoundType.",
    };
    ui.label(egui::RichText::new(from).small().color(theme::INK_DIM));
    if import.for_field.is_some() {
        ui.label(
            egui::RichText::new(
                "The field that opened this dialog is set to the new row's id when the import is written, in the same undo entry.",
            )
                .small()
                .color(theme::INK_DIM),
        );
    }
}

/// The play button's text: a triangle, or a square while it plays.
fn play_mark(on: bool) -> &'static str {
    match on {
        true => "\u{25a0}",
        false => "\u{25b6}",
    }
}

/// Row ids as a list: `9433, 9434`.
pub(super) fn ids_words(ids: &[u32]) -> String {
    ids.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
}

/// A length as a person reads it: `0.84 s`, `2:31`.
pub(in crate::ui) fn seconds_words(seconds: f32) -> String {
    match seconds < 60.0 {
        true => format!("{seconds:.2} s"),
        false => format!("{}:{:02}", (seconds / 60.0) as u32, (seconds % 60.0) as u32),
    }
}

/// A waveform: one vertical bar per peak level, drawn up and down from the
/// centre line.
pub(super) fn waveform(ui: &mut egui::Ui, peaks: &[f32], size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, theme::SUNK);
    if peaks.is_empty() {
        return;
    }
    let step = rect.width() / peaks.len() as f32;
    let half = rect.height() * 0.45;
    for (index, peak) in peaks.iter().enumerate() {
        let x = rect.left() + (index as f32 + 0.5) * step;
        let reach = (peak * half).max(0.5);
        painter.line_segment(
            [egui::pos2(x, rect.center().y - reach), egui::pos2(x, rect.center().y + reach)],
            egui::Stroke::new((step * 0.7).max(1.0), theme::ACCENT),
        );
    }
}

/// A project sound file's format, length and peak levels, as the Assets
/// workspace's details and inspector show them.
#[derive(Debug, Clone)]
pub(super) struct SoundFacts {
    pub(super) format: Option<Format>,
    /// Its length, or why it does not decode.
    pub(super) seconds: Result<f32, String>,
    pub(super) peaks: Vec<f32>,
}

/// The [`SoundFacts`] of the project's sound at `vpath`, decoded once per
/// change of the project's files and kept in egui's memory, because a long
/// track takes a noticeable time to decode.
pub(super) fn facts(ui: &egui::Ui, session: &EditSession, vpath: &str) -> Option<Arc<SoundFacts>> {
    let key = egui::Id::new(("asset-sound-facts", vpath.to_ascii_lowercase(), session.files_revision));
    if let Some(known) = ui.data(|data| data.get_temp::<Option<Arc<SoundFacts>>>(key)) {
        return known;
    }
    let facts = session.project.read(vpath).map(|bytes| {
        let decoded = sound::decode(&bytes);
        Arc::new(SoundFacts {
            format: Format::of(&bytes),
            peaks: decoded.as_ref().map(|pcm| pcm.peaks(crate::import::sounds::PEAKS * 3)).unwrap_or_default(),
            seconds: decoded.map(|pcm| pcm.seconds()).map_err(|e| e.to_string()),
        })
    });
    ui.data_mut(|data| data.insert_temp(key, facts.clone()));
    facts
}

/// The ids of the `SoundEntries` rows that hold the file at `vpath`, searched
/// again when a table changes and kept in egui's memory: the shipped table
/// has 4,623 rows and the details are drawn every frame.
pub(super) fn rows_playing(ui: &egui::Ui, session: &EditSession, vpath: &str) -> Option<Vec<u32>> {
    let key = egui::Id::new(("asset-sound-rows", vpath.to_ascii_lowercase(), session.table_revision));
    if let Some(known) = ui.data(|data| data.get_temp::<Vec<u32>>(key)) {
        return Some(known);
    }
    let rows = rows_naming(session.table(TABLE)?, vpath);
    ui.data_mut(|data| data.insert_temp(key, rows.clone()));
    Some(rows)
}

/// The mixer channel for a sound at `vpath`, chosen from its folder by
/// [`SoundPurpose::of_path`].
pub(super) fn channel_of(vpath: &str) -> vale_client::sound::mixer::Channel {
    SoundPurpose::of_path(vpath).channel()
}

/// A button that plays one of the project's sounds, or stops it while it
/// plays.
pub(super) fn play_button(ui: &mut egui::Ui, browser: &mut Browser, vpath: &str) {
    let on = browser.hearing_file.as_deref().is_some_and(|heard| heard.eq_ignore_ascii_case(vpath));
    let text = match on {
        true => format!("{} Stop", play_mark(true)),
        false => format!("{} Play", play_mark(false)),
    };
    if ui
        .button(text)
        .on_hover_text("Play the file once on the mixer channel its folder gives (music, ambience or effects), at that channel's volume setting.")
        .clicked()
    {
        browser.listen = Some(match on {
            true => Listen::Stop,
            false => Listen::File { path: vpath.to_string(), channel: channel_of(vpath) },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A length is read in seconds below a minute and as minutes and seconds
    /// above.
    #[test]
    fn a_length_in_words() {
        assert_eq!(seconds_words(0.844), "0.84 s");
        assert_eq!(seconds_words(151.2), "2:31");
        assert_eq!(heard_key(2), "import-sound:2");
    }
}
