//! The gossip window: one gossip menu at a time, what the creature says and
//! the options it offers, each option leading to the next menu.
//!
//! The window follows the selected creature, showing the menu its template's
//! `gossip_menu_id` names, and offers to make one when it names none. An
//! option that leads to a menu opens it in place, with Back to return; a menu
//! can also be opened by its number.
//!
//! What a text says is a `broadcast_text` line, read and edited through the
//! behaviour tool, which holds that table for scripts. A condition opens in
//! the condition window, and a script in the script window. See
//! `crate::tools::gossip` for what each edit writes.

use bevy_egui::egui;

use super::rowform::{choice_cell, flags_cell, meaning, page_row, page_spacing};
use super::theme;
use crate::session::EditSession;
use crate::tools::behaviour::Behaviour;
use crate::tools::gossip::{Gossip, Shown};
use vale_mangos::gossip::{self, MenuOption, MenuText, NpcText};
use vale_mangos::row::Life;

/// What the window is drawn from.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub gossip: &'a mut Gossip,
    pub behaviour: &'a mut Behaviour,
    pub now: f64,
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
    });
    if let Some(about) = subject.gossip.about.clone() {
        match about.gossip_menu_id {
            0 => {
                ui.label(egui::RichText::new(format!("{} has no gossip menu.", about.label)).color(theme::INK));
                let ready = subject.gossip.next_menu(&subject.session.server_edits).is_some()
                    && subject.behaviour.next_text_entry(subject.session).is_some();
                if ui
                    .add_enabled(ready, egui::Button::new("Make a menu for it"))
                    .on_hover_text(
                        "A new menu with one text, named in the template's gossip_menu_id, and \
                         the gossip flag added to its npc_flags so the server offers it.",
                    )
                    .on_disabled_hover_text("Reading the highest menu and text ids\u{2026}")
                    .clicked()
                {
                    make_menu_for(subject, &about, now);
                }
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
    let Some(line) = subject.behaviour.create_text(subject.session, NEW_TEXT, 0, now) else {
        return;
    };
    let Some(menu) = subject.gossip.new_menu(subject.session, line, now) else {
        return;
    };
    let table = vale_mangos::creature::TEMPLATE;
    let label = format!("{table} {} gossip", about.template_key.text());
    let gesture = || crate::session::Gesture { label: "Make gossip menu", subject: &label, now };
    subject.session.set_server_edit(table, &about.template_key, "gossip_menu_id", Some(menu.to_string()), Some(gesture()));
    if about.npc_flags & 0x1 == 0 {
        let flags = about.npc_flags | 0x1;
        subject.session.set_server_edit(table, &about.template_key, "npc_flags", Some(flags.to_string()), Some(gesture()));
    }
    subject.session.status = format!("gossip menu {menu} made for {}", about.label);
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
    row(ui, "condition", |ui| {
        if ui.add(egui::DragValue::new(&mut condition).speed(1.0)).changed() {
            subject.gossip.set_text(subject.session, text, "condition_id", condition.to_string(), now);
        }
        super::conditions::open_button(ui, condition);
    });
    let mut script = text.row.script_id;
    row(ui, "script", |ui| {
        if ui
            .add(egui::DragValue::new(&mut script).speed(1.0))
            .on_hover_text("A gossip_scripts id run when the menu opens with this text, or 0.")
            .changed()
        {
            subject.gossip.set_text(subject.session, text, "script_id", script.to_string(), now);
        }
        if script != 0 && ui.small_button("Open").clicked() {
            subject.behaviour.open_script(vale_mangos::scripts::GOSSIP, script);
        }
    });
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
            broadcast_line(ui, subject, line, &format!("gossip-line-{}-{n}", npc_text.row.id));
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
    row(ui, "line", |ui| match o.broadcast_text {
        0 => {
            let mut text = o.text.clone();
            let edit = egui::TextEdit::singleline(&mut text)
                .id_salt(("gossip-option-text", menu, o.id))
                .desired_width(f32::INFINITY);
            if ui.add(edit).changed() {
                set(subject, "option_text", vale_mangos::sql::text(&text));
            }
        }
        line => broadcast_line(ui, subject, line, &format!("gossip-option-line-{menu}-{}", o.id)),
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
    });
    row(ui, "script", |ui| {
        let mut script = o.action_script;
        if ui
            .add(egui::DragValue::new(&mut script).speed(1.0))
            .on_hover_text("A gossip_scripts id run when it is clicked, or 0.")
            .changed()
        {
            set(subject, "action_script_id", script.to_string());
        }
        if script != 0 && ui.small_button("Open").clicked() {
            subject.behaviour.open_script(vale_mangos::scripts::GOSSIP, script);
        }
        let mut poi = o.action_poi;
        ui.label(egui::RichText::new("point of interest").color(theme::INK_DIM));
        if ui
            .add(egui::DragValue::new(&mut poi).speed(1.0))
            .on_hover_text("A points_of_interest row marked on the map when it is clicked, or 0.")
            .changed()
        {
            set(subject, "action_poi_id", poi.to_string());
        }
    });
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
    if o.box_broadcast_text == 0 {
        row(ui, "box text", |ui| {
            let mut text = o.box_text.clone();
            let edit = egui::TextEdit::singleline(&mut text)
                .id_salt(("gossip-box-text", menu, o.id))
                .desired_width(f32::INFINITY);
            if ui.add(edit).on_hover_text("A confirmation box asked before the option acts. Empty for none.").changed() {
                set(subject, "box_text", vale_mangos::sql::text(&text));
            }
        });
    } else {
        row(ui, "box text", |ui| broadcast_line(ui, subject, o.box_broadcast_text, &format!("gossip-box-line-{menu}-{}", o.id)));
    }
    let mut condition = o.condition_id;
    row(ui, "condition", |ui| {
        if ui.add(egui::DragValue::new(&mut condition).speed(1.0)).changed() {
            set(subject, "condition_id", condition.to_string());
        }
        super::conditions::open_button(ui, condition);
    });
    for fault in o.check() {
        ui.label(egui::RichText::new(fault).size(theme::SMALL).color(theme::WARN));
    }
}

/// Take back a removal.
fn keep(session: &mut EditSession, table: &str, key: &vale_mangos::row::Key, now: f64) {
    let label = format!("{table} {}", key.text());
    session.set_server_row(table, key, None, Some(crate::session::Gesture { label: "Keep gossip row", subject: &label, now }));
}
