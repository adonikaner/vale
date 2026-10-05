//! The condition window, and the cell every form draws a condition column
//! with.
//!
//! The window shows one row of `conditions` at a time: its type, its four
//! values named and resolved by what the type says each is, its two flags, and
//! the conditions it combines or is combined into. Its list page searches
//! every row. The window is drawn whatever the tool.
//!
//! A form draws a column that names a condition (`condition_id`,
//! `required_condition`, `RequiredCondition`, a script step's condition
//! parameters) with its number and then [`cell`]: what the condition tests,
//! as a link that opens it, and buttons that choose an existing condition or
//! make one. The window writes a made or chosen entry back into the column
//! that opened it. The messages between the two are in
//! `crate::tools::conditions`; the forms carry none of the window's state.

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
use crate::tools::conditions::{self, Asker, Board, Conditions, Request, Shown, Told};
use crate::tools::quests::{ColumnTarget, Holder, PickFor, Picker, Quests};
use vale_client::assets::GameAssets;
use vale_mangos::condition::{self as rows, Condition, Means};
use vale_mangos::row::Life;
use vale_mangos::schema::Value;

/// How much of a form row the buttons after the line take, in points.
const BUTTONS: f32 = 110.0;

/// What a form draws after the number of a column that names a condition:
/// what the condition tests, as a link that opens it in the condition window,
/// then `choose…`, which opens the window's list, and for an empty column
/// `+ new`, which opens the page that makes one. Answers the entry the
/// window made or chose for the column, which the caller writes as it writes
/// a typed number.
///
/// `reply` identifies the column: the same on every frame, and different for
/// every column a form draws. `column` is what the window calls it
/// (`npc_vendor 54 2488 condition_id`). `width` fixes the line's width, for a
/// row of fixed cells; `None` lets it take what the row has left.
pub fn cell(ui: &mut egui::Ui, reply: egui::Id, column: &str, entry: u32, width: Option<f32>) -> Option<u32> {
    let answered = conditions::take_answer(ui.ctx(), reply).filter(|answered| *answered != entry);
    if entry != 0 {
        conditions::ask_about(ui.ctx(), entry);
    }
    let asker = Some(Asker { reply, column: column.to_string(), holds: entry });
    let (text, colour, hover) = line(&conditions::board(ui.ctx()), entry);
    let room = width.unwrap_or_else(|| (ui.available_width() - BUTTONS).max(80.0));
    let height = ui.spacing().interact_size.y;
    let opened = ui
        .allocate_ui_with_layout(egui::vec2(room, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_max_width(room);
            if width.is_some() {
                ui.set_min_width(room);
            }
            match entry {
                0 => {
                    ui.add(egui::Label::new(egui::RichText::new(text).small().color(colour)).truncate())
                        .on_hover_text(hover);
                    false
                }
                _ => reference::link(ui, &text, colour, &hover),
            }
        })
        .inner;
    if opened {
        conditions::ask(ui.ctx(), Request { entry, list: false, asker: asker.clone() });
    }
    if ui
        .small_button("choose\u{2026}")
        .on_hover_text("Search the conditions in the condition window, and use one in this column.")
        .clicked()
    {
        conditions::ask(ui.ctx(), Request { entry, list: true, asker: asker.clone() });
    }
    if entry == 0
        && ui
            .small_button("+ new")
            .on_hover_text("Make a condition in the condition window. Its entry is written into this column.")
            .clicked()
    {
        conditions::ask(ui.ctx(), Request { entry: 0, list: false, asker });
    }
    answered
}

/// The line a cell draws for `entry`: its text, its colour and its hover.
fn line(board: &Board, entry: u32) -> (String, egui::Color32, String) {
    let open = "Click to open it in the condition window.";
    if entry == 0 {
        return ("none".to_string(), theme::INK_FAINT, "0: no condition.".to_string());
    }
    if let Some(trouble) = &board.trouble {
        return (
            format!("condition {entry}"),
            theme::ACCENT,
            format!("The conditions could not be read: {trouble}\n{open}"),
        );
    }
    match board.told.get(&entry) {
        Some(Told::Tests { line, faults }) if faults.is_empty() => {
            (line.clone(), theme::ACCENT, format!("Condition {entry}: {line}\n{open}"))
        }
        Some(Told::Tests { line, faults }) => (
            line.clone(),
            theme::WARN,
            format!("Condition {entry}: {line}\nThe server would skip it: {}\n{open}", faults.join("; ")),
        ),
        Some(Told::Missing) => (
            format!("no condition {entry}"),
            theme::BAD,
            format!("Neither the conditions table nor this project holds condition {entry}.\n{open}"),
        ),
        Some(Told::Removed) => (
            format!("condition {entry}, removed"),
            theme::BAD,
            format!("This project removes condition {entry}, and this column still names it.\n{open}"),
        ),
        None => (format!("condition {entry}"), theme::INK_DIM, format!("Reading the conditions.\n{open}")),
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

/// The frame the window is drawn in, the same as the other server windows'.
fn frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::SHELL)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(8))
}

/// Take a form's request, tell the forms what the conditions they show test,
/// and draw the window when it is open.
pub fn window(ctx: &egui::Context, mut subject: Subject<'_>) -> Option<egui::Rect> {
    if let Some(request) = ctx.data_mut(|data| data.remove_temp::<Request>(conditions::request_id())) {
        subject.conditions.take(request);
    }
    tell(ctx, &mut subject);
    if !subject.conditions.open {
        return None;
    }
    let mut keep_open = true;
    let title = match (subject.conditions.listing, subject.conditions.showing) {
        (true, _) => "Conditions".to_string(),
        (false, Some(entry)) => format!("Condition {entry}"),
        (false, None) => "New condition".to_string(),
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

/// Answer the forms: what each entry they showed since the last draw tests.
/// Keeps the table read while any form shows one.
fn tell(ctx: &egui::Context, subject: &mut Subject<'_>) {
    let asked = conditions::take_asked(ctx);
    subject.conditions.wanted = !asked.is_empty();
    let mut board = Board {
        trouble: subject.conditions.trouble.clone(),
        read: subject.conditions.read(),
        told: Default::default(),
    };
    if board.read {
        for entry in asked {
            let told = match subject.conditions.shown(&subject.session.server_edits, entry) {
                None => Told::Missing,
                Some(shown) if shown.life == Life::Delete => Told::Removed,
                Some(shown) => Told::Tests {
                    line: sentence(subject, &shown.condition),
                    faults: subject.conditions.row_faults(&subject.session.server_edits, &shown.condition),
                },
            };
            board.told.insert(entry, told);
        }
    }
    conditions::post_board(ctx, board);
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
        if ui.button("List").on_hover_text("Search every condition by entry, value or type.").clicked() {
            subject.conditions.list();
        }
    });
    asker_line(ui, subject);
    ui.separator();
    if let Some(trouble) = &subject.conditions.trouble {
        theme::note(ui, format!("The conditions could not be read: {trouble}"));
        return;
    }
    if !subject.conditions.read() {
        theme::note(ui, "reading conditions\u{2026}");
        return;
    }
    if subject.conditions.listing {
        list_page(ui, subject);
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

/// The column the window answers into, when a form opened it, and the button
/// that stops answering it.
fn asker_line(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    let Some(asker) = subject.conditions.asker.clone() else {
        return;
    };
    ui.horizontal(|ui| {
        let holds = match asker.holds {
            0 => "no condition".to_string(),
            entry => format!("condition {entry}"),
        };
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("For {}, which holds {holds}", asker.column))
                    .small()
                    .color(theme::INK_DIM),
            )
            .truncate(),
        );
        if ui
            .small_button("\u{d7}")
            .on_hover_text("Stop answering this column: Use and Make no longer write into it.")
            .clicked()
        {
            subject.conditions.asker = None;
        }
    });
}

/// Write `entry` into the column that opened the window, and say so.
fn use_for_column(ctx: &egui::Context, subject: &mut Subject<'_>, entry: u32) {
    if let Some(column) = subject.conditions.asker.as_ref().map(|asker| asker.column.clone()) {
        conditions::answer(ctx, subject.conditions, entry);
        subject.session.status = format!("{column} set to condition {entry}");
    }
}

/// Whether the window answers a column that holds something other than
/// `entry`, so Use is offered.
fn offers_use(subject: &Subject<'_>, entry: u32) -> bool {
    subject.conditions.asker.as_ref().is_some_and(|asker| asker.holds != entry)
}

/// Every condition the search matches, one line each.
fn list_page(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut subject.conditions.search)
                .hint_text("entry, value or type")
                .desired_width(160.0),
        )
        .on_hover_text("A number matches a condition's entry or any of its four values; text matches part of a type's name.");
        let name = match subject.conditions.search_type.and_then(rows::type_of) {
            Some(known) => format!("{} {}", known.id, known.name),
            None => "every type".to_string(),
        };
        egui::ComboBox::from_id_salt("condition-list-type").selected_text(name).height(360.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut subject.conditions.search_type, None, "every type");
            for known in rows::TYPES.iter().filter(|known| known.id != 0) {
                ui.selectable_value(&mut subject.conditions.search_type, Some(known.id), format!("{:>3} {}", known.id, known.name));
            }
        });
    });
    let found = subject.conditions.matching(
        &subject.session.server_edits,
        &subject.conditions.search,
        subject.conditions.search_type,
    );
    meaning(ui, format!("{} conditions", found.len()));
    let height = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, height, found.len(), |ui, range| {
        for condition in &found[range] {
            let entry = condition.entry;
            ui.horizontal(|ui| {
                ui.set_min_height(height);
                if ui.link(entry.to_string()).on_hover_text("Open this condition.").clicked() {
                    subject.conditions.show(Some(entry));
                }
                if offers_use(subject, entry)
                    && ui.small_button("use").on_hover_text("Write this condition into the column the window was opened from.").clicked()
                {
                    use_for_column(ui.ctx(), subject, entry);
                }
                let said = sentence(subject, condition);
                ui.add(egui::Label::new(egui::RichText::new(said).color(theme::INK)).truncate());
            });
        }
    });
}

/// Choose a type and make a condition of it.
fn new_page(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    theme::note(
        ui,
        "A new condition takes the next entry above every other, which is also above any \
         condition an AND or an OR names, as the server requires.",
    );
    if subject.conditions.asker.as_ref().is_some_and(|asker| asker.holds == 0) {
        theme::note(ui, "Make writes the new condition's entry into the column the window was opened from.");
    }
    row(ui, "type", |ui| type_choice(ui, &mut subject.conditions.new_type, "new-condition-type"));
    if let Some(kind) = rows::type_of(subject.conditions.new_type) {
        meaning(ui, kind.about);
    }
    if ui.button("Make").clicked() {
        let kind = subject.conditions.new_type;
        let made = subject.conditions.create(subject.session, kind, [0; 4], 0, subject.now);
        if let Some((entry, new)) = made {
            subject.session.status = match new {
                true => format!("condition {entry} made"),
                false => format!("condition {entry} already tests that"),
            };
            if subject.conditions.asker.as_ref().is_some_and(|asker| asker.holds == 0) {
                use_for_column(ui.ctx(), subject, entry);
            }
            subject.conditions.show(Some(entry));
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
        if shown.life != Life::Delete
            && offers_use(subject, condition.entry)
            && ui
                .button("Use")
                .on_hover_text("Write this condition into the column the window was opened from.")
                .clicked()
        {
            use_for_column(ui.ctx(), subject, condition.entry);
        }
        if ui
            .button("Combine with AND\u{2026}")
            .on_hover_text(
                "Make an AND over this condition and a new one, entered above both. When the \
                 column the window was opened from holds this condition, it is set to the AND.",
            )
            .clicked()
        {
            combine(ui.ctx(), subject, condition.entry, -1);
        }
        if ui.button("Combine with OR\u{2026}").on_hover_text("The same, with OR.").clicked() {
            combine(ui.ctx(), subject, condition.entry, -2);
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
/// new one, since it is the one to fill in. The column the window was opened
/// from is set to the AND or the OR when it held `entry`.
fn combine(ctx: &egui::Context, subject: &mut Subject<'_>, entry: u32, kind: i32) {
    let now = subject.now;
    let Some((child, _)) = subject.conditions.create(subject.session, 8, [0; 4], 0, now) else {
        return;
    };
    if let Some((parent, _)) = subject.conditions.create(subject.session, kind, [entry as i32, child as i32, 0, 0], 0, now) {
        let asker = subject.conditions.asker.clone().filter(|asker| asker.holds == entry);
        subject.session.status = match asker {
            Some(asker) => {
                conditions::answer(ctx, subject.conditions, parent);
                format!("condition {parent} combines {entry} with the new condition {child}; {} set to {parent}", asker.column)
            }
            None => format!(
                "condition {parent} combines {entry} with the new condition {child}; name {parent} where {entry} was named"
            ),
        };
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

/// The table a value with this [`Means`] names, for its name and its picker.
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

/// What a value names, by name: a quest's title, an item's, creature's or
/// game object's name, a DBC row's name. `None` while the name is
/// being read, or when the value names nothing.
fn name_of(subject: &mut Subject<'_>, means: Means, value: i32) -> Option<String> {
    let id = u32::try_from(value).ok().filter(|id| *id != 0)?;
    let edits = &subject.session.server_edits;
    match means {
        Means::Quest => subject.quests.title_of(id, edits),
        Means::Item => subject.quests.item(id, edits).map(|item| item.name),
        Means::Creature => subject.quests.holder(Holder::Creature, id, edits),
        Means::GameObject => subject.quests.holder(Holder::Object, id, edits),
        _ => {
            let table = table_of(means)?;
            match super::quests::open_for_names(subject.session, subject.assets, table) {
                true => super::quests::name_in(subject.session, table, id),
                false => None,
            }
        }
    }
}

/// The condition in one line: its type and what each value is.
fn sentence(subject: &mut Subject<'_>, condition: &Condition) -> String {
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
            means => match name_of(subject, means, value) {
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
