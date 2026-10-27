//! What the spawn-data sections share: the spawns they act on and name, and
//! a keyed row's columns drawn through the form, for the Formation, Links,
//! Pool and Game Events sections of the two spawn inspectors and the Pool
//! and Game Event windows. The view of [`crate::subjects::keyed`].
//!
//! A section acts on the selection: the primary spawn, which the inspector
//! shows, and the spawns shift-clicked beside it. A row it lists names a
//! spawn by its template's name and its guid, and the spawn's own button
//! selects it; a spawn that is not on the open map is named by its guid.

use crate::session::EditSession;
use crate::subjects::formations::Placed;
use crate::subjects::located::Locations;
use crate::subjects::quests::Quests;
use crate::tools::spawns::spawn::{SpawnRef, SpawnSubject, SpawnTool, Spawned};
use crate::ui::shell::asks::ShellAsks;
use crate::ui::widgets::reference::Resolver;
use crate::ui::widgets::rowform::{server_field, FieldCtx, Hook};
use crate::ui::widgets::theme;
use bevy::prelude::Vec3;
use bevy_egui::egui;
use vale_client::assets::GameAssets;
use vale_mangos::row::{Assignment, Edits, Key, Life};
use vale_mangos::schema::{Column, Row};

/// A spawn as a section names and places it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Brief {
    pub(crate) guid: u64,
    pub(crate) entry: u32,
    /// The template's name and entry: `Stormwind City Guard (68)`.
    pub(crate) label: String,
    /// Where it stands, in the world's own axes.
    pub(crate) at: Vec3,
    /// The radians it faces.
    pub(crate) facing: f32,
}

impl Brief {
    pub(crate) fn of<P: Spawned>(spawn: &P) -> Brief {
        Brief {
            guid: spawn.guid(),
            entry: spawn.entry(),
            label: spawn.label(),
            at: spawn.at(),
            facing: spawn.orientation(),
        }
    }

    pub(crate) fn placed(&self) -> Placed {
        Placed { guid: self.guid, at: self.at, facing: self.facing }
    }
}

/// The spawns a section acts on: the primary and the others selected
/// beside it, as the project's edits leave them, and every spawn of the open
/// map by guid, for the rows a section lists.
pub(crate) struct Chosen<'a> {
    pub(crate) primary: Brief,
    pub(crate) others: Vec<Brief>,
    /// The open map's id, for a template link.
    pub(crate) map: u32,
    /// Whether the spawns are game objects, for the rows a section writes.
    pub(crate) objects: bool,
    /// Any spawn of the map by guid, as the project's edits leave it.
    every: Box<dyn Fn(u64) -> Option<Brief> + 'a>,
}

impl<'a> Chosen<'a> {
    /// The selection of `tool`, or `None` with nothing selected. The tool is
    /// borrowed and `edits` kept, so the sections can hold the session
    /// mutably while they draw.
    pub(crate) fn of<S: SpawnSubject>(tool: &'a SpawnTool<S>, edits: Edits, map: u32, objects: bool) -> Option<Chosen<'a>> {
        let brief = move |guid: u64| tool.edited(guid, Some(&edits)).map(|spawn| Brief::of(&spawn));
        let primary = brief(tool.selected?)?;
        let others = tool.also.iter().filter_map(|guid| brief(*guid)).collect();
        Some(Chosen { primary, others, map, objects, every: Box::new(brief) })
    }

    /// A spawn a list names, or `None` for one not on the map.
    pub(crate) fn find(&self, guid: u64) -> Option<Brief> {
        (self.every)(guid)
    }

    /// How a list names a spawn: its template and guid, or its guid alone
    /// when it is not on the open map.
    pub(crate) fn name(&self, guid: u64) -> String {
        match self.find(guid) {
            Some(brief) => format!("{} \u{b7} guid {guid}", brief.label),
            None => format!("guid {guid} \u{b7} not on this map"),
        }
    }

    /// The guids of the selected spawns other than the primary.
    pub(crate) fn other_guids(&self) -> Vec<u64> {
        self.others.iter().map(|other| other.guid).collect()
    }

    /// The guids of every selected spawn, the primary first.
    pub(crate) fn all_guids(&self) -> Vec<u64> {
        std::iter::once(self.primary.guid).chain(self.other_guids()).collect()
    }
}

/// What a view of a pool or an event draws with, in a window or in the
/// Spawning workspace: the session, the reference picker and its names, the
/// shell's asks, and where the spawns it lists stand.
pub(crate) struct View<'a> {
    pub(crate) session: &'a mut EditSession,
    pub(crate) quests: &'a mut Quests,
    pub(crate) asks: &'a mut ShellAsks,
    pub(crate) assets: &'a GameAssets,
    pub(crate) now: f64,
    pub(crate) located: &'a mut Locations,
}

impl View<'_> {
    /// The form's resolver over the view's state.
    pub(crate) fn resolver(&mut self) -> Resolver<'_> {
        Resolver { session: &mut *self.session, assets: self.assets, quests: &mut *self.quests, asks: &mut *self.asks }
    }

    /// The Go To button of a listed spawn: select it under its tool, on its
    /// map. Unavailable while its place is being read, and for a guid no
    /// spawn row holds.
    pub(crate) fn go_to_button(&mut self, ui: &mut egui::Ui, spawn: SpawnRef) {
        let known = self.located.find(spawn).is_some_and(|place| place.is_some());
        if ui
            .add_enabled(known, egui::Button::new("Go To").small())
            .on_hover_text(format!(
                "Select {} spawn {} and put the camera over it, on its map.",
                spawn.kind.word(),
                spawn.guid
            ))
            .on_disabled_hover_text("Not in the database, or still being read.")
            .clicked()
        {
            let ask = self.located.go_to(spawn);
            self.asks.push(ask);
        }
    }

    /// How a list names a spawn: its template and guid, and its map when it
    /// is not the open one.
    pub(crate) fn spawn_name(&mut self, spawn: SpawnRef) -> String {
        let open = self.session.map_id;
        match self.located.find(spawn) {
            None => format!("{} {}\u{2026}", spawn.kind.word(), spawn.guid),
            Some(None) => format!("{} {} \u{b7} not in the database", spawn.kind.word(), spawn.guid),
            Some(Some(place)) if place.map == open => format!("{} \u{b7} guid {}", place.label(), spawn.guid),
            Some(Some(place)) => format!("{} \u{b7} guid {} \u{b7} map {}", place.label(), spawn.guid, place.map),
        }
    }
}

/// Everything a section draws with, as one argument.
pub(crate) struct Section<'a> {
    pub(crate) session: &'a mut EditSession,
    pub(crate) quests: &'a mut Quests,
    pub(crate) asks: &'a mut ShellAsks,
    pub(crate) assets: &'a GameAssets,
    pub(crate) now: f64,
    pub(crate) located: &'a mut Locations,
    pub(crate) chosen: &'a Chosen<'a>,
    /// A spawn a section asked to select, which the panel selects once the
    /// sections are drawn.
    pub(crate) select: Option<u64>,
}

impl Section<'_> {
    /// The section's state as a [`View`], for a column, a window or a list.
    pub(crate) fn view(&mut self) -> View<'_> {
        View {
            session: &mut *self.session,
            quests: &mut *self.quests,
            asks: &mut *self.asks,
            assets: self.assets,
            now: self.now,
            located: &mut *self.located,
        }
    }

    /// The button that selects a listed spawn on the open map.
    pub(crate) fn select_button(&mut self, ui: &mut egui::Ui, guid: u64) {
        if self.chosen.find(guid).is_some()
            && guid != self.chosen.primary.guid
            && ui.small_button("Select").on_hover_text(format!("Select spawn {guid}.")).clicked()
        {
            self.select = Some(guid);
        }
    }
}

/// How wide a listed spawn's name is in a grid of spawns, in points. A
/// truncating label in a grid cell takes the narrowest width the column
/// allows, so the cell is given one.
const NAME_CELL: f32 = 340.0;

/// A listed spawn's name as one grid cell, cut to [`NAME_CELL`] with the
/// whole of it on hover.
pub(crate) fn name_cell(ui: &mut egui::Ui, name: String) {
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        egui::vec2(NAME_CELL, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_width(NAME_CELL);
            ui.add(egui::Label::new(egui::RichText::new(name).small()).truncate());
        },
    );
}

/// The database's row under `key`, as the form reads one: the key's
/// columns and `held`'s, unquoted. Empty but for the key when the database
/// holds no row, which is a row the project creates.
pub(crate) fn database_row(key: &Key, held: Option<&[Assignment]>) -> Row {
    let mut row = Row::new();
    for (column, value) in &key.0 {
        row.insert(column.clone(), Some(value.clone()));
    }
    for change in held.into_iter().flatten() {
        row.insert(change.column.to_string(), Some(vale_mangos::sql::unquote(&change.value)));
    }
    row
}

/// A keyed row's columns, one form row each, through [`server_field`]:
/// every column of `columns` that is not part of the key. `hook` draws a
/// column's cell or what follows it, as [`server_field`]'s does. Answers the
/// column and literal a cell wrote; the caller writes it with its subject's
/// writer.
#[allow(clippy::too_many_arguments)]
pub(in crate::ui) fn columns(
    ui: &mut egui::Ui,
    view: &mut View<'_>,
    table: &'static str,
    key: &Key,
    held: Option<&[Assignment]>,
    columns: &'static [Column],
    label: &'static str,
    mut hook: impl FnMut(Hook, &mut egui::Ui, &FieldCtx<'_>, &str) -> Option<Option<String>>,
) -> Option<(&'static str, String)> {
    let row = database_row(key, held);
    let edits = view.session.rows.edits.clone();
    let now = view.now;
    let mut written = None;
    ui.scope(|ui| {
        crate::ui::widgets::rowform::page_spacing(ui);
        for column in columns.iter().filter(|column| column.kind != vale_mangos::schema::Kind::Key) {
            let field = FieldCtx::new(ui, &edits, table, key, &row, column, label, now);
            let mut resolver = view.resolver();
            if let Some(value) = server_field(ui, &mut resolver, &field, |hook_of, ui, _, field, showing| {
                hook(hook_of, ui, field, showing)
            }) {
                written = Some((column.name, value));
            }
        }
    });
    written
}

/// The state dot a listed row starts with: amber for a row the project
/// creates, red for one it removes or one the server would skip, with every
/// reason on hover; nothing for a database row the project keeps.
pub(crate) fn state_dot(ui: &mut egui::Ui, life: Life, faults: &[String]) {
    super::loot::state_dot(ui, life, faults);
}

/// How much of a listed row the remove button keeps, in points.
const REMOVE_ROOM: f32 = 76.0;

/// One listed row: a header line of the fold arrow, the state dot, the
/// title and the button that removes the row or keeps one marked for
/// removal, and `body` under it while it is unfolded. The title is cut to
/// the room the button leaves, with the whole of it on hover. Answers what
/// the button asked, for the caller to act on once the row is drawn.
pub(in crate::ui) fn listed_row(
    ui: &mut egui::Ui,
    id: impl egui::AsIdSalt,
    life: Life,
    faults: &[String],
    title: String,
    remove_about: &str,
    body: impl FnOnce(&mut egui::Ui),
) -> Option<crate::ui::widgets::rowform::RowAct> {
    let mut act = None;
    let id = ui.make_persistent_id(id);
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
        .show_header(ui, |ui| {
            state_dot(ui, life, faults);
            let room = (ui.available_width() - REMOVE_ROOM).max(80.0);
            ui.allocate_ui_with_layout(
                egui::vec2(room, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_max_width(room);
                    ui.add(egui::Label::new(egui::RichText::new(title).small()).truncate());
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                act = crate::ui::widgets::rowform::remove_keep(ui, life, remove_about);
            });
        })
        .body(body);
    act
}

/// Take the removal mark off a row.
pub(in crate::ui) fn keep_row(view: &mut View<'_>, table: &'static str, key: &Key) {
    let label = format!("{table} {}", key.text());
    let gesture = crate::server::store::Gesture { label: "Cancel removal", subject: &label, now: view.now };
    view.session.set_server_row(table, key, None, Some(gesture));
}

/// A section's empty line or waiting line: the read's trouble, a waiting
/// note while the tables are read, or `None` when the tables are in hand.
pub(crate) fn not_ready(ui: &mut egui::Ui, trouble: Option<&String>, read: bool, what: &str) -> bool {
    if let Some(trouble) = trouble {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
        return true;
    }
    if !read {
        theme::waiting(ui, format!("reading {what}\u{2026}"));
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The database's row carries the key and the held columns unquoted.
    #[test]
    fn a_database_row_reads_as_the_form_reads_one() {
        let key = Key::one("guid", 7);
        let held = vec![Assignment { column: "description", value: vale_mangos::sql::text("Copper Vein") }];
        let row = database_row(&key, Some(&held));
        assert_eq!(row.get("guid").cloned().flatten().as_deref(), Some("7"));
        assert_eq!(row.get("description").cloned().flatten().as_deref(), Some("Copper Vein"));
        assert_eq!(database_row(&key, None).len(), 1);
    }
}
