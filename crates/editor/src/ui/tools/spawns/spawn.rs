//! What the creature and game object panels draw the same way, written once
//! over the subject: the panel's frame, the switches, the edits block, the
//! picker with its template buttons and created list, the selected spawn's
//! claim and buttons, the template window with its entry box, and the group
//! line, handles and placement block of a selection. The view of
//! [`crate::tools::spawns::spawn`].
//!
//! [`SpawnView`] is what a subject's panel adds: the pictures, the lines a
//! template is described by, the sections a selected spawn is drawn in, the
//! template's form, and the sentences that name its own tables. The forms
//! themselves differ (a game object's `data` columns are named by its type)
//! and stay in [`super::creatures`] and [`super::gameobjects`].

use bevy_egui::egui;

use crate::session::EditSession;
use crate::subjects::listing::Listed;
use crate::tools::placements::place;
use crate::tools::spawns::spawn::{SpawnSubject, SpawnTool, Spawned, Template, TemplateSubject};
use crate::ui::shell::asks::{ShellAsk, ShellAsks};
use crate::ui::widgets::rowform::{entry_cell, Renumber};
use crate::ui::widgets::theme;
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
    /// subject's line in [`edits_block`] and whether the selected spawn is
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
    /// The gizmo, for the Handles switch drawn under the Select/Place switch.
    /// `None` where the panel's parts are drawn without it, as in the
    /// template window.
    pub(crate) gizmo: Option<&'a mut crate::tools::placements::gizmo::Gizmo>,
    /// What a link on the panel asks of the shell: a row to open in another
    /// workspace, a tool to switch to. See [`crate::ui::shell::asks`].
    pub(crate) asks: &'a mut ShellAsks,
    /// Where the Go to button sends its request. See
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
    /// The subject in a heading: `Creature`, `Game object`.
    pub(crate) heading: &'static str,
    /// The subject's rows in a sentence's first word: `Creatures`.
    pub(crate) title: &'static str,
    /// Its workspace on the top bar: `Creatures`, `Game Objects`.
    pub(crate) workspace: &'static str,
    /// The panel's note while nothing is selected.
    pub(crate) click: &'static str,
    /// The Models switch's hover.
    pub(crate) models: &'static str,
    /// The model budget's hover, and its largest value.
    pub(crate) budget: &'static str,
    pub(crate) budget_max: usize,
    /// What Apply's removal of a spawn takes with it, before and after the
    /// apply.
    pub(crate) removal: &'static str,
    pub(crate) removed: &'static str,
    /// The + New button's hover, in the picker and in the workspace.
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
    /// panel draws the Handles switch, which only the inspector does.
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
    /// What the switches row holds beside Markers.
    fn beside_markers(_ui: &mut egui::Ui, _subject: &mut Subject<'_, Self>) {}
    /// The selected spawn's sections under the group line: its name, what it
    /// is, its claim, the buttons that open its windows, and its form.
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
        S::subject(session, editing, super::Server { settings: server, panel: server_panel, standings }, assets, now, true),
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
    controls(ui, subject);
    ui.add_space(8.0);

    if subject.tool.trouble.is_some() {
        nothing_to_show(ui, subject);
        return;
    }

    // What this project changes and how much of it is in the database are
    // read from the held standing, which is recomputed at most every half
    // second, and passed to both the block and the selected spawn's sentence.
    // Computing them is a walk of the store and a file read.
    //
    // Drawn above the mode switch because it is about the project rather than
    // about either mode. See [`edits_block`].
    let on_server = subject
        .standings
        .of(crate::server::sync::Half::Row(S::STACK), subject.session, subject.assets, subject.now)
        .clone();
    edits_block(ui, subject, &on_server);

    // Select or Place, the placement tools' own switch. See
    // `crate::tools::placements::place::Mode` for why this is a mode rather
    // than a second rail entry.
    let mut mode = subject.tool.mode;
    theme::segmented(ui, &mut mode, &place::MODES, |a, b| a == b);
    if mode != subject.tool.mode {
        subject.tool.mode = mode;
        // Switching away puts down whatever was on the cursor. The tool's
        // `ghost` also does this; doing it here as well makes the panel
        // correct on the frame of the switch rather than on the frame after.
        if mode == place::Mode::Select {
            subject.tool.new_spawn.chosen = None;
        }
        if mode == place::Mode::Place {
            S::on_place(subject);
        }
    }
    ui.add_space(6.0);

    if subject.tool.mode == place::Mode::Place {
        picker(ui, subject);
        return;
    }

    if subject.tool.spawn_count() == 0 {
        nothing_to_show(ui, subject);
        return;
    }
    let Some(spawn) = subject.tool.chosen_edited(Some(&subject.session.rows.edits)) else {
        theme::note(ui, S::TEXT.click);
        return;
    };

    if let Some(gizmo) = subject.gizmo.as_deref_mut() {
        handles(ui, gizmo);
    }
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
                label: tool.edited(guid, edits).map(|spawn| spawn.label()).unwrap_or_else(|| "not on this map".to_string()),
                detail: format!("guid {guid}"),
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

/// The least height the scrolled spawn form takes, in points.
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
    /// Make this member the primary, from the list of members.
    Primary(u64),
    /// Take this member out of the group, from the list of members.
    Drop(u64),
}

/// The line over a spawn's form when several spawns are selected: how many,
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
    ui.add_space(4.0);
    ui.label(egui::RichText::new(format!("{count} {noun} selected")).strong().color(theme::INK));
    theme::note(
        ui,
        "The form shows the highlighted spawn. Dragging, the handles, the keys and \
         delete act on every selected spawn.",
    );
    ui.horizontal(|ui| {
        if ui
            .small_button("Select primary only")
            .on_hover_text("Keep the highlighted spawn selected and deselect the rest.")
            .clicked()
        {
            asked = Some(GroupAsk::Only);
        }
        if ui.small_button("Clear").clicked() {
            asked = Some(GroupAsk::Clear);
        }
    });
    theme::note(ui, "shift + click adds or removes one · drag on empty ground selects a rectangle");
    match crate::ui::widgets::members::list(ui, noun, count, member) {
        Some(crate::ui::widgets::members::Pick::Primary(guid)) => asked = Some(GroupAsk::Primary(guid)),
        Some(crate::ui::widgets::members::Pick::Drop(guid)) => asked = Some(GroupAsk::Drop(guid)),
        None => {}
    }
    ui.add_space(4.0);
    asked
}

/// What the tool is showing, and the switches that decide how much of it.
fn controls<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>) {
    let spawns = subject.tool.spawns.len();
    let mine = subject.tool.created.len();
    let near = subject.tool.near.len();
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(match (subject.tool.reading(), mine) {
                (true, _) => "reading the map's spawns…".to_string(),
                (false, 0) => format!("{spawns} spawn(s) on this map, {near} near"),
                // The project's own are not in the read's count and are drawn
                // beside the ones that are, so they are counted here. Kept
                // short: the Reload button shares the line, and egui clips
                // text that does not fit.
                (false, mine) => format!("{spawns} on this map, {mine} new, {near} near"),
            })
            .small()
            .color(theme::INK_DIM),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Reload").on_hover_text("Read the map's spawns from the database again.").clicked() {
                subject.tool.forget();
                S::reloaded(subject);
            }
        });
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut subject.tool.show_models, "Models").on_hover_text(S::TEXT.models);
        ui.add(
            egui::DragValue::new(&mut subject.tool.model_budget)
                .speed(5.0)
                .range(0..=S::TEXT.budget_max)
                .prefix("at most "),
        )
        .on_hover_text(S::TEXT.budget);
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut subject.tool.show_markers, "Markers").on_hover_text(
            "Mark every spawn near the camera. Off, only the selected, hovered and \
             grouped spawns are marked, which saves frame time where many spawns are \
             near. Spawns can still be clicked.",
        );
        let other = S::KIND.other();
        let label = match other {
            crate::tools::spawns::spawn::SpawnKind::Creature => "Creatures",
            crate::tools::spawns::spawn::SpawnKind::GameObject => "Game Objects",
        };
        ui.checkbox(&mut subject.others.shown, label).on_hover_text(format!(
            "Mark the {} spawns near the camera too, faded. A click on one selects it \
             and switches to its tool.",
            other.word()
        ));
        S::beside_markers(ui, subject);
    });
}

/// Why the list is empty: no connection, still reading, or no spawn on the
/// map.
fn nothing_to_show<S: SpawnView>(ui: &mut egui::Ui, subject: &Subject<'_, S>) {
    match &subject.tool.trouble {
        Some(trouble) => {
            ui.label(egui::RichText::new(trouble).small().color(theme::WARN));
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
            theme::waiting(ui, "reading\u{2026}");
        }
        None => {
            theme::note(ui, format!("No {} spawns on this map.", S::WORDS.noun));
        }
    }
}

/// What this project changes about the subject's rows, and the button that
/// opens the panel that applies it.
///
/// Drawn whether or not a spawn is selected, because what the project has
/// changed is a question about the project rather than about the selection.
///
/// The operations are not on this panel. Apply, Put back and Discard are on
/// the top bar's Server… popover, beside the other subjects' and in the same
/// words; [`crate::ui::shell::sync`]'s module comment gives the reason for one
/// panel. This block shows only the state, so that a person looking at a
/// spawn can tell whether what they see is in the database without opening
/// another panel.
///
/// `on_server` is the subject's held [`crate::server::sync::Standing`]. Its
/// `outstanding` counts this project's rows and paths not yet in the
/// database; without that count the line below read the same whether nothing
/// had been applied or everything had.
fn edits_block<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>, on_server: &crate::server::sync::Standing) {
    let undoable = on_server.applied;
    let empty = on_server.changed == 0;
    if empty && undoable == 0 {
        return;
    }
    let outstanding = on_server.outstanding;

    ui.separator();
    theme::heading(ui, "This project");
    if !empty {
        // Amber while something is not yet applied, dim once all of it is in
        // the database. A warning colour that is always on stops being read
        // as a warning.
        let colour = match outstanding == 0 {
            true => theme::INK_DIM,
            false => theme::WARN,
        };
        ui.label(egui::RichText::new(format!("{} — {}", on_server.summary, S::SQL)).small().color(colour));
    }
    for refused in &on_server.refused {
        ui.label(egui::RichText::new(refused).small().color(theme::BAD));
    }

    // The project's state is two independent facts, and the line states both.
    // A revert puts the database back and leaves the project still claiming
    // the rows, so the editor goes on drawing the edited values.
    match (undoable, outstanding, empty) {
        (0, _, false) => theme::note(
            ui,
            "Not applied. The database still holds its previous values; the editor \
             draws this project's values.",
        ),
        // Everything this project claims is in the database. This case has
        // its own sentence so that it can be told from the case above.
        (n, 0, false) => {
            let line = match on_server.current {
                true => format!(
                    "Applied — all {n} row(s) are in the database. Restart the server to \
                     see them; Restore reverts them."
                ),
                // The signature is per process, so a project also reads this
                // way after a relaunch. See `EditSession::applied`.
                false => format!(
                    "Applied — all {n} row(s) are in the database, possibly with older \
                     values: an edit was made since, or this is a later session. Apply \
                     again to make sure."
                ),
            };
            ui.label(egui::RichText::new(line).small().color(theme::INK_DIM));
        }
        (n, out, false) => {
            ui.label(
                egui::RichText::new(format!("{out} not applied yet; {n} row(s) are in the database and undoable"))
                    .small()
                    .color(theme::WARN),
            );
        }
        (n, _, true) => {
            ui.label(
                egui::RichText::new(format!(
                    "{n} row(s) applied and undoable; the project no longer changes any of them"
                ))
                .small()
                .color(theme::WARN),
            );
        }
    }
    if ui
        .button("Server\u{2026}")
        .on_hover_text(format!(
            "Open the Server panel, which holds Apply, Restore and Discard for every \
             subject, including spells and items. Restore runs {}.",
            S::REVERT
        ))
        .clicked()
    {
        subject.server_panel.show();
    }
    theme::note(
        ui,
        format!(
            "Apply writes the rows only. The server reads {} once, at startup, so restart \
             it to see them.",
            S::WORDS.nouns
        ),
    );
    ui.separator();
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
    theme::heading(ui, S::TEXT.heading);
    template_actions(ui, subject);
    ui.add_space(4.0);
    match subject.tool.new_spawn.chosen.clone() {
        Some(chosen) => {
            ui.label(egui::RichText::new(chosen.label()).color(theme::INK));
            S::chosen_detail(ui, subject, &chosen);
            if chosen.is_new() {
                ui.label(
                    egui::RichText::new("New: this project creates it. There is no row in the database yet.")
                        .small()
                        .color(theme::ACCENT),
                );
            }
            theme::note(
                ui,
                format!(
                    "Click the ground to place it. Escape clears the chosen {}. Every column of \
                     the row can be edited afterwards in Select.",
                    S::WORDS.noun
                ),
            );
            theme::note(ui, ", and . turn it \u{b7} shift turns three times as far \u{b7} alt + mouse turns it");
            theme::note(ui, "ctrl with alt snaps the turn to 15\u{b0}");
            // The window's toggle, here as well as under a selected spawn, so
            // a template can be edited before any spawn of it exists.
            ui.horizontal_wrapped(|ui| {
                let open = subject.tool.template_window;
                if ui.selectable_label(open, S::TEXT.edit).on_hover_text(S::TEXT.edit_chosen).clicked() {
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
        None => theme::note(ui, S::TEXT.none_chosen),
    }

    created_templates(ui, subject);

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("find").color(theme::INK_DIM));
        ui.add(
            egui::TextEdit::singleline(&mut subject.tool.new_spawn.search)
                .hint_text("name or entry")
                .desired_width(f32::INFINITY),
        );
    });
    S::search_filters(ui, subject);
    if let Some(trouble) = subject.tool.new_spawn.trouble.clone() {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
    }
    if subject.tool.new_spawn.searching() {
        theme::waiting(ui, "searching\u{2026}");
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
        ui.label(egui::RichText::new(format!("no matching {}", S::WORDS.noun)).small().color(theme::INK_DIM));
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

/// The three buttons over the picker: a new template, a copy of the one the
/// window is about, and giving up a created one. The workspace's list has the
/// same row.
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
        if ui.add(egui::Button::new("+ New").min_size(size)).on_hover_text(S::TEXT.new_template).clicked() {
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
                    Some(entry) => format!("copied as {} {entry}", S::TEMPLATE),
                    None => format!("still reading this {noun}'s row"),
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
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(format!("{} {}(s) created by this project", templates.len(), S::WORDS.noun))
            .small()
            .color(theme::INK_DIM),
    );
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

/// What this project claims about the selected spawn, and the button that
/// changes it.
///
/// There are three states, and each means something different for what Apply
/// will do, so each is drawn as a sentence rather than a colour:
///
/// ```text
/// new       there is no row for it yet; Apply creates one
/// removed   there is, and Apply takes it away with the rows keyed by it
/// ordinary  Apply writes whatever columns have been changed
/// ```
pub(super) fn claim<S: SpawnView>(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_, S>,
    spawn: &S::Spawn,
    on_server: &crate::server::sync::Standing,
) {
    let now = subject.now;
    let guid = spawn.guid();
    // Whether this project has written this row to the database. Without it
    // the sentence below said "there is no row in the database" after Apply
    // had been pressed.
    let written = on_server.covers(S::SPAWN, &S::spawn_key(guid));
    if spawn.is_new() {
        ui.label(
            egui::RichText::new(match written {
                false => "New — this project creates it. There is no row in the database yet.",
                true => {
                    "New — created by this project and applied. The database holds the row; \
                     restart the server to see the spawn in the world."
                }
            })
            .small()
            .color(theme::ACCENT),
        );
    }
    if spawn.is_removed() {
        ui.label(
            egui::RichText::new(match written {
                false => S::TEXT.removal,
                true => S::TEXT.removed,
            })
            .small()
            .color(theme::BAD),
        );
    }
    ui.horizontal(|ui| match spawn.is_removed() {
        true => {
            if ui
                .button("Cancel removal")
                .on_hover_text("Cancel the removal. Column edits this project made to the spawn remain.")
                .clicked()
            {
                SpawnTool::<S>::keep(subject.session, guid, now);
                subject.session.status = format!("{} {guid} removal cancelled", S::WORDS.spawn);
            }
        }
        false => {
            // A spawn this project created is dropped rather than marked for
            // removal, and the button label says which: a `DELETE` for a row
            // this project never inserted would act on a row it does not own.
            let word = match spawn.is_new() {
                true => "Discard this spawn",
                false => "Remove this spawn",
            };
            let hover = match (spawn.is_new(), written) {
                (true, true) => "Remove the spawn from this project. The row is already in the database \
                                 and stays there; Restore on the Server panel removes it."
                    .to_string(),
                (true, false) => "Remove the spawn this project creates. Nothing in the database changes.".to_string(),
                (false, _) => format!(
                    "Mark the row for deletion. Apply deletes it; then restart the server, \
                     because .reload {} never removes a spawn it has already read.",
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
    });
}

/// The selected spawn's buttons: Go to, Duplicate, and how many spawns of
/// its template the map has.
pub(super) fn head_buttons<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>, spawn: &S::Spawn) {
    ui.horizontal(|ui| {
        if ui.small_button("Go to").on_hover_text("Move the camera to this spawn.").clicked() {
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
                 including the columns this project has changed.",
                S::WORDS.noun
            ))
            .on_disabled_hover_text("Still reading this spawn's row.")
            .clicked()
        {
            let now = subject.now;
            subject.session.status = match subject.tool.duplicate(subject.session, now) {
                Some(guid) => format!("duplicated as guid {guid}"),
                None => "nothing to duplicate".into(),
            };
        }
        let others = subject.tool.spawns.iter().filter(|other| other.entry() == spawn.entry()).count();
        ui.label(
            egui::RichText::new(format!("{others} spawn(s) of this {} on this map", S::WORDS.noun))
                .small()
                .color(theme::INK_FAINT),
        );
    });
}

/// The Handles switch for a selected spawn. A spawn has no tilt, so Turn
/// offers the z ring alone; see `crate::tools::spawns::spawn`.
fn handles(ui: &mut egui::Ui, gizmo: &mut crate::tools::placements::gizmo::Gizmo) {
    use crate::tools::placements::gizmo::{Handles, HANDLES};
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

/// The facing in degrees and the keys that move and turn a spawn. The
/// position and the facing in radians are also columns of the form below this
/// block; the facing is repeated here in degrees because the doodad and WMO
/// panels state their turns in degrees.
pub(super) fn placement(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    standing: &crate::tools::spawns::spawn::Standing,
    now: f64,
) {
    theme::heading(ui, "Placement");
    let mut degrees = standing.orientation.to_degrees();
    theme::row(ui, "facing", |ui| {
        if ui
            .add(egui::DragValue::new(&mut degrees).speed(1.0).fixed_decimals(1).suffix("\u{b0}"))
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

/// The template window's toggle under a selected spawn.
pub(super) fn edit_toggle<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>) {
    let open = subject.tool.template_window;
    if ui.selectable_label(open, S::TEXT.edit).on_hover_text(S::TEXT.edit_selected).clicked() {
        subject.tool.template_window = !open;
    }
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
                theme::waiting(ui, "reading the row\u{2026}");
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
                .color(theme::ACCENT),
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
