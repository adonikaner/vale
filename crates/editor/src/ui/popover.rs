//! The windows the top bar's buttons open: go-to, playtest login, the project
//! dialog, the server panel and the publish popover. Each holds text fields.
//!
//! ## Why the popovers are windows and not menus
//!
//! An egui menu (`ui.menu_button`) closes when anything inside it is clicked.
//! That suits a menu, whose items are commands, and does not suit a panel of
//! text fields: clicking into the password box closed the box. So each popover
//! is an `egui::Window` with an open flag, opened from its button and positioned
//! under it. A window keeps focus and a `TextEdit` inside one works. Clicking
//! outside closes it because [`Popovers::follow`] closes it, not because the
//! container does.
//!
//! ## Why each popover reports its rectangle to the shell
//!
//! [`super::Chrome`] is how a tool knows a press was on the interface and not
//! on the world. A docked panel is in the chrome because the root `Ui` shrinks
//! around it. A window floats over the viewport and shrinks nothing, so a press
//! on the password box would otherwise read as a press on the ground behind
//! it, the same fault the docked panels had. Each popover returns its rectangle
//! and the shell adds it to the chrome for the frame.

use bevy::prelude::*;
use bevy_egui::egui;

use super::theme;
use crate::camera::EditorCamera;
use crate::places::Places;
use crate::playtest::Login;

/// Which popovers are open, and where each was opened from.
#[derive(Resource, Default)]
pub struct Popovers {
    pub go_to: Popover,
    pub login: Popover,
    /// Which project is open, and making another one.
    pub project: Popover,
    /// Where this machine's server is, and what this project has done to it.
    pub server: Popover,
    /// The patch a publish is about to write: its name, and the button.
    pub publish: Popover,
    /// The name typed into the publish popover. Empty means the UTC stamp.
    pub patch_name: String,
    /// Whether `--server` has already opened the server popover. Its anchor is
    /// the button's rectangle, which does not exist until the bar has been
    /// drawn once, so the flag cannot be acted on at startup. `--projects`
    /// carries a flag of its own for the same reason. See [`Popover::show`].
    pub server_shown_once: bool,
}

/// One popover: whether it is open and where its button is.
pub struct Popover {
    open: bool,
    /// The bottom-left corner of the button that opens it. The window is
    /// placed under that point rather than where egui last left it.
    under: egui::Pos2,
    /// The button's own rectangle. [`Popovers::follow`] excuses a press inside
    /// it: a press on the button closes the window through `toggle`, and if
    /// `follow` also closed it the button would close and reopen the window in
    /// one click.
    button: egui::Rect,
}

impl Default for Popover {
    /// Closed, and anchored nowhere. `egui::Rect` has no `Default`, and
    /// `NOTHING` is the empty rectangle that contains no point, which is the
    /// right starting value for both fields: nothing has been pressed yet.
    fn default() -> Popover {
        Popover {
            open: false,
            under: egui::Pos2::ZERO,
            button: egui::Rect::NOTHING,
        }
    }
}

impl Popover {
    /// A click on the button that owns this: open it, or shut it if it is
    /// already open.
    pub fn toggle(&mut self, button: &egui::Response) {
        self.open = !self.open;
        self.under = button.rect.left_bottom() + egui::vec2(0.0, 4.0);
        self.button = button.rect;
    }

    /// Record the button's current position, so a window resize does not leave
    /// the popover anchored to a stale rectangle.
    ///
    /// The anchor is recorded whether or not the popover is open. That is what
    /// lets [`Self::show`] open one without a press: a scripted run has no
    /// pointer and cannot press the button.
    pub fn track(&mut self, button: &egui::Response) {
        self.under = button.rect.left_bottom() + egui::vec2(0.0, 4.0);
        self.button = button.rect;
    }

    /// Open it without a press. The anchor is whatever [`Self::track`] last
    /// recorded, so this lands under the button from the frame after.
    pub fn show(&mut self) {
        self.open = true;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
}

impl Popovers {
    /// Close every open popover the pointer was pressed outside of.
    ///
    /// A popover is transient and a click away from it dismisses it. egui's own
    /// windows do not close on an outside click, so this does it for them. A
    /// press on the button that opened the popover is excused: `toggle` already
    /// handles that press, and closing here as well would close and reopen the
    /// window in the same frame.
    fn follow(&mut self, ctx: &egui::Context, rects: &[(bool, egui::Rect)]) {
        let Some(at) = ctx.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        if !ctx.input(|i| i.pointer.any_pressed()) {
            return;
        }
        for (which, popover) in [
            &mut self.go_to,
            &mut self.login,
            &mut self.project,
            &mut self.server,
            &mut self.publish,
        ]
        .into_iter()
        .enumerate()
        {
            if !popover.open {
                continue;
            }
            let inside = rects
                .get(which)
                .is_some_and(|(shown, rect)| *shown && rect.contains(at));
            if !inside && !popover.button.contains(at) {
                popover.open = false;
            }
        }
    }
}

/// Draw every open popover and return their rectangles for the chrome.
///
/// The windows are drawn against the context rather than into the root `Ui`,
/// because they float over the panels and the docked regions must not shrink
/// around them.
pub fn draw(
    ctx: &egui::Context,
    popovers: &mut Popovers,
    camera: &mut EditorCamera,
    go_to: &mut super::topbar::GoTo,
    places: &Places,
    login: &mut Login,
    projects: &mut super::topbar::Projects,
    session: &mut crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    bookmarks: &mut crate::bookmarks::Bookmarks,
    server: &mut crate::server::settings::ServerSettings,
    queue: &mut crate::server::queue::ServerQueue,
    standings: &mut super::sync::Standings,
    now: f64,
    // Whether a playtest is running, which disables the two buttons that
    // rebuild the project's archive; and the step a tile regeneration is on.
    playing: bool,
    step: &crate::server::datadir::Step,
) -> Vec<egui::Rect> {
    let mut rects = Vec::new();
    let mut seen: Vec<(bool, egui::Rect)> = vec![(false, egui::Rect::NOTHING); 5];

    if popovers.go_to.open {
        if let Some(response) = window(ctx, "go-to", popovers.go_to.under, |ui| {
            go_somewhere(
                ui,
                camera,
                go_to,
                places,
                bookmarks,
                session,
                &mut popovers.go_to.open,
            );
        }) {
            seen[0] = (true, response);
            rects.push(response);
        }
    }
    if popovers.login.open {
        if let Some(response) = window(ctx, "playtest-login", popovers.login.under, |ui| {
            who(ui, login);
        }) {
            seen[1] = (true, response);
            rects.push(response);
        }
    }
    // The project list is a modal dialog, not a popover. Its contents are a
    // table of choices with consequences: every row is a switch that saves
    // and empties the undo stack. So it sits in the middle with a backdrop,
    // and a press outside it lands on the backdrop and nothing else. For the
    // same reason the whole viewport is reported as chrome while it is up,
    // and `follow` is told the press was inside.
    if popovers.project.open {
        let mut ask_server = false;
        let response = egui::Modal::new(egui::Id::new("projects")).show(ctx, |ui| {
            which_project(
                ui,
                projects,
                session,
                assets,
                &mut popovers.project.open,
                &mut ask_server,
            );
        });
        if response.should_close() {
            popovers.project.open = false;
        }
        // The confirmation's Put back first button set `ask_server`: the
        // dialog closes and the Server popover opens in its place. That
        // popover holds every block's Put back.
        if ask_server {
            popovers.project.open = false;
            popovers.server.show();
        }
        let whole = ctx.viewport_rect();
        seen[2] = (true, whole);
        rects.push(whole);
    }
    if !popovers.project.open {
        projects.listed = false;
        // A pending confirmation is dropped with the dialog. A dialog is
        // closed by Escape and by the backdrop as well as by its own button,
        // so this is the one place that sees every way out of it.
        projects.confirming = None;
    }
    if popovers.server.open {
        if let Some(response) = window(ctx, "server", popovers.server.under, |ui| {
            the_server(ui, session, assets, server, queue, standings, now, playing, step);
        }) {
            seen[3] = (true, response);
            rects.push(response);
        }
    }
    if popovers.publish.open {
        let mut close = false;
        if let Some(response) = window(ctx, "publish", popovers.publish.under, |ui| {
            close = the_patch(ui, &mut popovers.patch_name, session, assets, server, queue, step, playing);
        }) {
            seen[4] = (true, response);
            rects.push(response);
        }
        if close {
            popovers.publish.open = false;
        }
    }
    popovers.follow(ctx, &seen);
    rects
}

/// The publish popover: what a publish is about to write, the patch's name,
/// and the button that writes it.
///
/// It is a popover rather than a bare button because a patch has a name, and
/// the name is the one thing about it a person may want to choose. Returns
/// whether the window is to close.
#[allow(clippy::too_many_arguments)]
fn the_patch(
    ui: &mut egui::Ui,
    name: &mut String,
    session: &mut crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    server: &crate::server::settings::ServerSettings,
    queue: &mut crate::server::queue::ServerQueue,
    step: &crate::server::datadir::Step,
    playing: bool,
) -> bool {
    use crate::server::patch;

    ui.set_width(440.0);
    ui.label(egui::RichText::new("Publish a patch").strong().size(14.0));
    theme::note(
        ui,
        match server.copy_archive {
            false => {
                "One folder under the project's publish\\, holding everything a server and \
                 its players need with a README saying where each file goes. Nothing is \
                 applied to this machine's install, server or database."
            }
            true => {
                "One folder under the project's publish\\, holding everything a server and \
                 its players need with a README saying where each file goes. The client \
                 archive is also written into this install's Data folder (the Server \
                 panel's switch); the server and the database are not touched."
            }
        },
    );
    ui.add_space(8.0);
    let last = patch::last(&session.project);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("name").color(theme::INK_DIM));
        ui.add(
            egui::TextEdit::singleline(name)
                .desired_width(240.0)
                .hint_text(patch::default_name()),
        )
        .on_hover_text("A folder name. Empty for the UTC stamp.");
    });
    let chosen = match name.trim().is_empty() {
        true => None,
        false => Some(name.trim().to_string()),
    };
    let valid = chosen.as_deref().is_none_or(patch::is_a_name);
    if !valid {
        ui.label(egui::RichText::new("not a folder name").small().color(theme::BAD));
    }
    let exists = chosen
        .as_deref()
        .and_then(|n| session.project.path_for(&format!("{}\\{n}", patch::DIR)))
        .is_some_and(|p| p.is_dir());
    if exists {
        theme::note(ui, "A patch of this name exists and will be rewritten.");
    }
    match &last {
        Some(last) => theme::note(
            ui,
            format!("Server tiles unchanged since patch {last} are taken from it rather than rebuilt."),
        ),
        None => theme::note(ui, "The first patch: every tile the project carries is built."),
    }
    let busy = queue.busy();
    let mut close = false;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(valid && !playing && !busy, egui::Button::new("Publish"))
            .on_hover_text(
                "Save everything, then write client\\Patch-<X>.MPQ, server\\dbc, \
                 server\\sql\\<stamp>_world.sql, and the server's maps, vmaps and mmaps \
                 for every tile changed since the last patch. The tiles take minutes; \
                 the bar says which step is running, and the README and manifest are \
                 written when they finish.",
            )
            .on_disabled_hover_text(match (playing, busy) {
                (true, _) => "Not while a playtest is running.",
                (_, true) => "A server write is still running.",
                _ => "Type a folder name, or clear the box for the stamp.",
            })
            .clicked()
        {
            let chosen = chosen.clone().unwrap_or_else(patch::default_name);
            session.status = match patch::publish(session, assets, server, queue, step, &chosen) {
                Ok(said) => said,
                Err(e) => format!("publish: {e}"),
            };
            bevy::prelude::info!("publish: {}", session.status);
            close = true;
        }
        if let Some(folder) = session.project.path_for(patch::DIR) {
            if !patch::patches(&session.project).is_empty()
                && ui
                    .button("Open folder")
                    .on_hover_text(folder.display().to_string())
                    .clicked()
            {
                let _ = std::process::Command::new("explorer").arg(&folder).spawn();
            }
        }
    });
    let had = patch::patches(&session.project);
    if !had.is_empty() {
        ui.add_space(4.0);
        theme::note(ui, format!("patches so far: {}", had.join(", ")));
    }
    close
}

/// The server popover: where this machine's server is, and what this project
/// has done to it.
///
/// The panel has three kinds of section: where the server is, what a playtest
/// does with it, and what this project has done to it. The third is
/// [`super::sync`], which holds every server operation in one place. Which
/// rows and which columns were written is the project's own `sql\` folder,
/// which is plain text.
///
/// The panel is on the top bar rather than in the project dialog, where it
/// first was. The connection is a setting of this machine and not of the
/// project, so it did not belong in a per-project window; and the first
/// question asked of the feature was where its configuration was, so it was
/// moved to where the bar shows it.
fn the_server(
    ui: &mut egui::Ui,
    session: &mut crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    server: &mut crate::server::settings::ServerSettings,
    // The queue an Apply, a Put back and the Test button go on. See
    // [`crate::server::queue`].
    queue: &mut crate::server::queue::ServerQueue,
    // Where each subject last stood. This is a cache and not a fact: the panel
    // is drawn sixty times a second and one of its answers reads a whole DBC.
    // See [`super::sync::Standings`].
    standings: &mut super::sync::Standings,
    now: f64,
    playing: bool,
    step: &crate::server::datadir::Step,
) {
    use crate::server::settings::Source;

    ui.set_width(520.0);
    ui.label(egui::RichText::new("The server").strong().size(14.0));
    theme::note(
        ui,
        "vmangos reads its world from a database, not from the archives — so an edit to a \
         spell is a file for the client and a row for the server. A save writes both.",
    );
    ui.add_space(8.0);

    // ---- where it is.
    theme::heading(ui, "Where it is");
    let found = server.resolve();
    let from_env = matches!(found, Some((_, Source::Env(_))));
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("mangosd.conf").color(theme::INK_DIM));
        let field = ui.add_enabled(
            !from_env,
            egui::TextEdit::singleline(&mut server.conf)
                .desired_width(300.0)
                .hint_text("C:\\MaNGOS"),
        );
        // The setting is written when the box loses focus, not on every
        // keystroke. A file rewritten per character is written while the
        // person is still deciding what to type.
        if field.lost_focus() {
            server.conf = server.conf.trim().to_string();
            server.save(&assets.root);
            server.tried = None;
        }
    });
    theme::note(
        ui,
        "The folder or the file. Its WorldDatabase.Info line is the connection, so the \
         password stays in the server's own conf and not in this editor.",
    );

    match &found {
        Some((at, source)) => {
            ui.label(
                egui::RichText::new(format!("{}  ({})", at.line(), source.line()))
                    .small()
                    .color(theme::GOOD),
            );
        }
        None => {
            ui.label(
                egui::RichText::new("no database — nothing is applied, only written to sql\\")
                    .small()
                    .color(theme::WARN),
            );
        }
    }
    // Say when the environment overrides the box, rather than leaving a field
    // that accepts typing and does nothing. The precedence is in `settings`.
    if from_env {
        theme::note(
            ui,
            "The environment is set and wins over this box, so the box is held. Unset \
             VALE_WORLDDB and VALE_MANGOSD to use it.",
        );
    }

    ui.horizontal(|ui| {
        if ui
            .add_enabled(found.is_some() && !queue.testing(), egui::Button::new("Test"))
            .on_hover_text("Connect, count the rows, and change nothing.")
            .on_disabled_hover_text("There is nowhere to connect to.")
            .clicked()
        {
            // The test runs on a worker, so a server that is not answering
            // holds the button and not the window. See
            // [`crate::server::queue`].
            if let Some((at, _)) = server.resolve() {
                queue.test(at);
            }
        }
        if let Some(done) = queue.tested() {
            server.tried = Some(done);
        }
        if queue.testing() {
            ui.label(
                egui::RichText::new("connecting\u{2026}")
                    .small()
                    .color(theme::INK_DIM),
            );
        }
        match &server.tried {
            Some(Ok(line)) => {
                ui.label(egui::RichText::new(line).small().color(theme::GOOD));
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(e).small().color(theme::BAD));
            }
            None => {}
        }
    });

    // ---- its tiles, which are files and not rows.
    ui.add_space(10.0);
    theme::heading(ui, "Its tiles");
    let tools_from_env = std::env::var_os(vale_mangos::datadir::TOOLS_ENV).is_some();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("vmangos tools").color(theme::INK_DIM));
        let field = ui.add_enabled(
            !tools_from_env,
            egui::TextEdit::singleline(&mut server.tools)
                .desired_width(300.0)
                .hint_text("C:\\vmangos\\core\\bin\\Release"),
        );
        if field.lost_focus() {
            server.tools = server.tools.trim().to_string();
            server.save(&assets.root);
        }
    });
    theme::note(
        ui,
        "The folder holding mapextractor, vmapextractor, VMapAssembler and \
         MoveMapGenerator, a patched build. A Release build: the \
         navmesh generator is minutes a tile optimised and unusable otherwise.",
    );
    if tools_from_env {
        theme::note(
            ui,
            "VALE_VMANGOS_TOOLS is set and wins over this box, so the box is held.",
        );
    }
    // The tools folder is checked once per folder, not once per frame: the
    // check runs both extractors for their usage text. See
    // `ServerSettings::tools_status`.
    let asked = std::env::var(vale_mangos::datadir::TOOLS_ENV)
        .unwrap_or_else(|_| server.tools.trim().to_string());
    if server.tools_status.as_ref().map(|(dir, _)| dir.as_str()) != Some(asked.as_str()) {
        let status = crate::server::datadir::tools(server).and_then(|tools| {
            tools.check_patched()?;
            Ok(format!(
                "found, patched{}",
                match (&tools.off_mesh, &tools.config) {
                    (Some(_), Some(_)) => "; offmesh.txt and config.json found",
                    (None, None) => {
                        "; offmesh.txt and config.json not found — the navmesh is built \
                         without them"
                    }
                    (Some(_), None) => "; config.json not found",
                    (None, Some(_)) => "; offmesh.txt not found",
                }
            ))
        });
        server.tools_status = Some((asked, status));
    }
    match server.tools_status.as_ref().map(|(_, status)| status) {
        Some(Ok(line)) => {
            ui.label(egui::RichText::new(line).small().color(theme::GOOD));
        }
        Some(Err(e)) => {
            ui.label(egui::RichText::new(e).small().color(theme::WARN));
        }
        None => {}
    }
    match crate::server::datadir::data_dir(server) {
        Ok(dir) => {
            ui.label(
                egui::RichText::new(format!("DataDir {}", dir.display()))
                    .small()
                    .color(theme::INK_DIM),
            );
        }
        Err(e) => {
            ui.label(egui::RichText::new(e).small().color(theme::WARN));
        }
    }
    if ui
        .checkbox(&mut server.regenerate_tiles, "Regenerate tiles on publish")
        .on_hover_text(
            "Publish runs the four tools over every tile this project changed since the \
             last publish and writes the results into the server's maps, vmaps and mmaps. \
             A terrain edit is a second of extraction and a navmesh build for the tile and \
             its four neighbours; a moved building adds the vmap half of the whole map.",
        )
        .changed()
    {
        server.save(&assets.root);
    }
    // The tile step on its own, so it can be checked apart from the rest of a
    // publish. The map window has the per-tile form of this button.
    let changed = standings.changed_tiles(session, assets, now);
    let busy = queue.busy();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !playing && !busy && changed > 0,
                egui::Button::new(format!("Regenerate changed tiles ({changed})")),
            )
            .on_hover_text(
                "Rebuild this project's archive, then regenerate the server's files for \
                 every tile changed since the last regeneration. What Publish does for the \
                 tiles, and nothing else. Minutes; the bar says which step is running. \
                 Restart the server afterwards.",
            )
            .on_disabled_hover_text(match (playing, busy) {
                (true, _) => "Not while a playtest is running: it rewrites the archive the \
                              running game has open.",
                (_, true) => "A server write is still running.",
                _ => "No tile has changed since the last regeneration.",
            })
            .clicked()
        {
            session.status =
                crate::server::datadir::regenerate(session, assets, server, queue, step, None);
            standings.forget();
        }
        if busy {
            let line = step.line();
            let spinner = ui.add(egui::Spinner::new().size(14.0).color(theme::ACCENT));
            match line.is_empty() {
                true => spinner.on_hover_text("A server write is running."),
                false => spinner.on_hover_text(line),
            };
        }
    });
    theme::note(
        ui,
        "One tile at a time is the map window's Server files button, on Edit WDT/ADT. \
         The bar at the bottom says which step is running and how far along it is.",
    );
    // Whether the archive the tools read is also left in `Data\`. Off, the
    // tools read a staged copy of the install and nothing on this machine
    // changes; on, the archive goes where a client launched against this
    // install reads it. See `ServerSettings::copy_archive`.
    if ui
        .checkbox(&mut server.copy_archive, "Copy the client archive into Data")
        .on_hover_text(
            "A publish, and a regeneration of the server's tiles, also write this \
             project's Patch-<X>.MPQ into this install's Data folder, replacing the one \
             the project wrote before. Off, they build it where the tools can read it \
             and leave Data alone. Kept per project.",
        )
        .changed()
    {
        server.save(&assets.root);
    }

    // ---- what a publish hands over as rows.
    ui.add_space(10.0);
    theme::heading(ui, "Migration");
    let released = crate::server::release::released(&session.project);
    let last = match &released.id {
        Some(id) => format!(
            "last migration {id}, {} row(s) changed by it",
            released.entries.len()
        ),
        None => "no migration written yet".to_string(),
    };
    ui.label(egui::RichText::new(last).small().color(theme::INK_DIM));
    if ui
        .button("Write migration")
        .on_hover_text(
            "Save every subject's rows, then write publish\\migrations\\<stamp>_world.sql in \
             vmangos' own add_migration form, holding the change since this project's last \
             migration. What Publish does for the rows, and nothing else. Nothing is \
             written when nothing changed. Apply it with the mysql client.",
        )
        .clicked()
    {
        session.status = crate::server::release::migrate(session, assets);
        standings.forget();
    }

    // ---- what a playtest does with what it is told.
    ui.add_space(10.0);
    theme::heading(ui, "Playtesting");
    if ui
        .checkbox(&mut server.disable_caching, "Disable caching")
        .on_hover_text(
            "The client writes every query answer to WDB\\ and reads them back at the \
             next login, so an edited creature, item or quest keeps its old name and its \
             old stats. With caching off, a playtest starts with none of them and asks \
             the server for everything it meets.",
        )
        .changed()
    {
        server.save(&assets.root);
    }
    theme::note(
        ui,
        "It costs one query per thing the character meets, and it is what makes a row \
         edited here show up in the next playtest rather than after a restart.",
    );

    // ---- what this project has done to the server: every server operation.
    // See [`super::sync`], whose module comment says why they are in one
    // panel.
    ui.add_space(10.0);
    super::sync::project(
        ui,
        &mut super::sync::Work {
            session,
            assets,
            server,
            queue,
            standings,
            now,
        },
    );
}

/// The project dialog: which project the edits go into, the other projects
/// there are, and making another.
///
/// A project is a folder under `Edit\` and nothing else, with no manifest and
/// no format, so this lists the folders with what is in each and takes a name
/// for a new one.
///
/// The open project is its own section at the top, because it is what a
/// person opens the dialog to find out. In one flat list it was a row among
/// rows, distinguished only by the word "open" where the others had a button.
/// Under its name is its manifest (`super::manifest`): every file the folder
/// holds and what each one changes against the archives, read once when the
/// dialog opens. The other projects follow under their own heading, each with
/// a count by kind, then the new-project box, so the three things the dialog
/// does are three blocks in that order.
///
/// Switching saves first. That is stated on the row rather than asked about:
/// there is no case where somebody wants the work dropped, and a confirmation
/// nobody reads is how work gets lost.
///
/// Clear files and Delete are gated. `crate::server::held::held_by` reads what
/// the project has applied to the server from its revert files; the
/// confirmation lists it, and the destructive button is enabled only when
/// [`super::topbar::Doomed::may_proceed`] says so, which for a project that
/// holds anything means the project's name has been typed. For the open
/// project a Put back first button sets `ask_server`, and [`draw`] turns that
/// into closing this dialog and opening the Server popover. Discard, in
/// [`super::sync`], is not gated because it leaves the revert files in place.
fn which_project(
    ui: &mut egui::Ui,
    projects: &mut super::topbar::Projects,
    session: &mut crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    open: &mut bool,
    ask_server: &mut bool,
) {
    ui.set_width(680.0);
    if !projects.listed {
        projects.refresh(std::path::Path::new(&assets.root));
        projects.manifest = None;
    }
    if projects.manifest.is_none() {
        projects.manifest = Some(super::manifest::build(session, assets));
    }
    let here = session.project.name.clone();
    ui.label(egui::RichText::new("Projects").strong().size(14.0));
    theme::note(
        ui,
        "The open project is what the editor draws and what a playtest plays. Its files \
         shadow the archives; Publish packs them into the next patch archive.",
    );
    ui.add_space(8.0);

    // ---- the one that is open.
    theme::heading(ui, "Open project");
    let holds = projects
        .known
        .iter()
        .find(|(name, _)| *name == here)
        .map(|(_, summary)| summary.line())
        .unwrap_or_else(|| "empty".to_string());
    let tiles = session.unsaved.len();
    let tables = session.unsaved_tables.len();
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(&here)
                .strong()
                .size(15.0)
                .color(theme::ACCENT),
        );
        ui.label(egui::RichText::new(holds).color(theme::INK_DIM));
        ui.label(theme::number(session.project.root.display().to_string()).color(theme::INK_FAINT));
        if tiles + tables > 0 {
            ui.label(
                egui::RichText::new(format!("unsaved: {tiles} tiles, {tables} tables"))
                    .small()
                    .color(theme::WARN),
            );
        }
        // The open project cannot be deleted, so it is cleared instead.
        // Deleting the folder the session is writing into would leave it
        // writing into nothing; emptying it reaches the same end, and the
        // session forgets everything it read out of it. See
        // `EditSession::clear_open_project`. The press only arms the
        // confirmation below; `held_by` is read once here, at the press.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("Clear files")
                .on_hover_text(
                    "Throw away every file in this project. Nothing is saved first — what a \
                     save would write is what is being thrown away — and it cannot be undone.",
                )
                .clicked()
            {
                projects.confirming = Some(super::topbar::Doomed::new(
                    here.clone(),
                    true,
                    &crate::server::held::held_by(&session.project),
                ));
            }
        });
    });
    // The manifest: what the folder holds, file by file. Read once per
    // opening; `listed = false` drops it with the list.
    if let Some(report) = &projects.manifest {
        super::manifest::draw(ui, report, session);
    }

    // ---- the other projects.
    ui.add_space(10.0);
    let others: Vec<(String, vale_edit::project::Summary)> = projects
        .known
        .iter()
        .filter(|(name, _)| *name != here)
        .cloned()
        .collect();
    theme::heading(ui, "Other projects");
    let mut switch: Option<String> = None;
    if others.is_empty() {
        theme::note(ui, "There are no others. The name below makes one.");
    } else {
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new("projects-list")
                    .num_columns(4)
                    .spacing([14.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for (name, summary) in &others {
                            ui.label(egui::RichText::new(name).color(theme::INK));
                            ui.label(egui::RichText::new(summary.line()).small().color(theme::INK_DIM));
                            ui.label(
                                egui::RichText::new(match summary.modified {
                                    Some(at) => ago(std::time::SystemTime::now(), at),
                                    None => String::new(),
                                })
                                .small()
                                .color(theme::INK_FAINT),
                            );
                            ui.horizontal(|ui| {
                                if ui
                                    .button("Open")
                                    .on_hover_text(format!(
                                        "Save what is unsaved, then edit and playtest {name} instead."
                                    ))
                                    .clicked()
                                {
                                    switch = Some(name.clone());
                                }
                                // Delete is offered only for a project that
                                // is neither open nor the default. The open
                                // one is cleared from its own block above.
                                // The default is where edits go when no
                                // project has been chosen, and
                                // `project::delete` refuses it, so the button
                                // is disabled rather than offered and refused.
                                let is_default = name == vale_edit::project::DEFAULT;
                                let doom = ui
                                    .add_enabled(
                                        !is_default,
                                        egui::Button::new(
                                            egui::RichText::new("Delete").color(theme::BAD),
                                        ),
                                    )
                                    .on_hover_text(format!(
                                        "Delete {name} and everything in it. This cannot be undone."
                                    ))
                                    .on_disabled_hover_text(
                                        "The default project is where edits go when nobody has \
                                         said otherwise, so it cannot be deleted. Open it and \
                                         clear its files instead.",
                                    );
                                if doom.clicked() {
                                    let held = vale_edit::project::Project::open(&assets.root, name)
                                        .map(|project| crate::server::held::held_by(&project))
                                        .unwrap_or_default();
                                    projects.confirming = Some(super::topbar::Doomed::new(
                                        name.clone(),
                                        false,
                                        &held,
                                    ));
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
    }

    // ---- the confirmation, if a destructive button has been pressed.
    //
    // A modal of its own over the list rather than a frame inside it. Content
    // added to an open modal grows it, and egui centres an area from the size
    // it had the frame before, so the list jumped by half the frame's height
    // on the frame it appeared. A new modal is laid out before it is first
    // drawn, at a fixed width, and the list behind it keeps its size.
    if let Some(mut doomed) = projects.confirming.clone() {
        let holds = projects
            .known
            .iter()
            .find(|(name, _)| *name == doomed.name)
            .map(|(_, summary)| summary.clone())
            .unwrap_or_default();
        let what = match holds.total() {
            0 => "It is empty".to_string(),
            1 => "It holds 1 file".to_string(),
            n => format!("It holds {n} files: {}", holds.line()),
        };
        let mut decided = false;
        let response = egui::Modal::new(egui::Id::new("projects-doom")).show(ui.ctx(), |ui| {
            ui.set_width(460.0);
            ui.label(
                egui::RichText::new(match doomed.clear {
                    true => format!("Throw away every file in {}?", doomed.name),
                    false => format!("Delete {} and everything in it?", doomed.name),
                })
                .strong()
                .size(14.0)
                .color(theme::INK),
            );
            theme::note(ui, format!("{what}. This cannot be undone."));
            // The gate: a project that has applied to the server holds the
            // only record of what to put back, and Clear and Delete remove
            // it. See `crate::server::held`.
            if !doomed.held.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(
                        "It has applied to the server, and this folder holds the only \
                         record of what to put back:",
                    )
                    .color(theme::BAD),
                );
                for line in &doomed.held {
                    ui.label(
                        egui::RichText::new(format!("  {line}"))
                            .small()
                            .color(theme::BAD),
                    );
                }
                ui.add_space(4.0);
                theme::note(
                    ui,
                    match doomed.name == here {
                        true => {
                            "Put back first, from Server\u{2026} on the top bar. To proceed \
                             without putting back, type the project's name: the database \
                             keeps the rows and the server keeps the files, with nothing \
                             left that knows."
                        }
                        false => {
                            "Open the project and put back first, from Server\u{2026} on \
                             the top bar. To proceed without putting back, type the \
                             project's name: the database keeps the rows and the server \
                             keeps the files, with nothing left that knows."
                        }
                    },
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("name").color(theme::INK_DIM));
                    ui.add(
                        egui::TextEdit::singleline(&mut doomed.typed)
                            .hint_text(&doomed.name)
                            .desired_width(220.0),
                    );
                });
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if !doomed.held.is_empty() && doomed.name == here {
                    if ui
                        .button("Put back first")
                        .on_hover_text(
                            "Closes this and opens the Server panel. Put back on every block \
                             that has applied rows, then come back here.",
                        )
                        .clicked()
                    {
                        *ask_server = true;
                        decided = true;
                    }
                }
                if ui
                    .add_enabled(
                        doomed.may_proceed(),
                        egui::Button::new(
                            egui::RichText::new(match doomed.clear {
                                true => "Throw them away",
                                false => "Delete it",
                            })
                            .color(egui::Color32::from_rgb(0x0C, 0x16, 0x1E)),
                        )
                        .fill(theme::BAD),
                    )
                    .on_disabled_hover_text(
                        "Type the project's name above to proceed without putting back.",
                    )
                    .clicked()
                {
                    match doomed.clear {
                        // The open project: the session has to forget what it
                        // read out of the files as well.
                        true if doomed.name == here => {
                            session.clear_open_project(assets);
                        }
                        true => match vale_edit::project::clear(&assets.root, &doomed.name) {
                            Ok(went) => {
                                session.status = format!("{}: {went} files thrown away", doomed.name)
                            }
                            Err(e) => {
                                session.status = format!("could not clear {}: {e}", doomed.name)
                            }
                        },
                        false => match vale_edit::project::delete(&assets.root, &doomed.name) {
                            Ok(went) => {
                                session.status = format!("{} deleted, with {went} files", doomed.name)
                            }
                            Err(e) => {
                                session.status = format!("could not delete {}: {e}", doomed.name)
                            }
                        },
                    }
                    projects.listed = false;
                    decided = true;
                }
                if ui.button("Keep it").clicked() {
                    decided = true;
                }
            });
        });
        // Escape, or a press on the backdrop, keeps the project.
        if response.should_close() {
            decided = true;
        }
        projects.confirming = match decided {
            true => None,
            // Keep what was typed for the next frame.
            false => Some(doomed.clone()),
        };
    }

    // ---- making a new project.
    ui.add_space(10.0);
    theme::heading(ui, "New project");
    let mut make = false;
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut projects.new_name)
                .hint_text("name")
                .desired_width(220.0),
        );
        let named = vale_edit::project::is_a_name(&projects.new_name);
        make = (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            || ui
                .add_enabled(named, egui::Button::new("Create and open"))
                .on_disabled_hover_text(
                    "A project's name is a folder's name: no separators, no drive \
                     letters, and not empty.",
                )
                .clicked();
        if make && !named {
            make = false;
        }
    });
    if make {
        switch = Some(projects.new_name.trim().to_string());
    }

    theme::note(
        ui,
        match tiles + tables {
            0 => "Switching empties the undo stack: an entry holds bytes belonging to the \
                  folder it was made in."
                .to_string(),
            _ => format!(
                "Switching writes down what is unsaved first ({tiles} tiles, {tables} tables) \
                 and empties the undo stack: an entry holds bytes belonging to the folder it \
                 was made in."
            ),
        },
    );
    ui.add_space(6.0);
    if ui.button("Close").clicked() {
        *open = false;
    }

    if let Some(name) = switch {
        if session.switch_to(assets, &name) {
            projects.new_name.clear();
            projects.listed = false;
            *open = false;
        }
    }
}

/// How long before `now` a file was written, as `just now`, `4 min ago`,
/// `3 h ago` or `2 days ago`. The project list shows it so a person can tell
/// which project they were last working in.
fn ago(now: std::time::SystemTime, then: std::time::SystemTime) -> String {
    let secs = now.duration_since(then).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => match secs / 86_400 {
            1 => "1 day ago".to_string(),
            days => format!("{days} days ago"),
        },
    }
}

/// The window frame every popover shares: no title bar, no resize, placed
/// under the button that opened it.
///
/// The contents scroll rather than running off the bottom of the screen. A
/// popover sits at a fixed position under its button and grows downward to
/// hold its contents, so on a short viewport its lower part is off screen with
/// nothing to say so. The server panel is four subjects with an Apply, a Put
/// back and a paragraph each, and on a 1000-point screen `Quests`, the last of
/// the four, was below the bottom edge; a panel that ends in whitespace looks
/// complete, so the report was that quests appeared to have no Apply.
///
/// The height cap is the room between the button and the bottom edge, so a
/// panel that fits is laid out as before and no scrollbar appears. The two
/// popovers with a list inside keep their own inner caps; those bound a long
/// list, this bounds a short screen.
fn window(
    ctx: &egui::Context,
    id: &str,
    under: egui::Pos2,
    contents: impl FnOnce(&mut egui::Ui),
) -> Option<egui::Rect> {
    let room = (ctx.viewport_rect().bottom() - under.y - 24.0).max(160.0);
    egui::Window::new(id)
        .title_bar(false)
        .resizable(false)
        .fixed_pos(under)
        .frame(
            egui::Frame::default()
                .fill(theme::PANEL)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(10, 8)),
        )
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(room)
                .show(ui, contents);
        })
        .map(|response| response.response.rect)
}

/// The go-to popover: send the camera to a bookmark, a position, a tile, or a
/// zone.
///
/// The three typed forms match the three things a person has in hand. A
/// position is what `.gps` in the game prints and what the status line prints;
/// a tile is what the terrain checks and the file names use; a zone is a name
/// on the world map.
///
/// Every one of them goes through [`EditorCamera::go_to`], so every one of them
/// lands on the ground rather than at the height the camera was flying at.
fn go_somewhere(
    ui: &mut egui::Ui,
    camera: &mut EditorCamera,
    go_to: &mut super::topbar::GoTo,
    places: &Places,
    bookmarks: &mut crate::bookmarks::Bookmarks,
    session: &mut crate::session::EditSession,
    open: &mut bool,
) {
    ui.set_min_width(250.0);
    let here = camera.target;
    let tile = vale_assets::tile_for_position(here.x, here.y);

    // Bookmarks come first because a place somebody named is the place they
    // most often want. A bookmark is the whole view: the map, the focus with
    // its height, and the orbit. See `crate::bookmarks`.
    theme::heading(ui, "Bookmarks");
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut go_to.bookmark)
                .desired_width(140.0)
                .hint_text("name this view"),
        );
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let name = go_to.bookmark.trim().to_string();
        if (entered || ui.small_button("Keep").clicked()) && !name.is_empty() {
            bookmarks.add(crate::bookmarks::Bookmark {
                name,
                map: session.map.clone(),
                target: camera.target.to_array(),
                yaw: camera.yaw,
                pitch: camera.pitch,
                distance: camera.distance,
            });
            go_to.bookmark.clear();
        }
    });
    if bookmarks.list.is_empty() {
        theme::note(ui, "none yet: name the view and press Keep");
    }
    let mut jump: Option<crate::bookmarks::Bookmark> = None;
    let mut forget: Option<String> = None;
    for mark in &bookmarks.list {
        ui.horizontal(|ui| {
            let elsewhere = mark.map != session.map;
            let label = match elsewhere {
                true => format!("{} · {}", mark.name, mark.map),
                false => mark.name.clone(),
            };
            if ui
                .selectable_label(false, label)
                .on_hover_text(format!(
                    "{:.0}, {:.0}, {:.0} on {}{}",
                    mark.target[0],
                    mark.target[1],
                    mark.target[2],
                    mark.map,
                    match elsewhere {
                        true => " — opens that map first, saving what is open",
                        false => "",
                    }
                ))
                .clicked()
            {
                jump = Some(mark.clone());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Forget this bookmark.")
                    .clicked()
                {
                    forget = Some(mark.name.clone());
                }
            });
        });
    }
    if let Some(name) = forget {
        bookmarks.remove(&name);
    }
    if let Some(mark) = jump {
        if mark.map != session.map {
            if let Some((id, name)) = session
                .maps
                .iter()
                .find(|(_, name)| *name == mark.map)
                .cloned()
            {
                session.switch_map(name, id, camera);
            }
        }
        camera.target = Vec3::from(mark.target);
        camera.yaw = mark.yaw;
        camera.pitch = mark.pitch;
        camera.distance = mark.distance;
        // The height is the bookmark's own, so the ground is not asked for.
        // When the map changed, `switch_map` has already asked, and the tile
        // under the target decides.
        if mark.map == session.map {
            camera.wants_the_ground = false;
        }
        *open = false;
    }

    theme::heading(ui, "Position");
    ui.horizontal(|ui| {
        let entered = typed(ui, &mut go_to.x, &format!("{:.0}", here.x))
            | typed(ui, &mut go_to.y, &format!("{:.0}", here.y));
        if entered || ui.button("Go").clicked() {
            // A field left blank keeps the camera's current value, so an x
            // typed without a y is a sideways step and not a jump to y = 0.
            let x = go_to.x.trim().parse().unwrap_or(camera.target.x);
            let y = go_to.y.trim().parse().unwrap_or(camera.target.y);
            camera.go_to(Vec2::new(x, y));
            *open = false;
        }
    });

    theme::heading(ui, "Tile");
    ui.horizontal(|ui| {
        let entered = typed(ui, &mut go_to.tile_x, &tile.0.to_string())
            | typed(ui, &mut go_to.tile_y, &tile.1.to_string());
        if entered || ui.button("Go").clicked() {
            if let (Ok(x), Ok(y)) = (
                go_to.tile_x.trim().parse::<u32>(),
                go_to.tile_y.trim().parse::<u32>(),
            ) {
                if x < 64 && y < 64 {
                    camera.go_to(crate::places::middle_of_tile(x, y));
                    *open = false;
                }
            }
        }
    });

    theme::heading(ui, "Zone");
    if places.zones.is_empty() {
        // Every instance, and any map with no `WorldMapArea` rows. A note is
        // shown rather than an empty list, which reads as a list that has not
        // loaded yet.
        theme::note(ui, "this map has no zones on the world map");
        return;
    }
    egui::ScrollArea::vertical()
        .max_height(300.0)
        .show(ui, |ui| {
            for place in &places.zones {
                if ui.selectable_label(false, &place.name).clicked() {
                    camera.go_to(place.at);
                    *open = false;
                }
            }
        });
}

/// One text field. Returns whether Enter was pressed in it.
///
/// Enter means the same as the button beside the field. egui reports
/// `lost_focus` for a field that was typed into and then committed, and
/// checking the key as well distinguishes Enter from a click elsewhere. The
/// hint is the camera's current value, so an empty box shows the current
/// position rather than nothing.
fn typed(ui: &mut egui::Ui, text: &mut String, hint: &str) -> bool {
    let field = ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(64.0)
            .hint_text(hint),
    );
    field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// The playtest login popover: who a playtest logs in as, and whether it shows
/// the login screen.
///
/// Nothing in it is a setting of the editor's own. The account is
/// `WTF\Config.wtf`'s `accountName`, the character is `VALE_CHARACTER` or
/// `lastCharacterIndex`, and the password is the one stand-in this project
/// has. Nothing is written back. The popover lets all three be changed for the
/// session without a restart, which an editor needs because it logs in
/// repeatedly.
fn who(ui: &mut egui::Ui, login: &mut Login) {
    ui.set_min_width(260.0);
    theme::heading(ui, "Log in as");
    egui::Grid::new("playtest-login-grid")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(egui::RichText::new("account").color(theme::INK_DIM));
            ui.text_edit_singleline(&mut login.account);
            ui.end_row();
            ui.label(egui::RichText::new("password").color(theme::INK_DIM));
            ui.add(egui::TextEdit::singleline(&mut login.password).password(true));
            ui.end_row();
            ui.label(egui::RichText::new("character").color(theme::INK_DIM));
            ui.add(egui::TextEdit::singleline(&mut login.character).hint_text("last played"));
            ui.end_row();
        });
    let possible = login.can_go_straight_in();
    ui.add_enabled(
        possible,
        egui::Checkbox::new(&mut login.straight_in, "skip the login screen"),
    );
    if !possible {
        theme::note(
            ui,
            "skipping it needs both an account and a password; VALE_PASSWORD \
             fills the password in at startup",
        );
    }
    theme::note(
        ui,
        "neither is written down here: the account comes from Config.wtf and \
         the password is not stored at all",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn how_long_ago_is_said_in_the_largest_unit_that_fits() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let at = |secs: u64| now - Duration::from_secs(secs);
        assert_eq!(ago(now, at(0)), "just now");
        assert_eq!(ago(now, at(59)), "just now");
        assert_eq!(ago(now, at(60)), "1 min ago");
        assert_eq!(ago(now, at(5 * 60 + 30)), "5 min ago");
        assert_eq!(ago(now, at(3 * 3600)), "3 h ago");
        assert_eq!(ago(now, at(86_400)), "1 day ago");
        assert_eq!(ago(now, at(3 * 86_400 + 100)), "3 days ago");
        assert_eq!(
            ago(now, now + Duration::from_secs(5)),
            "just now",
            "a clock in the future is now"
        );
    }
}
