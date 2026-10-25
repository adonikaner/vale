//! What is done to a row other than editing a field: a `crate::tables::commands::Command`
//! carried out, and the right-click menus of a row and a field that list
//! them.

use bevy_egui::egui;

use crate::tables::{commands::Command, Modal};
use crate::tables;
use crate::ui::tables::field::{signed, write};
use crate::ui::tables::reference::follow_reference;
use crate::ui::tables::{View, Workspace};
use crate::ui::tools::places::lights::new_light_params;

use vale_assets::tables::schema::{Column, Kind};

/// Run a command on one row. A command whose result is read on the row's own
/// form opens the row, so one run from a menu on a row that is not open
/// shows what it did.
///
/// This is the one place a [`Command`] is carried out. The buttons and the
/// menus both call it, so a command does the same thing from either.
pub(super) fn run_command(
    ctx: &egui::Context,
    work: &mut Workspace<'_>,
    table: &str,
    record: usize,
    command: Command,
) {
    use tables::learning::{ability, race_class};
    let Some(id) = work
        .session
        .table(table)
        .and_then(|open| open.u32_at(record, 0))
    else {
        return;
    };
    let here = table == work.browser.table;
    let show = |work: &mut Workspace<'_>| {
        if here && work.browser.open != Some(record) {
            work.browser.open = Some(record);
            work.browser.forget_buffers();
        }
    };
    match command {
        Command::AddToSkillLine => {
            if !work.session.open_table(work.assets, ability::TABLE) {
                return;
            }
            if let Some(at) = tables::learning::add_ability(work.session, id) {
                show(work);
                // The new row is drawn in the fields view's Learning section.
                work.browser.view = View::Fields;
                work.browser.modal = Some(Modal::Pick {
                    table: ability::TABLE.to_string(),
                    record: at,
                    field: ability::SKILL,
                    points_at: "SkillLine",
                });
                work.browser.pick_query.clear();
                work.browser.pick_focus = true;
            }
        }
        Command::CreateTeachingSpell => {
            work.session.status = match tables::learning::add_teaching_spell(work.session, id) {
                Some(made) => format!("created teaching spell {made} for spell {id}"),
                None => "teaching spell not created".to_string(),
            };
            show(work);
        }
        Command::LookLike => {
            show(work);
            work.browser.modal = Some(Modal::LooksLike { record });
            work.browser.pick_query.clear();
            work.browser.pick_focus = true;
        }
        Command::CloneChain => {
            if let Some(done) = tables::rows::clone_chain(work.session, id, None) {
                let copy = done.new_id("SpellVisual", id).unwrap_or(0);
                work.session.status = format!(
                    "SpellVisual {id} copied to {copy} with {} kits and {} effects",
                    done.count("SpellVisualKit"),
                    done.count("SpellVisualEffectName")
                );
                follow_reference(work, "SpellVisual", copy);
            }
        }
        Command::GiveToRacesAndClasses => {
            if !work.session.open_table(work.assets, race_class::TABLE) {
                return;
            }
            tables::learning::add_race_class_row(work.session, id);
            show(work);
        }
        Command::AddSubArea => {
            if let Some(at) = tables::zones::add_sub_area(work.session, id) {
                work.session.status = "sub-area added; enter its name".to_string();
                if here {
                    work.browser.open_row(at);
                }
            }
        }
        // The shell gives the area to the Areas tool and switches to it.
        Command::PaintArea => work.asks.push(crate::ui::shell::asks::ShellAsk::PaintArea(id)),
        Command::RebakeSkin => {
            let extra = match table {
                tables::bake::EXTRA => Some(id),
                _ => tables::bake::extra_of(work.session, record),
            };
            let Some(extra) = extra else { return };
            work.session.status = match tables::bake::rebake(work.session, work.assets, extra, work.now) {
                Ok(done) | Err(done) => done,
            };
            work.browser.forget_buffers();
            show(work);
        }
        // A `LightParams` row is copied with its bands, which no column
        // points at; see `tables::rows::copy_light_params`.
        Command::Clone if table == "LightParams" => {
            if let Some(copy) = new_light_params(work, id, None) {
                if here {
                    if let Some(at) = work.session.table(table).and_then(|t| t.row_of(copy)) {
                        work.browser.open_row(at);
                    }
                }
            }
        }
        Command::Clone => {
            if let Some(at) = tables::rows::clone_row(work.session, table, record) {
                if here {
                    work.browser.open_row(at);
                }
            }
        }
        Command::Delete => {
            if tables::rows::delete_row(work.session, table, record) && here {
                // The open row is found by its place in the file, and the
                // rows after a removed one have each moved up by one.
                match work.browser.open {
                    Some(open) if open == record => work.browser.close_row(),
                    Some(open) if open > record => {
                        work.browser.open = Some(open - 1);
                        work.browser.forget_buffers();
                    }
                    _ => {}
                }
            }
        }
        Command::CopyId => {
            ctx.copy_text(id.to_string());
            work.session.status = format!("{table} id {id} is on the clipboard");
        }
    }
}

/// A row's right-click menu: the table's commands, each offered where it
/// applies to the row. Answers the one that was pressed, for the caller to
/// run once the list is drawn.
///
/// Every entry is also a button somewhere: the list's own row of buttons,
/// the form's head, or a section of the form. The menu is the shorter way to
/// a command and never the only way.
pub(super) fn row_menu(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table: &str,
    record: usize,
) -> Option<Command> {
    let mut pressed = None;
    let mut drawn = 0;
    for command in tables::commands::commands(table) {
        let Some(label) =
            tables::commands::command_label(work.browser, work.session, table, record, command)
        else {
            continue;
        };
        // A line between the table's own commands and the three every row has.
        if command == Command::Clone && drawn > 0 {
            ui.separator();
        }
        drawn += 1;
        if ui
            .button(label)
            .on_hover_text(tables::commands::command_about(command))
            .clicked()
        {
            pressed = Some(command);
            ui.close();
        }
    }
    pressed
}

/// A field's right-click menu, on its name: for a reference, the three
/// things its row of controls does, and for every field its value to the
/// clipboard.
pub(super) fn field_menu(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table_name: &str,
    record: usize,
    column: &Column,
    raw: u32,
) {
    if let Kind::Reference(points_at) = column.kind {
        let named = raw != 0 && raw != u32::MAX;
        if named && ui.button(format!("Open {points_at} {}", signed(raw))).clicked() {
            follow_reference(work, points_at, raw);
            ui.close();
        }
        if ui.button("Choose\u{2026}").clicked() {
            work.browser.modal = Some(Modal::Pick {
                table: table_name.to_string(),
                record,
                field: column.field,
                points_at,
            });
            work.browser.pick_query.clear();
            work.browser.pick_focus = true;
            ui.close();
        }
        if named && ui.button("Set to none").clicked() {
            // A kit's slots hold -1 for nothing and every other reference 0,
            // which is what a blank row of the table holds in the field.
            let none = vale_assets::tables::schema::blank_defaults(table_name)
                .iter()
                .find(|(field, _)| *field == column.field)
                .map(|(_, value)| *value)
                .unwrap_or(0);
            write(work, table_name, record, column, none);
            ui.close();
        }
        ui.separator();
    }
    let value = match column.kind {
        Kind::Float => format!("{}", f32::from_bits(raw)),
        Kind::Flags(_) => format!("0x{raw:08X}"),
        _ => signed(raw),
    };
    if ui.button(format!("Copy value {value}")).clicked() {
        ui.ctx().copy_text(value);
        ui.close();
    }
}
