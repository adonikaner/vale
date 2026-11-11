//! Two blocks of the Skills workspace's forms that a list of fields does not
//! show well: a skill line's spells, as its `SkillLineAbility` rows, and a
//! tier's steps, as its sixteen `MaxValue` and sixteen `Cost` columns paired.

use std::sync::Arc;

use bevy_egui::egui;

use crate::tables;
use crate::tables::commands::Command;
use crate::tables::learning::ability;
use crate::ui::tables::{note_about, Workspace};
use crate::ui::tables::commands::run_command;
use crate::ui::tables::reference::follow_reference;
use crate::ui::widgets::theme;

use vale_assets::tables::schema::{self, Kind, Schema};
use vale_assets::tables::tradeskill::Difficulty;

/// The height of one line of the abilities list. The list draws only the
/// lines in view, which needs one height for all of them.
const LINE: f32 = 26.0;

/// How tall the abilities list grows before it scrolls.
const LIST_HEIGHT: f32 = 380.0;

/// From this many abilities on, the list has a filter.
const FILTER_FROM: usize = 12;

/// The side of an ability's spell icon.
const ICON_SIDE: f32 = 20.0;

/// The width of a step's two number boxes.
const STEP_VALUE: f32 = 90.0;

/// One `SkillLineAbility` row of the open skill line, as its line in the
/// list shows it.
#[derive(Debug, Clone)]
struct Ability {
    /// The row's record in `SkillLineAbility`.
    row: usize,
    /// The row's id.
    id: u32,
    spell: u32,
    /// The spell's name and rank, or `Spell N` when `Spell` has no such row.
    name: String,
    rank: String,
    /// The classes and races the row is for, in words, or empty for every
    /// race and class.
    who: String,
    /// Whether learning the line teaches the spell: `LearnOnGetSkill` 1 or 2.
    with_line: bool,
    /// `ReqSkillValue`, `MinValue` and `MaxValue`.
    req: u32,
    min: u32,
    max: u32,
}

impl Ability {
    /// Whether the row holds a recipe's skill values. 5,026 of the 5,072
    /// shipped rows hold `ReqSkillValue` 1 and no `MaxValue`.
    fn is_recipe(&self) -> bool {
        self.max != 0
    }

    /// Whether the filter text `query` finds this line: in the name, the
    /// rank, the classes and races, or the spell id.
    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.name.to_lowercase().contains(&query)
            || self.rank.to_lowercase().contains(&query)
            || self.who.to_lowercase().contains(&query)
            || self.spell.to_string() == query
    }
}

/// A mask column of `SkillLineAbility` in words: the names of its set bits,
/// or empty for zero or every named bit, which both mean everyone.
fn mask_words(schema: &Schema, field: usize, value: u32) -> String {
    let Some(Kind::Flags(bits)) = schema.column(field).map(|column| column.kind) else {
        return String::new();
    };
    let every = bits.iter().fold(0u32, |all, (bit, _, _)| all | bit);
    if value == 0 || value & every == every {
        return String::new();
    }
    schema::named_bits(value, bits).join(", ")
}

/// The open skill line's abilities: the rows with no recipe values first,
/// then the recipes by the value at which they turn grey, each group by name
/// and then spell id. A profession's recipes come out in the order a
/// character outgrows them, since `ReqSkillValue` is 1 on almost every
/// shipped row, and a rank comes after the one before it. Built once per revision of
/// the open tables and kept in egui's memory. `Spell` opens after
/// `SkillLine`, so whether it is open is part of the key: the names are
/// read again when it arrives.
fn abilities(ui: &egui::Ui, work: &mut Workspace<'_>, skill: u32) -> Arc<Vec<Ability>> {
    let spells = work.session.table("Spell").is_some();
    let key = egui::Id::new(("skill-abilities", skill, work.session.table_revision, spells));
    if let Some(known) = ui.data(|data| data.get_temp::<Arc<Vec<Ability>>>(key)) {
        return known;
    }
    let rows = tables::learning::abilities_in(work.browser, work.session, skill);
    let mut all: Vec<Ability> = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(table) = work.session.table(ability::TABLE) else {
            break;
        };
        let num = |field: usize| table.u32_at(row, field).unwrap_or(0);
        let (id, spell) = (num(0), num(ability::SPELL));
        let (classes, races) = (num(ability::CLASSES), num(ability::RACES));
        let (with_line, req, min, max) = (
            num(ability::LEARN_ON_GET_SKILL) != 0,
            num(ability::REQ_SKILL_VALUE),
            num(ability::MIN_VALUE),
            num(ability::MAX_VALUE),
        );
        let who: Vec<String> = [
            mask_words(&schema::SKILL_LINE_ABILITY, ability::CLASSES, classes),
            mask_words(&schema::SKILL_LINE_ABILITY, ability::RACES, races),
        ]
        .into_iter()
        .filter(|words| !words.is_empty())
        .collect();
        let label = work.browser.describe_id(work.session, "Spell", spell);
        let (name, rank) = match label {
            Some(label) if !label.title.is_empty() => (label.title, label.sub),
            _ => (format!("Spell {spell}"), String::new()),
        };
        all.push(Ability { row, id, spell, name, rank, who: who.join(" \u{b7} "), with_line, req, min, max });
    }
    all.sort_by(|a, b| (a.max, &a.name, a.spell).cmp(&(b.max, &b.name, b.spell)));
    let all = Arc::new(all);
    ui.data_mut(|data| data.insert_temp(key, all.clone()));
    all
}

/// The trade skill window's colour for a difficulty, as
/// `TradeSkillTypeColor` in `Blizzard_TradeSkillUI.lua` gives it.
fn difficulty_colour(difficulty: Difficulty) -> egui::Color32 {
    match difficulty {
        Difficulty::Optimal => egui::Color32::from_rgb(255, 128, 64),
        Difficulty::Medium => egui::Color32::from_rgb(255, 255, 0),
        Difficulty::Easy => egui::Color32::from_rgb(64, 191, 64),
        _ => egui::Color32::from_rgb(128, 128, 128),
    }
}

/// The skill values at which a recipe turns yellow, green and grey, by the
/// trade skill window's rule
/// (`vale_assets::tables::tradeskill::Thresholds::difficulty`): a `MinValue` of
/// 0 stands for `MaxValue` less 25, and green starts halfway between yellow
/// and grey.
fn recipe_points(min: u32, max: u32) -> [(u32, Difficulty); 3] {
    let yellow = match min {
        0 => max.saturating_sub(25),
        min => min,
    };
    [
        (yellow, Difficulty::Medium),
        ((yellow + max) / 2, Difficulty::Easy),
        (max, Difficulty::Trivial),
    ]
}

/// A skill line's spells: one line per `SkillLineAbility` row that names the
/// line, with the spell's icon, name and rank, who the row is for, whether
/// learning the line teaches it, and a recipe's four skill values in the
/// trade skill window's colours. The name opens the spell in Spells; Edit
/// opens the row in Abilities; Remove deletes the row. Each of those says so
/// in its hover text, and the hover text of a recipe's numbers says what
/// they are, so the section has no line of instructions.
///
/// Under the list, Add Spell… makes a row in the line and opens the
/// spell picker on it, and Show in Abilities lists the line's rows there.
pub(super) fn abilities_block(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    let Some((skill, line_name)) = work.session.table("SkillLine").and_then(|lines| {
        Some((lines.u32_at(record, 0)?, lines.string_at(record, 3).unwrap_or_default()))
    }) else {
        return;
    };
    if !work.session.open_table(work.assets, ability::TABLE) {
        theme::note(ui, "SkillLineAbility did not open");
        return;
    }
    let all = abilities(ui, work, skill);
    let title = match all.len() {
        0 => "Abilities".to_string(),
        n => format!("Abilities ({n})"),
    };
    theme::section(ui, "skill-line-abilities", &title, true, false, |ui| {
        if all.is_empty() {
            note_about(
                ui,
                "No SkillLineAbility row names this skill line, so it holds no spells.",
                "Add a spell with Add Spell\u{2026} below, or from a spell's form under Learning.",
            );
        }
        let filter_id = egui::Id::new(("skill-abilities-filter", skill));
        let mut filter: String = ui.data(|data| data.get_temp(filter_id)).unwrap_or_default();
        if all.len() >= FILTER_FROM {
            ui.add(
                egui::TextEdit::singleline(&mut filter)
                    .hint_text("Name, rank, class or spell ID")
                    .desired_width(280.0),
            );
            ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
        }
        let shown: Vec<&Ability> = all.iter().filter(|line| line.matches(&filter)).collect();
        if shown.is_empty() && !all.is_empty() {
            theme::note(ui, "No ability matches the filter.");
        }
        let mut follow: Option<(&'static str, u32)> = None;
        let mut remove: Option<usize> = None;
        egui::ScrollArea::vertical()
            .id_salt(("skill-abilities-list", skill))
            .max_height(LIST_HEIGHT)
            .auto_shrink([false, true])
            .show_rows(ui, LINE, shown.len(), |ui, range| {
                for line in &shown[range] {
                    ui.allocate_ui(egui::vec2(ui.available_width(), LINE), |ui| {
                        ability_line(ui, work, line, &mut follow, &mut remove);
                    });
                }
            });
        ui.horizontal(|ui| {
            if ui
                .button("Add Spell\u{2026}")
                .on_hover_text(tables::commands::command_about(Command::AddSpellToSkillLine))
                .clicked()
            {
                let ctx = ui.ctx().clone();
                run_command(&ctx, work, "SkillLine", record, Command::AddSpellToSkillLine);
            }
            if !all.is_empty()
                && crate::ui::widgets::reference::link(
                    ui,
                    "Show in Abilities",
                    theme::ACCENT,
                    "Open the Abilities part filtered by this skill line's name.",
                )
            {
                work.browser.back.push(("SkillLine".to_string(), record));
                work.browser.look_at(ability::TABLE);
                work.browser.followed = true;
                work.browser.query = line_name.clone();
            }
        });
        if let Some((table, id)) = follow {
            follow_reference(work, table, id);
        }
        if let Some(row) = remove {
            tables::rows::delete_row(work.session, ability::TABLE, row);
        }
    });
}

/// One line of [`abilities_block`].
fn ability_line(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    line: &Ability,
    follow: &mut Option<(&'static str, u32)>,
    remove: &mut Option<usize>,
) {
    ui.horizontal(|ui| {
        let icon = work
            .browser
            .record_of(work.session, "Spell", line.spell)
            .and_then(|at| work.session.table("Spell")?.u32_at(at, schema::SPELL_ICON_FIELD))
            .and_then(|icon| work.icon_by_id(icon));
        work.icon(ui, icon, ICON_SIDE);
        if crate::ui::widgets::reference::link(
            ui,
            &line.name,
            theme::ACCENT,
            &format!("Open Spell {} in Spells.", line.spell),
        ) {
            *follow = Some(("Spell", line.spell));
        }
        if !line.rank.is_empty() {
            ui.label(egui::RichText::new(&line.rank).small().color(theme::INK_DIM));
        }
        if !line.who.is_empty() {
            ui.label(egui::RichText::new(&line.who).small().color(theme::INK_DIM));
        }
        if line.with_line {
            ui.label(egui::RichText::new("Learned with the line").small().color(theme::INK_FAINT))
                .on_hover_text("LearnOnGetSkill is set: a character that gains the skill line learns this spell.");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("Remove")
                .on_hover_text("Remove this SkillLineAbility row. One undo entry.")
                .clicked()
            {
                *remove = Some(line.row);
            }
            if ui
                .small_button("Edit")
                .on_hover_text(format!("Open SkillLineAbility {} with every field.", line.id))
                .clicked()
            {
                *follow = Some((ability::TABLE, line.id));
            }
            if line.is_recipe() {
                ui.add_space(8.0);
                let points = recipe_points(line.min, line.max);
                // The hover says what the numbers are: the section has no
                // line of instructions to say it.
                let hover = format!(
                    "The skill values at which the recipe turns yellow, green and grey: yellow \
                     from {}, green from {}, grey from {}, by MinValue and MaxValue. \
                     ReqSkillValue is {}, and is drawn in orange before them when it is above 1.",
                    points[0].0, points[1].0, points[2].0, line.req
                );
                ui.scope(|ui| {
                    // Right to left, so grey is drawn first.
                    for (value, difficulty) in points.into_iter().rev() {
                        ui.label(theme::number(value.to_string()).small().color(difficulty_colour(difficulty)));
                    }
                    // `ReqSkillValue` is 1 on almost every shipped row, so it
                    // is drawn only where it says something.
                    if line.req > 1 {
                        ui.label(
                            theme::number(line.req.to_string())
                                .small()
                                .color(difficulty_colour(Difficulty::Optimal)),
                        );
                    }
                })
                .response
                .on_hover_text(hover);
            }
        });
    });
}

/// A `SkillTiers` row's steps, in place of its Steps section's 32 columns:
/// one line per step with its `MaxValue` and its `Cost`, and an empty line
/// after the last step to add the next one. Remove on the last step clears
/// both of its columns, as one undo entry. The first line says how many of
/// the steps are used, and its info mark says what a tier is and how a step
/// is added.
pub(super) fn steps_block(ui: &mut egui::Ui, work: &mut Workspace<'_>, record: usize) {
    const TABLE: &str = "SkillTiers";
    let Some(table) = work.session.table(TABLE) else {
        return;
    };
    let steps: Vec<(u32, u32)> = (0..schema::SKILL_TIER_STEPS)
        .map(|step| {
            (
                table.u32_at(record, schema::SKILL_TIER_MAX + step).unwrap_or(0),
                table.u32_at(record, schema::SKILL_TIER_COST + step).unwrap_or(0),
            )
        })
        .collect();
    let used = steps
        .iter()
        .rposition(|&(max, cost)| max != 0 || cost != 0)
        .map_or(0, |last| last + 1);
    let title = match used {
        0 => "Steps".to_string(),
        n => format!("Steps ({n})"),
    };
    theme::section(ui, "skill-tier-steps", &title, true, false, |ui| {
        note_about(
            ui,
            format!("{used} of {} steps used", schema::SKILL_TIER_STEPS),
            "A skill line whose race/class row names this tier is ranked: its maximum skill \
             value rises one step at a time, as a profession's goes 75, 150, 225 and 300. \
             Type into the empty line to add a step.",
        );
        let mut clear: Option<usize> = None;
        egui::Grid::new(("skill-tier-steps-grid", record))
            .num_columns(4)
            .spacing(egui::vec2(12.0, 6.0))
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Step").small().color(theme::INK_DIM));
                ui.label(egui::RichText::new("MaxValue").small().color(theme::INK_DIM));
                ui.label(egui::RichText::new("Cost").small().color(theme::INK_DIM));
                ui.end_row();
                for (step, &(max, cost)) in steps.iter().enumerate().take((used + 1).min(schema::SKILL_TIER_STEPS)) {
                    let empty = step == used;
                    ui.label(egui::RichText::new(format!("{}", step + 1)).color(match empty {
                        true => theme::INK_FAINT,
                        false => theme::INK,
                    }));
                    for (field, value) in [(schema::SKILL_TIER_MAX + step, max), (schema::SKILL_TIER_COST + step, cost)] {
                        let mut typed = value as i64;
                        let changed = theme::sunk(ui, |ui| {
                            ui.add_sized(
                                egui::vec2(STEP_VALUE, 22.0),
                                egui::DragValue::new(&mut typed).range(0..=i64::from(u32::MAX)),
                            )
                        })
                        .changed();
                        if changed && typed as u32 != value {
                            let name = schema::SKILL_TIERS.column(field).map_or("", |column| column.name);
                            tables::fields::set_field(
                                work.session,
                                TABLE,
                                record,
                                field,
                                typed as u32,
                                &format!("Edit {name}"),
                                work.now,
                            );
                        }
                    }
                    if step + 1 == used
                        && ui
                            .small_button("Remove")
                            .on_hover_text("Set this step's MaxValue and Cost to 0. One undo entry.")
                            .clicked()
                    {
                        clear = Some(step);
                    }
                    ui.end_row();
                }
            });
        if let Some(step) = clear {
            tables::fields::set_fields(
                work.session,
                TABLE,
                record,
                &[(schema::SKILL_TIER_MAX + step, 0), (schema::SKILL_TIER_COST + step, 0)],
                "Remove a tier step",
                &format!("{TABLE} {record} remove step {step}"),
                work.now,
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::tables::tradeskill::Thresholds;

    /// The points [`recipe_points`] names are where the trade skill window's
    /// own rule changes colour, with and without a stated `MinValue`.
    #[test]
    fn a_recipes_colour_points_are_where_its_colour_changes() {
        for (min, max) in [(0, 115), (95, 135), (300, 320), (0, 10)] {
            let rule = Thresholds { yellow_at: min, grey_at: max, ..Default::default() };
            let mut before = Difficulty::Optimal;
            for (value, difficulty) in recipe_points(min, max) {
                assert_eq!(rule.difficulty(value), difficulty, "{min} {max} at {value}");
                if value > 0 {
                    assert!(rule.difficulty(value - 1) <= before, "{min} {max} below {value}");
                }
                before = difficulty;
            }
        }
    }
}
