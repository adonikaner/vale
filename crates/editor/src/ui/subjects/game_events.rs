//! The game events, drawn three ways: the Game Events section of the creature
//! and game object inspectors, the Game Event window it opens, and the Game
//! Events part of the Spawning workspace. The view of
//! [`crate::subjects::game_events`].
//!
//! ## The section
//!
//! One line per event the spawn answers, saying whether the event adds the
//! spawn or removes it, with Turn to change which. A creature's changes are
//! listed under them with their columns. Add to Event and Remove During
//! Event choose an event in the picker for the whole selection.
//!
//! ## One event, in the window and in the workspace
//!
//! [`event_view`] draws one `game_event` row through the form, with its
//! durations in days, hours and minutes and how many spawns, changes and
//! quests name it. In the workspace it also lists every spawn the event adds
//! or removes, of both kinds, by map, each named by where it stands
//! (`crate::subjects::located`) with Go To; every creature change; and every
//! quest it offers, which can be added and removed there. Show in World sets
//! the view bar's event filter to the event and goes to one of its spawns.

use super::keyed::{columns, keep_row, listed_row, not_ready, state_dot, Section, View};
use crate::subjects::game_events::{member_table, EventView, GameEvents, Phase};
use crate::subjects::keyed::Shown;
use crate::subjects::quests::{PickFor, Picker, SpawnInto, Target};
use crate::tools::spawns::spawn::{SpawnKind, SpawnRef};
use crate::ui::subjects::listing;
use crate::ui::widgets::rowform::{remove_keep, section, Hook, RowAct};
use crate::ui::widgets::theme;
use bevy_egui::egui;
use vale_mangos::game_event::{self, Change, Event, EventQuest, Member};

// ---------------------------------------------------------------------------
// The section
// ---------------------------------------------------------------------------

/// The Game Events section. Takes an event chosen in the picker for the
/// selection, and draws the window when one is open.
pub(in crate::ui) fn events_section(ui: &mut egui::Ui, sec: &mut Section<'_>, events: &mut GameEvents, patch: u32) {
    take_pick(sec, events);
    let edits = sec.session.rows.edits.clone();
    let primary = sec.chosen.primary.clone();
    let of = match sec.chosen.objects {
        true => SpawnKind::GameObject,
        false => SpawnKind::Creature,
    };
    let memberships = events.memberships(of, primary.guid, &edits);
    let changes = match of {
        SpawnKind::Creature => events.changes_of(primary.guid, &edits),
        SpawnKind::GameObject => Vec::new(),
    };
    let edited = memberships.iter().any(|shown| edits.touches(member_table(of), &shown.row.key()))
        || changes.iter().any(|shown| edits.touches(game_event::CREATURE_DATA, &shown.row.key()));
    let any = memberships.iter().any(Shown::kept) || changes.iter().any(Shown::kept);
    section(ui, ("spawn-game-events", primary.guid), "Game Events", edited, any, |ui| {
        theme::note(
            ui,
            format!(
                "{}: the events this spawn exists during, or is removed during. Read at \
                 startup; a change needs a restart.",
                member_table(of)
            ),
        );
        if not_ready(ui, events.trouble(), events.is_read(), "the game event tables") {
            return;
        }
        if memberships.is_empty() {
            theme::row(ui, "Events", |ui| {
                ui.label(egui::RichText::new("None: the spawn is always in the world").color(theme::INK_DIM));
            });
        }
        for shown in &memberships {
            membership(ui, sec, events, of, shown, patch);
        }
        let spawns = sec.chosen.all_guids();
        ui.horizontal_wrapped(|ui| {
            for (word, removed, about) in [
                ("Add to Event", false, "Choose a game event the selected spawns exist only during."),
                ("Remove During Event", true, "Choose a game event the selected spawns are removed during."),
            ] {
                if ui.button(format!("{word}\u{2026} ({})", spawns.len())).on_hover_text(about).clicked() {
                    sec.quests.picker = Some(Picker::new(
                        Target::List(game_event::EVENT),
                        PickFor::SpawnData {
                            into: SpawnInto::Event { objects: sec.chosen.objects, removed },
                            spawns: spawns.clone(),
                        },
                    ));
                }
            }
            if ui
                .button("New Event")
                .on_hover_text("Make a game event a script starts, with no dates, and open it. Set the dates and hardcoded to No to schedule it.")
                .clicked()
            {
                sec.session.status = match events.new_event(sec.session, &format!("New event for {}", primary.label), sec.now) {
                    Ok(entry) => {
                        events.open = Some(entry);
                        format!("game event {entry} made")
                    }
                    Err(e) => format!("no new event: {e}"),
                };
            }
        });
        if of == SpawnKind::Creature {
            theme::heading(ui, "Changes During Events");
            if changes.is_empty() {
                theme::note(ui, "game_event_creature_data: none.");
            }
            for shown in &changes {
                change(ui, &mut sec.view(), events, shown, false);
            }
            let open = memberships.iter().find(|shown| shown.kept() && !shown.row.removed()).map(|shown| shown.row.entry());
            if ui
                .add_enabled(open.is_some(), egui::Button::new("Add Change"))
                .on_hover_text("Change what this spawn is while its first event is on: its entry, display, equipment or a spell at each end. Starts changing nothing.")
                .on_disabled_hover_text("Add the spawn to an event first.")
                .clicked()
            {
                if let Some(entry) = open {
                    if let Err(e) = events.add_change(sec.session, primary.guid, entry, sec.now) {
                        sec.session.status = format!("not changed: {e}");
                    }
                }
            }
        }
    });
    window(ui.ctx(), &mut sec.view(), events, patch);
}

/// An event chosen in the picker for the selection, added on the frame after.
fn take_pick(sec: &mut Section<'_>, events: &mut GameEvents) {
    let Some((SpawnInto::Event { objects, removed }, spawns, entry)) = sec.quests.spawn_pick.clone() else {
        return;
    };
    sec.quests.spawn_pick = None;
    let of = match objects {
        true => SpawnKind::GameObject,
        false => SpawnKind::Creature,
    };
    sec.session.status = events.add(sec.session, of, &spawns, entry, removed, sec.now);
}

/// What an event is called in a line: its description, or its number when
/// `game_event` holds no row for it.
fn event_name(events: &GameEvents, entry: u32, edits: &vale_mangos::row::Edits) -> (String, Option<Event>) {
    match events.event(entry, edits).filter(Shown::kept) {
        Some(shown) => (format!("{entry} \u{b7} {}", shown.row.description), Some(shown.row)),
        None => (format!("{entry} \u{b7} no game_event row"), None),
    }
}

/// How much of an event's line its three buttons keep, in points.
const BUTTONS_ROOM: f32 = 170.0;

/// One event the spawn answers.
fn membership(ui: &mut egui::Ui, sec: &mut Section<'_>, events: &mut GameEvents, of: SpawnKind, shown: &Shown<Member>, patch: u32) {
    let edits = sec.session.rows.edits.clone();
    let member = shown.row;
    let (name, event) = event_name(events, member.entry(), &edits);
    let mut faults: Vec<String> = game_event::event_fault(i64::from(member.event)).into_iter().collect();
    match &event {
        None => faults.push(format!("game_event has no event {}; the server skips the row", member.entry())),
        Some(event) if !event.enabled_at(patch) => {
            faults.push(format!("the event is disabled at patch {patch}, so it never starts"))
        }
        _ => {}
    }
    // The buttons are laid from the right, so the event's name is cut to the
    // room they leave rather than pushing them off the row.
    ui.horizontal(|ui| {
        state_dot(ui, shown.life, &faults);
        let how = match member.removed() {
            true => "Removed during",
            false => "Added during",
        };
        ui.label(egui::RichText::new(how).small().color(theme::INK_DIM));
        let room = (ui.available_width() - BUTTONS_ROOM).max(60.0);
        ui.allocate_ui_with_layout(
            egui::vec2(room, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_max_width(room);
                ui.add(egui::Label::new(egui::RichText::new(name).small().color(theme::INK)).truncate());
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match remove_keep(ui, shown.life, "Takes the spawn out of the event: it is in the world whatever the event does.") {
                Some(RowAct::Keep) => keep_row(&mut sec.view(), member_table(of), &member.key()),
                Some(_) => events.remove(sec.session, of, member, sec.now),
                None => {}
            }
            if shown.kept()
                && ui
                    .small_button("Turn")
                    .on_hover_text("Turn between added during the event and removed during it.")
                    .clicked()
            {
                events.turn(sec.session, of, member, sec.now);
            }
            if ui.small_button("Open").on_hover_text("Open the event's window.").clicked() {
                events.open = Some(member.entry());
            }
        });
    });
}

/// One creature change, with its columns under it, and Go To on its spawn
/// in the workspace, where the change is listed by event.
fn change(ui: &mut egui::Ui, view: &mut View<'_>, events: &mut GameEvents, shown: &Shown<Change>, by_event: bool) {
    let edits = view.session.rows.edits.clone();
    let key = shown.row.key();
    let spawn = SpawnRef { kind: SpawnKind::Creature, guid: shown.row.guid };
    let (name, event) = event_name(events, shown.row.event, &edits);
    let mut faults: Vec<String> = game_event::event_fault(i64::from(shown.row.event)).into_iter().collect();
    if event.is_none() {
        faults.push(format!("game_event has no event {}; the server skips the row", shown.row.event));
    }
    let title = match by_event {
        true => format!("{} \u{b7} patch {}", view.spawn_name(spawn), shown.row.patch),
        false => format!("During {name} \u{b7} patch {}", shown.row.patch),
    };
    let act = listed_row(ui, ("event-change", key.text()), shown.life, &faults, title, "Removes the change.", |ui| {
        if shown.kept() {
            let held = shown.in_database.as_ref().map(Change::assignments);
            if let Some((column, value)) = columns(
                ui,
                view,
                game_event::CREATURE_DATA,
                &key,
                held.as_deref(),
                &game_event::CREATURE_DATA_COLUMNS,
                "Edit game event change",
                |_, _, _, _| None,
            ) {
                events.set_change_column(view.session, &key, column, value, view.now);
            }
        }
        if by_event {
            view.go_to_button(ui, spawn);
        }
    });
    match act {
        Some(RowAct::Keep) => keep_row(view, game_event::CREATURE_DATA, &key),
        Some(_) => events.remove_change(view.session, &key, view.now),
        None => {}
    }
}

/// The Game Event window, while an event is open.
fn window(ctx: &egui::Context, view: &mut View<'_>, events: &mut GameEvents, patch: u32) {
    let Some(entry) = events.open else { return };
    let title = title(events, entry, &view.session.rows.edits);
    let mut keep_open = true;
    theme::window(title, "game-event-window")
        .open(&mut keep_open)
        .default_size([560.0, 520.0])
        .default_pos([360.0, 140.0])
        .show(ctx, |ui| {
            if ui
                .button("Open in Spawning")
                .on_hover_text("Open the event in the Spawning workspace, with every spawn, change and quest it names.")
                .clicked()
            {
                view.asks.push(crate::ui::shell::asks::ShellAsk::Link(crate::tools::Tool::GameEvents));
            }
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                event_view(ui, view, events, entry, patch, false);
            });
        });
    if !keep_open {
        events.open = None;
    }
}

/// An event's title: its entry and description.
fn title(events: &GameEvents, entry: u32, edits: &vale_mangos::row::Edits) -> String {
    events
        .event(entry, edits)
        .map_or_else(|| format!("Game Event {entry}"), |shown| format!("Game Event {entry} \u{2014} {}", shown.row.description))
}

// ---------------------------------------------------------------------------
// One event
// ---------------------------------------------------------------------------

/// One event: its row through the form with its counts, and with `whole`,
/// every spawn, change and quest that names it.
pub(in crate::ui) fn event_view(ui: &mut egui::Ui, view: &mut View<'_>, events: &mut GameEvents, entry: u32, patch: u32, whole: bool) {
    let edits = view.session.rows.edits.clone();
    let Some(shown) = events.event(entry, &edits) else {
        ui.label(egui::RichText::new(format!("game_event has no event {entry}.")).color(theme::BAD));
        return;
    };
    let mut faults = shown.row.check();
    if !shown.row.enabled_at(patch) {
        faults.push(format!("disabled at patch {patch}: the server never starts it"));
    }
    for fault in &faults {
        ui.label(egui::RichText::new(fault).small().color(theme::WARN));
    }
    let counts = events.counts(entry, &edits);
    theme::row(ui, "Holds", |ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} creatures and {} game objects added, {} and {} removed, {} changes, {} quests",
                counts.creatures_added,
                counts.objects_added,
                counts.creatures_removed,
                counts.objects_removed,
                counts.changes,
                counts.quests
            ))
            .small(),
        );
    });
    if shown.kept() {
        let held = shown.in_database.as_ref().map(Event::assignments);
        let written = columns(
            ui,
            view,
            game_event::EVENT,
            &shown.row.key(),
            held.as_deref(),
            &game_event::EVENT_COLUMNS,
            "Edit game event",
            |hook, ui, field, showing| {
                if hook == Hook::After && matches!(field.column.name, "occurence" | "length") {
                    let minutes: u32 = showing.trim().parse().unwrap_or(0);
                    ui.label(egui::RichText::new(game_event::minutes_words(minutes)).small().color(theme::INK_DIM));
                }
                None
            },
        );
        if let Some((column, value)) = written {
            events.set_event_column(view.session, entry, column, value, view.now);
        }
    }
    if !whole {
        return;
    }
    spawns(ui, view, events, entry);
    let changes = events.changes_for(entry, &edits);
    theme::heading(ui, &format!("Creature Changes ({})", changes.iter().filter(|shown| shown.kept()).count()));
    for shown in &changes {
        change(ui, view, events, shown, true);
    }
    quests(ui, view, events, entry);
}

/// More spawns than this on one map are folded under its heading.
const FOLDED_PAST: usize = 40;

/// The spawns of one map in an event's list, with the table each is a row of.
type Members = Vec<(SpawnKind, Shown<Member>)>;

/// Every spawn the event adds or removes, of both kinds, by map.
fn spawns(ui: &mut egui::Ui, view: &mut View<'_>, events: &mut GameEvents, entry: u32) {
    let edits = view.session.rows.edits.clone();
    let members = events.members_of(entry, &edits);
    theme::heading(ui, &format!("Spawns ({})", members.iter().filter(|(_, shown)| shown.kept()).count()));
    // By map, the open map first; a spawn still being read or in no database
    // is listed last.
    let open = view.session.map_id;
    let mut by_map: std::collections::BTreeMap<(u8, u32), Members> = std::collections::BTreeMap::new();
    for (kind, shown) in members {
        let spawn = SpawnRef { kind, guid: shown.row.guid };
        let group = match view.located.find(spawn).flatten() {
            Some(place) if place.map == open => (0, place.map),
            Some(place) => (1, place.map),
            None => (2, 0),
        };
        by_map.entry(group).or_default().push((kind, shown));
    }
    for ((rank, map), rows) in by_map {
        let heading = match rank {
            2 => "Not in the database, or being read".to_string(),
            _ => {
                let name = view.session.maps.iter().find(|(id, _)| *id == map).map_or("", |(_, name)| name.as_str());
                format!("Map {map} \u{b7} {name} ({})", rows.len())
            }
        };
        egui::CollapsingHeader::new(egui::RichText::new(heading).small().strong())
            .id_salt(("event-spawns", entry, rank, map))
            .default_open(rows.len() <= FOLDED_PAST)
            .show(ui, |ui| {
                egui::Grid::new(("event-spawn-grid", entry, rank, map)).num_columns(5).spacing([8.0, 3.0]).striped(true).show(ui, |ui| {
                    for (kind, shown) in &rows {
                        spawn_line(ui, view, events, *kind, shown);
                        ui.end_row();
                    }
                });
            });
    }
}

/// One spawn of an event, as five grid cells: its state, its name, whether
/// the event adds or removes it, Go To and Turn, and the button that takes it
/// out.
fn spawn_line(ui: &mut egui::Ui, view: &mut View<'_>, events: &mut GameEvents, kind: SpawnKind, shown: &Shown<Member>) {
    let member = shown.row;
    let spawn = SpawnRef { kind, guid: member.guid };
    let faults: Vec<String> = game_event::event_fault(i64::from(member.event)).into_iter().collect();
    state_dot(ui, shown.life, &faults);
    view.spawn_cell(ui, spawn);
    let how = match member.removed() {
        true => "Removed during",
        false => "Added during",
    };
    ui.label(egui::RichText::new(how).small().color(theme::INK_DIM));
    ui.horizontal(|ui| {
        view.go_to_button(ui, spawn);
        if shown.kept()
            && ui
                .small_button("Turn")
                .on_hover_text("Turn between added during the event and removed during it.")
                .clicked()
        {
            events.turn(view.session, kind, member, view.now);
        }
    });
    match remove_keep(ui, shown.life, "Takes the spawn out of the event.") {
        Some(RowAct::Keep) => keep_row(view, member_table(kind), &member.key()),
        Some(_) => events.remove(view.session, kind, member, view.now),
        None => {}
    }
}

/// Every quest the event offers, and Add Quest.
fn quests(ui: &mut egui::Ui, view: &mut View<'_>, events: &mut GameEvents, entry: u32) {
    if let Some((event, quest)) = view.quests.event_quest_pick.take() {
        view.session.status = match events.add_quest(view.session, quest, event, view.now) {
            Ok(line) => line,
            Err(e) => format!("not changed: {e}"),
        };
    }
    let edits = view.session.rows.edits.clone();
    let quests = events.quests_of(entry, &edits);
    theme::heading(ui, &format!("Quests ({})", quests.iter().filter(|shown| shown.kept()).count()));
    for shown in &quests {
        let key = shown.row.key();
        let faults: Vec<String> = game_event::event_fault(i64::from(shown.row.event)).into_iter().collect();
        let quest = shown.row.quest;
        let title = format!("quest {quest} \u{b7} from patch {}", shown.row.patch_min);
        let act = listed_row(ui, ("event-quest", key.text()), shown.life, &faults, title, "Stops offering the quest only during the event.", |ui| {
            ui.horizontal(|ui| {
                let mut resolver = view.resolver();
                crate::ui::widgets::reference::name(ui, &mut resolver, vale_mangos::quest::TEMPLATE, quest);
            });
            if shown.kept() {
                let held = shown.in_database.as_ref().map(EventQuest::assignments);
                let written = columns(ui, view, game_event::QUEST, &key, held.as_deref(), &game_event::QUEST_COLUMNS, "Edit game event quest", |_, _, _, _| None);
                if let Some((column, value)) = written {
                    events.set_quest_column(view.session, &key, column, value, view.now);
                }
            }
        });
        match act {
            Some(RowAct::Keep) => keep_row(view, game_event::QUEST, &key),
            Some(_) => events.remove_quest(view.session, &key, view.now),
            None => {}
        }
    }
    if ui
        .button("Add Quest\u{2026}")
        .on_hover_text("Choose a quest the event offers only while it is on, by game_event_quest.")
        .clicked()
    {
        view.quests.picker = Some(Picker::new(Target::Quest, PickFor::EventQuest { event: entry }));
    }
}

// ---------------------------------------------------------------------------
// The workspace
// ---------------------------------------------------------------------------

/// What the Game Events part draws with.
struct Work<'a> {
    view: View<'a>,
    events: &'a mut GameEvents,
    filter: &'a mut EventView,
    patch: u32,
}

/// The Game Events part of the Spawning workspace: the list of events on the
/// left, one event on the right.
pub(crate) fn workspace(ui: &mut egui::Ui, _: crate::tools::Tool, bench: crate::ui::workspace::Bench<'_, '_, '_>) {
    let crate::ui::workspace::Bench { session, editing, playing, assets, now, .. } = bench;
    let patch = crate::server::settings::server_patch(&playing.server);
    let mut work = Work {
        view: View {
            session,
            quests: &mut editing.quests,
            asks: &mut editing.asks,
            assets,
            now,
            located: &mut editing.located,
        },
        events: &mut editing.game_events,
        filter: &mut editing.event_view,
        patch,
    };
    listing::frame(ui, &mut work, "game-events-list", list, form);
    super::quests::picker(
        ui.ctx(),
        work.view.session,
        work.view.quests,
        work.view.assets,
        &mut editing.thumbnails,
        None,
        work.view.now,
    );
}

/// The left column: the parts, the search, New Event, and the list.
fn list(ui: &mut egui::Ui, work: &mut Work<'_>) {
    if let Some(part) = crate::ui::shell::rail::parts(ui, crate::tools::Tool::GameEvents) {
        work.view.asks.push(crate::ui::shell::asks::ShellAsk::Link(part));
    }
    let edits = work.view.session.rows.edits.clone();
    let all = work.events.all(&edits);
    let trouble = work.events.trouble().cloned();
    if !listing::heading(ui, "Game Events", game_event::EVENT, all.len(), trouble.as_deref(), work.events.is_read()) {
        return;
    }
    listing::search_box(ui, &mut work.events.search, "Search by description or entry");
    if ui
        .button("+ New Event")
        .on_hover_text("Make a game event a script starts, with no dates. Set the dates and hardcoded to No to schedule it.")
        .clicked()
    {
        work.view.session.status = match work.events.new_event(work.view.session, "New event", work.view.now) {
            Ok(entry) => {
                work.events.open = Some(entry);
                format!("game event {entry} made")
            }
            Err(e) => format!("no new event: {e}"),
        };
    }
    ui.add_space(4.0);
    let search = work.events.search.trim().to_lowercase();
    let shown: Vec<&Shown<Event>> = all
        .iter()
        .filter(|shown| {
            search.is_empty() || shown.row.description.to_lowercase().contains(&search) || shown.row.entry.to_string() == search
        })
        .collect();
    listing::shown_count(ui, shown.len());
    let open = work.events.open;
    // One line of small text, not the two-line rows of the template lists:
    // the list draws only as many rows as fit at this height.
    let height = ui.spacing().interact_size.y;
    let mut clicked = None;
    theme::list_area(ui, None, height).show_rows(ui, height, shown.len(), |ui, range| {
        for at in range {
            let event = shown[at];
            let colour = match (event.in_database.is_none(), event.kept(), event.row.enabled_at(work.patch)) {
                (_, false, _) => theme::BAD,
                (true, _, _) => theme::WARN,
                (false, true, true) => theme::INK,
                (false, true, false) => theme::INK_FAINT,
            };
            let text = format!("{} \u{b7} {}", event.row.entry, event.row.description);
            let response = ui.add(
                egui::Button::selectable(open == Some(event.row.entry), egui::RichText::new(text).small().color(colour))
                    .truncate()
                    .min_size(egui::vec2(0.0, height)),
            );
            let response = match event.row.enabled_at(work.patch) {
                true => response,
                false => response.on_hover_text(format!("Not enabled at patch {}.", work.patch)),
            };
            if response.clicked() {
                clicked = Some(event.row.entry);
            }
        }
    });
    if let Some(entry) = clicked {
        work.events.open = Some(entry);
    }
}

/// The right side: the open event.
fn form(ui: &mut egui::Ui, work: &mut Work<'_>) {
    let Some(entry) = work.events.open else {
        theme::note(ui, "Open a game event on the left.");
        return;
    };
    let title = title(work.events, entry, &work.view.session.rows.edits);
    let edits = work.view.session.rows.edits.clone();
    let life = work.events.event(entry, &edits).map(|shown| shown.life);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).strong().size(15.0).color(theme::INK));
        if ui
            .button("Show in World")
            .on_hover_text(
                "Set the view bar's event filter to this event and go to one of its spawns: the \
                 spawn tools draw the world as it is while this event is on and no other.",
            )
            .clicked()
        {
            work.filter.phase = Phase::During(entry);
            show_in_world(&mut work.view, work.events, entry);
        }
        if let Some(life) = life {
            match remove_keep(ui, life, "Removes the event row. The rows naming it are left, and are marked.") {
                Some(RowAct::Keep) => keep_row(&mut work.view, game_event::EVENT, &game_event::event_key(entry)),
                Some(_) => work.events.remove_event(work.view.session, entry, work.view.now),
                None => {}
            }
        }
    });
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        event_view(ui, &mut work.view, work.events, entry, work.patch, true);
    });
}

/// Go to a spawn the event adds: one on the open map when there is one.
fn show_in_world(view: &mut View<'_>, events: &GameEvents, entry: u32) {
    let edits = view.session.rows.edits.clone();
    let open = view.session.map_id;
    let spawns: Vec<SpawnRef> = events
        .members_of(entry, &edits)
        .iter()
        .filter(|(_, shown)| shown.kept() && !shown.row.removed())
        .map(|(kind, shown)| SpawnRef { kind: *kind, guid: shown.row.guid })
        .collect();
    let here = spawns.iter().copied().find(|spawn| view.located.find(*spawn).flatten().is_some_and(|place| place.map == open));
    let any = spawns.iter().copied().find(|spawn| view.located.find(*spawn).flatten().is_some());
    match here.or(any) {
        Some(spawn) => {
            let ask = view.located.go_to(spawn);
            view.asks.push(ask);
        }
        None => view.session.status = format!("game event {entry} adds no spawn in the database to go to"),
    }
}

/// The inspector beside the Game Events part: the event filter and what the
/// colours in the world mean.
pub(crate) fn inspect(ui: &mut egui::Ui, _: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    theme::heading(ui, "Game Events");
    theme::note(
        ui,
        "In the creature and game object tools a spawn that exists only during an event is \
         marked violet. The view bar's event filter draws the world as it is during one event, \
         or outside every event. The game event tables are read at startup; a change needs a \
         restart.",
    );
    let phase = editing.event_view.phase;
    theme::row(ui, "Filter", |ui| {
        ui.label(egui::RichText::new(phase_words(phase)).color(theme::INK));
    });
    if phase != Phase::All && ui.button("Show Every Spawn").clicked() {
        editing.event_view.phase = Phase::All;
    }
}

/// What a filter is called.
pub(crate) fn phase_words(phase: Phase) -> String {
    match phase {
        Phase::All => "Every spawn".to_string(),
        Phase::Outside => "Outside events".to_string(),
        Phase::During(entry) => format!("During event {entry}"),
    }
}
