//! The windows the top bar's buttons and menu entries open: go-to, add
//! bookmark, the Playtest popover, the project dialog, the Server panel, the
//! publish popover, the Settings window and the shared session's popover.
//! Each holds text fields.
//!
//! In a shared session `crate::collab::ServerGate` decides what this editor
//! may do in the Server panel and the publish popover. A joined editor, and
//! one whose project is an offline copy of a host's, sees the publish popover
//! disabled with a line saying the host publishes. A joined editor that uses
//! the host's server, or whose own world database is the host's, sees the
//! Server panel's writes disabled: its project is a copy of the host's, and a
//! second apply of the same rows to the same database would run every
//! statement twice.
//!
//! ## Why the popovers are windows and not menus
//!
//! An egui menu (`ui.menu_button`) closes when anything inside it is clicked.
//! That suits a menu, whose items are commands, and does not suit a panel of
//! text fields: a click into the password box would close the box. So each popover
//! is an `egui::Window` with an open flag, opened from its button, or from an
//! entry of one of the bar's two menus, and positioned under that button or
//! menu. A window keeps focus and a `TextEdit` inside one works. Clicking
//! outside closes it because [`Popovers::follow`] closes it, not because the
//! container does.
//!
//! ## Why each popover reports its rectangle to the shell
//!
//! [`crate::ui::Viewport`] is how a tool knows a press was on the interface and not
//! on the world. A docked panel is outside the viewport's rectangle because the
//! root `Ui` shrinks around it. A window floats over the viewport and shrinks
//! nothing, so without its rectangle a press on the password box would read as
//! a press on the ground behind it. Each
//! popover returns its rectangle and the shell adds it to the viewport's
//! floating list for the frame.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::ui::widgets::theme;
use crate::camera::EditorCamera;
use crate::places::Places;
use crate::playtest::Login;

/// Which popovers are open, and where each was opened from.
#[derive(Resource, Default)]
pub(crate) struct Popovers {
    pub(crate) go_to: Popover,
    /// Naming the current view and keeping it. See [`crate::prefs::bookmarks`].
    pub(crate) bookmark: Popover,
    /// Who a playtest logs in as, on which server, and whether its client
    /// keeps the query cache.
    pub(crate) login: Popover,
    /// Which project is open, and making another one.
    pub(crate) project: Popover,
    /// What this project has done to the server.
    pub(crate) server: Popover,
    /// The patch a publish is about to write: its name, and the button.
    pub(crate) publish: Popover,
    /// Hosting or joining a shared session. See [`super::session`].
    pub(crate) session: Popover,
    /// This machine's settings. See [`super::settings`].
    pub(crate) settings: Popover,
    /// Submit, the conflicts and the history. See [`super::changes`].
    pub(crate) changes: Popover,
    /// The name typed into the publish popover. Empty means the UTC stamp.
    pub(crate) patch_name: String,
    /// The Server Tile Files dialog, opened from the server popover.
    pub(crate) tile_files: super::tilefiles::TileFiles,
    /// Whether `--server` has already opened the server popover. Its anchor is
    /// the button's rectangle, which does not exist until the bar has been
    /// drawn once, so the flag cannot be acted on at startup. `--projects`
    /// carries a flag of its own for the same reason. See [`Popover::show`].
    pub(crate) server_shown_once: bool,
    /// The same for `--session` and the Session popover.
    pub(crate) session_shown_once: bool,
    /// The same for `--settings` and the Settings window.
    pub(crate) settings_shown_once: bool,
    /// The same for `--changes` and the Changes window.
    pub(crate) changes_shown_once: bool,
}

/// One popover: whether it is open and where its button is.
pub(crate) struct Popover {
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
    pub(crate) fn toggle(&mut self, button: &egui::Response) {
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
    pub(crate) fn track(&mut self, button: &egui::Response) {
        self.under = button.rect.left_bottom() + egui::vec2(0.0, 4.0);
        self.button = button.rect;
    }

    /// Open it without a press. The anchor is whatever [`Self::track`] last
    /// recorded, so this lands under the button from the frame after.
    pub(crate) fn show(&mut self) {
        self.open = true;
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
            &mut self.bookmark,
            &mut self.session,
            &mut self.settings,
            &mut self.changes,
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

/// Draw every open popover and return their rectangles for
/// [`crate::ui::Viewport`]'s floating list.
///
/// The windows are drawn against the context rather than into the root `Ui`,
/// because they float over the panels and the docked regions must not shrink
/// around them.
pub(crate) fn draw(
    ctx: &egui::Context,
    popovers: &mut Popovers,
    camera: &mut EditorCamera,
    go_to: &mut super::topbar::GoTo,
    places: &Places,
    login: &mut Login,
    projects: &mut super::topbar::Projects,
    session: &mut crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    bookmarks: &mut crate::prefs::bookmarks::Bookmarks,
    server: &mut crate::server::settings::ServerSettings,
    queue: &mut crate::server::queue::ServerQueue,
    standings: &mut crate::server::sync::Standings,
    now: f64,
    // Whether a playtest is running, which disables the two buttons that
    // rebuild the project's archive; and the step a tile regeneration is on.
    playing: bool,
    step: &crate::server::datadir::Step,
    // The shared session, for its popover and for whether this editor is
    // joined to another's.
    collab: &mut crate::collab::Collab,
    // The drawing tablet's settings and what the pen is doing, for the
    // Settings window. See [`crate::tablet`].
    tablet: &mut crate::tablet::Tablet,
    pen: &crate::tablet::Pen,
) -> Vec<egui::Rect> {
    let mut rects = Vec::new();
    let mut seen: Vec<(bool, egui::Rect)> = vec![(false, egui::Rect::NOTHING); 9];

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
    if popovers.bookmark.open {
        if let Some(response) = window(ctx, "add-bookmark", popovers.bookmark.under, |ui| {
            keep_this_view(ui, camera, go_to, bookmarks, session, &mut popovers.bookmark.open);
        }) {
            seen[5] = (true, response);
            rects.push(response);
        }
    }
    if popovers.login.open {
        if let Some(response) = window(ctx, "playtest", popovers.login.under, |ui| {
            who(ui, login, server, assets, collab);
        }) {
            seen[1] = (true, response);
            rects.push(response);
        }
    }
    // The project list is a modal dialog, not a popover. Its contents are a
    // table of choices with consequences: every row is a switch that saves
    // and empties the undo stack. So it sits in the middle with a backdrop,
    // and a press outside it lands on the backdrop and nothing else. For the
    // same reason the whole viewport rectangle goes into the floating list
    // while it is up, and `follow` is told the press was inside.
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
                server,
                collab.role(),
            );
        });
        if response.should_close() {
            popovers.project.open = false;
        }
        // The confirmation's Restore First button set `ask_server`: the
        // dialog closes and the Server popover opens in its place. That
        // popover holds every subject's Restore.
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
        let mut ask_tile_files = false;
        let mut ask_settings = false;
        // In a shared session the Server panel's own buttons say what this
        // editor may write to its server. See `crate::collab::ServerGate`.
        if let Some(response) = window(ctx, "server", popovers.server.under, |ui| {
            the_server(
                ui,
                session,
                assets,
                server,
                queue,
                standings,
                now,
                playing,
                step,
                &mut ask_tile_files,
                &mut ask_settings,
                &collab.server_gate(),
            );
        }) {
            seen[3] = (true, response);
            rects.push(response);
        }
        // The dialog is modal, so the popover closes under it.
        if ask_tile_files {
            popovers.server.open = false;
            popovers.tile_files.show();
        }
        // The Settings window opens in the panel's place, under the
        // Project menu where it is anchored.
        if ask_settings {
            popovers.server.open = false;
            popovers.settings.show();
        }
    }
    if popovers.settings.open {
        if let Some(response) = window(ctx, "settings", popovers.settings.under, |ui| {
            super::settings::panel(ui, assets, server, queue, tablet, pen);
        }) {
            seen[7] = (true, response);
            rects.push(response);
        }
    }
    if let Some(whole) = super::tilefiles::draw(ctx, &mut popovers.tile_files, session, assets, server, queue, standings) {
        rects.push(whole);
    }
    if popovers.publish.open {
        let mut close = false;
        if let Some(response) = window(ctx, "publish", popovers.publish.under, |ui| {
            let refused = collab.server_gate().release_refused();
            hosts_do_this(ui, refused);
            ui.add_enabled_ui(refused.is_none(), |ui| {
                close = the_patch(ui, &mut popovers.patch_name, session, assets, server, queue, step, playing);
            });
        }) {
            seen[4] = (true, response);
            rects.push(response);
        }
        if close {
            popovers.publish.open = false;
        }
    }
    if popovers.session.open {
        let project = session.project.name.clone();
        // The tile under the camera, which the popover checks out.
        let (x, y) = vale_assets::tile_for_position(camera.target.x, camera.target.y);
        let here = vale_edit::TileKey::new(&session.map, x, y);
        if let Some(response) = window(ctx, "session", popovers.session.under, |ui| {
            super::session::panel(ui, collab, &project, &here);
        }) {
            seen[6] = (true, response);
            rects.push(response);
        }
    }
    match popovers.changes.open {
        true => {
            // The Targets tab reads several records on disk, so it is read
            // again every two seconds while the tab is showing.
            if collab.changes.tab == crate::collab::changes::Tab::Targets
                && collab.changes.targets.as_ref().is_none_or(|(at, _)| now - at > 2.0)
            {
                collab.changes.targets = Some((now, crate::server::targets::Overview::read(session, server)));
            }
            let mut asked = super::changes::Asked::Nothing;
            if let Some(response) = window(ctx, "changes", popovers.changes.under, |ui| {
                asked = super::changes::panel(ui, collab, session);
            }) {
                seen[8] = (true, response);
                rects.push(response);
            }
            match asked {
                super::changes::Asked::Nothing => {}
                super::changes::Asked::Save => crate::server::save_everything(session, assets, server, queue, playing),
                super::changes::Asked::Server => {
                    popovers.changes.open = false;
                    popovers.server.show();
                }
                super::changes::Asked::Publish => {
                    popovers.changes.open = false;
                    popovers.publish.show();
                }
            }
        }
        // A joined editor asks the host for its history once each time the
        // window opens.
        false => collab.changes.asked = false,
    }
    popovers.follow(ctx, &seen);
    rects
}

/// The line over a popover this editor may not use: who does it instead.
fn hosts_do_this(ui: &mut egui::Ui, refused: Option<&str>) {
    if let Some(refused) = refused {
        ui.label(egui::RichText::new(refused).size(theme::SMALL).color(theme::WARN));
        ui.add_space(4.0);
    }
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
    server: &mut crate::server::settings::ServerSettings,
    queue: &mut crate::server::queue::ServerQueue,
    step: &crate::server::datadir::Step,
    playing: bool,
) -> bool {
    use crate::server::publish::patch;

    ui.set_width(440.0);
    ui.label(egui::RichText::new("Publish").strong().size(14.0));
    theme::note(
        ui,
        match server.copy_archive {
            false => {
                "Writes one folder under the project's publish\\ with the client archive, the \
                 server files and a README listing where each file goes. Nothing is \
                 applied to this machine's install, server or database."
            }
            true => {
                "Writes one folder under the project's publish\\ with the client archive, the \
                 server files and a README listing where each file goes. The client \
                 archive is also copied into this install's Data folder (the checkbox \
                 below); the server and the database are not changed."
            }
        },
    );
    ui.add_space(8.0);
    let last = patch::last(&session.project);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Name").color(theme::INK_DIM));
        ui.add(
            egui::TextEdit::singleline(name)
                .desired_width(240.0)
                .hint_text(patch::default_name()),
        )
        .on_hover_text("The patch's folder name. Leave empty to use the UTC timestamp.");
    });
    let chosen = match name.trim().is_empty() {
        true => None,
        false => Some(name.trim().to_string()),
    };
    let valid = chosen.as_deref().is_none_or(patch::is_a_name);
    if !valid {
        ui.label(egui::RichText::new("Invalid folder name").small().color(theme::BAD));
    }
    let exists = chosen
        .as_deref()
        .and_then(|n| session.project.path_for(&format!("{}\\{n}", patch::DIR)))
        .is_some_and(|p| p.is_dir());
    if exists {
        theme::note(ui, "A patch with this name exists and will be overwritten.");
    }
    match &last {
        Some(last) => theme::note(
            ui,
            format!("Server tiles unchanged since patch {last} are taken from it rather than rebuilt."),
        ),
        None => theme::note(ui, "No previous patch: every tile in the project is built."),
    }
    // The two switches that change what a publish does, kept per project.
    ui.add_space(6.0);
    theme::heading(ui, "Publish Options");
    if ui
        .checkbox(&mut server.regenerate_tiles, "Regenerate the server's tiles")
        .on_hover_text(
            "Publish runs the four map tools over every tile this project changed since the \
             last publish and writes the results into the server's maps, vmaps and mmaps. \
             A terrain edit costs about a second of extraction plus a navmesh build for the tile \
             and its four neighbours; a moved building also re-extracts the vmaps of the whole \
             map. Turn off for a client-only publish.",
        )
        .changed()
    {
        server.save(&assets.root);
    }
    // Whether the archive the tools read is also left in `Data\`. Off, the
    // tools read a staged copy of the install and nothing on this machine
    // changes; on, the archive goes where a client launched against this
    // install reads it. See `ServerSettings::copy_archive`.
    if ui
        .checkbox(&mut server.copy_archive, "Copy the client archive into Data")
        .on_hover_text(
            "Publish and server tile regeneration also write this project's \
             Patch-<X>.MPQ into this install's Data folder, replacing the copy written \
             before. When off, the archive is built in a staging folder for the map tools \
             and Data is not changed.",
        )
        .changed()
    {
        server.save(&assets.root);
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
                 for every tile changed since the last patch. Tile regeneration takes \
                 minutes; the status line shows the current step. The README and \
                 manifest are written last.",
            )
            .on_disabled_hover_text(match (playing, busy) {
                (true, _) => "Not while a playtest is running.",
                (_, true) => "A server write is still running.",
                _ => "Enter a valid folder name, or leave the field empty to use the UTC timestamp.",
            })
            .clicked()
        {
            let chosen = chosen.clone().unwrap_or_else(patch::default_name);
            session.status = match patch::publish(session, assets, server, queue, step, &chosen) {
                Ok(said) => said,
                Err(e) => format!("publish failed: {e}"),
            };
            bevy::prelude::info!("publish: {}", session.status);
            close = true;
        }
        if let Some(folder) = session.project.path_for(patch::DIR) {
            if !patch::patches(&session.project).is_empty()
                && ui
                    .button("Open Folder")
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
        theme::note(ui, format!("Published patches: {}", had.join(", ")));
    }
    close
}

/// The server popover: what this project has done to the server.
///
/// The panel holds the Apply on save switch, one block per subject with its
/// Apply and Restore ([`super::sync`]), the regeneration of the server's
/// tiles, and the migration. The line at the top says whether there is a
/// database.
///
/// Where the database and the map tools are is set in the Settings window
/// ([`super::settings`]), because those are settings of this machine and not
/// of the project. With no database, the panel says so and has a Settings…
/// button that opens the window.
///
/// Two kinds of control are not here. The publish switches (regenerate the
/// server's tiles, copy the client archive into `Data\`) are in the publish
/// popover, beside the button they change. The query cache switch is in the
/// Playtest popover, since it changes what a playtest's client does and
/// nothing about the server.
#[allow(clippy::too_many_arguments)]
fn the_server(
    ui: &mut egui::Ui,
    session: &mut crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    server: &mut crate::server::settings::ServerSettings,
    // The queue an Apply, a Restore and the Test button go on. See
    // [`crate::server::queue`].
    queue: &mut crate::server::queue::ServerQueue,
    // Where each subject last stood, cached between frames: the panel is drawn
    // sixty times a second and one of its answers reads a whole DBC. See
    // [`crate::server::sync::Standings`].
    standings: &mut crate::server::sync::Standings,
    now: f64,
    playing: bool,
    step: &crate::server::datadir::Step,
    // Set when the Server Tile Files button is pressed; the caller opens the
    // dialog. See [`super::tilefiles`].
    ask_tile_files: &mut bool,
    // Set when the Settings… button is pressed; the caller opens the window.
    ask_settings: &mut bool,
    // Whether this editor may write to its server in a shared session. See
    // `crate::collab::ServerGate`.
    gate: &crate::collab::ServerGate,
) {
    ui.set_width(540.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Server").strong().size(14.0));
        match server.resolve() {
            Some((at, source)) => {
                ui.label(
                    egui::RichText::new(format!("world database {}", at.line()))
                        .small()
                        .color(theme::GOOD),
                )
                .on_hover_text(format!("Connection settings read {}.", source.line()));
            }
            None => {
                ui.label(
                    egui::RichText::new("no world database: saving writes sql\\ files and applies nothing")
                        .small()
                        .color(theme::WARN),
                );
            }
        }
    });
    ui.add_space(6.0);
    if server.resolve().is_none() {
        ui.horizontal(|ui| {
            theme::note(ui, "No world database is set, so nothing can be applied.");
            if ui
                .small_button("Settings\u{2026}")
                .on_hover_text("Set this machine's world database and map tools.")
                .clicked()
            {
                *ask_settings = true;
            }
        });
    }
    this_project(ui, session, assets, server, queue, standings, now, playing, step, ask_tile_files, gate);
}

/// Every server operation of the project, then the server's tiles and the
/// migration.
#[allow(clippy::too_many_arguments)]
fn this_project(
    ui: &mut egui::Ui,
    session: &mut crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    server: &mut crate::server::settings::ServerSettings,
    queue: &mut crate::server::queue::ServerQueue,
    standings: &mut crate::server::sync::Standings,
    now: f64,
    playing: bool,
    step: &crate::server::datadir::Step,
    ask_tile_files: &mut bool,
    gate: &crate::collab::ServerGate,
) {
    // Every server operation of the project. See [`super::sync`], whose
    // module comment says why they are in one panel.
    super::sync::project(
        ui,
        &mut crate::server::sync::Work {
            session,
            assets,
            server,
            queue,
            standings,
            now,
        },
        gate,
    );

    // The server's tiles, which are files and not rows. The tile step on its
    // own, so it can be checked apart from the rest of a publish; the map
    // window has the per-tile form of this button.
    ui.add_space(10.0);
    theme::heading(ui, "Server Tiles");
    // `None` until the first count, which runs on a worker, has finished. The
    // button is enabled while the count is unknown; a press with nothing
    // changed says so on the status line.
    let counts = standings.tile_counts(session, assets, server, now);
    let changed = counts.map(|c| c.changed);
    let busy = queue.busy();
    ui.horizontal(|ui| {
        let count = match changed {
            Some(n) => n.to_string(),
            None => "counting\u{2026}".to_string(),
        };
        if ui
            .add_enabled(
                !playing && !busy && changed != Some(0) && gate.writes(),
                egui::Button::new(format!("Regenerate Changed Tiles ({count})")),
            )
            .on_hover_text(
                "Rebuild this project's archive, then regenerate the server's map files for \
                 every tile changed since the last regeneration. This runs only the tile \
                 step of Publish. Takes minutes; the status line shows the current step. \
                 Restart the server afterwards.",
            )
            .on_disabled_hover_text(match (gate.refused.as_deref(), playing, busy) {
                (Some(refused), _, _) => refused,
                (_, true, _) => "Not while a playtest is running: regeneration rewrites the \
                                 archive the client has open.",
                (_, _, true) => "A server write is still running.",
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
        "Writes maps, vmaps and mmaps under the server's DataDir with the four map tools \
         set in Project \u{2192} Settings\u{2026}. To regenerate selected tiles only, use Server \
         files in Edit Map.",
    );
    ui.horizontal(|ui| {
        if ui
            .add_enabled(gate.writes(), egui::Button::new("Server Tile Files\u{2026}"))
            .on_hover_text(
                "The tile files a regeneration no longer produces, the files each project \
                 wrote, and a check for files nothing uses. Remove and put back from there; \
                 a regeneration removes no file.",
            )
            .on_disabled_hover_text(gate.refused.clone().unwrap_or_default())
            .clicked()
        {
            *ask_tile_files = true;
        }
        if let Some(c) = counts {
            let line = format!(
                "{} no longer produced, {} written by other projects",
                c.stale, c.others
            );
            let colour = match c.stale {
                0 => theme::INK_DIM,
                _ => theme::WARN,
            };
            ui.label(egui::RichText::new(line).small().color(colour));
        }
    });

    // What a publish hands over as rows.
    ui.add_space(10.0);
    theme::heading(ui, "Migration");
    let released = crate::server::publish::release::released(&session.project);
    let last = match &released.id {
        Some(id) => format!(
            "last migration {id}, {} row(s) changed by it",
            released.entries.len()
        ),
        None => "no migration written yet".to_string(),
    };
    ui.horizontal(|ui| {
        if ui
            .add_enabled(gate.release_refused().is_none(), egui::Button::new("Write migration"))
            .on_hover_text(
                "Save every subject's rows, then write publish\\migrations\\<stamp>_world.sql in \
                 vmangos' add_migration format. A migration holds every row the project \
                 changes, and puts back the rows an earlier migration changed that the \
                 project no longer does, so it applies whether or not the server has the \
                 earlier ones. This runs only the row step of Publish. Nothing is written \
                 when nothing changed since the last migration. Apply the file with the mysql \
                 client.",
            )
            .on_disabled_hover_text(gate.release_refused().unwrap_or_default())
            .clicked()
        {
            session.status = crate::server::publish::release::migrate(session, assets);
            standings.forget();
        }
        ui.label(egui::RichText::new(last).small().color(theme::INK_DIM));
    });
}

/// A project's size for its row in the project dialog: `52 files`, or
/// `empty`. The count of each type is the row's hover, and the manifest
/// below the open project lists the files themselves.
fn file_count(summary: &vale_edit::project::Summary) -> String {
    match summary.total() {
        0 => "empty".to_string(),
        1 => "1 file".to_string(),
        n => format!("{n} files"),
    }
}

/// Why the project dialog cannot clear the open project in a shared session.
/// See [`which_project`].
const NO_CLEAR_IN_A_SESSION: &str = "Not in a shared session: the other editors hold copies of \
                                     this project's files and its history. End the session first.";

/// Why a joined editor cannot open another project. See [`which_project`].
const NO_OPEN_WHILE_JOINED: &str = "Not while joined to a session: the open project is the copy \
                                    of the host's. Leave the session first.";

/// The project dialog: which project the edits go into, the other projects
/// there are, and making another.
///
/// A project is a folder under `Edit\` and nothing else, with no manifest and
/// no format, so this lists the folders with what is in each and takes a name
/// for a new one.
///
/// The open project is its own section at the top, because it is what a
/// person opens the dialog to find out. In one flat list it would be a row
/// among rows, distinguished only by the word "open" where the others have a
/// button. Under its name is its manifest (`super::manifest`): every file the folder
/// holds and what each one changes against the archives, read once when the
/// dialog opens. The other projects follow under their own heading, each with
/// a count by type, then the new-project box, so the three things the dialog
/// does are three blocks in that order.
///
/// Switching saves first. The row states this rather than asking: nobody
/// switching projects wants the unsaved work dropped, and a confirmation that
/// is clicked through unread can drop it.
///
/// Clear Files and Delete are gated. `crate::server::held::held_by` reads what
/// the project has applied to the server from its revert files; the
/// confirmation lists it, and the destructive button is enabled only when
/// [`super::topbar::Doomed::may_proceed`] says so, which for a project that
/// holds anything means the project's name has been typed. For the open
/// project a Restore First button sets `ask_server`, and [`draw`] turns that
/// into closing this dialog and opening the Server popover. Discard, in
/// [`super::sync`], is not gated because it leaves the revert files in place.
///
/// In a shared session the open project cannot be cleared. The session
/// numbers its edits from the project's log and the other editors hold copies
/// of its files: a clear would start the log again at edit 0 under a session
/// past it, and leave the other editors' copies as they were. A joined editor
/// cannot open another project, since the host's edits are applied to the
/// open one; a host can, and opening another project ends its session.
#[allow(clippy::too_many_arguments)]
fn which_project(
    ui: &mut egui::Ui,
    projects: &mut super::topbar::Projects,
    session: &mut crate::session::EditSession,
    assets: &vale_client::assets::GameAssets,
    open: &mut bool,
    ask_server: &mut bool,
    server: &crate::server::settings::ServerSettings,
    role: crate::collab::Role,
) {
    use crate::collab::Role;
    let shared = role != Role::Alone;
    let joined = role == Role::Joined;
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
        "The editor shows and playtests the open project. Its files overlay the \
         archives; Publish packs them into the next patch archive.",
    );
    ui.add_space(8.0);

    // ---- the open project.
    theme::heading(ui, "Open Project");
    let summary = projects
        .known
        .iter()
        .find(|(name, _)| *name == here)
        .map(|(_, summary)| summary.clone())
        .unwrap_or_default();
    let tiles = session.unsaved.len();
    let tables = session.unsaved_tables.len();
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(&here)
                .strong()
                .size(15.0)
                .color(theme::INK),
        );
        ui.label(egui::RichText::new(file_count(&summary)).color(theme::INK_DIM))
            .on_hover_text(format!("{}.\n{}", summary.line(), session.project.root.display()));
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
                .add_enabled(!shared, egui::Button::new("Clear Files"))
                .on_hover_text(
                    "Delete every file in this project and keep the empty folder. Unsaved \
                     changes are discarded, not saved first. This cannot be undone.",
                )
                .on_disabled_hover_text(NO_CLEAR_IN_A_SESSION)
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
    theme::heading(ui, "Other Projects");
    let mut switch: Option<String> = None;
    if others.is_empty() {
        theme::note(ui, "No other projects. Enter a name below to create one.");
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
                            ui.label(egui::RichText::new(file_count(summary)).small().color(theme::INK_DIM))
                                .on_hover_text(summary.line());
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
                                    .add_enabled(!joined, egui::Button::new("Open"))
                                    .on_hover_text(match role {
                                        Role::Hosting => format!(
                                            "Save unsaved changes, then switch to {name}. This ends the shared session."
                                        ),
                                        _ => format!("Save unsaved changes, then switch to {name}."),
                                    })
                                    .on_disabled_hover_text(NO_OPEN_WHILE_JOINED)
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
                                        "The default project receives edits when no other project \
                                         is chosen, so it cannot be deleted. Open it and use \
                                         Clear Files instead.",
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
            0 => "The project is empty".to_string(),
            1 => "The project holds 1 file".to_string(),
            n => format!("The project holds {n} files: {}", holds.line()),
        };
        let mut decided = false;
        let response = egui::Modal::new(egui::Id::new("projects-doom")).show(ui.ctx(), |ui| {
            ui.set_width(460.0);
            ui.label(
                egui::RichText::new(match doomed.clear {
                    true => format!("Permanently delete every file in {}?", doomed.name),
                    false => format!("Delete {} and everything in it?", doomed.name),
                })
                .strong()
                .size(14.0)
                .color(theme::INK),
            );
            theme::note(ui, format!("{what}. This cannot be undone."));
            if doomed.clear {
                theme::note(ui, "The project's history (log\\) is deleted with its files. Its published patches (publish\\) are kept.");
            }
            // The tile files stay: the DataDir's ledger, not the project,
            // records them, so they can be put back after the project is gone.
            let written = *doomed.tile_files.get_or_insert_with(|| {
                crate::server::datadir::data_dir(server)
                    .and_then(|dir| vale_mangos::datadir::Ledger::open(&dir))
                    .map(|ledger| ledger.written_by(&doomed.name).len())
                    .unwrap_or(0)
            });
            if written > 0 {
                theme::note(
                    ui,
                    format!(
                        "The project's regenerations wrote {written} tile file(s) in the server's \
                         DataDir. They stay there after the project is gone. To restore the \
                         server's own files, use Put Back Originals in Server\u{2026} > Server \
                         Tile Files."
                    ),
                );
            }
            // The gate: a project that has applied to the server holds the
            // only record of what to put back, and Clear and Delete remove
            // it. See `crate::server::held`.
            if !doomed.held.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(
                        "This project has applied changes to the server, and its folder \
                         holds the only record of how to restore them:",
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
                            "Restore first, from Server\u{2026} on the top bar. To proceed \
                             without restoring, type the project's name: the database \
                             keeps the applied rows and the server keeps the applied files, \
                             with no record left to revert them."
                        }
                        false => {
                            "Open the project and restore first, from Server\u{2026} on \
                             the top bar. To proceed without restoring, type the \
                             project's name: the database keeps the applied rows and the \
                             server keeps the applied files, with no record left to \
                             revert them."
                        }
                    },
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Name").color(theme::INK_DIM));
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
                        .button("Restore First")
                        .on_hover_text(
                            "Close this dialog and open the Server panel. Press Restore on \
                             every subject with applied rows, then return here.",
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
                                true => "Permanently Delete",
                                false => "Delete Project",
                            })
                            .color(egui::Color32::from_rgb(0x0C, 0x16, 0x1E)),
                        )
                        .fill(theme::BAD),
                    )
                    .on_disabled_hover_text(
                        "Type the project's name above to proceed without restoring.",
                    )
                    .clicked()
                {
                    match doomed.clear {
                        // The open project: the session has to forget what it
                        // read out of the files as well.
                        // A project's revert files go with its files, and
                        // with them the rows it held. See
                        // `crate::server::targets`.
                        true if doomed.name == here && shared => {
                            session.status = NO_CLEAR_IN_A_SESSION.to_string();
                        }
                        true if doomed.name == here => {
                            let _ = session.clear_open_project(assets);
                        }
                        true => match vale_edit::project::clear(&assets.root, &doomed.name) {
                            Ok(went) => {
                                session.status = format!("{}: {went} files permanently deleted", doomed.name)
                            }
                            Err(e) => {
                                session.status = format!("could not clear {}: {e}", doomed.name)
                            }
                        },
                        false => match vale_edit::project::delete(&assets.root, &doomed.name) {
                            Ok(went) => {
                                session.status = format!("{} deleted ({went} files)", doomed.name)
                            }
                            Err(e) => {
                                session.status = format!("could not delete {}: {e}", doomed.name)
                            }
                        },
                    }
                    projects.listed = false;
                    decided = true;
                }
                if ui.button("Cancel").clicked() {
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
    theme::heading(ui, "New Project");
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
                .add_enabled(named && !joined, egui::Button::new("Create and Open"))
                .on_disabled_hover_text(match joined {
                    true => NO_OPEN_WHILE_JOINED,
                    false => "A project's name is a folder's name: no separators, no drive \
                              letters, and not empty.",
                })
                .clicked();
        if make && (!named || joined) {
            make = false;
        }
    });
    if make {
        switch = Some(projects.new_name.trim().to_string());
    }

    theme::note(
        ui,
        match tiles + tables {
            0 => "Switching projects clears the undo stack, because each entry refers to \
                  files in the current project's folder."
                .to_string(),
            _ => format!(
                "Switching projects saves unsaved changes first ({tiles} tiles, {tables} tables) \
                 and clears the undo stack, because each entry refers to files in the \
                 current project's folder."
            ),
        },
    );
    ui.add_space(6.0);
    if ui.button("Close").clicked() {
        *open = false;
    }

    if let Some(name) = switch.filter(|_| !joined) {
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
/// hold its contents, so on a short viewport its lower part would be off
/// screen with nothing to say so; a panel cut off at the bottom edge looks
/// complete, and its last subject looks as if it had no Apply.
///
/// The height is the room between the button and the bottom edge: a panel
/// that fits is laid out at its own height with no scrollbar, and a panel
/// that does not is exactly that tall and scrolls. Without the minimum a
/// scroll area inside a window that sizes itself is drawn at a fraction of
/// the room. The two popovers with a list inside keep their own inner caps;
/// those bound a long list, this bounds a short screen.
fn window(
    ctx: &egui::Context,
    id: &str,
    under: egui::Pos2,
    contents: impl FnOnce(&mut egui::Ui),
) -> Option<egui::Rect> {
    let room = (ctx.viewport_rect().bottom() - under.y - 24.0).max(160.0);
    // In front of the areas over the viewport (Select and Measure, a tool's
    // toolbar), which are at the middle order. A popover under the Project
    // menu covers the viewport's top-left corner, where those are drawn.
    egui::Window::new(id)
        .order(egui::Order::Foreground)
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
                .id_salt(id)
                .max_height(room)
                .min_scrolled_height(room)
                .show(ui, contents);
        })
        .map(|response| response.response.rect)
}

/// Put the camera where a bookmark kept it, opening the bookmark's map first
/// when another map is open. The go-to popover's rows and the Map menu's
/// Bookmarks entries both call this.
pub(super) fn go_to_bookmark(
    mark: &crate::prefs::bookmarks::Bookmark,
    camera: &mut EditorCamera,
    session: &mut crate::session::EditSession,
) {
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
}

/// The add-bookmark popover: name the current view and keep it.
///
/// A bookmark is the whole view: the map, the focus with its height, and the
/// orbit. See `crate::prefs::bookmarks`.
fn keep_this_view(
    ui: &mut egui::Ui,
    camera: &EditorCamera,
    go_to: &mut super::topbar::GoTo,
    bookmarks: &mut crate::prefs::bookmarks::Bookmarks,
    session: &crate::session::EditSession,
    open: &mut bool,
) {
    ui.set_min_width(250.0);
    theme::heading(ui, "Add Bookmark");
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut go_to.bookmark)
                .desired_width(160.0)
                .hint_text("bookmark name"),
        );
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let name = go_to.bookmark.trim().to_string();
        if (entered || ui.small_button("Add").clicked()) && !name.is_empty() {
            bookmarks.add(crate::prefs::bookmarks::Bookmark {
                name,
                map: session.map.clone(),
                target: camera.target.to_array(),
                yaw: camera.yaw,
                pitch: camera.pitch,
                distance: camera.distance,
            });
            go_to.bookmark.clear();
            *open = false;
        }
    });
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
    bookmarks: &mut crate::prefs::bookmarks::Bookmarks,
    session: &mut crate::session::EditSession,
    open: &mut bool,
) {
    ui.set_min_width(250.0);
    let here = camera.target;
    let tile = vale_assets::tile_for_position(here.x, here.y);

    // Bookmarks come first because a place somebody named is the place they
    // most often want. They are kept from the Map menu's Bookmarks, Add
    // Bookmark…; see [`keep_this_view`].
    theme::heading(ui, "Bookmarks");
    if bookmarks.list.is_empty() {
        theme::note(ui, "No bookmarks. Map \u{2192} Bookmarks \u{2192} Add Bookmark\u{2026} adds one.");
    }
    let mut jump: Option<crate::prefs::bookmarks::Bookmark> = None;
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
                        true => " — saves the open map, then opens that map",
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
                    .on_hover_text("Delete this bookmark.")
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
        go_to_bookmark(&mark, camera, session);
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
        theme::note(ui, "This map has no WorldMapArea zones.");
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

/// Which realm a joined editor's playtest logs in to, as one note. The
/// switch is "Server" in the Session popover. See
/// `crate::collab::PlaytestServer`.
fn playtest_server(ui: &mut egui::Ui, collab: &crate::collab::Collab) {
    let host = collab.host_name().unwrap_or("the host");
    let realm = match collab.playtest_server {
        crate::collab::PlaytestServer::Local => "the realm in realmlist.wtf".to_string(),
        crate::collab::PlaytestServer::Host => format!("{host}'s realm at {}", collab.host_address().unwrap_or("")),
    };
    theme::note(ui, format!("A playtest logs in to {realm}. \"Server\" in the Session popover sets which server."));
}

/// The Playtest popover: in a shared session a note naming the realm a
/// playtest logs in to, then who it logs in as, whether it shows the login
/// screen, and whether its client caches query answers. Every setting of a
/// playtest is here; the server in a session is a setting of the session.
///
/// The login fields are not settings of the editor's own. The account is
/// `WTF\Config.wtf`'s `accountName`, the character is `VALE_CHARACTER` or
/// `lastCharacterIndex`, and the password is `VALE_PASSWORD`, the one stand-in
/// this project has. Nothing is written back. The popover lets all three be
/// changed for the session without a restart, which an editor needs because
/// it logs in repeatedly. The Disable caching switch is the exception: it is
/// `ServerSettings::disable_caching` and is saved with the other switches.
fn who(
    ui: &mut egui::Ui,
    login: &mut Login,
    server: &mut crate::server::settings::ServerSettings,
    assets: &vale_client::assets::GameAssets,
    collab: &mut crate::collab::Collab,
) {
    ui.set_min_width(300.0);
    ui.label(egui::RichText::new("Playtest").strong().size(14.0));
    ui.add_space(4.0);
    // In a shared session the note names the realm a playtest logs in to,
    // which the Session popover sets. See `crate::collab::PlaytestServer`.
    if matches!(collab.state, crate::collab::State::Joined { .. }) {
        playtest_server(ui, collab);
        ui.add_space(6.0);
    }
    theme::heading(ui, "Login");
    egui::Grid::new("playtest-login-grid")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Account").color(theme::INK_DIM));
            ui.text_edit_singleline(&mut login.account);
            ui.end_row();
            ui.label(egui::RichText::new("Password").color(theme::INK_DIM));
            ui.add(egui::TextEdit::singleline(&mut login.password).password(true));
            ui.end_row();
            ui.label(egui::RichText::new("Character").color(theme::INK_DIM));
            ui.add(egui::TextEdit::singleline(&mut login.character).hint_text("last played"));
            ui.end_row();
        });
    let possible = login.can_go_straight_in();
    ui.add_enabled(
        possible,
        egui::Checkbox::new(&mut login.straight_in, "Skip the login screen"),
    );
    if !possible {
        theme::note(
            ui,
            "Skipping the login screen needs an account and a password. \
             VALE_PASSWORD sets the password at startup.",
        );
    }
    theme::note(
        ui,
        "These fields are not saved. The account is read from Config.wtf and \
         the password is not stored.",
    );

    // The query cache is a switch of the playtest's client, not of the
    // server, so it is here beside the login. It is kept per project with the
    // other switches. See `ServerSettings::disable_caching`.
    ui.add_space(6.0);
    theme::heading(ui, "Query Cache");
    if ui
        .checkbox(&mut server.disable_caching, "Disable caching")
        .on_hover_text(
            "The client writes every query answer to WDB\\ and reads them back at the \
             next login, so an edited creature, item or quest keeps its old name and its \
             old stats. With caching disabled, a playtest starts with an empty cache and \
             queries the server for every creature, item and quest it encounters.",
        )
        .changed()
    {
        server.save(&assets.root);
    }
    theme::note(
        ui,
        "With caching disabled, each creature, item or quest the character encounters \
         costs one query, and an edited row appears in the next playtest.",
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
