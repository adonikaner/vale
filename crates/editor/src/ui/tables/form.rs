//! The open row's form: the head, the schema's sections of fields, the line
//! that says which rows use this one, and the stage beside a visual's or a
//! kit's fields.

use bevy_egui::egui;

use crate::tables::commands::Command;
use crate::tables;
use crate::ui::tables::commands::run_command;
use crate::ui::tables::field::field_in;
use crate::ui::tables::learning::{learning, skill_line_members};
use crate::ui::tables::modals::modals;
use crate::ui::tables::previews::{chain_show, dialog_previews, empty_pane};
use crate::ui::tables::reference::follow_reference;
use crate::ui::tables::storyboard::Show;
use crate::ui::tables::unschemad::unschemad;
use crate::ui::tables::zones::area_block;
use crate::ui::tables::{View, Workspace};
use crate::ui::tools::places::lights::{place_strip, sphere_block};
use crate::ui::widgets::theme;

use vale_assets::tables::schema::{Column, Kind, Schema};
use vale_assets::tables::schema;

/// The icon at the head of the form, which has room for more.
const HEAD_ICON: f32 = 36.0;

/// How wide the visual chain is on the storyboard view, leaving the rest of the
/// middle for the preview.
const CHAIN_WIDTH: f32 = 480.0;

/// How wide a visual's or a kit's fields are beside their preview. Wider
/// than the storyboard's cards: a reference's row is a number, a button, a
/// name and a second line of text.
const CHAIN_FORM: f32 = 680.0;

/// The narrowest the preview beside a visual's or a kit's fields is made.
/// Less than [`STAGE_FLOOR`], because these fields need the width more than
/// the storyboard's cards do.
const CHAIN_STAGE_FLOOR: f32 = 300.0;

/// The minimum width of the storyboard preview.
const STAGE_FLOOR: f32 = 360.0;

/// The right column: the open row, section by section.
pub(super) fn form(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    board: &mut super::storyboard::Storyboard,
    stage: &mut crate::preview::stage::Stage,
    lab: &mut crate::preview::lab::Lab,
) {
    let table_name = work.browser.table.clone();
    // A row removed, or an undo of one added, leaves an index past the end.
    if work
        .browser
        .open
        .zip(
            work.session
                .table(&table_name)
                .map(|table| table.record_count()),
        )
        .is_some_and(|(open, count)| open >= count)
    {
        work.browser.close_row();
    }
    let Some(record) = work.browser.open else {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Choose a row on the left.")
                    .size(14.0)
                    .color(theme::INK_DIM),
            );
        });
        stage.close();
        modals(ui, work, Some((stage, lab)));
        return;
    };

    let schema = schema::for_table(&table_name);

    ui.add_space(8.0);
    head(ui, work, record, stage, lab);

    if table_name == "Light" {
        place_strip(ui, work, record);
    }

    // Only a spell has a second view, because only a spell names a chain. On a
    // table with one view the switch is not drawn, since it would do nothing.
    let has_story = table_name == "Spell";
    if has_story {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.allocate_ui(egui::vec2(240.0, 24.0), |ui| {
                theme::segmented(
                    ui,
                    &mut work.browser.view,
                    &[("Fields", View::Fields), ("Storyboard", View::Storyboard)],
                    |a, b| a == b,
                );
            });
        });
    } else {
        work.browser.view = View::Fields;
    }
    ui.add_space(8.0);

    if work.browser.view == View::Storyboard {
        let spell = work
            .session
            .table(&table_name)
            .and_then(|table| table.u32_at(record, 0));
        // The preview pane has a minimum width, `STAGE_FLOOR`, and the chain
        // gets the rest. A fixed 480-point chain left the preview too narrow to
        // use on a 1280-wide window.
        let here = ui.available_width();
        let chain = CHAIN_WIDTH.min((here - STAGE_FLOOR).max(300.0));
        egui::Panel::left("storyboard-chain")
            .default_size(chain)
            .min_size(280.0)
            .max_size((here - STAGE_FLOOR).max(300.0))
            .resizable(true)
            .frame(egui::Frame::new().fill(theme::SHELL).inner_margin(4.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("data-storyboard")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                        super::storyboard::draw(ui, work, board, record);
                    });
            });
        // A dialog that plays its own rows has the stage while it is up.
        match dialog_previews(work) {
            true => empty_pane(ui, "the preview is shown in the open dialog"),
            false => {
                let title = match spell {
                    Some(id) => format!("Preview — spell {id}"),
                    None => "Preview".to_string(),
                };
                super::storyboard::stage_pane(
                    ui,
                    stage,
                    spell.map(Show::Spell),
                    &title,
                    &board.notes,
                    Some(&*board),
                );
            }
        }
        modals(ui, work, Some((stage, lab)));
        return;
    }

    // An effect always has the preview pane, laid out like the storyboard: the
    // fields on the left and the stage on the right with the model alone. Once
    // Position on character… is pressed, the lab's card is added above the
    // fields and the mannequin under the model. See `crate::preview::lab`.
    let id = work
        .session
        .table(&table_name)
        .and_then(|table| table.u32_at(record, 0))
        .unwrap_or(0);
    if table_name == "SpellVisualEffectName" {
        if !lab.is_open_on(id) {
            crate::ui::preview::lab::open_alone(lab, stage, work.browser, work.session, record);
        } else if let Some(table) = work.session.table(&table_name) {
            // Pass the row's current `Model` and `Scale` to
            // `Lab::follow_row`, which makes an edit to either show in the
            // pane beside the form.
            use vale_assets::tables::spell::{effect_scale, fields};
            let model = table
                .string_at(record, fields::EFFECT_MODEL)
                .unwrap_or_default();
            let scale = effect_scale(table.f32_at(record, fields::EFFECT_SCALE));
            lab.follow_row(&model, scale);
        }
        // The browser's preview reaches the stage through the lab.
        lab.preview = work.browser.model_preview.clone();
        let here = ui.available_width();
        // The lab's card needs width and an effect's five fields do not, so
        // the model view gives the extra room to the pane. The two modes use
        // different panel ids, so each keeps the width it was dragged to.
        let on_character = lab.on_character(id);
        let form_width = match on_character {
            true => (CHAIN_WIDTH + 140.0).min((here - STAGE_FLOOR).max(320.0)),
            false => 360.0_f32.min((here - STAGE_FLOOR).max(320.0)),
        };
        // The contents are limited to the panel's width from the last frame.
        // The panel's inner `Ui` reports more width than the panel shows, which
        // let a note wrap a hundred points past its edge. Using the width the
        // panel actually took lags a resize by one frame.
        let known = lab.panel_width;
        let shown = egui::Panel::left(match on_character {
            true => "lab-form",
            false => "model-form",
        })
        .default_size(form_width)
        .min_size(320.0)
        .max_size((here - STAGE_FLOOR).max(320.0))
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::SHELL).inner_margin(4.0))
        .show(ui, |ui| {
            if known > 0.0 {
                ui.set_max_width(known - 24.0);
            }
            egui::ScrollArea::vertical()
                .id_salt("data-lab")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if known > 0.0 {
                        ui.set_max_width(known - 24.0);
                    }
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 7.0);
                    ui.spacing_mut().interact_size.y = 24.0;
                    if lab.on_character(id) {
                        crate::ui::preview::lab::card(ui, work, lab, record);
                        ui.add_space(8.0);
                    }
                    fields(ui, work, record, schema);
                });
        });
        lab.panel_width = shown.response.rect.width();
        crate::ui::preview::lab::pane(ui, work, stage, lab);
        modals(ui, work, Some((stage, lab)));
        return;
    }
    // A visual and a kit have the preview pane beside their fields, as a
    // spell's storyboard and an effect have, so the row is seen as it is
    // edited. See [`chain_show`] for what is played.
    if matches!(table_name.as_str(), "SpellVisual" | "SpellVisualKit") {
        let here = ui.available_width();
        let room = (here - CHAIN_STAGE_FLOOR).max(320.0);
        // The contents are limited to the width the panel took on the last
        // frame, as the effect's form is, and for the same reason. A row
        // wider than that scrolls sideways, so the buttons at its end can
        // be reached on a narrow window.
        let width_id = egui::Id::new("chain-form-width");
        let known: f32 = ui.data(|data| data.get_temp(width_id)).unwrap_or(0.0);
        let shown = egui::Panel::left("chain-form")
            .default_size(CHAIN_FORM.min(room))
            .min_size(320.0)
            .max_size(room)
            .resizable(true)
            .frame(egui::Frame::new().fill(theme::SHELL).inner_margin(4.0))
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt("data-chain-form")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if known > 0.0 {
                            ui.set_max_width(known - 24.0);
                        }
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 7.0);
                        ui.spacing_mut().interact_size.y = 24.0;
                        fields(ui, work, record, schema);
                    });
            });
        ui.data_mut(|data| data.insert_temp(width_id, shown.response.rect.width()));
        match dialog_previews(work) {
            true => empty_pane(ui, "the preview is shown in the open dialog"),
            false => match chain_show(work, &table_name, id) {
                Some((show, title)) => {
                    super::storyboard::stage_pane(ui, stage, Some(show), &title, &[], None)
                }
                None => {
                    stage.close();
                    empty_pane(ui, "no preview for this row");
                }
            },
        }
        modals(ui, work, Some((stage, lab)));
        return;
    }
    // Outside the storyboard, the chain's panes and the lab nothing is
    // previewed, unless a dialog plays its rows. Closing the stage removes
    // the units from it and returns the camera.
    if !dialog_previews(work) {
        stage.close();
    }

    egui::ScrollArea::vertical()
        .id_salt("data-form")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Rows tall enough to click and spaced enough to read. This is set
            // on the form's own `Ui` rather than in the theme, because this is
            // the one panel in the editor that is a page of fields rather than
            // a few controls.
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 7.0);
            ui.spacing_mut().interact_size.y = 24.0;
            fields(ui, work, record, schema);
        });
    modals(ui, work, Some((stage, lab)));
}

/// The row's fields, section by section — or numbered, for a table with no
/// schema.
pub(in crate::ui) fn fields(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    record: usize,
    schema: Option<&'static Schema>,
) {
    match schema {
        Some(schema) => {
            // A spell's skill lines and teaching spells are rows of other
            // tables that exist only for the spell, so they are edited on
            // the spell's form, above its own columns.
            if schema.table == "Spell" {
                learning(ui, work, record);
            }
            // A zone's sub-areas are rows of the same table that name it, so
            // they are listed on the zone's form, above its own columns.
            if schema.table == tables::zones::area::TABLE {
                area_block(ui, work, record);
            }
            for section in schema.sections {
                // A light's sphere is drawn in the world's units. Its five
                // columns are in 1/36 of a yard measured from the corner of
                // the map (for example `557760`, `0`, `966720`), which cannot
                // be read or typed sensibly. `sphere_block` shows the same
                // five fields converted.
                if schema.table == "Light" && section.name == "Sphere" {
                    sphere_block(ui, work, record);
                    continue;
                }
                section_block(ui, work, record, schema, section);
            }
            // …and so are the rows that say who has a skill line.
            if schema.table == "SkillLine" {
                skill_line_members(ui, work, record);
            }
        }
        // A table with no schema still opens, as numbered fields with guessed
        // types. This is what `vale dbc <Table>` prints, and it is the
        // starting point for writing that table's schema.
        None => unschemad(ui, work, record),
    }
    ui.add_space(24.0);
}

/// The head of the form: where it came from, the picture, the name, the id,
/// and who points at it.
fn head(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    record: usize,
    stage: &mut crate::preview::stage::Stage,
    lab: &mut crate::preview::lab::Lab,
) {
    let table_name = work.browser.table.clone();
    let label = work.browser.describe(work.session, &table_name, record);
    let icon = work.icon_of(record);
    let mut open_lab = false;
    let mut clone_chain = false;
    let mut copy_id = false;
    // The table's own commands drawn as buttons, which its right-click menu
    // also lists: an area's two and a display's rebake.
    let mut own_command: Option<Command> = None;
    ui.horizontal(|ui| {
        if !work.browser.back.is_empty() && ui.button("< back").clicked() {
            work.browser.go_back();
        }
        work.icon(ui, icon, HEAD_ICON);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                let title = match label.title.is_empty() {
                    true => format!("{table_name} row"),
                    false => label.title.clone(),
                };
                ui.label(egui::RichText::new(title).strong().size(17.0).color(theme::INK));
                ui.label(theme::number(format!("#{}", label.id)).color(theme::INK_DIM));
                if work.session.unsaved_tables.contains(&table_name) {
                    ui.label(egui::RichText::new("unsaved").color(theme::WARN));
                }
                // Actions on this row other than editing a field.
                match table_name.as_str() {
                    "SpellVisual" => {
                        clone_chain = ui
                            .button("Clone chain")
                            .on_hover_text(tables::commands::command_about(Command::CloneChain))
                            .clicked();
                    }
                    "SpellVisualEffectName" if !lab.on_character(label.id) => {
                        open_lab = ui
                            .button("Position on character…")
                            .on_hover_text(
                                "Attach this model to a character model at an attachment point, \
                                 position it, and bake the offset into a copy of the model. \
                                 SpellVisualEffectName has no offset columns in 1.12, so the \
                                 position is stored in the model file.",
                            )
                            .clicked();
                    }
                    "AreaTable" | "CreatureDisplayInfo" | "CreatureDisplayInfoExtra" => {
                        let own: &[Command] = match table_name.as_str() {
                            "AreaTable" => &[Command::PaintArea, Command::AddSubArea],
                            _ => &[Command::RebakeSkin],
                        };
                        for &command in own {
                            let Some(said) = tables::commands::command_label(
                                work.browser,
                                work.session,
                                &table_name,
                                record,
                                command,
                            ) else {
                                continue;
                            };
                            if ui
                                .button(said)
                                .on_hover_text(tables::commands::command_about(command))
                                .clicked()
                            {
                                own_command = Some(command);
                            }
                        }
                    }
                    _ => {}
                }
            });
            ui.horizontal(|ui| {
                if !label.sub.is_empty() {
                    ui.label(egui::RichText::new(&label.sub).color(theme::INK_DIM));
                }
                ui.label(
                    egui::RichText::new(format!("{table_name}.dbc · row {record}"))
                        .small()
                        .color(theme::INK_FAINT),
                );
                copy_id = ui
                    .small_button("copy id")
                    .on_hover_text(tables::commands::command_about(Command::CopyId))
                    .clicked();
            });
        });
    });
    // One line listing the rows that point here; the full list is in the
    // inspector. Other spells point at a spell only through trigger columns,
    // which is rarely why a spell is opened, so the line is shown only for the
    // chain tables. An area is pointed at only by its sub-areas, which its
    // form lists under their own heading.
    if table_name != "Spell" && table_name != tables::zones::area::TABLE {
        used_by_line(ui, work, &table_name, label.id);
    }
    if open_lab {
        crate::ui::preview::lab::open_on(lab, stage, work.browser, work.session, record);
    }
    let ctx = ui.ctx().clone();
    if clone_chain {
        run_command(&ctx, work, &table_name, record, Command::CloneChain);
    }
    if let Some(command) = own_command {
        run_command(&ctx, work, &table_name, record, command);
    }
    if copy_id {
        run_command(&ctx, work, &table_name, record, Command::CopyId);
    }
}

/// The line naming the rows that point at this one, grouped by table, such as
/// `Used by 9 spells: Fireball, Fireball, … · 2 kits`. Each name is a link.
/// Shows a note instead when no row of an open table points here.
fn used_by_line(ui: &mut egui::Ui, work: &mut Workspace<'_>, table_name: &str, id: u32) {
    let uses = work.browser.used_by(work.session, table_name, id);
    if uses.is_empty() {
        theme::note(ui, "No row of an open table references this row.");
        return;
    }
    let mut follow: Option<(String, usize)> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(
            egui::RichText::new("Used by")
                .small()
                .color(theme::INK_FAINT),
        );
        // Grouped by table, with spells first because they are usually what a
        // person is looking for.
        let mut sources: Vec<String> = uses.iter().map(|at| at.table.clone()).collect();
        sources.sort();
        sources.dedup();
        sources.sort_by_key(|table| table != "Spell");
        for (which, source) in sources.iter().enumerate() {
            let here: Vec<usize> = uses
                .iter()
                .filter(|at| &at.table == source)
                .map(|at| at.record)
                .collect();
            if which > 0 {
                ui.label(egui::RichText::new("·").small().color(theme::INK_FAINT));
            }
            ui.label(
                egui::RichText::new(format!("{} {}", here.len(), plural(source, here.len())))
                    .small()
                    .color(theme::INK_DIM),
            );
            let mut shown = Vec::new();
            for &at in &here {
                let label = work.browser.describe(work.session, source, at);
                let name = match label.title.is_empty() {
                    true => format!("#{}", label.id),
                    false => label.title.clone(),
                };
                // A name repeated nine times is one spell's nine ranks; show it
                // once.
                if shown.contains(&name) {
                    continue;
                }
                if shown.len() >= 6 {
                    ui.label(egui::RichText::new("…").small().color(theme::INK_FAINT));
                    break;
                }
                if ui
                    .add(egui::Link::new(egui::RichText::new(&name).small()))
                    .clicked()
                {
                    follow = Some((source.clone(), at));
                }
                shown.push(name);
            }
        }
    });
    if let Some((table, record)) = follow {
        let id = work
            .session
            .table(&table)
            .and_then(|open| open.u32_at(record, 0))
            .unwrap_or(0);
        follow_reference(work, &table, id);
    }
}

/// `spell` / `spells`, by the table's own name.
pub(super) fn plural(table: &str, count: usize) -> String {
    let word = match table {
        "Spell" => "spell",
        "SpellVisual" => "visual",
        "SpellVisualKit" => "kit",
        "AreaTable" => "area",
        other => other,
    };
    match count {
        1 => word.to_string(),
        _ => format!("{word}s"),
    }
}

/// One section of the form: its fields, with the locale columns folded.
pub(in crate::ui) fn section_block(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    record: usize,
    schema: &Schema,
    section: &schema::Section,
) {
    let (main, other): (Vec<&Column>, Vec<&Column>) = section
        .fields
        .iter()
        .filter_map(|&field| schema.column(field))
        .partition(|column| !matches!(column.kind, Kind::Locale(_) | Kind::LocaleFlags));
    egui::CollapsingHeader::new(egui::RichText::new(section.name).size(14.0).strong())
        .default_open(true)
        .show(ui, |ui| {
            for column in main {
                field_row(ui, work, record, column);
            }
            if !other.is_empty() {
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!("Other locales ({})", other.len()))
                        .size(12.0)
                        .color(theme::INK_DIM),
                )
                .id_salt((section.name, "locales"))
                .default_open(false)
                .show(ui, |ui| {
                    for column in other {
                        field_row(ui, work, record, column);
                    }
                });
            }
        });
}

/// One field of the open table: its name, and the widget its kind asks for.
fn field_row(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize, column: &Column) {
    let table_name = work.browser.table.clone();
    field_in(ui, work, &table_name, record, column);
}
