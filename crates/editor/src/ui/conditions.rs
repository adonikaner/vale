//! The condition window: one row of `conditions` at a time, its type, its four
//! values named and resolved by what the type says each is, its two flags, and
//! the conditions it combines or is combined into.
//!
//! Any form opens it on an entry with [`open_button`], which posts the entry
//! for this window to take on the next frame (`crate::tools::conditions`), so
//! the forms carry none of its state. The window is drawn whatever the tool.

use bevy_egui::egui;

use super::reference::{self, Resolver};
use super::rowform::{choice_cell, flags_cell, meaning, page_row, page_spacing};

/// A form row with a name cell and no hover text.
fn row<R>(ui: &mut egui::Ui, name: &str, contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    page_row(ui, name, "", false, contents)
}
use super::theme;
use super::thumbnails::Thumbnails;
use crate::session::EditSession;
use crate::tools::conditions::{self, Conditions, Shown};
use crate::tools::quests::{ColumnTarget, PickFor, Picker, Quests};
use vale_client::assets::GameAssets;
use vale_mangos::condition::{self as rows, Condition, Means};
use vale_mangos::row::Life;
use vale_mangos::schema::Value;

/// The small button a form draws beside a column that names a condition: it
/// opens the condition, or the page that makes one when the column is 0.
pub fn open_button(ui: &mut egui::Ui, entry: u32) {
    let (label, tip) = match entry {
        0 => ("New\u{2026}", "Open the condition window on a new condition. Its entry goes into this column by hand."),
        _ => ("Open", "Open this condition in the condition window."),
    };
    if ui.small_button(label).on_hover_text(tip).clicked() {
        conditions::ask_to_open(ui.ctx(), entry);
    }
}

/// What the window is drawn from.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub conditions: &'a mut Conditions,
    pub quests: &'a mut Quests,
    pub assets: &'a GameAssets,
    pub thumbnails: &'a mut Thumbnails,
    pub now: f64,
}

/// The frame the window is drawn in, the other server windows' own.
fn frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::SHELL)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(8))
}

/// Take a form's request, and draw the window when it is open.
pub fn window(ctx: &egui::Context, mut subject: Subject<'_>) -> Option<egui::Rect> {
    if let Some(entry) = ctx.data_mut(|data| data.remove_temp::<u32>(conditions::request_id())) {
        subject.conditions.show((entry != 0).then_some(entry));
    }
    if !subject.conditions.open {
        return None;
    }
    let mut keep_open = true;
    let title = match subject.conditions.showing {
        Some(entry) => format!("Condition {entry}"),
        None => "New condition".to_string(),
    };
    let shown = egui::Window::new(title)
        .id(egui::Id::new("condition-window"))
        .open(&mut keep_open)
        .default_size([520.0, 520.0])
        .default_pos([380.0, 140.0])
        .resizable(true)
        .frame(frame())
        .show(ctx, |ui| {
            page_spacing(ui);
            contents(ui, &mut subject);
        });
    if !keep_open {
        subject.conditions.open = false;
    }
    super::quests::picker(ctx, subject.session, subject.quests, subject.assets, subject.thumbnails, None, subject.now);
    shown.map(|response| response.response.rect)
}

fn contents(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!subject.conditions.trail.is_empty(), egui::Button::new("Back"))
            .clicked()
        {
            subject.conditions.back();
        }
        ui.add(egui::DragValue::new(&mut subject.conditions.typed).speed(1.0));
        if ui.button("Open").on_hover_text("Open the condition with this entry.").clicked() {
            let typed = subject.conditions.typed;
            subject.conditions.show(Some(typed));
        }
        if ui.button("New\u{2026}").on_hover_text("Make a new condition.").clicked() {
            subject.conditions.show(None);
        }
    });
    ui.separator();
    if let Some(trouble) = &subject.conditions.trouble {
        theme::note(ui, format!("The conditions could not be read: {trouble}"));
        return;
    }
    if !subject.conditions.read() {
        theme::note(ui, "reading conditions\u{2026}");
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match subject.conditions.showing {
        None => new_page(ui, subject),
        Some(entry) => match subject.conditions.shown(&subject.session.server_edits, entry) {
            Some(shown) => condition_page(ui, subject, shown),
            None => theme::note(ui, format!("No condition {entry} in the table or the project.")),
        },
    });
}

/// Choose a type and make a condition of it.
fn new_page(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    theme::note(
        ui,
        "A new condition takes the next entry above every other, which is also above any \
         condition an AND or an OR names, as the server requires.",
    );
    row(ui, "type", |ui| type_choice(ui, &mut subject.conditions.new_type, "new-condition-type"));
    if let Some(kind) = rows::type_of(subject.conditions.new_type) {
        meaning(ui, kind.about);
    }
    if ui.button("Make").clicked() {
        let kind = subject.conditions.new_type;
        match subject.conditions.create(subject.session, kind, [0; 4], 0, subject.now) {
            Some((entry, true)) => {
                subject.session.status = format!("condition {entry} made");
                subject.conditions.show(Some(entry));
            }
            Some((entry, false)) => {
                subject.session.status = format!("condition {entry} already tests that");
                subject.conditions.show(Some(entry));
            }
            None => {}
        }
    }
}

/// The type list, every type by id and name.
fn type_choice(ui: &mut egui::Ui, kind: &mut i32, salt: &str) -> bool {
    let name = rows::type_of(*kind).map_or_else(|| format!("{kind} (unknown)"), |known| format!("{} {}", known.id, known.name));
    let before = *kind;
    egui::ComboBox::from_id_salt(salt).selected_text(name).height(360.0).show_ui(ui, |ui| {
        for known in rows::TYPES.iter().filter(|known| known.id != 0) {
            ui.selectable_value(kind, known.id, format!("{:>3} {}", known.id, known.name));
        }
    });
    *kind != before
}

fn condition_page(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: Shown) {
    let condition = shown.condition.clone();
    let now = subject.now;
    match shown.life {
        Life::Insert => theme::note(ui, "Made by this project."),
        Life::Delete => theme::note(ui, "Removed by this project."),
        Life::Update => {}
    }
    let mut kind = condition.kind;
    row(ui, "type", |ui| {
        if type_choice(ui, &mut kind, "condition-type") {
            subject.conditions.set(subject.session, &shown, "type", kind.to_string(), now);
        }
    });
    let known = rows::type_of(condition.kind);
    if let Some(known) = known {
        meaning(ui, format!("{} Needs {}.", known.about, known.needs));
    }
    for (n, column) in ["value1", "value2", "value3", "value4"].into_iter().enumerate() {
        let Some(slot) = known.and_then(|known| known.values[n]) else {
            if condition.values[n] != 0 {
                row(ui, column, |ui| {
                    ui.label(theme::number(condition.values[n].to_string()));
                    meaning(ui, "not read by this type");
                });
            }
            continue;
        };
        row(ui, slot.label, |ui| value_cell(ui, subject, &shown, n, column, slot.means));
    }
    let mut reverse = condition.flags & 0x1 != 0;
    let mut swap = condition.flags & 0x2 != 0;
    row(ui, "flags", |ui| {
        let a = ui.checkbox(&mut reverse, "reverse the result").changed();
        let b = ui.checkbox(&mut swap, "swap target and source").changed();
        if a || b {
            let flags = u8::from(reverse) | (u8::from(swap) << 1);
            subject.conditions.set(subject.session, &shown, "flags", flags.to_string(), now);
        }
    });
    ui.label(egui::RichText::new(sentence(subject, &condition)).color(theme::INK));

    for fault in subject.conditions.faults(&subject.session.server_edits, &condition) {
        ui.label(egui::RichText::new(fault).size(theme::SMALL).color(theme::WARN));
    }
    let users = subject.conditions.users(&subject.session.server_edits, condition.entry);
    if !users.is_empty() {
        ui.add_space(4.0);
        theme::heading(ui, "Combined into");
        ui.horizontal_wrapped(|ui| {
            for user in users {
                if ui.link(format!("condition {user}")).clicked() {
                    subject.conditions.show(Some(user));
                }
            }
        });
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .button("Combine with AND\u{2026}")
            .on_hover_text("Make an AND over this condition and a new one, entered above both.")
            .clicked()
        {
            combine(subject, condition.entry, -1);
        }
        if ui.button("Combine with OR\u{2026}").on_hover_text("The same, with OR.").clicked() {
            combine(subject, condition.entry, -2);
        }
        let label = match shown.life {
            Life::Delete => "Keep",
            _ => "Remove",
        };
        if ui
            .button(label)
            .on_hover_text("Other tables that name the condition keep its number; check them first.")
            .clicked()
        {
            match shown.life {
                Life::Delete => {
                    let key = condition.key();
                    let subject_line = format!("{} {}", rows::TABLE, key.text());
                    subject.session.set_server_row(
                        rows::TABLE,
                        &key,
                        None,
                        Some(crate::session::Gesture { label: "Keep condition", subject: &subject_line, now }),
                    );
                }
                _ => subject.conditions.remove(subject.session, &shown, now),
            }
        }
    });
}

/// Make an AND or an OR over `entry` and a new empty condition, and open the
/// new one, since it is the one to fill in.
fn combine(subject: &mut Subject<'_>, entry: u32, kind: i32) {
    let now = subject.now;
    let Some((child, _)) = subject.conditions.create(subject.session, 8, [0; 4], 0, now) else {
        return;
    };
    if let Some((parent, _)) = subject.conditions.create(subject.session, kind, [entry as i32, child as i32, 0, 0], 0, now) {
        subject.session.status = format!(
            "condition {parent} combines {entry} with the new condition {child}; name {parent} where {entry} was named"
        );
    }
    subject.conditions.show(Some(child));
}

/// One value, drawn as what the type says it is.
fn value_cell(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &Shown, n: usize, column: &'static str, means: Means) {
    let now = subject.now;
    let value = shown.condition.values[n];
    let showing = value.to_string();
    let salt = egui::Id::new(("condition-value", shown.condition.entry, n));
    let choices: Option<&[Value]> = match means {
        Means::Compare => Some(&COMPARES),
        Means::Team => Some(&TEAMS),
        Means::Rank => Some(&RANKS),
        Means::QuestState => Some(&QUEST_STATES),
        Means::Gender => Some(&GENDERS),
        Means::Bool => Some(&NO_YES),
        _ => None,
    };
    let written = match (choices, means) {
        (Some(values), _) => choice_cell(ui, &showing, values, salt),
        (None, Means::RaceMask) => flags_cell(ui, &showing, &vale_mangos::item::RACE_MASK, salt),
        (None, Means::ClassMask) => flags_cell(ui, &showing, &vale_mangos::item::CLASS_MASK, salt),
        _ => {
            let mut number = value as i64;
            let changed = ui
                .add(egui::DragValue::new(&mut number).range(i64::from(i32::MIN)..=i64::from(i32::MAX)))
                .changed();
            changed.then(|| number.to_string())
        }
    };
    if let Some(written) = written {
        subject.conditions.set(subject.session, shown, column, written, now);
    }
    match means {
        Means::Condition => {
            if value > 0 {
                let child = value as u32;
                let named = subject
                    .conditions
                    .shown(&subject.session.server_edits, child)
                    .and_then(|child| rows::type_of(child.condition.kind).map(|kind| kind.name))
                    .unwrap_or("missing");
                if ui.link(format!("{child}: {named}")).clicked() {
                    subject.conditions.show(Some(child));
                }
            }
        }
        _ => {
            if let Some(table) = table_of(means) {
                let target = ColumnTarget {
                    table: rows::TABLE,
                    key: shown.condition.key(),
                    column,
                    in_database: shown.in_database.as_ref().map(|held| held.values[n].to_string()),
                    label: "Edit condition",
                    subject: format!("{} {} {column}", rows::TABLE, shown.condition.key().text()),
                };
                if let Some(kind) = super::quests::target_of(table) {
                    if ui.small_button("\u{2026}").on_hover_text("choose by name").clicked() {
                        subject.quests.picker = Some(Picker::new(kind, PickFor::ServerColumn(target)));
                    }
                }
                if value > 0 {
                    let mut resolver = Resolver { session: subject.session, assets: subject.assets, quests: subject.quests };
                    reference::name(ui, &mut resolver, table, value as u32);
                }
            }
        }
    }
}

/// The table a value of this kind names, for its name and its picker.
fn table_of(means: Means) -> Option<&'static str> {
    match means {
        Means::Spell => Some("Spell"),
        Means::Item => Some(vale_mangos::item::TEMPLATE),
        Means::Quest => Some(vale_mangos::quest::TEMPLATE),
        Means::Creature => Some(vale_mangos::creature::TEMPLATE),
        Means::GameObject => Some(vale_mangos::gameobject::TEMPLATE),
        Means::Area => Some("AreaTable"),
        Means::Faction => Some("Faction"),
        Means::Skill => Some("SkillLine"),
        Means::Map => Some("Map"),
        _ => None,
    }
}

/// The condition in one line: its type and what each value is.
fn sentence(subject: &Subject<'_>, condition: &Condition) -> String {
    let Some(known) = rows::type_of(condition.kind) else {
        return format!("type {}", condition.kind);
    };
    let mut parts: Vec<String> = Vec::new();
    for (n, slot) in known.values.iter().enumerate() {
        let Some(slot) = slot else { continue };
        let value = condition.values[n];
        if value == 0 && slot.label.contains("optional") {
            continue;
        }
        let said = match slot.means {
            Means::Compare => rows::COMPARES.get(value as usize).copied().unwrap_or("?").to_string(),
            Means::Rank => rows::RANKS.get(value as usize).copied().unwrap_or("?").to_string(),
            Means::Condition => value.to_string(),
            _ => match table_of(slot.means).and_then(|table| super::quests::name_in(subject.session, table, value as u32)) {
                Some(name) => format!("{name} ({value})"),
                None => value.to_string(),
            },
        };
        parts.push(format!("{} {said}", slot.label));
    }
    let head = match condition.reversed() {
        true => format!("Not: {}", known.name),
        false => known.name.to_string(),
    };
    match parts.is_empty() {
        true => head,
        false => format!("{head}: {}", parts.join(", ")),
    }
}

const COMPARES: [Value; 3] = [
    Value { value: 0, name: "equal to" },
    Value { value: 1, name: "at least" },
    Value { value: 2, name: "at most" },
];

const TEAMS: [Value; 2] = [Value { value: 469, name: "Alliance" }, Value { value: 67, name: "Horde" }];

const RANKS: [Value; 8] = [
    Value { value: 0, name: "Hated" },
    Value { value: 1, name: "Hostile" },
    Value { value: 2, name: "Unfriendly" },
    Value { value: 3, name: "Neutral" },
    Value { value: 4, name: "Friendly" },
    Value { value: 5, name: "Honored" },
    Value { value: 6, name: "Revered" },
    Value { value: 7, name: "Exalted" },
];

const QUEST_STATES: [Value; 3] = [
    Value { value: 0, name: "any state" },
    Value { value: 1, name: "incomplete" },
    Value { value: 2, name: "complete" },
];

const GENDERS: [Value; 3] = [Value { value: 0, name: "male" }, Value { value: 1, name: "female" }, Value { value: 2, name: "none" }];

const NO_YES: [Value; 2] = [Value { value: 0, name: "no" }, Value { value: 1, name: "yes" }];
