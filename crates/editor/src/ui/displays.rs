//! The dialog a creature's or a game object's display id is chosen in: a
//! paged grid of rendered models over `CreatureDisplayInfo` or
//! `GameObjectDisplayInfo`, a search over the model path, and a large
//! preview of the row under the pointer.
//!
//! The state is [`crate::tools::displays::DisplayPick`]; this file draws it
//! and writes the choice through [`crate::tools::quests::ColumnTarget`]. The
//! item workspace's appearance picker ([`super::items`]) has the same shape
//! over `ItemDisplayInfo`, whose rows are icons and body textures as well as
//! models, so the two are not one dialog.

use bevy_egui::egui;

use super::theme;
use crate::session::EditSession;
use crate::tools::displays::{DisplayPick, Table, PAGE};
use vale_client::assets::GameAssets;

/// A cell of the grid, in points.
const CELL: f32 = 60.0;
/// The gap between cells.
const GAP: f32 = 4.0;
/// How many cells fit across the dialog.
const COLUMNS: usize = 8;
/// The dialog's width: the grid, and the modal's own margins.
const WIDTH: f32 = COLUMNS as f32 * (CELL + GAP) + 24.0;

/// Draw the dialog while one is open, and write the choice.
///
/// `pick` is the tool's slot for it; the dialog closes by taking it. Drawn
/// against the context, so the inspector and the template window both reach
/// it.
pub fn window(
    ctx: &egui::Context,
    pick: &mut Option<DisplayPick>,
    session: &mut EditSession,
    assets: &GameAssets,
    portraits: &mut crate::portraits::Portraits,
    now: f64,
) {
    // Once a pass: the sidebar and the template window both ask, and the
    // second draw of one pass would give egui two widgets with one id.
    let pass = ctx.cumulative_pass_nr();
    let pass_id = egui::Id::new("display-pick-pass");
    if ctx.data(|data| data.get_temp::<u64>(pass_id)) == Some(pass) {
        return;
    }
    ctx.data_mut(|data| data.insert_temp(pass_id, pass));
    let Some(mut open) = pick.take() else {
        return;
    };
    let mut close = false;
    let mut chosen: Option<u32> = None;
    let showing: u32 = session
        .server_edits
        .get(open.target.table, &open.target.key, open.target.column)
        .map(str::to_string)
        .or_else(|| open.target.in_database.clone())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let response = egui::Modal::new(egui::Id::new("display-pick")).show(ctx, |ui| {
        ui.set_width(WIDTH);
        let what = match open.table {
            Table::Creature => "Model for this creature",
            Table::Object => "Model for this object",
        };
        ui.label(egui::RichText::new(what).strong().size(14.0));
        ui.label(
            egui::RichText::new(format!("{} \u{b7} {}", open.target.column, open.table.name()))
                .small()
                .color(theme::INK_DIM),
        );
        let search = ui.add(
            egui::TextEdit::singleline(&mut open.query)
                .hint_text("part of the model path, or an id")
                .desired_width(WIDTH - 24.0),
        );
        if open.focus {
            search.request_focus();
            open.focus = false;
        }

        let total = open.matches(session, assets).len();
        if total == 0 {
            theme::note(
                ui,
                "Nothing matches. A display row carries no name; the text searched is the \
                 model's path and, for a creature, its skin names.",
            );
        }
        let range = super::items::pager(ui, &mut open.page, total, PAGE);
        let page: Vec<u32> = open.matches(session, assets)[range].to_vec();
        let at = open.page;
        egui::ScrollArea::vertical()
            .id_salt(("display-grid", at))
            .max_height(CELL * 5.0 + 24.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
                    for id in page {
                        if cell(ui, &mut open, assets, portraits, id, showing) {
                            chosen = Some(id);
                        }
                    }
                });
            });

        ui.add_space(6.0);
        if let Some(id) = open.preview {
            if preview(ui, &mut open, assets, portraits, id) {
                chosen = Some(id);
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                close = true;
            }
            if ui
                .button("Set to none")
                .on_hover_text(
                    "Write 0. A creature with display 0 is drawn by the server as a box; \
                     an object with display 0 is placed and drawn as a mark.",
                )
                .clicked()
            {
                chosen = Some(0);
            }
            ui.label(
                egui::RichText::new(format!(
                    "{total} of {} row(s) with a model \u{b7} Esc closes",
                    open.facts(session, assets).len()
                ))
                .small()
                .color(theme::INK_FAINT),
            );
        });
    });
    if let Some(id) = chosen {
        open.target.write(session, id.to_string(), now);
        close = true;
    }
    if response.should_close() {
        close = true;
    }
    if !close {
        *pick = Some(open);
    }
}

/// One cell of the grid: the model, and the id under the pointer. `true` when
/// it was clicked.
fn cell(
    ui: &mut egui::Ui,
    open: &mut DisplayPick,
    assets: &GameAssets,
    portraits: &mut crate::portraits::Portraits,
    id: u32,
    showing: u32,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(CELL), egui::Sense::click());
    let on = showing == id;
    if on || response.hovered() {
        ui.painter().rect_filled(
            rect,
            3.0,
            match on {
                true => theme::ACCENT_SUNK,
                false => theme::RAISED,
            },
        );
    }
    if response.hovered() {
        open.preview = Some(id);
    }
    let inner = rect.shrink(3.0);
    match picture_key(open, assets, portraits, id) {
        Some(key) => crate::portraits::paint(ui, portraits, &key, inner),
        None => {
            ui.painter().rect_filled(inner, 2.0, theme::DEAD);
        }
    }
    let hover = match open.path_of(id) {
        Some(path) => format!("{id}\n{path}"),
        None => id.to_string(),
    };
    response.on_hover_text(hover).clicked()
}

/// The key a row's picture is kept under: a creature's model with its skins,
/// or an object's model path. `None` for a row with no model.
///
/// A creature's key is registered with the portrait cache through
/// `want_worn` as it is made. The key alone is not enough: the slot that
/// renders it looks the skins up under the key, and a key it has never been
/// told about falls through to the plain model cache, which cannot open a
/// path with skins after it and marks the row as failed for the session.
fn picture_key(
    open: &mut DisplayPick,
    assets: &GameAssets,
    portraits: &mut crate::portraits::Portraits,
    id: u32,
) -> Option<String> {
    match open.table {
        Table::Creature => {
            let worn = open.worn(assets, id)?;
            Some(portraits.want_worn(&worn))
        }
        Table::Object => open.path_of(id).map(str::to_string),
    }
}

/// The row under the pointer, large, with its path and a button that chooses
/// it. `true` when the button was pressed.
fn preview(
    ui: &mut egui::Ui,
    open: &mut DisplayPick,
    assets: &GameAssets,
    portraits: &mut crate::portraits::Portraits,
    id: u32,
) -> bool {
    let mut used = false;
    ui.horizontal(|ui| {
        let key = match open.table {
            Table::Creature => open.worn(assets, id).map(|worn| portraits.want_large_worn(&worn)),
            Table::Object => open.path_of(id).map(str::to_string),
        };
        match key {
            Some(key) => {
                crate::portraits::paint_large(ui, portraits, &key, 120.0);
            }
            None => {
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(120.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 4.0, theme::SUNK);
            }
        }
        ui.vertical(|ui| {
            ui.label(theme::number(format!("display {id}")));
            match open.path_of(id) {
                Some(path) => {
                    ui.label(egui::RichText::new(path).color(theme::INK).size(13.0));
                    if let Table::Creature = open.table {
                        let skins: Vec<String> = open
                            .worn(assets, id)
                            .map(|worn| worn.skins.into_iter().filter(|s| !s.is_empty()).collect())
                            .unwrap_or_default();
                        if !skins.is_empty() {
                            ui.label(
                                egui::RichText::new(skins.join(", "))
                                    .small()
                                    .color(theme::INK_DIM),
                            );
                        }
                    }
                }
                None if id == 0 => {
                    ui.label(egui::RichText::new("no model").small().color(theme::INK_DIM));
                }
                None => {
                    ui.label(
                        egui::RichText::new("no such display row, or one with no model")
                            .small()
                            .color(theme::BAD),
                    );
                }
            }
            used = ui
                .button("Use this one")
                .on_hover_text("Write this display id into the column, and close.")
                .clicked();
        });
    });
    used
}
