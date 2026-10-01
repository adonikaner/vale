//! The bar across the top of the window: the Project menu and the Save
//! button, the Server button, the Map menu, the workspace control, and the
//! controls that start and stop a playtest.
//!
//! ## What the bar holds
//!
//! Every control on this bar is about the session rather than about the thing
//! being edited: which project, which map, where the camera is, whether the work
//! is saved, which workspace is open, and whether the game is running. A control
//! that changes the world belongs in the inspector on the right, beside the tool
//! that owns it.
//!
//! ## Two menus, and what stays a button
//!
//! The bar is one row, and at 1280 points wide it had no room left: the
//! project's name, Save, Publish, Server, the map drop-down with its id, the
//! tile map button, Go to and four workspaces filled it, and a fifth workspace
//! did not fit. The controls about one subject are now one menu each:
//!
//! ```text
//! Project: <name>   Projects…        the project dialog
//!                   Publish…         the publish popover
//! Map: <name> (id)  Open map     >   every map, by id
//!                   Edit WDT/ADT…    the tile map window
//!                   Go to…           the go-to popover
//!                   Bookmarks    >   each kept view, which a click returns to
//! ```
//!
//! Each menu's button states what a person reads the bar for without opening
//! it: which project is open, and which map with its `Map.dbc` id. The id is
//! the number every other authority keys on (`WorldMapArea`, `Light.dbc`,
//! vmangos' tables, a `.tele`), and the directory name is usable in none of
//! them.
//!
//! A menu entry is a command. A form is still a popover: an egui menu closes
//! when anything inside it is clicked, which suits a list of commands and does
//! not suit text fields, so `Publish…` and `Go to…` open the same windows the
//! buttons opened, placed under the menu. See [`super::popover`].
//!
//! Three controls stay buttons. Save's label says what is unsaved, which is
//! read without a click. Server… opens one panel with two tabs, so a menu of
//! two entries would add a click and remove nothing. The playtest login
//! button's label states who the playtest logs in as, and gains " · cached"
//! while the query cache is kept.
//!
//! After the Map menu comes the workspace control ([`super::rail::workspaces`]):
//! World, Spells, Items, Quests. World returns to the last rail tool; the other
//! three replace the viewport. [`Subjects`] carries the tool and the rail's
//! memory into [`draw`].

use bevy_egui::egui;

use super::theme;
use crate::camera::EditorCamera;
use crate::playtest::{Login, Playtest};
use crate::session::EditSession;
use crate::ui::popover::Popovers;
use vale_client::assets::GameAssets;
use vale_client::glue::autologin::AutoLogin;
use bevy::prelude::*;

/// The text typed into the "go to" boxes.
///
/// Held as text rather than as numbers: a field bound to an `f32` re-formats
/// its contents on every keystroke, so it cannot hold "-94" on the way to
/// "-9450". The parse happens when the jump is requested.
#[derive(Resource, Default)]
pub struct GoTo {
    pub(super) x: String,
    pub(super) y: String,
    pub(super) tile_x: String,
    pub(super) tile_y: String,
    /// The name a new bookmark is given — see [`crate::bookmarks`].
    pub(super) bookmark: String,
}

/// The playtest's resources, as one parameter.
///
/// They are bundled because the right-hand end of this bar is everything that
/// starts and stops a playtest, and because bundling keeps [`draw`] under the
/// argument count. `ui::Playing`, one layer out, exists for the same reason.
pub struct Session<'a> {
    pub state: &'a mut Playtest,
    pub client: &'a mut vale_client::world::session::Session,
    pub login: &'a Login,
    pub auto: &'a mut AutoLogin,
    pub open: &'a mut crate::playtest::ShellOpen,
    /// The queue a save's database writes go on. See
    /// [`crate::server::queue`].
    pub queue: &'a mut crate::server::queue::ServerQueue,
    /// Where this machine's server is. The Save button's apply needs it and
    /// the bar's Server… button opens it.
    pub server: &'a mut crate::server::settings::ServerSettings,
    /// Whether the session Playtest starts keeps the query answers the server
    /// gives it. See [`vale_client::world::session::QueryCaches`], and
    /// [`crate::server::settings::ServerSettings::disable_caching`], the
    /// switch it is written from.
    pub caches: &'a mut vale_client::world::session::QueryCaches,
    /// Which step of a tile regeneration is running. Publish starts one; see
    /// [`crate::server::datadir::Step`].
    pub step: &'a crate::server::datadir::Step,
}

pub fn draw(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    camera: &mut EditorCamera,
    popovers: &mut Popovers,
    assets: &GameAssets,
    playing: &mut Session,
    map_open: &mut bool,
    subjects: &mut Subjects<'_>,
    // The kept views the Map menu lists. See [`crate::bookmarks`].
    bookmarks: &crate::bookmarks::Bookmarks,
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
        project(ui, session, assets, popovers, in_world, playing.queue, playing.server, playing.step);
        // The server sits beside the project because the two are the two
        // destinations of one save: the project folder and the database. The
        // button opens a form, so it is a popover like the others; see
        // [`super::popover`].
        let at_server = ui
            .button("Server…")
            .on_hover_text(
                "What this project has applied to the world database, Apply and Put back \
                 for each subject, and where this machine's vmangos is.",
            );
        popovers.server.track(&at_server);
        if at_server.clicked() {
            popovers.server.toggle(&at_server);
        }
        separator(ui);
        map(ui, session, camera, popovers, bookmarks, map_open, in_world);
        separator(ui);
        // The workspace control: World, then Spells, Items and Quests. World
        // returns to the last rail tool; the other three replace the viewport.
        // See [`super::rail`], which holds the world tools' rail too.
        let server = playing.server.resolve().is_some();
        super::rail::workspaces(ui, subjects.tool, subjects.rail, in_world, server);

        // The playtest controls sit at the far right, apart from the rest,
        // because they are the only controls here that take the editor out of
        // editing. The layout is right to left, so the first control added is
        // the rightmost.
        //
        // While the panels are open over a playtest, this end of the bar holds
        // the playtest's own two controls, in the place of the button that
        // started it. Leaving the `playtest` window up over the shell instead
        // would state the same thing in two places on one screen.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match in_world {
                true => running(ui, playing, session),
                false => not_running(ui, session, assets, popovers, playing),
            }
        });
    });
}

/// What the workspace control changes: the tool in use, and the rail's memory
/// of the last world tool. Passed into [`draw`] as one parameter.
pub struct Subjects<'a> {
    pub tool: &'a mut crate::tools::Tool,
    pub rail: &'a mut super::rail::Rail,
}

/// The right-hand end of the bar while a playtest is running: the playtest's
/// state, and the two ways out of it.
///
/// The layout is right to left, so the first control added is the rightmost.
fn running(ui: &mut egui::Ui, playing: &mut Session, session: &mut EditSession) {
    if ui
        .button("End Playtest")
        .on_hover_text("Ctrl+P. Drops the connection and returns to the tools.")
        .clicked()
    {
        crate::playtest::stop(playing.state, playing.client, playing.auto, session);
        return;
    }
    if ui
        .button("Hide Panels")
        .on_hover_text("Ctrl+E. Leaves the playtest running, with the game's own screen.")
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

/// The right-hand end of the bar while no playtest is running: who it would
/// log in as, and the button that starts it.
///
/// The login control is a separate button beside the Playtest button.
fn not_running(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    assets: &GameAssets,
    popovers: &mut Popovers,
    playing: &mut Session,
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
    let who = ui
        .button(label)
        .on_hover_text("Who the playtest logs in as, whether it asks, and whether the client keeps its query cache.");
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
            "Save the project and play the map you are editing. Ctrl+P, and \
             Ctrl+P again to come back.",
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

/// The project section: the Project menu and the Save button.
///
/// The menu's button is the open project's name, which is the one thing on
/// this bar a person can lose work by being wrong about. Its entries open the
/// project dialog and the publish popover.
///
/// The Save button's label names what it will save, for example `Save 2 tiles
/// and server rows`, or `Saved` when there is nothing. A bare count such as
/// `Save 1`, which counted tiles with unsaved changes, does not say what it
/// counts.
fn project(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    assets: &GameAssets,
    popovers: &mut Popovers,
    playing: bool,
    // The queue a save's database writes go on. It is the only thing this
    // section needs from the server beyond where the server is.
    queue: &mut crate::server::queue::ServerQueue,
    // Where the server is, for the half of a save that reaches it.
    server: &crate::server::settings::ServerSettings,
    // The progress line a publish's tile regeneration writes.
    step: &crate::server::datadir::Step,
) {
    // A publish's tile regeneration reads the folder a second publish would
    // write, so Publish is disabled while one runs.
    let regenerating = queue.busy();
    let (mut ask_projects, mut ask_publish) = (false, false);
    let menu = ui
        .menu_button(format!("Project: {}", session.project.name), |ui| {
            // Disabled while a playtest is running. Switching saves what is
            // unsaved, rebuilds the archive overlay, empties the undo stack
            // and re-reads every tile on screen, and the running client reads
            // that overlay for the ground the character stands on. Ending the
            // playtest first is one keypress and leaves nothing half-swapped.
            if ui
                .add_enabled(!playing, egui::Button::new("Projects…"))
                .on_hover_text("Which project the edits go into, and making another one.")
                .on_disabled_hover_text(
                    "Not while a playtest is running: switching rebuilds the archive \
                     overlay the running game is reading. Ctrl+P first.",
                )
                .clicked()
            {
                ask_projects = true;
                ui.close();
            }
            // Disabled while a playtest is running. Publishing writes a new
            // archive into the folder whose chain this process has open, and
            // the running client reads that chain for the ground the
            // character is standing on. Saving is what reaches a running
            // game; publishing is for finished work.
            if ui
                .add_enabled(!playing && !regenerating, egui::Button::new("Publish…"))
                .on_hover_text(
                    "Write a patch: one folder under the project's publish\\ holding the \
                     client archive, the server's DBCs, a migration of the rows and the \
                     server's maps, vmaps and mmaps, with a README saying where each goes. \
                     Nothing is applied to this machine.",
                )
                .on_disabled_hover_text(match playing {
                    true => "Not while a playtest is running.",
                    false => "A server write is still running.",
                })
                .clicked()
            {
                ask_publish = true;
                ui.close();
            }
        })
        .response
        .on_hover_text("The open project. Every edit is written into its folder.");
    // Both windows are placed from the menu's button, tracked every frame so
    // `--projects` can open one without a press. See `Popover::show`.
    popovers.project.track(&menu);
    popovers.publish.track(&menu);
    if ask_projects {
        popovers.project.show();
    }
    if ask_publish {
        popovers.publish.show();
    }

    // Three kinds of unsaved work: tiles, tables and server rows. The same
    // press writes the server rows, so they are counted too. Without them the
    // label read "Saved" and the button was disabled while a creature or
    // waypoint edit was outstanding, which left Ctrl+S as the only way to
    // write it and nothing on screen saying a save was owed. See
    // `EditSession::server_unsaved`.
    let tiles = session.unsaved.len();
    let tables = session.unsaved_tables.len();
    let mut parts: Vec<String> = Vec::new();
    if tiles > 0 {
        parts.push(plural(tiles, "tile", "tiles"));
    }
    if tables > 0 {
        parts.push(plural(tables, "table", "tables"));
    }
    if session.server_unsaved() {
        // Not a count: the two stores hold rows and whole paths, which do not
        // add up to one number. The label says only that a save is owed.
        parts.push("server rows".to_string());
    }
    let anything = !parts.is_empty();
    let label = match anything {
        false => "Saved".to_string(),
        true => format!("Save {}", parts.join(" and ")),
    };
    // Saving during a playtest is also the reload. The write puts the edit in
    // the project folder and `crate::playtest::republish` puts it where the
    // running client reads it, so the next cast, the next entity to come into
    // view and the next model to be hung are the edited ones. What is already
    // standing keeps the handles it has.
    if ui
        .add_enabled(anything, egui::Button::new(label))
        .on_hover_text(match playing {
            true => format!(
                "Write the tiles, tables and server rows you have changed into {}, then \
                 hand them to the running game. Ctrl+S.",
                session.project.root.display()
            ),
            false => format!(
                "Write the tiles, tables and server rows you have changed into {}. \
                 Ctrl+S.",
                session.project.root.display()
            ),
        })
        .on_disabled_hover_text("Nothing has changed since the last save.")
        .clicked()
    {
        session.save_all();
        session.save_all_tables();
        // The server half of the same press: every subject's SQL written into
        // the project, the statements run if Apply on save is on, and a
        // `.reload` for each table that can be made live. One call, because a
        // save is one gesture; see [`crate::server::save`].
        crate::server::save(session, assets, server, queue);
        if playing {
            crate::playtest::republish(session, assets);
            session.status = "saved and republished to the playtest".to_string();
        }
    }

    // A spinner beside Save while a server write runs, with the step's name on its hover
    // text. The step's name is a sentence that changes every few seconds, and
    // a bar whose width follows it would move everything to its right; the
    // sentence and the fraction are on the status line's bar instead.
    if regenerating {
        let line = step.line();
        let spinner = ui.add(egui::Spinner::new().size(14.0).color(theme::ACCENT));
        match line.is_empty() {
            true => spinner.on_hover_text("A server write is running."),
            false => spinner.on_hover_text(line),
        };
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
pub struct Projects {
    pub new_name: String,
    /// Whether `--projects` has been acted on.
    pub shown_once: bool,
    /// Every project under `Edit\`, with what each holds. Read when the
    /// dialog opens and after it makes one, not every frame, because a summary
    /// walks the folder.
    pub known: Vec<(String, vale_edit::project::Summary)>,
    pub listed: bool,
    /// The open project's manifest, built with [`Self::known`] and dropped
    /// with it. See `super::manifest`.
    pub manifest: Option<super::manifest::Report>,
    /// The project a delete or clear is awaiting confirmation for, and `None`
    /// otherwise.
    ///
    /// Deleting a project is the one action in this editor that destroys work
    /// and cannot be undone, so it is the one action here that asks first.
    /// The dialog's own note gives the reason a switch is not confirmed: a
    /// confirmation nobody reads is how work gets lost, and a switch saves
    /// first. A delete does not save first, so that reason does not apply.
    pub confirming: Option<Doomed>,
}

/// A project one of the two destroying buttons has been pressed on, and which
/// button it was.
#[derive(Clone, PartialEq, Eq)]
pub struct Doomed {
    pub name: String,
    /// `true` for clear (delete the files and keep the folder), which is what
    /// the open project and the default project get in place of deletion.
    pub clear: bool,
    /// What the project has applied to the server, one line each, read once
    /// when the button was pressed. See `crate::server::held`. Empty for a
    /// project that has applied nothing.
    pub held: Vec<String>,
    /// What has been typed into the confirmation's name field.
    pub typed: String,
}

impl Doomed {
    pub fn new(name: String, clear: bool, held: &crate::server::held::Held) -> Doomed {
        Doomed {
            name,
            clear,
            held: held.lines(),
            typed: String::new(),
        }
    }

    /// Whether the destructive button is enabled: always for a project that
    /// has applied nothing, and otherwise only once the project's name has
    /// been typed. The record of what to put back is in the folder being
    /// deleted or cleared, so proceeding leaves the applied rows in the
    /// database with no record of how to put them back.
    pub fn may_proceed(&self) -> bool {
        self.held.is_empty() || self.typed.trim() == self.name
    }
}

impl Projects {
    /// Read the folders again.
    pub fn refresh(&mut self, install: &std::path::Path) {
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

/// The Map menu: which map is open, and the four things done to the map as a
/// whole.
///
/// The button shows the map's name with its `Map.dbc` id. The entries are the
/// map list, the tile map window, the go-to popover, and the bookmarks, each
/// of which a click returns the camera to.
///
/// Every entry is disabled while a playtest is running. Opening a map re-reads
/// every open tile and moves the editor's camera, and the character is
/// standing on the map being left. The other three act on the editor's free
/// camera or on the map's tiles, and a playtest replaces the camera with the
/// session's and reads those tiles.
fn map(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    camera: &mut EditorCamera,
    popovers: &mut Popovers,
    bookmarks: &crate::bookmarks::Bookmarks,
    map_open: &mut bool,
    playing: bool,
) {
    const HELD: &str = "Not while a playtest is running: the session has the camera and the \
                        character is on this map. Ctrl+P to come back to the tools.";
    let mut open_map: Option<(String, u32)> = None;
    let mut jump: Option<crate::bookmarks::Bookmark> = None;
    let mut ask_go = false;
    let menu = ui
        .menu_button(format!("Map: {} ({})", session.map, session.map_id), |ui| {
            ui.add_enabled_ui(!playing, |ui| {
                ui.menu_button("Open map", |ui| {
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
            // is here and not on the rail. See [`super::mapview`].
            if ui
                .add_enabled(!playing, egui::Button::new("Edit WDT/ADT…"))
                .on_hover_text(
                    "The map from above: which tiles exist, and making, deleting, \
                     copying and pasting them.",
                )
                .on_disabled_hover_text(HELD)
                .clicked()
            {
                *map_open = !*map_open;
                ui.close();
            }
            if ui
                .add_enabled(!playing, egui::Button::new("Go to…"))
                .on_hover_text("Move the camera: a position, a tile, or a zone. Keeps bookmarks.")
                .on_disabled_hover_text(HELD)
                .clicked()
            {
                ask_go = true;
                ui.close();
            }
            ui.add_enabled_ui(!playing, |ui| {
                ui.menu_button("Bookmarks", |ui| {
                    if bookmarks.list.is_empty() {
                        ui.label(
                            egui::RichText::new("none yet: Go to… keeps one")
                                .size(theme::SMALL)
                                .color(theme::INK_FAINT),
                        );
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
    if ask_go {
        popovers.go_to.show();
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
