//! The Settings window, opened from the Project menu: the settings that
//! belong to this machine rather than to a project.
//!
//! ```text
//! World database   the mangosd.conf the connection is read from, and Test
//! Map tools        the folder of the four vmangos map tools, and the
//!                  server's DataDir that mangosd.conf names
//! Drawing tablet   pen pressure, cursor motion, and what the pen is doing
//! ```
//!
//! The first two are kept in `Edit\server.txt` and the third in
//! `Edit\tablet.txt`, beside the project folders, so every project on this
//! machine reads the same values. A project's own switches are not here:
//! each is beside the action it changes. Apply on save is on the Server panel,
//! Disable caching is in the Playtest popover, and the two publish options are
//! in the publish popover.

use bevy_egui::egui;

use crate::ui::widgets::theme;

/// The window's width, in points.
const WIDTH: f32 = 520.0;

/// The window's contents.
pub(crate) fn panel(
    ui: &mut egui::Ui,
    assets: &vale_client::assets::GameAssets,
    server: &mut crate::server::settings::ServerSettings,
    // The queue the Test button's connection goes on. See
    // [`crate::server::queue`].
    queue: &mut crate::server::queue::ServerQueue,
    tablet: &mut crate::tablet::Tablet,
    pen: &crate::tablet::Pen,
) {
    ui.set_width(WIDTH);
    ui.label(egui::RichText::new("Settings").strong().size(14.0));
    theme::note(
        ui,
        "These settings belong to this machine and are kept in Edit\\server.txt and \
         Edit\\tablet.txt, not in the project.",
    );
    ui.add_space(4.0);
    world_database(ui, assets, server, queue);
    ui.add_space(10.0);
    map_tools(ui, server, assets);
    ui.add_space(10.0);
    drawing_tablet(ui, tablet, pen);
}

/// Where this machine's world database is, and the Test button.
fn world_database(
    ui: &mut egui::Ui,
    assets: &vale_client::assets::GameAssets,
    server: &mut crate::server::settings::ServerSettings,
    queue: &mut crate::server::queue::ServerQueue,
) {
    use crate::server::settings::Source;

    theme::heading(ui, "World database");
    theme::note(
        ui,
        "vmangos reads world data from a database, not from the archives, so a spell edit \
         changes a file for the client and a row for the server.",
    );
    let found = server.resolve();
    let from_env = matches!(found, Some((_, Source::Env(_))));
    setting_row(ui, "mangosd.conf", |ui| {
        let field = ui.add_enabled(
            !from_env,
            egui::TextEdit::singleline(&mut server.conf)
                .desired_width(ui.available_width() - 60.0)
                .hint_text("C:\\MaNGOS"),
        );
        // The setting is written when the box loses focus, not on every
        // keystroke, so the file is not rewritten while the path is typed.
        if field.lost_focus() {
            server.conf = server.conf.trim().to_string();
            server.save(&assets.root);
            server.tried = None;
        }
        if ui
            .add_enabled(found.is_some() && !queue.testing(), egui::Button::new("Test"))
            .on_hover_text("Connect to the world database and count its rows. Changes nothing.")
            .on_disabled_hover_text("No world database is set.")
            .clicked()
        {
            // The test runs on a worker, so a server that does not answer
            // holds the button and not the window. See
            // [`crate::server::queue`].
            if let Some((at, _)) = server.resolve() {
                queue.test(at);
            }
        }
    });
    theme::note(
        ui,
        "The vmangos folder or its mangosd.conf. The connection is read from the \
         WorldDatabase.Info line, so the password stays in mangosd.conf and is not \
         stored by this editor.",
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
                egui::RichText::new("no world database: changes are written to sql\\ and not applied")
                    .small()
                    .color(theme::WARN),
            );
        }
    }
    // The environment overrides the field. The field is disabled and this
    // line says why, so a field that accepts typing never does nothing. The
    // precedence is `ServerSettings::resolve`'s.
    if from_env {
        theme::note(
            ui,
            "VALE_WORLDDB or VALE_MANGOSD is set and overrides this field, so the field \
             is disabled. Unset both to use the field.",
        );
    }
    if let Some(done) = queue.tested() {
        server.tried = Some(done);
    }
    if queue.testing() {
        theme::waiting(ui, "connecting\u{2026}");
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
}

/// The folder of the four vmangos map tools, which build the server's tiles,
/// and the `DataDir` they write into.
fn map_tools(
    ui: &mut egui::Ui,
    server: &mut crate::server::settings::ServerSettings,
    assets: &vale_client::assets::GameAssets,
) {
    theme::heading(ui, "Map tools");
    let tools_from_env = std::env::var_os(vale_mangos::datadir::TOOLS_ENV).is_some();
    setting_row(ui, "vmangos tools", |ui| {
        let field = ui.add_enabled(
            !tools_from_env,
            egui::TextEdit::singleline(&mut server.tools)
                .desired_width(ui.available_width())
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
         MoveMapGenerator from a patched build. Use a Release build: the navmesh \
         generator takes minutes per tile when optimised and is too slow otherwise.",
    );
    if tools_from_env {
        theme::note(
            ui,
            "VALE_VMANGOS_TOOLS is set and overrides this field, so the field is disabled.",
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
                        "; offmesh.txt and config.json not found; the navmesh is built \
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
}

/// The drawing tablet settings, and what the pen is doing. They apply to the
/// Terrain, Textures and Shading brushes. See [`crate::tablet`].
fn drawing_tablet(ui: &mut egui::Ui, tablet: &mut crate::tablet::Tablet, pen: &crate::tablet::Pen) {
    theme::heading(ui, "Drawing tablet");
    let mut changed = false;
    changed |= ui
        .checkbox(&mut tablet.pressure_strength, "Pen pressure scales strength")
        .on_hover_text(
            "While the pen tip is on the tablet, the brush's strength is multiplied by the \
             pressure, 0 to 1. A mouse stroke is not affected.",
        )
        .changed();
    changed |= ui
        .checkbox(&mut tablet.pressure_radius, "Pen pressure scales radius")
        .on_hover_text(
            "While the pen tip is on the tablet, the brush's radius is multiplied by the \
             pressure, 0 to 1. The ring shows the radius in use.",
        )
        .changed();
    changed |= ui
        .checkbox(&mut tablet.pointer_position, "Motion from cursor position")
        .on_hover_text(
            "Right-drag turns the view by the change in the cursor's position instead of \
             raw mouse motion. A tablet reports raw motion as absolute positions, which \
             spin the view. On automatically while a Windows Ink pen is in range; turn it \
             on for a tablet driver with Windows Ink switched off.",
        )
        .changed();
    if changed {
        tablet.save();
    }
    theme::note(
        ui,
        match (pen.hooked, pen.seen, pen.in_range, pen.contact) {
            (false, ..) => "pen input is read on Windows only".to_string(),
            (true, false, ..) => {
                "no Windows Ink pen seen yet; pressure needs Windows Ink on in the tablet driver"
                    .to_string()
            }
            (true, true, _, true) => format!("pen on the tablet, pressure {:.2}", pen.pressure),
            (true, true, true, false) => "pen in range".to_string(),
            (true, true, false, false) => "pen out of range".to_string(),
        },
    );
    theme::note(ui, "pen tip clicks · barrel button held turns the view");
}

/// One setting row: its name in a fixed column wide enough for the longest,
/// then the control.
fn setting_row(ui: &mut egui::Ui, label: &str, contents: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [96.0, ui.spacing().interact_size.y],
            egui::Label::new(egui::RichText::new(label).color(theme::INK_NAME)).selectable(false),
        );
        contents(ui);
    });
}
