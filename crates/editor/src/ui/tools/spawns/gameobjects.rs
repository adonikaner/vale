//! The game-object tool's panel: one spawn, the template behind it, and what an
//! edit to either means.
//!
//! The layout follows [`super::creatures`], for the same reason. The spawn's 19
//! columns are in the sidebar because they describe one place in the world,
//! and they scroll under a summary of the template and its buttons, which stay
//! in place.
//! `gameobject_template`'s 34 columns are in a window opened by the Edit
//! object… button because they apply to every object of that kind. Each form
//! states which of the two it is before its first field, and both are
//! [`crate::ui::widgets::rowform`]'s page: one column per row under a folding heading per
//! group.
//!
//! ## Differences from the creature form
//!
//! * The 24 `data` columns are drawn under the names the row's type gives
//!   them: `lockId`, `lootId` and `chestRestockTime` on a chest, `slots` and
//!   `height` on a chair. The column's own name is drawn beside each, because
//!   the SQL and the server's log use the column name. The names follow the
//!   type the form currently shows, so an edit to `type` renames them on the
//!   next frame. Columns the type does not read are folded under their own
//!   heading. See `vale_mangos::gameobject::data_fields`.
//! * A lock is resolved through `Lock.dbc`, which is a client file. A chest
//!   whose lock is keyed on Mining or Herbalism is a gathering node, and the
//!   form and the heading name the skill and the rank. The server's row does
//!   not carry this.
//! * `displayId` names a `GameObjectDisplayInfo` row, which is a model path
//!   and nothing else, so its picker ([`crate::ui::subjects::displays`]) is a grid of every
//!   row's model, searched by path.
//! * An edit to `orientation` also writes the rotation quaternion on an upright
//!   spawn. On a tilted spawn the form states that it does not. See
//!   `crate::tools::spawns::gameobjects::GameObjects::turn`.
//!
//! ## What is here
//!
//! The panel's frame, the picker, the edits block, the claim and the template
//! window are the creature panel's too, in [`super::spawn`]. This file is what
//! the game object panel adds ([`GameObjectSubject`]'s [`SpawnView`]): the
//! type filter in the picker, the lock and the profession a gathering node
//! needs, the summary and the quest and loot windows, and the form with its
//! `data` columns named by type.
//!
//! Apply, Put back and Discard are on the top bar's Server… panel, in the
//! Objects block. See [`crate::ui::shell::sync`].

use bevy_egui::egui;

use super::spawn::{SpawnView, Text};
use crate::session::EditSession;
use crate::subjects::displays::{DisplayPick, Table as DisplayTable};
use crate::subjects::quests::{ColumnTarget, Holder};
use crate::tools::spawns::gameobjects::{GameObjectSubject, GameObjects, Known, Spawn};
use crate::ui::widgets::theme;
use vale_assets::tables::lock::{KeyKind, Locks};
use vale_client::assets::GameAssets;
use vale_mangos::gameobject::{self, Column, Group, Kind};
use vale_mangos::row::Key;

use crate::ui::shell::asks::ShellAsks;
use crate::ui::widgets::reference::{self, Resolver};
use crate::ui::widgets::rowform::{
    meaning, meaning_truncated, number_cell, page_spacing, section as page_section, server_field, FieldCtx, Hook,
    FORM_ROW,
};
use crate::ui::Editing;

/// Everything the game object panel needs, as one argument.
pub(crate) type Subject<'a> = super::spawn::Subject<'a, GameObjectSubject>;

impl<'a> Subject<'a> {
    /// The panel's subject out of the shell's state; see
    /// `super::creatures::Subject::new`.
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
            tool: &mut editing.objects,
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
            own: GameObjectPanel {
                pools: &mut editing.pools,
                game_events: &mut editing.game_events,
                located: &mut editing.located,
            },
        }
    }
}

/// What the game object panel reaches beside the shared panel's state: the
/// Pool and Game Events sections' state. See [`spawn_data`].
pub(crate) struct GameObjectPanel<'a> {
    pub(crate) pools: &'a mut crate::subjects::pools::Pools,
    pub(crate) game_events: &'a mut crate::subjects::game_events::GameEvents,
    pub(crate) located: &'a mut crate::subjects::located::Locations,
}

impl SpawnView for GameObjectSubject {
    type Panel<'a> = GameObjectPanel<'a>;
    const STACK: crate::server::stack::Subject = crate::server::stack::Subject::GameObjects;
    const SQL: &'static str = crate::server::gameobjects::SQL_VPATH;
    const REVERT: &'static str = crate::server::gameobjects::REVERT_VPATH;
    const TEXT: Text = Text {
        heading: "Game object",
        title: "Game objects",
        workspace: "Game Objects",
        click: "Click a game object in the viewport to select it. The nearest spawns are drawn \
                as models; the rest are drawn as squares on the ground.",
        models: "Draws the nearest spawns with their models. Off, every spawn is drawn as a \
                 square; spawns past the model limit are always squares.",
        budget: "Maximum number of spawns drawn as models, nearest first.",
        budget_max: 3000,
        removal: "Marked for removal. Apply deletes the gameobject row and the spawn's \
                  game_event_gameobject and gameobject_battleground rows.",
        removed: "Removed and applied. The gameobject row and its game_event_gameobject and \
                  gameobject_battleground rows are deleted; Restore reverts them.",
        new_template: "Creates a gameobject_template row with an entry from 2,000,000 up, clear \
                       of upstream vmangos entries. It starts as a Generic object of size 1 with \
                       no model, drawn as a marker until displayId is set. Opens the template \
                       window on the new row.",
        new_listed: "Creates a gameobject_template row with an entry from 2,000,000 up, clear of \
                     upstream vmangos entries. It starts as a Generic object of size 1 with no \
                     model, and opens here.",
        edit: "Edit object…",
        edit_chosen: "Opens the gameobject_template columns in a separate window. The window \
                      shows the chosen object.",
        edit_selected: "Opens the 34 gameobject_template columns in a separate window. The 24 \
                        data columns are labelled with the field names of this object's type.",
        none_chosen: "No object chosen. Search below by name or entry, or choose a type, then \
                      click the ground to place the chosen object.",
        too_short: "Type two letters of a name, or an entry, or choose a type.",
        no_model: Some(
            "displayId is 0 or has no GameObjectDisplayInfo.dbc row. The object is placed and \
             drawn as a marker with no model. Traps and spell focus objects usually have no \
             model.",
        ),
        list_search: "name, type or entry",
        no_display: "displayId is 0 or has no GameObjectDisplayInfo.dbc row. The editor draws a \
                     spawn of it as a marker. Traps and spell focus objects usually have no \
                     model.",
        place: "Switches to the rail's Objects tool in Place with this game object chosen. The \
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

    fn picture(ui: &mut egui::Ui, subject: &mut Subject<'_>, display_id: u32, rect: egui::Rect) -> bool {
        match subject.tool.model_of(subject.assets, display_id) {
            Some(path) => {
                crate::preview::portraits::paint(ui, subject.portraits, &path, rect);
                true
            }
            None => false,
        }
    }

    fn preview(ui: &mut egui::Ui, subject: &mut Subject<'_>, display_id: u32, side: Option<f32>) -> bool {
        let Some(path) = subject.tool.model_of(subject.assets, display_id) else {
            return false;
        };
        match side {
            Some(side) => crate::ui::widgets::preview::preview_pane_at(ui, subject.portraits, &path, side),
            None => crate::ui::widgets::preview::preview_pane(ui, subject.portraits, &path),
        }
        true
    }

    /// The profession a gathering node needs, or the type.
    fn row_line(subject: &mut Subject<'_>, known: &Known) -> String {
        gathered_by(locks(&mut subject.tool.own.locks, subject.assets), known).unwrap_or_else(|| known.type_word())
    }

    fn list_line(subject: &mut Subject<'_>, known: &Known) -> String {
        Self::row_line(subject, known)
    }

    fn chosen_detail(ui: &mut egui::Ui, subject: &mut Subject<'_>, chosen: &Known) {
        let gathered = gathered_by(locks(&mut subject.tool.own.locks, subject.assets), chosen);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let faint = |text: String| egui::RichText::new(text).small().color(theme::INK_DIM);
            ui.label(faint(format!("{} · display", chosen.type_word())));
            let id = egui::RichText::new(chosen.display_id.to_string()).small();
            match chosen.display_id {
                0 => {
                    ui.label(id.color(theme::INK_DIM));
                }
                display => reference::display_link(ui, id, subject.asks, DisplayTable::Object.name(), display),
            }
            if let Some(words) = gathered {
                ui.label(faint(format!("· {words}")));
            }
        });
    }

    /// The type filter. With a type chosen, the list fills without any search
    /// text.
    fn search_filters(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("type").color(theme::INK_DIM));
            let shown = match subject.tool.new_spawn.of_type {
                Some(kind) => gameobject::value_word(&gameobject::TYPES, kind),
                None => "any".to_string(),
            };
            egui::ComboBox::from_id_salt("object-type-filter")
                .selected_text(shown)
                .width(ui.available_width() - 8.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut subject.tool.new_spawn.of_type, None, "any");
                    for named in gameobject::TYPES {
                        ui.selectable_value(&mut subject.tool.new_spawn.of_type, Some(named.value), named.name);
                    }
                })
                .response
                .on_hover_text(
                    "Filters the matches by gameobject_template.type. With a type chosen, the \
                     list shows matches without search text. Mining veins, herbs and treasure \
                     chests are all type Chest.",
                );
        });
    }

    /// As on the creature panel, everything above the spawn's columns stays
    /// in place and only the columns scroll.
    fn selected(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn, on_server: &crate::server::sync::Standing) {
        head(ui, subject, spawn);
        super::spawn::claim(ui, subject, spawn, on_server);
        ui.add_space(6.0);
        what_it_is(ui, subject, spawn);
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .min_scrolled_height(super::spawn::SPAWN_FORM_MIN)
            .show(ui, |ui| spawn_form(ui, subject, spawn));
    }

    fn template_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, key: &Key, row: &gameobject::Row) {
        let form = Form { table: gameobject::TEMPLATE, key, row, spawn: None };
        groups(ui, subject, &form, &[Group::Identity, Group::Appearance, Group::Stats]);
    }

    fn renumber_reaches() -> String {
        format!(
            "vmangos has no foreign keys, so Apply updates the references itself: the row, \
             and the {} world database columns that refer to a game object by entry: every \
             spawn's id, the quest relations, the pool and locale rows, the spell script \
             targets, and quest_template's objectives, which store an object as the negative \
             of its entry. Restore reverts all of it.\n\nNot updated: a template's \
             linkedTrapId data column, gameobject_loot_template (keyed by data1, not by \
             entry), script datalong values, and the C++ scripts.",
            gameobject::TEMPLATE_REFERENCES.len()
        )
    }

    fn list_summary(ui: &mut egui::Ui, subject: &mut Subject<'_>, known: &Known) {
        let lock = known.lock().map(|id| {
            let locks = locks(&mut subject.tool.own.locks, subject.assets);
            format!("{id} \u{2014} {}", lock_words(locks, id))
        });
        egui::Grid::new("gameobject-data-summary").num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
            let mut rows: Vec<(&str, String)> = vec![
                ("Type", known.type_word()),
                ("Display", known.display_id.to_string()),
                ("Size", format!("{}", known.size)),
                ("Faction", known.faction.to_string()),
                ("Flags", gameobject::mask_words(&gameobject::FLAGS, known.flags)),
            ];
            if let Some(lock) = lock {
                rows.push(("Lock", lock));
            }
            if let Some(loot) = known.loot() {
                rows.push(("Loot", format!("gameobject_loot_template {loot}")));
            }
            for (name, value) in rows {
                ui.label(egui::RichText::new(name).color(theme::INK_DIM));
                match name == "Display" && known.display_id != 0 {
                    true => reference::display_link(
                        ui,
                        egui::RichText::new(value),
                        subject.asks,
                        DisplayTable::Object.name(),
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
        windows(ui, subject, known.entry, known.label(), false);
    }
}


/// `Lock.dbc` and `LockType.dbc`, read the first time a lock is drawn.
///
/// The reader is the client's own, `vale_assets::tables::lock`. If the
/// archives do not provide either table, the join is empty and every lock is
/// drawn as its number.
pub(in crate::ui) fn locks<'a>(held: &'a mut Option<Locks>, assets: &GameAssets) -> &'a Locks {
    held.get_or_insert_with(|| {
        let read = assets.reader();
        let lock = read("DBFilesClient\\Lock.dbc").unwrap_or_default();
        let lock_type = read("DBFilesClient\\LockType.dbc").unwrap_or_default();
        match lock.is_empty() {
            true => Locks::default(),
            false => Locks::parse(&lock, &lock_type),
        }
    })
}

/// What a lock requires, in words: every key that opens it, in the row's own
/// column order, for example `Mining 1` or `Pick Lock 150 or item 5396`.
///
/// A skill at rank 0 is named without the number. `Open` and `Close` are on
/// most levers and quest objects and require nothing.
pub(in crate::ui) fn lock_words(locks: &Locks, lock_id: u32) -> String {
    let keys = locks.keys(lock_id);
    if keys.is_empty() {
        return "no such Lock.dbc row".to_string();
    }
    let ways: Vec<String> = keys
        .iter()
        .map(|key| match key.kind {
            KeyKind::Item(entry) => format!("item {entry}"),
            KeyKind::Skill { lock_type, rank } => {
                let name = locks
                    .lock_type_name(lock_type)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("lock type {lock_type}"));
                match rank {
                    0 => name,
                    rank => format!("{name} {rank}"),
                }
            }
        })
        .collect();
    ways.join(" or ")
}

/// Whether a chest is a gathering node, and of which profession.
///
/// `LockType.dbc` rows 2 and 3 are Herbalism and Mining
/// (`vale_assets::tables::lock::lock_type`). The answer is the skill's name
/// and the rank the lock asks for.
pub(in crate::ui) fn gathered_by(locks: &Locks, known: &Known) -> Option<String> {
    if known.kind != gameobject::TYPE_CHEST {
        return None;
    }
    let (lock_type, rank) = locks.skill_lock(known.lock()?)?;
    let gathering = [
        vale_assets::tables::lock::lock_type::HERBALISM,
        vale_assets::tables::lock::lock_type::MINING,
    ];
    if !gathering.contains(&lock_type) {
        return None;
    }
    let name = locks.lock_type_name(lock_type).unwrap_or("gathering");
    // Copper and Peacebloom ask for skill 0, which is every miner and every
    // herbalist, so the number is left off.
    Some(match rank {
        0 => format!("{name} node"),
        rank => format!("{name} node, skill {rank}"),
    })
}

/// The selected spawn's name, what it is, and the buttons.
fn head(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    ui.label(egui::RichText::new(spawn.label()).strong().size(14.0));
    let type_word = spawn
        .known
        .as_ref()
        .map(Known::type_word)
        .unwrap_or_else(|| "no template".to_string());
    ui.label(
        egui::RichText::new(format!("guid {}  ·  {type_word}", spawn.guid))
            .small()
            .color(theme::INK_DIM),
    );
    if let Some(known) = &spawn.known {
        if let Some(words) = gathered_by(locks(&mut subject.tool.own.locks, subject.assets), known) {
            ui.label(egui::RichText::new(words).small().color(theme::ACCENT));
        }
    }
    super::spawn::head_buttons(ui, subject, spawn);
}

/// The `gameobject` row: one spawn, at one place.
fn spawn_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    theme::heading(ui, "This spawn");
    theme::note(
        ui,
        "The gameobject row: one placed instance of the object. Drag the selected spawn in \
         the viewport to move it; dragging keeps its height above the ground.",
    );
    if spawn.tilted {
        ui.label(
            egui::RichText::new(
                "Tilted: rotation0 or rotation1 is not 0, so the rotation quaternion is not \
                 derived from orientation alone. Editing orientation does not change \
                 rotation2 and rotation3 on this spawn. The editor and the client draw it \
                 upright.",
            )
            .small()
            .color(theme::WARN),
        );
    }
    let key = spawn.key();
    let Some(row) = subject.tool.spawn_row.as_ref().map(|held| held.row.clone()) else {
        theme::waiting(ui, "reading the row\u{2026}");
        return;
    };
    let standing = crate::tools::spawns::spawn::Standing::of::<crate::tools::spawns::gameobjects::GameObjectSubject>(spawn);
    super::spawn::placement(ui, subject.session, &standing, subject.now);
    spawn_data(ui, subject);
    ui.add_space(8.0);
    let form = Form { table: gameobject::SPAWN, key: &key, row: &row, spawn: Some(spawn) };
    groups(ui, subject, &form, &[Group::Identity, Group::Place]);
}

/// The Pool and Game Events sections: the rows of other tables that name the
/// selected spawn, acting on the whole selection. See
/// `crate::ui::subjects::keyed`.
fn spawn_data(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    use crate::ui::subjects::keyed::{Chosen, Section};
    let patch = crate::server::settings::server_patch(subject.server);
    let edits = subject.session.rows.edits.clone();
    let map = subject.session.map_id;
    let select = {
        let Some(chosen) = Chosen::of(&*subject.tool, edits, map, true) else { return };
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
        crate::ui::subjects::pools::pool_section(ui, &mut section, subject.own.pools, patch);
        crate::ui::subjects::game_events::events_section(ui, &mut section, subject.own.game_events, patch);
        section.select
    };
    if let Some(guid) = select {
        subject.tool.select_only(Some(guid));
    }
}

/// A summary of the object's template, with a picture of its model, and the
/// buttons that open the template window, the quest window and the loot
/// window. Drawn under the spawn's name with no heading of its own, outside
/// the scrolled form; the first line names the template row.
fn what_it_is(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    let edited = subject
        .session
        .rows.edits
        .touches(gameobject::TEMPLATE, &spawn.template_key());
    ui.label(
        egui::RichText::new(format!(
            "gameobject_template entry {} at content patch {}",
            spawn.entry,
            spawn.patch()
        ))
        .small()
        .color(match edited {
            true => theme::WARN,
            false => theme::INK_DIM,
        }),
    );
    ui.label(
        egui::RichText::new("Template edits change every spawn of this entry, on every map.")
            .small()
            .color(theme::INK_FAINT),
    );
    let Some(known) = spawn.known.clone() else {
        ui.label(
            egui::RichText::new(
                "No gameobject_template row at this content patch. The server skips this \
                 spawn at startup and logs an error.",
            )
            .small()
            .color(theme::BAD),
        );
        return;
    };

    let lock = known.lock().map(|id| {
        format!("{id} — {}", lock_words(locks(&mut subject.tool.own.locks, subject.assets), id))
    });
    // The model is drawn at the panel's right edge, after the columns.
    ui.horizontal(|ui| {
        egui::Grid::new("object-summary")
            .num_columns(2)
            .spacing([8.0, 2.0])
            .show(ui, |ui| {
                let mut rows: Vec<(&str, String)> = vec![
                    ("type", known.type_word()),
                    ("display", known.display_id.to_string()),
                    ("size", format!("{}", known.size)),
                    ("faction", known.faction.to_string()),
                    ("flags", gameobject::mask_words(&gameobject::FLAGS, known.flags)),
                ];
                if let Some(lock) = lock {
                    rows.push(("lock", lock));
                }
                if let Some(loot) = known.loot() {
                    rows.push(("loot", format!("gameobject_loot_template {loot}")));
                }
                for (name, value) in rows {
                    ui.label(egui::RichText::new(name).color(theme::INK_DIM));
                    match name == "display" && known.display_id != 0 {
                        true => crate::ui::widgets::reference::display_link(
                            ui,
                            egui::RichText::new(value),
                            subject.asks,
                            DisplayTable::Object.name(),
                            known.display_id,
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
            match subject.tool.model_of(subject.assets, known.display_id) {
                Some(path) => crate::preview::portraits::paint(ui, subject.portraits, &path, rect),
                None => {
                    ui.painter().rect_filled(rect, 3.0, theme::DEAD);
                }
            }
        });
    });

    ui.add_space(4.0);
    windows(ui, subject, spawn.entry, spawn.label(), true);
}

/// The buttons that open the template window and the object's quest and loot
/// windows. Each is a toggle, lit while its window is open.
///
/// `in_world` is whether a spawn is selected in the world. The Game Objects
/// workspace draws the same row without one, and then the template window and
/// the link to the workspace are left out, since the form is already in the
/// middle of the screen.
pub(in crate::ui) fn windows(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    entry: u32,
    label: String,
    in_world: bool,
) {
    ui.horizontal_wrapped(|ui| {
        if in_world {
            super::spawn::edit_toggle(ui, subject);
        }
        let quests_open = subject
            .quests
            .window_for
            .as_ref()
            .is_some_and(|(holder, _, _)| *holder == Holder::Object);
        if ui
            .selectable_label(quests_open, "Quests")
            .on_hover_text(
                "Lists, adds and removes the quests this object starts and ends: \
                 gameobject_questrelation and gameobject_involvedrelation. Each quest opens in \
                 the quest workspace. The rows are keyed by gameobject_template entry, so they \
                 apply to every spawn of it.",
            )
            .clicked()
        {
            subject.quests.window_for = match quests_open {
                true => None,
                false => Some((Holder::Object, entry, label.clone())),
            };
        }
        // The loot window stays open over the world and follows the selection,
        // as the quest window does. The button is enabled for every type,
        // because the window itself states when the server takes no loot from
        // the type. See `crate::ui::subjects::loot`.
        if ui
            .selectable_label(subject.loot.open, "Loot")
            .on_hover_text(
                "The loot of a chest, vein, herb or fishing pool: gameobject_loot_template, \
                 keyed by the lootId data column. Every object with the same loot id shares \
                 these rows.",
            )
            .clicked()
        {
            subject.loot.toggle();
        }
        if in_world {
            super::spawn::open_in_templates(ui, subject, entry);
        }
    });
}

/// The side of the picture beside the summary, in points.
const SUMMARY_PICTURE: f32 = 56.0;

/// Which row a form is over.
pub(in crate::ui) struct Form<'a> {
    pub(in crate::ui) table: &'static str,
    pub(in crate::ui) key: &'a Key,
    pub(in crate::ui) row: &'a gameobject::Row,
    /// The selected spawn, for `orientation`, the one column whose write
    /// depends on it; see [`field`]. `None` for the template window and the
    /// Game Objects workspace, which may be about an object with no spawn.
    pub(in crate::ui) spawn: Option<&'a Spawn>,
}

impl Form<'_> {
    /// The template's type as the form shows it: the project's value if it has
    /// one, otherwise the database's. It names the `data` columns, so an edit
    /// to `type` renames them on the next frame.
    fn shown_type(&self, session: &EditSession) -> u32 {
        self.shown_number(session, "type") as u32
    }

    /// What one column holds right now, as a number.
    fn shown_number(&self, session: &EditSession, column: &str) -> i64 {
        session
            .rows.edits
            .shown_integer(self.table, self.key, column, Some(self.row))
            .unwrap_or(0)
    }
}

/// One column as the form draws it: the table's own column, or, for a
/// template's `data` column, the field the row's type maps it to.
struct Drawn {
    /// The column written, always the table's own name.
    column: &'static Column,
    /// What the label says. A data field's own name with the column after it.
    title: String,
    kind: Kind,
    about: String,
}

/// What a column is drawn as, given the template's type.
fn drawn(table: &str, column: &'static Column, kind: u32) -> Drawn {
    let plain = Drawn {
        column,
        title: column.name.to_string(),
        kind: column.kind,
        about: column.about.to_string(),
    };
    if table != gameobject::TEMPLATE {
        return plain;
    }
    let Some(index) = gameobject::data_index(column.name) else {
        return plain;
    };
    match gameobject::data_field(kind, index) {
        Some(named) => Drawn {
            column,
            title: format!("{} · {}", named.name, column.name),
            kind: named.kind,
            about: named.about.to_string(),
        },
        None => Drawn { about: "not read by this type".to_string(), ..plain },
    }
}

/// Every group of a table's columns, one page section per group: the named
/// ones open, the rest folded.
///
/// The same as `super::creatures::groups`, except that the template's `Stats`
/// group (the 24 `data` columns) is split in two: the columns the type reads,
/// under the type's name, and the columns it does not read, folded.
pub(in crate::ui) fn groups(ui: &mut egui::Ui, subject: &mut Subject<'_>, form: &Form<'_>, open: &[Group]) {
    page_spacing(ui);
    let kind = form.shown_type(subject.session);
    let mut seen: Vec<Group> = Vec::new();
    let all: &'static [Column] =
        vale_mangos::row::table_named(gameobject::TABLES, form.table).map_or(&[], |table| table.columns);
    for column in all {
        if !seen.contains(&column.group) {
            seen.push(column.group);
        }
    }
    for group in seen {
        let columns: Vec<Drawn> = all
            .iter()
            .filter(|column| column.group == group)
            .map(|column| drawn(form.table, column, kind))
            .collect();
        let is_data = form.table == gameobject::TEMPLATE && group == Group::Stats;
        let (read, unread): (Vec<Drawn>, Vec<Drawn>) = match is_data {
            true => columns.into_iter().partition(|column| {
                gameobject::data_index(column.column.name)
                    .is_some_and(|index| gameobject::data_field(kind, index).is_some())
            }),
            false => (columns, Vec::new()),
        };
        let title = match is_data {
            true => format!(
                "{} fields",
                gameobject::value_word(&gameobject::TYPES, kind)
            ),
            false => group.name().to_string(),
        };
        if is_data && read.is_empty() {
            theme::note(
                ui,
                format!(
                    "{} reads none of the 24 data columns.",
                    gameobject::value_word(&gameobject::TYPES, kind)
                ),
            );
        } else {
            section(ui, subject, form, &title, &read, open.contains(&group), (group, 0));
        }
        if !unread.is_empty() {
            // Open only when one of them is non-zero. A value in a column the
            // type does not read is either left over from another type or a
            // mistake, and the user should see it in both cases.
            let holds = unread
                .iter()
                .any(|column| form.shown_number(subject.session, column.column.name) != 0);
            section(ui, subject, form, "Data columns this type does not read", &unread, holds, (group, 1));
        }
    }
    ui.add_space(12.0);
}

/// One folding section of a form, with a page row per column.
#[allow(clippy::too_many_arguments)]
fn section(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    form: &Form<'_>,
    title: &str,
    columns: &[Drawn],
    open: bool,
    which: (Group, usize),
) {
    let edited = columns.iter().any(|column| {
        subject
            .session
            .rows.edits
            .get(form.table, form.key, column.column.name)
            .is_some()
    });
    page_section(
        ui,
        (form.table, which.0.name(), which.1),
        title,
        edited,
        open,
        |ui| {
            for column in columns {
                field(ui, subject, form, column);
            }
        },
    );
}

/// One column on a row of its own, through [`server_field`], under the name
/// and type `drawn` gives it. A lock is resolved through `Lock.dbc` and a
/// display id drawn as its model. On an upright spawn the facing is written
/// to three columns, so its revert clears all three; see
/// [`GameObjects::turn`].
fn field(ui: &mut egui::Ui, subject: &mut Subject<'_>, form: &Form<'_>, drawn: &Drawn) {
    let column = drawn.column;
    let turns = form.table == gameobject::SPAWN && column.name == "orientation";
    let upright = form.spawn.is_some_and(|spawn| !spawn.tilted);
    let field = FieldCtx {
        title: &drawn.title,
        about: &drawn.about,
        kind: drawn.kind,
        reverts_with: match turns && upright {
            true => &["rotation2", "rotation3"],
            false => &[],
        },
        ..FieldCtx::new(ui, &subject.session.rows.edits, form.table, form.key, form.row, column, "Edit game object", subject.now)
    };
    let assets = subject.assets;
    let (objects, portraits) = (&mut *subject.tool, &mut *subject.portraits);
    let mut resolver = Resolver {
        session: &mut *subject.session,
        assets,
        quests: &mut *subject.quests,
        asks: &mut *subject.asks,
    };
    let written = server_field(ui, &mut resolver, &field, |hook, ui, resolver, field, showing| {
        match (hook, field.kind) {
            (Hook::Cell, Kind::Ref(dbc)) if DisplayTable::of(dbc).is_some() => {
                Some(display_cell(ui, objects, portraits, resolver.asks, assets, showing, field.target()))
            }
            // A lock is resolved through the client's own table, in words,
            // and has no picker: `Lock.dbc` carries no name to search.
            (Hook::Cell, Kind::Ref("Lock")) => {
                let written = number_cell(ui, Kind::Unsigned, showing);
                let words = match showing.trim().parse::<u32>().unwrap_or(0) {
                    0 => "no lock".to_string(),
                    lock => lock_words(locks(&mut objects.own.locks, assets), lock),
                };
                meaning_truncated(ui, words);
                Some(written)
            }
            _ => None,
        }
    });
    let Some(written) = written else {
        return;
    };
    match (form.spawn.filter(|_| turns), written.trim().parse::<f32>()) {
        (Some(spawn), Ok(orientation)) => {
            GameObjects::turn(subject.session, spawn.guid, spawn.tilted, orientation, subject.now)
        }
        _ => field.write(subject.session, written, subject.now),
    }
}

/// A display id: the number, a picture of the model it names, the button
/// that opens the picker, and the model's path.
fn display_cell(
    ui: &mut egui::Ui,
    objects: &mut GameObjects,
    portraits: &mut crate::preview::portraits::Portraits,
    asks: &mut ShellAsks,
    assets: &GameAssets,
    showing: &str,
    target: ColumnTarget,
) -> Option<String> {
    let written = number_cell(ui, Kind::Unsigned, showing);
    let id: u32 = showing.trim().parse().unwrap_or(0);
    let path = objects.model_of(assets, id);
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(FORM_ROW + 4.0), egui::Sense::hover());
    match path.as_deref() {
        Some(path) => crate::preview::portraits::paint(ui, portraits, path, rect),
        None if id != 0 => {
            ui.painter().rect_filled(rect, 3.0, theme::DEAD);
        }
        None => {}
    }
    if reference::choose_button(
        ui,
        "Opens a picker of every GameObjectDisplayInfo.dbc row that has a model, \
         shown as thumbnails and searchable by model path.",
    ) {
        objects.display_pick = Some(DisplayPick::open(DisplayTable::Object, target, id));
    }
    match path {
        Some(path) => reference::display_link(
            ui,
            egui::RichText::new(path).small(),
            asks,
            DisplayTable::Object.name(),
            id,
        ),
        None if id != 0 => {
            ui.label(
                egui::RichText::new("not in GameObjectDisplayInfo.dbc")
                    .small()
                    .color(theme::BAD),
            );
        }
        None => meaning(ui, "none: drawn as a marker"),
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str) -> &'static Column {
        gameobject::TEMPLATE_TABLE.column(name).expect("a column")
    }

    /// A data column is drawn under the type's name for it and written under
    /// its own name. `data0` is a chest's lock and a chair's seat count, and
    /// the statement names `data0` in both cases.
    #[test]
    fn a_data_column_is_named_by_the_type() {
        let chest = drawn(gameobject::TEMPLATE, column("data0"), gameobject::TYPE_CHEST);
        assert_eq!(chest.title, "lockId · data0");
        assert_eq!(chest.kind, Kind::Ref("Lock"));
        assert_eq!(chest.column.name, "data0");

        let chair = drawn(gameobject::TEMPLATE, column("data0"), 7);
        assert_eq!(chair.title, "slots · data0");
        assert_eq!(chair.kind, Kind::Unsigned);

        // A column the type does not read keeps its own name and says so.
        let unread = drawn(gameobject::TEMPLATE, column("data9"), 7);
        assert_eq!(unread.title, "data9");
        assert_eq!(unread.about, "not read by this type");

        // A column that is not a data column is unchanged whatever the type.
        let name = drawn(gameobject::TEMPLATE, column("name"), gameobject::TYPE_CHEST);
        assert_eq!(name.title, "name");
        assert_eq!(name.kind, Kind::Text);
    }

    /// The spawn table has no data columns, so none of its columns is renamed.
    /// `gameobject` has no column called `data0`.
    #[test]
    fn a_spawn_column_is_never_renamed() {
        for column in gameobject::SPAWN_TABLE.columns {
            let shown = drawn(gameobject::SPAWN, column, gameobject::TYPE_CHEST);
            assert_eq!(shown.title, column.name);
            assert_eq!(shown.kind, column.kind);
        }
    }

    /// An archive with no `Lock.dbc` draws a lock as a missing row rather than
    /// failing.
    #[test]
    fn a_lock_the_table_does_not_hold_says_so() {
        assert_eq!(lock_words(&Locks::default(), 38), "no such Lock.dbc row");
    }
}
