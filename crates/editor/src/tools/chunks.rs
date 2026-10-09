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
//! ctrl + c             copies the selected chunks
//! ctrl + v             pastes them, the copied block centred on the chunk
//!                      under the pointer, and stitches the block to the
//!                      ground around it when the panel asks for that. With
//!                      ctrl held the outline shows where
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
//! ## What a copy holds and what a paste writes
//!
//! A [`Clip`] holds each selected chunk's heights, layers and blend maps,
//! vertex colours, hole mask, water, area id and impassable flag, with its
//! offset in the block that bounds the selection. A layer's texture is kept by
//! path, because a layer's index into `MTEX` means nothing in another tile.
//! [`paste`] writes the parts [`Parts`] names, at the copied height or moved to
//! the level of the ground it replaces ([`Level`]). It does not turn or mirror
//! the block and copies no placements; the doodad and WMO tools copy those.
//!
//! ## Stitching the selection to the ground around it
//!
//! Two pieces of ground meet at a step wherever the two chunks on either side
//! of a chunk boundary store different heights for the vertices they share:
//! a pasted block against the ground it landed in, or the two sides of a tile
//! border that were edited apart. [`stitch`] closes the step along the
//! selection's border. For every vertex the selection shares with an
//! unselected chunk in an open tile it chooses one height ([`Yields`]: the
//! ground's, the selection's, or halfway) and writes it to both copies. Then
//! it blends: every vertex within [`Stitch::reach`] yards of the border, on
//! each side that moves, is lifted by the border's own lift there, scaled by
//! a smooth step that is one at the border and zero at the reach. Beyond the
//! reach the shape is kept.
//!
//! A step inside the selection is not stitched: select one side of it, so
//! the step is the selection's border. The normals of every chunk whose
//! heights moved, and of its four neighbours, are recomputed across tile
//! borders. Nothing standing on the moved ground is moved with it.
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
use vale_assets::world::adt::{CHUNKS_PER_SIDE, CHUNK_SIZE, INNER_SIDE, OUTER_SIDE, UNIT_SIZE};
use vale_client::render::axes;
use vale_client::world::camera::WorldCamera;
use vale_edit::adt::{alpha, heights, AdtFile};
use vale_edit::ops::{ChunkPaint, Edit};

/// Chunks along one side of a tile, as the grid's own integer type.
const SIDE: u32 = CHUNKS_PER_SIDE as u32;

/// Steps of the vertex lattice along one side of a chunk: eight, so the ninth
/// outer vertex of one chunk is the first of the next.
const PITCH: u32 = (OUTER_SIDE - 1) as u32;

/// Vertices in one interleaved pair of `MCVT` rows: nine outer, eight inner.
const STRIDE: usize = OUTER_SIDE + INNER_SIDE;

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

    /// The cell `dx` columns and `dy` rows from this one, or `None` past the
    /// edge of the grid.
    fn step(self, dx: i32, dy: i32) -> Option<Cell> {
        let x = self.x.checked_add_signed(dx)?;
        let y = self.y.checked_add_signed(dy)?;
        (x < 64 * SIDE && y < 64 * SIDE).then_some(Cell { x, y })
    }

    /// The four neighbours, each with the side of this cell's square it
    /// shares. `None` past the edge of the grid.
    fn neighbours(self) -> [(Side, Option<Cell>); 4] {
        [
            (Side::LowY, self.step(1, 0)),
            (Side::HighY, self.step(-1, 0)),
            (Side::LowX, self.step(0, 1)),
            (Side::HighX, self.step(0, -1)),
        ]
    }

    /// The neighbour across one side.
    fn across(self, side: Side) -> Option<Cell> {
        self.neighbours()
            .into_iter()
            .find(|(had, _)| *had == side)
            .and_then(|(_, cell)| cell)
    }

    /// The eight cells around this one that are on the grid.
    fn around(self) -> impl Iterator<Item = Cell> {
        [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]
            .into_iter()
            .filter_map(move |(dx, dy)| self.step(dx, dy))
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

    /// The nine outer vertices along this side, each as its index in this
    /// chunk and its index in the chunk across the side. The two are one
    /// vertex stored twice: `MCVT`'s rows run along decreasing world x and its
    /// columns along decreasing world y, so row 0 is the high-x side and
    /// column 8 the low-y side.
    fn along(self) -> [(usize, usize); OUTER_SIDE] {
        let last = OUTER_SIDE - 1;
        std::array::from_fn(|k| {
            let (own, across) = match self {
                Side::HighX => (heights::outer(0, k), heights::outer(last, k)),
                Side::LowX => (heights::outer(last, k), heights::outer(0, k)),
                Side::HighY => (heights::outer(k, 0), heights::outer(k, last)),
                Side::LowY => (heights::outer(k, last), heights::outer(k, 0)),
            };
            (own.expect("on the grid"), across.expect("on the grid"))
        })
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
    /// What `Ctrl+C` took.
    pub clip: Clip,
    /// Which parts of the clip `Ctrl+V` writes, and at what height.
    pub parts: Parts,
    pub level: Level,
    /// How the selection is stitched to the ground around it.
    pub stitch: Stitch,
    /// Whether `Ctrl+V` stitches the pasted block to the ground around it, in
    /// the same undo entry as the paste.
    pub stitch_pasted: bool,
    /// Which page of the panel is open.
    pub page: Page,
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
    /// What the selection's border meets.
    pub border: Border,
}

/// What a selection's border meets, for the panel. See [`stitch`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Border {
    /// Sides of selected chunks that meet an unselected chunk in an open
    /// tile. A side on a closed tile or past the edge of the map is not one.
    pub sides: usize,
    /// The largest difference, in yards, between the two heights stored for
    /// one vertex on those sides: the tallest step along the border.
    pub step: f32,
}

/// Which page of the chunk panel is open. The panel's operations are grouped
/// by what they write, one group a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Paste,
    Stitch,
    Area,
    Holes,
    Textures,
}

/// Which side of the border moves when the selection is stitched to the
/// ground around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Yields {
    /// The selected chunks move to meet the ground around them, which stays
    /// where it is.
    #[default]
    Selection,
    /// The ground around the selection moves to meet it, and the selection
    /// stays where it is.
    Ground,
    /// Both move halfway.
    Both,
}

/// How a stitch is made. See [`stitch`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stitch {
    pub yields: Yields,
    /// How far from the border the blend reaches, in yards, on each side that
    /// moves. Zero welds the border's vertices and moves nothing else.
    pub reach: f32,
}

impl Default for Stitch {
    fn default() -> Stitch {
        Stitch {
            yields: Yields::Selection,
            reach: CHUNK_SIZE,
        }
    }
}

/// The furthest a stitch reaches, in yards: three chunks.
pub const FURTHEST: f32 = 3.0 * CHUNK_SIZE;

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
    census.border = seam(session, selected).1;
    census
}

/// One cell's chunk's 145 world heights, when its tile is open.
fn chunk_heights(session: &EditSession, cell: Cell) -> Option<Vec<f32>> {
    let tile = session.tiles.get(&cell.tile())?;
    let chunk = tile.chunk(cell.chunk_in(tile)?)?;
    let found = heights::heights(chunk);
    (!found.is_empty()).then_some(found)
}

/// A vertex's place on the map's vertex lattice, in units of `UNIT_SIZE`:
/// along the grid's columns, then along its rows. An outer vertex lands on
/// whole numbers and an inner one on halves, and a vertex on a chunk's side
/// has the same place from both chunks.
fn lattice(cell: Cell, index: usize) -> (f32, f32) {
    let (dx, dy) = heights::vertex_offset(index);
    (
        (cell.x * PITCH) as f32 + dy / UNIT_SIZE,
        (cell.y * PITCH) as f32 + dx / UNIT_SIZE,
    )
}

/// [`lattice`] for an outer vertex, as whole numbers, or `None` for an inner
/// one.
fn outer_key(cell: Cell, index: usize) -> Option<(u32, u32)> {
    let (row, col) = (index / STRIDE, index % STRIDE);
    (col < OUTER_SIDE).then(|| (cell.x * PITCH + col as u32, cell.y * PITCH + row as u32))
}

/// One vertex on the border between the selection and the ground beyond it:
/// where it is on the lattice, and the heights each side's chunks store for
/// it. A vertex at a corner is stored by up to two chunks a side.
struct Join {
    at: (f32, f32),
    inside: Vec<f32>,
    outside: Vec<f32>,
}

/// Every vertex the selection shares with an unselected chunk in an open
/// tile, by its place on the lattice, with the count the panel reports.
fn seam(session: &EditSession, cells: &BTreeSet<Cell>) -> (BTreeMap<(u32, u32), Join>, Border) {
    let mut joins: BTreeMap<(u32, u32), Join> = BTreeMap::new();
    let mut found = Border::default();
    // Each chunk is decoded once, though a border cell is listed once a side.
    let mut read: BTreeMap<Cell, Option<Vec<f32>>> = BTreeMap::new();
    let mut heights_of = |cell: Cell| -> Option<Vec<f32>> {
        read.entry(cell)
            .or_insert_with(|| chunk_heights(session, cell))
            .clone()
    };
    for (cell, side) in border(cells) {
        let Some(other) = cell.across(side) else {
            continue;
        };
        let (Some(mine), Some(theirs)) = (heights_of(cell), heights_of(other)) else {
            continue;
        };
        found.sides += 1;
        for (own, across) in side.along() {
            let key = outer_key(cell, own).expect("an outer vertex");
            let join = joins.entry(key).or_insert_with(|| Join {
                at: lattice(cell, own),
                inside: Vec::new(),
                outside: Vec::new(),
            });
            join.inside.push(mine[own]);
            join.outside.push(theirs[across]);
            found.step = found.step.max((mine[own] - theirs[across]).abs());
        }
    }
    (joins, found)
}

/// The cells within `steps` of `from` on the grid, a diagonal counting as one
/// step, that `keep` accepts; `from`'s own cells where `keep` accepts them.
fn grow(from: impl IntoIterator<Item = Cell>, steps: u32, keep: impl Fn(Cell) -> bool) -> BTreeSet<Cell> {
    let mut found: BTreeSet<Cell> = from.into_iter().filter(|&cell| keep(cell)).collect();
    let mut edge: Vec<Cell> = found.iter().copied().collect();
    for _ in 0..steps {
        let mut next = Vec::new();
        for cell in edge {
            for other in cell.around() {
                if keep(other) && found.insert(other) {
                    next.push(other);
                }
            }
        }
        edge = next;
    }
    found
}

/// Stitch the selection to the ground around it, as one undo entry. Returns
/// how many chunks' heights changed. See the module comment.
pub fn stitch(session: &mut EditSession, cells: &BTreeSet<Cell>, how: Stitch) -> usize {
    session.history.begin(format!("Stitch {} chunks", cells.len()));
    let changed = stitch_in(session, cells, how);
    session.history.end();
    changed
}

/// [`stitch`], recorded in whatever history entry is open.
fn stitch_in(session: &mut EditSession, cells: &BTreeSet<Cell>, how: Stitch) -> usize {
    let (joins, _) = seam(session, cells);
    if joins.is_empty() {
        return 0;
    }
    let (w_in, w_out) = match how.yields {
        Yields::Selection => (1.0, 0.0),
        Yields::Ground => (0.0, 1.0),
        Yields::Both => (0.5, 0.5),
    };

    // Each border vertex: where it is, the height both copies end at, and how
    // far each side's ground moves there to reach it.
    struct Point {
        at: (f32, f32),
        target: f32,
        lift_in: f32,
        lift_out: f32,
    }
    let mean = |heights: &[f32]| heights.iter().sum::<f32>() / heights.len().max(1) as f32;
    let points: BTreeMap<(u32, u32), Point> = joins
        .iter()
        .map(|(key, join)| {
            let (inside, outside) = (mean(&join.inside), mean(&join.outside));
            let point = Point {
                at: join.at,
                target: inside + w_in * (outside - inside),
                lift_in: w_in * (outside - inside),
                lift_out: w_out * (inside - outside),
            };
            (*key, point)
        })
        .collect();
    // The points by the cell each is in, so a chunk reads the cells within
    // reach of it and not every point on the border.
    let mut buckets: BTreeMap<(i64, i64), Vec<&Point>> = BTreeMap::new();
    for point in points.values() {
        let key = (
            (point.at.0 / PITCH as f32).floor() as i64,
            (point.at.1 / PITCH as f32).floor() as i64,
        );
        buckets.entry(key).or_default().push(point);
    }

    let reach = how.reach.max(0.0) / UNIT_SIZE;
    let steps = (how.reach.max(0.0) / CHUNK_SIZE).ceil() as u32;
    let sides = border(cells);
    let mut work: Vec<(Cell, bool)> = Vec::new();
    if w_in > 0.0 {
        let edge = sides.iter().map(|(cell, _)| *cell);
        work.extend(grow(edge, steps, |cell| cells.contains(&cell)).into_iter().map(|cell| (cell, true)));
    }
    if w_out > 0.0 {
        let beyond = sides.iter().filter_map(|(cell, side)| cell.across(*side));
        work.extend(grow(beyond, steps, |cell| !cells.contains(&cell)).into_iter().map(|cell| (cell, false)));
    }

    let mut edits: BTreeMap<(u32, u32), Vec<Edit>> = BTreeMap::new();
    let mut moved: BTreeSet<Cell> = BTreeSet::new();
    for (cell, inside) in work {
        let Some(before) = chunk_heights(session, cell) else {
            continue;
        };
        // The border points within reach of any vertex of this chunk.
        let near: Vec<&Point> = {
            let (bx, by) = (cell.x as i64, cell.y as i64);
            let r = steps as i64 + 1;
            (bx - r..=bx + r)
                .flat_map(|x| (by - r..=by + r).map(move |y| (x, y)))
                .filter_map(|key| buckets.get(&key))
                .flatten()
                .copied()
                .collect()
        };
        let mut after = before.clone();
        for (index, height) in after.iter_mut().enumerate() {
            if let Some(point) = outer_key(cell, index).and_then(|key| points.get(&key)) {
                *height = point.target;
                continue;
            }
            if reach <= 0.0 {
                continue;
            }
            // The lift at the nearest border points, weighted by the inverse
            // square of their distance, and faded to nothing at the reach.
            let at = lattice(cell, index);
            let (mut sum, mut weight, mut nearest) = (0.0f32, 0.0f32, f32::INFINITY);
            for point in &near {
                let d = ((point.at.0 - at.0).powi(2) + (point.at.1 - at.1).powi(2)).sqrt();
                if d > reach {
                    continue;
                }
                let w = 1.0 / (d * d + 1e-3);
                sum += w * match inside {
                    true => point.lift_in,
                    false => point.lift_out,
                };
                weight += w;
                nearest = nearest.min(d);
            }
            if weight > 0.0 {
                let u = (nearest / reach).clamp(0.0, 1.0);
                *height += sum / weight * (1.0 - u * u * (3.0 - 2.0 * u));
            }
        }
        if after.iter().zip(&before).all(|(a, b)| (a - b).abs() < 1e-4) {
            continue;
        }
        let coord = cell.tile();
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        let Some(index) = cell.chunk_in(tile) else {
            continue;
        };
        let Some(chunk) = tile.chunk_mut(index) else {
            continue;
        };
        heights::set_heights(chunk, &after);
        edits.entry(coord).or_default().push(Edit::Heights {
            chunk: index,
            before,
            after,
        });
        moved.insert(cell);
    }
    // Locked vertices are put back before the shading is worked out, so the
    // shading across the tiles' sides reads the heights that stay.
    for (coord, list) in edits.iter_mut() {
        session.hold_locked(*coord, list);
    }
    let changed = edits
        .values()
        .flatten()
        .filter(|edit| matches!(edit, Edit::Heights { .. }))
        .count();
    reshade_across(session, &moved, &mut edits);
    commit(session, edits, false);
    changed
}

/// Record `edits` in the open history entry, tile by tile, and bring each
/// written tile up to date on screen: read again where `reread` asks for it,
/// otherwise patched live chunk by chunk. A tile whose only change is its
/// shading is patched live either way.
fn commit(session: &mut EditSession, edits: BTreeMap<(u32, u32), Vec<Edit>>, reread: bool) {
    for (coord, edits) in edits {
        if edits.is_empty() {
            continue;
        }
        let key = session.key(coord);
        let chunks: Vec<usize> = edits.iter().filter_map(Edit::chunk).collect();
        let shading_only = edits.iter().all(|edit| matches!(edit, Edit::Normals { .. }));
        session.history.record(&key, edits);
        session.publish(coord);
        match reread && !shading_only {
            true => {
                session.stale.insert(coord);
            }
            false => {
                for chunk in chunks {
                    session.touched(coord, chunk);
                }
            }
        }
    }
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
        true => "Set impassable",
        false => "Clear impassable",
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
        true => ("Cut holes in chunks", u16::MAX),
        false => ("Fill holes in chunks", 0),
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
    let label = format!("Set base texture {}", super::textures::leaf(path));
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
        "Replace {} with {}",
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

/// Which parts of a copied chunk a paste writes. All of them by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    /// `MCVT`, with `MCNR` recomputed from it.
    pub heights: bool,
    /// `MCLY` and `MCAL`: the layers and their blend maps.
    pub textures: bool,
    /// `MCCV`, the colour painted onto the vertices.
    pub shading: bool,
    /// The hole mask.
    pub holes: bool,
    /// `MCLQ` and the flags that declare it.
    pub water: bool,
    /// The area id and the impassable flag.
    pub area: bool,
}

impl Default for Parts {
    fn default() -> Parts {
        Parts {
            heights: true,
            textures: true,
            shading: true,
            holes: true,
            water: true,
            area: true,
        }
    }
}

/// The height a pasted chunk's ground is written at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// The heights as they were copied.
    #[default]
    Absolute,
    /// The copied heights moved up or down by one amount, so that their mean
    /// is the mean of the ground they replace. The shape is kept and the
    /// paste lands at the level of where it is put. Water is not moved: it is
    /// pasted at the level it was copied at.
    Relative,
}

/// A way a copied block is turned before it is pasted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    /// A quarter turn clockwise, seen from above: what was at the block's
    /// north-west corner is at its north-east.
    Clockwise,
    /// East and west changed places.
    Mirror,
}

impl Turn {
    /// Where the value now at `(row, col)` of a grid `n` on a side was before
    /// the turn. Rows run south and columns east, as every per-chunk grid in
    /// the file does, so seen from above a grid is a picture with its rows
    /// down the page.
    fn source(self, n: usize, row: usize, col: usize) -> (usize, usize) {
        match self {
            Turn::Clockwise => (n - 1 - col, row),
            Turn::Mirror => (row, n - 1 - col),
        }
    }
}

/// One copied chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipChunk {
    /// Columns and rows from the clip's corner of least column and row.
    pub offset: (u32, u32),
    heights: Vec<f32>,
    paint: alpha::Paint,
    /// The texture each of `paint`'s layers draws, by path. A layer holds an
    /// index into its own tile's `MTEX`, which means nothing in another tile.
    textures: Vec<String>,
    colours: Option<Vec<u8>>,
    holes: u16,
    pools: Vec<vale_edit::adt::liquid::Pool>,
    area: u32,
    impassable: bool,
}

impl ClipChunk {
    /// The chunk turned in place: every grid it carries moved by `how`. The
    /// area id and the impassable flag are the chunk's and do not move.
    ///
    /// ```text
    /// heights, shading   the 9x9 outer and the 8x8 inner vertices, each grid
    ///                    turned on its own
    /// blend maps         64x64, of which 63 span the chunk: those are turned
    ///                    and the last row and column repeat the one before
    /// holes              the 4x4 mask
    /// water              each pool's 9x9 vertices and 8x8 flags
    /// ```
    fn turned(&self, how: Turn) -> ClipChunk {
        use vale_assets::world::adt::{ALPHA_SIDE, INNER_SIDE, OUTER_SIDE};
        // Which vertex each vertex takes its value from.
        let mut from = vec![0usize; heights::VERTICES];
        for row in 0..OUTER_SIDE {
            for col in 0..OUTER_SIDE {
                let (r, c) = how.source(OUTER_SIDE, row, col);
                if let (Some(to), Some(at)) = (heights::outer(row, col), heights::outer(r, c)) {
                    from[to] = at;
                }
            }
        }
        for row in 0..INNER_SIDE {
            for col in 0..INNER_SIDE {
                let (r, c) = how.source(INNER_SIDE, row, col);
                if let (Some(to), Some(at)) = (heights::inner(row, col), heights::inner(r, c)) {
                    from[to] = at;
                }
            }
        }
        let heights: Vec<f32> = match self.heights.len() == heights::VERTICES {
            true => from.iter().map(|&at| self.heights[at]).collect(),
            false => self.heights.clone(),
        };
        // A shading region is a fixed number of bytes per vertex.
        let colours = self.colours.as_ref().map(|bytes| {
            let each = bytes.len() / heights::VERTICES;
            match each > 0 && bytes.len() == each * heights::VERTICES {
                true => from
                    .iter()
                    .flat_map(|&at| bytes[at * each..(at + 1) * each].iter().copied())
                    .collect(),
                false => bytes.clone(),
            }
        });
        // Texels 0 to 62 span the chunk and texel 63 is not sampled; see
        // `vale_edit::adt::alpha::texel_position`.
        let span = ALPHA_SIDE - 1;
        let mut paint = self.paint.clone();
        for map in &mut paint.maps {
            if map.len() != ALPHA_SIDE * ALPHA_SIDE {
                continue;
            }
            let was = map.clone();
            for ty in 0..ALPHA_SIDE {
                for tx in 0..ALPHA_SIDE {
                    let (r, c) = how.source(span, ty.min(span - 1), tx.min(span - 1));
                    map[ty * ALPHA_SIDE + tx] = was[r * ALPHA_SIDE + c];
                }
            }
        }
        let mut holes = 0u16;
        for row in 0..4 {
            for col in 0..4 {
                let (r, c) = how.source(4, row, col);
                if self.holes & (1 << (r * 4 + c)) != 0 {
                    holes |= 1 << (row * 4 + col);
                }
            }
        }
        ClipChunk {
            offset: self.offset,
            heights,
            paint,
            textures: self.textures.clone(),
            colours,
            holes,
            pools: self
                .pools
                .iter()
                .map(|pool| pool.rearranged(&|n, row, col| how.source(n, row, col)))
                .collect(),
            area: self.area,
            impassable: self.impassable,
        }
    }
}

/// What `Ctrl+C` took: the selected chunks, each by its offset in the block
/// that bounds them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clip {
    pub chunks: Vec<ClipChunk>,
    /// The bounding block's columns and rows.
    pub size: (u32, u32),
}

impl Clip {
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// The block turned as a whole: every chunk moved to its new place in the
    /// block and turned in place, so the ground it carries is the same ground
    /// seen from another side. A quarter turn swaps the block's two sizes.
    ///
    /// What is pasted is the turned ground and nothing that stood on it: a
    /// clip holds no doodads. A texture's own picture is not turned either,
    /// since a layer names a tileset and the tileset repeats the same way up
    /// everywhere.
    pub fn turned(&self, how: Turn) -> Clip {
        let (wide, tall) = self.size;
        let size = match how {
            Turn::Clockwise => (tall, wide),
            Turn::Mirror => (wide, tall),
        };
        let chunks = self
            .chunks
            .iter()
            .map(|chunk| {
                let (x, y) = chunk.offset;
                let mut turned = chunk.turned(how);
                turned.offset = match how {
                    Turn::Clockwise => (tall - 1 - y, x),
                    Turn::Mirror => (wide - 1 - x, y),
                };
                turned
            })
            .collect();
        Clip { chunks, size }
    }

    /// The cells a paste at `at` writes, each with the chunk it takes. The
    /// clip's bounding block is centred on `at`.
    pub fn footprint(&self, at: Cell) -> impl Iterator<Item = (Cell, &ClipChunk)> {
        let corner = (
            at.x.saturating_sub(self.size.0 / 2),
            at.y.saturating_sub(self.size.1 / 2),
        );
        self.chunks.iter().map(move |chunk| {
            let cell = Cell {
                x: corner.0 + chunk.offset.0,
                y: corner.1 + chunk.offset.1,
            };
            (cell, chunk)
        })
    }
}

/// Copy every selected chunk whose tile is open.
pub fn copy(session: &EditSession, cells: &BTreeSet<Cell>) -> Clip {
    let mut clip = Clip::default();
    let mut found: Vec<(Cell, ClipChunk)> = Vec::new();
    for &cell in cells {
        let Some(tile) = session.tiles.get(&cell.tile()) else {
            continue;
        };
        let Some(index) = cell.chunk_in(tile) else {
            continue;
        };
        let Some(chunk) = tile.chunk(index) else {
            continue;
        };
        let names = tile.texture_names();
        let paint = alpha::paint(chunk);
        let textures = paint
            .layers
            .iter()
            .map(|layer| names.get(layer.texture_id as usize).cloned().unwrap_or_default())
            .collect();
        found.push((
            cell,
            ClipChunk {
                offset: (0, 0),
                heights: vale_edit::adt::heights::heights(chunk),
                paint,
                textures,
                colours: vale_edit::adt::colours::capture(tile, index),
                holes: chunk.head().holes(),
                pools: vale_edit::adt::liquid::pools(tile, index),
                area: chunk.head().area_id(),
                impassable: vale_edit::adt::impass::impassable(tile, index),
            },
        ));
    }
    let (Some(left), Some(top)) = (
        found.iter().map(|(cell, _)| cell.x).min(),
        found.iter().map(|(cell, _)| cell.y).min(),
    ) else {
        return clip;
    };
    for (cell, mut chunk) in found {
        chunk.offset = (cell.x - left, cell.y - top);
        clip.size.0 = clip.size.0.max(chunk.offset.0 + 1);
        clip.size.1 = clip.size.1.max(chunk.offset.1 + 1);
        clip.chunks.push(chunk);
    }
    clip
}

/// A chunk's `MCNR` bytes, for the edit that records a change to them.
fn normals_of(tile: &AdtFile, chunk: usize) -> Option<Vec<u8>> {
    tile.chunk(chunk)?
        .region(vale_edit::adt::Region::Normals)
        .map(|sub| sub.data.clone())
}

/// Recompute the normals of the chunks `moved` names and of each one's four
/// neighbours, and add an [`Edit::Normals`] to `edits` for each that changed.
/// A normal is a central difference, so a chunk beside one that moved is
/// shaded from the moved heights too, and a chunk on a tile's side reads the
/// heights across it. Runs after every height is written, since it reads
/// them.
///
/// `pub(crate)` because the height brush shades its own strokes inside each
/// tile and asks this for the chunks on a tile's sides when the stroke ends.
pub(crate) fn reshade_across(
    session: &mut EditSession,
    moved: &BTreeSet<Cell>,
    edits: &mut BTreeMap<(u32, u32), Vec<Edit>>,
) {
    let mut shade: BTreeSet<Cell> = BTreeSet::new();
    for &cell in moved {
        shade.insert(cell);
        shade.extend(cell.neighbours().into_iter().filter_map(|(_, other)| other));
    }
    for (coord, cells) in by_tile(shade.into_iter()) {
        if !session.tiles.contains_key(&coord) {
            continue;
        }
        // The chunks across the tile's sides that these normals read, by
        // their place on this tile's 16x16 grid, taken before the tile is
        // borrowed to write.
        let mut beyond: BTreeMap<(i32, i32), Vec<f32>> = BTreeMap::new();
        for &cell in &cells {
            let (x, y) = cell.within();
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if (0..SIDE as i32).contains(&nx) && (0..SIDE as i32).contains(&ny) {
                    continue;
                }
                if let Some(found) = cell.step(dx, dy).and_then(|other| chunk_heights(session, other)) {
                    beyond.insert((nx, ny), found);
                }
            }
        }
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        let written = edits.entry(coord).or_default();
        for cell in cells {
            let Some(index) = cell.chunk_in(tile) else {
                continue;
            };
            let Some(before) = normals_of(tile, index) else {
                continue;
            };
            heights::recompute_normals_with(tile, index, &|x, y| beyond.get(&(x, y)).cloned());
            let after = normals_of(tile, index).unwrap_or_default();
            if after != before {
                written.push(Edit::Normals {
                    chunk: index,
                    before,
                    after,
                });
            }
        }
    }
}

/// Write `clip` centred on `at`, as one undo entry across every tile it
/// reaches. Returns the cells written; a cell whose tile is not open is
/// skipped.
///
/// Each part is written only where `parts` asks for it and only where it
/// differs from what the chunk holds. Every written tile is read again, since
/// a paste can change the mesh, the texture set and the water at once.
///
/// The pasted block's edge meets the ground around it at whatever height that
/// ground has. [`stitch`] joins the two.
pub fn paste(
    session: &mut EditSession,
    clip: &Clip,
    at: Cell,
    parts: Parts,
    level: Level,
) -> Vec<Cell> {
    session
        .history
        .begin(format!("Paste {} chunks", clip.chunks.len()));
    let written = paste_in(session, clip, at, parts, level);
    session.history.end();
    written
}

/// The clip pasted centred on `at`, with the panel's parts, height and stitch
/// settings, as one undo entry; the pasted footprint becomes the selection.
/// Answers the status line. `Ctrl+V` and the right-click menu's Paste here
/// both come here.
pub fn paste_at(session: &mut EditSession, chunks: &mut Chunks, at: Cell) -> String {
    if chunks.clip.is_empty() {
        return "no chunks have been copied".to_string();
    }
    let (clip, parts, level) = (chunks.clip.clone(), chunks.parts, chunks.level);
    let footprint: BTreeSet<Cell> = clip.footprint(at).map(|(cell, _)| cell).collect();
    // The paste and its stitch are one entry, so one undo takes both back.
    session
        .history
        .begin(format!("Paste {} chunks", clip.chunks.len()));
    let written = paste_in(session, &clip, at, parts, level);
    let stitched = match chunks.stitch_pasted && parts.heights && !written.is_empty() {
        true => stitch_in(session, &footprint, chunks.stitch),
        false => 0,
    };
    session.history.end();
    let said = match stitched {
        0 => format!("pasted onto {} of {} chunks", written.len(), clip.chunks.len()),
        n => format!(
            "pasted onto {} of {} chunks; the stitch changed {n} chunks",
            written.len(),
            clip.chunks.len()
        ),
    };
    // What was asked for is selected, so the outline shows where the paste
    // went even where it changed nothing.
    chunks.select(footprint);
    chunks.primary = Some(at);
    said
}

/// [`paste`], recorded in whatever history entry is open.
fn paste_in(
    session: &mut EditSession,
    clip: &Clip,
    at: Cell,
    parts: Parts,
    level: Level,
) -> Vec<Cell> {
    use vale_edit::adt::{colours, impass, liquid};

    let mut tiles: BTreeMap<(u32, u32), Vec<(Cell, &ClipChunk)>> = BTreeMap::new();
    for (cell, chunk) in clip.footprint(at) {
        if session.tiles.contains_key(&cell.tile()) {
            tiles.entry(cell.tile()).or_default().push((cell, chunk));
        }
    }

    // The one amount a relative paste moves every height by: the mean of the
    // ground being replaced, less the mean of what replaces it.
    let lift = match level {
        Level::Absolute => 0.0,
        Level::Relative => {
            let (mut had, mut brought, mut count) = (0.0f64, 0.0f64, 0usize);
            for (coord, cells) in &tiles {
                let tile = &session.tiles[coord];
                for (cell, chunk) in cells {
                    let Some(open) = cell.chunk_in(tile).and_then(|index| tile.chunk(index)) else {
                        continue;
                    };
                    had += heights::heights(open).iter().map(|&h| f64::from(h)).sum::<f64>();
                    brought += chunk.heights.iter().map(|&h| f64::from(h)).sum::<f64>();
                    count += chunk.heights.len();
                }
            }
            match count {
                0 => 0.0,
                n => ((had - brought) / n as f64) as f32,
            }
        }
    };

    let mut written: Vec<Cell> = Vec::new();
    let mut edits: BTreeMap<(u32, u32), Vec<Edit>> = BTreeMap::new();
    let mut moved: BTreeSet<Cell> = BTreeSet::new();
    for (coord, cells) in tiles {
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        let edits = edits.entry(coord).or_default();
        for (cell, from) in cells {
            let Some(chunk) = cell.chunk_in(tile) else {
                continue;
            };
            let had = edits.len();
            if parts.heights {
                let before = tile.chunk(chunk).map(heights::heights).unwrap_or_default();
                let after: Vec<f32> = from.heights.iter().map(|h| h + lift).collect();
                if before != after {
                    let edit = Edit::Heights {
                        chunk,
                        before,
                        after,
                    };
                    edit.apply(tile);
                    edits.push(edit);
                    moved.insert(cell);
                }
            }
            if parts.textures {
                edits.extend(repaint(tile, chunk, |tile, paint| {
                    let mut wanted = from.paint.clone();
                    for (layer, path) in wanted.layers.iter_mut().zip(&from.textures) {
                        layer.texture_id = tile.name_texture(path);
                    }
                    let changed = *paint != wanted;
                    *paint = wanted;
                    changed
                }));
            }
            if parts.shading {
                let before = colours::capture(tile, chunk);
                if before != from.colours {
                    let edit = Edit::Colours {
                        chunk,
                        before,
                        after: from.colours.clone(),
                    };
                    edit.apply(tile);
                    edits.push(edit);
                }
            }
            if parts.holes {
                let before = tile.chunk(chunk).map(|c| c.head().holes()).unwrap_or(0);
                if before != from.holes {
                    let edit = Edit::Holes {
                        chunk,
                        before,
                        after: from.holes,
                    };
                    edit.apply(tile);
                    edits.push(edit);
                }
            }
            // The water before the flag: both write the chunk's flags word,
            // and each edit records the word as it found it, so the undo
            // takes them back in the reverse order.
            if parts.water {
                let before = vale_edit::ops::ChunkLiquid::capture(tile, chunk);
                if let Some(open) = tile.chunk_mut(chunk) {
                    liquid::set_pools(open, &from.pools);
                }
                let after = vale_edit::ops::ChunkLiquid::capture(tile, chunk);
                if before != after {
                    edits.push(Edit::Liquid {
                        chunk,
                        before: Box::new(before),
                        after: Box::new(after),
                    });
                }
            }
            if parts.area {
                let before = tile.chunk(chunk).map(|c| c.head().area_id()).unwrap_or(0);
                if before != from.area {
                    let edit = Edit::Area {
                        chunk,
                        before,
                        after: from.area,
                    };
                    edit.apply(tile);
                    edits.push(edit);
                }
                let before = tile.chunk(chunk).map(|c| c.head().flags()).unwrap_or(0);
                impass::set_impassable(tile, chunk, from.impassable);
                let after = tile.chunk(chunk).map(|c| c.head().flags()).unwrap_or(0);
                if before != after {
                    edits.push(Edit::Flags {
                        chunk,
                        before,
                        after,
                    });
                }
            }
            if edits.len() > had {
                written.push(cell);
            }
        }
    }
    // Locked vertices are put back before the shading is worked out, as in
    // `stitch_in`.
    for (coord, list) in edits.iter_mut() {
        session.hold_locked(*coord, list);
    }
    reshade_across(session, &moved, &mut edits);
    commit(session, edits, true);
    written
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
        session.bypass_change_detection().status = "chunk selection cleared".to_string();
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
    if control && keys.just_pressed(KeyCode::KeyC) {
        chunks.clip = copy(session, &chunks.selected);
        session.bypass_change_detection().status = match chunks.clip.is_empty() {
            true => "no chunks are selected to copy".to_string(),
            false => format!("copied {} chunks", chunks.clip.chunks.len()),
        };
    }
    if control && keys.just_pressed(KeyCode::KeyV) {
        let status = match (chunks.clip.is_empty(), chunks.at) {
            (true, _) => "no chunks have been copied".to_string(),
            (false, None) => "no chunk under the pointer to paste onto".to_string(),
            (false, Some(at)) => paste_at(session, &mut chunks, at),
        };
        session.bypass_change_detection().status = status;
    }
}

/// `--chunks "<x,y>;<x,y>"`: select the block between the chunks two world
/// positions are over, once both are over open tiles. One position selects
/// its own chunk. `--stitch` then stitches the block to the ground around
/// it. See `crate::Args::chunks`.
fn scripted(
    mut chunks: ResMut<Chunks>,
    mut session: Option<ResMut<EditSession>>,
    args: Res<crate::Args>,
    mut done: Local<bool>,
) {
    let (Some(points), Some(session)) = (args.chunks.as_ref(), session.as_mut()) else {
        return;
    };
    if *done {
        return;
    }
    let cells: Option<Vec<Cell>> = points
        .iter()
        .take(2)
        .map(|&(x, y)| Cell::at(session, x, y))
        .collect();
    let Some(cells) = cells.filter(|cells| !cells.is_empty()) else {
        return;
    };
    *done = true;
    let (from, to) = (cells[0], cells[cells.len() - 1]);
    chunks.select(block(from, to));
    chunks.primary = Some(to);
    info!("--chunks: {} selected", chunks.selected.len());
    if args.stitch {
        let changed = stitch(session, &chunks.selected, chunks.stitch);
        info!("--stitch: {changed} chunks moved");
        session.status = format!("stitched: {changed} chunks moved");
        chunks.page = Page::Stitch;
    }
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
    mut marks: ResMut<crate::marks::Marks>,
    chunks: Res<Chunks>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if !state.editing() || *tool != Tool::Chunks {
        return;
    }
    let Some(session) = session else { return };
    let selected = Color::srgb(0.45, 0.80, 1.0);
    let sweeping = Color::srgb(1.0, 0.85, 0.35);
    let pasting = Color::srgb(0.45, 0.90, 0.55);
    let hovered = Color::srgba(1.0, 1.0, 1.0, 0.7);

    outline(&mut marks, &session, &chunks.selected, selected);
    let swept: BTreeSet<Cell> = chunks.sweeping().iter().copied().collect();
    outline(&mut marks, &session, &swept, sweeping);
    let Some(at) = chunks.at.filter(|_| swept.is_empty()) else {
        return;
    };
    // With control held and something copied, the outline is where `Ctrl+V`
    // would write. Otherwise it is the chunk under the pointer.
    let control = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    match control && !chunks.clip.is_empty() {
        true => {
            let footprint: BTreeSet<Cell> = chunks.clip.footprint(at).map(|(cell, _)| cell).collect();
            outline(&mut marks, &session, &footprint, pasting);
        }
        false => outline(&mut marks, &session, &BTreeSet::from([at]), hovered),
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
    marks: &mut crate::marks::Marks,
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
            marks.ground_line(last, next, colour, crate::marks::Look::Ghosted);
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

    /// Give every chunk of `cells` a sloped ground starting at `base`, so a
    /// paste has a shape to keep.
    fn slope(session: &mut EditSession, cells: &BTreeSet<Cell>, base: f32) {
        for cell in cells {
            let tile = session.tiles.get_mut(&cell.tile()).unwrap();
            let index = cell.chunk_in(tile).unwrap();
            let heights: Vec<f32> = (0..145).map(|n| base + n as f32 * 0.01).collect();
            vale_edit::adt::heights::set_heights(tile.chunk_mut(index).unwrap(), &heights);
        }
    }

    fn heights_of(session: &EditSession, cell: Cell) -> Vec<f32> {
        let tile = &session.tiles[&cell.tile()];
        vale_edit::adt::heights::heights(tile.chunk(cell.chunk_in(tile).unwrap()).unwrap())
    }

    fn base_of(session: &EditSession, cell: Cell) -> String {
        let tile = &session.tiles[&cell.tile()];
        let chunk = tile.chunk(cell.chunk_in(tile).unwrap()).unwrap();
        let id = alpha::paint(chunk).layers[0].texture_id as usize;
        tile.texture_names()[id].clone()
    }

    /// A block copied from one tile and pasted into another carries its
    /// heights, area, holes and base texture, the texture named in the
    /// destination's own list. It is centred on the chunk pasted at, it is one
    /// undo entry, and the undo puts every part back.
    #[test]
    fn a_paste_writes_every_part_and_is_one_entry() {
        let (here, there) = ((31, 49), (33, 49));
        let (mut session, install) = session_with("paste", &[here, there]);
        let source: BTreeSet<Cell> = block(Cell::of(here, (4, 4)), Cell::of(here, (5, 5)))
            .into_iter()
            .collect();
        let grass = "tileset\\test\\grass.blp";
        set_area(&mut session, &source, 87);
        set_holes(&mut session, &source, true);
        set_base(&mut session, &source, grass);
        slope(&mut session, &source, 20.0);
        let depth = session.history.depth_done();
        session.stale.clear();

        let clip = copy(&session, &source);
        assert_eq!((clip.chunks.len(), clip.size), (4, (2, 2)));
        let at = Cell::of(there, (8, 8));
        let written = paste(&mut session, &clip, at, Parts::default(), Level::Absolute);
        let wanted: BTreeSet<Cell> = block(Cell::of(there, (7, 7)), Cell::of(there, (8, 8)))
            .into_iter()
            .collect();
        assert_eq!(written.iter().copied().collect::<BTreeSet<Cell>>(), wanted);
        assert_eq!(session.history.depth_done(), depth + 1);
        assert!(session.stale.contains(&there) && !session.stale.contains(&here));
        for (cell, from) in clip.footprint(at) {
            assert_eq!(area_of(&session, cell), Some(87));
            assert_eq!(heights_of(&session, cell), from.heights);
            assert_eq!(base_of(&session, cell), grass);
        }
        let mut chunks = Chunks::default();
        chunks.select(wanted.iter().copied());
        assert_eq!(chunks.census(&session).cut, 4 * 16);

        // The same paste again changes nothing and records nothing.
        assert!(paste(&mut session, &clip, at, Parts::default(), Level::Absolute).is_empty());
        assert_eq!(session.history.depth_done(), depth + 1);

        undo(&mut session);
        for &cell in &wanted {
            assert_eq!(area_of(&session, cell), Some(12));
            assert!(heights_of(&session, cell).iter().all(|&h| h == 0.0));
            assert_eq!(base_of(&session, cell), BASE);
        }
        assert_eq!(chunks.census(&session).cut, 0);
        assert_eq!(session.tiles[&there].texture_names().len(), 1);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A copied block turned four quarter turns, or mirrored twice, is the
    /// block as copied. One quarter turn swaps its sizes, moves the
    /// north-west chunk to the north-east and that chunk's north-west vertex
    /// to its north-east, and leaves neighbouring chunks agreeing along the
    /// edge they share, so the turned block is still one piece of ground.
    #[test]
    fn a_copied_block_turns_and_mirrors_as_one_piece() {
        use vale_edit::adt::heights::outer;
        let here = (31, 49);
        let (mut session, install) = session_with("turn", &[here]);
        let source: BTreeSet<Cell> = block(Cell::of(here, (4, 4)), Cell::of(here, (5, 6)))
            .into_iter()
            .collect();
        // One plane across the whole block, so neighbouring chunks agree on
        // the vertices they share before anything is turned.
        for cell in &source {
            let tile = session.tiles.get_mut(&cell.tile()).unwrap();
            let index = cell.chunk_in(tile).unwrap();
            let origin = tile.chunk(index).unwrap().head().position();
            let heights: Vec<f32> = (0..vale_edit::adt::heights::VERTICES)
                .map(|vertex| {
                    let [x, y, _] = vale_edit::adt::heights::vertex_position(origin, vertex, 0.0);
                    (x * 0.25 + y * 0.5).round()
                })
                .collect();
            vale_edit::adt::heights::set_heights(tile.chunk_mut(index).unwrap(), &heights);
        }
        // One hole, in the north-west square of the north-west chunk.
        let corner: BTreeSet<Cell> = [Cell::of(here, (4, 4))].into_iter().collect();
        over(&mut session, &corner, "Cut", false, |tile, chunk| {
            let before = tile.chunk(chunk)?.head().holes();
            let edit = Edit::Holes {
                chunk,
                before,
                after: 1,
            };
            edit.apply(tile);
            Some(edit)
        });
        let clip = copy(&session, &source);
        assert_eq!((clip.chunks.len(), clip.size), (6, (2, 3)));

        let mut round = clip.clone();
        for _ in 0..4 {
            round = round.turned(Turn::Clockwise);
        }
        assert_eq!(round, clip, "four quarter turns");
        assert_eq!(clip.turned(Turn::Mirror).turned(Turn::Mirror), clip, "two mirrors");

        let turned = clip.turned(Turn::Clockwise);
        assert_eq!(turned.size, (3, 2));
        let at_offset = |clip: &Clip, offset: (u32, u32)| -> ClipChunk {
            clip.chunks
                .iter()
                .find(|chunk| chunk.offset == offset)
                .cloned()
                .expect("a chunk at that offset")
        };
        let was = at_offset(&clip, (0, 0));
        let now = at_offset(&turned, (2, 0));
        let (north_west, north_east) = (outer(0, 0).unwrap(), outer(0, 8).unwrap());
        assert_eq!(now.heights[north_east], was.heights[north_west]);
        // The hole was bit 0, the north-west square; it is the north-east now.
        assert_eq!((was.holes, now.holes), (1, 1 << 3));
        // Along the edge two turned chunks share, their heights agree.
        let (west, east) = (at_offset(&turned, (0, 0)), at_offset(&turned, (1, 0)));
        for row in 0..9 {
            assert_eq!(
                west.heights[outer(row, 8).unwrap()],
                east.heights[outer(row, 0).unwrap()],
                "row {row}"
            );
        }
        let (north, south) = (at_offset(&turned, (0, 0)), at_offset(&turned, (0, 1)));
        for col in 0..9 {
            assert_eq!(
                north.heights[outer(8, col).unwrap()],
                south.heights[outer(0, col).unwrap()],
                "column {col}"
            );
        }
        // A mirrored block's west column is the copied block's east column.
        let mirrored = clip.turned(Turn::Mirror);
        let (copied, flipped) = (at_offset(&clip, (1, 0)), at_offset(&mirrored, (0, 0)));
        assert_eq!(
            flipped.heights[outer(0, 0).unwrap()],
            copied.heights[outer(0, 8).unwrap()]
        );

        // The turned block pastes as any other does.
        let at = Cell::of(here, (10, 10));
        let written = paste(&mut session, &turned, at, Parts::default(), Level::Absolute);
        assert_eq!(written.len(), 6);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A relative paste keeps the copied shape and lands at the mean height
    /// of the ground it replaces. A part switched off is not written.
    #[test]
    fn a_relative_paste_keeps_the_shape_at_the_level_it_lands_on() {
        let (here, there) = ((31, 49), (33, 49));
        let (mut session, install) = session_with("level", &[here]);
        session
            .tiles
            .insert(there, blank_tile(there.0, there.1, BASE, 50.0, 12));
        let source = BTreeSet::from([Cell::of(here, (4, 4))]);
        slope(&mut session, &source, 20.0);
        set_area(&mut session, &source, 87);
        let clip = copy(&session, &source);
        let at = Cell::of(there, (8, 8));

        let only_area = Parts {
            heights: false,
            ..Parts::default()
        };
        paste(&mut session, &clip, at, only_area, Level::Relative);
        assert_eq!(area_of(&session, at), Some(87));
        assert!(heights_of(&session, at).iter().all(|&h| (h - 50.0).abs() < 1e-3));

        paste(&mut session, &clip, at, Parts::default(), Level::Relative);
        let (pasted, copied) = (heights_of(&session, at), &clip.chunks[0].heights);
        let mean = pasted.iter().sum::<f32>() / pasted.len() as f32;
        assert!((mean - 50.0).abs() < 1e-2, "{mean}");
        for n in 0..pasted.len() {
            let (rose, was) = (pasted[n] - pasted[0], copied[n] - copied[0]);
            assert!((rose - was).abs() < 1e-3, "vertex {n}: {rose} against {was}");
        }
        let _ = std::fs::remove_dir_all(&install);
    }

    /// Set every height of each cell's chunk to `height`, with no entry.
    fn raise(session: &mut EditSession, cells: &BTreeSet<Cell>, height: f32) {
        for cell in cells {
            let tile = session.tiles.get_mut(&cell.tile()).unwrap();
            let index = cell.chunk_in(tile).unwrap();
            let heights = vec![height; heights::VERTICES];
            heights::set_heights(tile.chunk_mut(index).unwrap(), &heights);
        }
    }

    /// One outer vertex's height, by row and column.
    fn height_at(session: &EditSession, cell: Cell, row: usize, col: usize) -> f32 {
        heights_of(session, cell)[heights::outer(row, col).unwrap()]
    }

    /// A raised block stitched with the selection yielding: its border
    /// vertices take the ground's height, the ground does not move, a vertex
    /// halfway to the reach is lifted halfway, and one at the reach keeps its
    /// height. The whole stitch is one entry and one undo puts the shape and
    /// the shading back.
    #[test]
    fn a_stitch_welds_the_border_and_blends_inward() {
        let here = (31, 49);
        let (mut session, install) = session_with("stitch", &[here]);
        let block: BTreeSet<Cell> = block(Cell::of(here, (4, 4)), Cell::of(here, (5, 5)))
            .into_iter()
            .collect();
        raise(&mut session, &block, 20.0);
        let mut chunks = Chunks::default();
        chunks.select(block.iter().copied());
        let border = chunks.census(&session).border;
        assert_eq!(border.sides, 8);
        assert!((border.step - 20.0).abs() < 1e-3, "{border:?}");

        let how = Stitch {
            yields: Yields::Selection,
            reach: CHUNK_SIZE,
        };
        assert_eq!(stitch(&mut session, &block, how), 4);
        assert_eq!(session.history.depth_done(), 1);
        let corner = Cell::of(here, (4, 4));
        // Row 0 is the block's high-x side and column 0 its high-y side: both
        // meet the ground at 0.
        for k in 0..OUTER_SIDE {
            assert!(height_at(&session, corner, 0, k).abs() < 1e-3);
            assert!(height_at(&session, corner, k, 0).abs() < 1e-3);
        }
        // The middle of the chunk is half a chunk from the border.
        let middle = height_at(&session, corner, 4, 4);
        assert!((middle - 10.0).abs() < 0.5, "{middle}");
        // The block's centre is a whole chunk from the border, which is the
        // reach.
        let centre = height_at(&session, corner, 8, 8);
        assert!((centre - 20.0).abs() < 0.5, "{centre}");
        // The ground around the block did not move.
        let outside = Cell::of(here, (3, 4));
        assert!(heights_of(&session, outside).iter().all(|&h| h == 0.0));
        let border = chunks.census(&session).border;
        assert!(border.step < 1e-3, "{border:?}");
        // Its shading did: the ground's edge vertices now sit beside a slope.
        let normals = vale_edit::adt::heights::normals(
            session.tiles[&here].chunk(outside.chunk_in(&session.tiles[&here]).unwrap()).unwrap(),
        );
        assert!(normals[heights::outer(4, 8).unwrap()][1].abs() > 0.01, "{:?}", normals[heights::outer(4, 8).unwrap()]);

        undo(&mut session);
        assert!(heights_of(&session, corner).iter().all(|&h| (h - 20.0).abs() < 1e-3));
        let normals = vale_edit::adt::heights::normals(
            session.tiles[&here].chunk(outside.chunk_in(&session.tiles[&here]).unwrap()).unwrap(),
        );
        assert!(normals[heights::outer(4, 8).unwrap()][1].abs() < 0.01);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// With the ground yielding, the block keeps its heights and the ground's
    /// border vertices rise to meet it; halfway, both copies of a border
    /// vertex end at the mean. A reach of zero welds the border and moves
    /// nothing else.
    #[test]
    fn the_ground_yields_or_both_meet_halfway() {
        let here = (31, 49);
        let (mut session, install) = session_with("yields", &[here]);
        let block = BTreeSet::from([Cell::of(here, (6, 6))]);
        raise(&mut session, &block, 20.0);
        let (inside, outside) = (Cell::of(here, (6, 6)), Cell::of(here, (7, 6)));

        let ground = Stitch {
            yields: Yields::Ground,
            reach: 0.0,
        };
        assert_eq!(stitch(&mut session, &block, ground), 4);
        assert!(heights_of(&session, inside).iter().all(|&h| (h - 20.0).abs() < 1e-3));
        // The ground's copy of the shared side, column 0 of the chunk across
        // the low-y side, is at 20; the rest of that chunk is untouched.
        for k in 0..OUTER_SIDE {
            assert!((height_at(&session, outside, k, 0) - 20.0).abs() < 1e-3);
        }
        assert!(height_at(&session, outside, 4, 1).abs() < 1e-3);
        undo(&mut session);

        // The block, its four neighbours, and the four diagonal ones, which
        // share a corner vertex with it and are within reach of it.
        let both = Stitch {
            yields: Yields::Both,
            reach: CHUNK_SIZE,
        };
        assert_eq!(stitch(&mut session, &block, both), 9);
        assert!((height_at(&session, inside, 4, 8) - 10.0).abs() < 1e-3);
        assert!((height_at(&session, outside, 4, 0) - 10.0).abs() < 1e-3);
        assert!(height_at(&session, outside, 4, 4) > 1.0);
        assert!(height_at(&session, inside, 4, 4) < 19.0);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A block on a tile's side is stitched to the next tile: both tiles are
    /// written in one entry, and the ground's chunk across the border is
    /// reshaded from the heights on this side of it.
    #[test]
    fn a_stitch_crosses_a_tile_border() {
        let (here, there) = ((31, 49), (32, 49));
        let (mut session, install) = session_with("border", &[here]);
        session
            .tiles
            .insert(there, blank_tile(there.0, there.1, BASE, 50.0, 12));
        let block = BTreeSet::from([Cell::of(here, (15, 4))]);
        raise(&mut session, &block, 20.0);
        let mut chunks = Chunks::default();
        chunks.select(block.iter().copied());
        let border = chunks.census(&session).border;
        assert_eq!(border.sides, 4);
        assert!((border.step - 30.0).abs() < 1e-3, "{border:?}");

        let how = Stitch::default();
        assert_eq!(stitch(&mut session, &block, how), 1);
        assert_eq!(session.history.depth_done(), 1);
        let inside = Cell::of(here, (15, 4));
        for k in 1..OUTER_SIDE - 1 {
            assert!((height_at(&session, inside, k, 8) - 50.0).abs() < 1e-3, "the low-y side meets the next tile");
            assert!(height_at(&session, inside, k, 0).abs() < 1e-3, "the high-y side meets this tile");
        }
        // A corner is stored by two chunks of the ground, one in each tile,
        // which disagree there; the one copy the block has takes their mean.
        assert!((height_at(&session, inside, 0, 8) - 25.0).abs() < 1e-3);
        // The chunk across the border is flat at 50, so its edge normal is
        // vertical unless the heights on this side are read.
        let across = Cell::of(there, (0, 4));
        let tile = &session.tiles[&there];
        let normals = vale_edit::adt::heights::normals(tile.chunk(across.chunk_in(tile).unwrap()).unwrap());
        let n = normals[heights::outer(4, 0).unwrap()];
        assert!(n[1].abs() > 0.01, "{n:?}");
        assert!(session.unsaved.contains(&here) && session.unsaved.contains(&there));

        undo(&mut session);
        assert!(heights_of(&session, inside).iter().all(|&h| (h - 20.0).abs() < 1e-3));
        let tile = &session.tiles[&there];
        let normals = vale_edit::adt::heights::normals(tile.chunk(across.chunk_in(tile).unwrap()).unwrap());
        assert!(normals[heights::outer(4, 0).unwrap()][1].abs() < 0.01);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A paste and its stitch share one entry when they are recorded between
    /// one begin and end, as Ctrl+V records them, and one undo takes both
    /// back.
    #[test]
    fn a_pasted_block_is_stitched_in_the_same_entry() {
        let here = (31, 49);
        let (mut session, install) = session_with("pastestitch", &[here]);
        let source = BTreeSet::from([Cell::of(here, (2, 2))]);
        raise(&mut session, &source, 20.0);
        let clip = copy(&session, &source);
        let at = Cell::of(here, (10, 10));

        session.history.begin("Paste 1 chunks");
        let written = paste_in(&mut session, &clip, at, Parts::default(), Level::Absolute);
        assert_eq!(written, vec![at]);
        let footprint: BTreeSet<Cell> = clip.footprint(at).map(|(cell, _)| cell).collect();
        assert_eq!(stitch_in(&mut session, &footprint, Stitch::default()), 1);
        session.history.end();
        assert_eq!(session.history.depth_done(), 1);
        assert!(height_at(&session, at, 0, 4).abs() < 1e-3);
        assert!((height_at(&session, at, 4, 4) - 10.0).abs() < 0.5);

        undo(&mut session);
        assert!(heights_of(&session, at).iter().all(|&h| h == 0.0));
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
