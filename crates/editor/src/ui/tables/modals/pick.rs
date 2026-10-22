//! The reference picker: the rows of the table a reference column points
//! at, searched and paged, or `SpellIcon`'s icons as a grid; and the picker
//! for a visual, a kit or an effect, which plays the row a press chooses.

use bevy_egui::egui;

use crate::tables::{sounds::Listen, RowLabel};
use crate::tables;
use crate::ui::tables::browser::{row, row_picture, row_pictured};
use crate::ui::tables::previews::{dialog_pane, preview_of, DIALOG_PREVIEW};
use crate::ui::tables::sounds::{plays_sounds, sound_buttons};
use crate::ui::tables::{with_blp, ROW_HEIGHT, Workspace};
use crate::ui::widgets::models::pager;
use crate::ui::widgets::theme;
use super::{PICKER_PAGE, PICKER_WIDTH};

use vale_assets::tables::schema;

/// The icon picker's grid: this many icons across at
/// [`crate::ui::widgets::thumbnails::ICON_SIDE`] points each, this many to a page.
const GRID_COLUMNS: usize = 8;
const GRID_PAGE: usize = 64;

/// The room an icon cell takes beyond the picture, for the hover ring.
const GRID_PAD: f32 = 8.0;

/// The picker for a visual, a kit or an effect: the list on the left and the
/// selected row playing on the right. A press on a row plays it; a double
/// press, Enter or Use writes it to the field. Answers whether the dialog
/// closes.
///
/// A list of labels does not identify these rows: a visual is labelled by the
/// spells that use it and a kit by its models' names, and a person choosing
/// one needs to see what it looks like.
#[allow(clippy::too_many_arguments)]
pub(super) fn pick_previewing(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    stage: &mut crate::preview::stage::Stage,
    lab: &mut crate::preview::lab::Lab,
    table: &str,
    record: usize,
    field: usize,
    points_at: &'static str,
) -> bool {
    if !work.session.open_table(work.assets, points_at) {
        return true;
    }
    let column_name = schema::for_table(table)
        .and_then(|schema| schema.column(field))
        .map(|column| column.name)
        .unwrap_or("this field");
    // The dialog opens on the row the field holds, playing what is chosen now.
    if work.browser.pick_focus {
        work.browser.pick_selected = work
            .session
            .table(table)
            .and_then(|open| open.u32_at(record, field))
            .filter(|id| *id != 0 && *id != u32::MAX);
    }
    let mut close = false;
    let mut chosen: Option<u32> = None;
    // A lighter backdrop than a plain picker's, as the model browser has: the
    // picture is in the dialog and the form behind it does not need to be
    // dimmed far.
    let response = egui::Modal::new(egui::Id::new("data-pick-preview"))
        .backdrop_color(egui::Color32::from_black_alpha(90))
        .show(ui.ctx(), |ui| {
            ui.set_width(PICKER_WIDTH + DIALOG_PREVIEW.x + 16.0);
            ui.label(
                egui::RichText::new(format!("{points_at} for {column_name}"))
                    .strong()
                    .size(14.0),
            );
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(PICKER_WIDTH);
                    let box_ = ui.add(
                        egui::TextEdit::singleline(&mut work.browser.pick_query)
                            .hint_text("name, id, or model name")
                            .desired_width(PICKER_WIDTH),
                    );
                    if work.browser.pick_focus {
                        box_.request_focus();
                        work.browser.pick_focus = false;
                    }
                    let total = work
                        .browser
                        .picks(work.session, points_at)
                        .map(|hits| hits.len())
                        .unwrap_or(0);
                    let range = pager(ui, &mut work.browser.pick_page, total, PICKER_PAGE);
                    let rows: Vec<usize> = work
                        .browser
                        .picks(work.session, points_at)
                        .map(|hits| hits[range.clone()].to_vec())
                        .unwrap_or_default();
                    let page = work.browser.pick_page;
                    egui::ScrollArea::vertical()
                        .id_salt(("pick-preview-rows", page))
                        .max_height(DIALOG_PREVIEW.y - 60.0)
                        .auto_shrink([false, false])
                        .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                            for at in range {
                                let here = rows[at];
                                let label = work.browser.describe(work.session, points_at, here);
                                let lit = work.browser.pick_selected == Some(label.id);
                                let picture = row_picture(work, points_at, here);
                                let pressed = row_pictured(ui, work, &label, picture, lit);
                                if pressed.double_clicked() {
                                    chosen = Some(label.id);
                                } else if pressed.clicked() {
                                    work.browser.pick_selected = Some(label.id);
                                }
                            }
                        });
                });
                let mut pane = dialog_pane(ui);
                let selected = work.browser.pick_selected;
                preview_of(&mut pane, work, stage, lab, points_at, selected);
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let selected = work.browser.pick_selected;
                if ui
                    .add_enabled(selected.is_some(), egui::Button::new("Use"))
                    .on_hover_text("Set the field to the selected row's id.")
                    .on_disabled_hover_text("Select a row first.")
                    .clicked()
                {
                    chosen = selected;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                ui.label(
                    egui::RichText::new(
                        "click previews the row · double-click, Enter or Use chooses it · \
                         Esc closes",
                    )
                    .small()
                    .color(theme::INK_FAINT),
                );
                if selected.is_some() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    chosen = selected;
                }
            });
        });
    if let Some(id) = chosen {
        tables::fields::set_field(
            work.session,
            table,
            record,
            field,
            id,
            &format!("Edit {column_name}"),
            work.now,
        );
        close = true;
    }
    close || response.should_close()
}

/// One cell of the icon grid: the picture at its own size, a ring when the
/// pointer is on it, and the row's name, id and path on hover. The whole
/// cell is the button.
fn icon_cell(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    path: Option<String>,
    label: &RowLabel,
) -> egui::Response {
    let side = crate::ui::widgets::thumbnails::ICON_SIDE as f32;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(side + GRID_PAD, side + GRID_PAD),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(3),
            theme::RAISED,
            egui::Stroke::new(1.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    let inner = rect.shrink(GRID_PAD / 2.0);
    match path.as_deref().and_then(|path| work.picture(path, side)) {
        Some(id) => {
            egui::Image::new(egui::load::SizedTexture::new(id, inner.size())).paint_at(ui, inner);
        }
        None => {
            ui.painter()
                .rect_filled(inner, egui::CornerRadius::same(2), theme::SUNK);
        }
    }
    let title = match label.title.is_empty() {
        true => format!("row {}", label.id),
        false => label.title.clone(),
    };
    response.on_hover_text(match path {
        Some(path) => format!("{title}  #{}\n{path}", label.id),
        None => format!("{title}  #{}", label.id),
    })
}

/// The reference picker over the table `points_at` names, for `field` of
/// `record` in `table`: the matching rows, searched and paged, or the
/// icons as a grid when the target is `SpellIcon`. Answers whether the
/// dialog closes.
pub(super) fn pick(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table: String,
    record: usize,
    field: usize,
    points_at: &'static str,
) -> bool {
    let mut close = false;
    if !work.session.open_table(work.assets, points_at) {
        work.browser.modal = None;
        return false;
    }
    let column_name = schema::for_table(&table)
        .and_then(|schema| schema.column(field))
        .map(|column| column.name)
        .unwrap_or("this field");
    // An icon is chosen by its picture, so the icon table is shown as a
    // grid of pictures at the file's own size rather than as rows with
    // a thumbnail beside a file name.
    let grid = points_at == "SpellIcon";
    let width = match grid {
        true => {
            GRID_COLUMNS as f32 * (crate::ui::widgets::thumbnails::ICON_SIDE as f32 + GRID_PAD + 6.0)
                + 12.0
        }
        false => PICKER_WIDTH,
    };
    let per_page = match grid {
        true => GRID_PAGE,
        false => PICKER_PAGE,
    };
    let sounds = plays_sounds(points_at);
    let mut picked: Option<u32> = None;
    let response = egui::Modal::new(egui::Id::new("data-pick")).show(ui.ctx(), |ui| {
        ui.set_width(width);
        ui.label(
            egui::RichText::new(format!("{points_at} for {column_name}"))
                .strong()
                .size(14.0),
        );
        let box_ = ui.add(
            egui::TextEdit::singleline(&mut work.browser.pick_query)
                .hint_text("name or id")
                .desired_width(width),
        );
        if work.browser.pick_focus {
            box_.request_focus();
            work.browser.pick_focus = false;
        }
        let total = work
            .browser
            .picks(work.session, points_at)
            .map(|hits| hits.len())
            .unwrap_or(0);
        let range = pager(ui, &mut work.browser.pick_page, total, per_page);
        let rows: Vec<usize> = work
            .browser
            .picks(work.session, points_at)
            .map(|hits| hits[range.clone()].to_vec())
            .unwrap_or_default();
        let page = work.browser.pick_page;
        if grid {
            egui::ScrollArea::vertical()
                .id_salt(("pick-grid", page))
                .max_height(
                    GRID_PAGE as f32 / GRID_COLUMNS as f32
                        * (crate::ui::widgets::thumbnails::ICON_SIDE as f32 + GRID_PAD + 6.0),
                )
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                        for here in rows {
                            let label =
                                work.browser.describe(work.session, points_at, here);
                            let path = work
                                .session
                                .table("SpellIcon")
                                .and_then(|icons| icons.string_at(here, 1))
                                .map(|path| with_blp(&path));
                            if icon_cell(ui, work, path, &label).clicked() {
                                picked = Some(label.id);
                            }
                        }
                    });
                });
        } else {
            egui::ScrollArea::vertical()
                .id_salt(("pick-rows", page))
                .max_height(380.0)
                .auto_shrink([false, true])
                .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                    for at in range {
                        let here = rows[at];
                        let label = work.browser.describe(work.session, points_at, here);
                        let pressed = match sounds {
                            true => {
                                ui.horizontal(|ui| {
                                    sound_buttons(
                                        ui, work, &table, points_at, label.id, false,
                                    );
                                    row(ui, work, &label, None, false)
                                })
                                .inner
                            }
                            false => row(ui, work, &label, None, false),
                        };
                        if pressed.clicked() {
                            picked = Some(label.id);
                        }
                    }
                });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                close = true;
            }
            ui.label(
                egui::RichText::new(match sounds {
                    true => {
                        "\u{25b6} plays \u{b7} click a name to choose it \u{b7} Esc \
                         closes \u{b7} PageUp and PageDown turn the pages"
                    }
                    false => "Esc closes · PageUp and PageDown turn the pages",
                })
                .small()
                .color(theme::INK_FAINT),
            );
        });
    });
    if let Some(id) = picked {
        tables::fields::set_field(
            work.session,
            &table,
            record,
            field,
            id,
            &format!("Edit {column_name}"),
            work.now,
        );
        close = true;
    }
    if response.should_close() {
        close = true;
    }
    // A sound heard in the dialog and not chosen would play on with
    // no button left on screen to stop it.
    if close {
        if let Some(heard) = work.browser.hearing {
            if heard.table == points_at && picked != Some(heard.id) {
                work.browser.listen = Some(Listen::Stop);
            }
        }
    }
    close
}
