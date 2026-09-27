//! The three windows a creature's behaviour is edited in, its events, the
//! spell list its template names, and one script, and the four choosers that
//! start an edit from what the database already holds.
//!
//! ## Events as triggers and steps
//!
//! The events window draws each `creature_ai_events` row as a card that reads
//! top to bottom as what the server does: `WHEN` and the trigger as a sentence
//! (`Health at or below 15%`), the conditions the sentence leaves out after it
//! (`once`, `50%`, `not in phase 2`), and `THEN` with every step of every
//! script the event runs, each after the time it runs at. `ONE OF` replaces
//! `THEN` when the event runs one script at random. The card's `details`
//! unfolds every column of the row in [`super::rowform`]'s page layout, so no
//! column is out of reach: an event's four parameters are named by its type
//! and the action columns take a script by id, by choosing one, or as a new
//! one.
//!
//! ## A script as a list of steps
//!
//! The script window draws a script as the steps the server runs, in its
//! `(delay, priority)` order. Between two steps is the wait between them in
//! seconds, which is the difference of their delays and can be dragged;
//! dragging it moves that step and every later step, so the later waits stay
//! as they were. A `creature_ai_scripts` script has no waits: the server runs
//! its steps at once and logs a delay there as unsupported, so the window
//! offers none and says what does wait, a Start script step. Each step can
//! be moved up or down, copied, removed, and a step can be put in before any
//! other; `vale_mangos::scripts::Script`'s step operations keep the delays
//! and priorities consistent with the order shown.
//! A step's `details` unfolds its every column, named by its command and its
//! target, including the absolute `delay` and `priority`. A Talk step's
//! details also hold the `broadcast_text` row each of its text cells names,
//! as a form of its own.
//!
//! ## The spell list as one line per slot
//!
//! Each of the eight slots is one line, `Cast Shoot on the current victim,
//! after 2 s to 5 s, then every 3 s to 6 s`, with its chance beside it, and
//! unfolds to its eleven columns. A list is shared by every template that
//! names it; the window says how many do, and Copy into a new list gives
//! this creature a list of its own.
//!
//! ## Nothing here reaches the database
//!
//! An event, a list or a text edit goes into the project's row store and onto
//! the undo stack; a script edit replaces the project's copy of the whole
//! script. A save writes `sql\behaviour.sql`; Apply is on the bar's Server…
//! with the other subjects'. See [`crate::server::behaviour`].

use std::cell::RefCell;
use std::collections::HashSet;

use super::reference;
use super::rowform::{
    choice_cell, flags_cell, meaning, number_cell, number_means, page_row, page_spacing, text_cell,
    unquote, FORM_ROW,
};
use super::theme;
use crate::session::EditSession;
use crate::tools::behaviour::{
    Behaviour, Chooser, CopyScripts, ScriptAnswer, Search, ShownEvent, ShownList, ShownText,
};
use crate::tools::quests::{ColumnTarget, Holder, PickFor, Picker, Quests};
use vale_client::assets::GameAssets;
use vale_mangos::broadcast;
use vale_mangos::creaturespells::{self, List, SLOTS};
use vale_mangos::eventai::{self, Event};
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

/// The client tables a sentence names rows of, opened once a frame so the
/// lookup can read them without the session borrowed mutably.
const NAMED_TABLES: [&str; 5] = ["Spell", "Emotes", "EmotesText", "FactionTemplate", "Languages"];

/// The triggers offered first under + New event, the ones the reference
/// database uses most. Every trigger is under the submenu after them.
const COMMON_TRIGGERS: [u32; 11] = [4, 0, 1, 2, 6, 11, 7, 30, 5, 8, 21];

/// The commands offered first under + Step, the ones the reference database's
/// `creature_ai_scripts` uses most. Every command is under the submenu.
const COMMON_COMMANDS: [u32; 16] = [0, 1, 15, 14, 74, 10, 18, 44, 47, 3, 20, 25, 42, 43, 52, 39];

/// How many steps of a script an event card lists before it says how many
/// more there are.
const STEPS_ON_A_CARD: usize = 6;

/// The frame every window here is drawn in.
fn frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::SHELL)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(8))
}

/// The frame of one card: an event, a step or a search hit.
fn card(stroke: egui::Color32) -> egui::Frame {
    egui::Frame::default()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 6))
}

/// The colour a row's life is drawn in: created amber, removed red, edited
/// or untouched the line colour.
fn life_colour(life: Life) -> egui::Color32 {
    match life {
        Life::Insert => theme::WARN,
        Life::Delete => theme::BAD,
        Life::Update => theme::LINE,
    }
}

/// A short word at the start of a line of a card: `WHEN`, `THEN`, `ONE OF`.
fn badge(ui: &mut egui::Ui, text: &str, colour: egui::Color32) {
    egui::Frame::default()
        .fill(theme::SUNK)
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.set_min_width(44.0);
            ui.label(egui::RichText::new(text).size(10.5).strong().color(colour));
        });
}

/// The fold button at the end of a card's first line. Answers whether it was
/// clicked.
fn fold_button(ui: &mut egui::Ui, open: bool) -> bool {
    let text = match open {
        true => "hide details",
        false => "details",
    };
    ui.selectable_label(open, egui::RichText::new(text).size(11.5)).clicked()
}

/// A card's first line: `left` wraps in the room `right` leaves, and
/// `right` is laid out from the right edge first, so a long sentence wraps
/// beside the buttons rather than under them.
fn header<L, R>(ui: &mut egui::Ui, left: impl FnOnce(&mut egui::Ui) -> L, right: impl FnOnce(&mut egui::Ui) -> R) -> (L, R) {
    egui::Sides::new().shrink_left().wrap().show(ui, left, right)
}

/// A small button that answers whether it was clicked.
fn small(ui: &mut egui::Ui, text: &str, hover: &str) -> bool {
    ui.small_button(text).on_hover_text(hover).clicked()
}

/// Names the rows a sentence refers to, from what is in hand: the client
/// tables through the session, creatures, objects, items and quests through
/// the quest tool's cache (which is asked for the ones it lacks), and texts
/// through the behaviour tool's.
struct Namer<'s> {
    session: &'s EditSession,
    behaviour: &'s Behaviour,
    quests: RefCell<&'s mut Quests>,
}

impl Namer<'_> {
    fn name(&self, table: &str, id: u32) -> Option<String> {
        match table {
            "broadcast_text" => self.behaviour.said(id, &self.session.server_edits),
            "creature_template" => self.quests.borrow_mut().holder(Holder::Creature, id),
            "gameobject_template" => self.quests.borrow_mut().holder(Holder::Object, id),
            "item_template" => self.quests.borrow_mut().item(id).map(|found| found.name),
            "quest_template" => self.quests.borrow().title_of(id, &self.session.server_edits),
            dbc => super::quests::name_in(self.session, dbc, id),
        }
    }

    fn event(&self, event: &Event) -> String {
        event.sentence(&|table: &str, id: u32| self.name(table, id))
    }

    fn step(&self, row: &ScriptRow) -> String {
        row.sentence(&|table: &str, id: u32| self.name(table, id))
    }

    fn slot(&self, slot: &creaturespells::Slot) -> String {
        slot.sentence(&|table: &str, id: u32| self.name(table, id))
    }
}

/// Build a [`Namer`] over the subject's parts.
fn namer<'s>(session: &'s EditSession, behaviour: &'s Behaviour, quests: &'s mut Quests) -> Namer<'s> {
    Namer { session, behaviour, quests: RefCell::new(quests) }
}

/// One script's steps as a card lists them: the time each runs at after the
/// script starts, and its sentence. `None` while the script is read.
fn steps_of(namer: &Namer<'_>, table: &'static str, id: u32) -> Option<Vec<(u32, String)>> {
    let script = namer.behaviour.script_of(table, id, namer.session)?;
    Some(script.sorted().iter().map(|row| (row.delay, namer.step(row))).collect())
}

/// The three windows, whichever are open, the chooser, and the picker they
/// share. Answers their rectangles, so a click inside one is not also a click
/// on the ground behind it.
pub fn windows(ctx: &egui::Context, mut subject: Subject<'_>) -> Vec<egui::Rect> {
    let mut out = Vec::new();
    if let Some((table, id, row, column, value)) = subject.quests.script_pick.take() {
        write_cell(&mut subject, table, id, row, column, value);
    }
    let any_open = subject.behaviour.events_open
        || subject.behaviour.spells_open
        || subject.behaviour.script_open
        || subject.behaviour.chooser.is_some();
    if any_open {
        for table in NAMED_TABLES {
            super::quests::open_for_names(subject.session, subject.assets, table);
        }
    }
    out.extend(events_window(ctx, &mut subject));
    out.extend(spells_window(ctx, &mut subject));
    out.extend(script_window(ctx, &mut subject));
    chooser(ctx, &mut subject);
    super::quests::picker(ctx, subject.session, subject.quests, subject.assets, None, subject.now);
    out
}

/// One cell of one step of a script set to `value`, and the script written.
fn write_cell(subject: &mut Subject<'_>, table: &'static str, id: u32, row: usize, column: &'static str, value: u32) {
    let Some(script) = subject.behaviour.script_of(table, id, subject.session) else {
        return;
    };
    let mut rows = script.sorted();
    if let Some(target) = rows.get_mut(row) {
        if target.set(column, &value.to_string()) {
            let next = Script { table, id, rows };
            let now = subject.now;
            subject.behaviour.set_script(subject.session, &next, now);
        }
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// What an event card draws, worked out before the card is drawn so the
/// drawing can borrow the subject mutably.
struct EventCard {
    shown: ShownEvent,
    trigger: String,
    terms: Vec<String>,
    /// Each script the event runs: its id, whether the project changes it,
    /// and its steps, or `None` while it is read.
    scripts: Vec<(u32, bool, Option<Vec<(u32, String)>>)>,
}

/// The events window: a card for every `creature_ai_events` row of the
/// creature, and the buttons that add one.
fn events_window(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    if !subject.behaviour.events_open {
        return None;
    }
    let about = subject.behaviour.about.clone()?;
    let mut keep_open = true;
    let shown = egui::Window::new(format!("{} \u{2014} events", about.label))
        .id(egui::Id::new("events-window"))
        .open(&mut keep_open)
        .default_size([760.0, 600.0])
        .default_pos([300.0, 110.0])
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
                .color(theme::INK_DIM),
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
            events_toolbar(ui, subject, about.entry, &events);
            let cards: Vec<EventCard> = {
                let namer = namer(subject.session, subject.behaviour, subject.quests);
                events
                    .iter()
                    .map(|shown| EventCard {
                        trigger: namer.event(&shown.event),
                        terms: shown.event.terms(),
                        scripts: shown
                            .event
                            .scripts
                            .iter()
                            .filter(|id| **id != 0)
                            .map(|id| {
                                (
                                    *id,
                                    namer.session.server_scripts.touches(scripts::CREATURE_AI, *id),
                                    steps_of(&namer, scripts::CREATURE_AI, *id),
                                )
                            })
                            .collect(),
                        shown: shown.clone(),
                    })
                    .collect()
            };
            egui::ScrollArea::vertical()
                .max_height(ui.available_height().max(60.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if cards.is_empty() {
                        theme::note(
                            ui,
                            "No events. The creature reacts to nothing beyond its AI class. \
                             + New event starts one; + Existing event copies one from any creature.",
                        );
                    }
                    for event in &cards {
                        event_card(ui, subject, event);
                        ui.add_space(4.0);
                    }
                });
        });
    if !keep_open {
        subject.behaviour.events_open = false;
    }
    shown.map(|shown| shown.response.rect)
}

/// The row of buttons over the cards: a new event by trigger, an existing
/// event from any creature, and folding every card.
fn events_toolbar(ui: &mut egui::Ui, subject: &mut Subject<'_>, creature: u32, events: &[ShownEvent]) {
    ui.horizontal(|ui| {
        ui.menu_button("+ New event", |ui| {
            ui.set_min_width(220.0);
            ui.label(egui::RichText::new("When it\u{2026}").small().color(theme::INK_FAINT));
            for value in COMMON_TRIGGERS {
                let Some(kind) = eventai::event_type(value) else { continue };
                if ui.button(kind.name).on_hover_text(kind.about).clicked() {
                    let now = subject.now;
                    subject.behaviour.add_event_of(subject.session, creature, value, now);
                    ui.close();
                }
            }
            ui.separator();
            ui.menu_button("Every trigger", |ui| {
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for kind in eventai::EVENT_TYPES.iter() {
                        if ui.button(format!("{} \u{b7} {}", kind.value, kind.name)).on_hover_text(kind.about).clicked() {
                            let now = subject.now;
                            subject.behaviour.add_event_of(subject.session, creature, kind.value, now);
                            ui.close();
                        }
                    }
                });
            });
        })
        .response
        .on_hover_text(
            "A new event of this creature on the trigger chosen, with no script yet. Numbered \
             creature entry times 100 plus the next free slot, the reference database's own \
             convention.",
        );
        if ui
            .button("+ Existing event\u{2026}")
            .on_hover_text(
                "Search every creature's events by comment, id or creature id, and copy one to \
                 this creature: with copies of its scripts, or naming the same scripts.",
            )
            .clicked()
        {
            subject.behaviour.chooser = Some(Chooser::Events(Search::default()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("fold all").clicked() {
                subject.behaviour.unfolded_events.clear();
            }
            if ui.small_button("unfold all").clicked() {
                subject.behaviour.unfolded_events.extend(events.iter().map(|shown| shown.event.id));
            }
            ui.label(egui::RichText::new(format!("{} event(s)", events.len())).small().color(theme::INK_FAINT));
        });
    });
    ui.add_space(4.0);
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
            set_template(subject, template_key, "ai_name", vale_mangos::sql::text("EventAI"));
        }
    });
}

/// One column of the creature's template row set, on the creature form's
/// undo entry.
fn set_template(subject: &mut Subject<'_>, template_key: &Key, column: &'static str, value: String) {
    let subject_line = format!("{} {} {column}", vale_mangos::creature::TEMPLATE, template_key.text());
    subject.session.set_server_edit(
        vale_mangos::creature::TEMPLATE,
        template_key,
        column,
        Some(value),
        Some(crate::session::Gesture { label: "Edit creature", subject: &subject_line, now: subject.now }),
    );
}

/// One event's card: `WHEN` and the trigger, `THEN` and the steps, and the
/// row's columns when unfolded.
fn event_card(ui: &mut egui::Ui, subject: &mut Subject<'_>, card_of: &EventCard) {
    let shown = &card_of.shown;
    let event = &shown.event;
    let open = subject.behaviour.unfolded_events.contains(&event.id);
    card(life_colour(shown.life)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let colour = match shown.life {
            Life::Delete => theme::BAD,
            _ => theme::INK,
        };
        let (_, fold) = header(
            ui,
            |ui| {
                badge(ui, "WHEN", theme::ACCENT);
                ui.label(egui::RichText::new(&card_of.trigger).size(13.5).strong().color(colour));
            },
            |ui| {
                let fold = fold_button(ui, open);
                ui.label(egui::RichText::new(format!("#{}", event.id)).small().color(theme::INK_FAINT));
                if !card_of.terms.is_empty() {
                    ui.label(egui::RichText::new(card_of.terms.join(" \u{b7} ")).small().color(theme::INK_DIM));
                }
                fold
            },
        );
        if fold {
            match open {
                true => subject.behaviour.unfolded_events.remove(&event.id),
                false => subject.behaviour.unfolded_events.insert(event.id),
            };
        }
        if !event.comment.is_empty() {
            ui.label(egui::RichText::new(&event.comment).small().italics().color(theme::INK_FAINT));
        }
        then_lines(ui, subject, event.random_action(), &card_of.scripts);
        match shown.life {
            Life::Insert => {
                ui.label(egui::RichText::new("new: Apply writes it").small().color(theme::WARN));
            }
            Life::Delete => {
                ui.label(egui::RichText::new("marked for removal: Apply deletes it").small().color(theme::BAD));
            }
            Life::Update => {}
        }
        if open {
            ui.separator();
            event_details(ui, subject, shown);
        }
    });
}

/// The `THEN` half of a card: every step of every script, each after the
/// time it runs at. A script's id opens it in the script window.
fn then_lines(ui: &mut egui::Ui, subject: &mut Subject<'_>, random: bool, scripts: &[(u32, bool, Option<Vec<(u32, String)>>)]) {
    ui.horizontal_top(|ui| {
        match random {
            true => badge(ui, "ONE OF", theme::GOOD),
            false => badge(ui, "THEN", theme::GOOD),
        }
        ui.vertical(|ui| {
            if scripts.is_empty() {
                ui.label(egui::RichText::new("nothing: the event runs no script").color(theme::INK_FAINT));
            }
            for (at, (id, claimed, steps)) in scripts.iter().enumerate() {
                ui.horizontal(|ui| {
                    if random && scripts.len() > 1 {
                        ui.label(egui::RichText::new(format!("{}.", at + 1)).color(theme::INK_DIM));
                    }
                    let open = subject.behaviour.script_open && subject.behaviour.script == Some((scripts::CREATURE_AI, *id));
                    if ui
                        .selectable_label(open, egui::RichText::new(format!("script {id}")).small())
                        .on_hover_text(format!("Open creature_ai_scripts {id} in the script window."))
                        .clicked()
                    {
                        subject.behaviour.open_script(scripts::CREATURE_AI, *id);
                    }
                    if *claimed {
                        ui.label(egui::RichText::new("changed").small().color(theme::WARN));
                    }
                });
                match steps {
                    None => {
                        ui.label(egui::RichText::new("reading\u{2026}").small().color(theme::INK_FAINT));
                    }
                    Some(steps) if steps.is_empty() => {
                        ui.label(egui::RichText::new("no steps").small().color(theme::INK_FAINT));
                    }
                    Some(steps) => {
                        for (delay, sentence) in steps.iter().take(STEPS_ON_A_CARD) {
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    egui::vec2(52.0, 16.0),
                                    egui::Label::new(egui::RichText::new(step_time(*delay)).small().color(theme::INK_FAINT)),
                                );
                                ui.add(egui::Label::new(egui::RichText::new(sentence).color(theme::INK)).wrap());
                            });
                        }
                        if steps.len() > STEPS_ON_A_CARD {
                            ui.label(
                                egui::RichText::new(format!("\u{2026} and {} more", steps.len() - STEPS_ON_A_CARD))
                                    .small()
                                    .color(theme::INK_FAINT),
                            );
                        }
                    }
                }
            }
        });
    });
}

/// The time a step runs at, after its script starts: `at once` or `+5 s`.
/// A delay is seconds.
fn step_time(delay: u32) -> String {
    match delay {
        0 => "at once".to_string(),
        delay => format!("+{}", vale_mangos::schema::span_words(u64::from(delay) * 1000)),
    }
}

/// Every column of one event, in three groups: the trigger, the actions and
/// the note; then remove or keep.
fn event_details(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &ShownEvent) {
    ui.add_enabled_ui(shown.life != Life::Delete, |ui| {
        theme::heading(ui, "Trigger");
        trigger_fields(ui, subject, shown);
        theme::heading(ui, "Actions");
        action_fields(ui, subject, shown);
        theme::heading(ui, "Note");
        let changed = subject.session.server_edits.get(eventai::TABLE, &shown.event.key(), "comment").is_some();
        let id = ui.make_persistent_id(("event", shown.event.id));
        if let Some(written) = page_row(ui, "comment", "for the person; the server never reads it", changed, |ui| {
            text_cell(ui, id.with("comment"), &vale_mangos::sql::text(&shown.event.comment))
        }) {
            let now = subject.now;
            subject.behaviour.set_event(subject.session, shown, "comment", written, now);
        }
    });
    ui.horizontal(|ui| {
        let (label, about) = match shown.life {
            Life::Delete => ("keep", "Take the removal mark off this event."),
            Life::Insert => ("discard", "Give up this event; nothing in the database is touched."),
            Life::Update => ("remove", "Mark the event for removal. Apply deletes it."),
        };
        if small(ui, label, about) {
            let now = subject.now;
            match shown.life {
                Life::Delete => subject.behaviour.keep_event(subject.session, shown, now),
                _ => subject.behaviour.remove_event(subject.session, shown, now),
            }
        }
        if small(ui, "copy", "A copy of this event on this creature, naming the same scripts.") {
            let about = subject.behaviour.about.clone();
            if let Some(about) = about {
                let now = subject.now;
                let _ = subject.behaviour.copy_event(
                    subject.session,
                    about.entry,
                    &about.label,
                    &shown.event,
                    CopyScripts::Share,
                    now,
                );
            }
        }
    });
}

/// The columns of the trigger: its type and parameters, chance, flags,
/// phases and condition.
fn trigger_fields(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &ShownEvent) {
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
}

/// The action columns: whether the scripts run in turn or one at random,
/// and the three script ids with what each can be set to.
fn action_fields(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &ShownEvent) {
    let event = shown.event.clone();
    let key = event.key();
    let edited = |subject: &Subject<'_>, column: &str| subject.session.server_edits.get(eventai::TABLE, &key, column).is_some();
    let mut random = event.random_action();
    let was = random;
    let flags_edited = edited(subject, "event_flags");
    page_row(ui, "runs", "event_flags' Random action bit", flags_edited, |ui| {
        ui.allocate_ui(egui::vec2(280.0, FORM_ROW), |ui| {
            theme::segmented(ui, &mut random, &[("every script, in turn", false), ("one, at random", true)], |a, b| a == b);
        });
    });
    if random != was {
        let flags = match random {
            true => event.flags | 0x02,
            false => event.flags & !0x02,
        };
        let now = subject.now;
        subject.behaviour.set_event(subject.session, shown, "event_flags", flags.to_string(), now);
    }
    for column in ["action1_script", "action2_script", "action3_script"] {
        let showing = event.get(column);
        let target = ColumnTarget {
            table: eventai::TABLE,
            key: key.clone(),
            column,
            in_database: shown.in_database.as_ref().map(|had| had.get(column)),
            label: "Edit event",
            subject: format!("{} {} {column}", eventai::TABLE, key.text()),
        };
        let changed = edited(subject, column);
        let written = page_row(ui, column, "a creature_ai_scripts id run when the event fires, or 0", changed, |ui| {
            let written = number_cell(ui, Kind::Unsigned, &showing);
            let script: u32 = showing.trim().parse().unwrap_or(0);
            let preferred = (column == "action1_script").then_some(event.id);
            script_buttons(ui, subject, scripts::CREATURE_AI, script, preferred, ScriptAnswer::Column(target)).or(written)
        });
        if let Some(written) = written {
            let now = subject.now;
            subject.behaviour.set_event(subject.session, shown, column, written, now);
        }
    }
}

/// The buttons after a script id: open it, choose an existing one, make a
/// new one, or clear it. `Some` with the id to write when a new script is
/// made or the column is cleared; a chosen one is written by the chooser.
///
/// A new script takes `preferred`, the event's own id, when the table has no
/// script there and the id is above every id read; otherwise one above the
/// highest the table holds or this project claims. The script opens empty.
fn script_buttons(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    table: &'static str,
    script: u32,
    preferred: Option<u32>,
    answer: ScriptAnswer,
) -> Option<String> {
    let mut out = None;
    if script != 0 {
        let open = subject.behaviour.script_open && subject.behaviour.script == Some((table, script));
        if ui
            .selectable_label(open, egui::RichText::new("edit\u{2026}").size(12.0))
            .on_hover_text(format!("Open {table} {script} in the script window."))
            .clicked()
        {
            subject.behaviour.open_script(table, script);
        }
    } else {
        meaning(ui, "none");
    }
    if ui
        .small_button("choose\u{2026}")
        .on_hover_text(format!("Search {table} by comment or id, and use an existing script."))
        .clicked()
    {
        subject.behaviour.chooser = Some(Chooser::Script { table, answer, search: Search::default() });
    }
    let next = match preferred {
        Some(preferred) => subject
            .behaviour
            .next_script_id(table, subject.session)
            .map(|next| match preferred >= next && !subject.session.server_scripts.touches(table, preferred) {
                true => preferred,
                false => next,
            }),
        None => subject.behaviour.next_script_id(table, subject.session),
    };
    let button = ui
        .add_enabled(next.is_some(), egui::Button::new(egui::RichText::new("+ new").size(12.0)).small())
        .on_hover_text(format!(
            "A new {table} script, numbered {}. It opens in the script window with no steps.",
            next.map(|id| id.to_string()).unwrap_or_default()
        ))
        .on_disabled_hover_text("Reading the table's highest id.");
    if button.clicked() {
        if let Some(id) = next {
            let now = subject.now;
            subject.behaviour.set_script(subject.session, &Script::empty(table, id), now);
            subject.behaviour.open_script(table, id);
            out = Some(id.to_string());
        }
    }
    if script != 0 && small(ui, "clear", "Write 0: the column runs no script. The script itself is not touched.") {
        out = Some("0".to_string());
    }
    out
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
/// `spell_list_id` names, one line per slot, and the buttons that give the
/// creature another list.
fn spells_window(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    if !subject.behaviour.spells_open {
        return None;
    }
    let about = subject.behaviour.about.clone()?;
    let mut keep_open = true;
    let shown = egui::Window::new(format!("{} \u{2014} spells", about.label))
        .id(egui::Id::new("spells-window"))
        .open(&mut keep_open)
        .default_size([740.0, 520.0])
        .default_pos([320.0, 140.0])
        .resizable(true)
        .frame(frame())
        .show(ctx, |ui| {
            page_spacing(ui);
            if let Some(trouble) = subject.behaviour.trouble.clone() {
                ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
                return;
            }
            if about.spell_list_id == 0 {
                theme::note(ui, "spell_list_id is 0: the creature casts nothing in combat.");
                list_toolbar(ui, subject, &about.template_key, &about.label, None);
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
                ui.horizontal(|ui| {
                    if ui
                        .small_button(format!("+ create list {}", about.spell_list_id))
                        .on_hover_text("A creature_spells row at that entry, with every slot empty.")
                        .clicked()
                    {
                        let now = subject.now;
                        subject.behaviour.create_list(subject.session, about.spell_list_id, &about.label, now);
                    }
                });
                list_toolbar(ui, subject, &about.template_key, &about.label, None);
                return;
            };
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("creature_spells {} \u{b7} {}", list.list.entry, list.list.name))
                        .strong()
                        .size(13.5),
                );
                ui.label(
                    egui::RichText::new(format!("creature_template {}'s spell_list_id", about.entry))
                        .small()
                        .color(theme::INK_FAINT),
                );
            });
            match (list.life, subject.behaviour.users_of(list.list.entry)) {
                (Life::Insert, _) => {
                    ui.label(egui::RichText::new("New: this list is in no database yet. Apply writes it.").small().color(theme::WARN));
                }
                (_, Some(users)) if users > 1 => {
                    ui.label(
                        egui::RichText::new(format!(
                            "{users} creatures name this list, and an edit changes every one of them. \
                             Copy into a new list to change this creature alone."
                        ))
                        .small()
                        .color(theme::WARN),
                    );
                }
                (_, Some(_)) => {
                    ui.label(egui::RichText::new("Only this creature names the list.").small().color(theme::INK_DIM));
                }
                (_, None) => {}
            }
            list_toolbar(ui, subject, &about.template_key, &about.label, Some(&list.list));
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .show(ui, |ui| list_fields(ui, subject, &list));
        });
    if !keep_open {
        subject.behaviour.spells_open = false;
    }
    shown.map(|shown| shown.response.rect)
}

/// The buttons that give the creature a list: an existing one, a copy of
/// the one it has, or a new empty one. Each points `spell_list_id` at it.
fn list_toolbar(ui: &mut egui::Ui, subject: &mut Subject<'_>, template_key: &Key, label: &str, has: Option<&List>) {
    let next = subject.behaviour.next_list_entry(subject.session);
    ui.horizontal(|ui| {
        if ui
            .button("Use another list\u{2026}")
            .on_hover_text("Search creature_spells by name or entry and point spell_list_id at the list chosen.")
            .clicked()
        {
            subject.behaviour.chooser = Some(Chooser::List(Search::default()));
        }
        if let Some(list) = has {
            let copy = ui
                .add_enabled(next.is_some(), egui::Button::new("Copy into a new list"))
                .on_hover_text(format!(
                    "A new creature_spells row at entry {} with this list's eight slots, and \
                     spell_list_id pointed at it, so an edit changes this creature alone.",
                    next.map(|n| n.to_string()).unwrap_or_default()
                ))
                .on_disabled_hover_text("Reading the table's highest entry.");
            if copy.clicked() {
                if let Some(entry) = next {
                    let now = subject.now;
                    let copy = list.copied_as(entry, label);
                    subject.behaviour.create_list_as(subject.session, &copy, "Copy spell list", now);
                    set_template(subject, template_key, "spell_list_id", entry.to_string());
                }
            }
        }
        let empty = ui
            .add_enabled(next.is_some(), egui::Button::new("+ New empty list"))
            .on_hover_text(format!(
                "A creature_spells row at entry {}, with every slot empty, and spell_list_id \
                 pointed at it.",
                next.map(|n| n.to_string()).unwrap_or_default()
            ))
            .on_disabled_hover_text("Reading the table's highest entry.");
        if empty.clicked() {
            if let Some(entry) = next {
                let now = subject.now;
                subject.behaviour.create_list(subject.session, entry, label, now);
                set_template(subject, template_key, "spell_list_id", entry.to_string());
            }
        }
    });
    ui.add_space(4.0);
}

/// The list's name and its eight slots, one line each.
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
    let written = page_row(ui, "name", "for the person; the server never reads it", edited("name"), |ui| {
        text_cell(ui, id.with("name"), &vale_mangos::sql::text(&list.name))
    });
    if let Some(written) = written {
        let now = subject.now;
        subject.behaviour.set_list(subject.session, shown, "name", written, now);
    }
    let sentences: Vec<String> = {
        let namer = namer(subject.session, subject.behaviour, subject.quests);
        list.slots.iter().map(|slot| namer.slot(slot)).collect()
    };
    for slot in 0..SLOTS {
        let columns = creaturespells::slot_columns(slot);
        let held = list.slots[slot];
        let touched = columns.iter().any(|column| edited(column.name));
        let open = subject.behaviour.unfolded_slots.contains(&slot);
        let stroke = match touched {
            true => theme::WARN,
            false => theme::LINE,
        };
        card(stroke).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let colour = match held.is_empty() {
                true => theme::INK_FAINT,
                false => theme::INK,
            };
            let (_, fold) = header(
                ui,
                |ui| {
                    ui.label(egui::RichText::new(format!("{}", slot + 1)).strong().color(theme::INK_FAINT));
                    ui.label(egui::RichText::new(&sentences[slot]).color(colour));
                },
                |ui| {
                    let fold = fold_button(ui, open);
                    if !held.is_empty() {
                        if held.script != 0 {
                            ui.label(egui::RichText::new(format!("script {}", held.script)).small().color(theme::INK_DIM));
                        }
                        ui.label(egui::RichText::new(format!("{}%", held.probability)).small().color(theme::INK_DIM));
                    }
                    fold
                },
            );
            if fold {
                match open {
                    true => subject.behaviour.unfolded_slots.remove(&slot),
                    false => subject.behaviour.unfolded_slots.insert(slot),
                };
            }
            if open {
                ui.separator();
                slot_fields(ui, subject, shown, slot, &edited, id);
            }
        });
        ui.add_space(3.0);
    }
}

/// The eleven columns of one slot.
fn slot_fields(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &ShownList, slot: usize, edited: &dyn Fn(&str) -> bool, id: egui::Id) {
    let list = &shown.list;
    let key = list.key();
    let held = list.slots[slot];
    let columns = creaturespells::slot_columns(slot);
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
        let written = page_row(ui, name, about, edited(column.name), |ui| match field {
            10 => {
                let written = number_cell(ui, Kind::Unsigned, &showing);
                let script: u32 = showing.trim().parse().unwrap_or(0);
                script_buttons(ui, subject, scripts::CREATURE_SPELLS, script, None, ScriptAnswer::Column(target)).or(written)
            }
            _ => value_cell(ui, subject, kind, &showing, id.with(column.name), Some(target), column.name),
        });
        if let Some(written) = written {
            let now = subject.now;
            subject.behaviour.set_list(subject.session, shown, column.name, written, now);
        }
    }
}

// ---------------------------------------------------------------------------
// Scripts
// ---------------------------------------------------------------------------

/// The script window: the steps of one script in the order the server runs
/// them, the wait before each, and the buttons that add, move and remove
/// steps.
fn script_window(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    if !subject.behaviour.script_open {
        return None;
    }
    let (table, id) = subject.behaviour.script?;
    let mut keep_open = true;
    let shown = egui::Window::new(format!("{table} {id} \u{2014} script"))
        .id(egui::Id::new("script-window"))
        .open(&mut keep_open)
        .default_size([760.0, 600.0])
        .default_pos([340.0, 150.0])
        .resizable(true)
        .frame(frame())
        .show(ctx, |ui| {
            page_spacing(ui);
            let table_info = scripts::table(table);
            ui.label(
                egui::RichText::new(format!(
                    "Named by {}. The steps run in this order, each after the wait before it.",
                    table_info.map(|t| t.keyed_by).unwrap_or("")
                ))
                .small()
                .color(theme::INK_DIM),
            );
            match table_info.and_then(|t| t.reload) {
                Some(reload) => theme::note(ui, format!("Live on `.reload {reload}`, which an apply sends.")),
                None => theme::note(ui, "Read at server start: an applied change needs a restart."),
            }
            if !scripts::waits(table) {
                theme::note(
                    ui,
                    "An event runs every step of a creature_ai_scripts script at once; the \
                     server does not honour a delay here. For steps with waits between them, \
                     add a Start script step and put the steps in the generic script it \
                     starts, where the waits are seconds.",
                );
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
                    ui.label(egui::RichText::new("Removed: Apply deletes every step under the id.").small().color(theme::BAD));
                }
                (true, false, _) => {
                    ui.label(egui::RichText::new("New: this script is in no database yet. Apply writes it.").small().color(theme::WARN));
                }
                (true, true, false) => {
                    ui.label(egui::RichText::new("Changed: Apply replaces every step under the id.").small().color(theme::WARN));
                }
                _ => {}
            }
            ui.horizontal(|ui| {
                step_menu(ui, subject, &script, script.rows.len(), "+ Step");
                if claimed
                    && ui
                        .small_button("discard changes")
                        .on_hover_text("Give up this project's claim on the script; the database's steps stand.")
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
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("fold all").clicked() {
                        subject.behaviour.unfolded_steps.clear();
                    }
                    if ui.small_button("unfold all").clicked() {
                        subject.behaviour.unfolded_steps.extend(0..script.rows.len());
                    }
                    ui.label(egui::RichText::new(format!("{} step(s)", script.rows.len())).small().color(theme::INK_FAINT));
                });
            });
            ui.add_space(4.0);
            let sentences: Vec<String> = {
                let namer = namer(subject.session, subject.behaviour, subject.quests);
                script.sorted().iter().map(|row| namer.step(row)).collect()
            };
            egui::ScrollArea::vertical()
                .max_height(ui.available_height().max(60.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("START").size(10.5).strong().color(theme::INK_FAINT));
                    if script.rows.is_empty() {
                        theme::note(ui, "No steps. A script with no steps does nothing; applied, it deletes the steps the database holds.");
                    }
                    let sorted = script.sorted();
                    for (at, row) in sorted.iter().enumerate() {
                        wait_line(ui, subject, &script, at);
                        step_card(ui, subject, &script, at, row, &sentences[at]);
                    }
                    if !script.rows.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(18.0);
                            step_menu(ui, subject, &script, script.rows.len(), "+ then\u{2026}");
                        });
                        ui.label(egui::RichText::new("END").size(10.5).strong().color(theme::INK_FAINT));
                    }
                });
        });
    if !keep_open {
        subject.behaviour.script_open = false;
    }
    shown.map(|shown| shown.response.rect)
}

/// The line before step `at`: the wait after the step before it, which can
/// be dragged, and the button that puts a step in here.
fn wait_line(ui: &mut egui::Ui, subject: &mut Subject<'_>, script: &Script, at: usize) {
    let wait = script.wait_before(at);
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        let word = match (at, wait) {
            (0, 0) => "at once",
            (0, _) => "after",
            (_, 0) => "then, at once",
            _ => "wait",
        };
        ui.label(egui::RichText::new(word).small().color(theme::INK_DIM));
        if scripts::waits(script.table) {
            let mut value = wait;
            let response = ui
                .add(egui::DragValue::new(&mut value).speed(0.1).range(0..=86_400).suffix(" s"))
                .on_hover_text(
                    "The wait before this step, in seconds. Changing it moves this step and \
                     every step after it, so the later waits stay as they are.",
                );
            if response.changed() && value != wait {
                let next = script.with_wait(at, value);
                let now = subject.now;
                subject.behaviour.set_script(subject.session, &next, now);
            }
            if wait >= 60 {
                meaning(ui, vale_mangos::schema::span_words(u64::from(wait) * 1000));
            }
        } else if wait > 0 {
            ui.label(
                egui::RichText::new(format!("delay {wait} s, which the server does not honour here"))
                    .small()
                    .color(theme::BAD),
            );
        }
        ui.add_space(8.0);
        step_menu(ui, subject, script, at, "+ here");
    });
}

/// A menu of commands that puts a new step in at `at`: the common commands,
/// then every command. The step runs straight after the step before it.
fn step_menu(ui: &mut egui::Ui, subject: &mut Subject<'_>, script: &Script, at: usize, label: &str) {
    let mut chosen: Option<u32> = None;
    ui.menu_button(label, |ui| {
        ui.set_min_width(220.0);
        for value in COMMON_COMMANDS {
            let Some(command) = scripts::command(value) else { continue };
            if ui.button(command.name).clicked() {
                chosen = Some(value);
                ui.close();
            }
        }
        ui.separator();
        ui.menu_button("Every command", |ui| {
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for command in scripts::COMMANDS.iter() {
                    if ui.button(format!("{} \u{b7} {}", command.command, command.name)).clicked() {
                        chosen = Some(command.command);
                        ui.close();
                    }
                }
            });
        });
    })
    .response
    .on_hover_text("Put a step in here, run straight after the step before it.");
    if let Some(command) = chosen {
        let next = script.with_step_inserted(at, ScriptRow::new(command));
        let now = subject.now;
        subject.behaviour.set_script(subject.session, &next, now);
        let unfolded = &mut subject.behaviour.unfolded_steps;
        *unfolded = unfolded.iter().map(|&n| if n >= at { n + 1 } else { n }).collect();
        unfolded.insert(at);
    }
}

/// One step: its number, its sentence, the buttons that move, copy and
/// remove it, and its columns when unfolded.
fn step_card(ui: &mut egui::Ui, subject: &mut Subject<'_>, script: &Script, at: usize, row: &ScriptRow, sentence: &str) {
    let open = subject.behaviour.unfolded_steps.contains(&at);
    let count = script.rows.len();
    let mut change: Option<(Script, Box<dyn Fn(usize) -> Option<usize>>)> = None;
    card(theme::LINE).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (_, fold) = header(
            ui,
            |ui| {
                ui.label(egui::RichText::new(format!("{}", at + 1)).strong().color(theme::INK_FAINT));
                ui.label(egui::RichText::new(sentence).size(13.0).color(theme::INK));
            },
            |ui| {
                let fold = fold_button(ui, open);
                if small(ui, "remove", "Remove this step.") {
                    change = Some((
                        script.with_step_removed(at),
                        Box::new(move |n| match n {
                            n if n == at => None,
                            n if n > at => Some(n - 1),
                            n => Some(n),
                        }),
                    ));
                }
                if small(ui, "copy", "A copy of this step straight after it.") {
                    change = Some((script.with_step_copied(at), Box::new(move |n| Some(if n > at { n + 1 } else { n }))));
                }
                if ui.add_enabled(at + 1 < count, egui::Button::new("down").small()).on_hover_text("Swap with the step after; the waits stay where they are.").clicked() {
                    change = Some((script.with_step_moved(at, false), Box::new(move |n| Some(swap(n, at, at + 1)))));
                }
                if ui.add_enabled(at > 0, egui::Button::new("up").small()).on_hover_text("Swap with the step before; the waits stay where they are.").clicked() {
                    change = Some((script.with_step_moved(at, true), Box::new(move |n| Some(swap(n, at, at - 1)))));
                }
                fold
            },
        );
        if fold {
            match open {
                true => subject.behaviour.unfolded_steps.remove(&at),
                false => subject.behaviour.unfolded_steps.insert(at),
            };
        }
        if !row.comments.is_empty() {
            ui.label(egui::RichText::new(&row.comments).small().italics().color(theme::INK_FAINT));
        }
        if open {
            ui.separator();
            step_fields(ui, subject, script, at, row);
        }
    });
    if let Some((next, renumber)) = change {
        let now = subject.now;
        subject.behaviour.set_script(subject.session, &next, now);
        let unfolded: HashSet<usize> = subject.behaviour.unfolded_steps.iter().filter_map(|&n| renumber(n)).collect();
        subject.behaviour.unfolded_steps = unfolded;
    }
}

/// `n` with `a` and `b` exchanged.
fn swap(n: usize, a: usize, b: usize) -> usize {
    match n {
        n if n == a => b,
        n if n == b => a,
        n => n,
    }
}

/// The columns of one step, named by its command and its target, and for a
/// Talk step the texts it says.
fn step_fields(ui: &mut egui::Ui, subject: &mut Subject<'_>, script: &Script, at: usize, row: &ScriptRow) {
    let command = scripts::command(row.command);
    let target = scripts::target(row.target_type);
    let id = ui.make_persistent_id(("script-row", script.table, script.id, at));
    let write = |subject: &mut Subject<'_>, column: &'static str, value: &str| {
        let mut changed = row.clone();
        if changed.set(column, value) {
            let next = script.with_step(at, changed);
            let now = subject.now;
            subject.behaviour.set_script(subject.session, &next, now);
        }
    };
    let number = |ui: &mut egui::Ui, kind: Kind, showing: &str| number_cell(ui, kind, showing);

    if let Some(written) = page_row(ui, "command", "what the step does", false, |ui| {
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
    if let Some(written) = page_row(ui, "condition_id", "a row of conditions the step is skipped without, or 0", false, |ui| number(ui, Kind::Unsigned, &row.get("condition_id"))) {
        write(subject, "condition_id", &written);
    }
    if let Some(written) = page_row(ui, "delay", "seconds after the script starts; the wait line above sets it relative to the step before", false, |ui| {
        let written = number(ui, Kind::Unsigned, &row.get("delay"));
        if let Some(means) = number_means(Kind::Seconds, &row.get("delay")) {
            meaning(ui, means);
        }
        written
    }) {
        write(subject, "delay", &written);
    }
    if let Some(written) = page_row(ui, "priority", "order among steps with one delay", false, |ui| number(ui, Kind::Unsigned, &row.get("priority"))) {
        write(subject, "priority", &written);
    }
    if let Some(written) = page_row(ui, "comments", "for the person; the server never reads it", false, |ui| {
        text_cell(ui, id.with("comments"), &vale_mangos::sql::text(&row.comments))
    }) {
        write(subject, "comments", &unquote(&written));
    }
    for entry in scripts::texts_of(row) {
        text_form(ui, subject, entry);
    }
}

/// The value cell of one script data column: as [`value_cell`], with the
/// picker answering through [`PickFor::ScriptCell`] since a script is not in
/// the row store, a script id offering the script buttons, and a text id
/// offering the text buttons.
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
    let cell = ScriptAnswer::Cell { table: script.table, id: script.id, row: at, column };
    match kind {
        Kind::Ref(broadcast::TABLE) => {
            let written = number_cell(ui, Kind::Unsigned, showing);
            text_buttons(ui, subject, showing, cell, script, at).or(written)
        }
        Kind::Ref(table) if scripts::table_named(table).is_some() => {
            let written = number_cell(ui, Kind::Unsigned, showing);
            let script_id: u32 = showing.trim().parse().unwrap_or(0);
            let table = scripts::table_named(table).unwrap_or(scripts::GENERIC);
            script_buttons(ui, subject, table, script_id, None, cell).or(written)
        }
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

/// The buttons after a Talk step's text id: what it says, choose an existing
/// text, or write a new one. `Some` with the entry to write when a new text
/// is made or the cell is cleared.
fn text_buttons(ui: &mut egui::Ui, subject: &mut Subject<'_>, showing: &str, cell: ScriptAnswer, script: &Script, at: usize) -> Option<String> {
    let entry: u32 = showing.trim().parse().unwrap_or(0);
    let mut out = None;
    match entry {
        0 => meaning(ui, "none"),
        entry => match subject.behaviour.said(entry, &subject.session.server_edits) {
            Some(said) => {
                // A truncated label is given its width: in a horizontal row
                // it otherwise asks for the whole text's width, and the
                // window grows to fit it.
                let width = (ui.available_width() - 150.0).max(80.0);
                ui.add_sized(
                    egui::vec2(width, FORM_ROW),
                    egui::Label::new(egui::RichText::new(format!("\u{201c}{said}\u{201d}")).color(theme::INK)).truncate(),
                );
            }
            None if subject.behaviour.text_read(entry) => {
                ui.label(egui::RichText::new("no such text").small().color(theme::BAD));
            }
            None => meaning(ui, "\u{2026}"),
        },
    }
    if ui
        .small_button("choose\u{2026}")
        .on_hover_text("Search broadcast_text by what is said or by entry, and use an existing text.")
        .clicked()
    {
        subject.behaviour.chooser = Some(Chooser::Text { answer: cell, search: Search::default() });
    }
    let next = subject.behaviour.next_text_entry(subject.session);
    let chat_type = script.sorted().get(at).map(|row| row.datalong[0]).unwrap_or(0);
    let button = ui
        .add_enabled(next.is_some(), egui::Button::new(egui::RichText::new("+ new").size(12.0)).small())
        .on_hover_text(format!(
            "A new broadcast_text row at entry {}, to be written in the form under this step.",
            next.map(|n| n.to_string()).unwrap_or_default()
        ))
        .on_disabled_hover_text("Reading broadcast_text's highest entry.");
    if button.clicked() {
        let now = subject.now;
        if let Some(entry) = subject.behaviour.create_text(subject.session, "", chat_type, now) {
            out = Some(entry.to_string());
        }
    }
    out
}

/// One `broadcast_text` row as a form of its own, under the Talk step that
/// says it: what is said, how, in what language, and the sound and emotes
/// that go with it.
fn text_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, entry: u32) {
    let Some(shown) = subject.behaviour.text_of(entry, &subject.session.server_edits) else {
        if subject.behaviour.text_read(entry) {
            ui.label(
                egui::RichText::new(format!("broadcast_text has no row {entry}; the step says nothing."))
                    .small()
                    .color(theme::BAD),
            );
        }
        return;
    };
    let key = shown.text.key();
    let changed: Vec<&'static str> = broadcast::COLUMNS
        .iter()
        .map(|column| column.name)
        .filter(|column| subject.session.server_edits.get(broadcast::TABLE, &key, column).is_some())
        .collect();
    let stroke = match shown.life {
        Life::Insert => theme::WARN,
        Life::Delete => theme::BAD,
        Life::Update if !changed.is_empty() => theme::WARN,
        Life::Update => theme::LINE,
    };
    let id = ui.make_persistent_id(("broadcast-text", entry));
    card(stroke).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("broadcast_text {entry}")).strong().size(12.5));
            let note = match shown.life {
                Life::Insert => "new: Apply writes it",
                Life::Delete => "marked for removal",
                Life::Update => "read at server start; an applied change needs a restart",
            };
            ui.label(egui::RichText::new(note).small().color(theme::INK_FAINT));
        });
        text_fields(ui, subject, &shown, &changed, id);
    });
}

/// The eleven editable columns of one text.
fn text_fields(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &ShownText, changed: &[&'static str], id: egui::Id) {
    let key = shown.text.key();
    for column in broadcast::COLUMNS.iter().skip(1) {
        let showing = shown.text.get(column.name);
        let edited = changed.contains(&column.name);
        let written = page_row(ui, column.name, column.about, edited, |ui| match column.kind {
            Kind::Text => text_cell(ui, id.with(column.name), &vale_mangos::sql::text(&showing)).map(|literal| unquote(&literal)),
            kind => {
                let target = ColumnTarget {
                    table: broadcast::TABLE,
                    key: key.clone(),
                    column: column.name,
                    in_database: shown.in_database.as_ref().map(|had| had.get(column.name)),
                    label: "Edit text",
                    subject: format!("{} {} {}", broadcast::TABLE, key.text(), column.name),
                };
                value_cell(ui, subject, kind, &showing, id.with(column.name), Some(target), column.name)
            }
        });
        if let Some(written) = written {
            let now = subject.now;
            subject.behaviour.set_text(subject.session, shown, column.name, written, now);
        }
    }
}

// ---------------------------------------------------------------------------
// Choosers
// ---------------------------------------------------------------------------

/// The open chooser, as a modal dialog over everything.
fn chooser(ctx: &egui::Context, subject: &mut Subject<'_>) {
    let Some(mut open) = subject.behaviour.chooser.take() else {
        return;
    };
    let mut close = false;
    let response = egui::Modal::new(egui::Id::new("behaviour-chooser")).show(ctx, |ui| {
        ui.set_width(620.0);
        page_spacing(ui);
        match &mut open {
            Chooser::Events(search) => close = events_chooser(ui, subject, search),
            Chooser::Script { table, answer, search } => close = script_chooser(ui, subject, table, answer, search),
            Chooser::List(search) => close = list_chooser(ui, subject, search),
            Chooser::Text { answer, search } => close = text_chooser(ui, subject, answer, search),
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Close").clicked() {
                close = true;
            }
            ui.label(egui::RichText::new("Esc closes").small().color(theme::INK_FAINT));
        });
    });
    if response.should_close() {
        close = true;
    }
    // A choice made in the dialog may have opened another chooser, which is
    // kept rather than the one drawn.
    if !close && subject.behaviour.chooser.is_none() {
        subject.behaviour.chooser = Some(open);
    }
}

/// The search box and the line under it. The box takes focus when the
/// chooser opens.
fn search_box<T>(ui: &mut egui::Ui, search: &mut Search<T>, hint: &str) {
    let field = ui.add(
        egui::TextEdit::singleline(&mut search.typed)
            .hint_text(hint)
            .desired_width(f32::INFINITY),
    );
    if search.sent.is_none() && search.found.is_none() {
        field.request_focus();
    }
    if let Some(trouble) = &search.trouble {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
    } else if search.searching() {
        theme::waiting(ui, "searching\u{2026}");
    } else if let Some((text, hits)) = &search.found {
        let line = match hits.len() {
            0 => format!("nothing matches \u{201c}{text}\u{201d}"),
            n if n >= crate::tools::behaviour::SEARCH_LIMIT => format!("the first {n}; type more to narrow it"),
            n => format!("{n} match(es)"),
        };
        ui.label(egui::RichText::new(line).small().color(theme::INK_FAINT));
    } else {
        ui.label(egui::RichText::new("Type three characters or a number.").small().color(theme::INK_FAINT));
    }
}

/// Add an existing event: search every creature's events and copy one to
/// this creature. Answers whether to close.
fn events_chooser(ui: &mut egui::Ui, subject: &mut Subject<'_>, search: &mut Search<Event>) -> bool {
    // The creature is the selection, which a scripted run makes a few frames
    // after the chooser opens; the chooser waits for it.
    let Some(about) = subject.behaviour.about.clone() else {
        theme::waiting(ui, "waiting for a creature to be selected\u{2026}");
        return false;
    };
    ui.label(egui::RichText::new(format!("Add an existing event to {}", about.label)).strong().size(14.0));
    theme::note(
        ui,
        "Searches creature_ai_events by comment, id and creature id. The reference database \
         starts a comment with the creature's name, so a creature's name finds its events and \
         \u{201c}Flee\u{201d} finds every creature that flees.",
    );
    search_box(ui, search, "Flee at 15% HP, Shadow Bolt, Hogger, 3160\u{2026}");
    let hits: Vec<Event> = search.hits().to_vec();
    let cards: Vec<(Event, String, Vec<String>, Vec<Option<Vec<(u32, String)>>>)> = {
        let namer = namer(subject.session, subject.behaviour, subject.quests);
        hits.iter()
            .map(|event| {
                let steps = event
                    .scripts
                    .iter()
                    .filter(|id| **id != 0)
                    .map(|id| steps_of(&namer, scripts::CREATURE_AI, *id))
                    .collect();
                (event.clone(), namer.event(event), event.terms(), steps)
            })
            .collect()
    };
    let mut status: Option<String> = None;
    egui::ScrollArea::vertical().max_height(440.0).auto_shrink([false, true]).show(ui, |ui| {
        for (event, trigger, terms, steps) in &cards {
            card(theme::LINE).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let comment = match event.comment.is_empty() {
                        true => format!("event {}", event.id),
                        false => event.comment.clone(),
                    };
                    ui.add(egui::Label::new(egui::RichText::new(comment).strong()).wrap());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("#{} \u{b7} creature {}", event.id, event.creature_id))
                                .small()
                                .color(theme::INK_FAINT),
                        );
                    });
                });
                ui.horizontal(|ui| {
                    badge(ui, "WHEN", theme::ACCENT);
                    ui.add(egui::Label::new(egui::RichText::new(trigger)).wrap());
                    if !terms.is_empty() {
                        ui.label(egui::RichText::new(terms.join(" \u{b7} ")).small().color(theme::INK_DIM));
                    }
                });
                ui.horizontal_top(|ui| {
                    badge(ui, if event.random_action() { "ONE OF" } else { "THEN" }, theme::GOOD);
                    ui.vertical(|ui| {
                        if steps.is_empty() {
                            ui.label(egui::RichText::new("nothing").color(theme::INK_FAINT));
                        }
                        for script in steps {
                            match script {
                                None => {
                                    ui.label(egui::RichText::new("reading\u{2026}").small().color(theme::INK_FAINT));
                                }
                                Some(steps) => {
                                    for (delay, sentence) in steps.iter().take(STEPS_ON_A_CARD) {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(step_time(*delay)).small().color(theme::INK_FAINT));
                                            ui.add(egui::Label::new(egui::RichText::new(sentence)).wrap());
                                        });
                                    }
                                }
                            }
                        }
                    });
                });
                ui.horizontal(|ui| {
                    let ready = steps.iter().all(Option::is_some);
                    let copy = ui
                        .add_enabled(ready, egui::Button::new("Add a copy"))
                        .on_hover_text(
                            "A new event of this creature with the same trigger, and a copy of each \
                             script under a new id, so an edit to it changes nothing else.",
                        )
                        .on_disabled_hover_text("Reading its scripts.");
                    let share = ui.button("Add, sharing its scripts").on_hover_text(
                        "A new event of this creature with the same trigger, naming the same \
                         scripts. An edit to one of them changes every event that runs it.",
                    );
                    let how = match (copy.clicked(), share.clicked()) {
                        (true, _) => Some(CopyScripts::Copy),
                        (_, true) => Some(CopyScripts::Share),
                        _ => None,
                    };
                    if let Some(how) = how {
                        let now = subject.now;
                        status = Some(
                            match subject.behaviour.copy_event(subject.session, about.entry, &about.label, event, how, now) {
                                Ok(id) => format!("added as event {id}"),
                                Err(e) => e,
                            },
                        );
                    }
                });
            });
            ui.add_space(4.0);
        }
    });
    if let Some(status) = status {
        subject.session.status = status;
    }
    false
}

/// Choose an existing script for one column. The creature's own events'
/// scripts are offered first. Answers whether to close.
fn script_chooser(ui: &mut egui::Ui, subject: &mut Subject<'_>, table: &'static str, answer: &ScriptAnswer, search: &mut Search<u32>) -> bool {
    let into = match answer {
        ScriptAnswer::Column(target) => format!("{} {} {}", target.table, target.key.text(), target.column),
        ScriptAnswer::Cell { table, id, row, column } => format!("{table} {id}, step {}, {column}", row + 1),
    };
    ui.label(egui::RichText::new(format!("Choose a {table} script")).strong().size(14.0));
    ui.label(egui::RichText::new(format!("for {into}")).small().color(theme::INK_DIM));
    search_box(ui, search, "a comment, or an id");
    let mut offered: Vec<u32> = Vec::new();
    if table == scripts::CREATURE_AI {
        if let Some(about) = subject.behaviour.about.clone() {
            for shown in subject.behaviour.events_of(about.entry, &subject.session.server_edits) {
                offered.extend(shown.event.scripts.iter().filter(|id| **id != 0));
            }
        }
    }
    offered.extend(search.hits().iter().copied());
    let mut seen = HashSet::new();
    offered.retain(|id| seen.insert(*id));
    let rows: Vec<(u32, String, Option<Vec<(u32, String)>>)> = {
        let namer = namer(subject.session, subject.behaviour, subject.quests);
        offered
            .iter()
            .map(|id| {
                let comment = namer
                    .behaviour
                    .script_of(table, *id, namer.session)
                    .and_then(|script| script.sorted().first().map(|row| row.comments.clone()))
                    .unwrap_or_default();
                (*id, comment, steps_of(&namer, table, *id))
            })
            .collect()
    };
    let mut chosen: Option<u32> = None;
    egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
        for (id, comment, steps) in &rows {
            card(theme::LINE).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("script {id}")).strong());
                    ui.add(egui::Label::new(egui::RichText::new(comment).small().color(theme::INK_DIM)).truncate());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Use this").clicked() {
                            chosen = Some(*id);
                        }
                    });
                });
                match steps {
                    None => {
                        ui.label(egui::RichText::new("reading\u{2026}").small().color(theme::INK_FAINT));
                    }
                    Some(steps) => {
                        for (delay, sentence) in steps.iter().take(STEPS_ON_A_CARD) {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(step_time(*delay)).small().color(theme::INK_FAINT));
                                ui.add(egui::Label::new(egui::RichText::new(sentence)).wrap());
                            });
                        }
                    }
                }
            });
            ui.add_space(3.0);
        }
    });
    match chosen {
        Some(id) => {
            answer_with(subject, answer, id);
            true
        }
        None => false,
    }
}

/// Write a chosen id where a chooser was opened for.
fn answer_with(subject: &mut Subject<'_>, answer: &ScriptAnswer, id: u32) {
    match answer {
        ScriptAnswer::Column(target) => target.write(subject.session, id.to_string(), subject.now),
        ScriptAnswer::Cell { table, id: script, row, column } => write_cell(subject, table, *script, *row, column, id),
    }
}

/// Choose an existing spell list for the creature. Answers whether to close.
fn list_chooser(ui: &mut egui::Ui, subject: &mut Subject<'_>, search: &mut Search<u32>) -> bool {
    let Some(about) = subject.behaviour.about.clone() else {
        theme::waiting(ui, "waiting for a creature to be selected\u{2026}");
        return false;
    };
    ui.label(egui::RichText::new(format!("Choose a spell list for {}", about.label)).strong().size(14.0));
    theme::note(
        ui,
        "Searches creature_spells by name and entry. The list chosen is shared with every \
         creature that names it; Copy into a new list, after, gives this creature its own.",
    );
    search_box(ui, search, "a name, or an entry");
    let lists: Vec<List> = search
        .hits()
        .iter()
        .filter_map(|entry| subject.behaviour.cached_list(*entry).cloned())
        .collect();
    let sentences: Vec<Vec<String>> = {
        let namer = namer(subject.session, subject.behaviour, subject.quests);
        lists
            .iter()
            .map(|list| list.slots.iter().filter(|slot| !slot.is_empty()).map(|slot| namer.slot(slot)).collect())
            .collect()
    };
    let mut chosen: Option<u32> = None;
    egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
        for (list, slots) in lists.iter().zip(&sentences) {
            card(theme::LINE).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{} \u{b7} {}", list.entry, list.name)).strong());
                    ui.label(egui::RichText::new(format!("{} spell(s)", list.filled())).small().color(theme::INK_DIM));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Use this list").clicked() {
                            chosen = Some(list.entry);
                        }
                    });
                });
                for sentence in slots {
                    ui.label(egui::RichText::new(sentence).small());
                }
            });
            ui.add_space(3.0);
        }
    });
    match chosen {
        Some(entry) => {
            set_template(subject, &about.template_key, "spell_list_id", entry.to_string());
            true
        }
        None => false,
    }
}

/// Choose an existing text for a Talk step. Answers whether to close.
fn text_chooser(ui: &mut egui::Ui, subject: &mut Subject<'_>, answer: &ScriptAnswer, search: &mut Search<u32>) -> bool {
    ui.label(egui::RichText::new("Choose a text").strong().size(14.0));
    theme::note(
        ui,
        "Searches broadcast_text by what is said and by entry. A text can be said by any \
         number of steps; an edit to it changes every one of them.",
    );
    search_box(ui, search, "words it says, or an entry");
    let texts: Vec<ShownText> = search
        .hits()
        .iter()
        .filter_map(|entry| subject.behaviour.text_of(*entry, &subject.session.server_edits))
        .collect();
    let mut chosen: Option<u32> = None;
    egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
        for shown in &texts {
            let text = &shown.text;
            card(theme::LINE).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{}", text.entry)).strong().color(theme::INK_DIM));
                    ui.label(
                        egui::RichText::new(vale_mangos::schema::value_word(&scripts::CHAT_TYPES, text.chat_type))
                            .small()
                            .color(theme::INK_FAINT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Use this text").clicked() {
                            chosen = Some(text.entry);
                        }
                    });
                });
                ui.add(egui::Label::new(egui::RichText::new(format!("\u{201c}{}\u{201d}", text.said()))).wrap());
            });
            ui.add_space(3.0);
        }
    });
    match chosen {
        Some(entry) => {
            answer_with(subject, answer, entry);
            true
        }
        None => false,
    }
}
