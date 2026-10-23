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
//! [`super::spawn::template_window`] holds `creature_template`'s 78, in a window that can be
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
//! Both forms are [`crate::ui::widgets::rowform`]'s page: the column's name in a fixed
//! cell, its value beside it, and what the value means after that, one column
//! per row, under a folding heading per group. A column that names a row of
//! another table draws the number, a `…` that chooses one by name through
//! [`crate::ui::subjects::quests::picker`], and the name the number resolves to as a link
//! ([`crate::ui::widgets::reference`]). The five display columns draw a picture of the
//! model beside the number and a `choose…` that opens [`crate::ui::subjects::displays`], a
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
//! [`crate::ui::tables`] is a browser over `DbcFile` records: a row is an index, a
//! field is a column of a fixed-width record, and an edit writes bytes. A server
//! row is a keyed map of text and an edit is a statement. Making one browser do
//! both would be a `Table` with two backends, a refactor of its own.
//!
//! ## What is here
//!
//! The panel's frame, the picker, the edits block, the claim and the template
//! window are the game object panel's too, in [`super::spawn`]. This file is
//! what the creature panel adds ([`CreatureSubject`]'s [`SpawnView`]): the
//! sections a selected creature is drawn in, its summary and its windows, the
//! waypoint mode beside the template window, the creature's pictures, and the
//! form with its display and gossip cells.

use bevy_egui::egui;

use super::spawn::{SpawnView, Text};
use crate::session::EditSession;
use crate::subjects::displays::{DisplayPick, Table as DisplayTable};
use crate::subjects::quests::ColumnTarget;
use crate::tools::spawns::creatures::{CreatureSubject, Creatures, Known, Spawn};
use crate::ui::widgets::theme;
use crate::ui::Editing;
use vale_client::assets::GameAssets;
use vale_mangos::creature::{self, Column, Group, Kind};
use vale_mangos::row::Key;

use crate::ui::shell::asks::ShellAsks;
use crate::ui::widgets::reference::{self, Resolver};
use crate::ui::widgets::rowform::{
    meaning, number_cell, page_spacing, section, server_field, FieldCtx, Hook, FORM_ROW,
};

/// Everything the creature panel needs, as one argument.
pub(crate) type Subject<'a> = super::spawn::Subject<'a, CreatureSubject>;

/// What the creature panel reaches beside the shared panel's state.
pub(crate) struct CreaturePanel<'a> {
    /// The waypoint mode, which the Waypoints button opens and shuts.
    ///
    /// Passed here rather than reached through the tool because the button
    /// that opens it is on this panel: a creature has to be selected before a
    /// path can be, and this panel is where a creature is selected.
    pub(crate) waypoints: &'a mut crate::tools::spawns::waypoints::Waypoints,
    /// The behaviour windows' state, for the Events and Spells buttons. See
    /// `crate::ui::subjects::behaviour::windows`.
    pub(crate) behaviour: &'a mut crate::subjects::behaviour::Behaviour,
    /// The Vendor and Trainer windows' state, for their two buttons. See
    /// `crate::ui::subjects::services::windows`.
    pub(crate) services: &'a mut crate::subjects::services::Services,
    /// The gossip window's state, for the Gossip button. See
    /// `crate::ui::subjects::gossip::window`.
    pub(crate) gossip: &'a mut crate::subjects::gossip::Gossip,
}

impl<'a> Subject<'a> {
    /// The panel's subject out of the shell's state. `gizmo` is whether the
    /// panel draws the Handles switch, which only the inspector does.
    pub(in crate::ui) fn new(
        session: &'a mut EditSession,
        editing: &'a mut Editing<'_>,
        server: super::Server<'a>,
        assets: &'a GameAssets,
        now: f64,
        gizmo: bool,
    ) -> Subject<'a> {
        let gizmo = match gizmo {
            true => Some(&mut *editing.gizmo),
            false => None,
        };
        Subject {
            session,
            tool: &mut editing.creatures,
            quests: &mut editing.quests,
            loot: &mut editing.loot,
            server: server.settings,
            server_panel: server.panel,
            standings: server.standings,
            assets,
            portraits: &mut editing.portraits,
            thumbnails: &mut editing.thumbnails,
            now,
            gizmo,
            asks: &mut editing.asks,
            fly: &mut editing.fly,
            own: CreaturePanel {
                waypoints: &mut editing.waypoints,
                behaviour: &mut editing.behaviour,
                services: &mut editing.services,
                gossip: &mut editing.gossip,
            },
        }
    }
}

impl SpawnView for CreatureSubject {
    type Panel<'a> = CreaturePanel<'a>;
    const STACK: crate::server::stack::Subject = crate::server::stack::Subject::Creatures;
    const SQL: &'static str = crate::server::creatures::SQL_VPATH;
    const REVERT: &'static str = crate::server::creatures::REVERT_VPATH;
    const TEXT: Text = Text {
        heading: "Creature",
        title: "Creatures",
        workspace: "Creatures",
        click: "Click a creature in the viewport to open it. The nearest ones are drawn as \
                models; everything further out is a dome.",
        models: "Draw the nearest spawns as creature models. Off, every spawn is drawn as a \
                 ring, as spawns past the model limit already are.",
        budget: "The maximum number of models drawn at once. The busiest tile on map 0 holds 288 \
                 spawns and the editor streams a 7x7 block, so drawing every one of them is \
                 several thousand creatures.",
        budget_max: 2000,
        removal: "Marked for removal. Applying deletes the row, and with it the spawn's \
                  waypoints, its addon and its game-event rows.",
        removed: "Removed and applied. The row and its rows in five dependent tables are \
                  deleted from the database; Restore reverts all of them.",
        new_template: "Create a creature_template row with an entry above any upstream entry. \
                       It starts as a level 1 friendly humanoid with no model: the server loads \
                       it, and the editor draws it as a box until display_id1 is set. The \
                       template window opens on it.",
        new_listed: "Creates a creature_template row with an entry above any upstream entry. It \
                     starts as a level 1 friendly humanoid with no model, and opens here.",
        edit: "Edit creature…",
        edit_chosen: "Open creature_template's 78 columns in a separate window. The window \
                      follows the chosen creature.",
        edit_selected: "Open creature_template's 78 columns in a separate window. The window can \
                        be moved off the panel and stay open while you work in the viewport.",
        none_chosen: "No creature chosen. Search below by name or entry, then click the ground \
                      to place it.",
        too_short: "Type at least two letters of a name, or an entry.",
        no_model: None,
        list_search: "name, subname or entry",
        no_display: "display_id1 is 0 or has no CreatureDisplayInfo.dbc row. The editor draws a \
                     spawn of it as a marker.",
        place: "Switches to the rail's Creatures tool in Place with this creature chosen. The \
                next click on the ground creates a spawn of it.",
    };

    fn subject<'a>(
        session: &'a mut EditSession,
        editing: &'a mut Editing<'_>,
        server: super::Server<'a>,
        assets: &'a GameAssets,
        now: f64,
        gizmo: bool,
    ) -> Subject<'a> {
        Subject::new(session, editing, server, assets, now, gizmo)
    }

    /// Requested on every frame the picture is drawn. Only pictures on screen
    /// pay for a render, and a request that is not repeated is dropped
    /// unserved. See `crate::preview::portraits`.
    fn picture(ui: &mut egui::Ui, subject: &mut Subject<'_>, display_id: u32, rect: egui::Rect) -> bool {
        match worn(subject, display_id) {
            Some(worn) => {
                let key = subject.portraits.want_worn(&worn);
                crate::preview::portraits::paint(ui, subject.portraits, &key, rect);
                true
            }
            None => false,
        }
    }

    /// A creature is drawn wearing its skins; the pose is the model's own,
    /// because a picture has no skeleton to animate.
    fn preview(ui: &mut egui::Ui, subject: &mut Subject<'_>, display_id: u32, side: Option<f32>) -> bool {
        let Some(worn) = worn(subject, display_id) else {
            return false;
        };
        let key = subject.portraits.want_large_worn(&worn);
        match side {
            Some(side) => crate::ui::widgets::preview::preview_pane_at(ui, subject.portraits, &key, side),
            None => crate::ui::widgets::preview::preview_pane(ui, subject.portraits, &key),
        }
        true
    }

    fn row_line(_subject: &mut Subject<'_>, known: &Known) -> String {
        known.subname.clone().filter(|text| !text.is_empty()).unwrap_or_default()
    }

    /// The subname and the levels.
    fn list_line(_subject: &mut Subject<'_>, known: &Known) -> String {
        let levels = format!("level {}", levels(known));
        match known.subname.as_deref().filter(|text| !text.is_empty()) {
            Some(subname) => format!("<{subname}> \u{b7} {levels}"),
            None => levels,
        }
    }

    fn chosen_detail(ui: &mut egui::Ui, subject: &mut Subject<'_>, chosen: &Known) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(
                egui::RichText::new(format!(
                    "level {}–{} · faction {} · display",
                    chosen.level.0, chosen.level.1, chosen.faction
                ))
                .small()
                .color(theme::INK_DIM),
            );
            let id = egui::RichText::new(chosen.display_id.to_string()).small();
            match chosen.display_id {
                0 => {
                    ui.label(id.color(theme::INK_DIM));
                }
                display => reference::display_link(ui, id, subject.asks, DisplayTable::Creature.name(), display),
            }
        });
    }

    /// Only one mode may take the click. The waypoint mode takes a press ahead
    /// of the placer (see `CreatureSubject::yields_press`), so arming Place
    /// while the waypoint mode is adding would make a click place a point
    /// instead of a creature.
    fn on_place(subject: &mut Subject<'_>) {
        subject.own.waypoints.adding = false;
    }

    fn reloaded(subject: &mut Subject<'_>) {
        subject.own.waypoints.forget();
    }

    fn beside_markers(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
        ui.checkbox(&mut subject.tool.own.show_services, "Services").on_hover_text(
            "Draw an icon over each creature within 150 yards for each service its npc_flags \
             offer: quests, vendor, trainer, flight master, innkeeper, banker and others. The \
             hover card lists them.",
        );
    }

    /// The spawn's name, the two folding sections and the buttons stay in
    /// place; only the spawn's columns scroll. The buttons are used while the
    /// form below is scrolled to any column, so they are outside both
    /// sections.
    ///
    /// The sections fold because the scrolled form takes the height they
    /// leave. With both open on a short window the form is at its minimum and
    /// the whole panel scrolls; with either shut the form has that much more.
    /// egui keeps each section's state under its id, so a section stays as it
    /// was left when another creature is selected.
    fn selected(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn, on_server: &crate::server::sync::Standing) {
        title(ui, spawn);
        let edits = &subject.session.rows.edits;
        let spawn_edited = edits.touches(creature::SPAWN, &spawn.key());
        let template_edited = edits.touches(creature::TEMPLATE, &spawn.template_key());
        section(ui, "creature-panel-spawn", "Spawn", spawn_edited, true, |ui| {
            head(ui, subject, spawn);
            super::spawn::claim(ui, subject, spawn, on_server);
        });
        section(ui, "creature-panel-template", "Creature", template_edited, true, |ui| {
            summary(ui, subject, spawn, template_edited);
        });
        ui.add_space(4.0);
        windows(ui, subject, spawn.entry, spawn.label(), Some(spawn));
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .min_scrolled_height(super::spawn::SPAWN_FORM_MIN)
            .show(ui, |ui| spawn_form(ui, subject, spawn));
    }

    fn template_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, key: &Key, row: &creature::Row) {
        groups(ui, subject, creature::TEMPLATE, key, row, &[Group::Identity, Group::Appearance]);
    }

    fn renumber_reaches() -> String {
        format!(
            "vmangos has no foreign keys, so Apply updates the references itself: the \
             row, and the {} world database columns that reference a creature by \
             entry: every spawn's id, the quest relations, npc_vendor, npc_trainer, \
             the AI events, the loot and movement templates. Restore reverts all of \
             it.\n\nNot updated: a script's datalong, the conditions table, and the \
             C++ scripts, which name entries as constants.",
            creature::TEMPLATE_REFERENCES.len()
        )
    }

    fn list_summary(ui: &mut egui::Ui, subject: &mut Subject<'_>, known: &Known) {
        egui::Grid::new("creature-data-summary").num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
            for (name, value) in [
                ("Level", levels(known)),
                ("Rank", creature::value_word(&creature::RANKS, known.rank)),
                ("Faction", known.faction.to_string()),
                ("Display", known.display_id.to_string()),
                ("NPC flags", creature::mask_words(&creature::NPC_FLAGS, known.npc_flags)),
            ] {
                ui.label(egui::RichText::new(name).color(theme::INK_DIM));
                match name == "Display" && known.display_id != 0 {
                    true => reference::display_link(
                        ui,
                        egui::RichText::new(value),
                        subject.asks,
                        DisplayTable::Creature.name(),
                        known.display_id,
                    ),
                    false => {
                        ui.label(egui::RichText::new(value).color(theme::INK));
                    }
                }
                ui.end_row();
            }
        });
    }

    fn list_windows(ui: &mut egui::Ui, subject: &mut Subject<'_>, known: &Known) {
        windows(ui, subject, known.entry, known.label(), None);
    }
}

/// The levels as a list or a summary shows them: one number when the two
/// are the same.
fn levels(known: &Known) -> String {
    match known.level.0 == known.level.1 {
        true => format!("{}", known.level.0),
        false => format!("{}\u{2013}{}", known.level.0, known.level.1),
    }
}

/// What a creature entry looks like, as the picker, the form and the preview
/// need it: its model, its skins, its hair and its geosets.
///
/// For 10,388 of `CreatureDisplayInfo`'s 10,534 rows the body texture comes
/// from the client, not from the M2, so a picture built from the path alone
/// draws magenta. `crate::subjects::displays::resolve_worn` makes that join
/// with `vale_assets`' own code, the same call `spawn_models` makes for a
/// creature in the world.
///
/// `None` for a display id the tables do not resolve. The picker draws such a
/// row with no picture rather than leaving it out.
///
/// Computed once per display id and kept in `CreatureOwn::worn`: it is a DBC
/// lookup, a string build and a `Dressing`, and a picker row asks for it on
/// every frame it is drawn.
pub(in crate::ui) fn worn(subject: &mut Subject<'_>, display_id: u32) -> Option<crate::preview::portraits::Worn> {
    worn_in(subject.tool, subject.assets, display_id)
}

/// [`worn`] for a caller that holds the creature tool's state and not the
/// whole subject.
fn worn_in(creatures: &mut Creatures, assets: &GameAssets, display_id: u32) -> Option<crate::preview::portraits::Worn> {
    let generation = assets.tables_generation();
    if creatures.own.worn_at != generation {
        creatures.own.worn.clear();
        creatures.own.worn_at = generation;
    }
    if let Some(had) = creatures.own.worn.get(&display_id) {
        return had.clone();
    }
    let found = crate::subjects::displays::resolve_worn(assets, display_id);
    creatures.own.worn.insert(display_id, found.clone());
    found
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
    super::spawn::head_buttons(ui, subject, spawn);
}

/// The `creature` row: one spawn, at one place.
fn spawn_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    theme::heading(ui, "This spawn");
    theme::note(
        ui,
        "The creature row: one spawn at one position. Drag the selected spawn in the \
         viewport to move it; a drag writes the position columns as one undo step.",
    );
    let key = spawn.key();
    let Some(row) = subject
        .tool
        .spawn_row
        .as_ref()
        .map(|held| held.row.clone())
    else {
        theme::waiting(ui, "reading the row\u{2026}");
        return;
    };
    let standing = crate::tools::spawns::spawn::Standing::of::<crate::tools::spawns::creatures::CreatureSubject>(spawn);
    super::spawn::placement(ui, subject.session, &standing, subject.now);
    groups(
        ui,
        subject,
        creature::SPAWN,
        &key,
        &row,
        &[Group::Identity, Group::Place],
    );
}

/// The Creature section: a summary of what the creature is.
///
/// A summary rather than a form: the columns a person wants to see while
/// looking at a spawn (how strong it is, what it offers) without the 78 that
/// editing it needs, with a picture of its model beside them. Editing is in
/// [`super::spawn::template_window`], behind the first button of [`windows`].
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
        egui::RichText::new("Edits here change every spawn of this entry, on every map.")
            .small()
            .color(theme::INK_FAINT),
    );

    // The few columns worth reading at a glance, from the row the map query
    // already carries, so this shows something before the whole row has been
    // fetched. The model is drawn at the panel's right edge, from the same
    // display id.
    ui.horizontal(|ui| {
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
                        "npc flags",
                        creature::mask_words(&creature::NPC_FLAGS, spawn.npc_flags),
                    ),
                ] {
                    ui.label(egui::RichText::new(name).color(theme::INK_DIM));
                    match name == "display" && spawn.display_id != 0 {
                        true => crate::ui::widgets::reference::display_link(
                            ui,
                            egui::RichText::new(value),
                            subject.asks,
                            DisplayTable::Creature.name(),
                            spawn.display_id,
                        ),
                        false => {
                            ui.label(egui::RichText::new(value).color(theme::INK));
                        }
                    }
                    ui.end_row();
                }
            });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::Vec2::splat(SUMMARY_PICTURE), egui::Sense::hover());
            match worn(subject, spawn.display_id) {
                Some(worn) => {
                    let key = subject.portraits.want_worn(&worn);
                    crate::preview::portraits::paint(ui, subject.portraits, &key, rect);
                }
                None => {
                    ui.painter().rect_filled(rect, 3.0, theme::DEAD);
                }
            }
        });
    });
}

/// The buttons that open the template window and what a creature has besides
/// its template: its path, quests, loot, events, spells and its two service
/// lists. Each is a toggle, lit while its window is open. Drawn outside the
/// folding sections, so they are reachable with both shut.
///
/// `spawn` is the selected spawn in the world. The Creatures workspace draws
/// the same row with none, and then the template window, the path and the
/// link to the workspace are left out: the form is already in the middle of
/// the screen, and a path belongs to a spawn.
pub(in crate::ui) fn windows(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    entry: u32,
    label: String,
    spawn: Option<&Spawn>,
) {
    ui.horizontal_wrapped(|ui| {
        if let Some(spawn) = spawn {
            spawn_windows(ui, subject, spawn);
        }
        // Quests is a window over the world, like the two beside it, and like
        // them it follows the selection. `crate::ui::subjects::quests` draws it, and the
        // shell keeps it on the chosen creature.
        let quests_open = subject
            .quests
            .window_for
            .as_ref()
            .is_some_and(|(holder, _, _)| *holder == crate::subjects::quests::Holder::Creature);
        if ui
            .selectable_label(quests_open, "Quests")
            .on_hover_text(
                "Quests this creature starts and ends: creature_questrelation and \
                 creature_involvedrelation rows, which can be added and removed. Each quest \
                 opens in the quest workspace. The rows reference the creature_template \
                 entry, so they apply to every spawn of it.",
            )
            .clicked()
        {
            subject.quests.window_for = match quests_open {
                true => None,
                false => Some((crate::subjects::quests::Holder::Creature, entry, label.clone())),
            };
        }
        // Loot is a window over the world that follows the selection, like
        // the quest window. It shows the rows this creature's three loot
        // columns name. See `crate::ui::subjects::loot`.
        if ui
            .selectable_label(subject.loot.open, "Loot")
            .on_hover_text(
                "The creature's drop, pickpocket and skinning loot: creature_loot_template, \
                 pickpocketing_loot_template and skinning_loot_template, by the three loot \
                 ids on its template. The rows are keyed by loot id, so they apply to every \
                 creature that shares it.",
            )
            .clicked()
        {
            subject.loot.toggle();
        }
        // Events and Spells are two windows over the world that follow the
        // selection, like the two beside them. See `crate::ui::subjects::behaviour`.
        if ui
            .selectable_label(subject.own.behaviour.events_open, "Events")
            .on_hover_text(
                "The creature's creature_ai_events: each trigger with its parameters, and \
                 the creature_ai_scripts it runs, opened in the script window. The server \
                 reads them only for a template whose ai_name is EventAI; the window can \
                 set it.",
            )
            .clicked()
        {
            subject.own.behaviour.events_open = !subject.own.behaviour.events_open;
        }
        if ui
            .selectable_label(subject.own.behaviour.spells_open, "Spells")
            .on_hover_text(
                "The creature's combat spells from creature_spells, by the template's \
                 spell_list_id: eight slots, each with a spell, chance, target, flags and \
                 timers.",
            )
            .clicked()
        {
            subject.own.behaviour.spells_open = !subject.own.behaviour.spells_open;
        }
        // Gossip is a window over the world that follows the selection, like
        // the ones beside it. See `crate::ui::subjects::gossip`.
        if ui
            .selectable_label(subject.own.gossip.open, "Gossip")
            .on_hover_text(
                "The gossip_menu named by the template's gossip_menu_id: its texts, its \
                 options and the menus they open. The window can create a menu for a \
                 creature that has none.",
            )
            .clicked()
        {
            subject.own.gossip.open = !subject.own.gossip.open;
            subject.own.gossip.menu = None;
            subject.own.gossip.trail.clear();
        }
        // Vendor and Trainer are two windows over the world that follow the
        // selection, like the four beside them. See `crate::ui::subjects::services`.
        use crate::subjects::services::Kind;
        if ui
            .selectable_label(subject.own.services.is_open(Kind::Vendor), "Vendor")
            .on_hover_text(
                "The creature's vendor items: npc_vendor by its entry, and npc_vendor_template \
                 by its vendor_id, which other creatures can share. Each item with its slot, \
                 stock and restock time.",
            )
            .clicked()
        {
            subject.own.services.toggle(Kind::Vendor);
        }
        if ui
            .selectable_label(subject.own.services.is_open(Kind::Trainer), "Trainer")
            .on_hover_text(
                "The creature's trainer spells: npc_trainer by its entry, and npc_trainer_template \
                 by its trainer_id, which other creatures can share. Each teaching spell with its \
                 required level, cost and required skill.",
            )
            .clicked()
        {
            subject.own.services.toggle(Kind::Trainer);
        }
        if spawn.is_some() {
            super::spawn::open_in_templates(ui, subject, entry);
        }
    });
}

/// The two buttons of [`windows`] that belong to a spawn in the world: the
/// template window and the path.
fn spawn_windows(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    super::spawn::edit_toggle(ui, subject);
    // Waypoints is a mode rather than a form: it takes the pointer while it
    // is open, so it is a toggle like the template window beside it, not a
    // button that opens something and returns. The toggle shows or hides the
    // window, not this creature's path. The path in the window follows the
    // selection, as the template window does. See
    // `tools::spawns::waypoints::follow_the_selection`.
    let showing = subject.own.waypoints.showing;
    if ui
        .selectable_label(showing, "Waypoints")
        .on_hover_text(
            "The creature's waypoint path from creature_movement, drawn on the ground. \
             Points can be selected, dragged, added and removed. The window follows the \
             selection and shows the path of the selected creature.",
        )
        .clicked()
    {
        match showing {
            true => subject.own.waypoints.close(),
            false => {
                subject.own.waypoints.showing = true;
                // Opened here as well as by the follow, so the window has its
                // creature on the frame the button is pressed rather than on
                // the next frame.
                subject.own.waypoints.open_for(spawn.guid, spawn.entry, spawn.movement_type);
            }
        }
    }
}

/// The side of the picture beside the summary, in points.
const SUMMARY_PICTURE: f32 = 56.0;

/// Every group of a table's columns, one page section per group: the named
/// ones open, the rest folded.
///
/// 78 columns is more than a person reads at once and the ones worth reading
/// first are not the ones that come first in the table. A group holding an
/// edit is opened whatever `open` says, so a change is never hidden under a
/// folded heading.
pub(in crate::ui) fn groups(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    table: &'static str,
    key: &Key,
    row: &creature::Row,
    open: &[Group],
) {
    page_spacing(ui);
    let mut seen: Vec<Group> = Vec::new();
    let all: &'static [Column] =
        vale_mangos::row::table_named(creature::TABLES, table).map_or(&[], |table| table.columns);
    for column in all {
        if !seen.contains(&column.group) {
            seen.push(column.group);
        }
    }
    for group in seen {
        let columns: Vec<&'static Column> = all
            .iter()
            .filter(|column| column.group == group)
            .collect();
        let edited = columns.iter().any(|column| {
            subject
                .session
                .rows.edits
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

/// One column on a row of its own, through [`server_field`]. The display
/// columns draw a picture of the model and the display picker; `gossip_menu_id`
/// has the buttons that open its menu or make a new one.
fn field(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    table: &'static str,
    key: &Key,
    row: &creature::Row,
    column: &'static Column,
) {
    let field = FieldCtx::new(ui, &subject.session.rows.edits, table, key, row, column, "Edit creature", subject.now);
    let assets = subject.assets;
    let (creatures, portraits, gossip) = (&mut *subject.tool, &mut *subject.portraits, &mut *subject.own.gossip);
    let mut resolver = Resolver {
        session: &mut *subject.session,
        assets,
        quests: &mut *subject.quests,
        asks: &mut *subject.asks,
    };
    let written = server_field(ui, &mut resolver, &field, |hook, ui, resolver, field, showing| {
        match (hook, field.kind) {
            (Hook::Cell, Kind::Ref(dbc)) if DisplayTable::of(dbc).is_some() => {
                Some(display_cell(ui, creatures, portraits, resolver.asks, assets, showing, field.target()))
            }
            (Hook::After, _) if field.table == creature::TEMPLATE && field.column.name == "gossip_menu_id" => {
                let npc_flags = resolver
                    .session
                    .rows
                    .edits
                    .shown_integer(field.table, field.key, "npc_flags", Some(field.row))
                    .map_or(0, |flags| flags.max(0) as u32);
                let menu = showing.trim().parse().unwrap_or(0);
                use vale_mangos::schema::RowValue;
                let label = format!(
                    "{} ({})",
                    field.row.text("name").unwrap_or("creature"),
                    field.row.integer("entry").unwrap_or(0)
                );
                crate::ui::subjects::gossip::menu_buttons(ui, resolver.session, gossip, field.key, menu, npc_flags, &label);
                None
            }
            _ => None,
        }
    });
    if let Some(written) = written {
        field.write(subject.session, written, subject.now);
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
    creatures: &mut Creatures,
    portraits: &mut crate::preview::portraits::Portraits,
    asks: &mut ShellAsks,
    assets: &GameAssets,
    showing: &str,
    target: ColumnTarget,
) -> Option<String> {
    let written = number_cell(ui, Kind::Unsigned, showing);
    let id: u32 = showing.trim().parse().unwrap_or(0);
    let worn = worn_in(creatures, assets, id);
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(FORM_ROW + 4.0), egui::Sense::hover());
    match worn.as_ref() {
        Some(worn) => {
            let key = portraits.want_worn(worn);
            crate::preview::portraits::paint(ui, portraits, &key, rect);
        }
        None if id != 0 => {
            ui.painter().rect_filled(rect, 3.0, theme::DEAD);
        }
        None => {}
    }
    if reference::choose_button(
        ui,
        "Choose a CreatureDisplayInfo.dbc row from model previews. Search by model \
         path or skin name. Only rows that resolve to a model are listed.",
    ) {
        creatures.display_pick = Some(DisplayPick::open(DisplayTable::Creature, target, id));
    }
    match worn {
        Some(worn) => reference::display_link(
            ui,
            egui::RichText::new(worn.path).small(),
            asks,
            DisplayTable::Creature.name(),
            id,
        ),
        None if id != 0 => {
            ui.label(
                egui::RichText::new("not in CreatureDisplayInfo.dbc")
                    .small()
                    .color(theme::BAD),
            );
        }
        None => meaning(ui, "none"),
    }
    written
}
