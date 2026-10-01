//! A selection of map chunks, and the operations that act on every chunk in
//! it as one undo entry.
//!
//! ## What the tool is for
//!
//! The area, texture and hole tools each act on the chunk under the pointer or
//! on a brush's reach. An area id over a district, a base texture over a
//! valley floor or the impassable flag over a ridge is the same value written
//! to many chunks, and painting it chunk by chunk is slow and leaves gaps. This
//! tool holds a set of chunks and writes one value to all of them.
//!
//! ## The rules
//!
//! The placement tools' rules (`super::group`), with the one difference a grid
//! allows: the rectangle is on the ground and not on the screen.
//!
//! ```text
//! click                selects the chunk under the pointer and drops the rest
//! shift + click        adds the chunk to the selection, or takes it out
//! drag                 a block: every chunk between the chunk the press was on
//!                      and the chunk under the pointer, as a rectangle of the
//!                      chunk grid. With shift it is added to the selection
//! ctrl + a             every chunk of the tile under the pointer; with shift,
//!                      added
//! escape               drops the selection
//! ```
//!
//! A screen rectangle selects by what is drawn inside it, which for chunks seen
//! at a shallow pitch is a wedge reaching to the horizon. The chunk grid is
//! regular, so two corners name a block exactly and the block is the same from
//! any camera angle. The block is outlined while the button is held.
//!
//! The selection is kept when another tool is chosen and is drawn only while
//! this one is. It is dropped by `Escape`, by the panel's button, and by a map
//! switch, because a cell names a place on the grid and not a map.
//!
//! ## A chunk is named by its place on the map's chunk grid
//!
//! [`Cell`] is a chunk's column and row on the grid of the whole map: the
//! tile's coordinate times sixteen plus `MCNK`'s own `indexX` and `indexY`.
//! Named this way a block that crosses a tile border is one rectangle, a
//! neighbour is one step, and a selection survives its tile being closed and
//! opened again. The world axes run against the grid: a larger column is a
//! smaller world y and a larger row is a smaller world x, which is
//! `vale_edit::adt::blank::chunk_origin`'s arithmetic.
//!
//! ## Which operations read the tile again
//!
//! An area id and the flags word are read by nothing on screen, so those two
//! operations publish the tile and do nothing else. A hole changes how many
//! vertices a chunk contributes and a texture change alters the chunk's
//! texture set, so those operations mark the tile stale and it is read again,
//! as the hole tool and the texture tool's own buttons do.

use std::collections::{BTreeMap, BTreeSet};

use crate::session::EditSession;
use crate::tools::Tool;
use bevy::prelude::*;
use vale_assets::world::adt::{CHUNKS_PER_SIDE, CHUNK_SIZE};
use vale_client::render::axes;
use vale_client::world::camera::WorldCamera;
use vale_edit::adt::{alpha, AdtFile};
use vale_edit::ops::{ChunkPaint, Edit};

/// Chunks along one side of a tile, as the grid's own integer type.
const SIDE: u32 = CHUNKS_PER_SIDE as u32;

/// One map chunk, named by its column and row on the map's chunk grid. See
/// the module comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cell {
    /// The tile's x coordinate times sixteen, plus `MCNK`'s `indexX`.
    pub x: u32,
    /// The tile's y coordinate times sixteen, plus `MCNK`'s `indexY`.
    pub y: u32,
}

impl Cell {
    /// The cell of the chunk at `(index_x, index_y)` in tile `coord`.
    pub fn of(coord: (u32, u32), within: (u32, u32)) -> Cell {
        Cell {
            x: coord.0 * SIDE + within.0,
            y: coord.1 * SIDE + within.1,
        }
    }

    /// The tile this cell is in.
    pub fn tile(self) -> (u32, u32) {
        (self.x / SIDE, self.y / SIDE)
    }

    /// `(indexX, indexY)` within that tile.
    pub fn within(self) -> (u32, u32) {
        (self.x % SIDE, self.y % SIDE)
    }

    /// The cell of the chunk a world position is over, or `None` where no open
    /// tile has one.
    pub fn at(session: &EditSession, x: f32, y: f32) -> Option<Cell> {
        let coord = vale_assets::tile_for_position(x, y);
        let tile = session.tiles.get(&coord)?;
        let chunk = vale_edit::adt::heights::chunk_at(tile, x, y)?;
        Some(Cell::of(coord, tile.chunk(chunk)?.head().index()))
    }

    /// Which of `tile`'s chunks this cell is.
    ///
    /// The files list their chunks row by row, so the answer is
    /// `indexY * 16 + indexX` and the chunk's own header confirms it. A tile
    /// that lists them in another order is searched.
    pub fn chunk_in(self, tile: &AdtFile) -> Option<usize> {
        let within = self.within();
        let usual = (within.1 * SIDE + within.0) as usize;
        if tile.chunk(usual).is_some_and(|chunk| chunk.head().index() == within) {
            return Some(usual);
        }
        tile.chunks
            .iter()
            .position(|chunk| chunk.head().index() == within)
    }

    /// The four neighbours, each with the side of this cell's square it
    /// shares. `None` past the edge of the grid.
    fn neighbours(self) -> [(Side, Option<Cell>); 4] {
        let step = |dx: i32, dy: i32| {
            let x = self.x.checked_add_signed(dx)?;
            let y = self.y.checked_add_signed(dy)?;
            (x < 64 * SIDE && y < 64 * SIDE).then_some(Cell { x, y })
        };
        [
            (Side::LowY, step(1, 0)),
            (Side::HighY, step(-1, 0)),
            (Side::LowX, step(0, 1)),
            (Side::HighX, step(0, -1)),
        ]
    }
}

/// One side of a chunk's square, named by the world coordinate that is fixed
/// along it. A chunk's origin is its corner of greatest x and y.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    /// `x = origin.x`, shared with the row before.
    HighX,
    /// `x = origin.x - CHUNK_SIZE`, shared with the row after.
    LowX,
    /// `y = origin.y`, shared with the column before.
    HighY,
    /// `y = origin.y - CHUNK_SIZE`, shared with the column after.
    LowY,
}

impl Side {
    /// The two ends of this side of the square whose origin is `origin`.
    fn ends(self, origin: [f32; 3]) -> ((f32, f32), (f32, f32)) {
        let (x0, y0) = (origin[0], origin[1]);
        let (x1, y1) = (x0 - CHUNK_SIZE, y0 - CHUNK_SIZE);
        match self {
            Side::HighX => ((x0, y0), (x0, y1)),
            Side::LowX => ((x1, y0), (x1, y1)),
            Side::HighY => ((x0, y0), (x1, y0)),
            Side::LowY => ((x0, y1), (x1, y1)),
        }
    }
}

/// The longest side a dragged block may have, in chunks: the default 7x7
/// block of tiles.
pub const LONGEST: u32 = 7 * SIDE;

/// Every cell of the rectangle whose opposite corners are `from` and `to`,
/// row by row. A side longer than [`LONGEST`] is cut short at `from`'s end.
pub fn block(from: Cell, to: Cell) -> Vec<Cell> {
    let span = |a: u32, b: u32| {
        let (low, high) = (a.min(b), a.max(b));
        match a <= b {
            true => low..=high.min(low + LONGEST - 1),
            false => high.saturating_sub(LONGEST - 1).max(low)..=high,
        }
    };
    let mut cells = Vec::new();
    for y in span(from.y, to.y) {
        for x in span(from.x, to.x) {
            cells.push(Cell { x, y });
        }
    }
    cells
}

/// What a release does to the selection.
///
/// `swept` is the block between the press and the release. A block of one
/// cell with shift held takes that cell out when it is already selected, which
/// is shift+click's second meaning; every other shifted release adds.
pub fn released(selected: &mut BTreeSet<Cell>, swept: &[Cell], adding: bool) {
    match (adding, swept) {
        (true, [one]) if selected.contains(one) => {
            selected.remove(one);
        }
        (true, _) => selected.extend(swept.iter().copied()),
        (false, _) => {
            selected.clear();
            selected.extend(swept.iter().copied());
        }
    }
}

/// The tool's state: the selection, what the pointer is over, and the block
/// being dragged.
#[derive(Resource, Debug, Default)]
pub struct Chunks {
    /// The selected chunks.
    pub selected: BTreeSet<Cell>,
    /// The chunk the last release was on. The panel's "Add all in" button reads
    /// its area id.
    pub primary: Option<Cell>,
    /// The chunk under the pointer, or `None` when the pointer is over no open
    /// tile.
    pub at: Option<Cell>,
    /// The press a drag began with, while the button is held.
    drag: Option<Drag>,
    /// What the panel reports about the selection, and the stamp it was
    /// counted at. See [`Chunks::census`].
    counted: Option<(Stamp, Census)>,
    /// Moved by every change to [`Self::selected`], for the census's stamp.
    revision: u64,
}

/// A held drag: where it began, where it has reached, and whether shift was
/// down at the press.
#[derive(Debug, Clone)]
struct Drag {
    from: Cell,
    to: Cell,
    adding: bool,
    /// The cells between the two, kept so the outline is not recomputed on
    /// frames the pointer stays on one chunk.
    swept: Vec<Cell>,
}

/// What the census was counted against: the selection's revision, the undo
/// stack's depth, whether there is a redo, and how many tiles are open.
type Stamp = (u64, usize, bool, usize);

/// What the selection holds, for the panel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Census {
    /// Selected chunks whose tile is open.
    pub open: usize,
    /// Selected chunks whose tile is not open. No operation reaches them.
    pub closed: usize,
    /// How many tiles the open ones are in.
    pub tiles: usize,
    /// Each area id among them with its chunk count, the largest count first.
    pub areas: Vec<(u32, usize)>,
    /// Each texture among them with the number of chunks that carry it, and
    /// how many of those carry it as their base. The largest count first.
    pub textures: Vec<(String, usize, usize)>,
    /// How many carry the impassable flag.
    pub impassable: usize,
    /// How many of their hole squares are cut, of sixteen per chunk.
    pub cut: usize,
    /// How many carry four textures, which is the most a chunk may.
    pub full: usize,
}

impl Chunks {
    /// Replace the selection.
    pub fn select(&mut self, cells: impl IntoIterator<Item = Cell>) {
        self.selected = cells.into_iter().collect();
        self.revision += 1;
    }

    /// Add to the selection.
    pub fn add(&mut self, cells: impl IntoIterator<Item = Cell>) {
        self.selected.extend(cells);
        self.revision += 1;
    }

    /// Drop the selection.
    pub fn clear(&mut self) {
        self.selected.clear();
        self.primary = None;
        self.revision += 1;
    }

    /// The block a held drag has swept, for the outline.
    pub fn sweeping(&self) -> &[Cell] {
        self.drag.as_ref().map(|drag| drag.swept.as_slice()).unwrap_or(&[])
    }

    /// What the selection holds. Counted when the selection, the undo stack or
    /// the set of open tiles has changed since the last count, and otherwise
    /// answered from the last one: reading a chunk's textures decodes its
    /// blend maps, and the panel asks sixty times a second.
    pub fn census(&mut self, session: &EditSession) -> &Census {
        let stamp: Stamp = (
            self.revision,
            session.history.depth_done(),
            session.history.next_redo().is_some(),
            session.tiles.len(),
        );
        if self.counted.as_ref().is_none_or(|(had, _)| *had != stamp) {
            self.counted = Some((stamp, count(session, &self.selected)));
        }
        &self.counted.as_ref().expect("counted above").1
    }

    /// Every open tile's chunks that lie in a tile the selection already
    /// touches.
    pub fn whole_tiles(&self, session: &EditSession) -> Vec<Cell> {
        let tiles: BTreeSet<(u32, u32)> = self.selected.iter().map(|cell| cell.tile()).collect();
        tiles
            .into_iter()
            .filter(|coord| session.tiles.contains_key(coord))
            .flat_map(tile_cells)
            .collect()
    }

    /// Every chunk of every open tile whose area id is `area`.
    pub fn in_area(session: &EditSession, area: u32) -> Vec<Cell> {
        let mut cells = Vec::new();
        for (coord, tile) in &session.tiles {
            for chunk in &tile.chunks {
                if chunk.head().area_id() == area {
                    cells.push(Cell::of(*coord, chunk.head().index()));
                }
            }
        }
        cells
    }
}

/// The 256 cells of one tile.
pub fn tile_cells(coord: (u32, u32)) -> impl Iterator<Item = Cell> {
    (0..SIDE).flat_map(move |y| (0..SIDE).map(move |x| Cell::of(coord, (x, y))))
}

/// The area id of one cell's chunk, when its tile is open.
pub fn area_of(session: &EditSession, cell: Cell) -> Option<u32> {
    let tile = session.tiles.get(&cell.tile())?;
    Some(tile.chunk(cell.chunk_in(tile)?)?.head().area_id())
}

/// Count what `selected` holds. See [`Census`].
fn count(session: &EditSession, selected: &BTreeSet<Cell>) -> Census {
    let mut census = Census::default();
    let mut tiles: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut areas: BTreeMap<u32, usize> = BTreeMap::new();
    let mut textures: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (coord, cells) in by_tile(selected.iter().copied()) {
        let Some(tile) = session.tiles.get(&coord) else {
            census.closed += cells.len();
            continue;
        };
        tiles.insert(coord);
        let names = tile.texture_names();
        for cell in cells {
            let Some(chunk) = cell.chunk_in(tile).and_then(|index| tile.chunk(index)) else {
                census.closed += 1;
                continue;
            };
            census.open += 1;
            let head = chunk.head();
            *areas.entry(head.area_id()).or_default() += 1;
            census.cut += head.holes().count_ones() as usize;
            if head.flags() & vale_edit::adt::impass::IMPASSABLE != 0 {
                census.impassable += 1;
            }
            let layers = alpha::paint(chunk).layers;
            if layers.len() >= alpha::MAX_LAYERS {
                census.full += 1;
            }
            for (index, layer) in layers.iter().enumerate() {
                let Some(name) = names.get(layer.texture_id as usize) else {
                    continue;
                };
                let entry = textures.entry(name.to_ascii_lowercase()).or_default();
                entry.0 += 1;
                if index == 0 {
                    entry.1 += 1;
                }
            }
        }
    }
    census.tiles = tiles.len();
    census.areas = areas.into_iter().collect();
    census.areas.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    census.textures = textures
        .into_iter()
        .map(|(name, (carried, based))| (name, carried, based))
        .collect();
    census.textures.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    census
}

/// Cells grouped by the tile each is in, the tiles in order.
fn by_tile(cells: impl Iterator<Item = Cell>) -> BTreeMap<(u32, u32), Vec<Cell>> {
    let mut tiles: BTreeMap<(u32, u32), Vec<Cell>> = BTreeMap::new();
    for cell in cells {
        tiles.entry(cell.tile()).or_default().push(cell);
    }
    tiles
}

/// Apply `change` to every cell's chunk and record what it answers, as one
/// undo entry across every tile written. Returns how many chunks changed.
///
/// `change` is given the open tile and the chunk's index in it. It makes the
/// change and answers the [`Edit`] that records it, or `None` for a chunk it
/// left alone. A cell whose tile is not open is skipped.
///
/// `reread` marks each written tile stale, for a change the screen cannot be
/// caught up with any other way. See the module comment.
fn over(
    session: &mut EditSession,
    cells: &BTreeSet<Cell>,
    label: &str,
    reread: bool,
    mut change: impl FnMut(&mut AdtFile, usize) -> Option<Edit>,
) -> usize {
    let mut changed = 0;
    for (coord, cells) in by_tile(cells.iter().copied()) {
        let key = session.key(coord);
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        let mut edits = Vec::new();
        for cell in cells {
            let Some(index) = cell.chunk_in(tile) else {
                continue;
            };
            edits.extend(change(tile, index));
        }
        if edits.is_empty() {
            continue;
        }
        if changed == 0 {
            session.history.begin(label);
        }
        changed += edits.len();
        session.history.record(&key, edits);
        session.publish(coord);
        if reread {
            session.stale.insert(coord);
        }
    }
    if changed > 0 {
        session.history.end();
    }
    changed
}

/// Give every selected chunk the area id `area`.
pub fn set_area(session: &mut EditSession, cells: &BTreeSet<Cell>, area: u32) -> usize {
    over(session, cells, &format!("Set area {area}"), false, |tile, chunk| {
        let before = tile.chunk(chunk)?.head().area_id();
        if before == area {
            return None;
        }
        let edit = Edit::Area {
            chunk,
            before,
            after: area,
        };
        edit.apply(tile);
        Some(edit)
    })
}

/// Set or clear the impassable flag on every selected chunk.
pub fn set_impassable(session: &mut EditSession, cells: &BTreeSet<Cell>, on: bool) -> usize {
    let label = match on {
        true => "Impassable",
        false => "Passable",
    };
    over(session, cells, label, false, |tile, chunk| {
        let before = tile.chunk(chunk)?.head().flags();
        if !vale_edit::adt::impass::set_impassable(tile, chunk, on) {
            return None;
        }
        let after = tile.chunk(chunk)?.head().flags();
        (after != before).then_some(Edit::Flags {
            chunk,
            before,
            after,
        })
    })
}

/// Cut all sixteen squares of every selected chunk, or put all sixteen back.
pub fn set_holes(session: &mut EditSession, cells: &BTreeSet<Cell>, cut: bool) -> usize {
    let (label, after) = match cut {
        true => ("Cut chunks", u16::MAX),
        false => ("Patch chunks", 0),
    };
    over(session, cells, label, true, |tile, chunk| {
        let before = tile.chunk(chunk)?.head().holes();
        if before == after {
            return None;
        }
        let edit = Edit::Holes {
            chunk,
            before,
            after,
        };
        edit.apply(tile);
        Some(edit)
    })
}

/// Change one chunk's paint with `change` and answer the edit, or `None` when
/// `change` answers that nothing moved.
///
/// The chunk's paint is captured before `change` runs, and `change` is given
/// the tile so that it can name a texture in `MTEX`: the capture holds the
/// tile's texture list, so an undo takes the name out again.
fn repaint(
    tile: &mut AdtFile,
    chunk: usize,
    change: impl FnOnce(&mut AdtFile, &mut alpha::Paint) -> bool,
) -> Option<Edit> {
    let before = ChunkPaint::capture(tile, chunk);
    let mut paint = alpha::paint(tile.chunk(chunk)?);
    if !change(tile, &mut paint) {
        return None;
    }
    alpha::set_paint(tile.chunk_mut(chunk)?, &paint);
    Some(Edit::Paint {
        chunk,
        before: Box::new(before),
        after: Box::new(ChunkPaint::capture(tile, chunk)),
    })
}

/// Where `path` is in a tile's texture list, without adding it.
fn texture_id(tile: &AdtFile, path: &str) -> Option<u32> {
    tile.texture_names()
        .iter()
        .position(|name| name.eq_ignore_ascii_case(path))
        .map(|at| at as u32)
}

/// Make `path` the base texture of every selected chunk, under whatever each
/// already carries. Every blend map is kept.
pub fn set_base(session: &mut EditSession, cells: &BTreeSet<Cell>, path: &str) -> usize {
    let label = format!("Base {}", super::textures::leaf(path));
    over(session, cells, &label, true, |tile, chunk| {
        repaint(tile, chunk, |tile, paint| {
            let id = tile.name_texture(path);
            paint.set_base(id)
        })
    })
}

/// Take every layer but the base off every selected chunk.
pub fn clear_paint(session: &mut EditSession, cells: &BTreeSet<Cell>) -> usize {
    over(session, cells, "Clear paint", true, |tile, chunk| {
        repaint(tile, chunk, |_, paint| paint.clear_to_base() > 0)
    })
}

/// In every selected chunk that carries `from`, make that layer draw `to`
/// and keep its blend map.
///
/// A chunk that already carries `to` on another layer is left alone: two
/// layers drawing one texture is a slot spent on nothing.
pub fn swap_texture(
    session: &mut EditSession,
    cells: &BTreeSet<Cell>,
    from: &str,
    to: &str,
) -> usize {
    let label = format!(
        "Swap {} to {}",
        super::textures::leaf(from),
        super::textures::leaf(to)
    );
    over(session, cells, &label, true, |tile, chunk| {
        let had = texture_id(tile, from)?;
        repaint(tile, chunk, |tile, paint| {
            let Some(layer) = paint.layer_of(had) else {
                return false;
            };
            if texture_id(tile, to).is_some_and(|id| paint.layer_of(id).is_some()) {
                return false;
            }
            let id = tile.name_texture(to);
            paint.set_layer_texture(layer, id)
        })
    })
}

/// Take the layer that draws `path` off every selected chunk that carries it
/// above its base. A base is replaced, never removed, so a chunk based on
/// `path` is left alone.
pub fn remove_texture(session: &mut EditSession, cells: &BTreeSet<Cell>, path: &str) -> usize {
    let label = format!("Remove {}", super::textures::leaf(path));
    over(session, cells, &label, true, |tile, chunk| {
        let had = texture_id(tile, path)?;
        repaint(tile, chunk, |_, paint| {
            paint.layer_of(had).is_some_and(|layer| paint.remove_layer(layer))
        })
    })
}

pub struct ChunkToolPlugin;

impl Plugin for ChunkToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Chunks>()
            // After the pick, as every tool is: the chunk is found from the
            // pointer's ray, and the ray must be this frame's.
            .add_systems(
                Update,
                (scripted, aim, press, draw).chain().after(crate::pick::aim),
            );
    }
}

/// Find the chunk under the pointer.
#[allow(clippy::too_many_arguments)]
fn aim(
    mut chunks: ResMut<Chunks>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    let chunks = chunks.bypass_change_detection();
    chunks.at = None;
    if !state.editing() || *tool != Tool::Chunks {
        return;
    }
    let Some(session) = session else { return };
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    // The ground with its holes ignored, as the area and hole tools aim: a
    // chunk cut through entirely is still a chunk, and aiming at the drawn
    // ground would make it unreachable.
    let Some(point) = crate::pick::solid_under(&session, &windows, &camera) else {
        return;
    };
    chunks.at = Cell::at(&session, point.x, point.y);
}

/// The press, the drag and the release, and the two keys.
#[allow(clippy::too_many_arguments)]
fn press(
    mut chunks: ResMut<Chunks>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    typing: Res<vale_client::lua::api::keyboard::KeyboardFocus>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut map: Local<String>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    // A cell is a place on the grid and not on a map, so a selection made on
    // one map would name other ground on the next.
    // The first frame only notes which map is open.
    if *map != session.map {
        let switched = !map.is_empty();
        *map = session.map.clone();
        if switched && (!chunks.selected.is_empty() || chunks.drag.is_some()) {
            chunks.clear();
            chunks.drag = None;
        }
    }
    // A release is answered wherever the pointer is and whichever tool is
    // chosen, so a drag that ends over a panel or after a tool switch ends.
    if !buttons.pressed(MouseButton::Left) {
        if let Some(drag) = chunks.drag.take() {
            if state.editing() && *tool == Tool::Chunks {
                let chunks = &mut *chunks;
                released(&mut chunks.selected, &drag.swept, drag.adding);
                chunks.revision += 1;
                chunks.primary = Some(drag.to);
                session.bypass_change_detection().status =
                    format!("{} chunks selected", chunks.selected.len());
            }
        }
    }
    if !state.editing() || *tool != Tool::Chunks {
        return;
    }

    if buttons.just_pressed(MouseButton::Left) {
        if let Some(at) = chunks.at {
            chunks.drag = Some(Drag {
                from: at,
                to: at,
                adding: super::group::shift(&keys),
                swept: vec![at],
            });
        }
    }
    // The pointer off the ground mid-drag leaves the block where it last was.
    if let (Some(at), Some(drag)) = (chunks.at, chunks.drag.as_mut()) {
        if drag.to != at {
            drag.to = at;
            drag.swept = block(drag.from, at);
        }
    }

    if wants.wants_keyboard_input() || typing.active {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) && !chunks.selected.is_empty() {
        chunks.clear();
        session.bypass_change_detection().status = "chunk selection dropped".to_string();
    }
    let control = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if control && keys.just_pressed(KeyCode::KeyA) {
        if let Some(at) = chunks.at.or(chunks.primary) {
            match super::group::shift(&keys) {
                true => chunks.add(tile_cells(at.tile())),
                false => chunks.select(tile_cells(at.tile())),
            }
            chunks.primary = Some(at);
            session.bypass_change_detection().status =
                format!("{} chunks selected", chunks.selected.len());
        }
    }
}

/// `--chunks "<x,y>;<x,y>"`: select the block between the chunks two world
/// positions are over, once both are over open tiles. One position selects
/// its own chunk. See `crate::Args::chunks`.
fn scripted(
    mut chunks: ResMut<Chunks>,
    session: Option<Res<EditSession>>,
    args: Res<crate::Args>,
    mut done: Local<bool>,
) {
    let (Some(points), Some(session)) = (args.chunks.as_ref(), session) else {
        return;
    };
    if *done {
        return;
    }
    let cells: Option<Vec<Cell>> = points
        .iter()
        .take(2)
        .map(|&(x, y)| Cell::at(&session, x, y))
        .collect();
    let Some(cells) = cells.filter(|cells| !cells.is_empty()) else {
        return;
    };
    *done = true;
    let (from, to) = (cells[0], cells[cells.len() - 1]);
    chunks.select(block(from, to));
    chunks.primary = Some(to);
    info!("--chunks: {} selected", chunks.selected.len());
}

/// How many segments one side of a chunk is drawn in, so the outline follows
/// the ground across the 33 yards of it.
const DRAPE: usize = 4;

/// Above this many outlined chunks each side is one straight line between its
/// two corners' heights, which keeps a selection of several tiles to a few
/// thousand lines.
const DRAPED_UP_TO: usize = 600;

/// How far above the ground the outline is lifted, in yards. The lines are in
/// the handle group and draw over the world whatever their depth; the lift is
/// so that they read as lying on the ground.
const LIFT: f32 = 0.15;

/// How far inside its own square a sample on a chunk's side is taken, in
/// yards. `ChunkGround::height_at` is exclusive at the far edge, and every
/// side drawn here is on an edge.
const INSET: f32 = 0.05;

/// Outline the selection, the block being dragged and the chunk under the
/// pointer.
///
/// Only the sides a set shares with a chunk outside it are drawn, so a block
/// is one outline and not a lattice.
fn draw(
    mut gizmos: Gizmos<crate::tools::gizmo::EditorHandles>,
    chunks: Res<Chunks>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !state.editing() || *tool != Tool::Chunks {
        return;
    }
    let Some(session) = session else { return };
    let selected = Color::srgb(0.45, 0.80, 1.0);
    let sweeping = Color::srgb(1.0, 0.85, 0.35);
    let hovered = Color::srgba(1.0, 1.0, 1.0, 0.7);

    outline(&mut gizmos, &session, &chunks.selected, selected);
    let swept: BTreeSet<Cell> = chunks.sweeping().iter().copied().collect();
    outline(&mut gizmos, &session, &swept, sweeping);
    if let (Some(at), true) = (chunks.at, swept.is_empty()) {
        outline(&mut gizmos, &session, &BTreeSet::from([at]), hovered);
    }
}

/// The sides of `cells` that border a cell outside the set. See [`draw`].
fn border(cells: &BTreeSet<Cell>) -> Vec<(Cell, Side)> {
    let mut sides = Vec::new();
    for &cell in cells {
        for (side, neighbour) in cell.neighbours() {
            if neighbour.is_none_or(|other| !cells.contains(&other)) {
                sides.push((cell, side));
            }
        }
    }
    sides
}

/// Draw the border of `cells` on the ground.
fn outline(
    gizmos: &mut Gizmos<crate::tools::gizmo::EditorHandles>,
    session: &EditSession,
    cells: &BTreeSet<Cell>,
    colour: Color,
) {
    let sides = border(cells);
    let steps = match sides.len() <= DRAPED_UP_TO * 4 {
        true => DRAPE,
        false => 1,
    };
    // The sides arrive grouped by cell, so each chunk's ground is built once.
    let mut ground: Option<(Cell, vale_assets::world::adt::ChunkGround)> = None;
    for (cell, side) in sides {
        if ground.as_ref().is_none_or(|(had, _)| *had != cell) {
            let Some(chunk) = session
                .tiles
                .get(&cell.tile())
                .and_then(|tile| tile.chunk(cell.chunk_in(tile)?))
            else {
                continue;
            };
            ground = Some((cell, vale_edit::adt::heights::ground(chunk)));
        }
        let Some((_, ground)) = ground.as_ref() else {
            continue;
        };
        let origin = ground.position;
        let inside = |value: f32, origin: f32| value.clamp(origin - CHUNK_SIZE + INSET, origin - INSET);
        let at = |x: f32, y: f32| {
            let z = ground
                .height_at(inside(x, origin[0]), inside(y, origin[1]))
                // Over a hole the chunk's own reference height is used.
                .unwrap_or(origin[2]);
            Vec3::from(axes::to_bevy([x, y, z + LIFT]))
        };
        let (from, to) = side.ends(origin);
        let mut last = at(from.0, from.1);
        for step in 1..=steps {
            let k = step as f32 / steps as f32;
            let next = at(from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k);
            gizmos.line(last, next, colour);
            last = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_edit::adt::blank::{blank_tile, chunk_origin, CHUNKS};

    const BASE: &str = "tileset\\test\\base.blp";

    /// A session holding blank tiles at `coords`, in a project folder of its
    /// own under the temporary directory.
    fn session_with(name: &str, coords: &[(u32, u32)]) -> (EditSession, std::path::PathBuf) {
        let install =
            std::env::temp_dir().join(format!("vale-chunks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = vale_edit::project::Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        for &coord in coords {
            session
                .tiles
                .insert(coord, blank_tile(coord.0, coord.1, BASE, 0.0, 12));
        }
        (session, install)
    }

    /// Undo the last entry on every tile it names, as `EditSession::undo`
    /// does without the archives.
    fn undo(session: &mut EditSession) {
        let change = session.history.undo().unwrap();
        for key in change.tiles() {
            let coord = (key.x, key.y);
            if let Some(tile) = session.tiles.get_mut(&coord) {
                change.revert(&key, tile);
            }
        }
    }

    /// A cell names the chunk the file has at that place: its tile, its index
    /// in the file, and the world position `tile_for_position` and `chunk_at`
    /// answer for. A step of one column is one chunk towards smaller y, and
    /// from the last column it crosses into the next tile.
    #[test]
    fn a_cell_is_the_chunk_at_that_place_on_the_grid() {
        let here = (31, 49);
        let (session, install) = session_with("grid", &[here, (32, 49), (31, 50)]);
        let tile = &session.tiles[&here];
        for index in 0..CHUNKS {
            let head = tile.chunk(index).unwrap().head();
            let cell = Cell::of(here, head.index());
            assert_eq!(cell.tile(), here);
            assert_eq!(cell.chunk_in(tile), Some(index));
            let origin = chunk_origin(here.0, here.1, index);
            let centre = [origin[0] - CHUNK_SIZE * 0.5, origin[1] - CHUNK_SIZE * 0.5];
            assert_eq!(Cell::at(&session, centre[0], centre[1]), Some(cell), "chunk {index}");
            // One column on is one chunk towards smaller y; one row on is one
            // chunk towards smaller x.
            let column = Cell { x: cell.x + 1, y: cell.y };
            let row = Cell { x: cell.x, y: cell.y + 1 };
            assert_eq!(Cell::at(&session, centre[0], centre[1] - CHUNK_SIZE), Some(column));
            assert_eq!(Cell::at(&session, centre[0] - CHUNK_SIZE, centre[1]), Some(row));
        }
        assert_eq!(Cell::of(here, (15, 0)).neighbours()[0].1.unwrap().tile(), (32, 49));
        assert_eq!(Cell::of(here, (0, 15)).neighbours()[2].1.unwrap().tile(), (31, 50));
        let _ = std::fs::remove_dir_all(&install);
    }

    /// Each side of a cell's square is the one it shares with the neighbour
    /// listed beside it: the side's middle is halfway between the two chunks'
    /// centres.
    #[test]
    fn each_side_is_shared_with_the_neighbour_it_is_listed_with() {
        let coord = (31, 49);
        let cell = Cell::of(coord, (5, 7));
        let origin_of = |cell: Cell| {
            let within = cell.within();
            let tile = cell.tile();
            let at = chunk_origin(tile.0, tile.1, (within.1 * SIDE + within.0) as usize);
            [at[0], at[1], 0.0]
        };
        let centre = |cell: Cell| {
            let origin = origin_of(cell);
            [origin[0] - CHUNK_SIZE * 0.5, origin[1] - CHUNK_SIZE * 0.5]
        };
        for (side, neighbour) in cell.neighbours() {
            let (from, to) = side.ends(origin_of(cell));
            let middle = [(from.0 + to.0) * 0.5, (from.1 + to.1) * 0.5];
            let (mine, theirs) = (centre(cell), centre(neighbour.unwrap()));
            for axis in 0..2 {
                let between = (mine[axis] + theirs[axis]) * 0.5;
                assert!(
                    (middle[axis] - between).abs() < 1e-2,
                    "{side:?}: {middle:?} is not between {mine:?} and {theirs:?}"
                );
            }
        }
    }

    /// A block is the rectangle between its corners whichever way it was
    /// dragged, and a side longer than the limit is cut short at the press.
    #[test]
    fn a_block_is_the_rectangle_between_two_cells() {
        let a = Cell { x: 500, y: 790 };
        let b = Cell { x: 503, y: 788 };
        let forward = block(a, b);
        assert_eq!(forward.len(), 4 * 3);
        let mut back = block(b, a);
        back.sort();
        let mut sorted = forward.clone();
        sorted.sort();
        assert_eq!(sorted, back);
        assert!(forward.contains(&a) && forward.contains(&b));
        assert_eq!(block(a, a), vec![a]);

        let far = Cell { x: 500 + 3 * LONGEST, y: 790 };
        let cut = block(a, far);
        assert_eq!(cut.len() as u32, LONGEST);
        assert!(cut.contains(&a), "the press's end is kept");
        let cut = block(far, a);
        assert_eq!(cut.len() as u32, LONGEST);
        assert!(cut.contains(&far), "the press's end is kept");
    }

    /// A release replaces the selection, a shifted one adds, and a shifted
    /// click on a selected chunk takes it out.
    #[test]
    fn a_release_replaces_adds_or_takes_out() {
        let cell = |x| Cell { x, y: 10 };
        let mut selected = BTreeSet::from([cell(1), cell(2)]);
        released(&mut selected, &[cell(5), cell(6)], false);
        assert_eq!(selected, BTreeSet::from([cell(5), cell(6)]));
        released(&mut selected, &[cell(6), cell(7)], true);
        assert_eq!(selected, BTreeSet::from([cell(5), cell(6), cell(7)]));
        released(&mut selected, &[cell(6)], true);
        assert_eq!(selected, BTreeSet::from([cell(5), cell(7)]));
        released(&mut selected, &[cell(9)], true);
        assert_eq!(selected, BTreeSet::from([cell(5), cell(7), cell(9)]));
    }

    /// The border of two adjacent cells is six sides, and of a 3x3 block
    /// twelve: the shared sides are not drawn.
    #[test]
    fn the_border_leaves_out_shared_sides() {
        let pair = BTreeSet::from([Cell { x: 4, y: 4 }, Cell { x: 5, y: 4 }]);
        assert_eq!(border(&pair).len(), 6);
        let nine: BTreeSet<Cell> = block(Cell { x: 4, y: 4 }, Cell { x: 6, y: 6 })
            .into_iter()
            .collect();
        assert_eq!(border(&nine).len(), 12);
    }

    /// An area set over a block that crosses a tile border is one undo entry,
    /// writes every chunk of the block in both tiles, and one undo puts every
    /// id back. A second press with the same id records nothing.
    #[test]
    fn an_area_over_two_tiles_is_one_entry() {
        let (here, there) = ((31, 49), (32, 49));
        let (mut session, install) = session_with("area", &[here, there]);
        let cells: BTreeSet<Cell> = block(Cell::of(here, (14, 2)), Cell::of(there, (1, 3)))
            .into_iter()
            .collect();
        assert_eq!(cells.len(), 8);

        assert_eq!(set_area(&mut session, &cells, 87), 8);
        assert_eq!(session.history.depth_done(), 1);
        assert!(cells.iter().all(|&cell| area_of(&session, cell) == Some(87)));
        assert_eq!(area_of(&session, Cell::of(here, (13, 2))), Some(12), "outside the block");
        assert!(session.unsaved.contains(&here) && session.unsaved.contains(&there));
        assert!(session.stale.is_empty(), "nothing on screen reads an area id");

        assert_eq!(set_area(&mut session, &cells, 87), 0);
        assert_eq!(session.history.depth_done(), 1, "no entry for no change");

        undo(&mut session);
        assert!(cells.iter().all(|&cell| area_of(&session, cell) == Some(12)));
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A cell whose tile is not open is skipped by an operation and counted
    /// as closed by the census.
    #[test]
    fn a_closed_tile_is_skipped_and_counted() {
        let here = (31, 49);
        let (mut session, install) = session_with("closed", &[here]);
        let mut chunks = Chunks::default();
        chunks.select(block(Cell::of(here, (15, 0)), Cell::of((32, 49), (0, 0))));
        assert_eq!(set_area(&mut session, &chunks.selected, 5), 1);
        let census = chunks.census(&session).clone();
        assert_eq!((census.open, census.closed, census.tiles), (1, 1, 1));
        assert_eq!(census.areas, vec![(5, 1)]);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// The impassable flag and the hole mask are written to every selected
    /// chunk, a hole change asks for the tile again, and each is undone whole.
    #[test]
    fn flags_and_holes_are_written_to_every_chunk() {
        let here = (31, 49);
        let (mut session, install) = session_with("flags", &[here]);
        let mut chunks = Chunks::default();
        chunks.select(block(Cell::of(here, (0, 0)), Cell::of(here, (3, 1))));

        assert_eq!(set_impassable(&mut session, &chunks.selected, true), 8);
        assert!(session.stale.is_empty());
        assert_eq!(chunks.census(&session).impassable, 8);
        assert_eq!(set_impassable(&mut session, &chunks.selected, true), 0);

        assert_eq!(set_holes(&mut session, &chunks.selected, true), 8);
        assert!(session.stale.contains(&here));
        assert_eq!(chunks.census(&session).cut, 8 * 16);
        assert_eq!(session.history.depth_done(), 2);

        undo(&mut session);
        assert_eq!(chunks.census(&session).cut, 0);
        undo(&mut session);
        assert_eq!(chunks.census(&session).impassable, 0);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// The texture operations: a base replaced on every chunk, that texture
    /// swapped for another, and an undo that also takes the new name out of
    /// the tile's list. A base is not removed, and a swap to a texture the
    /// chunk already carries is refused.
    #[test]
    fn textures_are_based_swapped_and_put_back() {
        let here = (31, 49);
        let (mut session, install) = session_with("paint", &[here]);
        let mut chunks = Chunks::default();
        chunks.select(block(Cell::of(here, (2, 2)), Cell::of(here, (3, 3))));
        let (grass, rock) = ("tileset\\test\\grass.blp", "tileset\\test\\rock.blp");
        let names = |session: &EditSession| session.tiles[&here].texture_names();

        assert_eq!(set_base(&mut session, &chunks.selected, grass), 4);
        assert_eq!(set_base(&mut session, &chunks.selected, grass), 0);
        assert_eq!(chunks.census(&session).textures, vec![(grass.to_string(), 4, 4)]);
        assert!(session.stale.contains(&here));

        assert_eq!(swap_texture(&mut session, &chunks.selected, grass, rock), 4);
        assert_eq!(chunks.census(&session).textures, vec![(rock.to_string(), 4, 4)]);
        assert_eq!(swap_texture(&mut session, &chunks.selected, grass, rock), 0);
        assert_eq!(remove_texture(&mut session, &chunks.selected, rock), 0, "a base stays");
        assert_eq!(clear_paint(&mut session, &chunks.selected), 0, "nothing above the base");
        assert_eq!(session.history.depth_done(), 2);
        assert_eq!(names(&session).len(), 3);

        undo(&mut session);
        assert_eq!(chunks.census(&session).textures, vec![(grass.to_string(), 4, 4)]);
        assert_eq!(names(&session).len(), 2, "the undo takes rock out of MTEX");
        undo(&mut session);
        assert_eq!(chunks.census(&session).textures, vec![(BASE.to_string(), 4, 4)]);
        assert_eq!(names(&session).len(), 1);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// Growing a selection to whole tiles takes every chunk of each tile it
    /// touches, and selecting by area takes every chunk of that area in the
    /// open tiles.
    #[test]
    fn a_selection_grows_to_tiles_and_to_an_area() {
        let (here, there) = ((31, 49), (32, 49));
        let (mut session, install) = session_with("grow", &[here, there]);
        let mut chunks = Chunks::default();
        chunks.select([Cell::of(here, (3, 3))]);
        assert_eq!(chunks.whole_tiles(&session).len(), CHUNKS);

        let patch: BTreeSet<Cell> = block(Cell::of(here, (15, 0)), Cell::of(there, (0, 1)))
            .into_iter()
            .collect();
        set_area(&mut session, &patch, 40);
        let found: BTreeSet<Cell> = Chunks::in_area(&session, 40).into_iter().collect();
        assert_eq!(found, patch);
        assert_eq!(Chunks::in_area(&session, 12).len(), 2 * CHUNKS - 4);
        let _ = std::fs::remove_dir_all(&install);
    }
}
