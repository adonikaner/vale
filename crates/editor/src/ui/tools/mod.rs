//! The inspector and the pointer tools' own panels and windows. A file here
//! has the same path and stem as the state it draws in `crate::tools`.
//!
//! ```text
//! ground/       the panels of the ground tools: terrain, grade, road, shading,
//!               textures, holes, areas, chunks and water
//! placements/   the panels of the doodad and WMO tools: the placement picker,
//!               a placement's numbers, and a group's
//! spawns/       the creature and game object forms and the waypoint window
//! places/       the panels of the flight path, area trigger and graveyard
//!               tools, and a light's form and bands
//! measure.rs    the Measure tool's panel: a point, a distance, and the slope
//! history.rs    what undo and redo would do, at the top of an editing tool's
//!               panel
//! tablet.rs     the drawing tablet's settings and the pen's state, under the
//!               three brushes a pen's pressure scales
//! ```
//!
//! This file is the inspector: the panel beside the viewport, which shows the
//! chosen tool's settings. [`draw`] chooses the tool's panel; the panels are in
//! the files above.
//!
//! ## One panel for every tool
//!
//! Every subject on the rail draws into the same rectangle, and the tool's
//! row of `crate::tools::SPECS` names the function that fills it. A person
//! learns where the settings are once, and a new subject takes the same
//! position instead of choosing one.
//!
//! A subject that is not built yet (a greyed row on the rail) has nothing to
//! draw, and says so in this one place.
//!
//! ## Axis names
//!
//! The panel labels coordinates `x`, `y` and `z`, meaning the world's x, y and
//! z: north, west and up, the same three the status line reads and `.gps`
//! prints in the game. It does not label them "north", "west" and "up": a
//! compass word names an axis only until something is turned, after which
//! "west" is a direction and `y` is still an axis.
//!
//! The conversion belongs to `vale_assets`, both ways, for positions and for
//! angles: a position through `placement_to_world` and the three angles through
//! `placement_euler_to_world`, which states that the file's second angle is
//! the turn about the world's z. See [`crate::tools::placements::doodads`], where this is
//! written down in full.
//!
//! ## Why the undo history is on this panel and not on the bar
//!
//! Undo applies to the edit, not to the session; [`crate::ui::shell::topbar`] holds the
//! session controls. Undo is also the control a person reaches for without
//! looking, so it sits at the top of the panel they are already looking at.

pub(crate) mod ground;
pub(crate) mod history;
pub(crate) mod measure;
pub(crate) mod placements;
pub(crate) mod places;
pub(crate) mod spawns;
pub(crate) mod tablet;

use bevy_egui::egui;

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::Tool;
use crate::ui::Editing;
use crate::ui::tools::history::history;
use crate::ui::widgets::theme;

/// What the panel is about this frame, as one argument.
///
/// Bundled for the same reason as [`Editing`]: the shell was at Bevy's
/// sixteen-parameter limit and this function took eight. `Editing` is the
/// state the tools hold across frames; this struct is what is true in the
/// current frame.
pub(crate) struct Subject<'a> {
    pub(crate) tool: Tool,
    pub(crate) session: &'a mut EditSession,
    /// The DBCs, for the panel that turns a row id into a name. `None` on a
    /// machine with no archives, where the zone tool shows bare numbers
    /// instead of refusing to open.
    pub(crate) tables: Option<&'a vale_assets::tables::dbc::DisplayTables>,
    /// The archives, for the panel that opens a table on demand: following a
    /// reference reads a DBC nothing has loaded yet. Without this field the
    /// button that follows a reference cannot open that table.
    pub(crate) assets: &'a vale_client::assets::GameAssets,
    /// Where the pointer is. The texture tool uses it to say what the chunk
    /// under the pointer carries; no other panel reads it yet.
    pub(crate) cursor: &'a Cursor,
    /// The frame clock used to fold a gesture into one history entry; see
    /// `vale_edit::undo::History::begin_gesture`.
    pub(crate) now: f64,
    /// Where this machine's server is, for the panels whose subject is a row
    /// in vmangos' database rather than a file.
    pub(crate) server: &'a crate::server::settings::ServerSettings,
    /// The bar's Server… popover, which holds every server operation. The
    /// same panels link to it; see [`crate::ui::shell::sync`].
    pub(crate) server_panel: &'a mut crate::ui::shell::popover::Popover,
    /// Where each server subject stands, held for half a second. The creature
    /// and object panels read their subject's line from it. See
    /// [`crate::server::sync::Standings`].
    pub(crate) standings: &'a mut crate::server::sync::Standings,
    /// The ground guides, for the one a tool's panel repeats: the texture
    /// brush shows the full chunks. See [`crate::tools::guides`].
    pub(crate) guides: &'a mut crate::tools::guides::Guides,
}

/// Draw the panel for whichever tool is chosen: the header, the undo row
/// for a tool that edits, and then the tool's own panel
/// ([`crate::tools::Spec::inspector`]).
pub(crate) fn draw(ui: &mut egui::Ui, subject: Subject<'_>, editing: &mut Editing<'_>) {
    let tool = subject.tool;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(tool.name())
                .strong()
                .size(13.0)
                .color(theme::INK),
        );
        // A workspace in the main window can be moved to a window of its own.
        // See `crate::popout`.
        if tool.covers_viewport() && !editing.popout.holds(tool) {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let pop = ui
                    .add_enabled(editing.popout.editing, egui::Button::new("Pop Out").small())
                    .on_hover_text(
                        "Open this workspace in a window of its own, and return this window \
                         to the world. Dock or close that window to bring it back. Each \
                         workspace can have a window, except that the table workspaces share \
                         one.",
                    )
                    .on_disabled_hover_text("Not during a playtest.");
                if pop.clicked() {
                    editing.popout.asks.push(crate::popout::Ask::Open(tool));
                }
            });
        }
    });
    ui.add_space(2.0);
    if tool.edits() {
        history(ui, subject.session, subject.assets);
    }
    ui.add_space(4.0);
    (tool.spec().inspector)(ui, subject, editing);
}

/// The Select tool's panel, which has no settings.
pub(crate) fn pointer(ui: &mut egui::Ui, _: Subject<'_>, _: &mut Editing<'_>) {
    theme::heading(ui, "Pointer");
    theme::note(
        ui,
        "Select reports the position under the pointer and changes \
         nothing. Choose a tool on the rail to edit, or Measure to inspect \
         a point and the distance between two points.",
    );
}
