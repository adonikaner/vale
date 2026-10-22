//! The previews of a row: which tables have one, the stage pane beside a
//! visual's or a kit's fields and inside a dialog, and a display row's
//! picture.

use bevy_egui::egui;

use crate::tables::Modal;
use crate::ui::tables::Workspace;
use crate::ui::tables::storyboard::Show;
use crate::ui::widgets::theme;

/// The inspector's picture of the open row of a display table: the creature,
/// the model or the item, drawn. Nothing for any other table.
///
/// A creature display is drawn as the client resolves it, through the tables
/// the client reads. Those are the project's copies once they are published,
/// which `session::republish::publish_for_the_preview` does a moment after an edit
/// while a display workspace is open, so a new or changed row is drawn as it
/// now is. See `ui::tables::appearance`.
pub fn display_preview(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let Some(table) = super::appearance::display_table(&work.browser.table) else {
        return;
    };
    let Some(record) = work.browser.open else { return };
    let Some(id) = work.session.table(table).and_then(|rows| rows.u32_at(record, 0)) else {
        return;
    };
    // As wide as the inspector, as the item workspace's preview is.
    let side = ui.available_width();
    super::appearance::pane(ui, work, table, id, side);
}

/// The size of the preview inside a picker's dialog, in points.
pub(super) const DIALOG_PREVIEW: egui::Vec2 = egui::vec2(420.0, 440.0);

/// Whether a picker over `points_at` shows the row it is about to choose: the
/// three tables of the spell chain a reference can name, and the display
/// tables. A visual and a kit are played on the stage, an effect's model is
/// shown in the model view, and a display row is drawn as a portrait.
pub(super) fn previews(points_at: &str) -> bool {
    previews_on_stage(points_at) || super::appearance::display_table(points_at).is_some()
}

/// Whether the picker's preview of a row of `points_at` plays on the stage.
/// The other previewing pickers, over the display tables, draw a portrait.
fn previews_on_stage(points_at: &str) -> bool {
    matches!(
        points_at,
        "SpellVisual" | "SpellVisualKit" | "SpellVisualEffectName"
    )
}

/// Whether the open dialog is drawing the stage. The form's own pane then
/// draws a placeholder: the stage is one picture, sized for one rectangle.
pub(super) fn dialog_previews(work: &Workspace<'_>) -> bool {
    match &work.browser.modal {
        Some(Modal::Pick { points_at, .. }) => previews_on_stage(points_at),
        Some(Modal::LooksLike { .. }) => true,
        _ => false,
    }
}

/// A pane with nothing to play: the background and one line saying why.
pub(super) fn empty_pane(ui: &mut egui::Ui, why: &str) {
    let all = ui.available_rect_before_wrap();
    ui.painter().rect_filled(all, 0.0, theme::SHELL);
    ui.painter().text(
        all.center(),
        egui::Align2::CENTER_CENTER,
        why,
        egui::FontId::proportional(theme::SMALL),
        theme::INK_FAINT,
    );
    ui.allocate_rect(all, egui::Sense::hover());
}

/// A `Ui` of exactly [`DIALOG_PREVIEW`] inside a dialog. Both panes fill the
/// rectangle they are given, and a dialog's own `Ui` has no fixed height to
/// fill.
pub(super) fn dialog_pane(ui: &mut egui::Ui) -> egui::Ui {
    let (rect, _) = ui.allocate_exact_size(DIALOG_PREVIEW, egui::Sense::hover());
    ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    )
}

/// What the stage plays for a row of the spell chain, and the pane's title.
///
/// A spell is played as itself. A kit is pushed onto one body. A visual a
/// spell uses is played as the first such spell, which shows its missile and
/// its area as well as its kits; a visual no spell uses has its kits pushed
/// in order. `None` for a row of any other table and for an id that is no
/// row.
pub(super) fn chain_show(work: &mut Workspace<'_>, table: &str, id: u32) -> Option<(Show, String)> {
    match table {
        "Spell" => Some((Show::Spell(id), format!("Preview — spell {id}"))),
        "SpellVisualKit" => Some((
            Show::Pushes(crate::preview::stage::Pushes::kit(id)),
            format!("Preview — kit {id}"),
        )),
        "SpellVisual" => {
            let user = work
                .browser
                .used_by(work.session, "SpellVisual", id)
                .into_iter()
                .find(|at| at.table == "Spell")
                .and_then(|at| work.session.table("Spell")?.u32_at(at.record, 0));
            match user {
                Some(spell) => Some((
                    Show::Spell(spell),
                    format!("Preview — visual {id}, as spell {spell}"),
                )),
                None => crate::preview::stage::Pushes::visual(work.session, id)
                    .map(|pushes| (Show::Pushes(pushes), format!("Preview — visual {id}"))),
            }
        }
        _ => None,
    }
}

/// The preview inside a picker's dialog, for the row that is selected in it.
pub(super) fn preview_of(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    stage: &mut crate::preview::stage::Stage,
    lab: &mut crate::preview::lab::Lab,
    table: &str,
    selected: Option<u32>,
) {
    let Some(id) = selected else {
        stage.close();
        empty_pane(ui, "click a row to preview it");
        return;
    };
    // A display row is drawn as a portrait, as the display pickers elsewhere
    // draw one. See `ui::tables::appearance`.
    if let Some(table) = super::appearance::display_table(table) {
        stage.close();
        let largest = DIALOG_PREVIEW.y - 90.0;
        super::appearance::pane(ui, work, table, id, largest);
        return;
    }
    // An effect is a model and not a cast, so it is shown as the Effects tab
    // shows one: alone, in the model view.
    if table == "SpellVisualEffectName" {
        match work.browser.record_of(work.session, table, id) {
            Some(row) => {
                if !lab.is_open_on(id) {
                    crate::ui::preview::lab::open_alone(lab, stage, work.browser, work.session, row);
                }
                lab.preview = None;
                crate::ui::preview::lab::pane(ui, work, stage, lab);
            }
            None => {
                stage.close();
                empty_pane(ui, "no SpellVisualEffectName row has this id");
            }
        }
        return;
    }
    match chain_show(work, table, id) {
        Some((show, title)) => {
            super::storyboard::stage_pane(ui, stage, Some(show), &title, &[], None)
        }
        None => {
            stage.close();
            empty_pane(ui, "no preview for this row");
        }
    }
}
