//! A table's rows as a searched, paged list, opened to find one row. A light
//! chosen here is also selected in the world.

use bevy_egui::egui;

use crate::ui::tables::browser::row;
use crate::ui::tables::{ROW_HEIGHT, Workspace};
use crate::ui::widgets::models::pager;
use crate::ui::widgets::theme;
use super::{PICKER_PAGE, PICKER_WIDTH};

/// The list of the rows of `table`, searched and paged; a chosen row is
/// opened. Answers whether the dialog closes.
pub(super) fn rows(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table: &'static str,
) -> bool {
    let mut close = false;
    if !work.session.open_table(work.assets, table) {
        work.browser.modal = None;
        return false;
    }
    let mut picked: Option<u32> = None;
    let response = egui::Modal::new(egui::Id::new("data-rows")).show(ui.ctx(), |ui| {
        ui.set_width(PICKER_WIDTH);
        ui.label(egui::RichText::new(table).strong().size(14.0));
        let box_ = ui.add(
            egui::TextEdit::singleline(&mut work.browser.pick_query)
                .hint_text("map, position, or id")
                .desired_width(PICKER_WIDTH),
        );
        if work.browser.pick_focus {
            box_.request_focus();
            work.browser.pick_focus = false;
        }
        let total = work
            .browser
            .picks(work.session, table)
            .map(|hits| hits.len())
            .unwrap_or(0);
        let range = pager(ui, &mut work.browser.pick_page, total, PICKER_PAGE);
        let rows: Vec<usize> = work
            .browser
            .picks(work.session, table)
            .map(|hits| hits[range.clone()].to_vec())
            .unwrap_or_default();
        let page = work.browser.pick_page;
        egui::ScrollArea::vertical()
            .id_salt(("rows-list", page))
            .max_height(380.0)
            .auto_shrink([false, true])
            .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                for at in range {
                    let here = rows[at];
                    let label = work.browser.describe(work.session, table, here);
                    if row(ui, work, &label, None, false).clicked() {
                        picked = Some(label.id);
                    }
                }
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                close = true;
            }
            ui.label(
                egui::RichText::new("Esc closes · PageUp and PageDown turn the pages")
                    .small()
                    .color(theme::INK_FAINT),
            );
        });
    });
    if let Some(id) = picked {
        work.browser.look_at(table);
        let session = &*work.session;
        work.browser.follow(session, table, id);
        // A light chosen here is also selected in the world, and the
        // camera flies to it. This list is opened to find a light that
        // is not on screen, so the view has to move to show it.
        if table == "Light" {
            if let Some(lights) = work.lights.as_deref_mut() {
                lights.selected = Some(id);
                if let Some(mark) = lights.marks.iter().copied().find(|m| m.id == id) {
                    if !mark.everywhere {
                        lights.fly_to(&mark);
                    }
                }
            }
        }
        close = true;
    }
    if response.should_close() {
        close = true;
    }
    close
}
