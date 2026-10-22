//! `Look like a spell…`: a spell's visual chosen by another spell, played on
//! the stage, and either shared or cloned.

use bevy_egui::egui;

use crate::tables;
use crate::ui::tables::browser::row;
use crate::ui::tables::previews::{dialog_pane, empty_pane, DIALOG_PREVIEW};
use crate::ui::tables::storyboard::Show;
use crate::ui::tables::{ROW_HEIGHT, Workspace};
use crate::ui::widgets::models::pager;
use crate::ui::widgets::theme;
use super::{PICKER_PAGE, PICKER_WIDTH};

use vale_assets::tables::schema;

/// The picker that chooses a spell's look by another spell: a search over
/// the spells that have a visual, the selected one playing beside the list,
/// and two ways to take it. Answers whether the dialog closes.
///
/// A visual has no name, so a person does not know which one they want. They
/// know a spell that looks right. Using its visual makes the two spells share
/// one `SpellVisual` row, which is how the ranks of one spell are stored.
/// Cloning it copies the visual with its kits and effects, for a look that
/// starts the same and is then changed.
pub(super) fn looks_like(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    stage: &mut crate::preview::stage::Stage,
    record: usize,
) -> bool {
    use vale_assets::tables::spell::fields::SPELL_VISUAL;
    if !work.session.open_table(work.assets, "Spell") {
        return true;
    }
    if work.browser.pick_focus {
        work.browser.pick_selected = None;
    }
    let own = work
        .session
        .table("Spell")
        .and_then(|spells| spells.u32_at(record, 0))
        .unwrap_or(0);
    let mut close = false;
    // `false` shares the visual and `true` clones it.
    let mut taken: Option<(u32, bool)> = None;
    let response = egui::Modal::new(egui::Id::new("data-looks-like"))
        .backdrop_color(egui::Color32::from_black_alpha(90))
        .show(ui.ctx(), |ui| {
            ui.set_width(PICKER_WIDTH + DIALOG_PREVIEW.x + 16.0);
            ui.label(
                egui::RichText::new(format!("Visual for spell {own} from another spell"))
                    .strong()
                    .size(14.0),
            );
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(PICKER_WIDTH);
                    let box_ = ui.add(
                        egui::TextEdit::singleline(&mut work.browser.pick_query)
                            .hint_text("spell name or id")
                            .desired_width(PICKER_WIDTH),
                    );
                    if work.browser.pick_focus {
                        box_.request_focus();
                        work.browser.pick_focus = false;
                    }
                    // Only a spell with a visual is a look to take.
                    let hits: Vec<usize> = {
                        let all: Vec<usize> = work
                            .browser
                            .picks(work.session, "Spell")
                            .map(|hits| hits.to_vec())
                            .unwrap_or_default();
                        let spells = work.session.table("Spell");
                        all.into_iter()
                            .filter(|&at| {
                                spells
                                    .and_then(|spells| spells.u32_at(at, SPELL_VISUAL))
                                    .is_some_and(|visual| visual != 0 && visual != u32::MAX)
                            })
                            .collect()
                    };
                    let range = pager(ui, &mut work.browser.pick_page, hits.len(), PICKER_PAGE);
                    let rows: Vec<usize> = hits[range].to_vec();
                    let page = work.browser.pick_page;
                    egui::ScrollArea::vertical()
                        .id_salt(("looks-like-rows", page))
                        .max_height(DIALOG_PREVIEW.y - 60.0)
                        .auto_shrink([false, false])
                        .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                            for at in range {
                                let here = rows[at];
                                let label = work.browser.describe(work.session, "Spell", here);
                                let icon = work
                                    .session
                                    .table("Spell")
                                    .and_then(|spells| spells.u32_at(here, schema::SPELL_ICON_FIELD))
                                    .and_then(|icon| work.icon_by_id(icon));
                                let lit = work.browser.pick_selected == Some(label.id);
                                if row(ui, work, &label, icon, lit).clicked() {
                                    work.browser.pick_selected = Some(label.id);
                                }
                            }
                        });
                });
                let mut pane = dialog_pane(ui);
                match work.browser.pick_selected {
                    Some(id) => crate::ui::tables::storyboard::stage_pane(
                        &mut pane,
                        stage,
                        Some(Show::Spell(id)),
                        &format!("Preview — spell {id}"),
                        &[],
                        None,
                    ),
                    None => {
                        stage.close();
                        empty_pane(&mut pane, "click a spell to preview it");
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let visual = work.browser.pick_selected.and_then(|id| {
                    let spells = work.session.table("Spell")?;
                    spells
                        .u32_at(spells.row_of(id)?, SPELL_VISUAL)
                        .filter(|visual| *visual != 0 && *visual != u32::MAX)
                });
                if ui
                    .add_enabled(visual.is_some(), egui::Button::new("Share visual"))
                    .on_hover_text(
                        "Set this spell's SpellVisual to the selected spell's visual. The two \
                         spells then share the row, and an edit to its kits changes both.",
                    )
                    .on_disabled_hover_text("Select a spell first.")
                    .clicked()
                {
                    taken = visual.map(|visual| (visual, false));
                }
                if ui
                    .add_enabled(visual.is_some(), egui::Button::new("Clone visual"))
                    .on_hover_text(
                        "Copy the selected spell's visual with every kit and effect it \
                         references, and set this spell's SpellVisual to the copy. Edits to \
                         the copy change only this spell.",
                    )
                    .on_disabled_hover_text("Select a spell first.")
                    .clicked()
                {
                    taken = visual.map(|visual| (visual, true));
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                ui.label(
                    egui::RichText::new("click previews the spell · Esc closes")
                        .small()
                        .color(theme::INK_FAINT),
                );
            });
        });
    match taken {
        Some((visual, false)) => {
            tables::fields::set_field(
                work.session,
                "Spell",
                record,
                SPELL_VISUAL,
                visual,
                "Edit SpellVisual",
                work.now,
            );
            work.session.status = format!("spell {own} now uses SpellVisual {visual}");
            close = true;
        }
        Some((visual, true)) => {
            if let Some(done) =
                tables::rows::clone_chain(work.session, visual, Some(("Spell", record, SPELL_VISUAL)))
            {
                let copy = done.new_id("SpellVisual", visual).unwrap_or(0);
                work.session.status = format!(
                    "SpellVisual {visual} copied to {copy} with {} kits and {} effects; spell {own} now uses {copy}",
                    done.count("SpellVisualKit"),
                    done.count("SpellVisualEffectName")
                );
            }
            close = true;
        }
        None => {}
    }
    close || response.should_close()
}
