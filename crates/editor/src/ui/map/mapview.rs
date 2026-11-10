//! The map from above: which tiles exist, the selection the tile operations
//! act on, and who holds a claim on which tile.
//!
//! ## Why tiles are chosen on a map
//!
//! A tile operation cannot take the tile under the pointer, because that
//! cannot create a tile. `crate::pick::Cursor`'s `ground` is `None` "off the
//! map, over a hole, or over a tile that is not open", so over a tile that
//! does not exist there is no pointer position and no tile for Create to name.
//!
//! Choosing in the viewport would also mean flying to where no tile exists,
//! in a viewport that draws nothing there: no landmark, no ground, and no way
//! to tell 33,48 from 34,48 except the status line.
//!
//! A map has 64x64 tiles, and choosing among them needs them laid out. This
//! window is a grid of 4,096 cells, each showing that tile's minimap picture
//! from the archives: a top-down render of the world, and the only existing
//! picture of a tile. Tiles that do not exist are drawn as holes.
//!
//! ## All tile operations are in this window
//!
//! Creating ground, deleting it, copying it, the three files derived from a
//! tile, and checking tiles out are all here, beside the selection they act
//! on. None of them uses the pointer, so none is a row on the rail, which
//! lists what the pointer edits. A panel elsewhere would leave the map
//! choosing the tiles while another place acted on them. The window is opened
//! from the top bar, because which tiles this map has is a fact about the
//! session, and the top bar holds session facts.
//!
//! ## Making a new map
//!
//! New map… in the top bar's Map menu opens a form for a map that does not
//! exist yet: its folder, name, type and loading screen. Making it writes a
//! `Map.dbc` row, a WDT with no tiles and the server's `map_template` row, and
//! opens the map, so the next step is Create on an empty grid. The form is
//! drawn here, as a modal, whether or not this window is open. See
//! [`crate::map::maps`].
//!
//! ## The layout
//!
//! A toolbar across the top holds the operations that change which tiles the
//! map has (Create, Delete, Copy, Paste) and the check outs. The map fills the
//! rest of the window left of a side panel, with a status line under it that
//! names the tile under the pointer and holds the zoom. The side panel holds
//! the selection, the claims, new ground's settings, and what acts on the
//! selection's derived files: shadows, minimaps, images and the server's
//! files. The panels are laid out before the map, so the map takes the room
//! they leave, and they are always shown, so selecting a tile does not resize
//! the map.
//!
//! ## Every operation works on the selection
//!
//! Click, box-drag, or ctrl-click to toggle. Every operation then applies to
//! the whole selection, so creating nine tiles or deleting a row is one action
//! rather than nine. A right click opens the operations as a menu, on the
//! selection, or on the tile under the pointer when that tile is not
//! selected. The operations themselves are in [`crate::map::tiles`].
//!
//! ## Zoom
//!
//! The wheel zooms around the pointer, and dragging with the right or middle
//! button moves the map. The status line's slider sets the same zoom, from
//! [`MIN_CELL`] to [`MAX_CELL`] points a tile. Fit frames the map's tiles and
//! the selection with one empty ring around them; Whole Map shows all 64x64
//! slots, for making ground where the map has none. The window opens fitted,
//! and fits again when another map is opened. Azeroth claims about 700 of the
//! 4,096 slots in a region twenty tiles wide, so a window that opened on the
//! whole grid would show it as a small patch.
//!
//! Each cell's edges are rounded to whole pixels, so at every zoom the cells
//! meet without gaps and do not shimmer as the map moves. The rulers along the
//! top and left stay in place while the map moves and name the columns and
//! rows.
//!
//! ## What a cell shows
//!
//! Its picture, when the archives have one; a plain fill for a tile that
//! exists without a picture, which is a tile this editor made; a sunken square
//! for an empty slot. Over that, a corner mark on a tile this project carries
//! (edited and saved, or made), in the warning colour while it has unsaved
//! changes, so the map shows where the work is.
//!
//! A claimed tile is tinted in its holder's colour, the colour the session
//! popover gives that person, and each holder's tiles are outlined as one
//! region: a solid line for a check out, a dashed line for a claim held only
//! by unsubmitted edits. Another author's claimed tile also carries a padlock
//! once a cell is [`LOCK_FROM`] points, because this editor's edits there are
//! refused. The selection is tinted and outlined as one region in the accent
//! colour, and the camera's tile is outlined in green.
//!
//! ## Claims
//!
//! The side panel lists each author with a claim on this map, with how many
//! tiles they have checked out and how many their unsubmitted edits hold; a
//! press on a row selects that author's tiles. Check Out asks the host for the
//! selected tiles that exist and that no other author holds. Release ends this
//! author's check outs in the selection, and Break ends other authors'. All
//! three need a shared session, because the host keeps the claims; alone, the
//! list shows the claims that unsubmitted edits in the log hold. See
//! `vale_collab::checkout`.
//!
//! ## The pictures use the texture picker's thumbnail cache
//!
//! A minimap tile is a BLP at a path, and [`crate::ui::widgets::thumbnails`]
//! already decodes BLPs by path, a few per frame, with a cache. The grid fills
//! in over about three seconds on a map with seven hundred tiles and costs
//! nothing on later frames, with the same behaviour and code as the tileset
//! list. From [`SHARP_FROM`] pixels a cell, the picture is asked for at
//! [`SHARP_SIDE`] pixels as well and drawn once it is decoded, because the
//! 32-pixel thumbnail blurs past that size.

use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::collab::{Ask, MapClaims, TileClaim};
use crate::session::EditSession;
use crate::ui::widgets::theme;
use crate::ui::widgets::thumbnails::Thumbnails;

/// Tiles across a map, in each axis.
const SIDE: u32 = 64;

/// The smallest and largest cell the zoom allows, in points a tile. At the
/// smallest the whole grid is 256 points across; at the largest one tile's
/// picture is drawn at [`SHARP_SIDE`] pixels.
const MIN_CELL: f32 = 4.0;
const MAX_CELL: f32 = 128.0;

/// The largest cell Fit chooses, in points, so a map of a few tiles does not
/// open with one tile filling the window.
const FIT_MAX: f32 = 64.0;

/// The factor the status line's - and + buttons zoom by.
const STEP: f32 = 1.25;

/// How fast the wheel zooms: the cell is multiplied by e to the power of
/// this times the scroll, in points. One wheel notch is about 50 points of
/// scroll, a factor of about 1.2.
const WHEEL: f32 = 0.004;

/// The rulers' width, top and left, in points.
const RULER: f32 = 16.0;

/// The side panel's width, in points.
const PANEL_WIDTH: f32 = 250.0;

/// From this many pixels a cell, a tile's picture is also asked for at
/// [`SHARP_SIDE`] pixels.
const SHARP_FROM: f32 = 48.0;
const SHARP_SIDE: u32 = 128;

/// From this many points a cell, each cell has its edge drawn. Below it the
/// edges cover most of the pictures.
const EDGE_FROM: f32 = 14.0;

/// From this many points a cell, another author's claimed tile carries a
/// padlock.
const LOCK_FROM: f32 = 20.0;

/// How many frames after it opens the window is moved to the top of its
/// layer order.
///
/// The Select and Measure buttons over the viewport and the tool bars beside
/// them are areas of the same order as windows, and egui puts an area that
/// appears for the first time above the others. Opened before the viewport
/// has a size, as `--tiles` opens it, the window was then drawn under those
/// buttons, which covered Create and Delete. Three frames covers the frame
/// the viewport gets its size; after that, a press decides which is on top,
/// as for every other window.
const RAISE_FRAMES: u8 = 3;

/// What a fit frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fit {
    /// The map's tiles and the selection, with one empty ring.
    Tiles,
    /// All 64x64 slots.
    Whole,
}

/// What the map window is holding.
#[derive(Resource, Default)]
pub(crate) struct MapView {
    pub(crate) open: bool,
    /// The tiles every operation applies to.
    pub(crate) selection: HashSet<(u32, u32)>,
    /// Where a box-drag began, while one is in progress.
    drag_from: Option<(u32, u32)>,
    /// The tiles the last Copy took, in the coordinates they were taken
    /// from.
    ///
    /// A set rather than one tile, so a region can be moved as a region. What
    /// Paste does with it depends on how many tiles it holds:
    ///
    /// * one tile fills every selected slot, which lays the same ground over,
    ///   for example, a nine-tile square;
    /// * several tiles are pasted as a block, keeping their relative layout,
    ///   with the copied region's corner placed on the selection's corner.
    ///   Any other rule would need a second gesture to say which copied tile
    ///   goes where.
    pub(crate) clipboard: Vec<(u32, u32)>,
    /// Points a tile. Zero until the first frame fits the map.
    cell: f32,
    /// The tile coordinate at the map area's top-left corner, fractional.
    origin: egui::Vec2,
    /// The map the view was last fitted to. When another map is open, the
    /// next frame fits again.
    fitted_for: String,
    /// A fit the status line's buttons asked for, made on the next frame.
    fit: Option<Fit>,
    /// The map area on the last frame, without the rulers. The zoom controls
    /// are drawn before the map and zoom about its centre.
    area: Option<egui::Rect>,
    /// The tile under the pointer on the last frame, for the status line.
    hovered: Option<(u32, u32)>,
    /// The tile a right click opened the menu on.
    menu_at: Option<(u32, u32)>,
    /// Select the camera's tile on the first frame the window is drawn.
    ///
    /// Set by [`MapView::opened`], which `--tiles` uses, so a `--shot` of the
    /// window shows the side panel's buttons enabled. The camera's tile is
    /// also the most likely one to be acted on.
    preselect: bool,
    /// The tiles this project carries, read from the project folder. Reading
    /// walks the folder, so the result is kept and re-read only when
    /// [`Self::edited_stale`] is set.
    edited: HashSet<(u32, u32)>,
    /// Set by whatever changes the project's tiles: the window opening, and
    /// every operation that makes, deletes or pastes one.
    pub(crate) edited_stale: bool,
    /// Whether the window was open on the last frame, for the re-read above.
    was_open: bool,
    /// Frames left in which the window is moved above the other areas of its
    /// order. See [`RAISE_FRAMES`].
    raise: u8,
    /// The claims on the open map's tiles. Set on every frame the window is
    /// drawn. See [`MapClaims`].
    claims: MapClaims,
    /// A check out, release or break the window asked for. The shell takes it
    /// and hands it to `crate::collab`.
    pub(crate) claim_ask: Option<Ask>,
}

impl MapView {
    /// A map view that starts open, for `--tiles`, which is how the window is
    /// captured in screenshots.
    pub(crate) fn opened() -> MapView {
        MapView {
            open: true,
            preselect: true,
            ..MapView::default()
        }
    }

    /// Select exactly these.
    pub(crate) fn select_only(&mut self, at: (u32, u32)) {
        self.selection.clear();
        self.selection.insert(at);
    }

    /// How many of the selection exist, and how many do not.
    pub(crate) fn split(&self, session: &EditSession) -> (usize, usize) {
        let exist = self
            .selection
            .iter()
            .filter(|&&at| session.wdt_claims(at))
            .count();
        (exist, self.selection.len() - exist)
    }

    /// Whether this project carries the tile, saved or not.
    fn in_project(&self, session: &EditSession, at: (u32, u32)) -> bool {
        self.edited.contains(&at) || session.unsaved.contains(&at)
    }

    fn reread_edited(&mut self, session: &EditSession) {
        self.edited.clear();
        for vpath in session.project.files() {
            if let Some(key) = vale_edit::TileKey::from_vpath(&vpath) {
                if key.map.eq_ignore_ascii_case(&session.map) {
                    self.edited.insert((key.x, key.y));
                }
            }
        }
        self.edited_stale = false;
    }

    /// The selected tiles, sorted, that `keep` keeps.
    fn selected_where(&self, keep: impl Fn((u32, u32), Option<&TileClaim>) -> bool) -> Vec<(u32, u32)> {
        let mut tiles: Vec<(u32, u32)> = self
            .selection
            .iter()
            .copied()
            .filter(|&at| keep(at, self.claims.tiles.get(&at)))
            .collect();
        tiles.sort_unstable();
        tiles
    }

    /// The selected tiles a Check Out takes: tiles that exist, that no other
    /// author holds, and that this editor's author has not checked out.
    fn checkable(&self, session: &EditSession) -> Vec<(u32, u32)> {
        self.selected_where(|at, claim| {
            session.wdt_claims(at) && claim.is_none_or(|claim| claim.mine && !claim.checked_out)
        })
    }

    /// The selected tiles this editor's author has checked out.
    fn releasable(&self) -> Vec<(u32, u32)> {
        self.selected_where(|_, claim| claim.is_some_and(|claim| claim.mine && claim.checked_out))
    }

    /// The selected tiles another author has checked out.
    fn breakable(&self) -> Vec<(u32, u32)> {
        self.selected_where(|_, claim| claim.is_some_and(|claim| !claim.mine && claim.checked_out))
    }

    /// The claims of these tiles, for a check out, a release or a break.
    fn claims_of(&self, tiles: &[(u32, u32)]) -> Vec<vale_collab::Claim> {
        tiles.iter().map(|&at| self.claims.claim(at)).collect()
    }

    /// Zoom to `cell` points a tile, keeping the point `about` of `area` over
    /// the same place on the map.
    fn zoom_about(&mut self, cell: f32, about: egui::Pos2, area: egui::Rect) {
        let cell = cell.clamp(MIN_CELL, MAX_CELL);
        let local = about - area.min;
        let under = self.origin + local / self.cell;
        self.cell = cell;
        self.origin = under - local / cell;
    }

    /// Keep at least two tiles of the grid in view in each axis, so the map
    /// cannot be moved out of the window.
    fn keep_in_view(&mut self, room: egui::Vec2) {
        let seen = room / self.cell;
        self.origin.x = self.origin.x.clamp(2.0 - seen.x, SIDE as f32 - 2.0);
        self.origin.y = self.origin.y.clamp(2.0 - seen.y, SIDE as f32 - 2.0);
    }
}

/// What the window asked for, handed back to the caller to act on.
///
/// The window performs nothing itself: it holds the session by shared
/// reference, and every operation needs it mutably and also needs the
/// archives. `crate::map::tiles` uses the same arrangement one layer down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asked {
    Create,
    Delete,
    Copy,
    Paste,
    FlyTo((u32, u32)),
    /// Rebake the selection's `MCSH` from what stands on it.
    Rebake,
    /// Redraw the selection's minimap pictures.
    Minimap,
    /// …and every tile of the map's.
    MinimapsAll,
    /// Regenerate the server's files for the selection: `maps`, `vmaps`,
    /// `mmaps`.
    ServerFiles,
    /// Ask where to save the selection's height map, and write it. See
    /// `crate::import::images`.
    ExportHeights,
    /// …or its blend map.
    ExportBlends,
    /// Ask for a picture to import, and show what importing it would do.
    ChooseImport,
    /// Import the picture chosen, as the dialog was left.
    Import,
    /// Make the map the form in `Tiles::new_map` describes. See
    /// `crate::map::maps`.
    NewMap,
    /// Save the form's edits to the map it edits. See
    /// `crate::map::maps::save`.
    SaveMap,
}

/// The tiles a fit frames, first and last inclusive: the claimed tiles and
/// the selection with one empty ring around them, or the whole grid on a map
/// with no tiles and nothing selected.
fn fit_bounds(session: &EditSession, selection: &HashSet<(u32, u32)>) -> ((u32, u32), (u32, u32)) {
    let mut lo = (SIDE, SIDE);
    let mut hi = (0u32, 0u32);
    for &(x, y) in session.claimed.iter().chain(selection.iter()) {
        lo = (lo.0.min(x), lo.1.min(y));
        hi = (hi.0.max(x), hi.1.max(y));
    }
    if lo.0 > hi.0 {
        return ((0, 0), (SIDE - 1, SIDE - 1));
    }
    (
        (lo.0.saturating_sub(1), lo.1.saturating_sub(1)),
        ((hi.0 + 1).min(SIDE - 1), (hi.1 + 1).min(SIDE - 1)),
    )
}

/// The cell size and origin that show tiles `lo` to `hi` (inclusive) centred
/// in `room`, at the largest whole number of points a tile that fits, between
/// [`MIN_CELL`] and `max`.
fn framing(lo: (u32, u32), hi: (u32, u32), room: egui::Vec2, max: f32) -> (f32, egui::Vec2) {
    let size = egui::vec2((hi.0 + 1 - lo.0) as f32, (hi.1 + 1 - lo.1) as f32);
    let cell = (room.x / size.x).min(room.y / size.y).floor().clamp(MIN_CELL, max);
    let centre = egui::vec2(lo.0 as f32, lo.1 as f32) + size * 0.5;
    (cell, centre - room / (2.0 * cell))
}

/// Which sides of the tile at `at` lie on its region's boundary, in the
/// order up, right, down, left. `same` answers whether the tile at a
/// coordinate, which may be off the grid, is in the region.
fn boundary(at: (u32, u32), same: impl Fn(i64, i64) -> bool) -> [bool; 4] {
    let (x, y) = (at.0 as i64, at.1 as i64);
    [!same(x, y - 1), !same(x + 1, y), !same(x, y + 1), !same(x - 1, y)]
}

/// Draw the window. Returns its rectangle, so the shell can keep clicks in it
/// out of the world, and whatever was asked for.
///
/// `gate` says what this editor may do in a shared session. When it is joined
/// to another editor's session, the buttons that make, delete or paste tiles,
/// draw minimaps and rebake shadows are disabled, because only the host does
/// those. Server Files is disabled when `gate` says this editor may not write
/// to its server. See `crate::collab::ServerGate`. A check out, release or
/// break is left in [`MapView::claim_ask`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    ctx: &egui::Context,
    view: &mut MapView,
    session: &EditSession,
    thumbnails: &mut Thumbnails,
    minimaps: &vale_assets::tables::minimap::MinimapTiles,
    tiles: &mut crate::map::tiles::Tiles,
    camera_tile: (u32, u32),
    camera_at: [f32; 2],
    screens: &vale_assets::tables::loading::LoadingScreens,
    gate: &crate::collab::ServerGate,
    claims: MapClaims,
) -> (Option<egui::Rect>, Option<Asked>) {
    view.claims = claims;
    // The New map form is opened from the top bar's Map menu and is a modal of
    // its own, so it is drawn whether or not this window is open.
    let made = new_map_dialog(ctx, session, tiles, thumbnails, screens, camera_at);
    if !view.open {
        view.was_open = false;
        return (None, made);
    }
    if !view.was_open {
        view.was_open = true;
        view.edited_stale = true;
        view.raise = RAISE_FRAMES;
    }
    if view.raise > 0 {
        view.raise -= 1;
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new("map-window")));
    }
    if view.edited_stale {
        view.reread_edited(session);
    }
    if view.preselect {
        view.preselect = false;
        view.select_only(camera_tile);
    }
    let mut asked = None;
    let mut open = view.open;

    let response = theme::window(format!("Map \u{2014} {}", session.map), "map-window")
        .open(&mut open)
        // Within a 720-point screen under the bar: a window taller than the
        // screen is moved up over the bar to fit, and then covers the button
        // that opened it.
        .default_size([900.0, 640.0])
        .min_size([560.0, 360.0])
        // Below the top bar, which holds the button that opens the window. A
        // window covering that button hides the control that closes it.
        .default_pos([40.0, 52.0])
        .show(ctx, |ui| {
            asked = contents(ui, view, session, thumbnails, minimaps, tiles, camera_tile, gate);
        });
    view.open = open;

    (response.map(|r| r.response.rect), asked.or(made))
}

/// The hover text of a button that is disabled for a joined editor, or
/// `otherwise` when it is disabled for another reason.
fn host_only(joined: bool, otherwise: &'static str) -> &'static str {
    match joined {
        true => "In a shared session only the host changes tiles and their files.",
        false => otherwise,
    }
}

#[allow(clippy::too_many_arguments)]
fn contents(
    ui: &mut egui::Ui,
    view: &mut MapView,
    session: &EditSession,
    thumbnails: &mut Thumbnails,
    minimaps: &vale_assets::tables::minimap::MinimapTiles,
    tiles: &mut crate::map::tiles::Tiles,
    camera_tile: (u32, u32),
    gate: &crate::collab::ServerGate,
) -> Option<Asked> {
    let mut asked = None;

    // The toolbar, the side panel and the status line are laid out before
    // the map, which then takes the room they leave. The map is sized to the
    // room there is, so it cannot grow the window.
    ui.horizontal(|ui| {
        asked = tile_buttons(ui, view, session, gate.role.joined());
        ui.separator();
        claim_buttons(ui, view, session, false);
        match view.clipboard.len() {
            0 => {}
            1 => {
                let at = view.clipboard[0];
                theme::note(ui, format!("copied {}, {}", at.0, at.1));
            }
            n => theme::note(ui, format!("copied {n} tiles")),
        }
    });
    ui.add_space(4.0);

    egui::Panel::right("map-side")
        .resizable(false)
        .exact_size(PANEL_WIDTH)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 10, ..egui::Margin::ZERO }))
        .show(ui, |ui| {
            // The panel's width is fixed, but a row wider than the panel
            // widens the scroll area, and egui then keeps the panel's width
            // by moving its left edge into the map, which is drawn over the
            // panel's first letters. Capping the scroll area clips a wide row
            // instead, and the rows below are laid out not to be wide.
            egui::ScrollArea::vertical()
                .id_salt("map-side-scroll")
                .auto_shrink([false, false])
                .max_width(ui.available_width())
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width());
                    if let Some(more) = side_panel(ui, view, session, tiles, gate) {
                        asked = Some(more);
                    }
                });
        });

    egui::Panel::bottom("map-status")
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin { top: 4, ..egui::Margin::ZERO }))
        .show(ui, |ui| status_line(ui, view, session));

    if let Some(more) = map_area(ui, view, session, thumbnails, minimaps, camera_tile, gate) {
        asked = Some(more);
    }
    asked
}

/// Create, Delete, Copy and Paste, each counting what it acts on: in the
/// toolbar, and in the map's right-click menu.
fn tile_buttons(ui: &mut egui::Ui, view: &MapView, session: &EditSession, joined: bool) -> Option<Asked> {
    let mut asked = None;
    // Create is enabled by empty slots in the selection and Delete by
    // existing tiles, so each button is enabled only when it has something to
    // act on, and neither silently does nothing.
    let (exist, absent) = view.split(session);
    if ui
        .add_enabled(absent > 0 && !joined, egui::Button::new(format!("Create {absent}")))
        .on_hover_text("Claims each selected empty tile in the WDT and writes flat ground, as New Ground in the side panel sets it.")
        .on_disabled_hover_text(host_only(joined, "Select empty tiles."))
        .clicked()
    {
        asked = Some(Asked::Create);
    }
    if ui
        .add_enabled(exist > 0 && !joined, egui::Button::new(format!("Delete {exist}")))
        .on_hover_text(
            "Clears each selected tile's WDT bit. A tile of the archives keeps its ADT in the \
             project. A tile the project created, which the archives do not have, is deleted \
             from the project with its ADT and minimap picture; this cannot be undone.",
        )
        .on_disabled_hover_text(host_only(joined, "Select tiles that exist."))
        .clicked()
    {
        asked = Some(Asked::Delete);
    }
    if ui
        .add_enabled(exist > 0, egui::Button::new(format!("Copy {exist}")))
        .on_hover_text(
            "Copies every selected tile that exists. One copied tile pastes into every \
             selected slot; several paste as a block, keeping their layout.",
        )
        .on_disabled_hover_text("Select tiles that exist.")
        .clicked()
    {
        asked = Some(Asked::Copy);
    }
    if ui
        .add_enabled(
            !view.clipboard.is_empty() && !view.selection.is_empty() && !joined,
            egui::Button::new("Paste"),
        )
        .on_hover_text(
            "Writes each copied tile into the selected slot, rewriting its positions \
             for the new place. Its terrain, doodads and buildings move with it.",
        )
        .on_disabled_hover_text(host_only(joined, "Copy tiles, then select where they go."))
        .clicked()
    {
        asked = Some(Asked::Paste);
    }
    asked
}

/// The hover text of a check out button with no shared session.
const NO_SESSION: &str = "Check outs are kept by a shared session's host. Host or join a session from \
                          the Session button.";

/// Check Out and Release, each counting the selected tiles it acts on, and
/// with `breaks`, Break when another author has one of them checked out.
/// What is pressed is left in [`MapView::claim_ask`].
fn claim_buttons(ui: &mut egui::Ui, view: &mut MapView, session: &EditSession, breaks: bool) {
    let can = view.claims.can_check_out;
    let checkable = view.checkable(session);
    if ui
        .add_enabled(can && !checkable.is_empty(), egui::Button::new(format!("Check Out {}", checkable.len())))
        .on_hover_text(
            "Claims the selected tiles before any edit, so no other editor edits them until they \
             are released or this editor submits. Check out tiles before working on them offline.",
        )
        .on_disabled_hover_text(match can {
            true => "Select tiles that exist and that no other author holds.",
            false => NO_SESSION,
        })
        .clicked()
    {
        view.claim_ask = Some(Ask::CheckOut(view.claims_of(&checkable)));
    }
    let releasable = view.releasable();
    if ui
        .add_enabled(can && !releasable.is_empty(), egui::Button::new(format!("Release {}", releasable.len())))
        .on_hover_text(
            "Ends this editor's check outs of the selected tiles. A tile with unsubmitted edits \
             stays claimed until they are submitted.",
        )
        .on_disabled_hover_text(match can {
            true => "None of the selected tiles is checked out by this editor.",
            false => NO_SESSION,
        })
        .clicked()
    {
        view.claim_ask = Some(Ask::Release(view.claims_of(&releasable)));
    }
    let breakable = view.breakable();
    if breaks && can && !breakable.is_empty() {
        let who: Vec<String> = {
            let mut who: Vec<String> = breakable
                .iter()
                .filter_map(|at| view.claims.tiles.get(at).map(|claim| claim.who.clone()))
                .collect();
            who.sort();
            who.dedup();
            who
        };
        if ui
            .button(format!("Break {}", breakable.len()))
            .on_hover_text(format!(
                "Ends {}'s check outs of the selected tiles. Edits made there offline may then \
                 conflict with others.",
                who.join(", ")
            ))
            .clicked()
        {
            view.claim_ask = Some(Ask::Break(view.claims_of(&breakable)));
        }
    }
}

/// Under the map: the tile under the pointer, or the selection when the
/// pointer is elsewhere, and the zoom.
fn status_line(ui: &mut egui::Ui, view: &mut MapView, session: &EditSession) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Right to left, so the controls read Fit, Whole Map, -, the
            // slider, + from the left.
            let mut cell = view.cell;
            if ui.small_button("+").on_hover_text("Zoom in.").clicked() {
                cell *= STEP;
            }
            ui.add(
                egui::Slider::new(&mut cell, MIN_CELL..=MAX_CELL)
                    .logarithmic(true)
                    .show_value(false),
            )
            .on_hover_text("Zoom, in points a tile. The wheel over the map zooms around the pointer.");
            if ui.small_button("-").on_hover_text("Zoom out.").clicked() {
                cell /= STEP;
            }
            if ui
                .small_button("Whole Map")
                .on_hover_text("Shows all 64x64 slots, for making ground where the map has none.")
                .clicked()
            {
                view.fit = Some(Fit::Whole);
            }
            if ui
                .small_button("Fit")
                .on_hover_text("Frames the map's tiles and the selection.")
                .clicked()
            {
                view.fit = Some(Fit::Tiles);
            }
            if cell != view.cell {
                if let Some(area) = view.area {
                    view.zoom_about(cell, area.center(), area);
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let (text, colour) = match view.hovered {
                    Some(at) => (describe(view, session, at), theme::INK),
                    None if view.selection.is_empty() => (selection_line(view, session), theme::INK_FAINT),
                    None => (selection_line(view, session), theme::INK_DIM),
                };
                ui.add(egui::Label::new(egui::RichText::new(text).color(colour)).truncate());
            });
        });
    });
}

/// The map itself, with its rulers, and the pointer over it.
#[allow(clippy::too_many_arguments)]
fn map_area(
    ui: &mut egui::Ui,
    view: &mut MapView,
    session: &EditSession,
    thumbnails: &mut Thumbnails,
    minimaps: &vale_assets::tables::minimap::MinimapTiles,
    camera_tile: (u32, u32),
    gate: &crate::collab::ServerGate,
) -> Option<Asked> {
    let mut asked = None;
    let size = ui.available_size().max(egui::vec2(160.0, 160.0));
    let (whole, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let area = egui::Rect::from_min_max(whole.min + egui::vec2(RULER, RULER), whole.max);
    let room = area.size();

    // ------------------------------------------------------- fit and zoom
    let refit = view.cell <= 0.0 || !view.fitted_for.eq_ignore_ascii_case(&session.map);
    if let Some(fit) = view.fit.take().or(refit.then_some(Fit::Tiles)) {
        let (lo, hi, max) = match fit {
            Fit::Tiles => {
                let (lo, hi) = fit_bounds(session, &view.selection);
                (lo, hi, FIT_MAX)
            }
            Fit::Whole => ((0, 0), (SIDE - 1, SIDE - 1), MAX_CELL),
        };
        (view.cell, view.origin) = framing(lo, hi, room, max);
        view.fitted_for = session.map.clone();
    }
    if response.hovered() {
        let (wheel, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = (wheel * WHEEL).exp() * pinch;
        if factor != 1.0 {
            if let Some(at) = response.hover_pos() {
                view.zoom_about(view.cell * factor, at, area);
            }
        }
    }
    if response.dragged_by(egui::PointerButton::Secondary) || response.dragged_by(egui::PointerButton::Middle) {
        view.origin -= response.drag_delta() / view.cell;
    }
    view.keep_in_view(room);
    view.area = Some(area);

    // ------------------------------------------------------------ geometry
    let cell = view.cell;
    let origin = view.origin;
    let ppp = ui.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let x_of = |col: f32| snap(area.min.x + (col - origin.x) * cell);
    let y_of = |row: f32| snap(area.min.y + (row - origin.y) * cell);
    let square = |at: (u32, u32)| {
        egui::Rect::from_min_max(
            egui::pos2(x_of(at.0 as f32), y_of(at.1 as f32)),
            egui::pos2(x_of(at.0 as f32 + 1.0), y_of(at.1 as f32 + 1.0)),
        )
    };
    // Only the cells on screen are drawn and ask for a picture: asking for
    // four thousand pictures a frame is the load the budget in `thumbnails`
    // exists to bound.
    let span = |from: f32, room: f32| {
        let first = from.floor().clamp(0.0, SIDE as f32) as u32;
        let last = (from + room / cell).ceil().clamp(0.0, SIDE as f32) as u32;
        first..last
    };
    let cols = span(origin.x, room.x);
    let rows = span(origin.y, room.y);
    let visible = || rows.clone().flat_map(|y| cols.clone().map(move |x| (x, y)));
    let of = |p: egui::Pos2| -> Option<(u32, u32)> {
        if !area.contains(p) {
            return None;
        }
        let tile = origin + (p - area.min) / cell;
        (tile.x >= 0.0 && tile.y >= 0.0 && tile.x < SIDE as f32 && tile.y < SIDE as f32)
            .then_some((tile.x as u32, tile.y as u32))
    };

    // ------------------------------------------------------------- the map
    let painter = ui.painter_at(area);
    painter.rect_filled(area, 0.0, theme::SHELL);
    let grid = egui::Rect::from_min_max(egui::pos2(x_of(0.0), y_of(0.0)), egui::pos2(x_of(SIDE as f32), y_of(SIDE as f32)));
    painter.rect_filled(grid, 0.0, theme::SUNK);
    let sharp = cell * ppp >= SHARP_FROM;
    for at in visible() {
        paint_cell(
            &painter,
            square(at),
            at,
            session,
            thumbnails,
            minimaps,
            Marks {
                selected: view.selection.contains(&at),
                in_project: view.in_project(session, at),
                unsaved: session.unsaved.contains(&at),
                claim: view.claims.tiles.get(&at),
                edged: cell >= EDGE_FROM,
                sharp,
            },
        );
    }

    // The regions' outlines go over every cell, so a neighbour's picture
    // does not cover half of a line.
    let line = match cell >= 8.0 {
        true => 2.0,
        false => 1.0,
    };
    for at in visible() {
        let Some(claim) = view.claims.tiles.get(&at) else {
            continue;
        };
        let sides = boundary(at, |x, y| {
            x >= 0
                && y >= 0
                && view
                    .claims
                    .tiles
                    .get(&(x as u32, y as u32))
                    .is_some_and(|other| other.author == claim.author && other.checked_out == claim.checked_out)
        });
        let [r, g, b] = claim.colour;
        paint_sides(&painter, square(at), sides, egui::Stroke::new(line, egui::Color32::from_rgb(r, g, b)), !claim.checked_out);
        if !claim.mine && cell >= LOCK_FROM {
            padlock(&painter, square(at), egui::Color32::from_rgb(r, g, b));
        }
    }
    for at in visible().filter(|at| view.selection.contains(at)) {
        let sides = boundary(at, |x, y| x >= 0 && y >= 0 && view.selection.contains(&(x as u32, y as u32)));
        paint_sides(&painter, square(at), sides, egui::Stroke::new(line.min(1.5), theme::ACCENT), false);
    }
    if cols.contains(&camera_tile.0) && rows.contains(&camera_tile.1) {
        painter.rect_stroke(square(camera_tile), 0.0, egui::Stroke::new(2.0, theme::GOOD), egui::StrokeKind::Inside);
    }
    view.hovered = response.hover_pos().and_then(of);
    if let Some(at) = view.hovered {
        painter.rect_stroke(square(at), 0.0, egui::Stroke::new(1.0, theme::INK), egui::StrokeKind::Inside);
    }
    rulers(ui, whole, cell, &cols, &rows, &x_of, &y_of);

    // --------------------------------------------------------- the pointer
    let additive = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
    if response.drag_started_by(egui::PointerButton::Primary) {
        view.drag_from = ui.input(|i| i.pointer.press_origin()).and_then(of);
    }
    if response.dragged_by(egui::PointerButton::Primary) {
        // The box is recomputed from both ends every frame rather than
        // accumulated, so dragging back shrinks the selection instead of
        // leaving a trail.
        if let (Some(from), Some(to)) = (view.drag_from, response.interact_pointer_pos().and_then(of)) {
            if !additive {
                view.selection.clear();
            }
            for y in from.1.min(to.1)..=from.1.max(to.1) {
                for x in from.0.min(to.0)..=from.0.max(to.0) {
                    view.selection.insert((x, y));
                }
            }
        }
    }
    if response.drag_stopped_by(egui::PointerButton::Primary) {
        view.drag_from = None;
    }
    if response.clicked() {
        if let Some(at) = response.interact_pointer_pos().and_then(of) {
            match additive {
                true => {
                    if !view.selection.remove(&at) {
                        view.selection.insert(at);
                    }
                }
                false => view.select_only(at),
            }
        }
    }
    if response.double_clicked() {
        if let Some(at) = response.interact_pointer_pos().and_then(of) {
            asked = Some(Asked::FlyTo(at));
        }
    }
    // A right click on a tile outside the selection selects it first, so the
    // menu acts on what was clicked.
    if response.secondary_clicked() {
        view.menu_at = response.interact_pointer_pos().and_then(of);
        if let Some(at) = view.menu_at {
            if !view.selection.contains(&at) {
                view.select_only(at);
            }
        }
    }
    response.context_menu(|ui| {
        if let Some(more) = tile_menu(ui, view, session, gate) {
            asked = Some(more);
        }
    });
    asked
}

/// The map's right-click menu: flying to the tile, the toolbar's operations,
/// the claims, and selecting.
fn tile_menu(ui: &mut egui::Ui, view: &mut MapView, session: &EditSession, gate: &crate::collab::ServerGate) -> Option<Asked> {
    let mut asked = None;
    let mut done = false;
    if let Some(at) = view.menu_at {
        if ui.button(format!("Fly to {}, {}", at.0, at.1)).clicked() {
            asked = Some(Asked::FlyTo(at));
        }
        ui.separator();
    }
    if let Some(more) = tile_buttons(ui, view, session, gate.role.joined()) {
        asked = Some(more);
    }
    ui.separator();
    claim_buttons(ui, view, session, true);
    if let Some(claim) = view.menu_at.and_then(|at| view.claims.tiles.get(&at)).cloned() {
        if ui.button(format!("Select {}'s Tiles", holder_name(&claim))).clicked() {
            select_holder(view, &claim.author, false);
            done = true;
        }
    }
    ui.separator();
    if ui.button("Select All").clicked() {
        select_all(view, session);
        done = true;
    }
    if ui.add_enabled(!view.selection.is_empty(), egui::Button::new("Clear Selection")).clicked() {
        view.selection.clear();
        done = true;
    }
    if done || asked.is_some() || view.claim_ask.is_some() {
        ui.close();
    }
    asked
}

/// A holder's name as a list says it: `This Editor` for this editor's author.
fn holder_name(claim: &TileClaim) -> String {
    match claim.mine {
        true => "This Editor".to_string(),
        false => claim.who.clone(),
    }
}

/// Select every tile `author` holds a claim on; with `add`, add them to the
/// selection.
fn select_holder(view: &mut MapView, author: &vale_collab::log::AuthorId, add: bool) {
    if !add {
        view.selection.clear();
    }
    let held: Vec<(u32, u32)> = view
        .claims
        .tiles
        .iter()
        .filter(|(_, claim)| claim.author == *author)
        .map(|(at, _)| *at)
        .collect();
    view.selection.extend(held);
}

/// Select every tile the map has.
fn select_all(view: &mut MapView, session: &EditSession) {
    view.selection = (0..SIDE)
        .flat_map(|y| (0..SIDE).map(move |x| (x, y)))
        .filter(|&at| session.wdt_claims(at))
        .collect();
}

/// The selection as the counts the operations act on: existing, empty, in
/// this project.
fn selection_line(view: &MapView, session: &EditSession) -> String {
    let (exist, absent) = view.split(session);
    let mine = view
        .selection
        .iter()
        .filter(|&&at| view.in_project(session, at))
        .count();
    match view.selection.len() {
        0 => "Nothing selected".to_string(),
        1 => {
            let at = view.selection.iter().next().copied().unwrap_or_default();
            format!("{}, {} \u{2014} {}", at.0, at.1, describe_state(view, session, at))
        }
        n => {
            let mut parts = vec![format!("{n} selected: {exist} exist, {absent} empty")];
            if mine > 0 {
                parts.push(format!("{mine} in this project"));
            }
            parts.join(" \u{b7} ")
        }
    }
}

/// `exists`, `empty`, `in this project`, `unsaved`, `open`, and the claim on
/// it: one tile's state as words.
fn describe_state(view: &MapView, session: &EditSession, at: (u32, u32)) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(
        match session.wdt_claims(at) {
            true => "exists",
            false => "empty",
        }
        .to_string(),
    );
    if session.unsaved.contains(&at) {
        parts.push("unsaved".to_string());
    } else if view.in_project(session, at) {
        parts.push("in this project".to_string());
    }
    if crate::map::tiles::is_open(session, at) {
        parts.push("open".to_string());
    }
    if let Some(claim) = view.claims.tiles.get(&at) {
        parts.push(claim.words());
    }
    parts.join(", ")
}

/// The status line's text for the tile under the pointer.
fn describe(view: &MapView, session: &EditSession, at: (u32, u32)) -> String {
    format!("{}, {} \u{2014} {}", at.0, at.1, describe_state(view, session, at))
}

/// The column and row numbers along the top and the left, which stay in
/// place while the map moves: every column at a close zoom and every few at
/// the others, with a tick per column.
fn rulers(
    ui: &egui::Ui,
    whole: egui::Rect,
    cell: f32,
    cols: &std::ops::Range<u32>,
    rows: &std::ops::Range<u32>,
    x_of: &dyn Fn(f32) -> f32,
    y_of: &dyn Fn(f32) -> f32,
) {
    let every = match cell {
        c if c >= 20.0 => 1,
        c if c >= 12.0 => 2,
        c if c >= 6.0 => 4,
        _ => 8,
    };
    let font = egui::FontId::proportional(theme::SMALL);
    let top = egui::Rect::from_min_max(whole.min + egui::vec2(RULER, 0.0), egui::pos2(whole.max.x, whole.min.y + RULER));
    let left = egui::Rect::from_min_max(whole.min + egui::vec2(0.0, RULER), egui::pos2(whole.min.x + RULER, whole.max.y));
    let corner = egui::Rect::from_min_size(whole.min, egui::vec2(RULER, RULER));
    let tick = egui::Stroke::new(1.0, theme::LINE);
    let painter = ui.painter_at(whole);
    painter.rect_filled(top, 0.0, theme::PANEL);
    painter.rect_filled(left, 0.0, theme::PANEL);
    painter.rect_filled(corner, 0.0, theme::PANEL);
    let along = ui.painter_at(top);
    for x in cols.clone() {
        let px = x_of(x as f32);
        along.line_segment([egui::pos2(px, top.max.y - 3.0), egui::pos2(px, top.max.y)], tick);
        if x % every == 0 {
            along.text(
                egui::pos2(px + cell * 0.5, top.center().y - 1.0),
                egui::Align2::CENTER_CENTER,
                x.to_string(),
                font.clone(),
                theme::INK_DIM,
            );
        }
    }
    let down = ui.painter_at(left);
    for y in rows.clone() {
        let py = y_of(y as f32);
        down.line_segment([egui::pos2(left.max.x - 3.0, py), egui::pos2(left.max.x, py)], tick);
        if y % every == 0 {
            down.text(
                egui::pos2(left.center().x, py + cell * 0.5),
                egui::Align2::CENTER_CENTER,
                y.to_string(),
                font.clone(),
                theme::INK_DIM,
            );
        }
    }
}

/// The side panel: the selection, the claims, new ground's settings, and the
/// files derived from the selected tiles.
fn side_panel(
    ui: &mut egui::Ui,
    view: &mut MapView,
    session: &EditSession,
    tiles: &mut crate::map::tiles::Tiles,
    gate: &crate::collab::ServerGate,
) -> Option<Asked> {
    use crate::map::tiles as tool;
    let joined = gate.role.joined();
    let mut asked = None;
    let (exist, absent) = view.split(session);
    // Both picture operations and an import need the tile open, and the
    // streamer opens only the 3x3 tiles around the camera. Their buttons
    // count the open selected tiles and are disabled when there are none,
    // rather than failing after a minute.
    let open = view.selection.iter().filter(|&&at| tool::is_open(session, at)).count();
    // Every tile the map has, for the whole-map buttons and the map's layout.
    let whole = (0..SIDE)
        .flat_map(|y| (0..SIDE).map(move |x| (x, y)))
        .filter(|&at| session.wdt_claims(at))
        .count();

    theme::heading(ui, "Selection");
    ui.label(egui::RichText::new(selection_line(view, session)).color(match view.selection.is_empty() {
        true => theme::INK_FAINT,
        false => theme::INK,
    }));
    ui.horizontal(|ui| {
        if ui
            .add_enabled(whole > 0, egui::Button::new("Select All"))
            .on_hover_text("Selects every tile the map has.")
            .clicked()
        {
            select_all(view, session);
        }
        if ui
            .add_enabled(!view.selection.is_empty(), egui::Button::new("Clear"))
            .on_hover_text("Clears the selection.")
            .clicked()
        {
            view.selection.clear();
        }
    });
    theme::note(
        ui,
        "Click a tile, drag a box, Ctrl+click to add or remove. Double-click flies there; right-click \
         opens a menu. The wheel zooms, and dragging with the right or middle button moves the map.",
    );

    claims_section(ui, view, session);

    // What new ground is made of, which only matters while there is some to
    // make. Create itself is on the toolbar with the other operations.
    theme::heading(ui, "New Ground");
    ui.add_enabled_ui(absent > 0, |ui| {
        theme::row(ui, "Height", |ui| {
            ui.add(egui::DragValue::new(&mut tiles.height).speed(1.0).suffix(" yd"))
                .on_hover_text("Height of a new tile's flat ground.");
        });
        theme::row(ui, "Area", |ui| {
            ui.add(egui::DragValue::new(&mut tiles.area).speed(1.0))
                .on_hover_text("The AreaTable id every chunk of a new tile is given. 0 is none.");
        });
        theme::row(ui, "Texture", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut tiles.texture)
                    .desired_width(f32::INFINITY)
                    .hint_text("Tileset\\…"),
            )
            .on_hover_text("The single texture a new tile is painted with.");
        });
    });
    if absent == 0 {
        theme::note(ui, "Select empty tiles to create ground on them.");
    }

    theme::heading(ui, "Shadows");
    if ui
        .add_enabled(open > 0 && !joined, egui::Button::new(format!("Rebake {open}")))
        .on_hover_text(format!(
            "Recomputes the baked shadow of each open selected tile from the objects on it, in \
             the background. About {} s per tile.",
            tool::REBAKE_SECONDS
        ))
        .on_disabled_hover_text(host_only(joined, "None of the selection is open: fly to it first."))
        .clicked()
    {
        asked = Some(Asked::Rebake);
    }
    ui.checkbox(&mut tiles.cast_from_ground, "Include terrain shadow").on_hover_text(
        "Also bakes shadow cast by the terrain onto itself. Off by default, because the shipped \
         bakes do not appear to include it: against two shipped tiles, hull-cast shadow matches \
         their MCSH at 1.2-1.4x the base rate and terrain-cast shadow at 0.4-0.9x, which is \
         chance or worse.",
    );

    theme::heading(ui, "Minimaps");
    ui.horizontal_wrapped(|ui| {
        if tiles.drawing {
            if ui
                .button("Stop Minimaps")
                .on_hover_text("Stops after the tile being drawn. Tiles already drawn are kept.")
                .clicked()
            {
                tiles.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            return;
        }
        let minutes = |tiles: usize| {
            let seconds = tiles as f32 * tool::MINIMAP_SECONDS;
            match seconds < 90.0 {
                true => format!("about {seconds:.0} s"),
                false => format!("about {:.0} min", seconds / 60.0),
            }
        };
        // Minimaps are drawn in the background for any tile that exists, open
        // or not, so these count the tiles there are.
        if ui
            .add_enabled(exist > 0 && !joined, egui::Button::new(format!("Selected {exist}")))
            .on_hover_text(format!(
                "Draws the selected tiles' minimap pictures into the project, in the background: {}.",
                minutes(exist)
            ))
            .on_disabled_hover_text(host_only(joined, "Select tiles that exist."))
            .clicked()
        {
            asked = Some(Asked::Minimap);
        }
        if ui
            .add_enabled(whole > 0 && !joined, egui::Button::new(format!("Whole Map {whole}")))
            .on_hover_text(format!(
                "Draws every tile's minimap picture into the project, in the background: {}. New \
                 tiles are added to the minimap index.",
                minutes(whole)
            ))
            .on_disabled_hover_text(host_only(joined, "The map has no tiles."))
            .clicked()
        {
            asked = Some(Asked::MinimapsAll);
        }
    });

    // The selection's ground as picture files. An export reads closed tiles
    // too; an import writes the open ones.
    theme::heading(ui, "Images");
    ui.horizontal_wrapped(|ui| {
        for (text, choice, about) in [
            ("Export Heights…", Asked::ExportHeights, "Saves the selection's heights as a 16-bit grey PNG."),
            (
                "Export Blends…",
                Asked::ExportBlends,
                "Saves how the selection's textures blend as an RGB PNG: red, green and blue are each \
                 chunk's second, third and fourth texture.",
            ),
        ] {
            if ui
                .add_enabled(exist > 0, egui::Button::new(text))
                .on_hover_text(about)
                .on_disabled_hover_text("Select tiles that exist.")
                .clicked()
            {
                asked = Some(choice);
            }
        }
        if ui
            .add_enabled(open > 0, egui::Button::new("Import…"))
            .on_hover_text("Chooses a PNG to import onto the selection: a height map or a blend map, any size.")
            .on_disabled_hover_text("None of the selection is open: fly to it first.")
            .clicked()
        {
            asked = Some(Asked::ChooseImport);
        }
    });
    if let Some(choice) = import_dialog(ui.ctx(), view, session, tiles) {
        asked = Some(choice);
    }

    theme::heading(ui, "Server");
    if ui
        .add_enabled(exist > 0 && gate.writes(), egui::Button::new(format!("Server Files {exist}")))
        .on_hover_text(
            "Saves, rebuilds this project's archive, and regenerates the server's maps, vmaps and \
             mmaps for each selected tile that exists: from this project where it carries the \
             tile, from the archives where it does not. Minutes: a navmesh tile and its four \
             neighbours, and the map's vmaps when a building moved. The bar says which step is \
             running; restart the server afterwards. Publish does this for every changed tile.",
        )
        .on_disabled_hover_text(gate.refused.clone().unwrap_or_else(|| "None of the selection exists.".to_string()))
        .clicked()
    {
        asked = Some(Asked::ServerFiles);
    }

    // What the map is: terrain, or one building with no tiles (most
    // dungeons). See `vale_edit::wdt::ONE_BUILDING`. It is changed in Map
    // Properties, which the button opens.
    theme::heading(ui, "Map");
    match &tiles.building {
        Some(path) => {
            let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
            ui.label(format!("Single-WMO map: {leaf}")).on_hover_text(path.clone());
        }
        None => theme::note(ui, format!("Terrain, {whole} tiles")),
    }
    if ui
        .button("Map Properties\u{2026}")
        .on_hover_text("Edits the map's rows and its layout: terrain or a single WMO.")
        .clicked()
    {
        tiles.new_map = Some(crate::map::maps::Form::editing(session.map_id, &session.map));
    }

    if !tiles.said.is_empty() {
        ui.add_space(6.0);
        theme::note(ui, tiles.said.clone());
    }

    theme::heading(ui, "Marks");
    legend(ui);
    asked
}

/// One author's claims on the open map, for the side panel's list.
struct Holder {
    author: vale_collab::log::AuthorId,
    name: String,
    mine: bool,
    colour: egui::Color32,
    checked_out: usize,
    edited: usize,
}

/// The authors with a claim on the open map, this editor's author first and
/// the rest by name.
fn holders(claims: &MapClaims) -> Vec<Holder> {
    let mut holders: Vec<Holder> = Vec::new();
    for claim in claims.tiles.values() {
        let at = match holders.iter().position(|holder| holder.author == claim.author) {
            Some(at) => at,
            None => {
                let [r, g, b] = claim.colour;
                holders.push(Holder {
                    author: claim.author,
                    name: holder_name(claim),
                    mine: claim.mine,
                    colour: egui::Color32::from_rgb(r, g, b),
                    checked_out: 0,
                    edited: 0,
                });
                holders.len() - 1
            }
        };
        holders[at].checked_out += claim.checked_out as usize;
        holders[at].edited += claim.edited as usize;
    }
    holders.sort_by(|a, b| b.mine.cmp(&a.mine).then_with(|| a.name.cmp(&b.name)));
    holders
}

/// The claims on the open map, one row per author, and the claim buttons.
fn claims_section(ui: &mut egui::Ui, view: &mut MapView, session: &EditSession) {
    theme::heading(ui, &format!("Claims ({})", view.claims.tiles.len()));
    let holders = holders(&view.claims);
    if holders.is_empty() {
        theme::note(ui, "No tile of this map is claimed.");
    }
    for holder in holders {
        let mut counts = Vec::new();
        if holder.checked_out > 0 {
            counts.push(format!("{} checked out", holder.checked_out));
        }
        if holder.edited > 0 {
            counts.push(format!("{} with unsubmitted edits", holder.edited));
        }
        // The name on one line, cut to the panel, and the counts under it,
        // wrapped: a name and its counts on one line ran past the panel.
        let row = ui
            .vertical(|ui| {
                ui.horizontal(|ui| {
                    let (swatch, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().rect_filled(swatch, 2.0, holder.colour);
                    ui.add(egui::Label::new(egui::RichText::new(&holder.name).color(theme::INK)).truncate());
                });
                theme::note(ui, counts.join(" \u{b7} "));
            })
            .response;
        let press = ui
            .interact(row.rect, ui.id().with(("map-holder", holder.author)), egui::Sense::click())
            .on_hover_text(format!("Selects {}'s tiles. Ctrl adds them to the selection.", holder.name));
        if press.hovered() {
            ui.painter().rect_stroke(row.rect.expand(2.0), 3.0, egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Outside);
        }
        if press.clicked() {
            let add = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
            select_holder(view, &holder.author, add);
        }
    }
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| claim_buttons(ui, view, session, true));
    if !view.claims.can_check_out {
        theme::note(
            ui,
            "Check outs need a shared session, whose host keeps them. Alone, the list shows the \
             tiles that unsubmitted edits in the log hold.",
        );
    }
}

/// Paints a small copy of one of the map's marks into a rectangle.
type Sample = fn(&egui::Painter, egui::Rect);

/// The marks the map draws, each beside a small copy of itself.
fn legend(ui: &mut egui::Ui) {
    let entries: [(&str, Sample); 7] = [
        ("In this project", |p, r| corner_mark(p, r, theme::ACCENT)),
        ("Unsaved changes", |p, r| corner_mark(p, r, theme::WARN)),
        ("Camera", |p, r| {
            p.rect_stroke(r, 0.0, egui::Stroke::new(2.0, theme::GOOD), egui::StrokeKind::Inside);
        }),
        ("Selected", |p, r| {
            p.rect_filled(r, 0.0, theme::ACCENT.gamma_multiply(0.25));
            p.rect_stroke(r, 0.0, egui::Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Inside);
        }),
        ("Checked out", |p, r| paint_sides(p, r, [true; 4], egui::Stroke::new(2.0, theme::INK_DIM), false)),
        ("Unsubmitted edits", |p, r| paint_sides(p, r, [true; 4], egui::Stroke::new(2.0, theme::INK_DIM), true)),
        ("Held by another author", |p, r| padlock(p, r, theme::INK_DIM)),
    ];
    for (text, paint) in entries {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 0.0, theme::RAISED);
            paint(ui.painter(), rect);
            theme::note(ui, text);
        });
    }
}

/// The New map form. Answers `NewMap` when Make is pressed with nothing
/// wrong; Cancel drops the form.
fn new_map_dialog(
    ctx: &egui::Context,
    session: &EditSession,
    tiles: &mut crate::map::tiles::Tiles,
    thumbnails: &mut Thumbnails,
    screens: &vale_assets::tables::loading::LoadingScreens,
    camera_at: [f32; 2],
) -> Option<Asked> {
    use vale_assets::tables::map::INSTANCE_TYPES;
    let form = tiles.new_map.as_mut()?;
    let mut asked = None;
    let mut close = false;
    let response = egui::Modal::new(egui::Id::new("map-new")).show(ctx, |ui| {
        ui.set_width(420.0);
        let title = match form.editing {
            None => "New Map".to_string(),
            Some(id) => format!("Map Properties \u{2014} {} ({id})", form.directory),
        };
        ui.label(egui::RichText::new(title).strong().size(14.0));
        ui.add_space(4.0);
        if form.editing.is_some() && !form.loaded {
            theme::note(ui, "reading the map's rows…");
            if ui.button("Cancel").clicked() {
                close = true;
            }
            return;
        }
        const FOLDER: &str = "The map's folder, World\\Maps\\<folder>\\, which is also the start \
                              of every tile's file name. Letters, digits and underscores, \
                              starting with a letter. It cannot be changed afterwards without \
                              renaming every tile.";
        const NAME: &str = "The map's name, as the loading screen, the map menu and the server \
                            show it. It can be changed later in Map.dbc.";
        const TYPE: &str = "The map's instance type. A world map is open ground everyone \
                            shares. A dungeon or raid is an instance: each group gets its own \
                            copy, entered through a portal, and it takes a player limit, a ghost \
                            entrance and a parent. A battleground is an instance the \
                            battleground queue sends players to.";
        const PLAYERS: &str = "How many characters one copy of the instance holds: 5 for a \
                               dungeon, 10, 20 or 40 for a raid. 0 for no limit.";
        theme::row_about(ui, "Folder", FOLDER, |ui| match form.editing {
            None => {
                ui.text_edit_singleline(&mut form.directory).on_hover_text(FOLDER);
            }
            Some(_) => {
                ui.label(egui::RichText::new(&form.directory).color(theme::INK))
                    .on_hover_text(FOLDER);
            }
        });
        theme::row_about(ui, "Name", NAME, |ui| {
            ui.text_edit_singleline(&mut form.name).on_hover_text(NAME);
        });
        theme::row_about(ui, "Type", TYPE, |ui| {
            egui::ComboBox::from_id_salt("map-new-type")
                .selected_text(
                    INSTANCE_TYPES
                        .iter()
                        .find(|(value, _)| *value == form.instance_type)
                        .map_or("?", |(_, name)| *name),
                )
                .show_ui(ui, |ui| {
                    for (value, name) in INSTANCE_TYPES {
                        ui.selectable_value(&mut form.instance_type, value, name);
                    }
                })
                .response
                .on_hover_text(TYPE);
        });
        if form.instance_type != 0 {
            theme::row_about(ui, "Players", PLAYERS, |ui| {
                ui.add(egui::DragValue::new(&mut form.max_players).range(0..=40))
                    .on_hover_text(PLAYERS);
            });
        }
        let open_screen = screens.screen_of(session.map_id);
        let screen_name = |id: Option<u32>| match id.and_then(|id| screens.screen(id)) {
            Some((name, _)) => name.to_string(),
            None => id.map_or_else(|| "none".to_string(), |id| format!("row {id}")),
        };
        const SCREEN: &str = "The picture shown while a character is loading into the map: a \
                              LoadingScreens row. Left alone, the new map uses the open map's.";
        theme::row_about(ui, "Loading Screen", SCREEN, |ui| {
            let chosen = match form.loading_screen {
                None => format!("the open map's ({})", screen_name(open_screen)),
                Some(id) => screen_name(Some(id)),
            };
            ui.label(egui::RichText::new(chosen).color(theme::INK));
            let label = match form.choosing_screen {
                true => "Close",
                false => "Choose\u{2026}",
            };
            if ui.button(label).on_hover_text(SCREEN).clicked() {
                form.choosing_screen = !form.choosing_screen;
            }
        });
        if form.choosing_screen {
            if let Some(choice) = screen_picker(ui, thumbnails, screens, form.loading_screen, open_screen) {
                form.loading_screen = choice;
                form.choosing_screen = false;
            }
        }
        edit_rows(ui, form);
        if form.is_instance() {
            instance_rows(ui, session, form, camera_at);
        }
        if form.editing.is_none() {
            ui.checkbox(&mut form.zone, "Create zone")
                .on_hover_text(
                    "An AreaTable zone named after the map, which the map's row names as its \
                     area. Paint it onto the ground with the Areas tool.",
                );
            ui.checkbox(&mut form.open_it, "Open after creating")
                .on_hover_text("Switches the editor to the new map once it is created, saving the open map first.");
        }
        let problem = form.problem(&session.maps);
        if let Some(why) = &problem {
            ui.label(egui::RichText::new(why).size(theme::SMALL).color(theme::WARN));
        }
        theme::note(
            ui,
            match form.editing {
                None => "The server needs the map's tiles extracted (Server Files) and a restart \
                         before a character can stand on it.",
                Some(_) => "Map.dbc changes reach the client through the project's archive. \
                            map_template changes are applied from the Server panel, and the \
                            server reads them on .reload map_template.",
            },
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let (button, answer) = match form.editing {
                None => ("Create", Asked::NewMap),
                Some(_) => ("Save", Asked::SaveMap),
            };
            if ui.add_enabled(problem.is_none(), egui::Button::new(button)).clicked() {
                asked = Some(answer);
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if close || response.should_close() {
        tiles.new_map = None;
    }
    asked
}

/// The rest of the map's `Map.dbc` row, a raid's reset and the layout, on
/// both forms; and on an existing map's, where its server row came from.
fn edit_rows(ui: &mut egui::Ui, form: &mut crate::map::maps::Form) {
    use crate::map::maps::ServerRow;
    const PVP: &str = "Map.dbc field 3. 4 of the 44 shipped maps set it.";
    const LEVELS: &str = "Map.dbc fields 13 and 14: the lowest and highest character level \
                          for the map. 0 is none. 4 of the 44 shipped maps set them.";
    const AREA: &str = "The zone the map belongs to: an AreaTable id. Written to Map.dbc \
                        field 19 and to the server's map_template.linked_zone. 0 is none.";
    const DESCRIPTION: &str = "Map.dbc fields 20 and 29. Most shipped maps leave both empty.";
    const RESET: &str = "map_template.reset_delay: days between the raid's resets. 0 is none.";
    const LAYOUT: &str = "Terrain: the map is drawn from its ADT tiles. Single WMO: the map is \
                          one WMO and has no tiles, as most dungeons are; the WDT's header flag \
                          says so and names the WMO. Only a map with no tiles can be made a \
                          single-WMO map.";
    theme::row_about(ui, "PvP", PVP, |ui| {
        ui.checkbox(&mut form.pvp, "").on_hover_text(PVP);
    });
    theme::row_about(ui, "Levels", LEVELS, |ui| {
        ui.add(egui::DragValue::new(&mut form.min_level).range(0..=255).prefix("min "))
            .on_hover_text(LEVELS);
        ui.add(egui::DragValue::new(&mut form.max_level).range(0..=255).prefix("max "))
            .on_hover_text(LEVELS);
    });
    // A new map's zone is made by Create zone unless that is off.
    if form.editing.is_some() || !form.zone {
        theme::row_about(ui, "Zone", AREA, |ui| {
            ui.add(egui::DragValue::new(&mut form.area).speed(1.0)).on_hover_text(AREA);
        });
    }
    for (n, label) in ["Description 0", "Description 1"].into_iter().enumerate() {
        theme::row_about(ui, label, DESCRIPTION, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut form.descriptions[n])
                    .desired_rows(1)
                    .desired_width(f32::INFINITY),
            )
            .on_hover_text(DESCRIPTION);
        });
    }
    if form.instance_type == 2 {
        theme::row_about(ui, "Reset", RESET, |ui| {
            ui.add(egui::DragValue::new(&mut form.reset_delay).range(0..=365).suffix(" days"))
                .on_hover_text(RESET);
        });
    }
    theme::row_about(ui, "Layout", LAYOUT, |ui| {
        ui.radio_value(&mut form.single_wmo, false, "Terrain").on_hover_text(LAYOUT);
        ui.radio_value(&mut form.single_wmo, true, "Single WMO").on_hover_text(LAYOUT);
    });
    if form.single_wmo {
        theme::row(ui, "WMO", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut form.wmo)
                    .desired_width(f32::INFINITY)
                    .hint_text(r"World\wmo\…\name.wmo"),
            )
            .on_hover_text("The archive path of the WMO's root file.");
        });
    }
    if form.editing.is_none() {
        return;
    }
    if form.single_wmo != !form.wmo_was.is_empty() || (form.single_wmo && form.wmo.trim() != form.wmo_was) {
        theme::note(
            ui,
            "A layout change is written to the project's WDT when saved, is not on the \
             undo history, and shows after the map is opened again.",
        );
    }
    let server = match &form.server {
        ServerRow::Unread | ServerRow::Reading => "reading the server's map_template row…".to_string(),
        ServerRow::Found { key, created: true, .. } => format!("map_template {}: created by this project", key.text()),
        ServerRow::Found { key, created: false, .. } => {
            format!("map_template {}: from the database, with this project's edits", key.text())
        }
        ServerRow::Missing => "the database has no map_template row for this map; the server columns \
                               will not be saved"
            .to_string(),
        ServerRow::Unreachable(why) => format!("map_template could not be read ({why}); the server \
                                               columns will not be saved"),
    };
    theme::note(ui, server);
}

/// A dungeon's or raid's parent and ghost entrance on the New map form.
/// `camera_at` is where the camera stands on the open map, north and west.
fn instance_rows(ui: &mut egui::Ui, session: &EditSession, form: &mut crate::map::maps::Form, camera_at: [f32; 2]) {
    use vale_mangos::map::CONTINENTS;
    let name_of = |id: u32| {
        session
            .maps
            .iter()
            .find(|(map, _)| *map == id)
            .map_or_else(|| format!("map {id}"), |(_, dir)| format!("{dir} ({id})"))
    };
    const GHOST: &str = "Where this dungeon's entrance is out in the world: a point on a \
                         continent, usually in front of the portal.\n\n\
                         A character who dies inside cannot release back into the dungeon. \
                         Their ghost is sent to the graveyard nearest this point, the corpse \
                         arrow points here, and walking into the portal brings them back to \
                         life inside. Without it, a ghost has no entrance to return to.\n\n\
                         It must be on a continent. Set the camera over the portal and \
                         press Use Camera Position, or enter the point below.";
    const PARENT: &str = "Only for a dungeon whose entrance is inside another dungeon, rather \
                          than out in the world: name that outer dungeon here.\n\n\
                          A character whose corpse lies in the outer dungeon may then walk \
                          their ghost into this one. Leave it at none for a dungeon entered \
                          from a continent, which is nearly all of them.";
    theme::row_about(ui, "Ghost Entrance", GHOST, |ui| {
        let chosen = match form.ghost {
            None => "none".to_string(),
            Some((map, _)) => name_of(map),
        };
        egui::ComboBox::from_id_salt("map-new-ghost")
            .selected_text(chosen)
            .show_ui(ui, |ui| {
                if ui.selectable_label(form.ghost.is_none(), "none").clicked() {
                    form.ghost = None;
                }
                for map in CONTINENTS {
                    let at = form.ghost.map_or([0.0, 0.0], |(_, at)| at);
                    if ui.selectable_label(form.ghost.map(|(m, _)| m) == Some(map), name_of(map)).clicked() {
                        form.ghost = Some((map, at));
                    }
                }
            })
            .response
            .on_hover_text(GHOST);
        let here = CONTINENTS.contains(&session.map_id);
        if ui
            .add_enabled(here, egui::Button::new("Use Camera Position"))
            .on_hover_text(
                "Uses the point the camera is over on the open map. Open the continent and \
                 fly to the portal first.",
            )
            .on_disabled_hover_text("The open map is not a continent.")
            .clicked()
        {
            form.ghost = Some((session.map_id, camera_at));
        }
    });
    if let Some((_, at)) = form.ghost.as_mut() {
        theme::row(ui, "", |ui| {
            const AXES: &str = "The ghost entrance on its continent, in yards: north and \
                                west, as .gps and the status line give them.";
            ui.label("North").on_hover_text(AXES);
            ui.add(egui::DragValue::new(&mut at[0]).speed(1.0).max_decimals(1))
                .on_hover_text(AXES);
            ui.label("West").on_hover_text(AXES);
            ui.add(egui::DragValue::new(&mut at[1]).speed(1.0).max_decimals(1))
                .on_hover_text(AXES);
        });
    }
    theme::row_about(ui, "Parent", PARENT, |ui| {
        let chosen = match form.parent {
            0 => "none".to_string(),
            id => name_of(id),
        };
        egui::ComboBox::from_id_salt("map-new-parent")
            .selected_text(chosen)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut form.parent, 0, "none");
                for (id, dir) in session.maps.iter().filter(|(id, _)| !CONTINENTS.contains(id)) {
                    ui.selectable_value(&mut form.parent, *id, format!("{dir} ({id})"));
                }
            })
            .response
            .on_hover_text(PARENT);
    });
}

/// The loading screens as pictures to choose from, the open map's first.
/// Answers the choice when one is clicked: `Some(None)` for the open map's,
/// `Some(Some(id))` for a row.
fn screen_picker(
    ui: &mut egui::Ui,
    thumbnails: &mut Thumbnails,
    screens: &vale_assets::tables::loading::LoadingScreens,
    chosen: Option<u32>,
    open_screen: Option<u32>,
) -> Option<Option<u32>> {
    // The pictures are 4:3, drawn at the thumbnail cache's icon size across.
    const CELL: egui::Vec2 = egui::vec2(128.0, 96.0);
    /// Three across, which fits the form's 420 points.
    const COLUMNS: usize = 3;
    let mut rows: Vec<(u32, &str, &str)> = screens.screens().collect();
    rows.sort_by_key(|(id, _, _)| *id);
    let mut picked = None;
    egui::ScrollArea::vertical()
        .id_salt("map-new-screens")
        .max_height(300.0)
        .show(ui, |ui| {
            // A grid of a fixed width: wrapped rows in a modal that sizes to
            // its contents never wrap, and the form grew across the window.
            egui::Grid::new("map-new-screen-grid").num_columns(COLUMNS).spacing([6.0, 6.0]).show(ui, |ui| {
                let open = open_screen.and_then(|id| screens.screen(id)).map(|(_, path)| path);
                let entries = std::iter::once((None, "the open map's", open))
                    .chain(rows.iter().map(|(id, name, path)| (Some(*id), *name, Some(*path))));
                for (n, (id, name, path)) in entries.enumerate() {
                    if n > 0 && n % COLUMNS == 0 {
                        ui.end_row();
                    }
                    ui.vertical(|ui| {
                        ui.set_width(CELL.x);
                        let (rect, response) = ui.allocate_exact_size(CELL, egui::Sense::click());
                        ui.painter().rect_filled(rect, 3.0, theme::SUNK);
                        if let Some(path) = path {
                            thumbnails.want_at(path, crate::ui::widgets::thumbnails::ICON_SIDE);
                            if let Some(texture) = thumbnails.get_at(path, crate::ui::widgets::thumbnails::ICON_SIDE) {
                                ui.painter().image(
                                    texture,
                                    rect.shrink(2.0),
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::WHITE,
                                );
                            }
                        }
                        let selected = id == chosen;
                        if selected || response.hovered() {
                            let colour = match selected {
                                true => theme::INK,
                                false => theme::INK_FAINT,
                            };
                            ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(2.0, colour), egui::StrokeKind::Inside);
                        }
                        let caption = match id {
                            Some(id) => format!("{id} {name}"),
                            None => name.to_string(),
                        };
                        ui.label(egui::RichText::new(caption).size(theme::SMALL).color(theme::INK));
                        if response.on_hover_text(path.unwrap_or("")).clicked() {
                            picked = Some(id);
                        }
                    });
                }
            });
        });
    picked
}

/// The import's confirmation: the picture, what it is used as, where it
/// lands, and for a height map the heights black and white stand for. Answers
/// `Import` when it is pressed; Cancel drops the picture.
fn import_dialog(
    ctx: &egui::Context,
    view: &MapView,
    session: &EditSession,
    tiles: &mut crate::map::tiles::Tiles,
) -> Option<Asked> {
    use crate::import::images::{self, Kind};
    let pending = tiles.pending.as_mut()?;
    let mut asked = None;
    let mut close = false;
    let selected: Vec<(u32, u32)> = view.selection.iter().copied().collect();
    let open = selected
        .iter()
        .filter(|&&at| crate::map::tiles::is_open(session, at))
        .count();
    let response = egui::Modal::new(egui::Id::new("map-import")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(egui::RichText::new(format!("Import {}", pending.name())).strong().size(14.0));
        theme::note(ui, format!("{} x {} pixels", pending.size.0, pending.size.1));
        ui.add_space(4.0);

        theme::row(ui, "Use As", |ui| {
            theme::segmented(
                ui,
                &mut pending.kind,
                &[("Heights", Kind::Heights), ("Blends", Kind::Blends)],
                |a, b| a == b,
            );
        });
        if pending.kind != pending.found {
            theme::note(
                ui,
                match pending.found {
                    Kind::Heights => "the picture is grey, which is usually a height map",
                    Kind::Blends => "the picture has colour, which is usually a blend map",
                },
            );
        }

        match images::block(&selected) {
            Some(block) => {
                theme::row(ui, "Onto", |ui| {
                    ui.label(images::block_words(block));
                });
                let fits = images::size_of(pending.kind, block.1);
                if fits != pending.size {
                    theme::note(ui, format!("scaled to {} x {} to fit", fits.0, fits.1));
                }
                if open < selected.len() {
                    theme::note(
                        ui,
                        format!("{open} of the {} selected tiles are open; the rest are not changed", selected.len()),
                    );
                }
            }
            None => theme::note(ui, "select tiles on the map"),
        }

        match pending.kind {
            Kind::Heights => {
                ui.add_space(4.0);
                for (label, value) in [("Black", &mut pending.low), ("White", &mut pending.high)] {
                    theme::row(ui, label, |ui| {
                        ui.add(
                            egui::DragValue::new(value)
                                .speed(0.25)
                                .fixed_decimals(1)
                                .suffix(" yd"),
                        );
                    });
                }
                theme::note(
                    ui,
                    match pending.range_in_file {
                        true => "range read from the file",
                        false => "range: the selected tiles' current lowest and highest ground",
                    },
                );
                ui.checkbox(&mut tiles.objects_follow, "Doodads and WMOs follow the ground");
            }
            Kind::Blends => {
                theme::note(ui, "changes how each chunk's textures blend; adds no texture");
            }
        }

        ui.add_space(6.0);
        let ready = open > 0 && (pending.kind == Kind::Blends || pending.high > pending.low);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(ready, egui::Button::new("Import"))
                .on_disabled_hover_text(match open {
                    0 => "None of the selection is open.",
                    _ => "White must be higher than black.",
                })
                .clicked()
            {
                asked = Some(Asked::Import);
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if close || response.should_close() {
        tiles.pending = None;
    }
    asked
}

/// What is drawn over a cell's picture.
struct Marks<'a> {
    selected: bool,
    /// This project carries the tile — see [`MapView::edited`].
    in_project: bool,
    /// The project has changed the tile since the last save.
    unsaved: bool,
    /// The claim on the tile, checked out or held by unsubmitted edits.
    claim: Option<&'a TileClaim>,
    /// Whether the cell is large enough for its edge to be drawn.
    edged: bool,
    /// Whether the cell is large enough on screen for the sharper picture.
    sharp: bool,
}

/// One cell: its picture if it has one, its state if it does not, and the
/// tints and the corner mark over it. The regions' outlines are drawn after
/// every cell, by the caller.
fn paint_cell(
    painter: &egui::Painter,
    square: egui::Rect,
    at: (u32, u32),
    session: &EditSession,
    thumbnails: &mut Thumbnails,
    minimaps: &vale_assets::tables::minimap::MinimapTiles,
    marks: Marks,
) {
    if session.wdt_claims(at) {
        // The tile's minimap picture, which is what makes this a map rather
        // than a checkerboard.
        match minimaps.texture(&session.map, at.0, at.1) {
            Some(path) => {
                thumbnails.want(&path);
                let mut picture = thumbnails.get(&path);
                if marks.sharp {
                    thumbnails.want_at(&path, SHARP_SIDE);
                    picture = thumbnails.get_at(&path, SHARP_SIDE).or(picture);
                }
                match picture {
                    Some(id) => {
                        painter.image(
                            id,
                            square,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    // Asked for and not decoded yet: the "exists" fill, so the
                    // grid reads correctly while it fills in.
                    None => {
                        painter.rect_filled(square, 0.0, theme::RAISED);
                    }
                }
            }
            // Claimed with no picture in the index — a tile this editor made.
            None => {
                painter.rect_filled(square, 0.0, theme::ACCENT_SUNK);
            }
        }
    }
    if marks.edged {
        painter.rect_stroke(
            square,
            0.0,
            egui::Stroke::new(1.0, theme::LINE.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(claim) = marks.claim {
        let [r, g, b] = claim.colour;
        let strength = match claim.checked_out {
            true => 0.22,
            false => 0.12,
        };
        painter.rect_filled(square, 0.0, egui::Color32::from_rgb(r, g, b).gamma_multiply(strength));
    }
    if marks.selected {
        painter.rect_filled(square, 0.0, theme::ACCENT.gamma_multiply(0.25));
    }
    if marks.in_project {
        corner_mark(
            painter,
            square,
            match marks.unsaved {
                true => theme::WARN,
                false => theme::ACCENT,
            },
        );
    }
}

/// A triangle in a cell's top-right corner, small enough that the picture
/// stays readable under it.
fn corner_mark(painter: &egui::Painter, square: egui::Rect, colour: egui::Color32) {
    let size = (square.width() * 0.35).clamp(3.0, 9.0);
    let corner = square.right_top();
    painter.add(egui::Shape::convex_polygon(
        vec![corner, egui::pos2(corner.x - size, corner.y), egui::pos2(corner.x, corner.y + size)],
        colour,
        egui::Stroke::NONE,
    ));
}

/// Draw the sides of `square` that `sides` marks, in the order up, right,
/// down, left, solid or dashed. Each line is inset by half its width, so the
/// outlines of two regions that meet lie side by side instead of over each
/// other.
fn paint_sides(painter: &egui::Painter, square: egui::Rect, sides: [bool; 4], stroke: egui::Stroke, dashed: bool) {
    let inset = square.shrink(stroke.width * 0.5);
    let corners = [inset.left_top(), inset.right_top(), inset.right_bottom(), inset.left_bottom()];
    for (side, on) in sides.into_iter().enumerate() {
        if !on {
            continue;
        }
        let ends = [corners[side], corners[(side + 1) % 4]];
        match dashed {
            true => painter.extend(egui::Shape::dashed_line(&ends, stroke, 4.0, 3.0)),
            false => {
                painter.line_segment(ends, stroke);
            }
        }
    }
}

/// A padlock in the middle of a cell, on a dark disc so it reads over any
/// picture: the mark of a tile another author holds.
fn padlock(painter: &egui::Painter, square: egui::Rect, colour: egui::Color32) {
    let size = (square.width().min(square.height()) * 0.3).clamp(4.0, 10.0);
    let centre = square.center();
    painter.circle_filled(centre, size * 1.15, theme::SHELL.gamma_multiply(0.8));
    let body = egui::Rect::from_center_size(centre + egui::vec2(0.0, size * 0.25), egui::vec2(size * 1.1, size * 0.8));
    painter.rect_filled(body, 1.0, colour);
    // The shackle: a half ring standing on the body.
    let radius = size * 0.35;
    let foot = egui::pos2(centre.x, body.top());
    let ring: Vec<egui::Pos2> = (0..=8)
        .map(|i| {
            let angle = std::f32::consts::PI * (1.0 + i as f32 / 8.0);
            foot + egui::vec2(angle.cos() * radius, angle.sin() * radius)
        })
        .collect();
    painter.add(egui::Shape::line(ring, egui::Stroke::new((size * 0.18).max(1.0), colour)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selecting_one_replaces_the_rest() {
        let mut view = MapView::default();
        view.selection.insert((1, 1));
        view.selection.insert((2, 2));
        view.select_only((5, 5));
        assert_eq!(view.selection.len(), 1);
        assert!(view.selection.contains(&(5, 5)));
    }

    /// A fit centres the tiles it frames, at a whole number of points a tile,
    /// so cell boundaries land on pixels.
    #[test]
    fn a_fit_centres_its_tiles_at_whole_points() {
        let room = egui::vec2(400.0, 300.0);
        let (cell, origin) = framing((30, 40), (39, 49), room, FIT_MAX);
        assert_eq!(cell, 30.0);
        let centre = origin + room / (2.0 * cell);
        assert_eq!(centre, egui::vec2(35.0, 45.0));
    }

    /// A fit stays inside the zoom's range: one tile is not drawn larger than
    /// the fit's limit, and a small room does not draw cells too small to
    /// click.
    #[test]
    fn a_fit_stays_in_the_zoom_range() {
        assert_eq!(framing((5, 5), (5, 5), egui::vec2(600.0, 600.0), FIT_MAX).0, FIT_MAX);
        assert_eq!(framing((0, 0), (63, 63), egui::vec2(100.0, 100.0), FIT_MAX).0, MIN_CELL);
    }

    /// The whole map fits a 640-point window, so Whole Map shows all 64x64
    /// tiles without moving the map.
    #[test]
    fn the_whole_map_fits_the_window() {
        let (cell, _) = framing((0, 0), (SIDE - 1, SIDE - 1), egui::vec2(640.0 - PANEL_WIDTH - RULER, 560.0), MAX_CELL);
        assert!(cell * SIDE as f32 <= 640.0 - PANEL_WIDTH - RULER);
        assert!(cell >= MIN_CELL);
    }

    /// Zooming keeps the tile under the pointer under the pointer.
    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let mut view = MapView { cell: 10.0, origin: egui::vec2(20.0, 20.0), ..MapView::default() };
        let area = egui::Rect::from_min_size(egui::pos2(16.0, 16.0), egui::vec2(400.0, 400.0));
        let about = egui::pos2(116.0, 66.0);
        let under = |view: &MapView| view.origin + (about - area.min) / view.cell;
        let before = under(&view);
        view.zoom_about(20.0, about, area);
        assert_eq!(view.cell, 20.0);
        assert_eq!(under(&view), before);
        view.zoom_about(1000.0, about, area);
        assert_eq!(view.cell, MAX_CELL);
    }

    /// A region's boundary is the sides whose neighbour is outside it.
    #[test]
    fn a_region_is_outlined_on_its_outside_sides() {
        let region = [(1u32, 1u32), (2, 1)];
        let inside = |x: i64, y: i64| x >= 0 && y >= 0 && region.contains(&(x as u32, y as u32));
        assert_eq!(boundary((1, 1), inside), [true, false, true, true]);
        assert_eq!(boundary((2, 1), inside), [true, true, true, false]);
        // A tile on the grid's edge has its neighbour off the grid.
        assert_eq!(boundary((0, 0), |x, y| x >= 0 && y >= 0 && (x, y) == (0, 0)), [true; 4]);
    }

    /// Release takes this author's check outs in the selection and Break
    /// another author's; neither takes a claim held only by edits.
    #[test]
    fn release_and_break_take_their_own_check_outs() {
        let claim = |mine: bool, checked_out: bool| TileClaim {
            author: [mine as u8; 16],
            who: "Sam".into(),
            mine,
            checked_out,
            edited: !checked_out,
            colour: [0, 0, 0],
        };
        let mut view = MapView::default();
        view.claims.tiles.insert((1, 0), claim(true, true));
        view.claims.tiles.insert((2, 0), claim(false, true));
        view.claims.tiles.insert((3, 0), claim(false, false));
        view.claims.tiles.insert((4, 0), claim(true, false));
        view.selection.extend([(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]);
        assert_eq!(view.releasable(), vec![(1, 0)]);
        assert_eq!(view.breakable(), vec![(2, 0)]);
    }
}
