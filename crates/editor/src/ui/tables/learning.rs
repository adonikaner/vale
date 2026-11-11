//! The rows that exist only for a spell or a skill line, drawn on that row's
//! form: a spell's Learning section (its `SkillLineAbility` rows and the
//! spells that teach it) and a skill line's `SkillRaceClassInfo` rows. A
//! skill line's own spells are `super::skills`'.

use bevy_egui::egui;

use crate::tables::commands::Command;
use crate::tables;
use crate::ui::tables::commands::run_command;
use crate::ui::tables::field::field_in;
use crate::ui::tables::reference::follow_reference;
use crate::ui::tables::{note_about, Workspace, LABEL};
use crate::ui::widgets::theme;

use vale_assets::tables::schema::Schema;
use vale_assets::tables::schema;

/// What the Skill Lines heading's info mark says: what the section lists, and
/// what a spell with no `SkillLineAbility` row gets.
const SKILL_LINES_ABOUT: &str = "The SkillLineAbility rows that name this spell. Most spells have \
                                 none. The spellbook lists such a spell under General, and the \
                                 training window omits a class trainer's service for it.";

/// What the Taught By heading's info mark says: what the section lists, and
/// what a teaching spell is.
const TAUGHT_BY_ABOUT: &str = "The spells that teach this spell. Most spells have none. A trainer's \
                               list names a teaching spell: a spell whose first effect is Learn \
                               Spell and whose EffectTriggerSpell 1 is this spell.";

/// A spell's Learning section: the skill lines it is in and the spells that
/// teach it, which are rows of other tables that exist only for this spell.
///
/// Neither is required. Most spells are in no skill line and have no teaching
/// spell, and each heading then has one line saying so and a button that
/// makes one. A row made here is an ordinary row of its table: the Skills
/// workspace's Abilities part lists it, and Open goes to it there.
///
/// A teaching spell has a Teaches row first, with the spell it teaches as a
/// link.
pub(super) fn learning(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    use vale_assets::tables::spellbook::spell_fields;
    let Some((spell, teaches)) = work.session.table("Spell").and_then(|spells| {
        let learns = spells.u32_at(record, spell_fields::EFFECT)? == vale_mangos::trainer::LEARN_SPELL;
        let taught = spells.u32_at(record, spell_fields::EFFECT_TRIGGER_SPELL)?;
        Some((spells.u32_at(record, 0)?, learns.then_some(taught)))
    }) else {
        return;
    };
    theme::section(ui, "Learning", "Learning", true, false, |ui| {
        if let Some(taught) = teaches.filter(|taught| *taught != 0) {
            ui.horizontal(|ui| {
                // The name cell is as wide as a field's, by the rule
                // `field_in` uses, so the link starts where the values of the
                // fields under it start.
                let label_width = LABEL.min(ui.available_width() * 0.45);
                theme::name_cell(
                    ui,
                    egui::vec2(label_width, 22.0),
                    egui::RichText::new("Teaches").color(theme::INK_NAME),
                    egui::Sense::hover(),
                )
                .on_hover_text("This spell is a teaching spell for the spell named here.");
                let name = work
                    .browser
                    .describe_id(work.session, "Spell", taught)
                    .map(|label| label.line())
                    .unwrap_or_else(|| format!("Spell {taught}"));
                if crate::ui::widgets::reference::link(ui, &name, theme::ACCENT, &format!("Open Spell {taught}.")) {
                    follow_reference(work, "Spell", taught);
                }
            });
        }
        skill_lines_of(ui, work, spell);
        taught_by(ui, work, spell);
    });
}

/// One row of another table drawn on the open row's form: a heading line
/// with what the row is, its id, a link to the row itself and a button that
/// removes it, then the fields named, and the fields of `more` under a
/// Skill Values fold that starts folded. Answers whether Remove was pressed.
fn owned_row(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    schema: &'static Schema,
    row: usize,
    title: &str,
    fields: &[usize],
    more: &[usize],
) -> bool {
    let table = schema.table;
    let id = work
        .session
        .table(table)
        .and_then(|open| open.u32_at(row, 0))
        .unwrap_or(0);
    let mut remove = false;
    egui::Frame::new()
        .fill(theme::PANEL)
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // The title takes a right-click, for the two buttons at the
                // right of the line.
                ui.add(
                    egui::Label::new(egui::RichText::new(title).color(theme::INK))
                        .sense(egui::Sense::click()),
                )
                .context_menu(|ui| {
                    if ui.button(format!("Open {table} {id}")).clicked() {
                        follow_reference(work, table, id);
                        ui.close();
                    }
                    if ui.button("Remove").clicked() {
                        remove = true;
                        ui.close();
                    }
                });
                ui.label(theme::number(format!("#{id}")).color(theme::INK_FAINT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    remove |= ui
                        .small_button("Remove")
                        .on_hover_text(format!("Remove this {table} row. One undo entry."))
                        .clicked();
                    if ui
                        .small_button("Open")
                        .on_hover_text(format!("Open {table} {id} with every field."))
                        .clicked()
                    {
                        follow_reference(work, table, id);
                    }
                });
            });
            for column in fields.iter().filter_map(|&field| schema.column(field)) {
                field_in(ui, work, table, row, column);
            }
            if !more.is_empty() {
                theme::fold_closed(ui, (table, id, "more"), "Skill Values", |ui| {
                    for column in more.iter().filter_map(|&field| schema.column(field)) {
                        field_in(ui, work, table, row, column);
                    }
                });
            }
        });
    ui.add_space(2.0);
    remove
}

/// The skill lines a spell is in, as the `SkillLineAbility` rows that name
/// it, each with its skill line, its classes and races, whether learning the
/// line gives the spell, and the spell that supersedes it. What a spell with
/// none gets is on the heading's info mark, [`SKILL_LINES_ABOUT`].
fn skill_lines_of(ui: &mut egui::Ui, work: &mut Workspace<'_>, spell: u32) {
    use tables::learning::ability;
    theme::heading_about(ui, "Skill Lines", SKILL_LINES_ABOUT);
    if !work.session.open_table(work.assets, ability::TABLE) {
        theme::note(ui, "SkillLineAbility did not open");
        return;
    }
    let rows = tables::learning::abilities_of(work.browser, work.session, spell);
    if rows.is_empty() {
        theme::note(ui, "No SkillLineAbility row names this spell.");
    }
    let mut remove: Option<usize> = None;
    for &row in &rows {
        let skill = work
            .session
            .table(ability::TABLE)
            .and_then(|open| open.u32_at(row, ability::SKILL))
            .unwrap_or(0);
        let title = work
            .browser
            .describe_id(work.session, "SkillLine", skill)
            .map(|label| label.title)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "No skill line chosen".to_string());
        if owned_row(
            ui,
            work,
            &schema::SKILL_LINE_ABILITY,
            row,
            &title,
            &[
                ability::SKILL,
                ability::CLASSES,
                ability::RACES,
                ability::LEARN_ON_GET_SKILL,
                ability::SUPERSEDED_BY,
            ],
            &[
                ability::REQ_SKILL_VALUE,
                ability::MIN_VALUE,
                ability::MAX_VALUE,
                ability::REQ_TRAIN_POINTS,
            ],
        ) {
            remove = Some(row);
        }
    }
    if ui
        .button("Add to Skill Line")
        .on_hover_text(tables::commands::command_about(Command::AddToSkillLine))
        .clicked()
    {
        if let Some(record) = work.browser.record_of(work.session, "Spell", spell) {
            let ctx = ui.ctx().clone();
            run_command(&ctx, work, "Spell", record, Command::AddToSkillLine);
        }
    }
    if let Some(row) = remove {
        tables::rows::delete_row(work.session, ability::TABLE, row);
    }
}

/// The spells that teach a spell, and a button that makes one.
///
/// A trainer's list names a teaching spell and not the spell a player ends
/// up with, so a spell a trainer is to teach needs one. A spell learned any
/// other way needs none. What a teaching spell is is on the heading's info
/// mark, [`TAUGHT_BY_ABOUT`].
fn taught_by(ui: &mut egui::Ui, work: &mut Workspace<'_>, spell: u32) {
    theme::heading_about(ui, "Taught By", TAUGHT_BY_ABOUT);
    let teachers = tables::learning::teachers_of(work.browser, work.session, spell);
    if teachers.is_empty() {
        theme::note(ui, "No teaching spell.");
    }
    let mut follow: Option<u32> = None;
    for &(row, instant) in &teachers {
        let label = work.browser.describe(work.session, "Spell", row);
        ui.horizontal(|ui| {
            if crate::ui::widgets::reference::link(ui, &label.line(), theme::ACCENT, &format!("Open Spell {}.", label.id)) {
                follow = Some(label.id);
            }
            // The rank, which is what tells one teaching spell's row from the
            // next rank's.
            if !label.sub.is_empty() {
                ui.label(egui::RichText::new(&label.sub).small().color(theme::INK_DIM));
            }
            ui.label(
                egui::RichText::new(match instant {
                    true => "Instant",
                    false => "Has a cast time",
                })
                .small()
                .color(theme::INK_DIM),
            );
        });
    }
    if let Some(id) = follow {
        follow_reference(work, "Spell", id);
    }
    // A trainer's list needs an instant teaching spell. A spell whose only
    // teaching spell has a cast time, as the teaching spell a book casts has,
    // is offered an instant one beside it.
    // The command's label is `None` when an instant one exists, and the
    // button is then not drawn. The menu's entry is a sentence; the button
    // says the same in a button's words, by the same test the label makes.
    let offered = work
        .browser
        .record_of(work.session, "Spell", spell)
        .and_then(|record| {
            tables::commands::command_label(
                work.browser,
                work.session,
                "Spell",
                record,
                Command::CreateTeachingSpell,
            )?;
            Some(record)
        });
    if let Some(record) = offered {
        let label = match teachers.is_empty() {
            true => "Create Teaching Spell",
            false => "Create Instant Teaching Spell",
        };
        if ui
            .button(label)
            .on_hover_text(tables::commands::command_about(Command::CreateTeachingSpell))
            .clicked()
        {
            let ctx = ui.ctx().clone();
            run_command(&ctx, work, "Spell", record, Command::CreateTeachingSpell);
        }
    }
}

/// A skill line's two blocks of other tables' rows: who has it, as the
/// `SkillRaceClassInfo` rows that name the line, then its spells, as
/// [`super::skills::abilities_block`] lists them.
pub(super) fn skill_line_members(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    use tables::learning::race_class;
    let Some(skill) = work
        .session
        .table("SkillLine")
        .and_then(|lines| lines.u32_at(record, 0))
    else {
        return;
    };
    theme::section(ui, "Races and classes", "Races and Classes", true, false, |ui| {
        if !work.session.open_table(work.assets, race_class::TABLE) {
            theme::note(ui, "SkillRaceClassInfo did not open");
            return;
        }
        let rows = tables::learning::race_class_rows_of(work.browser, work.session, skill);
        if rows.is_empty() {
            note_about(
                ui,
                "No SkillRaceClassInfo row names this skill line, so no race or class has it.",
                "The spells of a skill line that no race or class has appear on the \
                 spellbook's General tab.",
            );
        }
        let mut remove: Option<usize> = None;
        for &row in &rows {
            let title = work.browser.describe(work.session, race_class::TABLE, row).sub;
            if owned_row(
                ui,
                work,
                &schema::SKILL_RACE_CLASS_INFO,
                row,
                &title,
                &[
                    race_class::CLASSES,
                    race_class::RACES,
                    race_class::FLAGS,
                    race_class::MIN_LEVEL,
                    race_class::SKILL_TIER,
                ],
                &[],
            ) {
                remove = Some(row);
            }
        }
        if ui
            .button("Add Race/Class Row")
            .on_hover_text(tables::commands::command_about(Command::GiveToRacesAndClasses))
            .clicked()
        {
            let ctx = ui.ctx().clone();
            run_command(&ctx, work, "SkillLine", record, Command::GiveToRacesAndClasses);
        }
        if let Some(row) = remove {
            tables::rows::delete_row(work.session, race_class::TABLE, row);
        }
    });
    super::skills::abilities_block(ui, work, record);
}
