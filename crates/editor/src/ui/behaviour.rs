//! The three windows a creature's behaviour is edited in: its events, the
//! spell list its template names, and one script.
//!
//! ## Three windows over one creature
//!
//! Events and Spells are two buttons on a selected creature, beside Quests
//! and Loot, and each opens a window that follows the selection. A script is
//! opened from an event's action or a slot's script into the third window,
//! which stays on that script until another is opened.
//!
//! ## A row is drawn as the server reads it
//!
//! Every row is [`super::rowform`]'s page: one column per row, the name in a
//! fixed cell. An event's four parameters are named by its type, a script
//! row's eight data columns by its command and its two target parameters by
//! its target, from `vale_mangos::eventai` and `vale_mangos::scripts`,
//! so a form reads `spell_id` and `hp_max_percent` rather than `datalong` and
//! `event_param1`. A column that names a row of another table draws the
//! name and offers the picker ([`super::reference`]).
//!
//! ## Nothing here reaches the database
//!
//! An event or a list edit goes into the project's row store and onto the
//! undo stack; a script edit replaces the project's copy of the whole script.
//! A save writes `sql\behaviour.sql`; Apply is on the bar's Server… with the
//! other subjects'. See [`crate::server::behaviour`].

use super::reference;
use super::rowform::{
    choice_cell, flags_cell, meaning, number_cell, number_means, page_row, page_spacing, section,
    text_cell, FORM_ROW,
};
use super::theme;
use crate::session::EditSession;
use crate::tools::behaviour::{Behaviour, ShownEvent, ShownList};
use crate::tools::quests::{ColumnTarget, PickFor, Picker, Quests};
use vale_client::assets::GameAssets;
use vale_mangos::creaturespells::{self, SLOTS};
use vale_mangos::eventai;
use vale_mangos::row::{Key, Life};
use vale_mangos::schema::Kind;
use vale_mangos::scripts::{self, Script, ScriptRow};
use bevy_egui::egui;

/// Everything the windows need.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub behaviour: &'a mut Behaviour,
    /// The reference picker every form shares, and the name cache a
    /// reference resolves through.
    pub quests: &'a mut Quests,
    pub assets: &'a GameAssets,
    pub now: f64,
}

/// The frame every window here is drawn in.
fn frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::SHELL)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(8))
}

/// The three windows, whichever are open, and the picker they share.
/// Answers their rectangles, so a click inside one is not also a click on
/// the ground behind it.
pub fn windows(ctx: &egui::Context, mut subject: Subject<'_>) -> Vec<egui::Rect> {
    let mut out = Vec::new();
    if let Some((table, id, row, column, value)) = subject.quests.script_pick.take() {
        if let Some(mut script) = subject.behaviour.script_of(table, id, subject.session) {
            if let Some(target) = script.rows.get_mut(row) {
                target.set(column, &value.to_string());
                let now = subject.now;
                subject.behaviour.set_script(subject.session, &script, now);
            }
        }
    }
    out.extend(events_window(ctx, &mut subject));
    out.extend(spells_window(ctx, &mut subject));
    out.extend(script_window(ctx, &mut subject));
    super::quests::picker(ctx, subject.session, subject.quests, subject.assets, None, subject.now);
    out
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// The events window: every `creature_ai_events` row of the creature, each
/// as a folding block, and the button that adds one.
fn events_window(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    if !subject.behaviour.events_open {
        return None;
    }
    let about = subject.behaviour.about.clone()?;
    let mut keep_open = true;
    let shown = egui::Window::new(format!("{} \u{2014} events", about.label))
        .id(egui::Id::new("events-window"))
        .open(&mut keep_open)
        .default_size([720.0, 520.0])
        .default_pos([300.0, 120.0])
        .resizable(true)
        .frame(frame())
        .show(ctx, |ui| {
            page_spacing(ui);
            ui.label(
                egui::RichText::new(format!(
                    "creature_ai_events for creature_template {}: every spawn of it, on every map.",
                    about.entry
                ))
                .small()
                .color(theme::WARN),
            );
            ai_name_line(ui, subject, &about.template_key, &about.ai_name);
            if let Some(trouble) = subject.behaviour.trouble.clone() {
                ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
                return;
            }
            if !subject.behaviour.events_read(about.entry) {
                theme::waiting(ui, "reading the events\u{2026}");
                return;
            }
            let events = subject.behaviour.events_of(about.entry, &subject.session.server_edits);
            egui::Panel::bottom("events-foot")
                .resizable(false)
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 4)))
                .show(ui, |ui| {
                    if ui
                        .small_button("+ event")
                        .on_hover_text(
                            "A new event of this creature: a repeating timer in combat at five \
                             to ten seconds, with no script yet. Numbered creature entry times \
                             100 plus the next free slot, which is the reference database's \
                             own convention.",
                        )
                        .clicked()
                    {
                        let now = subject.now;
                        subject.behaviour.add_event(subject.session, about.entry, now);
                    }
                });
            egui::ScrollArea::vertical()
                .max_height(ui.available_height().max(60.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if events.is_empty() {
                        theme::note(ui, "No events. The creature reacts to nothing beyond its AI class.");
                    }
                    for shown in &events {
                        event_block(ui, subject, shown);
                    }
                });
        });
    if !keep_open {
        subject.behaviour.events_open = false;
    }
    shown.map(|shown| shown.response.rect)
}

/// Whether the creature's AI class reads the events table, and the button
/// that makes it so.
fn ai_name_line(ui: &mut egui::Ui, subject: &mut Subject<'_>, template_key: &Key, ai_name: &str) {
    if ai_name == "EventAI" {
        return;
    }
    ui.horizontal(|ui| {
        let shown = match ai_name.is_empty() {
            true => "the default AI".to_string(),
            false => ai_name.to_string(),
        };
        ui.label(
            egui::RichText::new(format!(
                "ai_name is {shown}, which never reads these events; only EventAI does."
            ))
            .small()
            .color(theme::BAD),
        );
        if ui
            .small_button("Set ai_name to EventAI")
            .on_hover_text(
                "Write EventAI into creature_template.ai_name. A guard or a pet keeps its own \
                 class with the events on top (GuardEventAI, PetEventAI).",
            )
            .clicked()
        {
            let subject_line = format!("{} {} ai_name", vale_mangos::creature::TEMPLATE, template_key.text());
            subject.session.set_server_edit(
                vale_mangos::creature::TEMPLATE,
                template_key,
                "ai_name",
                Some(vale_mangos::sql::text("EventAI")),
                Some(crate::session::Gesture { label: "Edit creature", subject: &subject_line, now: subject.now }),
            );
        }
    });
}

/// One event: a folding block headed by its id and its trigger in words,
/// holding its columns.
fn event_block(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &ShownEvent) {
    let event = &shown.event;
    let colour = match shown.life {
        Life::Insert => theme::WARN,
        Life::Delete => theme::BAD,
        Life::Update => theme::INK,
    };
    let title = format!("{} \u{b7} {}", event.id, event.summary());
    egui::CollapsingHeader::new(egui::RichText::new(title).size(13.5).color(colour))
        .id_salt(("event", event.id))
        .default_open(false)
        .show(ui, |ui| {
            ui.add_enabled_ui(shown.life != Life::Delete, |ui| {
                event_fields(ui, subject, shown);
            });
            ui.horizontal(|ui| {
                let (label, about) = match shown.life {
                    Life::Delete => ("keep", "Take the removal mark off this event."),
                    Life::Insert => ("discard", "Give up this event; nothing in the database is touched."),
                    Life::Update => ("remove", "Mark the event for removal. Apply deletes it."),
                };
                if ui.small_button(label).on_hover_text(about).clicked() {
                    let now = subject.now;
                    match shown.life {
                        Life::Delete => subject.behaviour.keep_event(subject.session, shown, now),
                        _ => subject.behaviour.remove_event(subject.session, shown, now),
                    }
                }
                if shown.life == Life::Insert {
                    ui.label(egui::RichText::new("new: Apply writes it").small().color(theme::WARN));
                }
                if shown.life == Life::Delete {
                    ui.label(egui::RichText::new("marked for removal").small().color(theme::BAD));
                }
            });
        });
}

/// The columns of one event, one page row each.
fn event_fields(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &ShownEvent) {
    let event = shown.event.clone();
    let key = event.key();
    let kind = eventai::event_type(event.event_type);
    let id = ui.make_persistent_id(("event", event.id));
    let set = |subject: &mut Subject<'_>, column: &'static str, value: String| {
        let now = subject.now;
        subject.behaviour.set_event(subject.session, shown, column, value, now);
    };
    // Which columns the project has changed, decided once: a closure over
    // the session would hold it borrowed across the writes below.
    let changed: Vec<&'static str> = eventai::COLUMNS
        .iter()
        .map(|column| column.name)
        .filter(|column| subject.session.server_edits.get(eventai::TABLE, &key, column).is_some())
        .collect();
    let edited = |column: &str| changed.contains(&column);

    let written = page_row(ui, "event_type", "what fires it", edited("event_type"), |ui| {
        let written = choice_cell(ui, &event.get("event_type"), &eventai::EVENT_TYPE_VALUES, id.with("type"));
        if let Some(kind) = kind {
            meaning(ui, kind.about);
        }
        written
    });
    if let Some(written) = written {
        set(subject, "event_type", written);
    }
    for (at, column) in ["event_param1", "event_param2", "event_param3", "event_param4"].into_iter().enumerate() {
        let param = kind.and_then(|kind| kind.params[at]);
        let showing = event.get(column);
        let (name, about, kind) = match param {
            Some(param) => (param.name, param.about, param.kind),
            None => (column, "not read by this event type", Kind::Signed),
        };
        if param.is_none() && showing == "0" {
            continue;
        }
        let target = ColumnTarget {
            table: eventai::TABLE,
            key: key.clone(),
            column,
            in_database: shown.in_database.as_ref().map(|had| had.get(column)),
            label: "Edit event",
            subject: format!("{} {} {column}", eventai::TABLE, key.text()),
        };
        let written = page_row(ui, name, about, edited(column), |ui| {
            value_cell(ui, subject, kind, &showing, id.with(column), Some(target), column)
        });
        if let Some(written) = written {
            set(subject, column, written);
        }
    }
    let written = page_row(ui, "event_chance", "percent chance to fire when triggered", edited("event_chance"), |ui| {
        let written = number_cell(ui, Kind::Unsigned, &event.get("event_chance"));
        meaning(ui, "%");
        written
    });
    if let Some(written) = written {
        set(subject, "event_chance", written);
    }
    let written = page_row(ui, "event_flags", "repeatable, random action, not while casting", edited("event_flags"), |ui| {
        flags_cell(ui, &event.get("event_flags"), &eventai::EVENT_FLAGS, id.with("flags"))
    });
    if let Some(written) = written {
        set(subject, "event_flags", written);
    }
    let written = page_row(ui, "event_inverse_phase_mask", "the phases it does not fire in", edited("event_inverse_phase_mask"), |ui| {
        flags_cell(ui, &event.get("event_inverse_phase_mask"), &eventai::PHASES, id.with("phases"))
    });
    if let Some(written) = written {
        set(subject, "event_inverse_phase_mask", written);
    }
    let written = page_row(ui, "condition_id", "a row of conditions, or 0", edited("condition_id"), |ui| {
        number_cell(ui, Kind::Unsigned, &event.get("condition_id"))
    });
    if let Some(written) = written {
        set(subject, "condition_id", written);
    }
    for column in ["action1_script", "action2_script", "action3_script"] {
        let showing = event.get(column);
        let written = page_row(ui, column, "a creature_ai_scripts id run when the event fires, or 0", edited(column), |ui| {
            let written = number_cell(ui, Kind::Unsigned, &showing);
            let script: u32 = showing.trim().parse().unwrap_or(0);
            script_buttons(ui, subject, scripts::CREATURE_AI, script, Some(event.id)).or(written)
        });
        if let Some(written) = written {
            set(subject, column, written);
        }
    }
    let written = page_row(ui, "comment", "for the person; the server never reads it", edited("comment"), |ui| {
        text_cell(ui, id.with("comment"), &vale_mangos::sql::text(&event.comment))
    });
    if let Some(written) = written {
        set(subject, "comment", written);
    }
}

/// The buttons after a script id: open it, or make a new script. `Some` with
/// the id to write when a new script is made.
///
/// A new script takes the event's own id when the table has no script there
/// and the id is above every id read; otherwise one above the highest the
/// table holds. The script opens empty and the window adds rows to it.
fn script_buttons(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    table: &'static str,
    script: u32,
    preferred: Option<u32>,
) -> Option<String> {
    if script != 0 {
        let open = subject.behaviour.script_open && subject.behaviour.script == Some((table, script));
        if ui
            .selectable_label(open, egui::RichText::new("edit\u{2026}").size(12.0))
            .on_hover_text(format!("Open {table} {script} in the script window."))
            .clicked()
        {
            subject.behaviour.open_script(table, script);
        }
        return None;
    }
    meaning(ui, "none");
    let next = subject.behaviour.next_script_id(table);
    let id = match (preferred, next) {
        (Some(preferred), Some(max)) if preferred >= max => Some(preferred),
        (_, Some(next)) => Some(next),
        _ => None,
    };
    let button = ui.add_enabled(id.is_some(), egui::Button::new(egui::RichText::new("+ script").size(12.0)).small());
    let button = button
        .on_hover_text(format!(
            "A new {table} script, numbered {}. It opens in the script window with no rows.",
            id.map(|id| id.to_string()).unwrap_or_default()
        ))
        .on_disabled_hover_text("Reading the table's highest id.");
    if button.clicked() {
        let id = id?;
        let now = subject.now;
        subject
            .behaviour
            .set_script(subject.session, &Script::empty(table, id), now);
        subject.behaviour.open_script(table, id);
        return Some(id.to_string());
    }
    None
}

/// The value cell of one parameter, by its kind: a number, a menu, a mask,
/// or a reference with its name and picker after it.
#[allow(clippy::too_many_arguments)]
fn value_cell(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    kind: Kind,
    showing: &str,
    id: egui::Id,
    target: Option<ColumnTarget>,
    column: &str,
) -> Option<String> {
    match kind {
        Kind::Choice(values) => choice_cell(ui, showing, values, id),
        Kind::Flags(bits) => flags_cell(ui, showing, bits, id),
        Kind::Ref(dbc) => {
            let written = number_cell(ui, Kind::Unsigned, showing);
            let mut resolver = reference::Resolver {
                session: &mut *subject.session,
                assets: subject.assets,
                quests: &mut *subject.quests,
            };
            if let Some(id) = reference::cell(ui, &mut resolver, dbc, showing, target) {
                reference::name(ui, &mut resolver, dbc, id);
            }
            written
        }
        Kind::Millis | Kind::Seconds => {
            let written = number_cell(ui, Kind::Unsigned, showing);
            if let Some(means) = number_means(kind, showing) {
                meaning(ui, means);
            }
            written
        }
        kind => {
            let written = number_cell(ui, kind, showing);
            if column.contains("chance") || column.contains("probability") {
                meaning(ui, "%");
            }
            written
        }
    }
}

// ---------------------------------------------------------------------------
// Spell lists
// ---------------------------------------------------------------------------

/// The spells window: the `creature_spells` row the template's
/// `spell_list_id` names, one section per slot.
fn spells_window(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    if !subject.behaviour.spells_open {
        return None;
    }
    let about = subject.behaviour.about.clone()?;
    let mut keep_open = true;
    let shown = egui::Window::new(format!("{} \u{2014} spells", about.label))
        .id(egui::Id::new("spells-window"))
        .open(&mut keep_open)
        .default_size([720.0, 520.0])
        .default_pos([320.0, 140.0])
        .resizable(true)
        .frame(frame())
        .show(ctx, |ui| {
            page_spacing(ui);
            ui.label(
                egui::RichText::new(format!(
                    "creature_spells {}: creature_template {}'s spell_list_id. Every creature \
                     whose template names this list casts from it.",
                    about.spell_list_id, about.entry
                ))
                .small()
                .color(theme::WARN),
            );
            if let Some(trouble) = subject.behaviour.trouble.clone() {
                ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
                return;
            }
            if about.spell_list_id == 0 {
                theme::note(ui, "spell_list_id is 0: the creature casts nothing in combat.");
                new_list_button(ui, subject, &about.template_key, &about.label);
                return;
            }
            if !subject.behaviour.list_read(about.spell_list_id)
                && subject.behaviour.list_of(about.spell_list_id, &subject.session.server_edits).is_none()
            {
                theme::waiting(ui, "reading the list\u{2026}");
                return;
            }
            let Some(list) = subject.behaviour.list_of(about.spell_list_id, &subject.session.server_edits) else {
                ui.label(
                    egui::RichText::new(format!(
                        "creature_spells has no row {}. The server logs the missing list at \
                         start and the creature casts nothing.",
                        about.spell_list_id
                    ))
                    .small()
                    .color(theme::BAD),
                );
                if ui
                    .small_button(format!("+ create list {}", about.spell_list_id))
                    .on_hover_text("A creature_spells row at that entry, with every slot empty.")
                    .clicked()
                {
                    let now = subject.now;
                    subject.behaviour.create_list(subject.session, about.spell_list_id, &about.label, now);
                }
                return;
            };
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .show(ui, |ui| list_fields(ui, subject, &list));
        });
    if !keep_open {
        subject.behaviour.spells_open = false;
    }
    shown.map(|shown| shown.response.rect)
}

/// The button that makes a list at the next free entry and points the
/// template at it.
fn new_list_button(ui: &mut egui::Ui, subject: &mut Subject<'_>, template_key: &Key, label: &str) {
    let next = subject.behaviour.next_list_entry();
    let button = ui
        .add_enabled(next.is_some(), egui::Button::new("+ new list").small())
        .on_hover_text(format!(
            "A creature_spells row at entry {}, with every slot empty, and \
             creature_template.spell_list_id set to it.",
            next.map(|n| n.to_string()).unwrap_or_default()
        ))
        .on_disabled_hover_text("Reading the table's highest entry.");
    if button.clicked() {
        let Some(entry) = next else { return };
        let now = subject.now;
        subject.behaviour.create_list(subject.session, entry, label, now);
        let subject_line = format!("{} {} spell_list_id", vale_mangos::creature::TEMPLATE, template_key.text());
        subject.session.set_server_edit(
            vale_mangos::creature::TEMPLATE,
            template_key,
            "spell_list_id",
            Some(entry.to_string()),
            Some(crate::session::Gesture { label: "Edit creature", subject: &subject_line, now }),
        );
    }
}

/// The list's name and its eight slots.
fn list_fields(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &ShownList) {
    let list = shown.list.clone();
    let key = list.key();
    let id = ui.make_persistent_id(("spell-list", list.entry));
    let changed: Vec<&'static str> = creaturespells::COLUMNS
        .iter()
        .map(|column| column.name)
        .filter(|column| subject.session.server_edits.get(creaturespells::TABLE, &key, column).is_some())
        .collect();
    let edited = |column: &str| changed.contains(&column);
    if shown.life == Life::Insert {
        ui.label(egui::RichText::new("New: this list is in no database yet. Apply writes it.").small().color(theme::WARN));
    }
    let written = page_row(ui, "name", "for the person; the server never reads it", edited("name"), |ui| {
        text_cell(ui, id.with("name"), &vale_mangos::sql::text(&list.name))
    });
    if let Some(written) = written {
        let now = subject.now;
        subject.behaviour.set_list(subject.session, shown, "name", written, now);
    }
    for slot in 0..SLOTS {
        let columns = creaturespells::slot_columns(slot);
        let held = list.slots[slot];
        let spell_name = match held.spell {
            0 => "empty".to_string(),
            spell => {
                super::quests::open_for_names(subject.session, subject.assets, "Spell");
                super::quests::name_in(subject.session, "Spell", spell).unwrap_or_else(|| format!("spell {spell}"))
            }
        };
        let touched = columns.iter().any(|column| edited(column.name));
        section(
            ui,
            ("spell-slot", list.entry, slot),
            &format!("Slot {} \u{b7} {spell_name}", slot + 1),
            touched,
            held.spell != 0,
            |ui| {
                let target_kind = scripts::target(held.cast_target);
                for (field, column) in columns.iter().enumerate() {
                    let showing = list.get(column.name);
                    // The two target parameters are named by the cast target.
                    let (name, about, kind) = match (field, target_kind) {
                        (3, Some(target)) if !target.param1.is_empty() => (target.param1, column.about, column.kind),
                        (4, Some(target)) if !target.param2.is_empty() => (target.param2, column.about, column.kind),
                        (3 | 4, _) if showing == "0" => continue,
                        _ => (column.name, column.about, column.kind),
                    };
                    let target = ColumnTarget {
                        table: creaturespells::TABLE,
                        key: key.clone(),
                        column: column.name,
                        in_database: shown.in_database.as_ref().map(|had| had.get(column.name)),
                        label: "Edit spell list",
                        subject: format!("{} {} {}", creaturespells::TABLE, key.text(), column.name),
                    };
                    let written = page_row(ui, name, about, edited(column.name), |ui| {
                        match field {
                            10 => {
                                let written = number_cell(ui, Kind::Unsigned, &showing);
                                let script: u32 = showing.trim().parse().unwrap_or(0);
                                script_buttons(ui, subject, scripts::CREATURE_SPELLS, script, None).or(written)
                            }
                            _ => value_cell(ui, subject, kind, &showing, id.with(column.name), Some(target), column.name),
                        }
                    });
                    if let Some(written) = written {
                        let now = subject.now;
                        subject.behaviour.set_list(subject.session, shown, column.name, written, now);
                    }
                }
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Scripts
// ---------------------------------------------------------------------------

/// The script window: every row of one script, in the order the server runs
/// them, each a folding block, and the button that adds a row.
fn script_window(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    if !subject.behaviour.script_open {
        return None;
    }
    let (table, id) = subject.behaviour.script?;
    let mut keep_open = true;
    let shown = egui::Window::new(format!("{table} {id} \u{2014} script"))
        .id(egui::Id::new("script-window"))
        .open(&mut keep_open)
        .default_size([760.0, 560.0])
        .default_pos([340.0, 160.0])
        .resizable(true)
        .frame(frame())
        .show(ctx, |ui| {
            page_spacing(ui);
            let table_info = scripts::table(table);
            ui.label(
                egui::RichText::new(format!(
                    "Named by {}. Every row under the id runs when the script starts, each after \
                     its delay.",
                    table_info.map(|t| t.keyed_by).unwrap_or("")
                ))
                .small()
                .color(theme::WARN),
            );
            match table_info.and_then(|t| t.reload) {
                Some(reload) => theme::note(ui, format!("Live on `.reload {reload}`, which an apply sends.")),
                None => theme::note(ui, "Read at server start: an applied change needs a restart."),
            }
            if let Some(trouble) = subject.behaviour.trouble.clone() {
                ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
                return;
            }
            let Some(script) = subject.behaviour.script_of(table, id, subject.session) else {
                theme::waiting(ui, "reading the script\u{2026}");
                return;
            };
            let claimed = subject.session.server_scripts.touches(table, id);
            let in_database = subject.behaviour.script_in_database(table, id);
            match (claimed, in_database, script.rows.is_empty()) {
                (true, true, true) => {
                    ui.label(egui::RichText::new("Removed: Apply deletes every row under the id.").small().color(theme::BAD));
                }
                (true, false, _) => {
                    ui.label(egui::RichText::new("New: this script is in no database yet. Apply writes it.").small().color(theme::WARN));
                }
                (true, true, false) => {
                    ui.label(egui::RichText::new("Changed: Apply replaces every row under the id.").small().color(theme::WARN));
                }
                _ => {}
            }
            egui::Panel::bottom("script-foot")
                .resizable(false)
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 4)))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("+ row")
                            .on_hover_text("A Talk row at the last row's delay. Change its command first.")
                            .clicked()
                        {
                            let mut next = script.clone();
                            let mut row = ScriptRow::new(0);
                            row.delay = next.rows.iter().map(|row| row.delay).max().unwrap_or(0);
                            next.rows.push(row);
                            let now = subject.now;
                            subject.behaviour.set_script(subject.session, &next, now);
                        }
                        if claimed
                            && ui
                                .small_button("discard changes")
                                .on_hover_text("Give up this project's claim on the script; the database's rows stand.")
                                .clicked()
                        {
                            let now = subject.now;
                            let subject_line = format!("{table} {id}");
                            subject.session.set_server_script(
                                table,
                                id,
                                None,
                                Some(crate::session::Gesture { label: "Edit script", subject: &subject_line, now }),
                            );
                        }
                    });
                });
            egui::ScrollArea::vertical()
                .max_height(ui.available_height().max(60.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if script.rows.is_empty() {
                        theme::note(ui, "No rows. A script with no rows does nothing; applied, it deletes the rows the database holds.");
                    }
                    let sorted = script.sorted();
                    for (at, row) in sorted.iter().enumerate() {
                        script_row_block(ui, subject, &script, at, row);
                    }
                });
        });
    if !keep_open {
        subject.behaviour.script_open = false;
    }
    shown.map(|shown| shown.response.rect)
}

/// One row of a script: a folding block headed by its delay and its command
/// in words. `at` is the row's index in the sorted script, which is the
/// order the store keeps.
fn script_row_block(ui: &mut egui::Ui, subject: &mut Subject<'_>, script: &Script, at: usize, row: &ScriptRow) {
    let title = format!("{} ms \u{b7} {}", row.delay, row.summary());
    egui::CollapsingHeader::new(egui::RichText::new(title).size(13.5).color(theme::INK))
        .id_salt(("script-row", script.table, script.id, at))
        .default_open(script.rows.len() <= 3)
        .show(ui, |ui| {
            script_row_fields(ui, subject, script, at, row);
            if ui
                .small_button("remove row")
                .on_hover_text("Take this row out of the script.")
                .clicked()
            {
                let mut next = Script { table: script.table, id: script.id, rows: script.sorted() };
                if at < next.rows.len() {
                    next.rows.remove(at);
                }
                let now = subject.now;
                subject.behaviour.set_script(subject.session, &next, now);
            }
        });
}

/// The columns of one script row, named by its command and its target.
fn script_row_fields(ui: &mut egui::Ui, subject: &mut Subject<'_>, script: &Script, at: usize, row: &ScriptRow) {
    let command = scripts::command(row.command);
    let target = scripts::target(row.target_type);
    let id = ui.make_persistent_id(("script-row", script.table, script.id, at));
    let write = |subject: &mut Subject<'_>, column: &'static str, value: &str| {
        let mut next = Script { table: script.table, id: script.id, rows: script.sorted() };
        if let Some(target) = next.rows.get_mut(at) {
            if target.set(column, value) {
                let now = subject.now;
                subject.behaviour.set_script(subject.session, &next, now);
            }
        }
    };
    let number = |ui: &mut egui::Ui, kind: Kind, showing: &str| number_cell(ui, kind, showing);

    if let Some(written) = page_row(ui, "delay", "milliseconds after the script starts", false, |ui| {
        let written = number(ui, Kind::Unsigned, &row.get("delay"));
        if let Some(means) = number_means(Kind::Millis, &row.get("delay")) {
            meaning(ui, means);
        }
        written
    }) {
        write(subject, "delay", &written);
    }
    if let Some(written) = page_row(ui, "priority", "order among rows with one delay", false, |ui| number(ui, Kind::Unsigned, &row.get("priority"))) {
        write(subject, "priority", &written);
    }
    if let Some(written) = page_row(ui, "command", "what the row does", false, |ui| {
        let written = choice_cell(ui, &row.get("command"), &scripts::COMMAND_VALUES, id.with("command"));
        if let Some(command) = command {
            meaning(ui, format!("on {}", command.source));
        }
        written
    }) {
        write(subject, "command", &written);
    }
    // The eight data columns, named by the command; a column the command
    // does not read is drawn only while it holds something.
    let datalong = ["datalong", "datalong2", "datalong3", "datalong4"];
    let dataint = ["dataint", "dataint2", "dataint3", "dataint4"];
    for (columns, params, fallback) in [
        (datalong, command.map(|c| c.datalong), Kind::Unsigned),
        (dataint, command.map(|c| c.dataint), Kind::Signed),
    ] {
        for (index, column) in columns.into_iter().enumerate() {
            let param = params.and_then(|params| params[index]);
            let showing = row.get(column);
            let (name, about, kind) = match param {
                Some(param) => (param.name, param.about, param.kind),
                None if showing == "0" => continue,
                None => (column, "not read by this command", fallback),
            };
            if let Some(written) = page_row(ui, name, about, false, |ui| {
                script_value_cell(ui, subject, script, at, column, kind, &showing, id.with(column))
            }) {
                write(subject, column, &written);
            }
        }
    }
    if let Some(written) = page_row(ui, "target_type", "who the command acts on", false, |ui| {
        choice_cell(ui, &row.get("target_type"), &scripts::TARGET_VALUES, id.with("target"))
    }) {
        write(subject, "target_type", &written);
    }
    for (column, name) in [("target_param1", target.map(|t| t.param1)), ("target_param2", target.map(|t| t.param2))] {
        let showing = row.get(column);
        let name = match name {
            Some(name) if !name.is_empty() => name,
            _ if showing == "0" => continue,
            _ => column,
        };
        if let Some(written) = page_row(ui, name, "named by the target type", false, |ui| number(ui, Kind::Unsigned, &showing)) {
            write(subject, column, &written);
        }
    }
    if let Some(written) = page_row(ui, "data_flags", "how source and target are swapped, and what a failure does", false, |ui| {
        flags_cell(ui, &row.get("data_flags"), &scripts::DATA_FLAGS, id.with("flags"))
    }) {
        write(subject, "data_flags", &written);
    }
    let coords = command.is_some_and(|c| c.coords) || row.x != 0.0 || row.y != 0.0 || row.z != 0.0 || row.o != 0.0;
    if coords {
        for column in ["x", "y", "z", "o"] {
            if let Some(written) = page_row(ui, column, "read by the commands that move or place", false, |ui| number(ui, Kind::Float, &row.get(column))) {
                write(subject, column, &written);
            }
        }
    }
    if let Some(written) = page_row(ui, "condition_id", "a row of conditions the row is skipped without, or 0", false, |ui| number(ui, Kind::Unsigned, &row.get("condition_id"))) {
        write(subject, "condition_id", &written);
    }
    if let Some(written) = page_row(ui, "comments", "for the person; the server never reads it", false, |ui| {
        text_cell(ui, id.with("comments"), &vale_mangos::sql::text(&row.comments))
    }) {
        write(subject, "comments", &super::rowform::unquote(&written));
    }
}

/// The value cell of one script data column: as [`value_cell`], with the
/// picker answering through [`PickFor::ScriptCell`] since a script is not
/// in the row store.
#[allow(clippy::too_many_arguments)]
fn script_value_cell(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    script: &Script,
    at: usize,
    column: &'static str,
    kind: Kind,
    showing: &str,
    id: egui::Id,
) -> Option<String> {
    match kind {
        Kind::Ref(dbc) => {
            let written = number_cell(ui, Kind::Unsigned, showing);
            if let Some(kind) = super::quests::target_of(dbc) {
                if ui
                    .add(egui::Button::new(egui::RichText::new("\u{2026}").size(13.0)).min_size(egui::vec2(24.0, FORM_ROW)))
                    .on_hover_text(format!("choose {column} by name"))
                    .clicked()
                {
                    subject.quests.picker = Some(Picker::new(
                        kind,
                        PickFor::ScriptCell { table: script.table, id: script.id, row: at, column },
                    ));
                }
            }
            let script_id: u32 = showing.trim().parse().unwrap_or(0);
            if scripts::table_named(dbc).is_some() {
                return script_buttons(ui, subject, scripts::table_named(dbc).unwrap_or(scripts::GENERIC), script_id, None)
                    .or(written);
            }
            let mut resolver = reference::Resolver {
                session: &mut *subject.session,
                assets: subject.assets,
                quests: &mut *subject.quests,
            };
            if let Some(id) = reference::cell(ui, &mut resolver, dbc, showing, None) {
                reference::name(ui, &mut resolver, dbc, id);
            }
            written
        }
        _ => value_cell(ui, subject, kind, showing, id, None, column),
    }
}
