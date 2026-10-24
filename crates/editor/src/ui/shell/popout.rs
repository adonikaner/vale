//! The pop-out windows' panels: a workspace in a window of its own.
//!
//! ```text
//! top        the workspace control without World, and Dock
//! right      the inspector, as the main window draws it for the workspace
//! middle     the workspace
//! bottom     the status line's message
//! ```
//!
//! A window is the main window's shell without the world: no rail, no view
//! bar, no viewport. The workspace and its inspector are drawn by the same
//! functions as in the main window ([`crate::ui::workspace`],
//! [`crate::ui::tools::draw`]), so a workspace looks and behaves the same in
//! any window. The windows that follow a creature, a game object or an item
//! are drawn in the window last used of those whose workspace has them
//! (`Popout::subject_window`), and the Import Texture dialog in the window
//! whose workspace asked for it (`Popout::import_window`).
//!
//! A request a panel here makes of the shell (a part of the workspace, a row
//! of another table, a quest) is carried out at the end of this window's
//! pass, by the functions the main window's shell uses
//! (`shell::asks::carry_out_since`). The workspace it chooses opens where its
//! group is, or in this window when no other window has it
//! (`Popout::navigate`). The state and the windows themselves are
//! [`crate::popout`].

use crate::ui::*;
use super::{popover, rail};
use crate::ui::import::assets;
use crate::ui::widgets::theme;
use bevy_egui::EguiContext;
use crate::popout::{Ask, PopoutCamera};

/// Draw the pop-out window in slot `SLOT`, while one is open there. Runs in
/// that slot's egui pass, `crate::popout::PopoutPass(SLOT)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw<const SLOT: usize>(
    mut contexts: Query<(&mut EguiContext, &PopoutCamera)>,
    mut session: Option<ResMut<EditSession>>,
    mut tool: ResMut<Tool>,
    mut editing: Editing,
    cursor: Res<Cursor>,
    time: Res<Time>,
    assets: Res<GameAssets>,
    mut playing: Playing,
    mut popovers: ResMut<popover::Popovers>,
    viewport: Res<Viewport>,
    mut viewing: Viewing,
) {
    let Some(session) = session.as_mut() else { return };
    let Some(mut context) = contexts
        .iter_mut()
        .find_map(|(context, camera)| (camera.0 == SLOT).then_some(context))
    else {
        return;
    };
    let Some(workspace_tool) = editing.popout.tool(SLOT) else { return };
    let ctx = context.get_mut().clone();
    theme::install(&ctx);
    // The main window's zoom, so the windows read at one size.
    if ctx.zoom_factor() != viewport.zoom {
        ctx.set_zoom_factor(viewport.zoom);
    }
    mark_dialog_owner(&ctx, editing.popout.last_focused == Some(SLOT));
    let now = time.elapsed_secs_f64();
    let mark = editing.asks.mark();
    let mut root = egui::Ui::new(
        ctx.clone(),
        "popout".into(),
        UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    egui::Panel::top("popout-top")
        .frame(bar(theme::PANEL))
        .show(&mut root, |ui| {
            ui.horizontal(|ui| {
                // The workspaces, so the window can move between them as the
                // main window's bar does. World is left out: this window has
                // no world.
                let mut chosen = workspace_tool;
                let server = playing.server.resolve().is_some();
                rail::workspaces(ui, &mut chosen, &mut viewing.rail, false, server, &[], false);
                if chosen != workspace_tool {
                    editing.popout.asks.push(Ask::Show(SLOT, chosen));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("Dock")
                        .on_hover_text("Put this workspace back in the main window and close this one.")
                        .clicked()
                    {
                        editing.popout.asks.push(Ask::Dock(SLOT));
                    }
                });
            });
        });

    // What the main window's status line says, since this window has none and
    // a save or an import reports there.
    egui::Panel::bottom("popout-status")
        .frame(bar(theme::SHELL))
        .show(&mut root, |ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(&session.status).small().color(theme::INK_DIM))
                    .truncate(),
            );
        });

    egui::Panel::right("popout-inspector")
        .default_size(theme::INSPECTOR_WIDTH)
        .min_size(theme::INSPECTOR_MIN)
        .max_size(theme::INSPECTOR_MAX)
        .resizable(true)
        .frame(side(theme::PANEL))
        .show(&mut root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                crate::ui::tools::draw(
                    ui,
                    crate::ui::tools::Subject {
                        tool: workspace_tool,
                        session,
                        tables: None,
                        assets: &assets,
                        cursor: &cursor,
                        now,
                        server: &playing.server,
                        server_panel: &mut popovers.server,
                        standings: &mut playing.standings,
                        guides: &mut viewing.guides,
                    },
                    &mut editing,
                );
                ui.allocate_rect(ui.available_rect_before_wrap(), egui::Sense::hover());
            });
        });

    let mut switch_to: Option<Tool> = None;
    let lent = editing.popout.browser_lent(SLOT, *tool);
    egui::CentralPanel::default_margins()
        .frame(egui::Frame::NONE.fill(theme::SHELL))
        .show(&mut root, |ui| {
            // The table browser is the main window's while its tool has a
            // table. See `crate::popout`.
            if lent {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "The table browser is in use by {} in the main window.",
                            tool.name()
                        ))
                        .size(14.0)
                        .color(theme::INK_DIM),
                    );
                    ui.label(
                        egui::RichText::new(
                            "Choose another tool there, or dock this workspace, to edit here.",
                        )
                        .color(theme::INK_FAINT),
                    );
                });
                return;
            }
            workspace(
                ui,
                workspace_tool,
                Bench {
                    session: &mut **session,
                    editing: &mut editing,
                    playing: &mut playing,
                    popovers: &mut popovers,
                    assets: &assets,
                    now,
                    wireframe: None,
                    particles: None,
                    switch_to: &mut switch_to,
                },
            );
        });

    if editing.popout.subject_window(*tool) == Some(SLOT) {
        subject_windows(
            &ctx,
            workspace_tool,
            Bench {
                session: &mut **session,
                editing: &mut editing,
                playing: &mut playing,
                popovers: &mut popovers,
                assets: &assets,
                now,
                wireframe: None,
                particles: None,
                switch_to: &mut switch_to,
            },
        );
    }
    let asked_by_field = editing.assets.import.as_ref().is_some_and(|import| import.for_field.is_some());
    if editing.popout.import_window(*tool, asked_by_field) == Some(SLOT) {
        assets::import_dialog(&ctx, &mut editing.assets, session, &assets);
    }
    // A tool a panel here asked for: a world tool goes to the main window,
    // and a workspace to where its group is, or to this window. See
    // `Popout::navigate`.
    let mut wanted = workspace_tool;
    let in_world = !editing.popout.editing;
    crate::ui::shell::asks::carry_out_since(
        mark,
        &mut editing,
        session,
        &assets,
        &mut wanted,
        &mut viewing.rail,
        in_world,
    );
    if let Some(asked) = switch_to.filter(|asked| !in_world || asked.survives_playtest()) {
        wanted = asked;
        viewing.rail.follow(asked);
    }
    editing.popout.navigate(SLOT, wanted, &mut tool);
    editing.popout.redirect(&mut tool, viewing.rail.last_world);
}
