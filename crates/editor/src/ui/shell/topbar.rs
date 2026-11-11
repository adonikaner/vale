//! The bar across the top of the window: the Project and Map menus and the
//! workspace control on the left, and on the right the controls that take
//! the work somewhere else: the shared session, Save, the Server panel and
//! Playtest.
//!
//! ## What the bar holds
//!
//! Every control on this bar is about the session rather than about the thing
//! being edited: which project, which map, whether the work is saved, which
//! workspace is open, and whether the game is running. A control that changes
//! the world belongs in the inspector on the right, beside the tool that owns
//! it.
//!
//! ```text
//! left    Project: <name>   Projects…      the project dialog
//!                           Settings…      this machine's settings
//!                           Publish…       the publish popover
//!         Map: <name> (id)  New Map…, Map Properties…, Open Map, Edit Map…,
//!                           Go To…, Bookmarks
//!         the workspace control
//! right   the session button        hosting or joining a shared session
//!         Changes                   edits not in a changelist, Submit, the
//!                                   conflicts and the history
//!         Save                      what is unsaved, and writing it
//!         Server…                   what the project has done to the server
//!         Playtest, and its popover who it logs in as, on which server, and
//!                                   whether the query cache is kept
//! ```
//!
//! Each menu's button states what a person reads the bar for without opening
//! it: which project is open, and which map with its `Map.dbc` id. The id is
//! the number every other authority keys on (`WorldMapArea`, `Light.dbc`,
//! vmangos' tables, a `.tele`), and the directory name is usable in none of
//! them.
//!
//! A menu entry is a command. A form is a popover: an egui menu closes when
//! anything inside it is clicked, which suits a list of commands and does not
//! suit text fields, so `Settings…`, `Publish…`, `Go To…` and `Add Bookmark…`
//! open windows placed under the menu. See [`super::popover`].
//!
//! ## Where a setting is
//!
//! A setting of this machine (the world database, the map tools, the drawing
//! tablet) is in the Settings window. A setting of the project is beside the
//! action it changes: Apply on save on the Server panel, the publish options
//! in the publish popover, and Disable caching in the Playtest popover. No
//! setting is in two places.
//!
//! ## The right-hand controls
//!
//! The controls on the right are the places an edit goes after it is made: to
//! the other editors of a shared session, into a changelist, into the project
//! folder, into the world database, and into a running game. They are drawn
//! in that order, with Playtest at the far right because it is the only one
//! that takes the editor out of editing. Save's label says what is unsaved.
//! The session button's label states the session; the Changes button's
//! counts the edits not in a changelist. The button beside Playtest
//! states who the playtest logs in as, ends in "⏷" because it opens the
//! Playtest popover, and gains " · cached" while the query cache is kept.
//! During a playtest, Playtest and its popover are replaced by the
//! playtest's state, Hide Panels and End Playtest; the others stay.
//!
//! After the Map menu comes the workspace control ([`super::rail::workspaces`]):
//! World, then each workspace that replaces the viewport. World returns to the
//! last rail tool. [`Subjects`] carries the tool and the rail's memory into
//! [`draw`].
//!
//! ## Where the workspace control wraps
//!
//! The workspace control is given the width between the Map menu and the
//! right-hand controls, and wraps to a second row inside it. The right-hand
//! controls are laid out right to left after it, so their width is measured on
//! each frame and kept in egui's temporary data for the next one; the first
//! frame assumes `RIGHT_ROOM`. Without the limit, a workspace that did not fit
//! was drawn under the right-hand controls and could not be clicked. Text does
//! not wrap inside the control, so a workspace that does not fit moves to the
//! next row whole rather than having its name narrowed.

use bevy_egui::egui;

use crate::ui::widgets::theme;
use crate::camera::EditorCamera;
use crate::playtest::{Login, Playtest};
use crate::session::EditSession;
use crate::ui::shell::popover::Popovers;
use vale_client::assets::GameAssets;
use vale_client::glue::autologin::AutoLogin;
use bevy::prelude::*;

/// The text typed into the "go to" boxes.
///
/// Held as text rather than as numbers: a field bound to an `f32` re-formats
/// its contents on every keystroke, so it cannot hold "-94" on the way to
/// "-9450". The parse happens when the jump is requested.
#[derive(Resource, Default)]
pub(crate) struct GoTo {
    pub(super) x: String,
    pub(super) y: String,
    pub(super) tile_x: String,
    pub(super) tile_y: String,
    /// The name a new bookmark is given — see [`crate::prefs::bookmarks`].
    pub(super) bookmark: String,
}

/// The playtest's resources, as one parameter.
///
/// They are bundled because the right-hand end of this bar is everything that
/// starts and stops a playtest, and because bundling keeps [`draw`] under the
/// argument count. `ui::Playing`, one layer out, exists for the same reason.
pub(crate) struct Session<'a> {
    pub(crate) state: &'a mut Playtest,
    pub(crate) client: &'a mut vale_client::world::session::Session,
    pub(crate) login: &'a Login,
    pub(crate) auto: &'a mut AutoLogin,
    pub(crate) open: &'a mut crate::playtest::ShellOpen,
    /// The queue a save's database writes go on. See
    /// [`crate::server::queue`].
    pub(crate) queue: &'a mut crate::server::queue::ServerQueue,
    /// Where this machine's server is. The Save button's apply needs it and
    /// the bar's Server… button opens it.
    pub(crate) server: &'a mut crate::server::settings::ServerSettings,
    /// Whether the session Playtest starts keeps the query answers the server
    /// gives it. See [`vale_client::world::session::QueryCaches`], and
    /// [`crate::server::settings::ServerSettings::disable_caching`], the
    /// switch it is written from.
    pub(crate) caches: &'a mut vale_client::world::session::QueryCaches,
    /// Which step of a tile regeneration is running. Publish starts one; see
    /// [`crate::server::datadir::Step`].
    pub(crate) step: &'a crate::server::datadir::Step,
}

pub(crate) fn draw(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    camera: &mut EditorCamera,
    popovers: &mut Popovers,
    assets: &GameAssets,
    playing: &mut Session,
    map_open: &mut bool,
    // The New map form, which New Map… opens. See [`crate::map::maps`].
    new_map: &mut Option<crate::map::maps::Form>,
    subjects: &mut Subjects<'_>,
    // The kept views the Map menu lists. See [`crate::prefs::bookmarks`].
    bookmarks: &crate::prefs::bookmarks::Bookmarks,
    // The shared session: the Session… button's label, and whether this
    // editor is joined to another editor's session, which disables Save and
    // Publish, and which server a playtest logs in to.
    collab: &crate::collab::Collab,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Vale IDE")
                .strong()
                .color(theme::INK)
                .size(13.0),
        );
        ui.label(
            egui::RichText::new("editor")
                .color(theme::INK_FAINT)
                .size(13.0),
        );
        separator(ui);

        // The project comes first: everything else is written into it, and it
        // is the one thing on this bar a person can lose work by being wrong
        // about.
        let in_world = playing.state.playing();
        project(ui, session, popovers, in_world, playing.queue.busy(), collab.server_gate().release_refused());
        separator(ui);
        map(ui, session, camera, popovers, bookmarks, map_open, new_map, in_world);
        separator(ui);
        // The workspace control: World, then each workspace that replaces the
        // viewport. See [`super::rail`], which holds the world tools' rail too.
        let server = playing.server.resolve().is_some();
        // The workspace control wraps inside the room the playtest controls
        // leave, measured on the frame before, so a workspace that does not
        // fit goes to a second row rather than under the playtest controls.
        let right_id = ui.id().with("topbar-right-width");
        let right = ui.data(|data| data.get_temp::<f32>(right_id)).unwrap_or(RIGHT_ROOM);
        let room = (ui.available_width() - right - 12.0).max(160.0);
        ui.allocate_ui_with_layout(
            egui::vec2(room, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
            |ui| {
                ui.set_max_width(room);
                // A button that does not fit goes to the next row whole;
                // with wrapping text, egui narrows its name to fit instead.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                super::rail::workspaces(ui, subjects.tool, subjects.rail, in_world, server, &subjects.popped, true);
            },
        );

        // The right-hand controls, in the order an edit reaches them from the
        // left: the session, Save, the Server panel, the playtest. The layout
        // is right to left, so the first control added is the rightmost.
        //
        // While the panels are open over a playtest, the far right holds the
        // playtest's own controls, in the place of the button that started
        // it. Leaving the `playtest` window up over the shell instead would
        // state the same thing in two places on one screen.
        // Aligned to the top: when the workspace control wraps to a second
        // row, these stay on the first row with the menus.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let start = ui.cursor().max.x;
            match in_world {
                true => running(ui, playing, session),
                false => not_running(ui, session, assets, popovers, playing, collab),
            }
            separator(ui);
            let at_server = ui.button("Server\u{2026}").on_hover_text(
                "What this project has done to the world database: Apply on save, Apply and \
                 Restore for each subject, the server's tiles and the migration.",
            );
            popovers.server.track(&at_server);
            if at_server.clicked() {
                popovers.server.toggle(&at_server);
            }
            save(ui, session, assets, playing, in_world, collab.is_joined().then(|| collab.to_submit()));
            let at_changes = super::changes::button(ui, collab);
            popovers.changes.track(&at_changes);
            if at_changes.clicked() {
                popovers.changes.toggle(&at_changes);
            }
            let at_session = super::session::button(ui, collab);
            popovers.session.track(&at_session);
            if at_session.clicked() {
                popovers.session.toggle(&at_session);
            }
            let used = start - ui.min_rect().min.x;
            ui.data_mut(|data| data.insert_temp(right_id, used));
        });
    });
}

/// The width the right-hand controls are given before they have been
/// measured, in points: the session button, Changes, Save, Server…, Playtest
/// and its login label.
const RIGHT_ROOM: f32 = 600.0;

/// What the workspace control changes: the tool in use, and the rail's memory
/// of the last world tool. Passed into [`draw`] as one parameter.
pub(crate) struct Subjects<'a> {
    pub(crate) tool: &'a mut crate::tools::Tool,
    pub(crate) rail: &'a mut super::rail::Rail,
    /// The workspaces in pop-out windows. See `crate::popout`.
    pub(crate) popped: Vec<crate::tools::Tool>,
}

/// The right-hand end of the bar while a playtest is running: the playtest's
/// state, and the two ways out of it.
///
/// The layout is right to left, so the first control added is the rightmost.
fn running(ui: &mut egui::Ui, playing: &mut Session, session: &mut EditSession) {
    if ui
        .button("End Playtest")
        .on_hover_text("Ctrl+P. Disconnects from the server and returns to the editor.")
        .clicked()
    {
        crate::playtest::stop(playing.state, playing.client, playing.auto, session);
        return;
    }
    if ui
        .button("Hide Panels")
        .on_hover_text("Ctrl+E. Hides the editor panels. The playtest keeps running.")
        .clicked()
    {
        playing.open.0 = false;
    }
    ui.label(
        egui::RichText::new(match *playing.state {
            Playtest::Playing => "in the world",
            _ => "logging in",
        })
        .size(theme::SMALL)
        .color(theme::INK_FAINT),
    );
    ui.label(
        egui::RichText::new("PLAYTEST")
            .color(theme::GOOD)
            .size(theme::SMALL),
    );
}

/// The right-hand end of the bar while no playtest is running: the button that
/// starts it, and the button that opens the Playtest popover.
///
/// The popover's button is to the right of Playtest and states who the
/// playtest logs in as. In a shared session its label also names the host's
/// server when a joined editor playtests there, which the Session popover
/// sets. See `crate::collab::PlaytestServer`.
fn not_running(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    assets: &GameAssets,
    popovers: &mut Popovers,
    playing: &mut Session,
    collab: &crate::collab::Collab,
) {
    // The button's label states the answer rather than a word like "Login":
    // `as Corwin`, or `login screen` when there is no password to skip it
    // with. The fact a person would open the popover to check is readable
    // without opening it. `cached` is added when the query cache is kept,
    // since that is the setting that makes an edit look as if it did not
    // take.
    let mut label = match playing.login.can_go_straight_in() && playing.login.straight_in {
        true => format!("as {}", playing.login.who()),
        false => "login screen".to_string(),
    };
    if !playing.server.disable_caching {
        label.push_str(" \u{b7} cached");
    }
    if playing.login.realm.is_some() {
        let host = collab.host_name().unwrap_or("host");
        label.push_str(&format!(" \u{b7} {host}'s server"));
    }
    label.push_str(" \u{23F7}");
    let who = ui.button(label).on_hover_text(
        "Playtest settings: the account and character, whether the login screen is skipped, \
         whether the client keeps its query cache, and in a shared session which server it \
         logs in to.",
    );
    popovers.login.track(&who);
    if who.clicked() {
        popovers.login.toggle(&who);
    }
    if ui
        .add(
            egui::Button::new(
                egui::RichText::new("Playtest").color(egui::Color32::from_rgb(0x0C, 0x16, 0x1E)),
            )
            .fill(theme::ACCENT)
            .corner_radius(egui::CornerRadius::same(3)),
        )
        .on_hover_text(
            "Save the project and start a playtest on the open map. Ctrl+P starts it; \
             Ctrl+P again ends it.",
        )
        .clicked()
    {
        crate::playtest::start(
            playing.state,
            session,
            assets,
            playing.login,
            playing.auto,
            playing.queue,
            playing.server,
            playing.caches,
            &collab.server_gate(),
        );
    }
}

/// A hairline between two groups on the bar.
fn separator(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let height = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.top() + 3.0..=rect.bottom() - 3.0,
        egui::Stroke::new(1.0, theme::LINE),
    );
    ui.add_space(4.0);
}

/// The Project menu: the project dialog, the Settings window and the publish
/// popover.
///
/// The menu's button is the open project's name, which is the one thing on
/// this bar a person can lose work by being wrong about.
fn project(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    popovers: &mut Popovers,
    playing: bool,
    // Whether a server write is running. A publish's tile regeneration reads
    // the folder a second publish would write, so Publish is disabled while
    // one runs.
    regenerating: bool,
    // Why this editor may not publish: a joined editor and an offline copy
    // leave it to the host. See `crate::collab::ServerGate::release_refused`.
    release_refused: Option<&'static str>,
) {
    let (mut ask_projects, mut ask_settings, mut ask_publish) = (false, false, false);
    let menu = ui
        .menu_button(format!("Project: {}", session.project.name), |ui| {
            // Disabled while a playtest is running. Switching saves what is
            // unsaved, rebuilds the archive overlay, empties the undo stack
            // and re-reads every tile on screen, and the running client reads
            // that overlay for the ground the character stands on. Ending the
            // playtest first is one keypress and leaves nothing half-swapped.
            if ui
                .add_enabled(!playing, egui::Button::new("Projects…"))
                .on_hover_text("Open, create, clear or delete projects.")
                .on_disabled_hover_text(
                    "Not while a playtest is running: switching projects rebuilds the archive \
                     overlay the client is reading. Press Ctrl+P to end the playtest first.",
                )
                .clicked()
            {
                ask_projects = true;
                ui.close();
            }
            if ui
                .button("Settings\u{2026}")
                .on_hover_text(
                    "This machine's settings: the world database, the vmangos map tools and the \
                     drawing tablet. Kept under Edit\\, not in the project.",
                )
                .clicked()
            {
                ask_settings = true;
                ui.close();
            }
            // Disabled while a playtest is running. Publishing writes a new
            // archive into the folder whose chain this process has open, and
            // the running client reads that chain for the ground the
            // character is standing on. Saving is what reaches a running
            // game; publishing is for finished work.
            if ui
                .add_enabled(!playing && !regenerating && release_refused.is_none(), egui::Button::new("Publish…"))
                .on_hover_text(
                    "Write a patch: one folder under the project's publish\\ containing the \
                     client archive, the server DBCs, an SQL migration of the rows and the \
                     server's maps, vmaps and mmaps, with a README listing where each goes. \
                     Nothing is applied to this machine.",
                )
                .on_disabled_hover_text(match (release_refused, playing) {
                    (Some(refused), _) => refused,
                    (None, true) => "Not while a playtest is running.",
                    (None, false) => "A server write is still running.",
                })
                .clicked()
            {
                ask_publish = true;
                ui.close();
            }
        })
        .response
        .on_hover_text("The open project. Every edit is written into its folder.");
    // The three windows are placed from the menu's button, tracked every
    // frame so `--projects` and `--settings` can open one without a press.
    // See `Popover::show`.
    popovers.project.track(&menu);
    popovers.settings.track(&menu);
    popovers.publish.track(&menu);
    if ask_projects {
        popovers.project.show();
    }
    if ask_settings {
        popovers.settings.show();
    }
    if ask_publish {
        popovers.publish.show();
    }
}

/// The Save button, and the spinner beside it while a server write runs.
///
/// The button's label names what it will save, for example `Save 2 tiles and
/// server rows`, or `Saved` when there is nothing. A bare count such as
/// `Save 1`, which counted tiles with unsaved changes, does not say what it
/// counts. Joined, it names what Submit closes instead, for example `Submit 2
/// tiles`, or `Saved by the host` when the Changes window lists no edit
/// (`crate::collab::ToSubmit`).
///
/// It is drawn in the right-to-left half of the bar, so the spinner is added
/// first and stands to the right of the button.
fn save(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    assets: &GameAssets,
    playing: &mut Session,
    in_world: bool,
    // While this editor is joined to another's shared session, whose host
    // saves: what its Submit closes.
    submit: Option<crate::collab::ToSubmit>,
) {
    let joined = submit.is_some();
    // The spinner's hover text is the step's name. The step's name is a
    // sentence that changes every few seconds, and a bar whose width follows
    // it would move everything beside it; the sentence and the fraction are
    // on the status line's bar instead.
    if playing.queue.busy() {
        let line = playing.step.line();
        let spinner = ui.add(egui::Spinner::new().size(14.0).color(theme::ACCENT));
        match line.is_empty() {
            true => spinner.on_hover_text("A server write is running."),
            false => spinner.on_hover_text(line),
        };
    }

    // Three kinds of unsaved work: tiles, tables and server rows. The same
    // press writes the server rows, so they are counted too. Without them the
    // label read "Saved" and the button was disabled while a creature or
    // waypoint edit was outstanding, which left Ctrl+S as the only way to
    // write it and nothing on screen saying a save was owed. See
    // `ServerRows::unsaved`.
    // Joined, the host saves: Save submits this editor's edits to it, as
    // Submit in the Changes window does, and the label names what the
    // Submit closes. A joined copy is never saved, so its unsaved tiles only
    // grow, and they include every tile another editor's edit changed.
    let (tiles, tables, rows) = match submit {
        Some(submit) if submit.edits == 0 => (0, 0, false),
        Some(submit) => (submit.tiles, submit.tables, submit.rows),
        None => (session.unsaved.len(), session.unsaved_tables.len(), session.rows.unsaved()),
    };
    let mut parts: Vec<String> = Vec::new();
    if tiles > 0 {
        parts.push(plural(tiles, "tile", "tiles"));
    }
    if tables > 0 {
        parts.push(plural(tables, "table", "tables"));
    }
    if rows {
        // Not a count: the two stores hold rows and whole paths, which do not
        // add up to one number. The label says only that a save is owed.
        parts.push("server rows".to_string());
    }
    // Edits that claim nothing yet, such as an import, or before the host's
    // claims arrive, are counted as edits.
    if let Some(submit) = submit.filter(|submit| submit.edits > 0 && parts.is_empty()) {
        parts.push(plural(submit.edits, "edit", "edits"));
    }
    let anything = !parts.is_empty();
    let label = match (joined, anything) {
        (true, false) => "Saved by the host".to_string(),
        (true, true) => format!("Submit {}", parts.join(" and ")),
        (false, false) => "Saved".to_string(),
        (false, true) => format!("Save {}", parts.join(" and ")),
    };
    // Saving during a playtest is also the reload. The write puts the edit in
    // the project folder and `crate::playtest::republish` puts it where the
    // running client reads it, so the next cast, the next entity to come into
    // view and the next model to be hung are the edited ones. What is already
    // standing keeps the handles it has.
    if ui
        .add_enabled(anything, egui::Button::new(label))
        .on_hover_text(match (joined, in_world) {
            (true, _) => "Submit this editor's edits to the host as one changelist, as Submit in the Changes \
                 window does. The host saves them. Ctrl+S."
                .to_string(),
            (false, true) => format!(
                "Write the changed tiles, tables and server rows into {}, then \
                 republish them to the running playtest. Ctrl+S.",
                session.project.root.display()
            ),
            (false, false) => format!(
                "Write the changed tiles, tables and server rows into {}. \
                 Ctrl+S.",
                session.project.root.display()
            ),
        })
        .on_disabled_hover_text(match joined {
            true => "Nothing to submit. In a shared session the host saves; this editor's copy of the host's project receives each edit as it is made.",
            false => "Nothing has changed since the last save.",
        })
        .clicked()
    {
        match joined {
            true => session.save_asked = true,
            false => crate::server::save_everything(session, assets, playing.server, playing.queue, in_world),
        }
    }
}

/// `1 tile`, `3 tiles`.
fn plural(count: usize, one: &str, many: &str) -> String {
    match count {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

/// The text typed into the project popover, kept across the frames it is typed
/// over.
///
/// Beside [`GoTo`], for the same reason: a text field's contents are the
/// panel's state, and the session does not hold them.
#[derive(Resource, Default)]
pub(crate) struct Projects {
    pub(crate) new_name: String,
    /// Whether `--projects` has been acted on.
    pub(crate) shown_once: bool,
    /// Whether `--guides-menu` has been acted on.
    pub(crate) guides_shown_once: bool,
    /// Every project under `Edit\`, with what each holds. Read when the
    /// dialog opens and after it makes one, not every frame, because a summary
    /// walks the folder.
    pub(crate) known: Vec<(String, vale_edit::project::Summary)>,
    pub(crate) listed: bool,
    /// The open project's manifest, built with [`Self::known`] and dropped
    /// with it. See `super::manifest`.
    pub(crate) manifest: Option<super::manifest::Report>,
    /// The project a delete or clear is awaiting confirmation for, and `None`
    /// otherwise.
    ///
    /// Deleting a project is the one action in this editor that destroys work
    /// and cannot be undone, so it is the one action here that asks first.
    /// The dialog's own note gives the reason a switch is not confirmed: a
    /// confirmation nobody reads is how work gets lost, and a switch saves
    /// first. A delete does not save first, so that reason does not apply.
    pub(crate) confirming: Option<Doomed>,
}

/// A project one of the two destroying buttons has been pressed on, and which
/// button it was.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Doomed {
    pub(crate) name: String,
    /// `true` for clear (delete the files and keep the folder), which is what
    /// the open project and the default project get in place of deletion.
    pub(crate) clear: bool,
    /// What the project has applied to the server, one line each, read once
    /// when the button was pressed. See `crate::server::held`. Empty for a
    /// project that has applied nothing.
    pub(crate) held: Vec<String>,
    /// What has been typed into the confirmation's name field.
    pub(crate) typed: String,
    /// How many tile files in the live `DataDir` the project's regenerations
    /// wrote last, read from the `DataDir`'s ledger on the confirmation's
    /// first frame. `None` until then.
    pub(crate) tile_files: Option<usize>,
}

impl Doomed {
    pub(crate) fn new(name: String, clear: bool, held: &crate::server::held::Held) -> Doomed {
        Doomed {
            name,
            clear,
            held: held.lines(),
            typed: String::new(),
            tile_files: None,
        }
    }

    /// Whether the destructive button is enabled: always for a project that
    /// has applied nothing, and otherwise only once the project's name has
    /// been typed. The record of what to put back is in the folder being
    /// deleted or cleared, so proceeding leaves the applied rows in the
    /// database with no record of how to put them back.
    pub(crate) fn may_proceed(&self) -> bool {
        self.held.is_empty() || self.typed.trim() == self.name
    }
}

impl Projects {
    /// Read the folders again.
    pub(crate) fn refresh(&mut self, install: &std::path::Path) {
        self.known = vale_edit::project::projects(install)
            .into_iter()
            .map(|name| {
                let summary = vale_edit::project::summary(install, &name);
                (name, summary)
            })
            .collect();
        self.listed = true;
    }
}

/// The Map menu: which map is open, and the five things done to the map as a
/// whole.
///
/// The button shows the map's name with its `Map.dbc` id. The entries are the
/// new map form, the map list, the tile map window, the go-to popover, and the
/// bookmarks: Add Bookmark… first, then each kept view, which a click returns
/// the camera to.
///
/// Every entry is disabled while a playtest is running. Making or opening a
/// map re-reads every open tile and moves the editor's camera, and the
/// character is standing on the map being left. The others act on the
/// editor's free camera or on the map's tiles, and a playtest replaces the
/// camera with the session's and reads those tiles.
#[allow(clippy::too_many_arguments)]
fn map(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    camera: &mut EditorCamera,
    popovers: &mut Popovers,
    bookmarks: &crate::prefs::bookmarks::Bookmarks,
    map_open: &mut bool,
    new_map: &mut Option<crate::map::maps::Form>,
    playing: bool,
) {
    const HELD: &str = "Not while a playtest is running: the playtest controls the camera and the \
                        character is on this map. Press Ctrl+P to end the playtest.";
    let mut open_map: Option<(String, u32)> = None;
    let mut jump: Option<crate::prefs::bookmarks::Bookmark> = None;
    let mut ask_go = false;
    let mut ask_bookmark = false;
    let menu = ui
        .menu_button(format!("Map: {} ({})", session.map, session.map_id), |ui| {
            // The form is a modal of its own, drawn whether or not the map
            // window is open. See [`crate::ui::map::mapview`].
            if ui
                .add_enabled(!playing, egui::Button::new("New Map\u{2026}"))
                .on_hover_text(
                    "Create a new map: a Map.dbc row, a WDT with no tiles and the server's \
                     map_template row. The map opens with no tiles; add them with Create \
                     in Edit Map.",
                )
                .on_disabled_hover_text(HELD)
                .clicked()
            {
                *new_map = Some(crate::map::maps::Form::default());
                ui.close();
            }
            if ui
                .add_enabled(!playing, egui::Button::new("Map Properties\u{2026}"))
                .on_hover_text(
                    "Edit the open map: its Map.dbc row (name, instance type, PvP, levels, \
                     players, zone, descriptions, loading screen), the server's map_template \
                     row, and whether it is terrain or a single WMO.",
                )
                .on_disabled_hover_text(HELD)
                .clicked()
            {
                *new_map = Some(crate::map::maps::Form::editing(session.map_id, &session.map));
                ui.close();
            }
            ui.add_enabled_ui(!playing, |ui| {
                ui.menu_button("Open Map", |ui| {
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        for (id, name) in &session.maps {
                            let row = format!("{id:>4}  {name}");
                            if ui.selectable_label(session.map == *name, row).clicked() {
                                open_map = Some((name.clone(), *id));
                                ui.close();
                            }
                        }
                    });
                });
            })
            .response
            .on_disabled_hover_text(HELD);
            // Which tiles this map has is a fact about the session, and it is
            // the one thing in the editor not chosen with the pointer, so it
            // is here and not on the rail. See [`crate::ui::map::mapview`].
            if ui
                .add_enabled(!playing, egui::Button::new("Edit Map…"))
                .on_hover_text(
                    "Open the tile map: which tiles exist, and creating, deleting, \
                     copying and pasting tiles.",
                )
                .on_disabled_hover_text(HELD)
                .clicked()
            {
                *map_open = !*map_open;
                ui.close();
            }
            if ui
                .add_enabled(!playing, egui::Button::new("Go To…"))
                .on_hover_text("Move the camera: a bookmark, a position, a tile, or a zone.")
                .on_disabled_hover_text(HELD)
                .clicked()
            {
                ask_go = true;
                ui.close();
            }
            ui.add_enabled_ui(!playing, |ui| {
                ui.menu_button("Bookmarks", |ui| {
                    // Naming a view is a text field, so it is a popover like
                    // Go To…; see the module comment.
                    if ui
                        .button("Add Bookmark\u{2026}")
                        .on_hover_text("Save the current camera view as a named bookmark.")
                        .clicked()
                    {
                        ask_bookmark = true;
                        ui.close();
                    }
                    if !bookmarks.list.is_empty() {
                        ui.separator();
                    }
                    for mark in &bookmarks.list {
                        let label = match mark.map != session.map {
                            true => format!("{} · {}", mark.name, mark.map),
                            false => mark.name.clone(),
                        };
                        if ui.button(label).clicked() {
                            jump = Some(mark.clone());
                            ui.close();
                        }
                    }
                });
            })
            .response
            .on_disabled_hover_text(HELD);
        })
        .response
        .on_hover_text(
            "The open map and its Map.dbc id. The light chain, the collision world, \
             WorldMapArea and the server all key on the id.",
        );
    popovers.go_to.track(&menu);
    popovers.bookmark.track(&menu);
    if ask_go {
        popovers.go_to.show();
    }
    if ask_bookmark {
        popovers.bookmark.show();
    }
    if let Some((name, id)) = open_map {
        session.switch_map(name, id, camera);
    }
    if let Some(mark) = jump {
        super::popover::go_to_bookmark(&mark, camera, session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project that has applied nothing is cleared or deleted on one press.
    /// One that has applied anything needs its own name typed exactly, apart
    /// from surrounding spaces, because the press loses the only record of
    /// what to put back.
    #[test]
    fn a_project_with_rows_on_the_server_needs_its_name_typed() {
        let free = Doomed::new("goldshire".into(), false, &crate::server::held::Held::default());
        assert!(free.may_proceed());
        let held = crate::server::held::Held {
            rows: vec![("creatures", Some(3))],
            dbcs: Vec::new(),
        };
        let mut gated = Doomed::new("goldshire".into(), true, &held);
        assert!(!gated.may_proceed());
        gated.typed = "goldshir".into();
        assert!(!gated.may_proceed());
        gated.typed = " goldshire ".into();
        assert!(gated.may_proceed(), "surrounding spaces are not a mistyping");
        assert_eq!(gated.held.len(), 1);
    }
}
