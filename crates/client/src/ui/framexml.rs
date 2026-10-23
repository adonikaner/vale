//! The draw walk: the widget tree [`crate::lua`] loads, turned into the list
//! of items [`crate::ui::mesh`] draws.
//!
//! ```text
//! lua::draw::collect   what is visible, where, in what order   <- no window
//! walk                 when to collect, into Drawn             <- this file
//! ui::mesh             a batch per run of items                <- the GPU
//! ```
//!
//! Everything that can be decided without a window is decided in `lua::draw`,
//! so the assertions about what the interface draws are unit tests in
//! `lua/draw.rs` rather than screenshots. This file decides only when the
//! list is rebuilt, and keeps it in [`Drawn`] between rebuilds.
//!
//! The interface is laid out in the game's virtual units, not in pixels: a
//! screen is `768 / uiScale` units tall and as many across as the window's
//! ratio gives. [`crate::lua::widgets::layout::ui_height`] and
//! [`crate::lua::widgets::layout::units_wide`] define the space, and
//! [`crate::ui::scale::InterfaceViewport`] places it in the window. The walk
//! solves every rectangle against that space; the painter and the pointer
//! convert through the same viewport, so a hit test hits what was drawn.

use bevy::prelude::*;

use crate::lua::host::LuaHost;
use crate::lua::widgets::draw::Item;
#[cfg(feature = "diagnostics")]
use crate::ui::report::{HudReport, Slot};

/// The HUD line this file writes; see [`crate::ui::report`]. Slot 31, directly
/// under the host's `lua:` lines at 30, because the three are read together:
/// what loaded, what it asked for and did not get, and what was drawn.
#[cfg(feature = "diagnostics")]
const REPORT: Slot = Slot(31);

/// Kill switch: `VALE_NO_INTERFACE=1` loads the whole widget tree and draws
/// none of it.
///
/// `render::particles` has the same kind of switch, for the same reason: this
/// pass walks a tree and the painter draws a few hundred quads, and comparing
/// two runs that differ only in this variable measures what that costs. It
/// does not turn the interface off: the tree still loads, the events still
/// fire and the scripts still run. It removes the walk, and with it the paint,
/// and nothing else.
const KILL_SWITCH: &str = "VALE_NO_INTERFACE";

/// The result of the last draw walk, held between walks.
///
/// The walk depends on the interface, not on the frame: where every visible
/// object is, in what order, with what paint. [`walk`] therefore runs it on
/// the frame after each [`crate::lua::api::update::InterfaceClock`] tick (the
/// tick frame runs the `OnUpdate` handlers and the `<Model>` tick, which
/// change most of what it reads), and on a frame where the interface's
/// coordinate space changed. The frames in between keep the stored list.
///
/// The cache is refreshed by the clock, not invalidated by a generation
/// counter. A counter incremented by every setter would have to cover every
/// write of a texture, text, colour, tex-coord, layer, blend, alpha, strata,
/// level, bar value, backdrop and shown state, and a missed one draws a stale
/// picture with no error. The clock has no such list: the walk always re-reads
/// the live tree, so a write between ticks is drawn at most one tick and one
/// frame late.
#[derive(Resource, Default)]
pub(super) struct Drawn {
    /// Read by [`super::mesh`], which draws the list.
    pub(super) items: Vec<Item>,
    /// The interface's coordinate space the items were solved in, as
    /// `(width, height)` in the game's units, or `None` while there is no
    /// interface.
    ///
    /// When it changes, [`walk`] walks on that frame instead of waiting for
    /// the next tick. Every rectangle in the interface is measured from
    /// `UIParent`, so without this the old layout would be drawn scaled into
    /// the new space for up to a tick.
    solved_for: Option<(f32, f32)>,
    /// Whether the first walk with items has been logged.
    reported: bool,
}

pub(crate) struct FrameXmlPlugin;

impl Plugin for FrameXmlPlugin {
    fn build(&self, app: &mut App) {
        // The list is there under the kill switch too, empty, because the
        // painter reads it.
        app.init_resource::<Drawn>();
        if std::env::var(KILL_SWITCH).is_ok() {
            warn!("{KILL_SWITCH} is set — the interface loads and nothing draws it");
            return;
        }
        // A plain `PostUpdate` system. Everything that writes widget state
        // (the mouse and keyboard polls, the event dispatch, the interface
        // tick) runs in `Update`, so the walk sees all of this frame's writes
        // with no edge of its own. The painter orders itself after it; see
        // `ui::mesh::UiMeshPlugin`.
        app.add_systems(PostUpdate, walk);
        #[cfg(feature = "diagnostics")]
        app.add_systems(
            PostUpdate,
            report.after(walk).run_if(crate::ui::report::watched),
        );
    }
}

/// This pass's own HUD line: how many items the last walk produced and how
/// many textures the painter holds for them.
///
/// A system of its own rather than three lines inside [`walk`], so that the
/// walk does not hold a `HudReport`. [`super::debug`] says what the
/// `diagnostics` feature removes and why it removes it completely.
#[cfg(feature = "diagnostics")]
fn report(
    drawn: Res<Drawn>,
    textures: Res<super::mesh::textures::UiTextures>,
    mut hud: ResMut<HudReport>,
) {
    if drawn.solved_for.is_none() {
        hud.clear(REPORT, "framexml");
        return;
    }
    hud.set(
        crate::ui::report::Section::Interface,
        REPORT,
        "framexml",
        format!(
            "interface: {} items drawn, {} textures held",
            drawn.items.len(),
            textures.held()
        ),
    );
}

/// Refresh the draw list in [`Drawn`] when it is due.
///
/// The walk runs on the frame after an interface tick
/// (`InterfaceClock::walk_due`), so that the tick's `OnUpdate` handlers and
/// the walk do not land in the same frame, and on a frame where the
/// interface's coordinate space changed; other frames keep the stored list.
///
/// An event, a click and a keystroke all arrive at frame rate and all write
/// widget state, so a health bar that moved between ticks is drawn after the
/// next one: up to a tick and a frame of display latency, never a lost update,
/// since the walk re-reads the live tree rather than replaying a diff. The
/// whole visible interface refreshes 30 times a second, as in the 1.12.1
/// client.
///
/// A resize by dragging costs almost nothing: the window is held to the
/// space's shape while it can be dragged (see
/// [`crate::lua::widgets::layout::units_wide`]), so dragging re-solves no
/// rectangle and invalidates no memo; only the viewport's scale changes. A
/// mode change alters the space's width, and the `solved_for` comparison then
/// walks once, on the frame the window goes fullscreen. An unload sets
/// `solved_for` to `None`, which forces one walk when the interface comes
/// back.
pub(super) fn walk(
    host: Option<NonSend<LuaHost>>,
    // The list the walk produced, and the clock that says whether to walk again;
    // see [`Drawn`] and [`crate::lua::api::update::InterfaceClock`].
    mut drawn: ResMut<Drawn>,
    clock: Res<crate::lua::api::update::InterfaceClock>,
    time: Res<Time>,
    // The same effect as [`KILL_SWITCH`], switchable at run time. The tree
    // still loads, the events still fire and the scripts still run; the walk
    // and the paint stop. [`crate::render::tuning`] gives the reason for a
    // runtime switch.
    tuning: Res<crate::render::tuning::WorldTuning>,
    // The interface's space; see [`crate::ui::scale`].
    viewport: Res<crate::ui::scale::InterfaceViewport>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Interface);
    let live = host.as_ref().is_some_and(|host| host.interface().is_some());
    let Some(host) = host.filter(|_| tuning.interface && live && viewport.window.is_some())
    else {
        // The list is cleared, not kept. The switch and the logout both reach
        // this branch, and a kept list would be drawn again as soon as either
        // came back: the old world's action bar over the new one's, for one
        // tick. This is the one place the cache can show a stale picture.
        if drawn.solved_for.is_some() || !drawn.items.is_empty() {
            drawn.items.clear();
            drawn.solved_for = None;
        }
        return;
    };
    let solved_for = viewport.space;
    if clock.walk_due() || drawn.solved_for != Some(solved_for) {
        // The time is on `GetTime()`'s base, so that a line the interface
        // stamped from Lua and the expiry this pass applies use one clock.
        //
        // `None` means nothing the walk reads has changed since the last walk
        // (see `LuaHost::drawn_if_changed`), and `Drawn` is left untouched so
        // that the painter sees no change either.
        let now = crate::interface::api::get_time(&time);
        if let Some(items) = host.drawn_if_changed(solved_for, now, &drawn.items) {
            drawn.items = items;
        }
        if drawn.solved_for != Some(solved_for) {
            drawn.solved_for = Some(solved_for);
        }
    }
    // Logged once, when the walk first has items. The HUD's copy scrolls off
    // a small window; the log is kept for every run.
    if !drawn.reported && !drawn.items.is_empty() {
        drawn.reported = true;
        info!("interface: first walk — {} items", drawn.items.len());
    }
}
