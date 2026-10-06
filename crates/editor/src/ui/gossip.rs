//! The gossip window: one gossip menu at a time, what the creature says and
//! the options it offers, each option leading to the next menu.
//!
//! The window follows the selected creature, showing the menu its template's
//! `gossip_menu_id` names. New menu makes a menu of one text and opens it.
//! For the selected creature, New menu for it makes one and names it in the
//! template, and Use names the menu shown. An option that leads to a menu
//! opens it in place, with Back to return; a menu can also be opened by its
//! number. The creature form's `gossip_menu_id` row has two buttons, open
//! and + new, drawn by [`menu_buttons`].
//!
//! What a text says is a `broadcast_text` line, read and edited through the
//! behaviour tool, which holds that table for scripts. A condition opens in
//! the condition window, and a script in the script window. See
//! `crate::tools::gossip` for what each edit writes.
//!
//! ## The controls that set each id
//!
//! ```text
//! text_id              Add text makes one; Add existing names one by number
//! a line's text        edited in place; choose… an existing broadcast_text
//! script_id,           edit…, choose…, + new and clear, the script window's
//! action_script_id     own buttons (`super::behaviour::script_controls`)
//! action_menu_id       Open, choose… through the reference picker, New menu
//! action_poi_id        the point's name and place edited in place, choose…
//!                      by name, + new at the selected creature
//! option and box text  typed, or a broadcast_text: choose…, + new from the
//!                      typed text, typed to go back
//! condition_id         the condition cell (`super::conditions::cell`)
//! ```

use bevy_egui::egui;

use super::rowform::{choice_cell, flags_cell, meaning, page_row, page_spacing};
use super::theme;
use crate::session::EditSession;
use crate::tools::behaviour::{Behaviour, Chooser, ScriptAnswer, Search};
use crate::tools::gossip::{Gossip, Shown};
use crate::tools::quests::{ColumnTarget, PickFor, Picker, Quests};
use vale_mangos::gossip::{self, MenuOption, MenuText, NpcText, Point};
use vale_mangos::row::{Key, Life};

/// What the window is drawn from.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub gossip: &'a mut Gossip,
    pub behaviour: &'a mut Behaviour,
    /// The reference picker, which chooses the menu an option leads to.
    pub quests: &'a mut Quests,
    pub now: f64,
}

/// Where a chooser or the picker writes one gossip column.
fn target(table: &'static str, key: Key, column: &'static str, in_database: Option<String>, label: &'static str) -> ColumnTarget {
    let subject = format!("{table} {} {column}", key.text());
    ColumnTarget { table, key, column, in_database, label, subject }
}

/// What a new text says until it is typed over.
const NEW_TEXT: &str = "Greetings, $N.";

fn row<R>(ui: &mut egui::Ui, name: &str, contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    page_row(ui, name, "", false, contents)
}

fn frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::SHELL)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(8))
}

fn card(life: Life) -> egui::Frame {
    let stroke = match life {
        Life::Insert => theme::WARN,
        Life::Delete => egui::Color32::from_rgb(0xC8, 0x50, 0x50),
        Life::Update => theme::LINE,
    };
    egui::Frame::default()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 6))
}

pub fn window(ctx: &egui::Context, mut subject: Subject<'_>) -> Option<egui::Rect> {
    if !subject.gossip.open {
        return None;
    }
    let mut keep_open = true;
    let title = match subject.gossip.showing() {
        Some(menu) => format!("Gossip \u{2014} menu {menu}"),
        None => "Gossip".to_string(),
    };
    let shown = egui::Window::new(title)
        .id(egui::Id::new("gossip-window"))
        .open(&mut keep_open)
        .default_size([620.0, 640.0])
        .default_pos([360.0, 120.0])
        .resizable(true)
        .frame(frame())
        .show(ctx, |ui| {
            page_spacing(ui);
            contents(ui, &mut subject);
        });
    if !keep_open {
        subject.gossip.open = false;
    }
    point_chooser(ctx, &mut subject);
    shown.map(|response| response.response.rect)
}

fn contents(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    let now = subject.now;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!subject.gossip.trail.is_empty() || subject.gossip.menu.is_some(), egui::Button::new("Back"))
            .on_hover_text("The menu shown before, or the creature's own.")
            .clicked()
        {
            subject.gossip.back();
        }
        ui.add(egui::DragValue::new(&mut subject.gossip.typed).speed(1.0));
        if ui.button("Open").on_hover_text("Open the gossip menu with this entry.").clicked() {
            let typed = subject.gossip.typed;
            subject.gossip.show(typed);
        }
        let ready = can_make(subject.session, subject.gossip, subject.behaviour);
        if ui
            .add_enabled(ready, egui::Button::new("New menu"))
            .on_hover_text("A new menu with one text, opened here. Nothing names it until a creature, an object or an option does.")
            .on_disabled_hover_text("Reading the highest menu and text ids\u{2026}")
            .clicked()
        {
            if let Some(menu) = new_menu(subject.session, subject.gossip, subject.behaviour, now) {
                subject.gossip.show(menu);
                subject.session.status = format!("gossip menu {menu} made");
            }
        }
    });
    if let Some(about) = subject.gossip.about.clone() {
        let ready = can_make(subject.session, subject.gossip, subject.behaviour);
        match about.gossip_menu_id {
            0 => {
                ui.label(egui::RichText::new(format!("{} has no gossip menu.", about.label)).color(theme::INK));
            }
            menu => {
                theme::note(ui, format!("{}'s menu is {menu}.", about.label));
                if about.npc_flags & 0x1 == 0 {
                    ui.label(
                        egui::RichText::new(
                            "Its npc_flags lack the gossip flag (1), so the server does not offer \
                             the menu.",
                        )
                        .size(theme::SMALL)
                        .color(theme::WARN),
                    );
                }
            }
        }
        ui.horizontal_wrapped(|ui| {
            let label = match about.gossip_menu_id {
                0 => "Make a menu for it",
                _ => "New menu for it",
            };
            if ui
                .add_enabled(ready, egui::Button::new(label))
                .on_hover_text(
                    "A new menu with one text, named in the template's gossip_menu_id, and \
                     the gossip flag added to its npc_flags so the server offers it. A menu \
                     it named before is left as it is, for whatever else names it.",
                )
                .on_disabled_hover_text("Reading the highest menu and text ids\u{2026}")
                .clicked()
            {
                make_menu_for(subject, &about, now);
            }
            if let Some(shown) = subject.gossip.showing().filter(|shown| *shown != about.gossip_menu_id) {
                if ui
                    .button(format!("Use menu {shown} for it"))
                    .on_hover_text("Name the menu shown here in the template's gossip_menu_id, and add the gossip flag.")
                    .clicked()
                {
                    name_menu(subject.session, &about.template_key, about.npc_flags, shown, now);
                    subject.session.status = format!("{} now offers gossip menu {shown}", about.label);
                }
            }
        });
    }
    ui.separator();
    if let Some(trouble) = &subject.gossip.trouble {
        theme::note(ui, format!("The gossip tables could not be read: {trouble}"));
        return;
    }
    let Some(menu) = subject.gossip.showing() else {
        theme::note(ui, "Select a creature with a gossip menu, or open a menu by its number.");
        return;
    };
    if !subject.gossip.is_read(menu) {
        theme::note(ui, format!("reading gossip menu {menu}\u{2026}"));
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        theme::heading(ui, "What it says");
        let texts = subject.gossip.texts_of(&subject.session.server_edits, menu);
        if texts.is_empty() {
            theme::note(ui, "No text: the server shows the default greeting.");
        }
        if texts.iter().filter(|text| text.life != Life::Delete).count() > 1 {
            meaning(ui, "The server shows the text whose condition the player meets.");
        }
        for text in &texts {
            card(text.life).show(ui, |ui| text_card(ui, subject, text));
            ui.add_space(4.0);
        }
        let ready = subject.behaviour.next_text_entry(subject.session).is_some()
            && subject.gossip.next_npc_text(&subject.session.server_edits).is_some();
        if ui
            .add_enabled(ready, egui::Button::new("Add text"))
            .on_hover_text("A new npc_text of one new line, added to this menu.")
            .on_disabled_hover_text("Reading the highest text ids\u{2026}")
            .clicked()
        {
            if let Some(line) = subject.behaviour.create_text(subject.session, NEW_TEXT, 0, now) {
                subject.gossip.add_text(subject.session, menu, line, now);
            }
        }
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut subject.gossip.existing_text).speed(1.0));
            let taken = texts.iter().any(|text| text.row.text_id == subject.gossip.existing_text);
            if ui
                .add_enabled(subject.gossip.existing_text != 0 && !taken, egui::Button::new("Add existing text"))
                .on_hover_text("Put the npc_text row with this id into the menu; other menus may show it too.")
                .on_disabled_hover_text("Type the id of an npc_text row this menu does not show yet.")
                .clicked()
            {
                let id = subject.gossip.existing_text;
                subject.gossip.add_existing_text(subject.session, menu, id, now);
            }
        });
        ui.add_space(8.0);
        theme::heading(ui, "Options");
        let options = subject.gossip.options_of(&subject.session.server_edits, menu);
        if options.is_empty() {
            theme::note(ui, "No option.");
        }
        for option in &options {
            card(option.life).show(ui, |ui| option_card(ui, subject, menu, option));
            ui.add_space(4.0);
        }
        if ui.button("Add option").on_hover_text("A gossip line at the end of the menu, leading nowhere yet.").clicked() {
            subject.gossip.add_option(subject.session, menu, "Tell me more.", now);
        }
    });
}

/// Make a menu for the selected creature and name it in the template.
fn make_menu_for(subject: &mut Subject<'_>, about: &crate::tools::gossip::About, now: f64) {
    let Some(menu) = new_menu(subject.session, subject.gossip, subject.behaviour, now) else {
        return;
    };
    name_menu(subject.session, &about.template_key, about.npc_flags, menu, now);
    subject.gossip.show(menu);
    subject.session.status = format!("gossip menu {menu} made for {}", about.label);
}

/// Whether the next menu and text ids are known, which making a menu needs.
fn can_make(session: &EditSession, gossip: &Gossip, behaviour: &Behaviour) -> bool {
    gossip.next_menu(&session.server_edits).is_some() && behaviour.next_text_entry(session).is_some()
}

/// Make a menu of one new text. Answers its entry.
fn new_menu(session: &mut EditSession, gossip: &mut Gossip, behaviour: &mut Behaviour, now: f64) -> Option<u32> {
    let line = behaviour.create_text(session, NEW_TEXT, 0, now)?;
    gossip.new_menu(session, line, now)
}

/// Name `menu` in a creature template's `gossip_menu_id`, and add the gossip
/// npc flag (1) when `npc_flags` lacks it, as one undo entry.
fn name_menu(session: &mut EditSession, template_key: &vale_mangos::row::Key, npc_flags: u32, menu: u32, now: f64) {
    let table = vale_mangos::creature::TEMPLATE;
    let label = format!("{table} {} gossip", template_key.text());
    let gesture = || crate::session::Gesture { label: "Name gossip menu", subject: &label, now };
    session.set_server_edit(table, template_key, "gossip_menu_id", Some(menu.to_string()), Some(gesture()));
    if npc_flags & 0x1 == 0 {
        let flags = npc_flags | 0x1;
        session.set_server_edit(table, template_key, "npc_flags", Some(flags.to_string()), Some(gesture()));
    }
}

/// The buttons the creature form draws after its `gossip_menu_id`: open the
/// menu in the gossip window, and make a new one and name it here.
/// `menu` is the column's value and `npc_flags` the template's.
#[allow(clippy::too_many_arguments)]
pub fn menu_buttons(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    gossip: &mut Gossip,
    behaviour: &mut Behaviour,
    template_key: &vale_mangos::row::Key,
    menu: u32,
    npc_flags: u32,
    now: f64,
) {
    if menu != 0 && ui.small_button("open").on_hover_text("Open this menu in the gossip window.").clicked() {
        gossip.open = true;
        gossip.show(menu);
    }
    let ready = can_make(session, gossip, behaviour);
    if ui
        .add_enabled(ready, egui::Button::new("+ new").small())
        .on_hover_text(
            "A new menu with one text, named here, with the gossip flag added to npc_flags, and \
             opened in the gossip window. A menu named here before is left as it is.",
        )
        .on_disabled_hover_text("Open the gossip window once, so the highest menu and text ids are read.")
        .clicked()
    {
        if let Some(made) = new_menu(session, gossip, behaviour, now) {
            name_menu(session, template_key, npc_flags, made, now);
            gossip.open = true;
            gossip.show(made);
            session.status = format!("gossip menu {made} made");
        }
    }
}

/// One text of the menu: its lines, its condition and its script.
fn text_card(ui: &mut egui::Ui, subject: &mut Subject<'_>, text: &Shown<MenuText>) {
    let now = subject.now;
    let id = text.row.text_id;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("text {id}")).strong().color(theme::INK));
        let label = match text.life {
            Life::Delete => "Keep",
            _ => "Remove",
        };
        if ui.small_button(label).clicked() {
            match text.life {
                Life::Delete => keep(subject.session, gossip::MENU, &text.row.key(), now),
                _ => subject.gossip.remove_text(subject.session, text, now),
            }
        }
    });
    match subject.gossip.npc_text_of(&subject.session.server_edits, id) {
        Some(npc_text) => lines(ui, subject, &npc_text),
        None => theme::note(ui, format!("npc_text {id} is not in the table, so the server skips this text.")),
    }
    let mut condition = text.row.condition_id;
    let column = format!("{} {} condition_id", gossip::MENU, text.row.key().text());
    let written = row(ui, "condition", |ui| {
        let typed = ui.add(egui::DragValue::new(&mut condition).speed(1.0)).changed();
        let answered = super::conditions::cell(ui, egui::Id::new(("gossip-text-condition", &column)), &column, text.row.condition_id, None);
        answered.or(typed.then_some(condition))
    });
    if let Some(condition) = written.filter(|condition| *condition != text.row.condition_id) {
        subject.gossip.set_text(subject.session, text, "condition_id", condition.to_string(), now);
    }
    let mut script = text.row.script_id;
    let answer = ScriptAnswer::Column(target(
        gossip::MENU,
        text.row.key(),
        "script_id",
        text.in_database.as_ref().map(|held| held.script_id.to_string()),
        "Edit gossip text",
    ));
    let written = row(ui, "script", |ui| {
        let typed = ui
            .add(egui::DragValue::new(&mut script).speed(1.0))
            .on_hover_text("A gossip_scripts id run when the menu opens with this text, or 0.")
            .changed();
        let table = vale_mangos::scripts::GOSSIP;
        let made = super::behaviour::script_controls(ui, subject.session, subject.behaviour, table, text.row.script_id, None, answer, now);
        made.or_else(|| typed.then(|| script.to_string()))
    });
    if let Some(written) = written {
        subject.gossip.set_text(subject.session, text, "script_id", written, now);
    }
}

/// An npc_text's lines: what each says and its chance.
fn lines(ui: &mut egui::Ui, subject: &mut Subject<'_>, npc_text: &Shown<NpcText>) {
    let now = subject.now;
    for (n, (line, chance)) in npc_text.row.lines.iter().copied().enumerate() {
        if line == 0 {
            continue;
        }
        ui.horizontal(|ui| {
            let mut odds = chance;
            if ui
                .add(egui::DragValue::new(&mut odds).range(0.0..=1.0).speed(0.01))
                .on_hover_text("The chance this line is the one shown.")
                .changed()
            {
                let value = vale_mangos::sql::float(odds);
                subject.gossip.set_npc_text(subject.session, npc_text, gossip::CHANCE_COLUMNS[n], value, now);
            }
            // The line is given a width: a text box that asks for all of a
            // row grows the window by whatever follows it, every frame.
            let width = (ui.available_width() - 90.0).max(120.0);
            let salt = format!("gossip-line-{}-{n}", npc_text.row.id);
            ui.allocate_ui(egui::vec2(width, ui.spacing().interact_size.y), |ui| broadcast_line(ui, subject, line, &salt));
            let answer = ScriptAnswer::Column(target(
                gossip::NPC_TEXT,
                npc_text.row.key(),
                gossip::TEXT_COLUMNS[n],
                npc_text.in_database.as_ref().map(|held| held.lines[n].0.to_string()),
                "Edit gossip text",
            ));
            text_chooser_button(ui, subject.behaviour, answer);
        });
    }
    let free = npc_text.row.lines.iter().position(|(line, _)| *line == 0);
    if let Some(free) = free {
        let ready = subject.behaviour.next_text_entry(subject.session).is_some();
        if ui
            .add_enabled(ready, egui::Button::new("Add line").small())
            .on_hover_text("Another line for the server to choose between, by chance.")
            .clicked()
        {
            if let Some(line) = subject.behaviour.create_text(subject.session, NEW_TEXT, 0, now) {
                let subject_line = format!("{} {} line", gossip::NPC_TEXT, npc_text.row.id);
                subject.session.as_one("Add gossip line", &subject_line, |session| {
                    subject.gossip.set_npc_text(session, npc_text, gossip::TEXT_COLUMNS[free], line.to_string(), now);
                    subject.gossip.set_npc_text(session, npc_text, gossip::CHANCE_COLUMNS[free], "1".to_string(), now);
                });
            }
        }
    }
}

/// A broadcast text, editable in place once read.
fn broadcast_line(ui: &mut egui::Ui, subject: &mut Subject<'_>, entry: u32, salt: &str) {
    let now = subject.now;
    match subject.behaviour.text_of(entry, &subject.session.server_edits) {
        Some(shown) => {
            let mut said = shown.text.said().to_string();
            let edit = egui::TextEdit::singleline(&mut said).id_salt(salt).desired_width(f32::INFINITY);
            if ui.add(edit).on_hover_text(format!("broadcast_text {entry}")).changed() {
                subject.behaviour.set_text(subject.session, &shown, "male_text", said, now);
            }
        }
        None if subject.behaviour.text_read(entry) => meaning(ui, format!("broadcast_text {entry} is not in the table")),
        None => meaning(ui, format!("reading broadcast_text {entry}\u{2026}")),
    }
}

/// The button that opens the behaviour tool's `broadcast_text` chooser for
/// one column.
fn text_chooser_button(ui: &mut egui::Ui, behaviour: &mut Behaviour, answer: ScriptAnswer) {
    if ui
        .small_button("choose\u{2026}")
        .on_hover_text("Search broadcast_text by what is said or by entry, and use an existing text.")
        .clicked()
    {
        behaviour.chooser = Some(Chooser::Text { answer, search: Search::default() });
    }
}

/// An option's line or its box text: typed into `typed_column`, or a
/// `broadcast_text` row named in `text_column`. When `text_column` is 0, the
/// typed text is edited and the row offers choose… and + new; + new makes a
/// broadcast_text of the typed text. When `text_column` names a broadcast
/// text, that text is edited in place and the row offers choose… and typed;
/// typed writes 0 to `text_column`, which returns the option to its typed text.
#[allow(clippy::too_many_arguments)]
fn line_or_text(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    option: &Shown<MenuOption>,
    typed_column: &'static str,
    text_column: &'static str,
    typed: &str,
    text: u32,
    salt: &str,
    hover: &str,
) {
    let now = subject.now;
    let held = |held: &MenuOption| match text_column {
        "option_broadcast_text" => held.broadcast_text,
        _ => held.box_broadcast_text,
    };
    let answer = ScriptAnswer::Column(target(
        gossip::OPTION,
        option.row.key(),
        text_column,
        option.in_database.as_ref().map(|had| held(had).to_string()),
        "Edit gossip option",
    ));
    let mut written: Option<(&'static str, String)> = None;
    match text {
        0 => {
            let mut said = typed.to_string();
            let width = (ui.available_width() - 170.0).max(120.0);
            let edit = egui::TextEdit::singleline(&mut said).id_salt(salt).desired_width(width);
            if ui.add(edit).on_hover_text(hover).changed() {
                written = Some((typed_column, vale_mangos::sql::text(&said)));
            }
            text_chooser_button(ui, subject.behaviour, answer);
            let ready = subject.behaviour.next_text_entry(subject.session).is_some();
            if ui
                .add_enabled(ready, egui::Button::new("+ new").small())
                .on_hover_text("A new broadcast_text saying what is typed here, named in this column.")
                .on_disabled_hover_text("Reading broadcast_text's highest entry.")
                .clicked()
            {
                if let Some(made) = subject.behaviour.create_text(subject.session, typed, 0, now) {
                    written = Some((text_column, made.to_string()));
                }
            }
        }
        text => {
            let width = (ui.available_width() - 170.0).max(120.0);
            ui.allocate_ui(egui::vec2(width, ui.spacing().interact_size.y), |ui| broadcast_line(ui, subject, text, salt));
            text_chooser_button(ui, subject.behaviour, answer);
            if ui
                .small_button("typed")
                .on_hover_text("Write 0 here and use the typed text instead. The broadcast_text row is not touched.")
                .clicked()
            {
                written = Some((text_column, "0".to_string()));
            }
        }
    }
    if let Some((column, value)) = written {
        subject.gossip.set_option(subject.session, option, column, value, now);
    }
}

/// An option's point of interest: its number, then the point's name and
/// place, and the buttons that choose one or make one at the selected
/// creature.
fn point_row(ui: &mut egui::Ui, subject: &mut Subject<'_>, menu: u32, option: &Shown<MenuOption>) {
    let now = subject.now;
    let o = &option.row;
    let mut poi = o.action_poi;
    let written = row(ui, "point of interest", |ui| {
        let typed = ui
            .add(egui::DragValue::new(&mut poi).speed(1.0))
            .on_hover_text("A points_of_interest row marked on the player's map when it is clicked, or 0.")
            .changed();
        let mut out = typed.then_some(poi);
        if ui.small_button("choose\u{2026}").on_hover_text("Search the points of interest by name or entry.").clicked() {
            subject.gossip.point_chooser = Some((menu, o.id, String::new()));
        }
        let at = subject.gossip.about.as_ref().and_then(|about| about.at);
        let ready = subject.gossip.next_point(&subject.session.server_edits).is_some();
        if o.action_poi == 0
            && ui
                .add_enabled(ready, egui::Button::new("+ new").small())
                .on_hover_text(
                    "A new point of interest where the selected creature stands, named after the \
                     option's line, and named here.",
                )
                .on_disabled_hover_text("Reading points_of_interest.")
                .clicked()
        {
            let [x, y] = at.unwrap_or([0.0, 0.0]);
            let name = match o.text.is_empty() {
                true => "New point",
                false => o.text.as_str(),
            };
            out = subject.gossip.new_point(subject.session, x, y, name, now);
        }
        if o.action_poi != 0 && ui.small_button("clear").on_hover_text("Write 0: the option marks nothing.").clicked() {
            out = Some(0);
        }
        out
    });
    if let Some(poi) = written.filter(|poi| *poi != o.action_poi) {
        subject.gossip.set_option(subject.session, option, "action_poi_id", poi.to_string(), now);
    }
    if o.action_poi == 0 {
        return;
    }
    match subject.gossip.point(&subject.session.server_edits, o.action_poi) {
        Some(point) => point_form(ui, subject, &point),
        None if subject.gossip.points_read() => {
            ui.label(
                egui::RichText::new(format!(
                    "points_of_interest has no row {}; the server marks nothing.",
                    o.action_poi
                ))
                .size(theme::SMALL)
                .color(theme::WARN),
            );
        }
        None => meaning(ui, "reading points_of_interest\u{2026}"),
    }
}

/// One point of interest's columns, edited in place.
fn point_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, point: &Shown<Point>) {
    let now = subject.now;
    let p = point.row.clone();
    let mut writes: Vec<(&'static str, String)> = Vec::new();
    row(ui, "  name", |ui| {
        let mut name = p.name.clone();
        let edit = egui::TextEdit::singleline(&mut name)
            .id_salt(("gossip-point-name", p.entry))
            .desired_width(f32::INFINITY);
        if ui.add(edit).on_hover_text("icon_name: the name shown with the mark.").changed() {
            writes.push(("icon_name", vale_mangos::sql::text(&name)));
        }
    });
    row(ui, "  at", |ui| {
        let (mut x, mut y) = (p.x, p.y);
        if ui.add(egui::DragValue::new(&mut x).speed(1.0).prefix("x ")).changed() {
            writes.push(("x", vale_mangos::sql::float(x)));
        }
        if ui.add(egui::DragValue::new(&mut y).speed(1.0).prefix("y ")).changed() {
            writes.push(("y", vale_mangos::sql::float(y)));
        }
        if let Some([cx, cy]) = subject.gossip.about.as_ref().and_then(|about| about.at) {
            if ui
                .small_button("at the creature")
                .on_hover_text("Move the point to where the selected creature stands.")
                .clicked()
            {
                writes.push(("x", vale_mangos::sql::float(cx)));
                writes.push(("y", vale_mangos::sql::float(cy)));
            }
        }
    });
    row(ui, "  icon, flags, data", |ui| {
        for (column, value, about) in [
            ("icon", p.icon, "The icon drawn at the place; every shipped row uses 6."),
            ("flags", p.flags, "Sent to the client as they are; every shipped row uses 99."),
            ("data", p.data, "Sent to the client as it is; every shipped row uses 0."),
        ] {
            let mut value = value;
            if ui.add(egui::DragValue::new(&mut value).speed(1.0)).on_hover_text(about).changed() {
                writes.push((column, value.to_string()));
            }
        }
    });
    for fault in p.check() {
        ui.label(egui::RichText::new(fault).size(theme::SMALL).color(theme::WARN));
    }
    if writes.is_empty() {
        return;
    }
    let subject_line = format!("{} {}", gossip::POI, p.key().text());
    let (gossip, session) = (&mut *subject.gossip, &mut *subject.session);
    session.as_one("Edit point of interest", &subject_line, |session| {
        for (column, value) in writes {
            gossip.set_point(session, point, column, value, now);
        }
    });
}

/// The point of interest chooser: every point, searched by name or entry,
/// and the one clicked written into the option it was opened for.
fn point_chooser(ctx: &egui::Context, subject: &mut Subject<'_>) {
    let Some((menu, id, mut search)) = subject.gossip.point_chooser.clone() else {
        return;
    };
    let mut keep_open = true;
    let mut chosen: Option<u32> = None;
    egui::Window::new("Choose a point of interest")
        .id(egui::Id::new("gossip-point-chooser"))
        .open(&mut keep_open)
        .default_size([420.0, 420.0])
        .resizable(true)
        .frame(frame())
        .show(ctx, |ui| {
            ui.add(egui::TextEdit::singleline(&mut search).hint_text("name or entry").desired_width(f32::INFINITY));
            if !subject.gossip.points_read() {
                theme::note(ui, "reading points_of_interest\u{2026}");
                return;
            }
            let found = subject.gossip.points_matching(&subject.session.server_edits, &search);
            meaning(ui, format!("{} point(s)", found.len()));
            let height = ui.spacing().interact_size.y;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, height, found.len(), |ui, range| {
                for point in &found[range] {
                    ui.horizontal(|ui| {
                        if ui.small_button("use").clicked() {
                            chosen = Some(point.row.entry);
                        }
                        ui.label(theme::number(point.row.entry.to_string()));
                        ui.add(egui::Label::new(egui::RichText::new(&point.row.name).color(theme::INK)).truncate());
                        meaning(ui, format!("{:.0}, {:.0}", point.row.x, point.row.y));
                    });
                }
            });
        });
    subject.gossip.point_chooser = (keep_open && chosen.is_none()).then_some((menu, id, search));
    if let Some(entry) = chosen {
        let option = subject
            .gossip
            .options_of(&subject.session.server_edits, menu)
            .into_iter()
            .find(|shown| shown.row.id == id);
        if let Some(option) = option {
            let now = subject.now;
            subject.gossip.set_option(subject.session, &option, "action_poi_id", entry.to_string(), now);
        }
    }
}

/// One option: its line, what it does, where it leads, its box and its
/// condition.
fn option_card(ui: &mut egui::Ui, subject: &mut Subject<'_>, menu: u32, option: &Shown<MenuOption>) {
    let now = subject.now;
    let o = option.row.clone();
    let key = o.key();
    let set = |subject: &mut Subject<'_>, column: &'static str, value: String| {
        subject.gossip.set_option(subject.session, option, column, value, now);
    };
    ui.horizontal(|ui| {
        if ui.small_button("Up").on_hover_text("Move up").clicked() {
            subject.gossip.move_option(subject.session, menu, o.id, -1, now);
        }
        if ui.small_button("Down").on_hover_text("Move down").clicked() {
            subject.gossip.move_option(subject.session, menu, o.id, 1, now);
        }
        ui.label(theme::number(format!("{}", o.id)));
        if let Some(icon) = choice_cell(ui, &o.icon.to_string(), &gossip::ICONS, egui::Id::new(("gossip-icon", menu, o.id))) {
            set(subject, "option_icon", icon);
        }
        let label = match option.life {
            Life::Delete => "Keep",
            _ => "Remove",
        };
        if ui.small_button(label).clicked() {
            match option.life {
                Life::Delete => keep(subject.session, gossip::OPTION, &key, now),
                _ => subject.gossip.remove_option(subject.session, option, now),
            }
        }
    });
    row(ui, "line", |ui| {
        let salt = format!("gossip-option-line-{menu}-{}", o.id);
        let hover = "option_text: the line, when option_broadcast_text is 0.";
        line_or_text(ui, subject, option, "option_text", "option_broadcast_text", &o.text, o.broadcast_text, &salt, hover);
    });
    row(ui, "opens", |ui| {
        if let Some(kind) = choice_cell(ui, &o.option_type.to_string(), &gossip::OPTION_TYPES, egui::Id::new(("gossip-type", menu, o.id))) {
            let kind: u32 = kind.parse().unwrap_or(0);
            let subject_line = format!("{} {} type", gossip::OPTION, key.text());
            subject.session.as_one("Edit gossip option", &subject_line, |session| {
                subject.gossip.set_option(session, option, "option_id", kind.to_string(), now);
                subject.gossip.set_option(session, option, "npc_option_npcflag", gossip::flag_for(kind).to_string(), now);
            });
        }
    });
    row(ui, "needs flag", |ui| {
        if let Some(flags) = flags_cell(ui, &o.npc_flag.to_string(), &vale_mangos::creature::NPC_FLAGS, egui::Id::new(("gossip-flag", menu, o.id))) {
            set(subject, "npc_option_npcflag", flags);
        }
    });
    if let Some(about) = subject.gossip.about.as_ref().filter(|about| about.gossip_menu_id == menu) {
        if o.npc_flag != 0 && about.npc_flags & o.npc_flag == 0 {
            ui.label(
                egui::RichText::new(format!(
                    "{}'s npc_flags lack this flag, so it does not see this option.",
                    about.label
                ))
                .size(theme::SMALL)
                .color(theme::WARN),
            );
        }
    }
    row(ui, "leads to menu", |ui| {
        let mut next = o.action_menu as i64;
        if ui
            .add(egui::DragValue::new(&mut next).range(-1..=i64::from(gossip::MAX_MENU)))
            .on_hover_text("The menu it opens: 0 none, -1 closes the window.")
            .changed()
        {
            set(subject, "action_menu_id", next.to_string());
        }
        match o.action_menu {
            0 => {
                let ready = subject.gossip.next_menu(&subject.session.server_edits).is_some()
                    && subject.behaviour.next_text_entry(subject.session).is_some();
                if ui
                    .add_enabled(ready, egui::Button::new("New menu").small())
                    .on_hover_text("A new menu with one text, which this option opens.")
                    .clicked()
                {
                    if let Some(line) = subject.behaviour.create_text(subject.session, NEW_TEXT, 0, now) {
                        if let Some(made) = subject.gossip.new_menu(subject.session, line, now) {
                            set(subject, "action_menu_id", made.to_string());
                        }
                    }
                }
            }
            -1 => meaning(ui, "closes the window"),
            next if next > 0 => {
                if ui.small_button("Open").clicked() {
                    subject.gossip.show(next as u32);
                }
            }
            _ => {}
        }
        if let Some(kind) = super::quests::target_of(gossip::MENU) {
            if ui
                .small_button("choose\u{2026}")
                .on_hover_text("Search the gossip menus by what they say or who offers them.")
                .clicked()
            {
                let column = target(
                    gossip::OPTION,
                    key.clone(),
                    "action_menu_id",
                    option.in_database.as_ref().map(|held| held.action_menu.to_string()),
                    "Edit gossip option",
                );
                subject.quests.picker = Some(Picker::new(kind, PickFor::ServerColumn(column)));
            }
        }
    });
    let answer = ScriptAnswer::Column(target(
        gossip::OPTION,
        key.clone(),
        "action_script_id",
        option.in_database.as_ref().map(|held| held.action_script.to_string()),
        "Edit gossip option",
    ));
    let written = row(ui, "script", |ui| {
        let mut script = o.action_script;
        let typed = ui
            .add(egui::DragValue::new(&mut script).speed(1.0))
            .on_hover_text("A gossip_scripts id run when it is clicked, or 0.")
            .changed();
        let table = vale_mangos::scripts::GOSSIP;
        let made = super::behaviour::script_controls(ui, subject.session, subject.behaviour, table, o.action_script, None, answer, now);
        made.or_else(|| typed.then(|| script.to_string()))
    });
    if let Some(written) = written {
        set(subject, "action_script_id", written);
    }
    point_row(ui, subject, menu, option);
    row(ui, "box", |ui| {
        let mut coded = o.box_coded != 0;
        if ui.checkbox(&mut coded, "asks for a code").changed() {
            set(subject, "box_coded", u8::from(coded).to_string());
        }
        let mut money = o.box_money;
        ui.label(egui::RichText::new("costs").color(theme::INK_DIM));
        if ui
            .add(egui::DragValue::new(&mut money).speed(10.0).suffix(" c"))
            .on_hover_text("What the confirmation box says it costs, in copper.")
            .changed()
        {
            set(subject, "box_money", money.to_string());
        }
    });
    row(ui, "box text", |ui| {
        let salt = format!("gossip-box-line-{menu}-{}", o.id);
        let hover = "box_text: a confirmation box asked before the option acts. Empty for none.";
        line_or_text(ui, subject, option, "box_text", "box_broadcast_text", &o.box_text, o.box_broadcast_text, &salt, hover);
    });
    let mut condition = o.condition_id;
    let column = format!("{} {} condition_id", gossip::OPTION, key.text());
    let written = row(ui, "condition", |ui| {
        let typed = ui.add(egui::DragValue::new(&mut condition).speed(1.0)).changed();
        let answered = super::conditions::cell(ui, egui::Id::new(("gossip-option-condition", &column)), &column, o.condition_id, None);
        answered.or(typed.then_some(condition))
    });
    if let Some(condition) = written.filter(|condition| *condition != o.condition_id) {
        set(subject, "condition_id", condition.to_string());
    }
    for fault in o.check() {
        ui.label(egui::RichText::new(fault).size(theme::SMALL).color(theme::WARN));
    }
}

/// Cancel the removal of a row by dropping the project's edit of it.
fn keep(session: &mut EditSession, table: &str, key: &vale_mangos::row::Key, now: f64) {
    let label = format!("{table} {}", key.text());
    session.set_server_row(table, key, None, Some(crate::session::Gesture { label: "Keep gossip row", subject: &label, now }));
}
