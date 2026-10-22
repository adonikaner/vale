//! One field of a row, drawn by what its schema column is, and the widgets
//! the field's types share. An `ItemSet` item column is drawn here too.

use bevy_egui::egui;

use crate::tables::Modal;
use crate::tables;
use crate::ui::tables::appearance::FileField;
use crate::ui::tables::commands::field_menu;
use crate::ui::tables::reference::reference;
use crate::ui::tables::{LABEL, TEXT, VALUE, Workspace};
use crate::ui::widgets::theme;

use vale_assets::tables::schema::{Column, Kind};
use vale_assets::tables::schema;

/// A column's value as the file states it: an `int32`, so the `-1` that half
/// the effect columns carry reads as `-1` and not as `4294967295`.
///
/// The cut is at `i32::MAX`: below it the signed and unsigned readings agree,
/// and above it no column in these tables holds a real value, since a row id
/// has at most five digits and a mask is drawn as hex.
pub(super) fn signed(raw: u32) -> String {
    match raw > i32::MAX as u32 {
        true => format!("{}", raw as i32),
        false => format!("{raw}"),
    }
}

/// One field of one record of any open table.
///
/// The table is named because the record is not always a row of the table
/// the browser is on: a spell's form draws fields of the `SkillLineAbility`
/// rows that name the spell. A text column is edited only on its own
/// table's form, since the text buffers are the open row's.
pub(super) fn field_in(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table_name: &str,
    record: usize,
    column: &Column,
) {
    let Some(raw) = work
        .session
        .table(table_name)
        .and_then(|table| table.u32_at(record, column.field))
    else {
        return;
    };

    ui.horizontal(|ui| {
        // The field index and the schema's note go in a tooltip rather than
        // beside each label, where they would add sixty `[117]`-style suffixes
        // down the page.
        let mut tip = format!("field {}", column.field);
        if !column.about.is_empty() {
            tip.push_str("\n\n");
            tip.push_str(column.about);
        }
        // The label is at most [`LABEL`] wide and at most 45% of the available
        // width. [`LABEL`] suits the middle workspace, where the form has 700
        // points. The same form is also drawn in the inspector, where a fixed
        // 210 clipped the resolved name of every reference at the window's
        // edge.
        let label_width = LABEL.min(ui.available_width() * 0.45);
        // The name takes a right-click; see [`field_menu`].
        ui.add_sized(
            egui::vec2(label_width, 22.0),
            egui::Label::new(egui::RichText::new(column.name).color(theme::INK_DIM))
                .truncate()
                .sense(egui::Sense::click()),
        )
        .on_hover_text(tip)
        .context_menu(|ui| field_menu(ui, work, table_name, record, column, raw));
        match column.kind {
            Kind::Id => {
                ui.label(theme::number(format!("{raw}")));
            }
            Kind::Int | Kind::LocaleFlags => {
                let mut value = raw;
                if number(ui, &mut value).changed() {
                    write(work, table_name, record, column, value);
                }
            }
            // `Unused` is drawn signed for the same reason as `reference`. It is
            // the column most likely to hold the `-1` this table writes for
            // "nothing": `SpellVisualKit`'s unused slot holds one on almost
            // every row. An `Int` stays unsigned, because those are counts and
            // durations, and so do the locale flags, which are a mask.
            Kind::Signed | Kind::Unused => {
                let mut value = raw as i32;
                if number(ui, &mut value).changed() {
                    write(work, table_name, record, column, value as u32);
                }
            }
            Kind::Float => {
                let mut value = f32::from_bits(raw);
                let response = ui.add_sized(
                    egui::vec2(VALUE, 22.0),
                    egui::DragValue::new(&mut value).speed(0.01),
                );
                if response.changed() {
                    write(work, table_name, record, column, value.to_bits());
                }
            }
            Kind::Flags(bits) => {
                let mut value = raw;
                if number(ui, &mut value).changed() {
                    write(work, table_name, record, column, value);
                }
                // A button that opens the list of named bits, when the mask
                // has one. Everything that reads a mask reads it bit by bit,
                // so the bits are the value and the number is only their
                // encoding (see `vale_assets::tables::spellbits`). Without the
                // list, setting "castable while dead" required knowing that
                // it is bit 23.
                if !bits.is_empty()
                    && ui
                        .add(
                            egui::Button::new(egui::RichText::new("…").size(13.0))
                                .min_size(egui::vec2(24.0, 22.0)),
                        )
                        .on_hover_text(format!("set or clear the named bits of {}", column.name))
                        .clicked()
                {
                    work.browser.modal = Some(Modal::Bits {
                        table: table_name.to_string(),
                        record,
                        field: column.field,
                        column: column.name,
                        bits,
                    });
                }
                ui.label(theme::number(format!("0x{raw:08X}")));
                // The names of the set bits, shown on the row so they can be
                // read without opening the dialog. The dialog is for changing
                // them.
                let set = schema::named_bits(raw, bits);
                if !set.is_empty() {
                    ui.label(
                        egui::RichText::new(set.join(" · "))
                            .small()
                            .color(theme::INK_DIM),
                    );
                }
                // Set bits with no name are shown rather than hidden. Such a
                // bit is either missing from the list or was typed by hand,
                // and the user needs to see both cases.
                let unnamed = schema::unnamed_bits(raw, bits);
                if unnamed != 0 {
                    ui.label(
                        egui::RichText::new(format!("+0x{unnamed:08X} unnamed"))
                            .small()
                            .color(theme::WARN),
                    );
                }
            }
            Kind::Bool => {
                let mut on = raw != 0;
                if ui.checkbox(&mut on, "").changed() {
                    write(work, table_name, record, column, u32::from(on));
                }
                // A gate with a value other than 0 or 1 shows the value.
                if raw > 1 {
                    ui.label(theme::number(format!("{raw}")).color(theme::WARN));
                }
            }
            Kind::Enum(names) => {
                // A value the list does not name is shown as a number, as
                // the game does. It is shown signed for the same reason as
                // `reference`: these columns are `int32`, and their "nothing
                // here" value is `-1`, which reads as `4294967295` as a `u32`.
                let shown = names
                    .iter()
                    .find(|&&(value, _)| value == raw)
                    .map(|&(_, name)| name.to_string())
                    .unwrap_or_else(|| signed(raw));
                let mut chosen = raw;
                egui::ComboBox::from_id_salt((table_name, record, column.field))
                    .selected_text(egui::RichText::new(shown).size(13.5))
                    .width(TEXT * 0.6)
                    .show_ui(ui, |ui| {
                        for &(value, name) in names {
                            ui.selectable_value(&mut chosen, value, format!("{value}  {name}"));
                        }
                    });
                if chosen != raw {
                    write(work, table_name, record, column, chosen);
                }
            }
            Kind::Colour => {
                let mut rgb = [
                    ((raw >> 16) & 0xff) as u8,
                    ((raw >> 8) & 0xff) as u8,
                    (raw & 0xff) as u8,
                ];
                // The top byte is kept unchanged. It is zero on all but one
                // shipped `LightIntBand` row, and rebuilding the word from the
                // three channels would clear the `0xff` that row carries. See
                // `Kind::Colour`.
                let keep = raw & 0xff00_0000;
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    let packed =
                        (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
                    write(work, table_name, record, column, keep | packed);
                }
                ui.label(
                    theme::number(format!("{} {} {}", rgb[0], rgb[1], rgb[2]))
                        .color(theme::INK_DIM),
                );
                // The packed word as hex, because the channel order is the
                // part of this column a reader is most likely to doubt.
                ui.label(theme::number(format!("0x{raw:06X}")).color(theme::INK_DIM));
            }
            Kind::Reference(points_at) => {
                reference(ui, work, table_name, record, column, points_at, raw)
            }
            Kind::Item => item_cell(ui, work, table_name, record, column, raw),
            // A text column of a row drawn on another table's form is shown
            // and not edited: the buffers hold the open row's text.
            Kind::Text | Kind::Locale(_) if table_name != work.browser.table => {
                let text = work
                    .session
                    .table(table_name)
                    .and_then(|table| table.string_at(record, column.field))
                    .unwrap_or_default();
                ui.label(egui::RichText::new(text).color(theme::INK));
            }
            Kind::Text | Kind::Locale(_) => {
                let wide = column.name.contains("Description");
                let text = work
                    .browser
                    .buffer(work.session, record, column.field)
                    .clone();
                let mut editing = text.clone();
                // Limited to the room left on the row. Beside the lab's pane
                // the form is narrow, and a fixed width ran past its edge. A
                // file field keeps room for its thumbnail and browse button.
                let beside = match super::appearance::file_field(table_name, column.field) {
                    Some(FileField::Texture(_)) => 130.0,
                    Some(_) => 100.0,
                    None => 90.0,
                };
                let width = TEXT.min((ui.available_width() - beside).max(140.0));
                let response = match wide {
                    true => ui.add(
                        egui::TextEdit::multiline(&mut editing)
                            .desired_rows(3)
                            .desired_width(width),
                    ),
                    false => ui.add(
                        egui::TextEdit::singleline(&mut editing)
                            .desired_width(width)
                            .min_size(egui::vec2(width, 24.0)),
                    ),
                };
                if editing != text {
                    *work.browser.buffer(work.session, record, column.field) = editing.clone();
                }
                // Written when the text box loses focus, not on every
                // keystroke. The string block is append-only, so a write per
                // character would leave an unused copy of every prefix of the
                // word in the file. See `vale_edit::dbc`.
                if response.lost_focus() {
                    let now_in_file = work
                        .session
                        .table(table_name)
                        .and_then(|table| table.string_at(record, column.field))
                        .unwrap_or_default();
                    if editing != now_in_file {
                        tables::fields::set_text(
                            work.session,
                            table_name,
                            record,
                            column.field,
                            &editing,
                            &format!("Edit {}", column.name),
                            work.now,
                        );
                    }
                }
                // A model path is checked against the archives as it is
                // typed, since a path that opens nothing draws nothing.
                if table_name == "SpellVisualEffectName"
                    && column.field == vale_assets::tables::spell::fields::EFFECT_MODEL
                {
                    let id = work
                        .session
                        .table(table_name)
                        .and_then(|table| table.u32_at(record, 0))
                        .unwrap_or(0);
                    presence(ui, work.model_present(id));
                    if ui
                        .small_button("browse…")
                        .on_hover_text("search every model in the archives by path")
                        .clicked()
                    {
                        work.browser.modal = Some(Modal::Models {
                            record,
                            field: column.field,
                        });
                        work.browser.model_query.clear();
                        work.browser.pick_focus = true;
                    }
                }
                // A display table's model and texture fields are chosen by
                // looking at them. See `ui::tables::appearance`.
                match super::appearance::file_field(table_name, column.field) {
                    Some(FileField::Model | FileField::ItemModel) => {
                        if ui
                            .small_button("browse\u{2026}")
                            .on_hover_text(
                                "Search every model in the archives and the project by path.",
                            )
                            .clicked()
                        {
                            work.browser.modal = Some(Modal::Models {
                                record,
                                field: column.field,
                            });
                            work.browser.model_query.clear();
                            work.browser.pick_focus = true;
                        }
                    }
                    Some(FileField::Texture(of)) => {
                        super::appearance::texture_field(
                            ui,
                            work,
                            table_name,
                            record,
                            column.field,
                            of,
                            &text,
                        );
                    }
                    None => {}
                }
            }
        }
    });
}

/// An item column: the entry, a picker over `item_template`, and the item's
/// name as a link to the item workspace.
///
/// The name and the picker are the item form's, read from the world
/// database through the quest tool's cache. Without a database the entry is
/// drawn alone, since nothing in the archives names an item.
fn item_cell(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table_name: &str,
    record: usize,
    column: &Column,
    raw: u32,
) {
    use crate::subjects::quests::{PickFor, Picker, SetFollow, Target};
    let mut value = raw;
    if number(ui, &mut value).changed() {
        write(work, table_name, record, column, value);
    }
    let table = table_name.to_string();
    // The set this row is, when the column is one of a set's items.
    let set = (table_name == "ItemSet")
        .then(|| work.session.table(table_name)?.u32_at(record, 0))
        .flatten();
    let Some(quests) = work.quests.as_deref_mut() else {
        ui.label(egui::RichText::new("item").small().color(theme::INK_FAINT))
            .on_hover_text(
                "An item_template entry. No world database is connected, so its name cannot be read.",
            );
        return;
    };
    if ui
        .add(
            egui::Button::new(egui::RichText::new("…").size(13.0)).min_size(egui::vec2(24.0, 22.0)),
        )
        .on_hover_text("choose an item by name")
        .clicked()
    {
        quests.picker = Some(Picker::new(
            Target::Item,
            PickFor::TableField {
                table,
                record,
                field: column.field,
                column: column.name,
            },
        ));
    }
    if raw == 0 {
        ui.label(egui::RichText::new("none").color(theme::INK_FAINT));
        return;
    }
    crate::ui::widgets::reference::name(
        ui,
        &mut crate::ui::widgets::reference::Resolver {
            session: work.session,
            assets: work.assets,
            quests,
        },
        vale_mangos::item::TEMPLATE,
        raw,
    );
    // The other half of the membership: the item's own `set_id`, which is
    // what the server counts set pieces by. An item picked through the
    // dialog has it written for it; one typed as a number, or one the
    // shipped data disagrees about, is marked here with a button.
    let Some(set) = set else {
        return;
    };
    let Some(found) = quests.item(raw, &work.session.server_edits) else {
        return;
    };
    let pending = quests.set_follows.iter().any(|follow| follow.joined == raw);
    if found.set_id != set && !pending {
        ui.label(
            egui::RichText::new(match found.set_id {
                0 => "item set_id is 0".to_string(),
                other => format!("item set_id is {other}"),
            })
            .small()
            .color(theme::WARN),
        )
        .on_hover_text(
            "This set lists the item, but the item's item_template.set_id does not name this \
             set. The server counts set pieces by item_template.set_id, so the item does not \
             count towards the set bonuses until set_id matches.",
        );
        if ui
            .small_button("write set_id")
            .on_hover_text(format!("Write set_id {set} on this item's item_template row. One undo entry."))
            .clicked()
        {
            quests.set_follows.push(SetFollow {
                set,
                left: 0,
                joined: raw,
            });
        }
    }
}

/// A dot saying whether the archives hold a model, with the reason on hover.
pub(super) fn presence(ui: &mut egui::Ui, present: Option<bool>) {
    let (colour, why) = match present {
        Some(true) => (theme::GOOD, "model file found in the archives"),
        Some(false) => (theme::BAD, "no such model file in the archives; this effect draws nothing"),
        None => (theme::INK_FAINT, "SpellVisualEffectName is not open yet"),
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, colour);
    response.on_hover_text(why);
}

/// A number field at the common width.
pub(super) fn number<T: egui::emath::Numeric>(ui: &mut egui::Ui, value: &mut T) -> egui::Response {
    ui.add_sized(egui::vec2(VALUE, 22.0), egui::DragValue::new(value))
}

pub(super) fn write(work: &mut Workspace<'_>, table_name: &str, record: usize, column: &Column, value: u32) {
    tables::fields::set_field(
        work.session,
        table_name,
        record,
        column.field,
        value,
        &format!("Edit {}", column.name),
        work.now,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `-1` is drawn as `-1`. Half the effect columns of a kit hold it, since
    /// `blank_defaults` and the shipped rows write it for "nothing here".
    /// Drawn as the `u32` the reader returns, it would show as `4294967295`
    /// where a row id belongs.
    #[test]
    fn a_column_that_says_nothing_reads_as_minus_one() {
        assert_eq!(signed(u32::MAX), "-1");
        assert_eq!(signed(0), "0");
        assert_eq!(signed(3068), "3068");
        assert_eq!(signed(i32::MAX as u32), "2147483647");
        assert_eq!(signed(i32::MAX as u32 + 1), "-2147483648");
    }
}
