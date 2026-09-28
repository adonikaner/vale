//! The map from above: which tiles exist, and the ones you are about to change.
//!
//! ## Why tiles are chosen on a map
//!
//! An earlier version of the tile tool acted on the tile under the pointer,
//! and that could not create a tile. `crate::pick::Cursor`'s `ground` is
//! `None` "off the map, over a hole, or over a tile that is not open", so over
//! a tile that does not exist there is no pointer position, the panel had no
//! tile to name, and **Create** could never run.
//!
//! It also required the user to fly to where no tile exists, in a viewport
//! that draws nothing there: no landmark, no ground, and no way to tell 33,48
//! from 34,48 except the status line.
//!
//! A map has 64x64 tiles, and choosing among them needs them laid out. This
//! window is a grid of 4,096 cells, each showing that tile's minimap picture
//! from the archives: a top-down render of the world, and the only existing
//! picture of a tile. Tiles that do not exist are drawn as holes.
//!
//! ## All tile operations are in this window
//!
//! Creating ground, deleting it, copying it, and the three files derived from
//! a tile are all here. They were first a **Tiles** row on the rail with its
//! own panel, which was wrong for two reasons: the rail lists what the pointer
//! edits, and none of these operations uses the pointer; and the split left
//! the map choosing the tiles while a panel elsewhere acted on them. After a
//! report from the window, the operations were moved to where the selection
//! is. The rail lost the row and the top bar gained a button, because which
//! tiles this map has is a fact about the session, and the top bar holds
//! session facts.
//!
//! ## Every operation works on the selection
//!
//! Click, box-drag, or ctrl-click to toggle. Every operation then applies to
//! the whole selection, so creating nine tiles or deleting a row is one action
//! rather than nine. The operations themselves are in
//! [`crate::tools::tiles`].
//!
//! ## The fit zoom frames the map's existing tiles
//!
//! Azeroth claims about 700 of the 4,096 slots, in a region twenty tiles wide.
//! Drawn as the whole grid, that region was a small patch in the middle of a
//! window of cells that looked like background, and nothing on screen
//! identified the rows. The **fit** zoom, which the window opens on, frames
//! the claimed tiles with one empty ring around them, at whatever cell size
//! fills the window. **far**, **mid** and **near** show the whole grid at
//! three sizes, for making ground where the map has none. Rulers along the top
//! and left name the columns and rows at every zoom, and at every zoom except
//! the smallest each cell has its edge drawn.
//!
//! ## What a cell shows
//!
//! Its picture, when the archives have one; a plain fill for a tile that
//! exists without a picture, which is a tile this editor made; a sunken square
//! for an empty slot. Over that, a corner mark on a tile this project carries
//! (edited and saved, or made), and a dot on one edited since the last save,
//! so the map shows where the work is.
//!
//! ## The pictures use the texture picker's thumbnail cache
//!
//! A minimap tile is a BLP at a path, and [`super::thumbnails`] already
//! decodes BLPs by path, a few per frame, with a cache. The grid fills in over
//! about three seconds on a map with seven hundred tiles and costs nothing on
//! later frames, with the same behaviour and code as the tileset list.

use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy_egui::egui;

use super::theme;
use super::thumbnails::Thumbnails;
use crate::session::EditSession;

/// Tiles across a map, in each axis.
const SIDE: u32 = 64;

/// How big a cell is, in points, at each of the three whole-grid zooms.
///
/// Three fixed sizes rather than a slider: the whole map, a quarter of it, and
/// close enough to read a coastline. A continuous zoom on a 4,096-cell grid is
/// hard to set precisely and keeps changing the layout. The fit zoom is index
/// 0 and has no fixed size; see [`frame`].
const ZOOMS: [f32; 4] = [0.0, 8.0, 14.0, 22.0];

/// The smallest and largest cell the fit zoom will draw, in points. Eight is
/// the smallest a cell can be clicked; past forty the pictures blur.
const FIT_CELL: (f32, f32) = (8.0, 40.0);

/// How wide the zoom control is, in points.
///
/// A fixed width rather than the rest of the row, because `theme::segmented`
/// fills whatever width it is given: at the end of a row it drew a control as
/// wide as the window.
const ZOOM_WIDTH: f32 = 190.0;

/// The rulers' width, top and left, in points.
const RULER: f32 = 16.0;

/// What the map window is holding.
#[derive(Resource, Default)]
pub struct MapView {
    pub open: bool,
    /// The tiles every operation applies to.
    pub selection: HashSet<(u32, u32)>,
    /// Where a box-drag began, while one is in progress.
    drag_from: Option<(u32, u32)>,
    /// The tiles the last **Copy** took, in the coordinates they were taken
    /// from.
    ///
    /// A set rather than one tile, so a region can be moved as a region. What
    /// **Paste** does with it depends on how many tiles it holds:
    ///
    /// * one tile fills every selected slot, which lays the same ground over,
    ///   for example, a nine-tile square;
    /// * several tiles are pasted as a block, keeping their relative layout,
    ///   with the copied region's corner placed on the selection's corner.
    ///   Any other rule would need a second gesture to say which copied tile
    ///   goes where.
    pub clipboard: Vec<(u32, u32)>,
    zoom: usize,
    /// Select the camera's tile on the first frame the window is drawn.
    ///
    /// Set by [`MapView::opened`], which `--tiles` uses. The window's foot
    /// panel appears only while something is selected, so without this
    /// `--shot` could not capture the half of the window that holds the
    /// settings. It is also a sensible default for a user, who is most likely
    /// to act on the tile the camera stands on.
    preselect: bool,
    /// Where the view is scrolled to, as the top-left tile.
    scroll: egui::Vec2,
    /// The tiles this project carries, read from the project folder. Reading
    /// walks the folder, so the result is kept and re-read only when
    /// [`Self::edited_stale`] is set.
    edited: HashSet<(u32, u32)>,
    /// Set by whatever changes the project's tiles: the window opening, and
    /// every operation that makes, deletes or pastes one.
    pub edited_stale: bool,
    /// Whether the window was open on the last frame, for the re-read above.
    was_open: bool,
}

impl MapView {
    /// A map view that starts open, for `--tiles`, which is how the window is
    /// captured in screenshots.
    pub fn opened() -> MapView {
        MapView {
            open: true,
            preselect: true,
            ..MapView::default()
        }
    }

    /// Select exactly these.
    pub fn select_only(&mut self, at: (u32, u32)) {
        self.selection.clear();
        self.selection.insert(at);
    }

    /// How many of the selection exist, and how many do not.
    pub fn split(&self, session: &EditSession) -> (usize, usize) {
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
            if let Some((map, x, y)) = crate::server::datadir::parse_tile_vpath(&vpath) {
                if map.eq_ignore_ascii_case(&session.map) {
                    self.edited.insert((x, y));
                }
            }
        }
        self.edited_stale = false;
    }
}

/// What the window asked for, handed back to the caller to act on.
///
/// The window performs nothing itself: it holds the session by shared
/// reference, and every operation needs it mutably and also needs the
/// archives. `tools::tiles` uses the same arrangement one layer down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    Create,
    Delete,
    Copy,
    Paste,
    FlyTo((u32, u32)),
    /// Rebake the selection's `MCSH` from what stands on it.
    Rebake,
    /// Redraw the selection's minimap picture.
    Minimap,
    /// Regenerate the server's files for the selection: `maps`, `vmaps`,
    /// `mmaps`.
    ServerFiles,
}

/// The tiles a zoom draws: the top-left tile, how many across and down, and
/// the cell size.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame {
    origin: (u32, u32),
    size: (u32, u32),
    cell: f32,
}

impl Frame {
    #[cfg(test)]
    fn contains(&self, at: (u32, u32)) -> bool {
        at.0 >= self.origin.0
            && at.1 >= self.origin.1
            && at.0 < self.origin.0 + self.size.0
            && at.1 < self.origin.1 + self.size.1
    }
}

/// The frame for a zoom, given the room the grid has.
///
/// The fit zoom takes the bounding box of the claimed tiles and the
/// selection, with one empty ring around it, at the largest whole cell size
/// that fits the room. The map therefore opens on the tiles it has rather than
/// on 4,096 slots, most of them empty. A map with no tiles fits the whole
/// grid.
fn frame(zoom: usize, session: &EditSession, selection: &HashSet<(u32, u32)>, room: egui::Vec2) -> Frame {
    if zoom > 0 {
        return Frame {
            origin: (0, 0),
            size: (SIDE, SIDE),
            cell: ZOOMS[zoom.min(ZOOMS.len() - 1)],
        };
    }
    let mut lo = (SIDE, SIDE);
    let mut hi = (0u32, 0u32);
    for &(x, y) in session.claimed.iter().chain(selection.iter()) {
        lo = (lo.0.min(x), lo.1.min(y));
        hi = (hi.0.max(x), hi.1.max(y));
    }
    if lo.0 > hi.0 {
        return Frame {
            origin: (0, 0),
            size: (SIDE, SIDE),
            cell: FIT_CELL.0,
        };
    }
    let origin = (lo.0.saturating_sub(1), lo.1.saturating_sub(1));
    let end = ((hi.0 + 2).min(SIDE), (hi.1 + 2).min(SIDE));
    let size = (end.0 - origin.0, end.1 - origin.1);
    let cell = ((room.x - RULER) / size.0 as f32)
        .min((room.y - RULER) / size.1 as f32)
        .floor()
        .clamp(FIT_CELL.0, FIT_CELL.1);
    Frame { origin, size, cell }
}

/// Draw the window. Returns its rectangle, so the shell can keep clicks in it
/// out of the world, and whatever was asked for.
pub fn draw(
    ctx: &egui::Context,
    view: &mut MapView,
    session: &EditSession,
    thumbnails: &mut Thumbnails,
    minimaps: &vale_assets::tables::minimap::MinimapTiles,
    tiles: &mut crate::tools::tiles::Tiles,
    camera_tile: (u32, u32),
) -> (Option<egui::Rect>, Option<Asked>) {
    if !view.open {
        view.was_open = false;
        return (None, None);
    }
    if !view.was_open {
        view.was_open = true;
        view.edited_stale = true;
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

    let response = egui::Window::new(format!("Map \u{2014} {}", session.map))
        .id(egui::Id::new("map-window"))
        .open(&mut open)
        // Within a 720-point screen under the bar: a window taller than the
        // screen is moved up over the bar to fit, and then covers the button
        // that opened it.
        .default_size([640.0, 650.0])
        // Below the top bar, which holds the button that opens the window. A
        // window covering that button hides the control that closes it.
        .default_pos([40.0, 52.0])
        .resizable(true)
        .frame(
            egui::Frame::default()
                .fill(theme::SHELL)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ctx, |ui| {
            asked = contents(ui, view, session, thumbnails, minimaps, tiles, camera_tile);
        });
    view.open = open;

    (response.map(|r| r.response.rect), asked)
}

fn contents(
    ui: &mut egui::Ui,
    view: &mut MapView,
    session: &EditSession,
    thumbnails: &mut Thumbnails,
    minimaps: &vale_assets::tables::minimap::MinimapTiles,
    tiles: &mut crate::tools::tiles::Tiles,
    camera_tile: (u32, u32),
) -> Option<Asked> {
    let mut asked = None;

    // ------------------------------------------------------------- the bar
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(selection_line(view, session)).color(
            match view.selection.is_empty() {
                true => theme::INK_FAINT,
                false => theme::INK,
            },
        ));
        // A fixed width rather than the rest of the row, laid out left to
        // right. `theme::segmented` sizes itself from `available_width` and
        // lays its segments out in the row's direction: at the end of a
        // right-to-left row it was as wide as the window and read
        // `near mid far fit`.
        ui.add_space(12.0);
        ui.scope(|ui| {
            ui.set_max_width(ZOOM_WIDTH);
            let mut zoom = view.zoom;
            theme::segmented(
                ui,
                &mut zoom,
                &[("fit", 0usize), ("far", 1), ("mid", 2), ("near", 3)],
                |a, b| a == b,
            );
            view.zoom = zoom;
        });
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        // **Create** is enabled by empty slots in the selection and **Delete**
        // by existing tiles, so each button is enabled only when it has
        // something to act on, and neither silently does nothing.
        let (exist, absent) = view.split(session);
        if ui
            .add_enabled(absent > 0, egui::Button::new(format!("Create {absent}")))
            .on_hover_text("Claims each selected empty tile in the WDT and writes flat ground.")
            .clicked()
        {
            asked = Some(Asked::Create);
        }
        if ui
            .add_enabled(exist > 0, egui::Button::new(format!("Delete {exist}")))
            .on_hover_text(
                "Clears the WDT bit. The ADT stays in the project, so this is reversible.",
            )
            .clicked()
        {
            asked = Some(Asked::Delete);
        }
        ui.separator();
        if ui
            .add_enabled(exist > 0, egui::Button::new(format!("Copy {exist}")))
            .on_hover_text(
                "Takes every selected tile that exists. One pastes into every slot; several \
                 paste as a block, keeping their layout.",
            )
            .clicked()
        {
            asked = Some(Asked::Copy);
        }
        let holding = view.clipboard.len();
        if ui
            .add_enabled(
                holding > 0 && !view.selection.is_empty(),
                egui::Button::new("Paste"),
            )
            .on_hover_text(
                "Rewrites each copied tile to sit where it is going. Its ground, its \
                 doodads and its buildings all move with it.",
            )
            .clicked()
        {
            asked = Some(Asked::Paste);
        }
        match view.clipboard.len() {
            0 => {}
            1 => {
                let at = view.clipboard[0];
                theme::note(ui, format!("holding {}, {}", at.0, at.1));
            }
            n => theme::note(ui, format!("holding {n} tiles")),
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(!view.selection.is_empty(), egui::Button::new("Clear"))
                .on_hover_text("Selects nothing.")
                .clicked()
            {
                view.selection.clear();
            }
        });
    });

    ui.add_space(4.0);
    theme::note(
        ui,
        "Click a tile, drag a box, ctrl-click to add. Double-click to fly there.",
    );
    ui.add_space(4.0);

    // ------------------------------------------------------------- the foot
    // The foot is laid out first, from the bottom, and the grid gets the
    // remaining height. The foot was previously drawn after the grid with a
    // constant height reserved for it, and the constant was smaller than
    // three rows and a status line: the window's content overran its frame by
    // the difference, egui grew the window by that much, the grid (sized to
    // the available height) grew with it, and the next frame overran again.
    // Selecting a tile then grew the window to the bottom of the screen at
    // every zoom. A bottom panel inside the window takes exactly the height
    // its rows need and leaves the rest to the grid.
    if !view.selection.is_empty() {
        egui::Panel::bottom("map-foot")
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                ui.separator();
                if let Some(more) = foot_panel(ui, view, session, tiles) {
                    asked = Some(more);
                }
            });
    }

    // ------------------------------------------------------------ the grid
    let room = egui::vec2(
        ui.available_width(),
        ui.available_height().max(FIT_CELL.0 * 8.0),
    );
    let frame = frame(view.zoom, session, &view.selection, room);
    let cell = frame.cell;
    let full = egui::vec2(
        RULER + cell * frame.size.0 as f32,
        RULER + cell * frame.size.1 as f32,
    );
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .max_height(room.y)
        .show(ui, |ui| {
            let (whole, response) = ui.allocate_exact_size(full, egui::Sense::click_and_drag());
            let rect = egui::Rect::from_min_max(whole.min + egui::vec2(RULER, RULER), whole.max);
            view.scroll = rect.min.to_vec2();
            let painter = ui.painter_at(whole);
            painter.rect_filled(rect, 0.0, theme::SUNK);

            // Draw only the cells on screen: all of them at the far zoom, a
            // few hundred at the near zoom. Asking for four thousand pictures
            // a frame is the load the budget in `thumbnails` exists to bound.
            let clip = ui.clip_rect();
            let first = |v: f32, min: f32| (((v - min) / cell).floor().max(0.0)) as u32;
            let last = |v: f32, min: f32, n: u32| (((v - min) / cell).ceil().min(n as f32)) as u32;
            let (x0, x1) = (first(clip.min.x, rect.min.x), last(clip.max.x, rect.min.x, frame.size.0));
            let (y0, y1) = (first(clip.min.y, rect.min.y), last(clip.max.y, rect.min.y, frame.size.1));
            let edged = cell >= ZOOMS[2];

            for row in y0..y1 {
                for col in x0..x1 {
                    let at = (frame.origin.0 + col, frame.origin.1 + row);
                    let square = egui::Rect::from_min_size(
                        rect.min + egui::vec2(col as f32 * cell, row as f32 * cell),
                        egui::vec2(cell, cell),
                    );
                    paint_cell(
                        &painter,
                        square,
                        at,
                        session,
                        thumbnails,
                        minimaps,
                        Marks {
                            selected: view.selection.contains(&at),
                            camera: at == camera_tile,
                            in_project: view.in_project(session, at),
                            unsaved: session.unsaved.contains(&at),
                            edged,
                        },
                    );
                }
            }
            rulers(&painter, whole, &frame, x0..x1, y0..y1);

            // ------------------------------------------------- the pointer
            let of = |p: egui::Pos2| -> Option<(u32, u32)> {
                if !rect.contains(p) {
                    return None;
                }
                let col = ((p.x - rect.min.x) / cell) as u32;
                let row = ((p.y - rect.min.y) / cell) as u32;
                (col < frame.size.0 && row < frame.size.1)
                    .then_some((frame.origin.0 + col, frame.origin.1 + row))
            };

            if response.drag_started() {
                view.drag_from = response.interact_pointer_pos().and_then(of);
            }
            if response.dragged() {
                // The box is recomputed from both ends every frame rather than
                // accumulated, so dragging back shrinks the selection instead
                // of leaving a trail.
                if let (Some(from), Some(to)) =
                    (view.drag_from, response.interact_pointer_pos().and_then(of))
                {
                    let additive = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
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
            if response.drag_stopped() {
                view.drag_from = None;
            }

            if response.clicked() {
                if let Some(at) = response.interact_pointer_pos().and_then(of) {
                    let additive = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
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

            // Name the tile under the pointer in a hover text: a cell can be
            // eight points across, and nothing else identifies it.
            if let Some(at) = response.hover_pos().and_then(of) {
                response.clone().on_hover_text(describe(view, session, at));
            }
        });

    asked
}

/// The bar's first text: what is selected, as the counts the operations act on
/// (existing, empty, in this project).
fn selection_line(view: &MapView, session: &EditSession) -> String {
    let (exist, absent) = view.split(session);
    let mine = view
        .selection
        .iter()
        .filter(|&&at| view.in_project(session, at))
        .count();
    match view.selection.len() {
        0 => "nothing selected".to_string(),
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

/// `exists`, `empty`, `in this project`, `unsaved` — one tile's state as words.
fn describe_state(view: &MapView, session: &EditSession, at: (u32, u32)) -> String {
    let mut parts: Vec<&str> = Vec::new();
    parts.push(match session.wdt_claims(at) {
        true => "exists",
        false => "empty",
    });
    if session.unsaved.contains(&at) {
        parts.push("unsaved");
    } else if view.in_project(session, at) {
        parts.push("in this project");
    }
    if crate::tools::tiles::is_open(session, at) {
        parts.push("open");
    }
    parts.join(", ")
}

/// The hover text for a cell.
fn describe(view: &MapView, session: &EditSession, at: (u32, u32)) -> String {
    format!("{}, {}\n{}", at.0, at.1, describe_state(view, session, at))
}

/// The column and row numbers, along the top and the left, every cell at
/// the near zoom and every few at the others, with a tick per cell.
fn rulers(
    painter: &egui::Painter,
    whole: egui::Rect,
    frame: &Frame,
    cols: std::ops::Range<u32>,
    rows: std::ops::Range<u32>,
) {
    let cell = frame.cell;
    let every = match cell {
        c if c >= 20.0 => 1,
        c if c >= 12.0 => 2,
        _ => 4,
    };
    let font = egui::FontId::proportional(theme::SMALL);
    let top = egui::Rect::from_min_size(whole.min, egui::vec2(whole.width(), RULER));
    let left = egui::Rect::from_min_size(whole.min, egui::vec2(RULER, whole.height()));
    painter.rect_filled(top, 0.0, theme::PANEL);
    painter.rect_filled(left, 0.0, theme::PANEL);
    for col in cols {
        let x = frame.origin.0 + col;
        let px = whole.min.x + RULER + col as f32 * cell;
        painter.line_segment(
            [egui::pos2(px, whole.min.y + RULER - 3.0), egui::pos2(px, whole.min.y + RULER)],
            egui::Stroke::new(1.0, theme::LINE),
        );
        if x % every == 0 {
            painter.text(
                egui::pos2(px + cell * 0.5, whole.min.y + RULER * 0.5 - 1.0),
                egui::Align2::CENTER_CENTER,
                x.to_string(),
                font.clone(),
                theme::INK_DIM,
            );
        }
    }
    for row in rows {
        let y = frame.origin.1 + row;
        let py = whole.min.y + RULER + row as f32 * cell;
        painter.line_segment(
            [egui::pos2(whole.min.x + RULER - 3.0, py), egui::pos2(whole.min.x + RULER, py)],
            egui::Stroke::new(1.0, theme::LINE),
        );
        if y % every == 0 {
            painter.text(
                egui::pos2(whole.min.x + RULER * 0.5, py + cell * 0.5),
                egui::Align2::CENTER_CENTER,
                y.to_string(),
                font.clone(),
                theme::INK_DIM,
            );
        }
    }
}

/// What new ground in the selection is made of, and the three files derived
/// from each selected tile.
///
/// Drawn under the grid rather than beside it, and only while something is
/// selected, because every control here acts on the selection.
fn foot_panel(
    ui: &mut egui::Ui,
    view: &MapView,
    session: &EditSession,
    tiles: &mut crate::tools::tiles::Tiles,
) -> Option<Asked> {
    use crate::tools::tiles as tool;

    let mut asked = None;
    let (exist, absent) = view.split(session);
    let label = |ui: &mut egui::Ui, text: &str| {
        ui.add_sized(
            [64.0, 18.0],
            egui::Label::new(
                egui::RichText::new(text)
                    .size(theme::SMALL)
                    .color(theme::INK_DIM),
            ),
        );
    };

    // What new ground is made of, which only matters while there is some to
    // make. `Create` itself is on the toolbar with the other operations.
    ui.horizontal(|ui| {
        label(ui, "New ground");
        ui.add_enabled_ui(absent > 0, |ui| {
            ui.add(
                egui::DragValue::new(&mut tiles.height)
                    .speed(1.0)
                    .suffix(" yd")
                    .prefix("height "),
            )
            .on_hover_text("How high the flat ground of a new tile sits.");
            ui.add(
                egui::DragValue::new(&mut tiles.area)
                    .speed(1.0)
                    .prefix("area "),
            )
            .on_hover_text("The AreaTable id every chunk of a new tile is given. 0 is none.");
            ui.add(
                egui::TextEdit::singleline(&mut tiles.texture)
                    .desired_width(220.0)
                    .hint_text("Tileset\\…"),
            )
            .on_hover_text("The one texture a new tile is painted with.");
        });
        if absent == 0 {
            theme::note(ui, "every selected tile exists");
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        // Both picture operations need the tile open, and the streamer opens
        // only the 3x3 tiles around the camera. The buttons count the open
        // selected tiles and are disabled when there are none, rather than
        // failing after a minute.
        let open = view
            .selection
            .iter()
            .filter(|&&at| tool::is_open(session, at))
            .count();
        label(ui, "Pictures");
        if ui
            .add_enabled(
                open > 0,
                egui::Button::new(format!("Rebake shadows ({open})")),
            )
            .on_hover_text(format!(
                "Recomputes MCSH from what stands on each open selected tile. About {}s \
                 a tile, on a background thread; the editor stays usable.",
                tool::REBAKE_SECONDS
            ))
            .on_disabled_hover_text("None of the selection is open: fly to a tile to open it.")
            .clicked()
        {
            asked = Some(Asked::Rebake);
        }
        if ui
            .add_enabled(
                open > 0,
                egui::Button::new(format!("Redraw minimap ({open})")),
            )
            .on_hover_text(
                "Draws each open selected tile's 256x256 picture from its own layers, \
                 light, shadow and water, and writes it into the project as DXT1.",
            )
            .on_disabled_hover_text("None of the selection is open: fly to a tile to open it.")
            .clicked()
        {
            asked = Some(Asked::Minimap);
        }
        ui.checkbox(&mut tiles.cast_from_ground, "shadow from the ground too")
            .on_hover_text(
                "Also cast shadow from the terrain onto itself. Off by default: \
                 measured against two shipped tiles, hull-cast shadow matches \
                 their MCSH at 1.2-1.4x the base rate and terrain-cast shadow at \
                 0.4-0.9x, which is chance or worse, so the shipped bakes do not \
                 appear to include it.",
            );
        if open == 0 {
            theme::note(ui, "none open: fly to it first");
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        label(ui, "Server");
        if ui
            .add_enabled(exist > 0, egui::Button::new(format!("Server files ({exist})")))
            .on_hover_text(
                "Save, rebuild this project's archive, and regenerate the server's maps, \
                 vmaps and mmaps for each selected tile that exists — from this project \
                 where it carries the tile, from the archives where it does not. Minutes: \
                 a navmesh tile and its four neighbours, and the map's vmaps when a \
                 building moved. The bar says which step is running; restart the server \
                 afterwards. Publish does this for every changed tile.",
            )
            .on_disabled_hover_text("None of the selection exists.")
            .clicked()
        {
            asked = Some(Asked::ServerFiles);
        }
    });

    if !tiles.said.is_empty() {
        ui.add_space(2.0);
        theme::note(ui, tiles.said.clone());
    }
    asked
}

/// What is drawn over a cell's picture.
struct Marks {
    selected: bool,
    camera: bool,
    /// This project carries the tile — see [`MapView::edited`].
    in_project: bool,
    /// The project has changed the tile since the last save.
    unsaved: bool,
    /// Whether the cell is large enough for its edge to be drawn.
    edged: bool,
}

/// One cell: its picture if it has one, its state if it does not.
fn paint_cell(
    painter: &egui::Painter,
    square: egui::Rect,
    at: (u32, u32),
    session: &EditSession,
    thumbnails: &mut Thumbnails,
    minimaps: &vale_assets::tables::minimap::MinimapTiles,
    marks: Marks,
) {
    let exists = session.wdt_claims(at);
    let inner = square.shrink(0.5);

    if exists {
        // The tile's minimap picture, which is what makes this a map rather
        // than a checkerboard.
        match minimaps.texture(&session.map, at.0, at.1) {
            Some(path) => {
                thumbnails.want(&path);
                match thumbnails.get(&path) {
                    Some(id) => painter.image(
                        id,
                        inner,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    ),
                    // Asked for and not decoded yet: the "exists" fill, so the
                    // grid reads correctly while it fills in.
                    None => painter.rect_filled(inner, 0.0, theme::RAISED),
                };
            }
            // Claimed with no picture in the index — a tile this editor made.
            None => {
                painter.rect_filled(inner, 0.0, theme::ACCENT_SUNK);
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
    if marks.in_project {
        // A corner, so the picture stays readable under it.
        let size = (square.width() * 0.35).clamp(3.0, 9.0);
        let corner = square.right_top();
        painter.add(egui::Shape::convex_polygon(
            vec![
                corner,
                egui::pos2(corner.x - size, corner.y),
                egui::pos2(corner.x, corner.y + size),
            ],
            match marks.unsaved {
                true => theme::WARN,
                false => theme::ACCENT,
            },
            egui::Stroke::NONE,
        ));
    }

    if marks.selected {
        painter.rect_stroke(
            inner,
            0.0,
            egui::Stroke::new(1.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
        painter.rect_filled(inner, 0.0, theme::ACCENT.gamma_multiply(0.25));
    }
    if marks.camera {
        painter.rect_stroke(
            square,
            0.0,
            egui::Stroke::new(2.0, theme::GOOD),
            egui::StrokeKind::Inside,
        );
    }
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

    /// Every zoom is a whole number of points, so a cell boundary lands on a
    /// pixel and the grid does not shimmer as it scrolls.
    #[test]
    fn the_zooms_are_whole_points() {
        for zoom in &ZOOMS[1..] {
            assert_eq!(*zoom, zoom.floor());
            assert!(*zoom >= 8.0, "a cell smaller than this cannot be clicked");
        }
        assert_eq!(FIT_CELL.0, FIT_CELL.0.floor());
    }

    /// The whole map fits a 640-point window at the far zoom, so all 64x64
    /// tiles are visible without scrolling.
    #[test]
    fn the_whole_map_fits_at_the_far_zoom() {
        assert!(RULER + ZOOMS[1] * SIDE as f32 <= 640.0);
    }

    /// A frame's cells are the tiles it names and no others.
    #[test]
    fn a_frame_holds_its_own_tiles() {
        let frame = Frame {
            origin: (30, 40),
            size: (4, 3),
            cell: 20.0,
        };
        assert!(frame.contains((30, 40)));
        assert!(frame.contains((33, 42)));
        assert!(!frame.contains((34, 42)));
        assert!(!frame.contains((29, 40)));
    }
}
