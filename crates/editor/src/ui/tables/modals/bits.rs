//! The mask dialog: one checkbox per bit, with the bit's name and note.

use bevy_egui::egui;

use crate::tables;
use crate::ui::tables::Workspace;
use crate::ui::widgets::theme;
use super::PICKER_WIDTH;

/// How tall the mask dialog's list may get before it scrolls.
///
/// The longest mask has 32 bits, most with a note, which is taller than a
/// window. This height shows about a dozen rows, enough to see a group of bits
/// without the dialog filling the screen.
const BITS_HEIGHT: f32 = 420.0;

/// The mask dialog for `field` of `record` in `table`: one checkbox per
/// bit. Answers whether the dialog closes.
pub(super) fn bits(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table: String,
    record: usize,
    field: usize,
    column: &'static str,
    bits: &'static [(u32, &'static str, &'static str)],
) -> bool {
    let mut close = false;
    // The mask as a list of checkboxes. Every bit gets a row with its
    // name, its note where vmangos wrote one, and its mask, and the
    // value is rebuilt from the checkboxes rather than typed.
    let raw = work
        .session
        .table(&table)
        .and_then(|open| open.u32_at(record, field))
        .unwrap_or(0);
    let mut wanted = raw;
    let response = egui::Modal::new(egui::Id::new("data-bits")).show(ui.ctx(), |ui| {
        ui.set_width(PICKER_WIDTH);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(column).strong().size(14.0));
            ui.label(theme::number(format!("0x{raw:08X}")));
        });
        theme::note(
            ui,
            "Bit names are from vmangos; the 1.12 client data has no table of bit names.",
        );
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .max_height(BITS_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (mask, name, note) in bits {
                    let mut on = wanted & mask != 0;
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut on, *name).changed() {
                            wanted = match on {
                                true => wanted | mask,
                                false => wanted & !mask,
                            };
                        }
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.label(
                                    theme::number(format!("0x{mask:08X}"))
                                        .color(theme::INK_FAINT),
                                );
                            },
                        );
                    });
                    if !note.is_empty() {
                        ui.label(
                            egui::RichText::new(*note).small().color(theme::INK_FAINT),
                        );
                    }
                }
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Close").clicked() {
                close = true;
            }
            // A button clears every bit at once, rather than up to
            // thirty-two clicks; it is the usual way to empty a mask.
            if ui.button("None").on_hover_text("clear every bit").clicked() {
                wanted = 0;
            }
            ui.label(
                egui::RichText::new("Esc closes")
                    .small()
                    .color(theme::INK_FAINT),
            );
        });
    });
    if wanted != raw {
        tables::fields::set_field(
            work.session,
            &table,
            record,
            field,
            wanted,
            &format!("Edit {column}"),
            work.now,
        );
    }
    if response.should_close() {
        close = true;
    }
    close
}
