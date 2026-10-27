//! The sub-areas of an `AreaTable` row, drawn on its form: the areas that
//! name it as their parent, each a link, and the button that makes one.

use bevy_egui::egui;

use crate::tables::commands::Command;
use crate::tables;
use crate::ui::tables::Workspace;
use crate::ui::tables::commands::run_command;
use crate::ui::tables::reference::follow_reference;
use crate::ui::widgets::theme;

/// The head of an area's form: where an area gets its place and what the
/// server reads of it, and for a zone, the sub-areas inside it.
///
/// The sub-areas are the rows of `AreaTable` whose parent is this row. Each
/// is a link to its own form, and `+ Add a sub-area` makes one. A sub-area's
/// own form shows its zone in the Parent column, which is a link too.
pub(super) fn area_block(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    use tables::zones::area;
    let Some((id, parent)) = work.session.table(area::TABLE).and_then(|areas| {
        Some((areas.u32_at(record, 0)?, areas.u32_at(record, area::PARENT)?))
    }) else {
        return;
    };
    theme::note(
        ui,
        "An area covers the map chunks that carry its id. Paint on map puts the area on \
         the Areas tool's brush. After a restart the server reads the id, map, parent, \
         explore bit, flags, level, name, team and liquid type from area_template; only \
         the client reads the sound columns and the other locales.",
    );
    ui.add_space(4.0);
    if parent != 0 {
        return;
    }
    let rows = tables::zones::sub_areas_of(work.browser, work.session, id);
    let title = match rows.len() {
        0 => "Sub-areas".to_string(),
        n => format!("Sub-areas ({n})"),
    };
    egui::CollapsingHeader::new(egui::RichText::new(title).size(14.0).strong())
        .id_salt("area-sub-areas")
        .default_open(true)
        .show(ui, |ui| {
            if rows.is_empty() {
                theme::note(
                    ui,
                    "No area has this zone as its parent, so the client shows the zone's \
                     name everywhere inside it.",
                );
            }
            let mut follow: Option<u32> = None;
            // A zone has up to 45 sub-areas in the shipped table, so the
            // list scrolls in a fixed height and the columns stay in reach.
            egui::ScrollArea::vertical()
                .id_salt("area-sub-area-rows")
                .max_height(SUB_AREA_ROWS)
                .show(ui, |ui| {
                    for &row in &rows {
                        let label = work.browser.describe(work.session, area::TABLE, row);
                        ui.horizontal(|ui| {
                            let hover = format!("Open area {}.", label.id);
                            if crate::ui::widgets::reference::link(ui, &label.title, theme::ACCENT, &hover) {
                                follow = Some(label.id);
                            }
                            ui.label(theme::number(format!("#{}", label.id)).color(theme::INK_DIM));
                        });
                    }
                });
            if ui
                .button("+ Add a sub-area")
                .on_hover_text(tables::commands::command_about(Command::AddSubArea))
                .clicked()
            {
                let ctx = ui.ctx().clone();
                run_command(&ctx, work, area::TABLE, record, Command::AddSubArea);
            }
            if let Some(id) = follow {
                follow_reference(work, area::TABLE, id);
            }
        });
}

/// The most height a zone's list of sub-areas takes before it scrolls, in
/// points: about eight rows.
const SUB_AREA_ROWS: f32 = 180.0;
