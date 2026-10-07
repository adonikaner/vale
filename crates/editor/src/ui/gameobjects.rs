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
//! [`super::rowform`]'s page: one column per row under a folding heading per
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
//!   and nothing else, so its picker ([`super::displays`]) is a grid of every
//!   row's model, searched by path.
//! * An edit to `orientation` also writes the rotation quaternion on an upright
//!   spawn. On a tilted spawn the form states that it does not. See
//!   `crate::tools::gameobjects::GameObjects::turn`.
//!
//! Apply, Put back and Discard are on the top bar's Server… panel, in the
//! Objects block. See [`super::sync`].

use bevy_egui::egui;

use super::theme;
use crate::server::gameobjects::{REVERT_VPATH as REVERT, SQL_VPATH as SQL};
use crate::session::EditSession;
use crate::tools::displays::{DisplayPick, Table as DisplayTable};
use crate::tools::gameobjects::{GameObjects, Known, Spawn};
use crate::tools::place;
use crate::tools::quests::{ColumnTarget, Holder};
use vale_assets::tables::lock::{KeyKind, Locks};
use vale_client::assets::GameAssets;
use vale_mangos::gameobject::{self, Column, Group, Kind, RowValue};
use vale_mangos::row::{Key, Life};

use super::rowform::{
    choice_cell, draft_or, finished, flags_cell, meaning, meaning_truncated, number_cell,
    number_means, page_row, page_spacing, revert_button, section as page_section, text_cell,
    FORM_ROW, FORM_VALUE,
};

/// Everything the panel needs, as one argument.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub objects: &'a mut GameObjects,
    /// The quest tool's state: which object's quests its window is showing,
    /// the reference picker every form shares, and the name cache a
    /// reference column resolves through.
    pub quests: &'a mut crate::tools::quests::Quests,
    /// The loot window's state, for the Loot button beside the Quests button.
    pub loot: &'a mut crate::tools::loot::Loot,
    pub server: &'a crate::server::settings::ServerSettings,
    /// The top bar's Server… panel, which holds every server operation.
    pub server_panel: &'a mut super::popover::Popover,
    /// The archives, for what a display id looks like, what a lock needs and
    /// the client tables a reference resolves through.
    pub assets: &'a GameAssets,
    pub portraits: &'a mut crate::portraits::Portraits,
    /// The icons the reference picker's spell and item rows draw. See
    /// `super::thumbnails`.
    pub thumbnails: &'a mut super::thumbnails::Thumbnails,
    pub now: f64,
    /// The gizmo, for the Handles switch drawn under the Select/Place switch.
    /// `None` where the panel's parts are drawn without it, as in the
    /// template window.
    pub gizmo: Option<&'a mut crate::tools::gizmo::Gizmo>,
}

/// `Lock.dbc` and `LockType.dbc`, read the first time a lock is drawn.
///
/// The reader is the client's own, `vale_assets::tables::lock`. If the
/// archives do not provide either table, the join is empty and every lock is
/// drawn as its number.
pub(super) fn locks<'a>(held: &'a mut Option<Locks>, assets: &GameAssets) -> &'a Locks {
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
pub(super) fn lock_words(locks: &Locks, lock_id: u32) -> String {
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
pub(super) fn gathered_by(locks: &Locks, known: &Known) -> Option<String> {
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

/// Draw the game-object tool's panel, and the two dialogs its forms open.
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
        &mut subject.objects.display_pick,
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

    if subject.objects.trouble.is_some() {
        nothing_to_show(ui, subject);
        return;
    }

    let plan = crate::server::gameobjects::plan(subject.session);
    let on_server = crate::server::gameobjects::OnTheServer::read_with(subject.session, &plan);
    edits_block(ui, subject, &plan, &on_server);

    let mut mode = subject.objects.mode;
    theme::segmented(ui, &mut mode, &place::MODES, |a, b| a == b);
    if mode != subject.objects.mode {
        subject.objects.mode = mode;
        if mode == place::Mode::Select {
            subject.objects.new_spawn.chosen = None;
        }
    }
    ui.add_space(6.0);

    if subject.objects.mode == place::Mode::Place {
        picker(ui, subject);
        return;
    }
    if subject.objects.spawn_count() == 0 {
        nothing_to_show(ui, subject);
        return;
    }
    let Some(spawn) = subject.objects.chosen_edited(Some(&subject.session.server_edits)) else {
        theme::note(
            ui,
            "Click a game object in the viewport to select it. The nearest spawns are drawn \
             as models; the rest are drawn as squares on the ground.",
        );
        return;
    };

    if let Some(gizmo) = subject.gizmo.as_deref_mut() {
        super::creatures::handles(ui, gizmo);
    }
    match super::creatures::group_line(ui, subject.objects.also.len() + 1, "game objects") {
        Some(super::creatures::GroupAsk::Only) => subject.objects.also.clear(),
        Some(super::creatures::GroupAsk::Clear) => subject.objects.select(None),
        None => {}
    }
    // As on the creature panel, everything above the spawn's columns stays in
    // place and only the columns scroll.
    head(ui, subject, &spawn);
    claim(ui, subject, &spawn, &on_server);
    ui.add_space(6.0);
    what_it_is(ui, subject, &spawn);
    ui.add_space(8.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .min_scrolled_height(super::creatures::SPAWN_FORM_MIN)
        .show(ui, |ui| spawn_form(ui, subject, &spawn));
}

/// What the tool is showing, and the two switches that decide how much of it.
fn controls(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    let spawns = subject.objects.spawns.len();
    let mine = subject.objects.created.len();
    let near = subject.objects.near.len();
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(match (subject.objects.reading(), mine) {
                (true, _) => "reading the map's spawns…".to_string(),
                (false, 0) => format!("{spawns} spawn(s) on this map, {near} near"),
                // Kept short: the Reload button shares the line, and egui
                // clips text that does not fit.
                (false, mine) => format!("{spawns} on this map, {mine} new, {near} near"),
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
                subject.objects.forget();
            }
        });
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut subject.objects.show_models, "Models").on_hover_text(
            "Draws the nearest spawns with their models. Off, every spawn is drawn as a \
             square; spawns past the model limit are always squares.",
        );
        ui.add(
            egui::DragValue::new(&mut subject.objects.model_budget)
                .speed(5.0)
                .range(0..=3000)
                .prefix("at most "),
        )
        .on_hover_text("Maximum number of spawns drawn as models, nearest first.");
    });
}

/// Why the list is empty: a connection error, a read in progress, or a map
/// with no game objects.
fn nothing_to_show(ui: &mut egui::Ui, subject: &Subject<'_>) {
    match &subject.objects.trouble {
        Some(trouble) => {
            ui.label(egui::RichText::new(trouble).small().color(theme::WARN));
            theme::note(
                ui,
                "Game objects are rows in the vmangos world database, not files in the \
                 archives, so this tool needs a database connection. Set it in Server… on \
                 the top bar.",
            );
        }
        None if subject.objects.reading() => {
            theme::waiting(ui, "reading\u{2026}");
        }
        None => theme::note(ui, "This map has no game object spawns."),
    }
}

/// What this project changes in the server's game objects, and the button
/// that opens the panel that applies it. The game-object counterpart of
/// `super::creatures::edits_block`.
fn edits_block(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    plan: &crate::server::gameobjects::Plan,
    on_server: &crate::server::gameobjects::OnTheServer,
) {
    let undoable = on_server.rows();
    if plan.is_empty() && undoable == 0 {
        return;
    }
    let outstanding = plan
        .rows
        .iter()
        .filter(|row| !on_server.covers(row.table, &row.key))
        .count();

    ui.separator();
    theme::heading(ui, "This project");
    if !plan.is_empty() {
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
    match (undoable, outstanding, plan.is_empty()) {
        (0, _, false) => theme::note(
            ui,
            "Not applied. The database holds its original values; the editor draws this \
             project's values.",
        ),
        (n, 0, false) => {
            let line = match on_server.current() {
                true => format!(
                    "Applied — all {n} row(s) are in the database. Restart the server to load \
                     them; Restore reverts them."
                ),
                false => format!(
                    "Applied — all {n} row(s) are in the database, but their values may differ \
                     from the values shown: a row was edited after the apply, or the apply was \
                     made in an earlier session. Apply again to write the values shown."
                ),
            };
            ui.label(egui::RichText::new(line).small().color(theme::INK_DIM));
        }
        (n, out, false) => {
            ui.label(
                egui::RichText::new(format!(
                    "{out} row(s) not applied; {n} applied row(s) can be restored"
                ))
                .small()
                .color(theme::WARN),
            );
        }
        (n, _, true) => {
            ui.label(
                egui::RichText::new(format!(
                    "{n} applied row(s) can be restored; this project no longer changes any of them"
                ))
                .small()
                .color(theme::WARN),
            );
        }
    }
    if ui
        .button("Server\u{2026}")
        .on_hover_text(format!(
            "Opens the Server panel, which applies, restores or discards these rows. \
             Restore runs {REVERT}.",
        ))
        .clicked()
    {
        subject.server_panel.show();
    }
    theme::note(
        ui,
        "Apply writes the rows only. The server reads game objects once, at startup; restart \
         it to load them.",
    );
    ui.separator();
}

/// The Place mode: choose an object, then click the ground.
///
/// The same as `super::creatures::picker`, plus a type filter. With a type
/// chosen, the list fills without any search text.
fn picker(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    theme::heading(ui, "Game object");
    template_actions(ui, subject);
    ui.add_space(4.0);
    match subject.objects.new_spawn.chosen.clone() {
        Some(chosen) => {
            ui.label(egui::RichText::new(chosen.label()).color(theme::INK));
            let gathered = gathered_by(locks(&mut subject.objects.locks, subject.assets), &chosen);
            ui.label(
                egui::RichText::new(format!(
                    "{} · display {}{}",
                    chosen.type_word(),
                    chosen.display_id,
                    gathered.map(|words| format!(" · {words}")).unwrap_or_default()
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
                "Click the ground to place it. Escape clears the chosen object. Every \
                 column of the new row can be edited afterwards in Select.",
            );
            theme::note(ui, ", and . turn it \u{b7} shift turns three times as far \u{b7} alt + mouse turns it");
            theme::note(ui, "ctrl with alt snaps the turn to 15\u{b0}");
            // The window's toggle, here as well as under a selected spawn, so
            // an object can be edited before any spawn of it exists.
            let open = subject.objects.template_window;
            if ui
                .selectable_label(open, "Edit object…")
                .on_hover_text(
                    "Opens the gameobject_template columns in a separate window. The window \
                     shows the chosen object.",
                )
                .clicked()
            {
                subject.objects.template_window = !open;
            }
            match subject.objects.model_of(subject.assets, chosen.display_id) {
                Some(path) => {
                    ui.add_space(4.0);
                    super::inspector::preview_pane(ui, subject.portraits, &path);
                }
                None => theme::note(
                    ui,
                    "displayId is 0 or has no GameObjectDisplayInfo.dbc row. The object is \
                     placed and drawn as a marker with no model. Traps and spell focus objects \
                     usually have no model.",
                ),
            }
        }
        None => theme::note(
            ui,
            "No object chosen. Search below by name or entry, or choose a type, then click \
             the ground to place the chosen object.",
        ),
    }

    created_templates(ui, subject);

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("find").color(theme::INK_DIM));
        ui.add(
            egui::TextEdit::singleline(&mut subject.objects.new_spawn.search)
                .hint_text("name or entry")
                .desired_width(f32::INFINITY),
        );
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("type").color(theme::INK_DIM));
        let shown = match subject.objects.new_spawn.of_type {
            Some(kind) => gameobject::value_word(&gameobject::TYPES, kind),
            None => "any".to_string(),
        };
        egui::ComboBox::from_id_salt("object-type-filter")
            .selected_text(shown)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut subject.objects.new_spawn.of_type, None, "any");
                for named in gameobject::TYPES {
                    ui.selectable_value(
                        &mut subject.objects.new_spawn.of_type,
                        Some(named.value),
                        named.name,
                    );
                }
            })
            .response
            .on_hover_text(
                "Filters the matches by gameobject_template.type. With a type chosen, the \
                 list shows matches without search text. Mining veins, herbs and treasure \
                 chests are all type Chest.",
            );
    });
    if let Some(trouble) = subject.objects.new_spawn.trouble.clone() {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
    }
    if subject.objects.new_spawn.searching() {
        theme::waiting(ui, "searching\u{2026}");
        return;
    }
    let typed = subject.objects.new_spawn.search.trim().len();
    if typed < 2 && subject.objects.new_spawn.of_type.is_none() {
        theme::note(ui, "Type two letters of a name, or an entry, or choose a type.");
        return;
    }
    let matches = subject.objects.new_spawn.matches.clone();
    if matches.is_empty() {
        ui.label(
            egui::RichText::new("no matching game object")
                .small()
                .color(theme::INK_DIM),
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("new-object-matches")
        .max_height(MATCH_LIST_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for known in &matches {
                let on = subject
                    .objects
                    .new_spawn
                    .chosen
                    .as_ref()
                    .is_some_and(|had| had.entry == known.entry);
                if object_row(ui, subject, known, on) {
                    subject.objects.new_spawn.chosen = Some(known.clone());
                }
            }
        });
    if matches.len() >= 50 {
        theme::note(ui, "Showing the first 50 matches. Type more of the name to narrow the list.");
    }
}

/// How much of the panel the picker's list may take.
const MATCH_LIST_HEIGHT: f32 = 260.0;

/// The three buttons over the picker: a new template, a copy of the one the
/// window is about, and giving up a created one. The creature picker has the
/// same row.
fn template_actions(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    let now = subject.now;
    let patch = crate::tools::creatures::server_patch(subject.server);
    let shown = subject.objects.template_subject(&subject.session.server_edits);
    let row_read = shown.as_ref().is_some_and(|shown| {
        subject.objects.template.as_ref().is_some_and(|held| held.entry == shown.entry)
    });
    let created = shown.as_ref().is_some_and(|shown| shown.is_new());
    ui.horizontal(|ui| {
        let third = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(40.0);
        let size = egui::vec2(third, 22.0);
        if ui
            .add(egui::Button::new("+ New").min_size(size))
            .on_hover_text(
                "Creates a gameobject_template row with an entry from 2,000,000 up, clear of \
                 upstream vmangos entries. It starts as a Generic object of size 1 with no \
                 model, drawn as a marker until displayId is set. Opens the template window \
                 on the new row.",
            )
            .clicked()
        {
            let entry =
                subject.objects.create_template(subject.session, "New Game Object", patch, now);
            subject.session.status = format!("gameobject_template {entry} created");
        }
        if ui
            .add_enabled(row_read, egui::Button::new("Copy").min_size(size))
            .on_hover_text(
                "Creates a copy of the chosen gameobject_template row under a new entry. \
                 Each column takes the value shown: this project's edit where there is one, \
                 otherwise the database value.",
            )
            .on_disabled_hover_text("Choose an object first.")
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject.session.status =
                    match subject.objects.copy_template(subject.session, shown, patch, now) {
                        Some(entry) => format!("copied as gameobject_template {entry}"),
                        None => "still reading this object's row".to_string(),
                    };
            }
        }
        if ui
            .add_enabled(created, egui::Button::new("Discard").min_size(size))
            .on_hover_text(
                "Discards the gameobject_template row this project creates. The database is \
                 not changed. A row already in the database cannot be removed: \
                 .reload gameobject_template never drops an entry it has read.",
            )
            .on_disabled_hover_text("Only an object this project created can be discarded.")
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject.objects.discard_template(subject.session, shown.entry, now);
                subject.session.status = format!("gameobject_template {} discarded", shown.entry);
            }
        }
    });
}

/// The templates this project creates, listed above the search because no
/// query returns them. Clicking one chooses it, as a search match does.
fn created_templates(ui: &mut egui::Ui, subject: &mut Subject<'_>) {
    let templates = subject.objects.templates.clone();
    if templates.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(format!("{} object(s) this project creates", templates.len()))
            .small()
            .color(theme::INK_DIM),
    );
    egui::ScrollArea::vertical()
        .id_salt("created-object-templates")
        .max_height(CREATED_LIST_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for known in &templates {
                let on = subject
                    .objects
                    .new_spawn
                    .chosen
                    .as_ref()
                    .is_some_and(|had| had.entry == known.entry);
                if object_row(ui, subject, known, on) {
                    subject.objects.new_spawn.chosen = Some(known.clone());
                }
            }
        });
}

/// The height the created list may take before it scrolls.
const CREATED_LIST_HEIGHT: f32 = 160.0;

/// One row of the picker: a picture of the model, the name, the type and the
/// entry.
fn object_row(ui: &mut egui::Ui, subject: &mut Subject<'_>, known: &Known, on: bool) -> bool {
    let sub = match gathered_by(locks(&mut subject.objects.locks, subject.assets), known) {
        Some(words) => words,
        None => known.type_word(),
    };
    let shape = theme::list_row(
        ui,
        theme::ListRow {
            title: &known.name,
            sub: &sub,
            trailing: &known.entry.to_string(),
            tint: theme::INK,
            picture: true,
        },
        on,
    );
    match subject.objects.model_of(subject.assets, known.display_id) {
        Some(path) => crate::portraits::paint(ui, subject.portraits, &path, shape.picture),
        None => {
            ui.painter().rect_filled(shape.picture, 3.0, theme::LINE);
        }
    }
    shape.response.clicked()
}

/// What this project claims about the selected spawn (new, marked for
/// removal, or neither), one sentence per state, and the button that changes
/// the claim.
fn claim(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    spawn: &Spawn,
    on_server: &crate::server::gameobjects::OnTheServer,
) {
    let now = subject.now;
    let written = on_server.covers(gameobject::SPAWN, &spawn.key());
    if spawn.is_new() {
        ui.label(
            egui::RichText::new(match written {
                false => "New — this project creates it. There is no row in the database yet.",
                true => {
                    "New — created by this project and applied. The row is in the database; \
                     restart the server to load it."
                }
            })
            .small()
            .color(theme::ACCENT),
        );
    }
    if spawn.is_removed() {
        ui.label(
            egui::RichText::new(match written {
                false => {
                    "Marked for removal. Apply deletes the gameobject row and the spawn's \
                     game_event_gameobject and gameobject_battleground rows."
                }
                true => {
                    "Removed and applied. The gameobject row and its game_event_gameobject and \
                     gameobject_battleground rows are deleted; Restore reverts them."
                }
            })
            .small()
            .color(theme::BAD),
        );
    }
    ui.horizontal(|ui| match spawn.is_removed() {
        true => {
            if ui
                .button("Cancel removal")
                .on_hover_text(
                    "Unmarks the spawn for removal. This project's column edits to it are \
                     kept.",
                )
                .clicked()
            {
                GameObjects::keep(subject.session, spawn.guid, now);
                subject.session.status = format!("game object {} removal cancelled", spawn.guid);
            }
        }
        false => {
            let word = match spawn.is_new() {
                true => "Discard this spawn",
                false => "Remove this spawn",
            };
            if ui
                .button(egui::RichText::new(word).color(theme::BAD))
                .on_hover_text(match (spawn.is_new(), written) {
                    (true, true) => {
                        "Discards the spawn from this project. The row is already in the \
                         database and stays there; Restore removes it."
                    }
                    (true, false) => {
                        "Discards the spawn this project creates. The database is not changed."
                    }
                    (false, _) => {
                        "Marks the gameobject row for deletion. Apply deletes it; then restart \
                         the server, because .reload gameobject never removes a spawn it has \
                         already read."
                    }
                })
                .clicked()
            {
                GameObjects::remove(subject.session, spawn.guid, now);
                subject.session.status = match spawn.is_new() {
                    true => format!("game object {} discarded", spawn.guid),
                    false => format!("game object {} is marked for removal", spawn.guid),
                };
                if spawn.is_new() {
                    subject.objects.select(None);
                }
            }
        }
    });
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
        if let Some(words) = gathered_by(locks(&mut subject.objects.locks, subject.assets), known) {
            ui.label(egui::RichText::new(words).small().color(theme::ACCENT));
        }
    }
    ui.horizontal(|ui| {
        if ui.small_button("Go to").on_hover_text("Moves the camera to this spawn.").clicked() {
            subject.objects.fly_to(spawn.at);
        }
        let has_row = subject
            .objects
            .spawn_row
            .as_ref()
            .is_some_and(|held| Some(held.guid) == subject.objects.selected);
        if ui
            .add_enabled(has_row, egui::Button::new("Duplicate"))
            .on_hover_text(
                "Creates a spawn of this object two yards north, with every column copied \
                 from this spawn, including this project's edits.",
            )
            .on_disabled_hover_text("Still reading this spawn's row.")
            .clicked()
        {
            let now = subject.now;
            subject.session.status = match subject.objects.duplicate(subject.session, now) {
                Some(guid) => format!("duplicated as guid {guid}"),
                None => "nothing to duplicate".into(),
            };
        }
        let others = subject
            .objects
            .spawns
            .iter()
            .filter(|other| other.entry == spawn.entry)
            .count();
        ui.label(
            egui::RichText::new(format!("{others} spawn(s) of this object on this map"))
                .small()
                .color(theme::INK_FAINT),
        );
    });
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
    let Some(row) = subject.objects.spawn_row.as_ref().map(|held| held.row.clone()) else {
        theme::waiting(ui, "reading the row\u{2026}");
        return;
    };
    let standing = crate::tools::spawn::Standing {
        kind: crate::tools::spawn::Kind::Object { tilted: spawn.tilted },
        guid: spawn.guid,
        at: spawn.at,
        orientation: spawn.orientation,
    };
    super::creatures::placement(ui, subject.session, &standing, subject.now);
    let form = Form { table: gameobject::SPAWN, key: &key, row: &row, spawn: Some(spawn) };
    groups(ui, subject, &form, &[Group::Identity, Group::Place]);
}

/// A summary of the object's template, with a picture of its model, and the
/// buttons that open the template window, the quest window and the loot
/// window. Drawn under the spawn's name with no heading of its own, outside
/// the scrolled form; the first line names the template row.
fn what_it_is(ui: &mut egui::Ui, subject: &mut Subject<'_>, spawn: &Spawn) {
    let edited = subject
        .session
        .server_edits
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
        format!("{id} — {}", lock_words(locks(&mut subject.objects.locks, subject.assets), id))
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
                    ui.label(egui::RichText::new(value).color(theme::INK));
                    ui.end_row();
                }
            });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::Vec2::splat(SUMMARY_PICTURE), egui::Sense::hover());
            match subject.objects.model_of(subject.assets, known.display_id) {
                Some(path) => crate::portraits::paint(ui, subject.portraits, &path, rect),
                None => {
                    ui.painter().rect_filled(rect, 3.0, theme::DEAD);
                }
            }
        });
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let open = subject.objects.template_window;
        if ui
            .selectable_label(open, "Edit object…")
            .on_hover_text(
                "Opens the 34 gameobject_template columns in a separate window. The 24 data \
                 columns are labelled with the field names of this object's type.",
            )
            .clicked()
        {
            subject.objects.template_window = !open;
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
                false => Some((Holder::Object, spawn.entry, spawn.label())),
            };
        }
        // The loot window stays open over the world and follows the selection,
        // as the quest window does. The button is enabled for every type,
        // because the window itself states when the server takes no loot from
        // the type. See `super::loot`.
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
    });
}

/// The side of the picture beside the summary, in points.
const SUMMARY_PICTURE: f32 = 56.0;

/// The `gameobject_template` window, where an object's template is edited.
pub fn template_window(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    if !subject.objects.template_window {
        return None;
    }
    // The picker's object in Place, the selected spawn's template in Select;
    // see `GameObjects::template_subject`.
    let shown = subject.objects.template_subject(&subject.session.server_edits)?;
    let mut open = true;
    let response =
        egui::Window::new(format!("{} — gameobject_template {}", shown.label, shown.entry))
            .id(egui::Id::new("gameobject-template"))
            .open(&mut open)
            .default_size([680.0, 640.0])
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
                    .objects
                    .template
                    .as_ref()
                    .filter(|held| held.entry == shown.entry)
                    .map(|held| held.row.clone())
                else {
                    theme::waiting(ui, "reading the row\u{2026}");
                    return;
                };
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    let form =
                        Form { table: gameobject::TEMPLATE, key: &key, row: &row, spawn: None };
                    groups(
                        ui,
                        subject,
                        &form,
                        &[Group::Identity, Group::Appearance, Group::Stats],
                    );
                });
            });
    subject.objects.template_window = open;
    // The dialogs are drawn here as well as under the panel, because the
    // window is drawn whether or not the inspector is showing this tool's
    // panel; each dialog keeps itself to one draw a frame.
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
                    "New: this row is not in the database yet. Apply writes it; restart the \
                     server to load spawns of it.",
                )
                .small()
                .color(theme::ACCENT),
            );
        }
        _ => {
            ui.label(
                egui::RichText::new(format!(
                    "Edits change every spawn of this entry, on every map — {} on this map",
                    subject
                        .objects
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
            .on_hover_text("the game object id");
        let id = ui.make_persistent_id(("gameobject-template-entry", shown.patch));
        entry_field(ui, subject, shown, id);
    });
}

/// The `entry` box: the one field of the window that is not a column edit.
/// It is committed on Enter or on leaving the box, and the commit is one
/// operation whatever the row is: the project's claim on the row is re-keyed.
/// See [`crate::tools::gameobjects::GameObjects::rekey_template`], and
/// `super::creatures::entry_field`, which is the same field for the other table.
fn entry_field(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    shown: &crate::tools::creatures::TemplateSubject,
    id: egui::Id,
) {
    let mut text = draft_or(ui, id, shown.entry.to_string());
    let response = ui.add_sized(
        egui::vec2(FORM_VALUE, FORM_ROW),
        egui::TextEdit::singleline(&mut text).id(id).horizontal_align(egui::Align::Center),
    );
    // The refusal is kept until the box is touched again, so that it is read.
    let trouble_id = id.with("trouble");
    if let Some(text) = finished(ui, id, &response, text) {
        let trouble = match text.trim().parse::<u32>() {
            Err(_) => Some(format!("{text:?} is not a valid entry number")),
            Ok(wanted) => {
                let now = subject.now;
                match subject.objects.rekey_template(subject.session, shown, wanted, now) {
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
        (true, _) => ("changes this new object's entry".to_string(), theme::INK_FAINT),
        (_, true) => (
            format!("the database row is entry {} until applied", shown.read_entry),
            theme::WARN,
        ),
        _ => (
            "changes the row's entry and every world database reference to it".to_string(),
            theme::INK_FAINT,
        ),
    };
    ui.label(egui::RichText::new(word).small().color(colour)).on_hover_text(format!(
        "vmangos has no foreign keys, so Apply updates the references itself: the row, \
         and the {} world database columns that refer to a game object by entry: every \
         spawn's id, the quest relations, the pool and locale rows, the spell script \
         targets, and quest_template's objectives, which store an object as the negative \
         of its entry. Restore reverts all of it.\n\nNot updated: a template's \
         linkedTrapId data column, gameobject_loot_template (keyed by data1, not by \
         entry), script datalong values, and the C++ scripts.",
        vale_mangos::gameobject::TEMPLATE_REFERENCES.len()
    ));
}

/// Which row a form is over.
struct Form<'a> {
    table: &'static str,
    key: &'a Key,
    row: &'a gameobject::Row,
    /// The selected spawn, for `orientation`, the one column whose write
    /// depends on it; see [`field`]. `None` for the template window, which
    /// may be about an object with no spawn.
    spawn: Option<&'a Spawn>,
}

impl Form<'_> {
    /// The template's type as the form shows it: the project's value if it has
    /// one, otherwise the database's. It names the `data` columns, so an edit
    /// to `type` renames them on the next frame.
    fn shown_type(&self, session: &EditSession) -> u32 {
        session
            .server_edits
            .get(self.table, self.key, "type")
            .and_then(|value| value.trim().parse().ok())
            .or_else(|| self.row.integer("type").map(|kind| kind as u32))
            .unwrap_or(0)
    }

    /// What one column holds right now, as a number.
    fn shown_number(&self, session: &EditSession, column: &str) -> i64 {
        session
            .server_edits
            .get(self.table, self.key, column)
            .and_then(|value| value.trim().parse::<f64>().ok())
            .or_else(|| self.row.number(column))
            .unwrap_or(0.0) as i64
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
fn groups(ui: &mut egui::Ui, subject: &mut Subject<'_>, form: &Form<'_>, open: &[Group]) {
    page_spacing(ui);
    let kind = form.shown_type(subject.session);
    let mut seen: Vec<Group> = Vec::new();
    for column in gameobject::columns_of(form.table) {
        if !seen.contains(&column.group) {
            seen.push(column.group);
        }
    }
    for group in seen {
        let columns: Vec<Drawn> = gameobject::columns_of(form.table)
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
            .server_edits
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

/// One column on a row of its own: its name, the control its kind asks for,
/// what the value means, and the revert arrow when this project has changed
/// it.
fn field(ui: &mut egui::Ui, subject: &mut Subject<'_>, form: &Form<'_>, drawn: &Drawn) {
    let (table, key, column) = (form.table, form.key, drawn.column);
    let edited = subject
        .session
        .server_edits
        .get(table, key, column.name)
        .map(str::to_string);
    let in_database = column.literal(form.row);
    let showing = edited.clone().or_else(|| in_database.clone());
    // No revert arrow on a row this project creates: every column of it is
    // the project's own, and the database has no value to revert to.
    let creating = subject.session.server_edits.life(table, key) == Life::Insert;
    // On an upright object a spawn's facing is written to three columns. See
    // `GameObjects::turn`.
    let turns = table == gameobject::SPAWN && column.name == "orientation";

    let id = ui.make_persistent_id((table, key.text(), column.name));
    let pending = page_row(ui, &drawn.title, &drawn.about, edited.is_some(), |ui| {
        let Some(showing) = showing.as_deref() else {
            ui.label(egui::RichText::new("NULL").color(theme::INK_FAINT));
            return None;
        };
        // A reference whose name did not fit beside its number, to be drawn
        // on the row after this one.
        let mut pending: Option<(&'static str, u32)> = None;
        let written = match drawn.kind {
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
            // A lock is resolved through the client's own table, in words,
            // and has no picker: `Lock.dbc` carries no name to search.
            Kind::Ref("Lock") => {
                let written = number_cell(ui, Kind::Unsigned, showing);
                let lock = showing.trim().parse::<u32>().unwrap_or(0);
                let words = match lock {
                    0 => "no lock".to_string(),
                    lock => lock_words(locks(&mut subject.objects.locks, subject.assets), lock),
                };
                meaning_truncated(ui, words);
                written
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
        if let Some(written) = written {
            let turning = form.spawn.filter(|_| turns);
            match (turning, written.trim().parse::<f32>()) {
                (Some(spawn), Ok(orientation)) => GameObjects::turn(
                    subject.session,
                    spawn.guid,
                    spawn.tilted,
                    orientation,
                    subject.now,
                ),
                _ => target(table, key, column, &in_database, creating).write(
                    subject.session,
                    written,
                    subject.now,
                ),
            }
        }
        if edited.is_some() && !creating && revert_button(ui, in_database.as_deref()) {
            let gesture = format!("{table} {} {} revert", key.text(), column.name);
            // Reverting the facing also reverts rotation2 and rotation3,
            // because the edit wrote them too.
            let upright = form.spawn.is_some_and(|spawn| !spawn.tilted);
            let columns: &[&str] = match turns && upright {
                true => &["orientation", "rotation2", "rotation3"],
                false => std::slice::from_ref(&column.name),
            };
            for name in columns {
                subject.session.set_server_edit(
                    table,
                    key,
                    name,
                    None,
                    Some(crate::session::Gesture {
                        label: "Revert column",
                        subject: &gesture,
                        now: subject.now,
                    }),
                );
            }
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

/// Where a value typed or chosen for a column is written; see
/// `super::creatures::target`, which is the same for the other table.
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
        label: "Edit game object",
        subject: format!("{table} {} {}", key.text(), column.name),
    }
}

/// A display id: the number, a picture of the model it names, the button
/// that opens the picker, and the model's path.
fn display_cell(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    showing: &str,
    target: ColumnTarget,
) -> Option<String> {
    let written = number_cell(ui, Kind::Unsigned, showing);
    let id: u32 = showing.trim().parse().unwrap_or(0);
    let path = subject.objects.model_of(subject.assets, id);
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(FORM_ROW + 4.0), egui::Sense::hover());
    match path.as_deref() {
        Some(path) => crate::portraits::paint(ui, subject.portraits, path, rect),
        None if id != 0 => {
            ui.painter().rect_filled(rect, 3.0, theme::DEAD);
        }
        None => {}
    }
    if ui
        .small_button("choose\u{2026}")
        .on_hover_text(
            "Opens a picker of every GameObjectDisplayInfo.dbc row that has a model, \
             shown as thumbnails and searchable by model path.",
        )
        .clicked()
    {
        subject.objects.display_pick = Some(DisplayPick::open(DisplayTable::Object, target, id));
    }
    match path {
        Some(path) => meaning_truncated(ui, path),
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
        gameobject::column(gameobject::TEMPLATE, name).expect("a column")
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
        for column in gameobject::columns_of(gameobject::SPAWN) {
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

    /// A chosen value and a typed value for one column share one undo
    /// subject, so the two fold into one entry; a row this project creates
    /// carries no database value for the write to clear against.
    #[test]
    fn a_target_names_the_row_and_the_column() {
        let key = gameobject::template_key(2000000, 10);
        let held = Some("42".to_string());
        let target = target(gameobject::TEMPLATE, &key, column("displayId"), &held, false);
        assert_eq!(target.column, "displayId");
        assert_eq!(target.in_database.as_deref(), Some("42"));
        assert_eq!(target.subject, format!("gameobject_template {} displayId", key.text()));
        let created = target_for_created(&key, &held);
        assert_eq!(created.in_database, None);
    }

    fn target_for_created(key: &Key, held: &Option<String>) -> ColumnTarget {
        target(gameobject::TEMPLATE, key, column("displayId"), held, true)
    }
}
