//! What the creature and game object panels draw the same way, written once
//! over the subject. The view of [`crate::tools::spawns::spawn`].
//!
//! ```text
//! viewport toolbar   Select or Place, the handles, which spawns are drawn and
//!                    how, Reload, and the keys of the chosen mode
//! inspector          the map's counts and the project's state on the server;
//!                    in Place, the template picker; in Select, the group
//!                    line, then the selected spawn's Spawn and Location
//!                    sections and whatever the subject adds
//! ```
//!
//! [`SpawnView`] is what a subject's panel adds: the pictures, the lines a
//! template is described by, the sections a selected spawn is drawn in, the
//! template's form, and the sentences that name its own tables. The forms
//! themselves differ (a game object's `data` columns are named by its type)
//! and stay in [`super::creatures`] and [`super::gameobjects`]. This file also
//! holds the template window with its entry box.

use bevy_egui::egui;

use crate::session::EditSession;
use crate::subjects::listing::Listed;
use crate::tools::placements::place;
use crate::tools::spawns::spawn::{SpawnSubject, SpawnTool, Spawned, Template, TemplateSubject};
use crate::ui::shell::asks::{ShellAsk, ShellAsks};
use crate::ui::widgets::rowform::{entry_cell, Renumber};
use crate::ui::widgets::theme::{self, property};
use crate::ui::Editing;
use vale_client::assets::GameAssets;
use vale_mangos::row::{Key, Life};

/// Everything a spawn panel needs, as one argument.
pub(crate) struct Subject<'a, S: SpawnView> {
    pub(crate) session: &'a mut EditSession,
    /// The tool's state: the spawns on the map, the selection, the picker
    /// and the two whole rows the forms draw.
    pub(crate) tool: &'a mut SpawnTool<S>,
    /// The quest tool's state: which template's quests its window is showing
    /// (see `crate::ui::subjects::quests::holder_window`), the reference
    /// picker every form shares, and the name cache a reference column
    /// resolves through.
    pub(crate) quests: &'a mut crate::subjects::quests::Quests,
    /// The loot window's state, for the Loot button beside Quests. See
    /// `crate::ui::subjects::loot::window`.
    pub(crate) loot: &'a mut crate::subjects::loot::Loot,
    /// Where this machine's server is. The panel's sentences read it, and it
    /// decides whether there is anything to say.
    pub(crate) server: &'a crate::server::settings::ServerSettings,
    /// The top bar's Server… popover, which holds every server operation.
    /// A panel that reports an edit as not applied must be able to open the
    /// panel that applies it. See [`crate::ui::shell::sync`].
    pub(crate) server_panel: &'a mut crate::ui::shell::popover::Popover,
    /// Where each server subject stands, held for half a second: the
    /// subject's line in [`overview`] and whether the selected spawn is
    /// applied are read from it rather than planned each frame. See
    /// [`crate::server::sync::Standings`].
    pub(crate) standings: &'a mut crate::server::sync::Standings,
    /// The archives, for what a display id looks like and for the client
    /// tables a reference resolves through.
    pub(crate) assets: &'a GameAssets,
    /// The pictures drawn in the picker's rows, the form's display cells and
    /// the preview panes. See `crate::preview::portraits`.
    pub(crate) portraits: &'a mut crate::preview::portraits::Portraits,
    /// The icons the reference picker's spell and item rows draw. See
    /// `crate::ui::widgets::thumbnails`.
    pub(crate) thumbnails: &'a mut crate::ui::widgets::thumbnails::Thumbnails,
    /// The frame clock the undo stack folds a gesture by. See
    /// `crate::server::store::Gesture`. Typing into a name box is one entry
    /// rather than one per keystroke because every keystroke names the same
    /// subject inside the fold window.
    pub(crate) now: f64,
    /// The gizmo, for the Handles switch on the toolbar. `None` where the
    /// panel's parts are drawn without it, as in the template window.
    pub(crate) gizmo: Option<&'a mut crate::tools::placements::gizmo::Gizmo>,
    /// What a link on the panel asks of the shell: a row to open in another
    /// workspace, a tool to switch to. See [`crate::ui::shell::asks`].
    pub(crate) asks: &'a mut ShellAsks,
    /// Where the Go To button sends its request. See
    /// [`crate::camera::FlyTo`].
    pub(crate) fly: &'a mut bevy::prelude::Messages<crate::camera::FlyTo>,
    /// Whether the other spawn tool's spawns are drawn too. See
    /// [`crate::tools::spawns::spawn::OtherSpawns`].
    pub(crate) others: &'a mut crate::tools::spawns::spawn::OtherSpawns,
    /// What only this subject's panel reaches.
    pub(crate) own: S::Panel<'a>,
}

/// The sentences a subject's panel says that are its own.
pub(crate) struct Text {
    /// The subject's template in a heading: `Creature Template`.
    pub(crate) heading: &'static str,
    /// The subject's rows in a sentence's first word: `Creatures`.
    pub(crate) title: &'static str,
    /// Its workspace on the top bar: `Creatures`, `Game Objects`.
    pub(crate) workspace: &'static str,
    /// The panel's note while nothing is selected.
    pub(crate) click: &'static str,
    /// The Models switch's hover.
    pub(crate) models: &'static str,
    /// The model limit's hover, and its largest value.
    pub(crate) budget: &'static str,
    pub(crate) budget_max: usize,
    /// What Apply's removal of a spawn takes with it, before and after the
    /// apply.
    pub(crate) removal: &'static str,
    pub(crate) removed: &'static str,
    /// The New button's hover, in the picker and in the workspace.
    pub(crate) new_template: &'static str,
    pub(crate) new_listed: &'static str,
    /// The button that opens the template window, and its hover in the
    /// picker and under a selected spawn.
    pub(crate) edit: &'static str,
    pub(crate) edit_chosen: &'static str,
    pub(crate) edit_selected: &'static str,
    /// The picker's note while nothing is chosen, and while the search is
    /// too short to send.
    pub(crate) none_chosen: &'static str,
    pub(crate) too_short: &'static str,
    /// The note under a chosen template with no model, if the subject says
    /// one.
    pub(crate) no_model: Option<&'static str>,
    /// The workspace's search box hint.
    pub(crate) list_search: &'static str,
    /// The workspace inspector's note for a template with no model.
    pub(crate) no_display: &'static str,
    /// The workspace's Place in World hover.
    pub(crate) place: &'static str,
}

/// What a subject's panel adds to the shared one.
pub(crate) trait SpawnView: SpawnSubject {
    /// What only this subject's panel reaches beside [`Subject`]'s fields.
    type Panel<'a>;
    /// The subject's line on the bar's Server….
    const STACK: crate::server::stack::Subject;
    /// The SQL file the subject's edits are written to, and the one that
    /// puts them back.
    const SQL: &'static str;
    const REVERT: &'static str;
    const TEXT: Text;

    /// The panel's subject out of the shell's state. `gizmo` is whether the
    /// subject carries the gizmo, which only the toolbar uses.
    fn subject<'a>(
        session: &'a mut EditSession,
        editing: &'a mut Editing<'_>,
        server: super::Server<'a>,
        assets: &'a GameAssets,
        now: f64,
        gizmo: bool,
    ) -> Subject<'a, Self>;
    /// Paint a small picture of a display id into `rect`. Answers whether the
    /// display resolved.
    fn picture(ui: &mut egui::Ui, subject: &mut Subject<'_, Self>, display_id: u32, rect: egui::Rect) -> bool;
    /// Draw the large preview of a display id, `side` points wide, or the
    /// preview pane's own width. Answers whether the display resolved.
    fn preview(ui: &mut egui::Ui, subject: &mut Subject<'_, Self>, display_id: u32, side: Option<f32>) -> bool;
    /// The line under a template's name in the picker.
    fn row_line(subject: &mut Subject<'_, Self>, known: &Self::Known) -> String;
    /// The line under a template's name in the workspace's list.
    fn list_line(subject: &mut Subject<'_, Self>, known: &Self::Known) -> String;
    /// What the picker says about the chosen template under its name.
    fn chosen_detail(ui: &mut egui::Ui, subject: &mut Subject<'_, Self>, chosen: &Self::Known);
    /// The picker's filters under the search box.
    fn search_filters(_ui: &mut egui::Ui, _subject: &mut Subject<'_, Self>) {}
    /// What switching to Place does beside arming the picker.
    fn on_place(_subject: &mut Subject<'_, Self>) {}
    /// What Reload forgets beside the map.
    fn reloaded(_subject: &mut Subject<'_, Self>) {}
    /// What the toolbar holds after the other spawn type's switch.
    fn beside_markers(_ui: &mut egui::Ui, _subject: &mut Subject<'_, Self>) {}
    /// The selected spawn's sections under the group line: its name, the
    /// shared Spawn and Location sections ([`spawn_sections`]), what its
    /// template is, the buttons that open its windows, and its form.
    fn selected(
        ui: &mut egui::Ui,
        subject: &mut Subject<'_, Self>,
        spawn: &Self::Spawn,
        on_server: &crate::server::sync::Standing,
    );
    /// The template row's form, in the template window and the workspace.
    fn template_form(ui: &mut egui::Ui, subject: &mut Subject<'_, Self>, key: &Key, row: &vale_mangos::schema::Row);
    /// What a renumber reaches, for the entry box's hover.
    fn renumber_reaches() -> String;
    /// The summary rows of the workspace's inspector.
    fn list_summary(ui: &mut egui::Ui, subject: &mut Subject<'_, Self>, known: &Self::Known);
    /// The windows a template has besides its row, in the workspace's
    /// inspector.
    fn list_windows(ui: &mut egui::Ui, subject: &mut Subject<'_, Self>, known: &Self::Known);
}

/// Draw a subject's panel, and the two dialogs its forms open.
pub(crate) fn draw<S: SpawnView>(ui: &mut egui::Ui, mut subject: Subject<'_, S>) {
    panel(ui, &mut subject);
    dialogs(ui.ctx(), &mut subject);
}

/// A spawn tool's inspector panel. The server's spawns are picked in the
/// viewport like a light, so their form is drawn in the inspector. It shows
/// two rows: the spawn that was clicked and the template behind it.
pub(crate) fn inspect<S: SpawnView>(
    ui: &mut egui::Ui,
    subject: crate::ui::tools::Subject<'_>,
    editing: &mut crate::ui::Editing<'_>,
) {
    let crate::ui::tools::Subject { session, assets, now, server, server_panel, standings, .. } = subject;
    draw(
        ui,
        S::subject(session, editing, super::Server { settings: server, panel: server_panel, standings }, assets, now, false),
    );
}

/// The two dialogs a form here can open: the display picker and the
/// reference picker. Drawn against the context, so the sidebar, the template
/// window and the workspace reach the same two.
pub(in crate::ui) fn dialogs<S: SpawnView>(ctx: &egui::Context, subject: &mut Subject<'_, S>) {
    crate::ui::subjects::displays::window(
        ctx,
        &mut subject.tool.display_pick,
        subject.session,
        subject.assets,
        subject.portraits,
        subject.now,
    );
    crate::ui::subjects::quests::picker(
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
fn panel<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>) {
    if subject.tool.trouble.is_some() {
        overview(ui, subject, None);
        ui.add_space(8.0);
        nothing_to_show(ui, subject);
        return;
    }

    // What this project changes and how much of it is in the database are
    // read from the held standing, which is recomputed at most every half
    // second, and passed to both the summary and the selected spawn's State.
    // Computing them is a walk of the store and a file read.
    let on_server = subject
        .standings
        .of(crate::server::sync::Half::Row(S::STACK), subject.session, subject.assets, subject.now)
        .clone();
    overview(ui, subject, Some(&on_server));
    ui.add_space(8.0);

    if subject.tool.mode == place::Mode::Place {
        picker(ui, subject);
        return;
    }

    if subject.tool.spawn_count() == 0 {
        nothing_to_show(ui, subject);
        return;
    }
    let Some(spawn) = subject.tool.chosen_edited(Some(&subject.session.rows.edits)) else {
        title(ui, "No Selection", None);
        theme::note(ui, S::TEXT.click);
        return;
    };

    let asked = {
        let tool = &*subject.tool;
        let edits = Some(&subject.session.rows.edits);
        let primary = spawn.guid();
        let member = |index: usize| {
            let guid = match index {
                0 => primary,
                _ => tool.also[index - 1],
            };
            crate::ui::widgets::members::Member {
                id: guid,
                label: tool.edited(guid, edits).map(|spawn| spawn.label()).unwrap_or_else(|| "Not on this map".to_string()),
                detail: format!("GUID {guid}"),
            }
        };
        group_line(ui, tool.also.len() + 1, S::WORDS.nouns, member)
    };
    match asked {
        Some(GroupAsk::Only) => subject.tool.also.clear(),
        Some(GroupAsk::Clear) => subject.tool.select_only(None),
        Some(GroupAsk::Primary(guid)) => subject.tool.promote(guid),
        Some(GroupAsk::Drop(guid)) => subject.tool.also.retain(|&had| had != guid),
        None => {}
    }
    S::selected(ui, subject, &spawn, &on_server);
}

/// The size of a selection's title.
const TITLE: f32 = 17.0;

/// The selection's title, one line cut with an ellipsis, and a smaller line
/// under it when there is one.
pub(super) fn title(ui: &mut egui::Ui, text: &str, sub: Option<&str>) {
    ui.add(egui::Label::new(egui::RichText::new(text).strong().size(TITLE).color(theme::INK)).truncate());
    if let Some(sub) = sub.filter(|sub| !sub.is_empty()) {
        ui.add(egui::Label::new(egui::RichText::new(sub).size(theme::SMALL).color(theme::INK_DIM)).truncate());
    }
}

/// A property's value that is not edited here, cut to the room left.
pub(super) fn value(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    value_in(ui, text, theme::INK)
}

/// [`value`] in another colour.
pub(super) fn value_in(ui: &mut egui::Ui, text: impl Into<String>, colour: egui::Color32) -> egui::Response {
    ui.add(egui::Label::new(egui::RichText::new(text.into()).color(colour)).truncate())
}

/// What the line about a group of spawns asked for.
pub(super) enum GroupAsk {
    /// Keep the spawn whose form is open and drop the rest.
    Only,
    /// Drop the whole selection.
    Clear,
    /// Make this member the primary, from the list of members.
    Primary(u64),
    /// Take this member out of the group, from the list of members.
    Drop(u64),
}

/// The block over a spawn's form when several spawns are selected: how many,
/// that the form is the primary's, the two buttons that shrink the group, and
/// the list of members (`crate::ui::widgets::members`), of which `member(0)` is
/// the primary. Nothing is drawn for one. See `crate::tools::placements::group`.
pub(super) fn group_line(
    ui: &mut egui::Ui,
    count: usize,
    noun: &str,
    member: impl Fn(usize) -> crate::ui::widgets::members::Member,
) -> Option<GroupAsk> {
    if count < 2 {
        return None;
    }
    let mut asked = None;
    theme::heading(ui, "Selection");
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("{count} {noun} selected")).color(theme::INK));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Clear").on_hover_text("Deselect every spawn.").clicked() {
                asked = Some(GroupAsk::Clear);
            }
            if ui
                .small_button("Keep Primary")
                .on_hover_text("Keep the highlighted spawn selected and deselect the rest.")
                .clicked()
            {
                asked = Some(GroupAsk::Only);
            }
        });
    });
    theme::note(
        ui,
        "The properties below are the highlighted spawn's. Dragging, the handles, the keys and \
         Delete act on every selected spawn.",
    );
    match crate::ui::widgets::members::list(ui, noun, count, member) {
        Some(crate::ui::widgets::members::Pick::Primary(guid)) => asked = Some(GroupAsk::Primary(guid)),
        Some(crate::ui::widgets::members::Pick::Drop(guid)) => asked = Some(GroupAsk::Drop(guid)),
        None => {}
    }
    ui.add_space(6.0);
    asked
}

/// The block at the top of the panel: how many spawns the map has, Reload,
/// and what this project changes about the subject's rows with the button
/// that opens the panel that applies it. `on_server` is `None` while the
/// tool has no connection.
///
/// The project's state is drawn whether or not a spawn is selected, because
/// what the project has changed is a question about the project rather than
/// about the selection.
///
/// The operations are not here. Apply, Restore and Discard are on the top
/// bar's Server… popover, beside the other subjects' and in the same words;
/// [`crate::ui::shell::sync`]'s module comment gives the reason for one panel.
/// This block shows only the state, so that a person looking at a spawn can
/// tell whether what they see is in the database without opening another
/// panel.
///
/// The standing's `outstanding` counts this project's rows and paths not yet
/// in the database; without that count the state line read the same whether
/// nothing had been applied or everything had.
fn overview<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>, on_server: Option<&crate::server::sync::Standing>) {
    let spawns = subject.tool.spawns.len();
    let mine = subject.tool.created.len();
    let near = subject.tool.near.len();
    egui::Frame::new()
        .fill(theme::SUNK)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                match subject.tool.reading() {
                    true => {
                        ui.label(egui::RichText::new("Loading spawns\u{2026}").size(theme::SMALL).color(theme::INK_DIM));
                    }
                    false => {
                        count(ui, spawns, "on this map");
                        dot_between(ui);
                        count(ui, near, "near");
                        // The project's own are not in the read's count and
                        // are drawn beside the ones that are, so they are
                        // counted here.
                        if mine > 0 {
                            dot_between(ui);
                            count(ui, mine, "new");
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Reload").on_hover_text("Read the map's spawns from the database again.").clicked() {
                        subject.tool.forget();
                        S::reloaded(subject);
                    }
                });
            });
            if let Some(on_server) = on_server {
                project_state(ui, subject, on_server);
            }
        });
}

/// A number and what it counts, the number in the body colour.
fn count(ui: &mut egui::Ui, n: usize, what: &str) {
    ui.label(theme::number(n.to_string()));
    ui.label(egui::RichText::new(what).size(theme::SMALL).color(theme::INK_DIM));
}

/// The faint dot between two counts.
fn dot_between(ui: &mut egui::Ui) {
    ui.label(egui::RichText::new("\u{b7}").color(theme::INK_FAINT));
}

/// The project's part of [`overview`]: the summary and the SQL file, the
/// state on the server in one line, and the Server… button.
fn project_state<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>, on_server: &crate::server::sync::Standing) {
    let undoable = on_server.applied;
    let empty = on_server.changed == 0;
    if empty && undoable == 0 {
        return;
    }
    let outstanding = on_server.outstanding;
    ui.add_space(2.0);
    // The project's state is two independent facts, and the line states both.
    // A revert puts the database back and leaves the project still claiming
    // the rows, so the editor goes on drawing the edited values.
    let (state, colour, about) = match (undoable, outstanding, empty) {
        (0, _, false) => (
            "Not applied".to_string(),
            theme::WARN,
            "The database still holds its previous values; the editor draws this project's values.".to_string(),
        ),
        // Everything this project claims is in the database. This case has
        // its own words so that it can be told from the case above.
        (n, 0, false) => match on_server.current {
            true => (
                format!("Applied ({n} rows); restart the server"),
                theme::INK_DIM,
                format!("All {n} rows are in the database. Restart the server to see them; Restore reverts them."),
            ),
            // The signature is per process, so a project also reads this way
            // after a relaunch. See `EditSession::applied`.
            false => (
                format!("Applied ({n} rows); may be out of date"),
                theme::WARN,
                format!(
                    "All {n} rows are in the database, possibly with older values: an edit was \
                     made since, or this is a later session. Apply again to make sure."
                ),
            ),
        },
        (n, out, false) => (
            format!("{out} not applied; {n} applied"),
            theme::WARN,
            format!("{out} changes are not in the database yet; {n} rows are in the database and can be restored."),
        ),
        (n, _, true) => (
            format!("{n} applied rows no longer changed"),
            theme::WARN,
            format!("{n} rows are applied and can be restored; the project no longer changes any of them."),
        ),
    };
    // The Server… button is laid out first, from the right, so the summary is
    // cut to the room left beside it rather than drawn under it.
    property(ui, "Project", &format!("{}, written to {}.", on_server.summary, S::SQL), |ui| {
        let colour = match outstanding == 0 {
            true => theme::INK,
            false => theme::WARN,
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("Server\u{2026}")
                .on_hover_text(format!(
                    "Open the Server panel, which holds Apply, Restore and Discard for every \
                     subject. Apply writes the rows only: the server reads {} once, at startup, \
                     so restart it to see them. Restore runs {}.",
                    S::WORDS.nouns,
                    S::REVERT
                ))
                .clicked()
            {
                subject.server_panel.show();
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                value_in(ui, on_server.summary.clone(), colour);
            });
        });
    });
    ui.add(egui::Label::new(egui::RichText::new(state).size(theme::SMALL).color(colour)).wrap())
        .on_hover_text(about);
    for refused in &on_server.refused {
        ui.label(egui::RichText::new(refused).size(theme::SMALL).color(theme::BAD));
    }
}

/// Why the list is empty: no connection, still reading, or no spawn on the
/// map.
fn nothing_to_show<S: SpawnView>(ui: &mut egui::Ui, subject: &Subject<'_, S>) {
    match &subject.tool.trouble {
        Some(trouble) => {
            ui.label(egui::RichText::new(trouble).size(theme::SMALL).color(theme::WARN));
            theme::note(
                ui,
                format!(
                    "{} are rows in vmangos' database rather than files in the archives, so \
                     this tool needs a database connection. Set it in Server… on the top bar.",
                    S::TEXT.title
                ),
            );
        }
        None if subject.tool.reading() => {
            theme::waiting(ui, "Loading\u{2026}");
        }
        None => {
            theme::note(ui, format!("No {} spawns on this map.", S::WORDS.noun));
        }
    }
}

/// The toolbar over the viewport while a spawn tool is chosen:
///
/// ```text
/// Select  Place | Handles  Move  Turn  Off | Models  Max  Markers  <other type>  <own switches> | Reload
/// the keys of the chosen mode
/// ```
///
/// Select or Place is the placement tools' own switch; see
/// `crate::tools::placements::place::Mode` for why it is a mode rather than a
/// second rail entry. The handles are drawn only in Select, the one mode with
/// a selection to move. See [`theme::toolbar`].
pub(crate) fn toolbar<S: SpawnView>(
    ctx: &egui::Context,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    subject: &mut Subject<'_, S>,
) -> Option<egui::Rect> {
    use crate::tools::placements::gizmo::{Handles, HANDLES};
    let id = format!("{}-toolbar", S::SPAWN);
    theme::toolbar(ctx, &id, viewport, corner, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for (name, mode) in place::MODES {
                let hover = match mode {
                    place::Mode::Select => "Click a spawn to select it, and drag to move it.",
                    place::Mode::Place => "Choose a template in the inspector, then click the ground to create a spawn of it.",
                };
                if toggle(ui, subject.tool.mode == mode, name).on_hover_text(hover).clicked() && subject.tool.mode != mode {
                    subject.tool.mode = mode;
                    // Switching away puts down whatever was on the cursor. The
                    // tool's `ghost` also does this; doing it here as well
                    // makes the panel correct on the frame of the switch
                    // rather than on the frame after.
                    match mode {
                        place::Mode::Select => subject.tool.new_spawn.chosen = None,
                        place::Mode::Place => S::on_place(subject),
                    }
                }
            }
            if subject.tool.mode == place::Mode::Select {
                if let Some(gizmo) = subject.gizmo.as_deref_mut() {
                    gap(ui);
                    ui.label(egui::RichText::new("Handles").color(theme::INK_DIM));
                    // A spawn has no tilt, so Turn offers the z ring alone;
                    // see `crate::tools::spawns::spawn`.
                    for (name, handles) in HANDLES {
                        let hover = match handles {
                            Handles::Move => "Drag an arrow to move the spawn along one axis.",
                            Handles::Turn => "Drag the ring to turn the spawn about Z.",
                            Handles::Off => "No handles: the pointer picks and drags on the ground.",
                        };
                        if toggle(ui, gizmo.handles == handles, name).on_hover_text(hover).clicked() {
                            gizmo.handles = handles;
                        }
                    }
                }
            }
            gap(ui);
            if toggle(ui, subject.tool.show_models, "Models").on_hover_text(S::TEXT.models).clicked() {
                subject.tool.show_models = !subject.tool.show_models;
            }
            ui.add_enabled(
                subject.tool.show_models,
                egui::DragValue::new(&mut subject.tool.model_budget).speed(5.0).range(0..=S::TEXT.budget_max).prefix("Max "),
            )
            .on_hover_text(S::TEXT.budget);
            if toggle(ui, subject.tool.show_markers, "Markers")
                .on_hover_text(
                    "Mark every spawn near the camera. Off, only the selected, hovered and \
                     grouped spawns are marked, which saves frame time where many spawns are \
                     near. Spawns can still be clicked.",
                )
                .clicked()
            {
                subject.tool.show_markers = !subject.tool.show_markers;
            }
            let other = S::KIND.other();
            let label = match other {
                crate::tools::spawns::spawn::SpawnKind::Creature => "Creatures",
                crate::tools::spawns::spawn::SpawnKind::GameObject => "Objects",
            };
            if toggle(ui, subject.others.shown, label)
                .on_hover_text(format!(
                    "Mark the {} spawns near the camera too, faded. A click on one selects it \
                     and switches to its tool.",
                    other.word()
                ))
                .clicked()
            {
                subject.others.shown = !subject.others.shown;
            }
            S::beside_markers(ui, subject);
        });
        let keys = match subject.tool.mode {
            place::Mode::Select => "Click: select \u{b7} Shift + click: add or remove \u{b7} Drag on empty ground: \
                                    select an area \u{b7} Arrows: nudge \u{b7} Page Up/Down: raise \u{b7} Shift: ten \
                                    times \u{b7} , and .: turn \u{b7} Alt + mouse: turn \u{b7} Ctrl + Alt: 15\u{b0} \
                                    steps \u{b7} Delete: remove \u{b7} Ctrl + D: duplicate"
                .to_string(),
            place::Mode::Place => format!(
                "Click the ground: place the chosen {} \u{b7} , and .: turn \u{b7} Shift: three times as far \u{b7} \
                 Alt + mouse: turn \u{b7} Ctrl + Alt: 15\u{b0} steps \u{b7} Esc: clear the choice",
                S::WORDS.noun
            ),
        };
        ui.add(egui::Label::new(egui::RichText::new(keys).size(theme::SMALL).color(theme::INK_FAINT)).wrap());
    })
}

/// A toolbar button that shows whether it is on.
pub(super) fn toggle(ui: &mut egui::Ui, on: bool, text: &str) -> egui::Response {
    ui.add(egui::Button::selectable(on, text).min_size(egui::vec2(0.0, 24.0)))
}

/// The space and rule between two groups of toolbar controls.
fn gap(ui: &mut egui::Ui) {
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(2.0);
}

/// The Place mode: choose a template, then click the ground.
///
/// It has the same shape as the placement tools; see
/// [`crate::tools::placements::place`] for why this is a mode rather than a
/// second rail entry. Only what is chosen differs: a doodad is a path in the
/// archives and a spawn's template is a row of the template table, so the list
/// comes from a query rather than from a listing, and the search covers names
/// as well as entries.
///
/// There is no Place button. The mode arms the pointer with the chosen
/// template, so clicking a row is the last step before clicking the ground. A
/// separate button press would add a step to every placement and give two
/// ways to reach the same state.
fn picker<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>) {
    match subject.tool.new_spawn.chosen.clone() {
        Some(chosen) => {
            title(ui, &chosen.label(), None);
            S::chosen_detail(ui, subject, &chosen);
            if chosen.is_new() {
                ui.label(
                    egui::RichText::new("New: this project creates it. There is no row in the database yet.")
                        .size(theme::SMALL)
                        .color(theme::WARN),
                );
            }
            theme::note(
                ui,
                format!(
                    "Click the ground to place it. Every column of the new spawn can be edited \
                     afterwards in Select. Esc clears the chosen {}.",
                    S::WORDS.noun
                ),
            );
            // The window's toggle, here as well as under a selected spawn, so
            // a template can be edited before any spawn of it exists.
            ui.horizontal_wrapped(|ui| {
                let open = subject.tool.template_window;
                if ui.add(egui::Button::new(S::TEXT.edit).selected(open)).on_hover_text(S::TEXT.edit_chosen).clicked() {
                    subject.tool.template_window = !open;
                }
                open_in_templates(ui, subject, chosen.entry());
            });
            // The large preview, the same pane the placement pickers use. See
            // `crate::preview::portraits`.
            ui.add_space(4.0);
            if !S::preview(ui, subject, chosen.display_id(), None) {
                if let Some(note) = S::TEXT.no_model {
                    theme::note(ui, note);
                }
            }
        }
        None => {
            title(ui, "No Template Chosen", None);
            theme::note(ui, S::TEXT.none_chosen);
        }
    }

    theme::heading(ui, S::TEXT.heading);
    template_actions(ui, subject);
    created_templates(ui, subject);

    theme::heading(ui, "Search");
    ui.add(
        egui::TextEdit::singleline(&mut subject.tool.new_spawn.search)
            .hint_text("Name or entry")
            .desired_width(f32::INFINITY),
    );
    S::search_filters(ui, subject);
    if let Some(trouble) = subject.tool.new_spawn.trouble.clone() {
        ui.label(egui::RichText::new(trouble).size(theme::SMALL).color(theme::BAD));
    }
    if subject.tool.new_spawn.searching() {
        theme::waiting(ui, "Searching\u{2026}");
        return;
    }
    // The search is sent with two letters, or with none once a type is
    // chosen; see `crate::tools::spawns::spawn::systems::search`.
    if subject.tool.new_spawn.search.trim().len() < 2 && subject.tool.new_spawn.of_type.is_none() {
        theme::note(ui, S::TEXT.too_short);
        return;
    }
    let matches = subject.tool.new_spawn.matches.clone();
    if matches.is_empty() {
        theme::note(ui, format!("No matching {}.", S::WORDS.nouns));
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt((S::TEMPLATE, "new-spawn-matches"))
        .max_height(MATCH_LIST_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for known in &matches {
                let on = subject.tool.new_spawn.chosen.as_ref().is_some_and(|had| had.entry() == known.entry());
                if template_row(ui, subject, known, on) {
                    subject.tool.new_spawn.chosen = Some(known.clone());
                }
            }
        });
    if matches.len() >= 50 {
        theme::note(ui, "Showing the first 50 matches. Type more of the name to narrow the list.");
    }
}

/// The height the picker's match list may take, so the preview pane above it
/// keeps its room.
const MATCH_LIST_HEIGHT: f32 = 260.0;

/// The three buttons of the picker's template section: a new template, a copy
/// of the one the window is about, and giving up a created one. The
/// workspace's list has the same row.
fn template_actions<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>) {
    let now = subject.now;
    let patch = crate::server::settings::server_patch(subject.server);
    let shown = subject.tool.template_subject(&subject.session.rows.edits);
    let row_read = shown
        .as_ref()
        .is_some_and(|shown| subject.tool.template.as_ref().is_some_and(|held| held.entry == shown.entry));
    let created = shown.as_ref().is_some_and(TemplateSubject::is_new);
    let noun = S::WORDS.noun;
    ui.horizontal(|ui| {
        let third = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(40.0);
        let size = egui::vec2(third, 22.0);
        if ui.add(egui::Button::new("New").min_size(size)).on_hover_text(S::TEXT.new_template).clicked() {
            let entry = subject.tool.create_template(subject.session, S::WORDS.new_name, patch, now);
            subject.session.status = format!("{} {entry} created", S::TEMPLATE);
        }
        if ui
            .add_enabled(row_read, egui::Button::new("Copy").min_size(size))
            .on_hover_text(format!(
                "Copy the chosen {noun} to a new entry with every column as shown: the \
                 database value where this project has not changed it, and the project's \
                 value where it has."
            ))
            .on_disabled_hover_text(format!("Choose a {noun} first."))
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject.session.status = match subject.tool.copy_template(subject.session, shown, patch, now) {
                    Some(entry) => format!("Copied as {} {entry}", S::TEMPLATE),
                    None => format!("Still reading this {noun}'s row"),
                };
            }
        }
        if ui
            .add_enabled(created, egui::Button::new("Discard").min_size(size))
            .on_hover_text(format!(
                "Remove the template this project creates. Nothing in the database \
                 changes. A template already in the database cannot be removed: \
                 .reload {} never drops an entry it has read.",
                S::TEMPLATE
            ))
            .on_disabled_hover_text(format!("Only a {noun} this project created can be discarded."))
            .clicked()
        {
            if let Some(shown) = shown.as_ref() {
                subject.tool.discard_template(subject.session, shown.entry, now);
                subject.session.status = format!("{} {} discarded", S::TEMPLATE, shown.entry);
            }
        }
    });
}

/// The templates this project creates, listed above the search because no
/// query returns them. Clicking one chooses it, as a search match does.
fn created_templates<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>) {
    let templates = subject.tool.templates.clone();
    if templates.is_empty() {
        return;
    }
    ui.add_space(4.0);
    theme::note(ui, format!("Created by this project ({})", templates.len()));
    egui::ScrollArea::vertical()
        .id_salt((S::TEMPLATE, "created-templates"))
        .max_height(CREATED_LIST_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for known in &templates {
                let on = subject.tool.new_spawn.chosen.as_ref().is_some_and(|had| had.entry() == known.entry());
                if template_row(ui, subject, known, on) {
                    subject.tool.new_spawn.chosen = Some(known.clone());
                }
            }
        });
}

/// The height the created list may take before it scrolls.
const CREATED_LIST_HEIGHT: f32 = 160.0;

/// One row of the picker: a picture of the model, the name, the subject's
/// line and the entry. Answers whether it was clicked.
///
/// The row is [`theme::list_row`], as it is for the spell and item lists, so
/// the lists share one set of text sizes. The picture is a rendered model
/// rather than a decoded icon, requested on every frame the row is drawn, so
/// only the rows on screen pay for one. See [`crate::preview::portraits`].
fn template_row<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>, known: &S::Known, on: bool) -> bool {
    let sub = S::row_line(subject, known);
    let shape = theme::list_row(
        ui,
        theme::ListRow {
            title: known.name(),
            sub: &sub,
            trailing: &known.entry().to_string(),
            tint: theme::INK,
            picture: true,
        },
        on,
    );
    // A display id the tables do not resolve: the row still lists the
    // template, because the server loads a spawn of it.
    if !S::picture(ui, subject, known.display_id(), shape.picture) {
        ui.painter().rect_filled(shape.picture, 3.0, theme::LINE);
    }
    shape.response.clicked()
}

/// The selected spawn's Spawn and Location sections, which both subjects
/// draw the same way.
///
/// Spawn names the row and says what this project does to it. There are
/// three states, and each means something different for what Apply will do,
/// so each is named rather than shown as a colour alone:
///
/// ```text
/// new       there is no row for it yet; Apply creates one
/// removed   there is, and Apply takes it away with the rows keyed by it
/// ordinary  Apply writes whatever columns have been changed
/// ```
///
/// Location is the three position columns and the facing in degrees, written
/// through the same calls the tool's drag and keys use, so a value dragged
/// here is one undo entry as a drag in the world is.
pub(super) fn spawn_sections<S: SpawnView>(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_, S>,
    spawn: &S::Spawn,
    on_server: &crate::server::sync::Standing,
) {
    let now = subject.now;
    let guid = spawn.guid();
    let key = S::spawn_key(guid);
    // Whether this project has written this row to the database. Without it
    // the state said "there is no row in the database" after Apply had been
    // pressed.
    let written = on_server.covers(S::SPAWN, &key);
    let edited = subject.session.rows.edits.touches(S::SPAWN, &key);

    theme::heading(ui, "Spawn");
    property(ui, "GUID", &format!("The {} row's key.", S::SPAWN), |ui| {
        value(ui, guid.to_string());
    });
    let (state, colour, about): (&str, egui::Color32, &str) = match (spawn.is_new(), spawn.is_removed(), written) {
        (_, true, false) => ("Marked for removal", theme::BAD, S::TEXT.removal),
        (_, true, true) => ("Removed and applied", theme::BAD, S::TEXT.removed),
        (true, false, false) => ("New, not applied", theme::WARN, "This project creates the spawn. There is no row in the database yet."),
        (true, false, true) => (
            "New, applied",
            theme::WARN,
            "Created by this project and applied. The database holds the row; restart the server to see the spawn in the world.",
        ),
        (false, false, _) if edited => ("Edited", theme::WARN, "This project changes columns of this row."),
        (false, false, _) => ("Unchanged", theme::INK, "This project does not change this row."),
    };
    property(ui, "State", about, |ui| {
        value_in(ui, state, colour);
    });
    let copies = subject.tool.spawns.iter().filter(|other| other.entry() == spawn.entry()).count();
    property(ui, "Same Entry", &format!("Spawns of this {} on the open map.", S::WORDS.noun), |ui| {
        value(ui, format!("{copies} on this map"));
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("Go To").on_hover_text("Move the camera to this spawn.").clicked() {
            subject.fly.write(crate::camera::FlyTo::at(spawn.at()));
        }
        // Duplicate is on the selection rather than in the picker because it
        // copies a row: every column of this spawn, including the ones this
        // project has changed. The picker chooses a template. The placement
        // tools' Duplicate arms their picker instead, because a doodad is
        // only its model.
        let has_row = subject.tool.spawn_row.as_ref().is_some_and(|held| Some(held.guid) == subject.tool.selected);
        if ui
            .add_enabled(has_row, egui::Button::new("Duplicate"))
            .on_hover_text(format!(
                "Create a spawn of this {} two yards north with every column of this one, \
                 including the columns this project has changed. Ctrl + D.",
                S::WORDS.noun
            ))
            .on_disabled_hover_text("Still reading this spawn's row.")
            .clicked()
        {
            subject.session.status = match subject.tool.duplicate(subject.session, now) {
                Some(guid) => format!("Duplicated as GUID {guid}"),
                None => "Nothing to duplicate".into(),
            };
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            remove_button(ui, subject, spawn, written);
        });
    });

    theme::heading(ui, "Location");
    let standing = crate::tools::spawns::spawn::Standing::of::<S>(spawn);
    let mut at = [standing.at.x, standing.at.y, standing.at.z];
    if theme::xyz(ui, &mut at) {
        standing.move_to(subject.session, bevy::math::Vec3::from(at), now);
    }
    let mut degrees = standing.orientation.to_degrees();
    property(
        ui,
        "Facing",
        "The orientation column in degrees, anticlockwise from north. Written back in radians.",
        |ui| {
            if ui.add(egui::DragValue::new(&mut degrees).speed(1.0).fixed_decimals(1).suffix("\u{b0}")).changed() {
                standing.face(subject.session, degrees.to_radians(), now);
            }
        },
    );
    if let Some(ground) = crate::tools::placements::doodads::ground_height(subject.session, at[0], at[1]) {
        let above = at[2] - ground;
        property(ui, "Above Ground", "Z minus the height of the loaded ground directly below.", |ui| {
            let colour = match above < -0.5 {
                true => theme::WARN,
                false => theme::INK,
            };
            value_in(ui, format!("{above:.1} yd"), colour);
        });
    }
    theme::note(
        ui,
        "To move the spawn, drag it in the viewport, use the handles on the toolbar, or press \
         the arrow keys.",
    );
}

/// The button that removes the selected spawn, discards one this project
/// created, or cancels a removal.
fn remove_button<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>, spawn: &S::Spawn, written: bool) {
    let now = subject.now;
    let guid = spawn.guid();
    if spawn.is_removed() {
        if ui
            .button("Cancel Removal")
            .on_hover_text("Cancel the removal. Column edits this project made to the spawn remain.")
            .clicked()
        {
            SpawnTool::<S>::keep(subject.session, guid, now);
            subject.session.status = format!("{} {guid} removal cancelled", S::WORDS.spawn);
        }
        return;
    }
    // A spawn this project created is dropped rather than marked for
    // removal, and the button label says which: a `DELETE` for a row this
    // project never inserted would act on a row it does not own.
    let word = match spawn.is_new() {
        true => "Discard Spawn",
        false => "Remove Spawn",
    };
    let hover = match (spawn.is_new(), written) {
        (true, true) => "Remove the spawn from this project. The row is already in the database \
                         and stays there; Restore on the Server panel removes it."
            .to_string(),
        (true, false) => "Remove the spawn this project creates. Nothing in the database changes.".to_string(),
        (false, _) => format!(
            "Mark the row for deletion. Apply deletes it; then restart the server, because \
             .reload {} never removes a spawn it has already read. Delete.",
            S::SPAWN
        ),
    };
    if ui.button(egui::RichText::new(word).color(theme::BAD)).on_hover_text(hover).clicked() {
        SpawnTool::<S>::remove(subject.session, guid, now);
        subject.session.status = match spawn.is_new() {
            true => format!("{} {guid} discarded", S::WORDS.spawn),
            false => format!("{} {guid} is marked for removal", S::WORDS.spawn),
        };
        if spawn.is_new() {
            subject.tool.select_only(None);
        }
    }
}

/// The template window's toggle and the link to the workspace, on one row
/// under a selected spawn's template summary.
pub(super) fn template_buttons<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>, entry: u32) {
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let open = subject.tool.template_window;
        if ui.add(egui::Button::new(S::TEXT.edit).selected(open)).on_hover_text(S::TEXT.edit_selected).clicked() {
            subject.tool.template_window = !open;
        }
        open_in_templates(ui, subject, entry);
    });
}

/// The link from the tool to its workspace on the top bar, open on this
/// template. The shell switches tools after the frame is drawn.
pub(super) fn open_in_templates<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>, entry: u32) {
    if ui
        .button("Open in Workspace")
        .on_hover_text(format!(
            "Opens this {table} row in the {workspace} workspace on the top bar, which lists \
             every {table} row and edits the open one in the middle of the screen. The row \
             and its edits are the same as in the template window.",
            table = S::TEMPLATE,
            workspace = S::TEXT.workspace,
        ))
        .clicked()
    {
        subject.tool.open_listed(entry);
        subject.asks.push(ShellAsk::Link(S::WORKSPACE));
    }
}

/// The heading and note over a selected spawn's full row, then the row's
/// columns, which the subject draws.
pub(super) fn row_heading<S: SpawnView>(ui: &mut egui::Ui) {
    theme::heading(ui, "Spawn Row");
    theme::note(
        ui,
        format!(
            "Every column of the {} row. Columns this project changes are amber, with a button \
             that reverts each.",
            S::SPAWN
        ),
    );
}

/// The template's own window, where it is edited.
///
/// Drawn on the root context rather than inside the inspector, like the map
/// window, because a window inside a panel is clipped to it, and this is a
/// window so that it can be larger than the panel and placed elsewhere on the
/// screen.
///
/// The title names the template and its entry, as the body does: two
/// templates' windows hold numbers that look alike, and the entry tells them
/// apart.
pub(crate) fn template_window<S: SpawnView>(ctx: &egui::Context, subject: &mut Subject<'_, S>) -> Option<egui::Rect> {
    if !subject.tool.template_window {
        return None;
    }
    // The picker's template in Place, the selected spawn's template in
    // Select; see `SpawnTool::template_subject`.
    let shown = subject.tool.template_subject(&subject.session.rows.edits)?;
    let mut open = true;
    let title = format!("{} — {} {}", shown.label, S::TEMPLATE, shown.entry);
    let response = theme::window(title, S::TEMPLATE)
        .open(&mut open)
        // Wide enough for the page layout's three columns: the name cell, the
        // value cell and a name resolved beside it.
        .default_size([680.0, 640.0])
        // Clear of the top bar and to the left of the inspector, so it covers
        // neither the button that opened it nor the spawn it is about.
        .default_pos([300.0, 80.0])
        .show(ctx, |ui| {
            template_head(ui, subject, &shown);
            ui.add_space(6.0);
            let key = shown.key();
            let Some(row) =
                subject.tool.template.as_ref().filter(|held| held.entry == shown.entry).map(|held| held.row.clone())
            else {
                theme::waiting(ui, "Loading the row\u{2026}");
                return;
            };
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                S::template_form(ui, subject, &key, &row);
            });
        });
    subject.tool.template_window = open;
    // The dialogs are drawn here as well as under the panel, because the
    // window is drawn whether or not the inspector is showing this tool's
    // panel; `Quests::picker_pass` and the display picker's own slot keep
    // each to one draw a frame.
    dialogs(ctx, subject);
    response.map(|shown| shown.response.rect)
}

/// The window's head: what this project says of the row, how many spawns of
/// it the open map has, and the entry box.
fn template_head<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>, shown: &TemplateSubject) {
    match shown.claim {
        Life::Insert => {
            ui.label(
                egui::RichText::new(
                    "New: this row is not in the database yet. Apply writes it; restart the \
                     server before spawns of it appear in the world.",
                )
                .small()
                .color(theme::WARN),
            );
        }
        _ => {
            ui.label(
                egui::RichText::new(format!(
                    "Edits here change every spawn of this entry, on every map — {} on this map",
                    subject.tool.spawns.iter().filter(|other| other.entry() == shown.entry).count()
                ))
                .small()
                .color(theme::WARN),
            );
        }
    }
    ui.horizontal(|ui| {
        entry_label::<S>(ui, shown);
        let id = ui.make_persistent_id((S::TEMPLATE, "template-entry", shown.patch));
        entry_field(ui, subject, shown, id);
    });
}

/// The `entry` label before the entry box: in the warning colour while the
/// project moves the row.
pub(in crate::ui) fn entry_label<S: SpawnView>(ui: &mut egui::Ui, shown: &TemplateSubject) {
    let colour = match shown.entry != shown.read_entry {
        true => theme::WARN,
        false => theme::INK_DIM,
    };
    ui.label(egui::RichText::new("entry").color(colour))
        .on_hover_text(format!("The {} entry. Changing it renumbers the row.", S::TEMPLATE));
}

/// The `entry` box: the one field of the window that is not a column edit.
///
/// It does the same for any row: the project's claim on the row is re-keyed,
/// so the template is one row before and after; see
/// [`SpawnTool::rekey_template`]. For a row the database holds, the apply then
/// moves the row and every column of the world database that names the
/// template by entry.
pub(in crate::ui) fn entry_field<S: SpawnView>(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_, S>,
    shown: &TemplateSubject,
    id: egui::Id,
) {
    let renumber = Renumber {
        entry: shown.entry,
        read_entry: shown.read_entry,
        is_new: shown.is_new(),
        noun: S::WORDS.noun,
        reaches: S::renumber_reaches(),
    };
    let now = subject.now;
    entry_cell(ui, id, &renumber, |wanted| subject.tool.rekey_template(subject.session, shown, wanted, now));
}
