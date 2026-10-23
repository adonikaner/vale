//! The workspace a tool puts in the middle of a window: [`workspace`] draws
//! the one `tool` names, from the shell's state borrowed as one [`Bench`].
//! The main window and the pop-out window both draw through it. Each
//! workspace's own function is next to its panels.

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::Tool;
use crate::ui::{shell, tools, Editing, Playing};

use vale_client::assets::GameAssets;

/// What a workspace is drawn from, borrowed from the shell's bundles. Both
/// windows draw a workspace with it: the main window's middle and the
/// pop-out window ([`shell::popout`]).
pub(crate) struct Bench<'a, 'e, 'p> {
    pub(super) session: &'a mut EditSession,
    pub(super) editing: &'a mut Editing<'e>,
    pub(super) playing: &'a mut Playing<'p>,
    pub(super) popovers: &'a mut shell::popover::Popovers,
    pub(super) assets: &'a GameAssets,
    pub(super) now: f64,
    /// The view bar's wireframe and particle switches, for the data
    /// workspace's model view; `None` where the window has no view bar.
    pub(super) wireframe: Option<&'a mut bool>,
    pub(super) particles: Option<&'a mut bool>,
    /// A tool a panel asks to switch to, applied by the caller after
    /// everything is drawn.
    pub(super) switch_to: &'a mut Option<Tool>,
}

/// The workspace of `tool`, filling `ui`: the function its row of
/// `crate::tools::SPECS` names ([`crate::tools::Spec::workspace`]), or nothing
/// for a tool that keeps the viewport.
pub(super) fn workspace(ui: &mut egui::Ui, tool: Tool, bench: Bench<'_, '_, '_>) {
    if let Some(draw) = tool.spec().workspace {
        draw(ui, tool, bench);
    }
}

/// The shell's server state the spawn panels read, out of the bench's bundles.
pub(super) fn server<'a>(
    playing: &'a mut Playing<'_>,
    popovers: &'a mut shell::popover::Popovers,
) -> tools::spawns::Server<'a> {
    tools::spawns::Server {
        settings: &playing.server,
        panel: &mut popovers.server,
        standings: &mut playing.standings,
    }
}
