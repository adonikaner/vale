//! A table with no schema, drawn and edited: every field as its number, with
//! a float box where the four bytes read as a float and a text box where
//! they are the offset of a string.

use bevy_egui::egui;

use crate::tables;
use crate::ui::tables::field::number;
use crate::ui::tables::{TEXT, VALUE, Workspace};
use crate::ui::widgets::theme;

/// A table with no schema: every field, numbered, with what it might be.
///
/// Each field is four bytes and is drawn as the number they hold. Where the
/// column holds the offsets of strings (`tables::labels::text_fields`), a text box
/// edits the string; where the bytes read as a float of ordinary size, a
/// second box edits the float. Both are readings of the same four bytes, and the number
/// box always writes them as they are, so a field that is neither can still
/// be set.
pub(super) fn unschemad(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    let table_name = work.browser.table.clone();
    let Some(table) = work.session.table(&table_name) else {
        return;
    };
    let fields = table.field_count();
    theme::note(
        ui,
        "No schema for this table: the columns are numbered. A text box is drawn where \
         the column holds the offsets of strings, and a float box where the four bytes \
         read as a float. Both are guesses about the column. See \
         vale_assets::tables::schema.",
    );
    for field in 0..fields {
        let Some(raw) = work
            .session
            .table(&table_name)
            .and_then(|table| table.u32_at(record, field))
        else {
            continue;
        };
        // Whether the column holds strings is asked of the column, since a
        // small number can be the offset of a string on one row.
        let is_text = work
            .browser
            .text_fields(work.session, &table_name)
            .get(field)
            .copied()
            .unwrap_or(false);
        ui.horizontal(|ui| {
            ui.add_sized(
                egui::vec2(70.0, 22.0),
                egui::Label::new(theme::number(format!("[{field}]"))),
            );
            // Field 0 is the row's id, which only a new row mints.
            if field == 0 {
                ui.label(theme::number(format!("{raw}")));
                ui.label(egui::RichText::new("id").small().color(theme::INK_FAINT));
                return;
            }
            let mut value = raw;
            if number(ui, &mut value).changed() {
                tables::fields::set_field(
                    work.session,
                    &table_name,
                    record,
                    field,
                    value,
                    &format!("Edit field {field}"),
                    work.now,
                );
            }
            if is_text {
                loose_text(ui, work, record, field);
                return;
            }
            let mut float = f32::from_bits(raw);
            if raw != 0 && float.is_finite() && float.abs() > 1.0e-4 && float.abs() < 1.0e6 {
                let response = ui
                    .add_sized(
                        egui::vec2(VALUE, 22.0),
                        egui::DragValue::new(&mut float).speed(0.01),
                    )
                    .on_hover_text("The same four bytes read as a float.");
                if response.changed() {
                    tables::fields::set_field(
                        work.session,
                        &table_name,
                        record,
                        field,
                        float.to_bits(),
                        &format!("Edit field {field}"),
                        work.now,
                    );
                }
                ui.label(egui::RichText::new("f32").small().color(theme::INK_FAINT));
            }
        });
    }
}

/// The text box for a numbered field that holds a string, written when the
/// box loses focus, as a schema's text column is and for the same reason:
/// the string block is append-only.
fn loose_text(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize, field: usize) {
    let table_name = work.browser.table.clone();
    let text = work.browser.buffer(work.session, record, field).clone();
    let mut editing = text.clone();
    let width = TEXT.min((ui.available_width() - 40.0).max(140.0));
    let response = ui.add(
        egui::TextEdit::singleline(&mut editing)
            .desired_width(width)
            .min_size(egui::vec2(width, 24.0)),
    );
    if editing != text {
        *work.browser.buffer(work.session, record, field) = editing.clone();
    }
    if response.lost_focus() {
        let now_in_file = work
            .session
            .table(&table_name)
            .and_then(|table| table.string_at(record, field))
            .unwrap_or_default();
        if editing != now_in_file {
            tables::fields::set_text(
                work.session,
                &table_name,
                record,
                field,
                &editing,
                &format!("Edit field {field}"),
                work.now,
            );
        }
    }
}
