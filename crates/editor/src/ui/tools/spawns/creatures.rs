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
//! a button. The sidebar is one scrolled column of sections:
//!
//! ```text
//! Spawn              guid, what this project does to it, Go To, Duplicate,
//!                    Remove (super::spawn::spawn_sections)
//! Location           position, facing, height above the ground
//! Creature Template  a read-only summary of the template row, Edit
//!                    Template… and Open in Workspace
//! Related Data       the buttons that open the creature's other windows,
//!                    one row per kind
//! Addon              the spawn's creature_addon row
//! Formation, Links, Pool, Game Events
//! Spawn Row          the spawn's own 21 columns
//! ```
//!
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
//! The toolbar, the summary block, the picker, the Spawn and Location sections
//! and the template window are the game object panel's too, in [`super::spawn`]. This file is
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
use crate::ui::widgets::theme::{self, property};
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
    /// The Equipment window's state, for its button, and the selected
    /// spawn's addon row, for the Addon section. See
    /// `crate::ui::subjects::addons::windows` and [`addon_form`].
    pub(crate) addons: &'a mut crate::subjects::addons::Addons,
    /// The Formation and Links sections' state. See [`spawn_data`].
    pub(crate) formations: &'a mut crate::subjects::formations::Formations,
    /// The Pool section's and window's state.
    pub(crate) pools: &'a mut crate::subjects::pools::Pools,
    /// The Game Events section's and window's state.
    pub(crate) game_events: &'a mut crate::subjects::game_events::GameEvents,
    /// Where the spawns the Pool and Game Event windows list stand.
    pub(crate) located: &'a mut crate::subjects::located::Locations,
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
            others: &mut editing.others,
            own: CreaturePanel {
                waypoints: &mut editing.waypoints,
                behaviour: &mut editing.behaviour,
                services: &mut editing.services,
                gossip: &mut editing.gossip,
                addons: &mut editing.addons,
                formations: &mut editing.formations,
                pools: &mut editing.pools,
                game_events: &mut editing.game_events,
                located: &mut editing.located,
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
        heading: "Creature Template",
        title: "Creatures",
        workspace: "Creatures",
        click: "Click a creature in the viewport to show its properties here. The nearest ones \
                are drawn as models; everything further out is a dome.",
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
        edit: "Edit Template…",
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
        let on = subject.tool.own.show_services;
        if super::spawn::toggle(ui, on, "Services")
            .on_hover_text(
                "Draw an icon over each creature within 150 yards for each service its npc_flags \
                 offer: quests, vendor, trainer, flight master, innkeeper, banker and others. The \
                 hover card lists them.",
            )
            .clicked()
        {
            subject.tool.own.show_services = !on;
        }
    }

    /// The sections in the order the module comment lists them, in the
    /// inspector's one scroll.
    fn selected(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn, on_server: &crate::server::sync::Standing) {
        let subname = spawn.subname.as_deref().filter(|s| !s.is_empty()).map(|s| format!("<{s}>"));
        super::spawn::title(ui, &spawn.label(), subname.as_deref());
        super::spawn::spawn_sections(ui, subject, spawn, on_server);
        summary(ui, subject, spawn);
        theme::heading(ui, "Related Data");
        windows(ui, subject, spawn.entry, spawn.label(), Some(spawn));
        spawn_form(ui, subject, spawn);
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

/// The spawn's own rows: its addon, the rows of other tables that name it,
/// and then every column of its `creature` row.
fn spawn_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    // The addon row before the spawn's own columns: it is what makes this
    // spawn differ from the others of its template, and the columns below
    // are mostly its key and its position, which the Location section
    // already edits. Place is folded for the same reason.
    addon_form(ui, subject, spawn);
    spawn_data(ui, subject);
    super::spawn::row_heading::<CreatureSubject>(ui);
    let key = spawn.key();
    let Some(row) = subject
        .tool
        .spawn_row
        .as_ref()
        .map(|held| held.row.clone())
    else {
        theme::waiting(ui, "Loading the row\u{2026}");
        return;
    };
    groups(ui, subject, creature::SPAWN, &key, &row, &[Group::Identity]);
}

/// The Addon section of the spawn form, drawn before the spawn's own
/// columns: the spawn's `creature_addon` row, which overrides the template's
/// display, mount, equipment set and auras for this one spawn and says how
/// it stands. A checkbox makes the row or removes it; its columns follow,
/// one per row of the page layout. See [`crate::subjects::addons`].
fn addon_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    use vale_mangos::addon;
    use vale_mangos::row::Life;
    theme::heading(ui, "Addon");
    theme::note(
        ui,
        "The creature_addon row: this spawn's own display, mount, weapons, stance, emote \
         and auras, over the template's. The server reads it at startup; a change needs a \
         restart.",
    );
    let guid = spawn.guid;
    if let Some(trouble) = subject.own.addons.trouble().cloned() {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
        return;
    }
    if !subject.own.addons.is_addon_read(guid) {
        theme::waiting(ui, "Loading the addon row\u{2026}");
        return;
    }
    let edits = subject.session.rows.edits.clone();
    let shown = subject.own.addons.addon_of(&edits, guid);
    let live = shown.as_ref().filter(|(_, _, life)| *life != Life::Delete);
    let mut on = live.is_some();
    if ui
        .checkbox(&mut on, "Has an addon row")
        .on_hover_text(
            "Whether this spawn has a row. A new row changes nothing until a column is set: \
             the template's display, mount, set and auras, standing, melee weapon in hand, no \
             emote. Unticking removes the row, and the spawn stands as every other spawn of \
             the template.",
        )
        .changed()
    {
        match on {
            true => subject.own.addons.add_addon(subject.session, guid, subject.now),
            false => subject.own.addons.remove_addon(subject.session, guid, subject.now),
        }
        return;
    }
    let Some((addon, _, _)) = live.cloned() else {
        return;
    };
    let (key, held) = subject.own.addons.addon_held(guid);
    ui.scope(|ui| {
        page_spacing(ui);
        for column in addon::ADDON_COLUMNS.iter().filter(|column| column.kind != Kind::Key) {
            addon_field(ui, subject, guid, &key, &held, column);
        }
    });
    for fault in addon.check() {
        ui.label(egui::RichText::new(fault).size(theme::SMALL).color(theme::WARN));
    }
}

/// The Formation, Links, Pool and Game Events sections: the rows of other
/// tables that name the selected spawn, acting on the whole selection. See
/// `crate::ui::subjects::keyed`. A spawn a section asks to select is
/// selected once all four are drawn.
fn spawn_data(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    use crate::ui::subjects::keyed::{Chosen, Section};
    let patch = crate::server::settings::server_patch(subject.server);
    let edits = subject.session.rows.edits.clone();
    let map = subject.session.map_id;
    let select = {
        let Some(chosen) = Chosen::of(&*subject.tool, edits, map, false) else { return };
        let mut section = Section {
            session: &mut *subject.session,
            quests: &mut *subject.quests,
            asks: &mut *subject.asks,
            assets: subject.assets,
            now: subject.now,
            located: &mut *subject.own.located,
            chosen: &chosen,
            select: None,
        };
        crate::ui::subjects::formations::formation_section(ui, &mut section, subject.own.formations);
        crate::ui::subjects::formations::links_section(ui, &mut section, subject.own.formations);
        crate::ui::subjects::pools::pool_section(ui, &mut section, subject.own.pools, patch);
        crate::ui::subjects::game_events::events_section(ui, &mut section, subject.own.game_events, patch);
        section.select
    };
    if let Some(guid) = select {
        subject.tool.select_only(Some(guid));
    }
}

/// One column of the addon row, through [`server_field`]. The display column
/// draws its picture and picker as the template's do, and the mount and
/// equipment columns draw their -1 as the template's own, with a button to
/// override it and one to go back; an equipment id of its own opens the
/// Equipment window on that set.
fn addon_field(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    guid: u64,
    key: &Key,
    row: &creature::Row,
    column: &'static Column,
) {
    use vale_mangos::addon;
    let field = FieldCtx::new(ui, &subject.session.rows.edits, addon::ADDON, key, row, column, "Edit creature addon", subject.now);
    let assets = subject.assets;
    let (creatures, portraits, addons) = (&mut *subject.tool, &mut *subject.portraits, &mut *subject.own.addons);
    let mut resolver = Resolver {
        session: &mut *subject.session,
        assets,
        quests: &mut *subject.quests,
        asks: &mut *subject.asks,
    };
    let written = server_field(ui, &mut resolver, &field, |hook, ui, resolver, field, showing| {
        match (hook, field.column.name) {
            (Hook::Cell, "display_id") => Some(display_cell(
                ui,
                creatures,
                portraits,
                resolver.asks,
                assets,
                showing,
                field.target(),
                |_| {},
            )),
            (Hook::After, "display_id") if showing.trim() == "0" => {
                ui.label(egui::RichText::new("template's").small().color(theme::INK_DIM))
                    .on_hover_text("0: the spawn is drawn as the template's display.");
                None
            }
            (Hook::Cell, "mount_display_id" | "equipment_id") => {
                let value: i64 = showing.trim().parse().unwrap_or(-1);
                let (own, none) = match field.column.name {
                    "mount_display_id" => ("the template's mount", "no mount"),
                    _ => ("the template's set", "no weapons"),
                };
                if value < 0 {
                    ui.label(egui::RichText::new(own).small().color(theme::INK_DIM));
                    return Some(
                        ui.small_button("override")
                            .on_hover_text(format!("Give this spawn its own value; 0 is {none}."))
                            .clicked()
                            .then(|| "0".to_string()),
                    );
                }
                // The button that goes back to the template's value comes
                // before the model's path, which is truncated to the room
                // left and would otherwise push the button off the form.
                let mut reset = false;
                let mut back = |ui: &mut egui::Ui| {
                    reset |= ui
                        .small_button("template's")
                        .on_hover_text(format!("Write -1: {own}."))
                        .clicked();
                };
                let written = match field.column.name {
                    "mount_display_id" => display_cell(
                        ui,
                        creatures,
                        portraits,
                        resolver.asks,
                        assets,
                        showing,
                        field.target(),
                        back,
                    ),
                    _ => {
                        let written = number_cell(ui, Kind::Unsigned, showing);
                        if value == 0 {
                            ui.label(egui::RichText::new(none).small().color(theme::INK_DIM));
                        } else if ui
                            .small_button("open\u{2026}")
                            .on_hover_text("Open the Equipment window on this set.")
                            .clicked()
                        {
                            addons.equipment_open = true;
                        }
                        back(ui);
                        written
                    }
                };
                Some(match reset {
                    true => Some("-1".to_string()),
                    false => written,
                })
            }
            _ => None,
        }
    });
    if let Some(written) = written {
        subject.own.addons.set_addon_column(subject.session, guid, column.name, written, subject.now);
    }
}

/// The Creature Template section: a summary of what the creature is, with a
/// picture of its model, and the buttons that open the template window and
/// the workspace.
///
/// A summary rather than a form: the columns a person wants to see while
/// looking at a spawn (how strong it is, what it offers) without the 78 that
/// editing it needs. Editing is in [`super::spawn::template_window`].
///
/// The note under the heading states the template's scope, which is what
/// tells an edit here from an edit to the spawn. It is in the warning colour
/// while this project changes the row.
fn summary(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    let edited = subject.session.rows.edits.touches(creature::TEMPLATE, &spawn.template_key());
    theme::heading(ui, CreatureSubject::TEXT.heading);
    let scope = format!("Changes apply to every spawn of entry {}, on every map.", spawn.entry);
    match edited {
        true => {
            ui.label(egui::RichText::new(format!("Edited. {scope}")).size(theme::SMALL).color(theme::WARN));
        }
        false => theme::note(ui, scope),
    }

    // The few columns worth reading at a glance, from the row the map query
    // already carries, so this shows something before the whole row has been
    // fetched. The model is drawn at the panel's right edge, from the same
    // display id.
    ui.horizontal(|ui| {
        let width = (ui.available_width() - SUMMARY_PICTURE - 8.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_max_width(width);
            property(ui, "Entry", "The creature_template entry.", |ui| {
                super::spawn::value(ui, spawn.entry.to_string());
            });
            if spawn.patch != 0 {
                property(ui, "Content Patch", "The patch column of the template row the server uses.", |ui| {
                    super::spawn::value(ui, spawn.patch.to_string());
                });
            }
            property(ui, "Level", "", |ui| {
                super::spawn::value(ui, levels_of(spawn.level));
            });
            property(ui, "Rank", "", |ui| {
                super::spawn::value(ui, creature::value_word(&creature::RANKS, spawn.rank));
            });
            property(ui, "Faction", "A FactionTemplate.dbc row.", |ui| {
                super::spawn::value(ui, spawn.faction.to_string());
            });
            property(ui, "Display", "display_id1, a CreatureDisplayInfo.dbc row.", |ui| match spawn.display_id {
                0 => {
                    super::spawn::value_in(ui, "None", theme::INK_DIM);
                }
                display => reference::display_link(
                    ui,
                    egui::RichText::new(display.to_string()),
                    subject.asks,
                    DisplayTable::Creature.name(),
                    display,
                ),
            });
            property(ui, "NPC Flags", "What the creature offers: npc_flags.", |ui| {
                super::spawn::value(ui, creature::mask_words(&creature::NPC_FLAGS, spawn.npc_flags));
            });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(SUMMARY_PICTURE), egui::Sense::hover());
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
    super::spawn::template_buttons(ui, subject, spawn.entry);
}

/// A creature's levels: one number when the two are the same.
fn levels_of((low, high): (u32, u32)) -> String {
    match low == high {
        true => format!("{low}"),
        false => format!("{low}\u{2013}{high}"),
    }
}

/// The buttons that open what a creature has besides its template, one
/// property row per kind: its path, its AI, what a player can do with it,
/// what it sells and teaches, and its items. Each is a toggle, lit while its
/// window is open, and each window follows the selection.
///
/// `spawn` is the selected spawn in the world. The Creatures workspace draws
/// the same rows with none, and then the path is left out, because a path
/// belongs to a spawn.
pub(in crate::ui) fn windows(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    entry: u32,
    label: String,
    spawn: Option<&Spawn>,
) {
    if let Some(spawn) = spawn {
        property(ui, "Movement", "The spawn's waypoint path.", |ui| waypoints_button(ui, subject, spawn));
    }
    property(ui, "AI", "The creature's scripted behaviour and combat spells.", |ui| {
        // Events and Spells: see `crate::ui::subjects::behaviour`.
        if ui
            .add(egui::Button::new("Events").selected(subject.own.behaviour.events_open))
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
            .add(egui::Button::new("Spells").selected(subject.own.behaviour.spells_open))
            .on_hover_text(
                "The creature's combat spells from creature_spells, by the template's \
                 spell_list_id: eight slots, each with a spell, chance, target, flags and \
                 timers.",
            )
            .clicked()
        {
            subject.own.behaviour.spells_open = !subject.own.behaviour.spells_open;
        }
    });
    property(ui, "Interaction", "What a player can do when talking to the creature.", |ui| {
        // Quests: `crate::ui::subjects::quests` draws the window, and the
        // shell keeps it on the chosen creature.
        let quests_open = subject
            .quests
            .window_for
            .as_ref()
            .is_some_and(|(holder, _, _)| *holder == crate::subjects::quests::Holder::Creature);
        if ui
            .add(egui::Button::new("Quests").selected(quests_open))
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
        // Gossip: see `crate::ui::subjects::gossip`.
        if ui
            .add(egui::Button::new("Gossip").selected(subject.own.gossip.open))
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
    });
    property(ui, "Services", "What the creature sells and teaches.", |ui| {
        // Vendor and Trainer: see `crate::ui::subjects::services`.
        use crate::subjects::services::Kind;
        if ui
            .add(egui::Button::new("Vendor").selected(subject.own.services.is_open(Kind::Vendor)))
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
            .add(egui::Button::new("Trainer").selected(subject.own.services.is_open(Kind::Trainer)))
            .on_hover_text(
                "The creature's trainer spells: npc_trainer by its entry, and npc_trainer_template \
                 by its trainer_id, which other creatures can share. Each teaching spell with its \
                 required level, cost and required skill.",
            )
            .clicked()
        {
            subject.own.services.toggle(Kind::Trainer);
        }
    });
    property(ui, "Items", "What the creature drops and holds.", |ui| {
        // Loot: the rows this creature's three loot columns name. See
        // `crate::ui::subjects::loot`.
        if ui
            .add(egui::Button::new("Loot").selected(subject.loot.open))
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
        // Equipment: see `crate::ui::subjects::addons`.
        if ui
            .add(egui::Button::new("Equipment").selected(subject.own.addons.equipment_open))
            .on_hover_text(
                "The weapons the creature holds: the creature_equip_template set its \
                 template's equipment_id names, or the one a selected spawn's addon row names. \
                 Each row is three items and a probability, and a spawn rolls one row.",
            )
            .clicked()
        {
            subject.own.addons.equipment_open = !subject.own.addons.equipment_open;
        }
    });
}

/// The Waypoints toggle.
///
/// Waypoints is a mode rather than a form: it takes the pointer while it is
/// open, so it is a toggle like the template window, not a button that opens
/// something and returns. The toggle shows or hides the window, not this
/// creature's path. The path in the window follows the selection, as the
/// template window does. See `tools::spawns::waypoints::follow_the_selection`.
fn waypoints_button(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    let showing = subject.own.waypoints.showing;
    if ui
        .add(egui::Button::new("Waypoints").selected(showing))
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
            (Hook::Cell, Kind::Ref(dbc)) if DisplayTable::of(dbc).is_some() => Some(display_cell(
                ui,
                creatures,
                portraits,
                resolver.asks,
                assets,
                showing,
                field.target(),
                |_| {},
            )),
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
/// that opens the picker, whatever `before_path` draws, and the model's path.
/// The path comes last because it is truncated to the room that is left.
///
/// The number stays editable beside the picture, because a user often has the
/// id already, for example from a wiki. The picture is the same rendered
/// portrait the Place picker's rows draw.
#[allow(clippy::too_many_arguments)]
fn display_cell(
    ui: &mut egui::Ui,
    creatures: &mut Creatures,
    portraits: &mut crate::preview::portraits::Portraits,
    asks: &mut ShellAsks,
    assets: &GameAssets,
    showing: &str,
    target: ColumnTarget,
    before_path: impl FnOnce(&mut egui::Ui),
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
    before_path(ui);
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
