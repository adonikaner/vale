//! The condition window, and the cell every form draws a condition column
//! with.
//!
//! The window shows one condition at a time in one of two views. Tests draws
//! it as a tree, in the event window's cards: a WHEN group (all of, any of,
//! none of, or not all of these), and under it each test as a sentence that
//! unfolds into its type, chosen from a menu by category, its values and its
//! flags, and nested groups. The tree is a draft until Save, which writes
//! new rows for what changed (`crate::tools::conditions::Draft`). Rows draws
//! the one row: its type, its four values named and resolved by what the type
//! says each is, its two flags, and the conditions it combines or is combined
//! into, each change written to the row where it is. The list page searches
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
use crate::tools::conditions::{self, Asker, Board, Conditions, Draft, Request, Shown, Told};
use crate::tools::quests::{ColumnTarget, Holder, PickFor, Picker, Quests};
use vale_client::assets::GameAssets;
use vale_mangos::condition::{self as rows, Means, Node};
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
        .on_hover_text("Open the condition window\'s list and choose a condition for this column.")
        .clicked()
    {
        conditions::ask(ui.ctx(), Request { entry, list: true, asker: asker.clone() });
    }
    if entry == 0
        && ui
            .small_button("+ new")
            .on_hover_text("Create a condition in the condition window and write its entry to this column.")
            .clicked()
    {
        conditions::ask(ui.ctx(), Request { entry: 0, list: false, asker });
    }
    answered
}

/// The line a cell draws for `entry`: its text, its colour and its hover.
fn line(board: &Board, entry: u32) -> (String, egui::Color32, String) {
    let open = "Click to open the condition in the condition window.";
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
            format!("Condition {entry}: {line}\nThe server skips this condition: {}\n{open}", faults.join("; ")),
        ),
        Some(Told::Missing) => (
            format!("no condition {entry}"),
            theme::BAD,
            format!("Neither the conditions table nor this project holds condition {entry}.\n{open}"),
        ),
        Some(Told::Removed) => (
            format!("condition {entry}, removed"),
            theme::BAD,
            format!("This project removes condition {entry}, but this column still refers to it.\n{open}"),
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
        .default_size([640.0, 600.0])
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
                    line: capital(&describe_entry(subject, entry)),
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
        if ui.button("New\u{2026}").on_hover_text("Create a new condition.").clicked() {
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
    let Some(entry) = subject.conditions.showing else {
        tree_page(ui, subject, 0);
        return;
    };
    ui.allocate_ui(egui::vec2(220.0, 24.0), |ui| {
        theme::segmented(ui, &mut subject.conditions.rows_view, &[("Tests", false), ("Rows", true)], |a, b| a == b);
    })
    .response
    .on_hover_text(
        "Tests shows the condition as the tests it is made of; Save writes new rows. Rows shows this one conditions row and edits its columns in place.",
    );
    ui.add_space(4.0);
    if !subject.conditions.rows_view {
        tree_page(ui, subject, entry);
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        match subject.conditions.shown(&subject.session.server_edits, entry) {
            Some(shown) => condition_page(ui, subject, shown),
            None => theme::note(ui, format!("No condition {entry} in the table or the project.")),
        }
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
                egui::RichText::new(format!("Writes to {}, which holds {holds}", asker.column))
                    .small()
                    .color(theme::INK_DIM),
            )
            .truncate(),
        );
        if ui
            .small_button("\u{d7}")
            .on_hover_text("Detach this column: Use and Save no longer write to it.")
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
                let said = capital(&describe_entry(subject, entry));
                ui.add(egui::Label::new(egui::RichText::new(said).color(theme::INK)).truncate());
            });
        }
    });
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
        Life::Insert => theme::note(ui, "Created by this project."),
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
    let said = capital(&describe_entry(subject, condition.entry));
    ui.label(egui::RichText::new(said).color(theme::INK));

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
                "Create a new empty condition and an AND condition over it and this one, then \
                 open the new condition. If the column the window was opened from holds this \
                 condition, the column is set to the AND.",
            )
            .clicked()
        {
            combine(ui.ctx(), subject, condition.entry, -1);
        }
        if ui.button("Combine with OR\u{2026}").on_hover_text("As Combine with AND, with an OR condition.").clicked() {
            combine(ui.ctx(), subject, condition.entry, -2);
        }
        let label = match shown.life {
            Life::Delete => "Keep",
            _ => "Remove",
        };
        if ui
            .button(label)
            .on_hover_text("Rows in other tables that refer to this condition keep its entry; check them first.")
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
                "condition {parent} combines {entry} with the new condition {child}; replace references to {entry} with {parent}"
            ),
        };
    }
    subject.conditions.show(Some(child));
}

/// One value, drawn as what the type says it is.
fn value_cell(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &Shown, n: usize, column: &'static str, means: Means) {
    let now = subject.now;
    let value = shown.condition.values[n];
    let salt = egui::Id::new(("condition-value", shown.condition.entry, n));
    let written = value_input(ui, means, value, salt).map(|value| value.to_string());
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

// ---------------------------------------------------------------------------
// The Tests view: the condition as a tree of tests
// ---------------------------------------------------------------------------

/// A change to the draft, collected while the tree is drawn and made after
/// it, since the tree is drawn from a copy.
enum Change {
    /// Put this node at the path.
    Replace(Vec<usize>, Node),
    /// Add this node to the group at the path.
    Add(Vec<usize>, Node),
    Remove(Vec<usize>),
    /// Unfold or fold the test at the path.
    Fold(Vec<usize>),
}

/// The condition as a tree: the Save bar, then the root group with every
/// test and group under it. `from` is the entry shown, or 0 for a new one.
fn tree_page(ui: &mut egui::Ui, subject: &mut Subject<'_>, from: u32) {
    subject.conditions.draft_for(&subject.session.server_edits, from);
    // A value chosen through the picker on the last frame; see
    // `PickFor::ConditionValue`.
    if let Some((path, slot, id)) = subject.quests.condition_pick.take() {
        if let Some(Node::Test { values, .. }) = subject.conditions.draft.as_mut().and_then(|draft| draft.tree.at_mut(&path)) {
            values[slot] = id as i32;
        }
    }
    let Some(draft) = subject.conditions.draft.clone() else {
        return;
    };
    save_bar(ui, subject, &draft);
    ui.add_space(4.0);
    let mut changes = Vec::new();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        group_card(ui, subject, &draft, &[], &draft.tree, &mut changes);
    });
    let Some(draft) = subject.conditions.draft.as_mut() else {
        return;
    };
    for change in changes {
        match change {
            Change::Replace(path, node) => {
                if let Some(at) = draft.tree.at_mut(&path) {
                    *at = node;
                }
            }
            Change::Add(path, node) => {
                if let Some(Node::Group { members, .. }) = draft.tree.at_mut(&path) {
                    let unfold = matches!(node, Node::Test { .. });
                    members.push(node);
                    if unfold {
                        let mut added = path.clone();
                        added.push(members.len() - 1);
                        draft.open.insert(added);
                    }
                }
            }
            Change::Remove(path) => {
                draft.tree.remove(&path);
                draft.open.clear();
            }
            Change::Fold(path) => {
                if !draft.open.remove(&path) {
                    draft.open.insert(path);
                }
            }
        }
    }
}

/// Save and Revert, and a line saying what Save would write.
fn save_bar(ui: &mut egui::Ui, subject: &mut Subject<'_>, draft: &Draft) {
    let changed = draft.changed();
    let plan = changed.then(|| subject.conditions.plan(&subject.session.server_edits));
    let mut save = false;
    let mut revert = false;
    ui.horizontal(|ui| {
        save = ui
            .add_enabled(matches!(plan, Some(Ok(_))), egui::Button::new("Save"))
            .on_hover_text(
                "Write the tests as conditions rows. A part that an existing row already tests \
                 reuses that row; the rest become new rows. Existing rows are not changed. The \
                 column the window was opened from is set to the resulting entry.",
            )
            .clicked();
        revert = ui
            .add_enabled(changed, egui::Button::new("Revert"))
            .on_hover_text("Discard the draft and restore the tests as they were read.")
            .clicked();
        if !changed && draft.from != 0 && offers_use(subject, draft.from) {
            if ui
                .button("Use")
                .on_hover_text("Write this condition into the column the window was opened from.")
                .clicked()
            {
                use_for_column(ui.ctx(), subject, draft.from);
            }
        }
        match &plan {
            None if draft.from == 0 => meaning(ui, "Add a test, then Save."),
            None => meaning(ui, "No changes."),
            Some(Ok(built)) if built.created.is_empty() => {
                meaning(ui, format!("Save uses condition {}, which already tests this.", built.root))
            }
            Some(Ok(built)) => meaning(
                ui,
                format!("Save writes {} new row(s); the result is condition {}.", built.created.len(), built.root),
            ),
            Some(Err(why)) => {
                ui.label(egui::RichText::new(why).small().color(theme::WARN));
            }
        }
    });
    if changed && draft.from != 0 && subject.conditions.asker.is_none() {
        theme::note(
            ui,
            format!(
                "No column opened this window, so Save only creates and opens the new condition; \
                 nothing refers to it yet. Condition {} and everything that refers to it are not \
                 changed. To edit one row in place, use the Rows view.",
                draft.from
            ),
        );
    }
    if revert {
        if let Some(draft) = subject.conditions.draft.as_mut() {
            draft.tree = draft.read.clone();
            draft.open.clear();
        }
    }
    if save {
        let from = draft.from;
        match subject.conditions.save(subject.session, subject.now) {
            Ok(root) => {
                match subject.conditions.asker.is_some() {
                    true => use_for_column(ui.ctx(), subject, root),
                    false => {
                        subject.session.status = match (from, root == from) {
                            (0, _) => format!("condition {root} created"),
                            (_, true) => format!("condition {root} already tests this"),
                            (_, false) => format!("saved as condition {root}; replace references to condition {from} with {root}"),
                        }
                    }
                }
                subject.conditions.show(Some(root));
            }
            Err(why) => subject.session.status = why,
        }
    }
}

/// The words a group is shown as, by whether it is an OR and whether its
/// result is reversed.
const GROUP_KINDS: [(&str, bool, bool); 4] = [
    ("all of these", false, false),
    ("any of these", true, false),
    ("none of these", true, true),
    ("not all of these", false, true),
];

/// A group: what of its members must hold, its members, and the buttons that
/// add to it.
fn group_card(ui: &mut egui::Ui, subject: &mut Subject<'_>, draft: &Draft, path: &[usize], node: &Node, changes: &mut Vec<Change>) {
    let Node::Group { any, flags, members } = node else {
        return;
    };
    let reversed = flags & rows::REVERSE != 0;
    super::behaviour::card(theme::LINE).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (_, removed) = super::behaviour::header(
            ui,
            |ui| {
                if path.is_empty() {
                    super::behaviour::badge(ui, "WHEN", theme::ACCENT);
                }
                let shown = GROUP_KINDS
                    .iter()
                    .find(|(_, a, r)| *a == *any && *r == reversed)
                    .map_or("all of these", |(words, _, _)| *words);
                egui::ComboBox::from_id_salt(("condition-group", path))
                    .selected_text(egui::RichText::new(shown).strong())
                    .show_ui(ui, |ui| {
                        for (words, a, r) in GROUP_KINDS {
                            if ui.selectable_label(a == *any && r == reversed, words).clicked() {
                                let flags = (flags & !rows::REVERSE) | if r { rows::REVERSE } else { 0 };
                                changes.push(Change::Replace(
                                    path.to_vec(),
                                    Node::Group { any: a, flags, members: members.clone() },
                                ));
                            }
                        }
                    })
                    .response
                    .on_hover_text(
                        "All of these is an AND row, any of these an OR row; none of these and \
                         not all of these are the same rows with the result reversed.",
                    );
                ui.label(egui::RichText::new(match any {
                    true => "holds",
                    false => "hold",
                }).color(theme::INK_DIM));
            },
            |ui| !path.is_empty() && ui.small_button("\u{d7}").on_hover_text("Remove this group and its tests.").clicked(),
        );
        if removed {
            changes.push(Change::Remove(path.to_vec()));
        }
        if flags & 0x2 != 0 {
            meaning(ui, "target and source swapped for every test in this group");
        }
        if members.is_empty() {
            meaning(ui, "No tests yet. Add one below.");
        }
        for (at, member) in members.iter().enumerate() {
            let mut inner = path.to_vec();
            inner.push(at);
            match member {
                Node::Group { .. } => group_card(ui, subject, draft, &inner, member, changes),
                Node::Test { .. } => test_card(ui, subject, draft, &inner, member, changes),
                Node::Missing(entry) => missing_card(ui, *entry, &inner, changes),
            }
        }
        ui.horizontal(|ui| {
            type_menu(ui, "+ test", |kind| {
                changes.push(Change::Add(path.to_vec(), Node::Test { kind, values: [0; 4], flags: 0 }));
            });
            ui.menu_button("+ group", |ui| {
                for (words, a, r) in GROUP_KINDS {
                    if ui.button(words).clicked() {
                        let flags = if r { rows::REVERSE } else { 0 };
                        changes.push(Change::Add(path.to_vec(), Node::Group { any: a, flags, members: Vec::new() }));
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Add a nested group, for a condition such as \"A, and either B or C\".");
        });
    });
}

/// One test: what it tests in words, and when unfolded its type, values and
/// flags.
fn test_card(ui: &mut egui::Ui, subject: &mut Subject<'_>, draft: &Draft, path: &[usize], node: &Node, changes: &mut Vec<Change>) {
    let Node::Test { kind, values, flags } = node else {
        return;
    };
    let (kind, values, flags) = (*kind, *values, *flags);
    let open = draft.open.contains(path);
    let known = rows::type_of(kind);
    let said = capital(&test_words(subject, kind, values));
    super::behaviour::card(if known.is_some() { theme::LINE } else { theme::WARN }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (_, (removed, fold)) = super::behaviour::header(
            ui,
            |ui| {
                if flags & rows::REVERSE != 0 {
                    super::behaviour::badge(ui, "NOT", theme::BAD);
                }
                ui.label(egui::RichText::new(said).color(theme::INK));
            },
            |ui| {
                let removed = ui.small_button("\u{d7}").on_hover_text("Remove this test.").clicked();
                let fold = super::behaviour::fold_button(ui, open);
                (removed, fold)
            },
        );
        if removed {
            changes.push(Change::Remove(path.to_vec()));
        }
        if fold {
            changes.push(Change::Fold(path.to_vec()));
        }
        if !open {
            return;
        }
        ui.separator();
        let replace = |kind: i32, values: [i32; 4], flags: u8| Change::Replace(path.to_vec(), Node::Test { kind, values, flags });
        let current = known.map_or_else(|| format!("type {kind}"), |known| known.name.to_string());
        row(ui, "test", |ui| {
            type_menu(ui, &current, |chosen| changes.push(replace(chosen, [0; 4], flags)));
        });
        match known {
            Some(known) => meaning(ui, format!("{} Needs {}.", known.about, known.needs)),
            None => meaning(ui, "Not a type vmangos defines; the server skips it."),
        }
        for n in 0..4 {
            let slot = known.and_then(|known| known.values[n]);
            match slot {
                Some(slot) => {
                    let written = row(ui, slot.label, |ui| draft_value(ui, subject, path, n, slot, values[n]));
                    if let Some(value) = written {
                        let mut values = values;
                        values[n] = value;
                        changes.push(replace(kind, values, flags));
                    }
                }
                None if values[n] != 0 => row(ui, ["value1", "value2", "value3", "value4"][n], |ui| {
                    ui.label(theme::number(values[n].to_string()));
                    meaning(ui, "not read by this type");
                }),
                None => {}
            }
        }
        let mut reverse = flags & rows::REVERSE != 0;
        let mut swap = flags & 0x2 != 0;
        row(ui, "flags", |ui| {
            let a = ui.checkbox(&mut reverse, "not").on_hover_text("Reverse the result: the test holds when this is false.").changed();
            let b = ui
                .checkbox(&mut swap, "swap target and source")
                .on_hover_text("Swap the target and the source for this test.")
                .changed();
            if a || b {
                changes.push(replace(kind, values, u8::from(reverse) | (u8::from(swap) << 1)));
            }
        });
    });
}

/// An entry the tree names that no row holds.
fn missing_card(ui: &mut egui::Ui, entry: u32, path: &[usize], changes: &mut Vec<Change>) {
    super::behaviour::card(theme::BAD).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (_, removed) = super::behaviour::header(
            ui,
            |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "Condition {entry} does not exist, so the server skips everything that names this condition."
                    ))
                    .color(theme::BAD),
                );
            },
            |ui| ui.small_button("\u{d7}").on_hover_text("Remove this entry from the group.").clicked(),
        );
        if removed {
            changes.push(Change::Remove(path.to_vec()));
        }
    });
}

/// A menu of every test type, by what it is about. `chosen` is called with
/// the type picked.
fn type_menu(ui: &mut egui::Ui, label: &str, mut chosen: impl FnMut(i32)) {
    ui.menu_button(label, |ui| {
        ui.set_min_width(220.0);
        for category in rows::Category::ALL {
            ui.menu_button(category.name(), |ui| {
                ui.set_min_width(240.0);
                for known in rows::TYPES.iter().filter(|known| rows::category(known.id) == Some(category)) {
                    if ui.button(known.name).on_hover_text(known.about).clicked() {
                        chosen(known.id);
                        ui.close();
                    }
                }
            });
        }
    });
}

/// One value of a test in the draft, drawn as what its type says it is.
/// Answers a new value.
fn draft_value(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &[usize], n: usize, slot: rows::Slot, value: i32) -> Option<i32> {
    let written = value_input(ui, slot.means, value, egui::Id::new(("condition-draft", path, n)));
    match slot.means {
        Means::Condition if value > 0 => {
            let said = describe_entry(subject, value as u32);
            ui.add(egui::Label::new(egui::RichText::new(capital(&said)).small().color(theme::INK_DIM)).truncate());
        }
        means => {
            if let Some(table) = table_of(means) {
                if let Some(kind) = super::quests::target_of(table) {
                    if ui.small_button("\u{2026}").on_hover_text("choose by name").clicked() {
                        let purpose = PickFor::ConditionValue { path: path.to_vec(), slot: n, label: slot.label };
                        subject.quests.picker = Some(Picker::new(kind, purpose));
                    }
                }
                if value > 0 {
                    let mut resolver = Resolver { session: subject.session, assets: subject.assets, quests: subject.quests };
                    reference::name(ui, &mut resolver, table, value as u32);
                }
            }
        }
    }
    written
}

/// The control for one value by what it means: a choice, a mask, or a
/// number. Answers a new value.
fn value_input(ui: &mut egui::Ui, means: Means, value: i32, salt: egui::Id) -> Option<i32> {
    let showing = value.to_string();
    let choices: Option<&[Value]> = match means {
        Means::Compare => Some(&COMPARES),
        Means::Team => Some(&TEAMS),
        Means::Rank => Some(&RANKS),
        Means::QuestState => Some(&QUEST_STATES),
        Means::Gender => Some(&GENDERS),
        Means::Bool => Some(&NO_YES),
        Means::Which => Some(&WHICH),
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
    written.and_then(|written| written.trim().parse::<i64>().ok()).map(|value| value as i32)
}

// ---------------------------------------------------------------------------
// A condition in words
// ---------------------------------------------------------------------------

/// The condition `entry` in words, every test and group it is made of.
fn describe_entry(subject: &mut Subject<'_>, entry: u32) -> String {
    let tree = {
        let conditions = &*subject.conditions;
        let edits = &subject.session.server_edits;
        rows::tree(entry, |entry| conditions.row(edits, entry))
    };
    describe(subject, &tree)
}

/// A tree in words: a test as its sentence, a group as what of its members
/// must hold with the members in brackets.
fn describe(subject: &mut Subject<'_>, node: &Node) -> String {
    match node {
        Node::Test { kind, values, flags } => {
            let mut said = test_words(subject, *kind, *values);
            if flags & rows::REVERSE != 0 {
                said = format!("NOT {said}");
            }
            if flags & 0x2 != 0 {
                said.push_str(" (target and source swapped)");
            }
            said
        }
        Node::Missing(entry) => format!("condition {entry}, which does not exist"),
        Node::Group { flags, members, .. } if members.len() == 1 && *flags == 0 => describe(subject, &members[0]),
        Node::Group { any, flags, members } => {
            let reversed = flags & rows::REVERSE != 0;
            let words = GROUP_KINDS
                .iter()
                .find(|(_, a, r)| a == any && *r == reversed)
                .map_or("all of these", |(words, _, _)| *words);
            let parts: Vec<String> = members.iter().map(|member| describe(subject, member)).collect();
            format!("{} ({})", words.trim_end_matches(" these"), parts.join("; "))
        }
    }
}

/// One test in words: its type's phrase with every value named.
fn test_words(subject: &mut Subject<'_>, kind: i32, values: [i32; 4]) -> String {
    let Some(known) = rows::type_of(kind) else {
        return format!("type {kind}, which vmangos does not define");
    };
    let Some(phrase) = rows::phrase(kind) else {
        return known.name.to_string();
    };
    let words: Vec<String> = (0..4)
        .map(|n| match known.values[n] {
            Some(slot) => value_words(subject, slot.means, values[n]),
            None => values[n].to_string(),
        })
        .collect();
    rows::render(phrase, values, |n| words[n].clone())
}

/// One value in words, by what it means.
fn value_words(subject: &mut Subject<'_>, means: Means, value: i32) -> String {
    let named = |list: &[Value]| list.iter().find(|known| i64::from(known.value) == i64::from(value)).map(|known| known.name.to_string());
    let mask = |bits: &[vale_mangos::schema::Bit]| {
        let names: Vec<&str> = bits.iter().filter(|bit| value as u32 & bit.bit != 0).map(|bit| bit.name).collect();
        match names.is_empty() {
            true => "any".to_string(),
            false => names.join(" or "),
        }
    };
    let said = match means {
        Means::Number => Some(value.to_string()),
        Means::Compare => named(&COMPARES),
        Means::Team => named(&TEAMS),
        Means::Rank => named(&RANKS),
        Means::QuestState => named(&QUEST_STATES),
        Means::Gender => named(&GENDERS),
        Means::Bool => named(&NO_YES),
        Means::Which => named(&WHICH),
        Means::RaceMask => Some(mask(&vale_mangos::item::RACE_MASK)),
        Means::ClassMask => Some(mask(&vale_mangos::item::CLASS_MASK)),
        Means::Condition => Some(format!("condition {value}")),
        means => name_of(subject, means, value),
    };
    said.unwrap_or_else(|| match noun(means) {
        "" => value.to_string(),
        noun => format!("{noun} {value}"),
    })
}

/// What a referenced value is called before its name is known.
fn noun(means: Means) -> &'static str {
    match means {
        Means::Spell => "spell",
        Means::Item => "item",
        Means::Quest => "quest",
        Means::Creature => "creature",
        Means::GameObject => "game object",
        Means::Area => "area",
        Means::Faction => "faction",
        Means::Skill => "skill",
        Means::Map => "map",
        _ => "",
    }
}

/// `text` with its first letter in capitals.
fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
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

const WHICH: [Value; 3] = [Value { value: 0, name: "any" }, Value { value: 1, name: "a hostile" }, Value { value: 2, name: "a friendly" }];
