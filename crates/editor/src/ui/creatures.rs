//! The creature tool's panel: one spawn, the template behind it, and what an
//! edit to either means.
//!
//! ## The spawn is in the panel and the template is in a window
//!
//! A click in the viewport lands on a `creature` row and the `creature_template`
//! row behind it, and each says, before any field, how much of the world it
//! changes:
//!
//! ```text
//! This spawn      one creature, at one place
//! Its template    every spawn of this creature_template entry, everywhere
//! ```
//!
//! Apart from drawing the columns, the panel exists to state that scope. A
//! person editing `level_min` in a form that does not say which row it is on
//! cannot tell "make this one guard elite" from "make every Stormwind guard
//! elite".
//!
//! The two rows are drawn in two places on the same split: the columns of the
//! clicked spawn are in the sidebar, and the columns of its template are behind
//! a button. The sidebar carries the spawn's own 21 columns, which describe a
//! place in the world shown by the viewport beside them. Above them are two
//! folding sections and a row of buttons: Spawn (the selected spawn's guid,
//! level, what it offers and what this project does to it), Creature (a
//! read-only summary of the template row) and the buttons that open its
//! windows. Those stay in place and the 21 columns scroll under them, in the
//! height the sections leave: folding a section gives its height to the
//! columns.
//! [`template_window`] holds `creature_template`'s 78, in a window that can be
//! dragged off the panel and left open while the pointer works in the world.
//!
//! The sidebar is also where a creature's rows in other tables are opened
//! from, and each is its own window for the same reason: waypoints are a path drawn
//! in the world and edited beside it, a quest list is a list, and a trainer's
//! spells are another list. One panel holding all of them would be a single
//! long scroll.
//!
//! ## How a column is drawn
//!
//! Both forms are [`super::rowform`]'s page: the column's name in a fixed
//! cell, its value beside it, and what the value means after that, one column
//! per row, under a folding heading per group. A column that names a row of
//! another table draws the number, a `…` that chooses one by name through
//! [`super::quests::picker`], and the name the number resolves to as a link
//! ([`super::reference`]). The five display columns draw a picture of the
//! model beside the number and a `choose…` that opens [`super::displays`], a
//! grid of every `CreatureDisplayInfo` row as a rendered model.
//!
//! ## Where a typed value goes
//!
//! A typed value goes into the project's own edits store, through
//! [`crate::session::EditSession::set_server_edit`], already escaped as a SQL
//! literal, and onto the editor's undo stack. A save writes the store and the
//! SQL it comes to. Nothing reaches the database until Apply is pressed, and
//! the server is restarted afterwards. [`crate::server::creatures`] states the
//! reason.
//!
//! A field the project has edited is drawn in the warning colour, and a revert
//! button takes the one column back to what the database holds.
//!
//! ## Why this form is separate from the table browser
//!
//! [`crate::ui::data`] is a browser over `DbcFile` records: a row is an index, a
//! field is a column of a fixed-width record, and an edit writes bytes. A server
//! row is a keyed map of text and an edit is a statement. Making one browser do
//! both would be a `Table` with two backends, a refactor of its own.

use bevy_egui::egui;

use super::theme;
use crate::server::creatures::{REVERT_VPATH as REVERT, SQL_VPATH as SQL};
use crate::session::EditSession;
use crate::tools::creatures::{Creatures, Spawn};
use crate::tools::displays::{DisplayPick, Table as DisplayTable};
use crate::tools::place;
use crate::tools::quests::ColumnTarget;
use vale_client::assets::GameAssets;
use vale_mangos::creature::{self, Column, Group, Kind};
use vale_mangos::row::{Key, Life};

use super::rowform::{
    choice_cell, draft_or, finished, flags_cell, meaning, meaning_truncated, number_cell,
    number_means, page_row, page_spacing, revert_button, section, text_cell, FORM_ROW,
    FORM_VALUE,
};

/// Everything the panel needs, as one argument.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub creatures: &'a mut Creatures,
    /// The waypoint mode, which the Waypoints button opens and shuts.
    ///
    /// Passed here rather than reached through the tool because the button
    /// that opens it is on this panel: a creature has to be selected before a
    /// path can be, and this panel is where a creature is selected.
    pub waypoints: &'a mut crate::tools::waypoints::Waypoints,
    /// The quest tool's state: which creature's quests its window is showing
    /// (see `super::quests::holder_window`), the reference picker every form
    /// shares, and the name cache a reference column resolves through.
    pub quests: &'a mut crate::tools::quests::Quests,
    /// The loot window's state, for the Loot button beside Quests. See
    /// `super::loot::window`.
    pub loot: &'a mut crate::tools::loot::Loot,
    /// The behaviour windows' state, for the Events and Spells buttons. See
    /// `super::behaviour::windows`.
    pub behaviour: &'a mut crate::tools::behaviour::Behaviour,
    /// The Vendor and Trainer windows' state, for their two buttons. See
    /// `super::services::windows`.
    pub services: &'a mut crate::tools::services::Services,
    /// The gossip window's state, for the Gossip button. See
    /// `super::gossip::window`.
    pub gossip: &'a mut crate::tools::gossip::Gossip,
    /// Where this machine's server is. The panel's sentences read it, and it
    /// decides whether there is anything to say.
    pub server: &'a crate::server::settings::ServerSettings,
    /// The top bar's Server… popover, which holds every server operation.
    /// A panel that reports an edit as not applied must be able to open the
    /// panel that applies it. See [`super::sync`].
    pub server_panel: &'a mut super::popover::Popover,
    /// The archives, for what a display id looks like and for the client
    /// tables a reference resolves through.
    pub assets: &'a GameAssets,
    /// The pictures drawn in the picker's rows, the form's display cells and
    /// the preview panes. See `crate::portraits`.
    pub portraits: &'a mut crate::portraits::Portraits,
    /// The icons the reference picker's spell and item rows draw. See
    /// `super::thumbnails`.
    pub thumbnails: &'a mut super::thumbnails::Thumbnails,
    /// The frame clock the undo stack folds a gesture by. See
    /// `crate::session::Gesture`. Typing into a name box is one entry rather
    /// than one per keystroke because every keystroke names the same subject
    /// inside the fold window.
    pub now: f64,
    /// The gizmo, for the Handles switch drawn under the Select/Place switch.
    /// `None` where the panel's parts are drawn without it, as in the
    /// template window.
    pub gizmo: Option<&'a mut crate::tools::gizmo::Gizmo>,
}

/// Draw the creature tool's panel, and the two dialogs its forms open.
pub fn draw(ui: &mut egui::Ui, mut subject: Subject<'_>) {
    panel(ui, &mut subject);
    dialogs(ui.ctx(), &mut subject);
}

/// The two dialogs a form here can open: the display picker and the
/// reference picker. Drawn against the context, so the sidebar and the
/// template window reach the same two.
fn dialogs(ctx: &egui::Context, subject: &mut Subject<'_>) {
    super::displays::window(
        ctx,
        &mut subject.creatures.display_pick,
        subject.session,
        subject.assets,
        subject.portraits,
        subject.now,
    );
    super::quests::picker(
        ctx,
        subject.session,
        subject.quests,
        subject.assets,
        subject.thumbnails,
        None,
        subject.now,
    );
}

/// The panel itself.
fn panel(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    controls(ui, subject);
    ui.add_space(8.0);

    if subject.creatures.trouble.is_some() {
        nothing_to_show(ui, subject);
        return;
    }

    // What this project changes and how much of it is in the database are
    // worked out once a frame and passed to both the block and the selected
    // spawn's sentence. Each value is a walk of the store or a file read.
    //
    // Drawn above the mode switch because it is about the project rather than
    // about either mode. See [`edits_block`].
    let plan = crate::server::creatures::plan(subject.session);
    let on_server = crate::server::creatures::OnTheServer::read_with(subject.session, &plan);
    edits_block(ui, subject, &plan, &on_server);

    // Select or Place, the placement tools' own switch. See
    // `crate::tools::place::Mode` for why this is a mode rather than a second
    // rail entry.
    let mut mode = subject.creatures.mode;
    theme::segmented(ui, &mut mode, &place::MODES, |a, b| a == b);
    if mode != subject.creatures.mode {
        subject.creatures.mode = mode;
        // Switching away puts down whatever was on the cursor.
        // `tools::creatures::ghost` also does this; doing it here as well
        // makes the panel correct on the frame of the switch rather than on
        // the frame after.
        if mode == place::Mode::Select {
            subject.creatures.new_spawn.chosen = None;
        }
        // Only one mode may take the click. The waypoint mode takes a press
        // ahead of the placer (see `tools::creatures::press`), so arming Place
        // while the waypoint mode is adding would make a click place a point
        // instead of a creature.
        if mode == place::Mode::Place {
            subject.waypoints.adding = false;
        }
    }
    ui.add_space(6.0);

    if subject.creatures.mode == place::Mode::Place {
        picker(ui, subject);
        return;
    }

    if subject.creatures.spawn_count() == 0 {
        nothing_to_show(ui, subject);
        return;
    }
    let Some(spawn) = subject
        .creatures
        .chosen_edited(Some(&subject.session.server_edits))
    else {
        theme::note(
            ui,
            "Click a creature in the viewport to open it. The nearest ones are drawn as \
             models; everything further out is a ring on the ground.",
        );
        return;
    };

    if let Some(gizmo) = subject.gizmo.as_deref_mut() {
        handles(ui, gizmo);
    }
    match group_line(ui, subject.creatures.also.len() + 1, "creatures") {
        Some(GroupAsk::Only) => subject.creatures.also.clear(),
        Some(GroupAsk::Clear) => subject.creatures.select_only(None),
        None => {}
    }
    // The spawn's name, the two folding sections and the buttons stay in
    // place; only the spawn's columns scroll. The buttons are used while the
    // form below is scrolled to any column, so they are outside both sections.
    //
    // The sections fold because the scrolled form takes the height they
    // leave. With both open on a short window the form is at its minimum and
    // the whole panel scrolls; with either shut the form has that much more.
    // egui keeps each section's state under its id, so a section stays as it
    // was left when another creature is selected.
    title(ui, &spawn);
    let edits = &subject.session.server_edits;
    let spawn_edited = edits.touches(creature::SPAWN, &spawn.key());
    let template_edited = edits.touches(creature::TEMPLATE, &spawn.template_key());
    section(ui, "creature-panel-spawn", "Spawn", spawn_edited, true, |ui| {
        head(ui, subject, &spawn);
        claim(ui, subject, &spawn, &on_server);
    });
    section(ui, "creature-panel-template", "Creature", template_edited, true, |ui| {
        summary(ui, subject, &spawn, template_edited);
    });
    ui.add_space(4.0);
    windows(ui, subject, &spawn);
    ui.add_space(8.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .min_scrolled_height(SPAWN_FORM_MIN)
        .show(ui, |ui| spawn_form(ui, subject, &spawn));
}

/// The least height the scrolled spawn form takes, in points. Shared with the
/// game-object panel.
///
/// The inspector is itself a scroll area. When the window is too short for
/// the fixed part and this much of the form, the whole panel scrolls instead.
/// Without a minimum the form would be egui's default of 64 points, two rows.
pub(super) const SPAWN_FORM_MIN: f32 = 240.0;

/// What the line about a group of spawns asked for.
pub(super) enum GroupAsk {
    /// Keep the spawn whose form is open and drop the rest.
    Only,
    /// Drop the whole selection.
    Clear,
}

/// The line over a spawn's form when several spawns are selected: how many,
/// that the form is the primary's, and the two buttons that shrink the group.
/// Nothing is drawn for one. See `crate::tools::group`.
///
/// `pub(super)` because the game-object panel draws the same line.
pub(super) fn group_line(ui: &mut egui::Ui, count: usize, noun: &str) -> Option<GroupAsk> {
    if count < 2 {
        return None;
    }
    let mut asked = None;
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("{count} {noun} selected"))
            .strong()
            .color(theme::INK),
    );
    theme::note(
        ui,
        "This form is the brighter one's. A drag, the handles, the keys and \
         delete act on all of them.",
    );
    ui.horizontal(|ui| {
        if ui
            .small_button("Only this one")
            .on_hover_text("Keep this one and drop the rest.")
            .clicked()
        {
            asked = Some(GroupAsk::Only);
        }
        if ui.small_button("Clear").clicked() {
            asked = Some(GroupAsk::Clear);
        }
    });
    theme::note(ui, "shift + click adds or removes one · drag on empty ground selects a rectangle");
    ui.add_space(4.0);
    asked
}

/// What the tool is showing, and the two switches that decide how much of it.
fn controls(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    let spawns = subject.creatures.spawns.len();
    let mine = subject.creatures.created.len();
    let near = subject.creatures.near.len();
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(match (subject.creatures.reading(), mine) {
                (true, _) => "reading the map…".to_string(),
                (false, 0) => format!("{spawns} spawn(s) on this map, {near} near"),
                // The project's own are not in the read's count and are drawn
                // beside the ones that are, so they are counted here.
                (false, mine) => {
                    format!(
                        "{spawns} spawn(s) on this map and {mine} of this project's, {near} near"
                    )
                }
            })
            .small()
            .color(theme::INK_DIM),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("Reload")
                .on_hover_text("Read the map's spawns from the database again.")
                .clicked()
            {
                subject.creatures.forget();
                subject.waypoints.forget();
            }
        });
    });

    ui.horizontal(|ui| {
        ui.checkbox(&mut subject.creatures.show_models, "Models")
            .on_hover_text(
                "Draw the nearest spawns as the creatures they are. Off leaves the rings, \
                 which are what everything past the budget gets anyway.",
            );
        ui.add(
            egui::DragValue::new(&mut subject.creatures.model_budget)
                .speed(5.0)
                .range(0..=2000)
                .prefix("at most "),
        )
        .on_hover_text(
            "How many models at once. The busiest tile on map 0 holds 288 spawns and the \
             editor streams a 7x7 block, so drawing every one of them is several thousand \
             creatures.",
        );
        ui.checkbox(&mut subject.creatures.show_services, "Services")
            .on_hover_text(
                "Draw icons over each creature within 150 yards for what its npc_flags \
                 offer: quests, a vendor list, training, a flight, an inn, a bank and the \
                 rest. The hover card names them.",
            );
    });
}

/// Why the list is empty: no connection, still reading, or no creature on
/// the map.
fn nothing_to_show(ui: &mut egui::Ui, subject: &Subject<'_>) {
    match &subject.creatures.trouble {
        Some(trouble) => {
            ui.label(egui::RichText::new(trouble).small().color(theme::WARN));
            theme::note(
                ui,
                "Creatures are rows in vmangos' database rather than files in the archives, \
                 so this tool needs a connection. Server… on the top bar is where it is set.",
            );
        }
        None if subject.creatures.reading() => {
            theme::waiting(ui, "reading\u{2026}");
        }
        None => {
            theme::note(ui, "No creature stands on this map.");
        }
    }
}

/// What this project changes about the server's creatures, and the button
/// that opens the panel that applies it.
///
/// Drawn whether or not a spawn is selected, because what the project has
/// changed is a question about the project rather than about the selection.
///
/// The operations are not on this panel. Apply, Put back and Discard are on
/// the top bar's Server… popover, beside the other two subjects' and in the
/// same words; [`super::sync`]'s module comment gives the reason for one
/// panel. This block shows only the state, so that a person looking at a
/// creature can tell whether what they see is in the database without opening
/// another panel.
fn edits_block(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    plan: &crate::server::creatures::Plan,
    on_server: &crate::server::creatures::OnTheServer,
) {
    let undoable = on_server.rows();
    if plan.is_empty() && undoable == 0 {
        return;
    }
    // How many of this project's rows and paths are not yet in the database.
    // Without this count the line below read the same whether nothing had
    // been applied or everything had.
    let outstanding = plan
        .rows
        .iter()
        .filter(|row| !on_server.covers(row.table, &row.key))
        .count()
        + plan
            .paths
            .iter()
            .filter(|path| !on_server.covers(path.which.table(), &path.key()))
            .count();

    ui.separator();
    theme::heading(ui, "This project");
    if !plan.is_empty() {
        // Amber while something is not yet applied, dim once all of it is in
        // the database. A warning colour that is always on stops being read
        // as a warning.
        let colour = match outstanding == 0 {
            true => theme::INK_DIM,
            false => theme::WARN,
        };
        ui.label(
            egui::RichText::new(format!("{} — {}", plan.line(), SQL))
                .small()
                .color(colour),
        );
    }
    for refused in &plan.refused {
        ui.label(egui::RichText::new(refused).small().color(theme::BAD));
    }

    // The project's state is two independent facts, and the line states both.
    // A revert puts the database back and leaves the project still claiming
    // the rows, so the editor goes on drawing the edited values.
    match (undoable, outstanding, plan.is_empty()) {
        (0, _, false) => theme::note(
            ui,
            "Not applied. The database still holds what it did; what is drawn here is \
             this project's own.",
        ),
        // Everything this project claims is in the database. This case has
        // its own sentence so that it can be told from the case above.
        (n, 0, false) => {
            let line = match on_server.current() {
                true => format!(
                    "Applied — all {n} row(s) are in the database. Restart the server to \
                     see them; Put back undoes it."
                ),
                // The signature is per process, so a project also reads this
                // way after a relaunch. See `EditSession::applied_creatures`.
                false => format!(
                    "Applied — all {n} row(s) are in the database, though not \
                     necessarily at the values shown: something has been edited since, \
                     or this is a later session. Apply again to be sure."
                ),
            };
            ui.label(egui::RichText::new(line).small().color(theme::INK_DIM));
        }
        (n, out, false) => {
            ui.label(
                egui::RichText::new(format!(
                    "{out} not applied yet; {n} row(s) are in the database and undoable"
                ))
                .small()
                .color(theme::WARN),
            );
        }
        (n, _, true) => {
            ui.label(
                egui::RichText::new(format!(
                    "{n} row(s) applied and undoable, and this project now changes none of them"
                ))
                .small()
                .color(theme::WARN),
            );
        }
    }
    if ui
        .button("Server\u{2026}")
        .on_hover_text(format!(
            "Apply these rows, put them back, or give them up — every server operation is \
             on one panel, with the spells and the items. {REVERT} is what puts them back.",
        ))
        .clicked()
    {
        subject.server_panel.show();
    }
    theme::note(
        ui,
        "Applying writes the rows and nothing else — the server reads its creatures once, \
         at startup, so restart it to see them.",
    );
    ui.separator();
}

/// What a creature entry looks like, as the picker, the form and the preview
/// need it: its model, its skins, its hair and its geosets.
///
/// For 10,388 of `CreatureDisplayInfo`'s 10,534 rows the body texture comes
/// from the client, not from the M2, so a picture built from the path alone
/// draws magenta. `crate::tools::displays::resolve_worn` makes that join
/// with `vale_assets`' own code, the same call `spawn_models` makes for a
/// creature in the world.
///
/// `None` for a display id the tables do not resolve. The picker draws such a
/// row with no picture rather than leaving it out.
///
/// Computed once per display id and kept in `Creatures::worn`: it is a DBC
/// lookup, a string build and a `Dressing`, and a picker row asks for it on
/// every frame it is drawn.
fn worn(subject: &mut Subject<'_>, display_id: u32) -> Option<crate::portraits::Worn> {
    if let Some(had) = subject.creatures.worn.get(&display_id) {
        return had.clone();
    }
    let found = crate::tools::displays::resolve_worn(subject.assets, display_id);
    subject.creatures.worn.insert(display_id, found.clone());
    found
}

/// The Place mode: choose a creature, then click the ground.
///
/// It has the same shape as the placement tools; see
/// [`crate::tools::place`] for why this is a mode rather than a second rail
/// entry. Only what is chosen differs: a doodad is a path in the archives and
/// a creature is a row of `creature_template`, so the list comes from a query
/// rather than from a listing, and the search covers names as well as
/// entries.
///
/// There is no Place button. The mode arms the pointer with the chosen
/// creature, so clicking a row is the last step before clicking the ground.
/// A separate button press would add a step to every placement and give two
/// ways to reach the same state.
fn picker(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    theme::heading(ui, "Creature");
    template_actions(ui, subject);
    ui.add_space(4.0);
    match subject.creatures.new_spawn.chosen.clone() {
        Some(chosen) => {
            ui.label(egui::RichText::new(chosen.label()).color(theme::INK));
            ui.label(
                egui::RichText::new(format!(
                    "level {}–{} · faction {} · display {}",
                    chosen.level.0, chosen.level.1, chosen.faction, chosen.display_id
                ))
                .small()
                .color(theme::INK_DIM),
            );
            if chosen.is_new() {
                ui.label(
                    egui::RichText::new(
                        "New: this project creates it. There is no row in the database yet.",
                    )
                    .small()
                    .color(theme::ACCENT),
                );
            }
            theme::note(
                ui,
                "Click the ground to place it. Escape puts it down; every column of \
                 the row can be edited afterwards in Select.",
            );
            theme::note(ui, ", and . turn it \u{b7} shift is three times \u{b7} alt + mouse turns it");
            theme::note(ui, "ctrl with alt snaps the turn to 15\u{b0}");
            // The window's toggle, here as well as under a selected spawn, so
            // a creature can be edited before any spawn of it exists.
            let open = subject.creatures.template_window;
            if ui
                .selectable_label(open, "Edit creature…")
                .on_hover_text(
                    "Open creature_template's 78 columns in a window of their own. The \
                     window follows the chosen creature.",
                )
                .clicked()
            {
                subject.creatures.template_window = !open;
            }
            // The large preview, the same pane the placement pickers use. See
            // `crate::portraits`, and `ui::inspector::preview_pane`, which is
            // this widget. A creature is drawn wearing its skins; the pose is
            // the model's own, because a picture has no skeleton to animate.
            if let Some(worn) = worn(subject, chosen.display_id) {
                let key = subject.portraits.want_large_worn(&worn);
                ui.add_space(4.0);
                super::inspector::preview_pane(ui, subject.portraits, &key);
            }
        }
        None => theme::note(
            ui,
            "No creature chosen. Find one below by name or entry; the next click on the \
             ground puts it there.",
        ),
    }

    created_templates(ui, subject);

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("find").color(theme::INK_DIM));
        ui.add(
            egui::TextEdit::singleline(&mut subject.creatures.new_spawn.search)
                .hint_text("name or entry")
                .desired_width(f32::INFINITY),
        );
    });
    if let Some(trouble) = subject.creatures.new_spawn.trouble.clone() {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
    }

    if subject.creatures.new_spawn.searching() {
        theme::waiting(ui, "searching\u{2026}");
        return;
    }
    if subject.creatures.new_spawn.search.trim().len() < 2 {
        theme::note(ui, "Type two letters of a name, or an entry.");
        return;
    }
    let matches = subject.creatures.new_spawn.matches.clone();
    if matches.is_empty() {
        ui.label(
            egui::RichText::new("no creature of that name")
                .small()
                .color(theme::INK_DIM),
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("new-spawn-matches")
        .max_height(MATCH_LIST_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for known in &matches {
                let on = subject
                    .creatures
                    .new_spawn
                    .chosen
                    .as_ref()
                    .is_some_and(|had| had.entry == known.entry);
                if creature_row(ui, subject, known, on) {
                    subject.creatures.new_spawn.chosen = Some(known.clone());
                }
            }
        });
}

/// The three buttons over the picker: a new template, a copy of the one the
/// window is about, and giving up a created one. The item workspace's list
/// has the same row.
fn template_actions(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    let now = subject.now;
    let patch = crate::tools::creatures::server_patch(subject.server);
    let shown = subject
        .creatures
        .template_subject(&subject.session.server_edits);
    let row_read = shown.as_ref().is_some_and(|shown| {
        subject
            .creatures
            .template
            .as_ref()
            .is_some_and(|held| held.entry == shown.entry)
    });
    let created = shown.as_ref().is_some_and(|shown| shown.is_new());
    ui.horizontal(|ui| {
        let third = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(40.0);
        let size = egui::vec2(third, 22.0);
        if ui
            .add(egui::Button::new("+ New").min_size(size))
            .on_hover_text(
                "A new creature_template row, numbered above anything upstream will \
                 reach. It starts as a level 1 friendly humanoid with no model: a row \
                 the server loads, drawn as a box until display_id1 is chosen. The \
                 window opens on it.",
            )
            .clicked()
        {
            let entry = subject
                .creatures
                .create_template(subject.session, "New Creature", patch, now);
            subject.session.status = format!("creature_template {entry} created");
        }
        if ui
            .add_enabled(row_read, egui::Button::new("Copy").min_size(size))
            .on_hover_text(
                "A copy of the chosen creature under a new entry, with every column as \
                 it is shown: the database's value where this project has not changed \
                 it and the project's where it has.",
            )
            .on_disabled_hover_text("Choose a creature first.")
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject.session.status =
                    match subject
                        .creatures
                        .copy_template(subject.session, shown, patch, now)
                    {
                        Some(entry) => format!("copied as creature_template {entry}"),
                        None => "still reading this creature's row".to_string(),
                    };
            }
        }
        if ui
            .add_enabled(created, egui::Button::new("Discard").min_size(size))
            .on_hover_text(
                "Give up the template this project was going to create. Nothing in the \
                 database is touched. A template the database holds cannot be removed: \
                 .reload creature_template never drops an entry it has read.",
            )
            .on_disabled_hover_text("Only a creature this project created can be discarded.")
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject
                    .creatures
                    .discard_template(subject.session, shown.entry, now);
                subject.session.status = format!("creature_template {} discarded", shown.entry);
            }
        }
    });
}

/// The templates this project creates, listed above the search because no
/// query returns them. Clicking one chooses it, as a search match does.
fn created_templates(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    let templates = subject.creatures.templates.clone();
    if templates.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(format!(
            "{} creature(s) this project creates",
            templates.len()
        ))
        .small()
        .color(theme::INK_DIM),
    );
    egui::ScrollArea::vertical()
        .id_salt("created-templates")
        .max_height(CREATED_LIST_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for known in &templates {
                let on = subject
                    .creatures
                    .new_spawn
                    .chosen
                    .as_ref()
                    .is_some_and(|had| had.entry == known.entry);
                if creature_row(ui, subject, known, on) {
                    subject.creatures.new_spawn.chosen = Some(known.clone());
                }
            }
        });
}

/// The height the created list may take before it scrolls.
const CREATED_LIST_HEIGHT: f32 = 160.0;

/// One row of the picker: a picture of the creature, its name and its entry.
///
/// The row is [`theme::list_row`], as it is for the spell and item lists, so
/// the three lists share one set of text sizes. The part specific to this
/// picker is the picture, which is a rendered model rather than a decoded
/// icon. See [`crate::portraits`].
fn creature_row(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    known: &crate::tools::creatures::Known,
    on: bool,
) -> bool {
    let sub = known
        .subname
        .as_deref()
        .filter(|text| !text.is_empty())
        .unwrap_or_default();
    let shape = theme::list_row(
        ui,
        theme::ListRow {
            title: &known.name,
            sub,
            trailing: &known.entry.to_string(),
            tint: theme::INK,
            picture: true,
        },
        on,
    );
    // Requested on every frame the row is drawn. Only rows on screen pay for
    // a picture, and a request that is not repeated is dropped unserved. See
    // `crate::portraits`.
    match worn(subject, known.display_id) {
        Some(worn) => {
            let key = subject.portraits.want_worn(&worn);
            crate::portraits::paint(ui, subject.portraits, &key, shape.picture);
        }
        // A display id the tables do not resolve. The row still lists the
        // creature, because the server will load a spawn of it and the client
        // will draw it as nothing.
        None => {
            ui.painter().rect_filled(shape.picture, 3.0, theme::LINE);
        }
    }
    shape.response.clicked()
}

/// The height the picker's match list may take, so the preview pane above it
/// keeps its room.
const MATCH_LIST_HEIGHT: f32 = 260.0;

/// What this project claims about the selected spawn, and the button that
/// changes it.
///
/// There are three states, and each means something different for what Apply
/// will do, so each is drawn as a sentence rather than a colour:
///
/// ```text
/// new       there is no row for it yet; Apply creates one
/// removed   there is, and Apply takes it away with its path and its addon
/// ordinary  Apply writes whatever columns have been changed
/// ```
fn claim(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    spawn: &Spawn,
    on_server: &crate::server::creatures::OnTheServer,
) {
    let now = subject.now;
    // Whether this project has written this row to the database. Without it
    // the sentence below said "there is no row in the database" after Apply
    // had been pressed.
    let written = on_server.covers(creature::SPAWN, &spawn.key());
    if spawn.is_new() {
        ui.label(
            egui::RichText::new(match written {
                false => "New — this project creates it. There is no row in the database yet."
                    .to_string(),
                true => "New — created by this project and applied. The database holds the \
                         row; the server has to be restarted to see it in the world."
                    .to_string(),
            })
            .small()
            .color(theme::ACCENT),
        );
    }
    if spawn.is_removed() {
        ui.label(
            egui::RichText::new(match written {
                false => "Marked for removal. Applying deletes the row, and with it the \
                          spawn's waypoints, its addon and its game-event rows."
                    .to_string(),
                true => "Removed, and applied. The row and its five dependent tables are \
                         gone from the database; Put back restores all six."
                    .to_string(),
            })
            .small()
            .color(theme::BAD),
        );
    }
    ui.horizontal(|ui| {
        match spawn.is_removed() {
            true => {
                if ui
                    .button("Keep it")
                    .on_hover_text(
                        "Take the removal back. Any columns this project had changed on it \
                         are still changed.",
                    )
                    .clicked()
                {
                    crate::tools::creatures::Creatures::keep(subject.session, spawn.guid, now);
                    subject.session.status = format!("spawn {} is kept", spawn.guid);
                }
            }
            false => {
                // A creature this project created is dropped rather than
                // marked for removal, and the button label says which: a
                // `DELETE` for a row this project never inserted would act on
                // a row it does not own.
                let word = match spawn.is_new() {
                    true => "Discard this spawn",
                    false => "Remove this spawn",
                };
                if ui
                    .button(egui::RichText::new(word).color(theme::BAD))
                    .on_hover_text(match (spawn.is_new(), written) {
                        (true, true) => {
                            "Give up this project's claim on the spawn. The row \
                                         is already in the database and stays there — Put \
                                         the rows back is what removes it."
                        }
                        (true, false) => {
                            "Give up the spawn this project was going to create. \
                                 Nothing in the database is touched."
                        }
                        (false, _) => {
                            "Mark the row for deletion. Nothing happens until \
                                  Apply, and then the server has to be restarted — .reload \
                                  creature never erases a spawn it has already read."
                        }
                    })
                    .clicked()
                {
                    crate::tools::creatures::Creatures::remove(subject.session, spawn.guid, now);
                    subject.session.status = match spawn.is_new() {
                        true => format!("spawn {} discarded", spawn.guid),
                        false => format!("spawn {} is marked for removal", spawn.guid),
                    };
                    if spawn.is_new() {
                        subject.creatures.select_only(None);
                    }
                }
            }
        }
    });
}

/// The selected spawn's name and subname. Drawn above the folding sections,
/// so the panel says which creature is open when both are shut.
fn title(ui: &mut egui::Ui, spawn: &Spawn) {
    ui.label(egui::RichText::new(spawn.label()).strong().size(14.0));
    if let Some(subname) = spawn.subname.as_deref().filter(|s| !s.is_empty()) {
        ui.label(
            egui::RichText::new(format!("<{subname}>"))
                .small()
                .color(theme::INK_DIM),
        );
    }
}

/// The Spawn section's first part: the selected spawn's guid, level, rank and
/// faction, what it offers, and the two buttons.
fn head(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    ui.label(
        egui::RichText::new(format!(
            "guid {}  ·  level {}–{}  ·  {}  ·  faction {}",
            spawn.guid,
            spawn.level.0,
            spawn.level.1,
            creature::value_word(&creature::RANKS, spawn.rank),
            spawn.faction
        ))
        .small()
        .color(theme::INK_DIM),
    );
    if spawn.npc_flags != 0 {
        ui.label(
            egui::RichText::new(creature::mask_words(&creature::NPC_FLAGS, spawn.npc_flags))
                .small()
                .color(theme::ACCENT),
        );
    }
    ui.horizontal(|ui| {
        if ui
            .small_button("Go to")
            .on_hover_text("Put the camera on it.")
            .clicked()
        {
            let at = spawn.at;
            subject.creatures.fly_to(at);
        }
        // Duplicate is on the selection rather than in the picker because it
        // copies a row: every column of this spawn, including the ones this
        // project has changed. The picker chooses a creature. The placement
        // tools' Duplicate arms their picker instead, because a doodad is
        // only its model.
        let has_row = subject
            .creatures
            .spawn_row
            .as_ref()
            .is_some_and(|held| Some(held.guid) == subject.creatures.selected);
        if ui
            .add_enabled(has_row, egui::Button::new("Duplicate"))
            .on_hover_text(
                "Another spawn of this creature two yards north, carrying every column \
                 this one has — including the ones this project has changed.",
            )
            .on_disabled_hover_text("Still reading this spawn's row.")
            .clicked()
        {
            let now = subject.now;
            subject.session.status = match subject.creatures.duplicate(subject.session, now) {
                Some(guid) => format!("duplicated as guid {guid}"),
                None => "nothing to duplicate".into(),
            };
        }
        let others = subject
            .creatures
            .spawns
            .iter()
            .filter(|other| other.entry == spawn.entry)
            .count();
        ui.label(
            egui::RichText::new(format!("{others} of this creature on this map"))
                .small()
                .color(theme::INK_FAINT),
        );
    });
}

/// The `creature` row: one spawn, at one place.
fn spawn_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    theme::heading(ui, "This spawn");
    theme::note(
        ui,
        "One creature, at one place. Click it in the viewport to open it, then drag the \
         one that is open to move it — the position columns are what a drag writes, as \
         one undo step.",
    );
    let key = spawn.key();
    let Some(row) = subject
        .creatures
        .spawn_row
        .as_ref()
        .map(|held| held.row.clone())
    else {
        theme::waiting(ui, "reading the row\u{2026}");
        return;
    };
    let standing = crate::tools::spawn::Standing {
        kind: crate::tools::spawn::Kind::Creature,
        guid: spawn.guid,
        at: spawn.at,
        orientation: spawn.orientation,
    };
    placement(ui, subject.session, &standing, subject.now);
    groups(
        ui,
        subject,
        creature::SPAWN,
        &key,
        &row,
        &[Group::Identity, Group::Place],
    );
}

/// The Handles switch for a selected spawn. Shared with the game-object
/// panel. A spawn has no tilt, so Turn offers the z ring alone; see
/// `crate::tools::spawn`.
pub(super) fn handles(ui: &mut egui::Ui, gizmo: &mut crate::tools::gizmo::Gizmo) {
    use crate::tools::gizmo::{Handles, HANDLES};
    theme::heading(ui, "Handles");
    theme::segmented(ui, &mut gizmo.handles, &HANDLES, |a, b| a == b);
    theme::note(
        ui,
        match gizmo.handles {
            Handles::Move => "drag an arrow to move along one axis",
            Handles::Turn => "drag the ring to turn about z",
            Handles::Off => "no handles: the pointer picks and drags on the ground",
        },
    );
    ui.add_space(6.0);
}

/// The facing in degrees and the keys that move and turn a spawn. Shared
/// with the game-object panel. The position and the facing in radians are
/// also columns of the form below this block; the facing is repeated here in
/// degrees because the doodad and WMO panels state their turns in degrees.
pub(super) fn placement(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    standing: &crate::tools::spawn::Standing,
    now: f64,
) {
    theme::heading(ui, "Placement");
    let mut degrees = standing.orientation.to_degrees();
    theme::row(ui, "facing", |ui| {
        if ui
            .add(
                egui::DragValue::new(&mut degrees)
                    .speed(1.0)
                    .fixed_decimals(1)
                    .suffix("\u{b0}"),
            )
            .on_hover_text(
                "The orientation column in degrees, anticlockwise from north. \
                 Written back in radians.",
            )
            .changed()
        {
            standing.face(session, degrees.to_radians(), now);
        }
    });
    theme::note(ui, "arrows nudge \u{b7} page up/down raises \u{b7} shift is ten times");
    theme::note(ui, ", and . turn about z \u{b7} alt + mouse turns it");
    theme::note(ui, "ctrl with alt snaps the turn to 15\u{b0}");
    theme::note(ui, "delete removes it \u{b7} ctrl+d duplicates");
    ui.add_space(6.0);
}

/// The Creature section: a summary of what the creature is.
///
/// A summary rather than a form: the columns a person wants to see while
/// looking at a spawn (how strong it is, what it offers) without the 78 that
/// editing it needs, with a picture of its model beside them. Editing is in
/// [`template_window`], behind the first button of [`windows`].
///
/// The first line names the template row, which is what tells its scope from
/// the spawn's. `edited` is whether this project changes that row.
fn summary(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn, edited: bool) {
    ui.label(
        egui::RichText::new(format!(
            "creature_template entry {} at content patch {}",
            spawn.entry, spawn.patch
        ))
        .small()
        .color(match edited {
            true => theme::WARN,
            false => theme::INK_DIM,
        }),
    );
    ui.label(
        egui::RichText::new("Every one of these in the world, on every map.")
            .small()
            .color(theme::INK_FAINT),
    );

    // The few columns worth reading at a glance, from the row the map query
    // already carries, so this shows something before the whole row has been
    // fetched. The model beside them, from the same display id.
    ui.horizontal(|ui| {
        let (rect, _) =
            ui.allocate_exact_size(egui::Vec2::splat(SUMMARY_PICTURE), egui::Sense::hover());
        match worn(subject, spawn.display_id) {
            Some(worn) => {
                let key = subject.portraits.want_worn(&worn);
                crate::portraits::paint(ui, subject.portraits, &key, rect);
            }
            None => {
                ui.painter().rect_filled(rect, 3.0, theme::DEAD);
            }
        }
        egui::Grid::new("creature-summary")
            .num_columns(2)
            .spacing([8.0, 2.0])
            .show(ui, |ui| {
                for (name, value) in [
                    ("level", format!("{}–{}", spawn.level.0, spawn.level.1)),
                    ("rank", creature::value_word(&creature::RANKS, spawn.rank)),
                    ("faction", spawn.faction.to_string()),
                    ("display", spawn.display_id.to_string()),
                    (
                        "offers",
                        creature::mask_words(&creature::NPC_FLAGS, spawn.npc_flags),
                    ),
                ] {
                    ui.label(egui::RichText::new(name).color(theme::INK_DIM));
                    ui.label(egui::RichText::new(value).color(theme::INK));
                    ui.end_row();
                }
            });
    });
}

/// The buttons that open the template window and what a creature has besides
/// its template: its path, quests, loot, events, spells and its two service
/// lists. Each is a toggle, lit while its window is open. Drawn outside the
/// folding sections, so they are reachable with both shut.
fn windows(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    ui.horizontal_wrapped(|ui| {
        let open = subject.creatures.template_window;
        if ui
            .selectable_label(open, "Edit creature…")
            .on_hover_text(
                "Open creature_template's 78 columns in a window of their own. It can be \
                 dragged off the panel and left open while the pointer works in the world.",
            )
            .clicked()
        {
            subject.creatures.template_window = !open;
        }
        // Waypoints is a mode rather than a form: it takes the pointer while
        // it is open, so it is a toggle like the template window beside it,
        // not a button that opens something and returns. The toggle shows or
        // hides the window, not this creature's path. The path in the window
        // follows the selection, as the template window does. See
        // `tools::waypoints::follow_the_selection`.
        let showing = subject.waypoints.showing;
        if ui
            .selectable_label(showing, "Waypoints")
            .on_hover_text(
                "The path this creature walks, out of creature_movement: drawn on the ground, \
                 its points picked, dragged, added and removed. It follows the selection, so \
                 clicking another creature shows that creature's path.",
            )
            .clicked()
        {
            match showing {
                true => subject.waypoints.close(),
                false => {
                    subject.waypoints.showing = true;
                    // Opened here as well as by the follow, so the window has
                    // its creature on the frame the button is pressed rather
                    // than on the next frame.
                    subject
                        .waypoints
                        .open_for(spawn.guid, spawn.entry, spawn.movement_type);
                }
            }
        }
        // Quests is a window over the world, like the two beside it, and like
        // them it follows the selection. `super::quests` draws it, and the
        // shell keeps it on the chosen creature.
        let quests_open = subject
            .quests
            .window_for
            .as_ref()
            .is_some_and(|(holder, _, _)| *holder == crate::tools::quests::Holder::Creature);
        if ui
            .selectable_label(quests_open, "Quests")
            .on_hover_text(
                "What it gives and what it takes: creature_questrelation and \
                 creature_involvedrelation, listed, added to and removed from. Each quest \
                 opens in the quest workspace. The rows name this creature_template entry, \
                 so they are about every spawn of it.",
            )
            .clicked()
        {
            subject.quests.window_for = match quests_open {
                true => None,
                false => Some((
                    crate::tools::quests::Holder::Creature,
                    spawn.entry,
                    spawn.label(),
                )),
            };
        }
        // Loot is a window over the world that follows the selection, like
        // the quest window. It shows the rows this creature's three loot
        // columns name. See `super::loot`.
        if ui
            .selectable_label(subject.loot.open, "Loot")
            .on_hover_text(
                "What it drops, what is pickpocketed from it and what it is skinned for: \
                 creature_loot_template, pickpocketing_loot_template and \
                 skinning_loot_template, by the three loot ids on its template. The rows \
                 name a loot id, so they are about every creature that shares it.",
            )
            .clicked()
        {
            subject.loot.toggle();
        }
        // Events and Spells are two windows over the world that follow the
        // selection, like the two beside them. See `super::behaviour`.
        if ui
            .selectable_label(subject.behaviour.events_open, "Events")
            .on_hover_text(
                "What it reacts to, out of creature_ai_events: each trigger with its \
                 parameters named, and the creature_ai_scripts each runs, opened into the \
                 script window. Read only by a template whose ai_name is EventAI, which the \
                 window offers to set.",
            )
            .clicked()
        {
            subject.behaviour.events_open = !subject.behaviour.events_open;
        }
        if ui
            .selectable_label(subject.behaviour.spells_open, "Spells")
            .on_hover_text(
                "What it casts in combat, out of creature_spells by the template's \
                 spell_list_id: eight slots, each a spell, its chance, its target, its \
                 flags and its timers.",
            )
            .clicked()
        {
            subject.behaviour.spells_open = !subject.behaviour.spells_open;
        }
        // Gossip is a window over the world that follows the selection, like
        // the ones beside it. See `super::gossip`.
        if ui
            .selectable_label(subject.gossip.open, "Gossip")
            .on_hover_text(
                "What it says when spoken to and the options it offers: the gossip_menu \
                 its template's gossip_menu_id names, with its texts, options and the \
                 menus they lead to. Offers to make a menu when it has none.",
            )
            .clicked()
        {
            subject.gossip.open = !subject.gossip.open;
            subject.gossip.menu = None;
            subject.gossip.trail.clear();
        }
        // Vendor and Trainer are two windows over the world that follow the
        // selection, like the four beside them. See `super::services`.
        use crate::tools::services::Kind;
        if ui
            .selectable_label(subject.services.is_open(Kind::Vendor), "Vendor")
            .on_hover_text(
                "What it sells: npc_vendor under its entry, and npc_vendor_template under its \
                 vendor_id, which other creatures may share. Each item with its place in the \
                 list, its stock and its restock time.",
            )
            .clicked()
        {
            subject.services.toggle(Kind::Vendor);
        }
        if ui
            .selectable_label(subject.services.is_open(Kind::Trainer), "Trainer")
            .on_hover_text(
                "What it teaches: npc_trainer under its entry, and npc_trainer_template under its \
                 trainer_id, which other creatures may share. Each teaching spell with the level, \
                 the price and the skill it needs.",
            )
            .clicked()
        {
            subject.services.toggle(Kind::Trainer);
        }
    });
}

/// The side of the picture beside the summary, in points.
const SUMMARY_PICTURE: f32 = 56.0;

/// `creature_template`'s own window, where a creature is edited.
///
/// Drawn on the root context rather than inside the inspector, like the map
/// window, because a window inside a panel is clipped to it, and this is a
/// window so that it can be larger than the panel and placed elsewhere on the
/// screen.
///
/// The title names the creature and its entry, as the body does: two
/// creatures' windows hold numbers that look alike, and the entry tells them
/// apart.
pub fn template_window(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    if !subject.creatures.template_window {
        return None;
    }
    // The picker's creature in Place, the selected spawn's template in
    // Select; see `Creatures::template_subject`.
    let shown = subject
        .creatures
        .template_subject(&subject.session.server_edits)?;
    let mut open = true;
    let response = egui::Window::new(format!(
        "{} — creature_template {}",
        shown.label, shown.entry
    ))
    // A fixed id, independent of the title. egui keeps a window's position
    // under its id, and the id defaults to the title, so a window titled after
    // the creature went back to its default position every time another
    // creature was clicked.
    .id(egui::Id::new("creature-template"))
    .open(&mut open)
    // Wide enough for the page layout's three columns: the name cell, the
    // value cell and a name resolved beside it.
    .default_size([680.0, 640.0])
    // Clear of the top bar and to the left of the inspector, so it covers
    // neither the button that opened it nor the spawn it is about.
    .default_pos([300.0, 80.0])
    .resizable(true)
    .frame(
        egui::Frame::default()
            .fill(theme::SHELL)
            .stroke(egui::Stroke::new(1.0, theme::LINE))
            .corner_radius(egui::CornerRadius::same(4))
            .inner_margin(egui::Margin::same(8)),
    )
    .show(ctx, |ui| {
        template_head(ui, subject, &shown);
        ui.add_space(6.0);
        let key = shown.key();
        let Some(row) = subject
            .creatures
            .template
            .as_ref()
            .filter(|held| held.entry == shown.entry)
            .map(|held| held.row.clone())
        else {
            theme::waiting(ui, "reading the row\u{2026}");
            return;
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                groups(
                    ui,
                    subject,
                    creature::TEMPLATE,
                    &key,
                    &row,
                    &[Group::Identity, Group::Appearance],
                );
            });
    });
    subject.creatures.template_window = open;
    // The dialogs are drawn here as well as under the panel, because the
    // window is drawn whether or not the inspector is showing this tool's
    // panel; `Quests::picker_pass` and the display picker's own slot keep
    // each to one draw a frame.
    dialogs(ctx, subject);
    response.map(|shown| shown.response.rect)
}

/// The window's head: what this project says of the row, how many spawns of
/// it the open map has, and the entry box.
fn template_head(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    shown: &crate::tools::creatures::TemplateSubject,
) {
    match shown.claim {
        Life::Insert => {
            ui.label(
                egui::RichText::new(
                    "New: this row is in no database yet. Apply writes it, and the server \
                     has to be restarted for a spawn of it to stand.",
                )
                .small()
                .color(theme::ACCENT),
            );
        }
        _ => {
            ui.label(
                egui::RichText::new(format!(
                    "Every one of these in the world, on every map — {} on this one",
                    subject
                        .creatures
                        .spawns
                        .iter()
                        .filter(|other| other.entry == shown.entry)
                        .count()
                ))
                .small()
                .color(theme::WARN),
            );
        }
    }
    ui.horizontal(|ui| {
        let colour = match shown.entry != shown.read_entry {
            true => theme::WARN,
            false => theme::INK_DIM,
        };
        ui.label(egui::RichText::new("entry").color(colour))
            .on_hover_text("the creature id");
        let id = ui.make_persistent_id(("creature-template-entry", shown.patch));
        entry_field(ui, subject, shown, id);
    });
}

/// The `entry` box: the one field of the window that is not a column edit.
///
/// Committed on Enter or on leaving the box rather than per keystroke, because
/// a key that moved on every digit typed would re-key the row four times on
/// the way to `20000`, and three of those are entries somebody else may own.
///
/// It does the same for any row: the project's claim on the row is re-keyed,
/// so the template is one row before and after; see
/// [`crate::tools::creatures::Creatures::rekey_template`]. For a row the
/// database holds, the apply then moves the row and every column of the world
/// database that names a creature by entry.
fn entry_field(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    shown: &crate::tools::creatures::TemplateSubject,
    id: egui::Id,
) {
    let mut text = draft_or(ui, id, shown.entry.to_string());
    let response = ui.add_sized(
        egui::vec2(FORM_VALUE, FORM_ROW),
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .horizontal_align(egui::Align::Center),
    );
    // The refusal is kept until the box is touched again. One raised on the
    // frame Enter was pressed and dropped on the next would not be read.
    let trouble_id = id.with("trouble");
    if let Some(text) = finished(ui, id, &response, text) {
        let trouble = match text.trim().parse::<u32>() {
            Err(_) => Some(format!("{text:?} is not an entry")),
            Ok(wanted) => {
                let now = subject.now;
                match subject
                    .creatures
                    .rekey_template(subject.session, shown, wanted, now)
                {
                    Ok(()) => None,
                    Err(why) => Some(why),
                }
            }
        };
        ui.data_mut(|data| match trouble {
            Some(why) => {
                data.insert_temp(trouble_id, why);
            }
            None => {
                data.remove_temp::<String>(trouble_id);
            }
        });
    }
    if let Some(why) = ui.data_mut(|data| data.get_temp::<String>(trouble_id)) {
        ui.label(egui::RichText::new(why).small().color(theme::BAD));
        return;
    }
    let (word, colour) = match (shown.is_new(), shown.read_entry != shown.entry) {
        (true, _) => ("renumbers this new creature".to_string(), theme::INK_FAINT),
        (_, true) => (
            format!(
                "the database has it at {} until this is applied",
                shown.read_entry
            ),
            theme::WARN,
        ),
        _ => (
            "moves the row, and what names it in the world database follows".to_string(),
            theme::INK_FAINT,
        ),
    };
    ui.label(egui::RichText::new(word).small().color(colour))
        .on_hover_text(format!(
            "vmangos has no foreign keys, so the apply writes the cascade itself: the \
             row, and the {} columns of the world database that name a creature by \
             entry: every spawn's id, the quest relations, npc_vendor, npc_trainer, \
             the AI events, the loot and movement templates. Put back returns all of \
             it.\n\nNot reached: a script's datalong, the conditions table, and the \
             C++ scripts, which name entries as constants.",
            vale_mangos::creature::TEMPLATE_REFERENCES.len()
        ));
}

/// Every group of a table's columns, one page section per group: the named
/// ones open, the rest folded.
///
/// 78 columns is more than a person reads at once and the ones worth reading
/// first are not the ones that come first in the table. A group holding an
/// edit is opened whatever `open` says, so a change is never hidden under a
/// folded heading.
fn groups(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    table: &'static str,
    key: &Key,
    row: &creature::Row,
    open: &[Group],
) {
    page_spacing(ui);
    let mut seen: Vec<Group> = Vec::new();
    for column in creature::columns_of(table) {
        if !seen.contains(&column.group) {
            seen.push(column.group);
        }
    }
    for group in seen {
        let columns: Vec<&'static Column> = creature::columns_of(table)
            .iter()
            .filter(|column| column.group == group)
            .collect();
        let edited = columns.iter().any(|column| {
            subject
                .session
                .server_edits
                .get(table, key, column.name)
                .is_some()
        });
        section(
            ui,
            (table, group.name()),
            group.name(),
            edited,
            open.contains(&group),
            |ui| {
                for column in columns {
                    field(ui, subject, table, key, row, column);
                }
            },
        );
    }
    ui.add_space(12.0);
}

/// One column on a row of its own: its name, the control its [`Kind`] asks for,
/// what the value means, and the revert arrow when this project has changed
/// it.
fn field(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    table: &'static str,
    key: &Key,
    row: &creature::Row,
    column: &'static Column,
) {
    let edited = subject
        .session
        .server_edits
        .get(table, key, column.name)
        .map(str::to_string);
    // What the field shows: this project's value where it has one, and the
    // database's otherwise.
    let in_database = column.literal(row);
    let showing = edited.clone().or_else(|| in_database.clone());
    // No revert arrow on a row this project creates. Every column of such a
    // row is the project's own and the database has no value to revert to.
    // Clearing one would leave the `INSERT` naming one column fewer than the
    // table has.
    let creating = subject.session.server_edits.life(table, key) == Life::Insert;

    // One id per table, row and column. Two spawns of the same creature have
    // the same column names, and egui keys a widget's state (a text box's
    // draft, whether a menu is open) on its id, so a shared id would carry the
    // half-typed name of one row into the next row clicked.
    let id = ui.make_persistent_id((table, key.text(), column.name));
    let pending = page_row(ui, column.name, column.about, edited.is_some(), |ui| {
        let Some(showing) = showing.as_deref() else {
            ui.label(egui::RichText::new("NULL").color(theme::INK_FAINT));
            return None;
        };
        // A reference whose name did not fit beside its number, to be drawn
        // on the row after this one.
        let mut pending: Option<(&'static str, u32)> = None;
        let written = match column.kind {
            // The key is what the `WHERE` names, so it is shown and never
            // edited: writing one would move the row the edit is about.
            Kind::Key => {
                ui.label(theme::number(showing));
                None
            }
            Kind::Text => text_cell(ui, id.with("text"), showing),
            Kind::Flags(bits) => flags_cell(ui, showing, bits, id),
            Kind::Choice(values) => choice_cell(ui, showing, values, id),
            Kind::Ref(dbc) if DisplayTable::of(dbc).is_some() => {
                let target = target(table, key, column, &in_database, creating);
                display_cell(ui, subject, showing, target)
            }
            Kind::Ref(dbc) => {
                let written = number_cell(ui, Kind::Unsigned, showing);
                let target = target(table, key, column, &in_database, creating);
                let mut resolver = super::reference::Resolver {
                    session: &mut *subject.session,
                    assets: subject.assets,
                    quests: &mut *subject.quests,
                };
                pending = super::reference::cell(ui, &mut resolver, dbc, showing, Some(target))
                    .map(|id| (dbc, id));
                written
            }
            kind => {
                let written = number_cell(ui, kind, showing);
                if let Some(means) = number_means(kind, showing) {
                    meaning(ui, means);
                }
                written
            }
        };
        // A gossip menu can be opened from here, and a new one made and named.
        if table == creature::TEMPLATE && column.name == "gossip_menu_id" {
            let npc_flags = subject
                .session
                .server_edits
                .get(table, key, "npc_flags")
                .map(str::to_string)
                .or_else(|| creature::column(table, "npc_flags").and_then(|flags| flags.literal(row)))
                .and_then(|flags| flags.trim().parse::<u32>().ok())
                .unwrap_or(0);
            let menu = showing.trim().parse().unwrap_or(0);
            use vale_mangos::schema::RowValue;
            let label = format!("{} ({})", row.text("name").unwrap_or("creature"), row.integer("entry").unwrap_or(0));
            super::gossip::menu_buttons(ui, subject.session, subject.gossip, key, menu, npc_flags, &label);
        }
        if let Some(written) = written {
            target(table, key, column, &in_database, creating).write(
                subject.session,
                written,
                subject.now,
            );
        }
        if edited.is_some() && !creating && revert_button(ui, in_database.as_deref()) {
            let gesture = format!("{table} {} {} revert", key.text(), column.name);
            subject.session.set_server_edit(
                table,
                key,
                column.name,
                None,
                Some(crate::session::Gesture {
                    label: "Revert column",
                    subject: &gesture,
                    now: subject.now,
                }),
            );
        }
        pending
    });
    if let Some((dbc, id)) = pending {
        page_row(ui, "", "", false, |ui| {
            let mut resolver = super::reference::Resolver {
                session: &mut *subject.session,
                assets: subject.assets,
                quests: &mut *subject.quests,
            };
            super::reference::name(ui, &mut resolver, dbc, id);
        });
    }
}

/// Where a value typed or chosen for a column is written: the column, the
/// value the database holds there, and the undo entry the write goes on.
///
/// The subject names the row and the column, so a value typed into the box
/// and one chosen in a dialog fold into one undo entry. A row this project
/// creates has no database value, so a choice of the shown value is written
/// rather than clearing the column.
fn target(
    table: &'static str,
    key: &Key,
    column: &'static Column,
    in_database: &Option<String>,
    creating: bool,
) -> ColumnTarget {
    ColumnTarget {
        table,
        key: key.clone(),
        column: column.name,
        in_database: match creating {
            true => None,
            false => in_database.clone(),
        },
        label: "Edit creature",
        subject: format!("{table} {} {}", key.text(), column.name),
    }
}

/// A display id: the number, a picture of the model it names, the button
/// that opens the picker, and the model's path.
///
/// The number stays editable beside the picture, because a user often has the
/// id already, for example from a wiki. The picture is the same rendered
/// portrait the Place picker's rows draw.
fn display_cell(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    showing: &str,
    target: ColumnTarget,
) -> Option<String> {
    let written = number_cell(ui, Kind::Unsigned, showing);
    let id: u32 = showing.trim().parse().unwrap_or(0);
    let worn = worn(subject, id);
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(FORM_ROW + 4.0), egui::Sense::hover());
    match worn.as_ref() {
        Some(worn) => {
            let key = subject.portraits.want_worn(worn);
            crate::portraits::paint(ui, subject.portraits, &key, rect);
        }
        None if id != 0 => {
            ui.painter().rect_filled(rect, 3.0, theme::DEAD);
        }
        None => {}
    }
    if ui
        .small_button("choose\u{2026}")
        .on_hover_text(
            "Every row of CreatureDisplayInfo.dbc that resolves to a model, as pictures, \
             searched by the model's path or a skin's name.",
        )
        .clicked()
    {
        subject.creatures.display_pick = Some(DisplayPick::open(DisplayTable::Creature, target, id));
    }
    match worn {
        Some(worn) => meaning_truncated(ui, worn.path),
        None if id != 0 => {
            ui.label(
                egui::RichText::new("no such display row")
                    .small()
                    .color(theme::BAD),
            );
        }
        None => meaning(ui, "none"),
    }
    written
}
