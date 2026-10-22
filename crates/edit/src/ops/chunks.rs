//! A selection of map chunks named by their place on the map's chunk grid,
//! and the operations on it that need no renderer: the grid's rules, the
//! census a panel reports, stitching a selection to the ground around it,
//! and a copied block with its turns and its paste.
//!
//! ## A chunk is named by its place on the map's chunk grid
//!
//! [`Cell`] is a chunk's column and row on the grid of the whole map: the
//! tile's coordinate times sixteen plus `MCNK`'s own `indexX` and `indexY`.
//! Named this way a block that crosses a tile border is one rectangle, a
//! neighbour is one step, and a selection survives its tile being closed and
//! opened again. The world axes run against the grid: a larger column is a
//! smaller world y and a larger row is a smaller world x, which is
//! [`crate::adt::blank::chunk_origin`]'s arithmetic.
//!
//! ## Stitching a selection to the ground around it
//!
//! Two pieces of ground meet at a step wherever the two chunks on either side
//! of a chunk boundary store different heights for the vertices they share.
//! [`stitch`] closes the step along the selection's border. For every vertex
//! the selection shares with an unselected chunk in an open tile it chooses
//! one height ([`Yields`]: the ground's, the selection's, or halfway) and
//! writes it to both copies. Then it blends: every vertex within
//! [`Stitch::reach`] yards of the border, on each side that moves, is lifted
//! by the border's own lift there, scaled by a smooth step that is one at the
//! border and zero at the reach. Beyond the reach the shape is kept.
//!
//! ## What a copy holds and what a paste writes
//!
//! A [`Clip`] holds each copied chunk's heights, layers and blend maps,
//! vertex colours, hole mask, water, area id and impassable flag, with its
//! offset in the block that bounds the copy. A layer's texture is kept by
//! path, because a layer's index into `MTEX` means nothing in another tile.
//! [`paste`] writes the parts [`Parts`] names, at the copied height or moved
//! to the level of the ground it replaces ([`Level`]).
//!
//! ## What the caller does with the edits
//!
//! [`stitch`] and [`paste`] write the open tiles and answer the [`Edit`]s that
//! record what they wrote, by tile, with the cells whose heights moved. The
//! caller puts back any locked vertices, passes the moved cells to
//! [`reshade_across`] so the shading on both sides of every moved border is
//! worked out again, and records the edits.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::BuildHasher;

use crate::adt::{alpha, colours, heights, impass, liquid, AdtFile};
use crate::ops::{ChunkLiquid, ChunkPaint, Edit};
use vale_assets::world::adt::{
    chunk_index, chunk_on_grid, inner_index, outer_index, outer_row_col, vertex_offset, CHUNKS_PER_SIDE,
    CHUNK_SIZE, HEIGHTS_PER_CHUNK, OUTER_SIDE, UNIT_SIZE,
};
use vale_assets::world::wdt::MAP_TILES;

/// The open tiles by coordinate, as the caller holds them. A tile that is not
/// open answers `None`, and every operation here skips it.
pub trait Tiles {
    fn tile(&self, coord: (u32, u32)) -> Option<&AdtFile>;
}

/// [`Tiles`] that can be written.
pub trait TilesMut: Tiles {
    fn tile_mut(&mut self, coord: (u32, u32)) -> Option<&mut AdtFile>;
}

impl<S: BuildHasher> Tiles for HashMap<(u32, u32), AdtFile, S> {
    fn tile(&self, coord: (u32, u32)) -> Option<&AdtFile> {
        self.get(&coord)
    }
}

impl<S: BuildHasher> TilesMut for HashMap<(u32, u32), AdtFile, S> {
    fn tile_mut(&mut self, coord: (u32, u32)) -> Option<&mut AdtFile> {
        self.get_mut(&coord)
    }
}

/// Chunks along one side of a tile, as the grid's own integer type.
const SIDE: u32 = CHUNKS_PER_SIDE as u32;

/// Steps of the vertex lattice along one side of a chunk: eight, so the ninth
/// outer vertex of one chunk is the first of the next.
const PITCH: u32 = (OUTER_SIDE - 1) as u32;

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

    /// The cell of chunk `index`, in `MCIN` order, of tile `coord`: the
    /// chunk's place in a file that lists its chunks row by row.
    pub fn of_chunk(coord: (u32, u32), index: usize) -> Cell {
        let (x, y) = vale_assets::world::adt::chunk_xy(index);
        Cell::of(coord, (x as u32, y as u32))
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
    pub fn at(tiles: &impl Tiles, x: f32, y: f32) -> Option<Cell> {
        let coord = vale_assets::tile_for_position(x, y);
        let tile = tiles.tile(coord)?;
        let chunk = heights::chunk_at(tile, x, y)?;
        Some(Cell::of(coord, tile.chunk(chunk)?.head().index()))
    }

    /// Which of `tile`'s chunks this cell is.
    ///
    /// The files list their chunks row by row, so the answer is
    /// `indexY * 16 + indexX` and the chunk's own header confirms it. A tile
    /// that lists them in another order is searched.
    pub fn chunk_in(self, tile: &AdtFile) -> Option<usize> {
        let within = self.within();
        let usual = chunk_index(within.0 as usize, within.1 as usize);
        if tile.chunk(usual).is_some_and(|chunk| chunk.head().index() == within) {
            return Some(usual);
        }
        tile.chunks
            .iter()
            .position(|chunk| chunk.head().index() == within)
    }

    /// The cell `dx` columns and `dy` rows from this one, or `None` past the
    /// edge of the grid.
    pub fn step(self, dx: i32, dy: i32) -> Option<Cell> {
        let x = self.x.checked_add_signed(dx)?;
        let y = self.y.checked_add_signed(dy)?;
        let end = MAP_TILES as u32 * SIDE;
        (x < end && y < end).then_some(Cell { x, y })
    }

    /// The four neighbours, each with the side of this cell's square it
    /// shares. `None` past the edge of the grid.
    pub fn neighbours(self) -> [(Side, Option<Cell>); 4] {
        [
            (Side::LowY, self.step(1, 0)),
            (Side::HighY, self.step(-1, 0)),
            (Side::LowX, self.step(0, 1)),
            (Side::HighX, self.step(0, -1)),
        ]
    }

    /// The neighbour across one side.
    pub fn across(self, side: Side) -> Option<Cell> {
        self.neighbours()
            .into_iter()
            .find(|(had, _)| *had == side)
            .and_then(|(_, cell)| cell)
    }

    /// The eight cells around this one that are on the grid.
    pub fn around(self) -> impl Iterator<Item = Cell> {
        [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]
            .into_iter()
            .filter_map(move |(dx, dy)| self.step(dx, dy))
    }
}

/// One side of a chunk's square, named by the world coordinate that is fixed
/// along it. A chunk's origin is its corner of greatest x and y.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
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
    pub fn ends(self, origin: [f32; 3]) -> ((f32, f32), (f32, f32)) {
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
    pub fn along(self) -> [(usize, usize); OUTER_SIDE] {
        let last = OUTER_SIDE - 1;
        std::array::from_fn(|k| match self {
            Side::HighX => (outer_index(0, k), outer_index(last, k)),
            Side::LowX => (outer_index(last, k), outer_index(0, k)),
            Side::HighY => (outer_index(k, 0), outer_index(k, last)),
            Side::LowY => (outer_index(k, last), outer_index(k, 0)),
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

/// What a selection holds, for a panel. See [`census`].
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

/// What a selection's border meets, for a panel. See [`stitch`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Border {
    /// Sides of selected chunks that meet an unselected chunk in an open
    /// tile. A side on a closed tile or past the edge of the map is not one.
    pub sides: usize,
    /// The largest difference, in yards, between the two heights stored for
    /// one vertex on those sides: the tallest step along the border.
    pub step: f32,
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

/// The 256 cells of one tile.
pub fn tile_cells(coord: (u32, u32)) -> impl Iterator<Item = Cell> {
    (0..SIDE).flat_map(move |y| (0..SIDE).map(move |x| Cell::of(coord, (x, y))))
}

/// Count what `selected` holds.
pub fn census(loaded: &impl Tiles, selected: &BTreeSet<Cell>) -> Census {
    let mut census = Census::default();
    let mut tiles: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut areas: BTreeMap<u32, usize> = BTreeMap::new();
    let mut textures: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (coord, cells) in by_tile(selected.iter().copied()) {
        let Some(tile) = loaded.tile(coord) else {
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
            if head.flags() & impass::IMPASSABLE != 0 {
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
    census.border = seam(loaded, selected).1;
    census
}

/// One cell's chunk's 145 world heights, when its tile is open.
pub fn chunk_heights(tiles: &impl Tiles, cell: Cell) -> Option<Vec<f32>> {
    let tile = tiles.tile(cell.tile())?;
    let chunk = tile.chunk(cell.chunk_in(tile)?)?;
    let found = heights::heights(chunk);
    (!found.is_empty()).then_some(found)
}

/// A vertex's place on the map's vertex lattice, in units of `UNIT_SIZE`:
/// along the grid's columns, then along its rows. An outer vertex lands on
/// whole numbers and an inner one on halves, and a vertex on a chunk's side
/// has the same place from both chunks.
fn lattice(cell: Cell, index: usize) -> (f32, f32) {
    let (dx, dy) = vertex_offset(index);
    (
        (cell.x * PITCH) as f32 + dy / UNIT_SIZE,
        (cell.y * PITCH) as f32 + dx / UNIT_SIZE,
    )
}

/// [`lattice`] for an outer vertex, as whole numbers, or `None` for an inner
/// one.
fn outer_key(cell: Cell, index: usize) -> Option<(u32, u32)> {
    let (row, col) = outer_row_col(index)?;
    Some((cell.x * PITCH + col as u32, cell.y * PITCH + row as u32))
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
fn seam(tiles: &impl Tiles, cells: &BTreeSet<Cell>) -> (BTreeMap<(u32, u32), Join>, Border) {
    let mut joins: BTreeMap<(u32, u32), Join> = BTreeMap::new();
    let mut found = Border::default();
    // Each chunk is decoded once, though a border cell is listed once a side.
    let mut read: BTreeMap<Cell, Option<Vec<f32>>> = BTreeMap::new();
    let mut heights_of = |cell: Cell| -> Option<Vec<f32>> {
        read.entry(cell)
            .or_insert_with(|| chunk_heights(tiles, cell))
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

/// What [`stitch`] or [`paste`] wrote: the edits that record it, by tile,
/// and the cells whose heights moved, for [`reshade_across`].
#[derive(Default)]
pub struct Written {
    pub edits: BTreeMap<(u32, u32), Vec<Edit>>,
    pub moved: BTreeSet<Cell>,
    /// The cells a paste changed anything in. Empty after a stitch.
    pub cells: Vec<Cell>,
}

/// Stitch the selection to the ground around it: write the heights into
/// `tiles` and answer the edits. The shading is not worked out; see the
/// module comment.
pub fn stitch(tiles: &mut impl TilesMut, cells: &BTreeSet<Cell>, how: Stitch) -> Written {
    let (joins, _) = seam(tiles, cells);
    if joins.is_empty() {
        return Written::default();
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
        let Some(before) = chunk_heights(tiles, cell) else {
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
        let Some(tile) = tiles.tile_mut(coord) else {
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
    Written {
        edits,
        moved,
        cells: Vec::new(),
    }
}

/// Cells grouped by the tile each is in, the tiles in order.
pub fn by_tile(cells: impl Iterator<Item = Cell>) -> BTreeMap<(u32, u32), Vec<Cell>> {
    let mut tiles: BTreeMap<(u32, u32), Vec<Cell>> = BTreeMap::new();
    for cell in cells {
        tiles.entry(cell.tile()).or_default().push(cell);
    }
    tiles
}

/// The sides of `cells` that border a cell outside the set, or the edge of
/// the map.
pub fn border(cells: &BTreeSet<Cell>) -> Vec<(Cell, Side)> {
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

/// Change one chunk's paint with `change` and answer the edit, or `None` when
/// `change` answers that nothing moved.
///
/// The chunk's paint is captured before `change` runs, and `change` is given
/// the tile so that it can name a texture in `MTEX`: the capture holds the
/// tile's texture list, so an undo takes the name out again.
pub fn repaint(
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
pub fn texture_id(tile: &AdtFile, path: &str) -> Option<u32> {
    tile.texture_names()
        .iter()
        .position(|name| name.eq_ignore_ascii_case(path))
        .map(|at| at as u32)
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
    pools: Vec<liquid::Pool>,
    area: u32,
    impassable: bool,
}

impl ClipChunk {
    /// The chunk's 145 world heights as copied.
    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

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
        use vale_assets::world::adt::{ALPHA_SIDE, INNER_SIDE};
        // Which vertex each vertex takes its value from.
        let mut from = vec![0usize; HEIGHTS_PER_CHUNK];
        for row in 0..OUTER_SIDE {
            for col in 0..OUTER_SIDE {
                let (r, c) = how.source(OUTER_SIDE, row, col);
                from[outer_index(row, col)] = outer_index(r, c);
            }
        }
        for row in 0..INNER_SIDE {
            for col in 0..INNER_SIDE {
                let (r, c) = how.source(INNER_SIDE, row, col);
                from[inner_index(row, col)] = inner_index(r, c);
            }
        }
        let heights: Vec<f32> = match self.heights.len() == HEIGHTS_PER_CHUNK {
            true => from.iter().map(|&at| self.heights[at]).collect(),
            false => self.heights.clone(),
        };
        // A shading region is a fixed number of bytes per vertex.
        let colours = self.colours.as_ref().map(|bytes| {
            let each = bytes.len() / HEIGHTS_PER_CHUNK;
            match each > 0 && bytes.len() == each * HEIGHTS_PER_CHUNK {
                true => from
                    .iter()
                    .flat_map(|&at| bytes[at * each..(at + 1) * each].iter().copied())
                    .collect(),
                false => bytes.clone(),
            }
        });
        // Texels 0 to 62 span the chunk and texel 63 is not sampled; see
        // `crate::adt::alpha::texel_position`.
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

/// A copied block: the copied chunks, each by its offset in the block
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
pub fn copy(tiles: &impl Tiles, cells: &BTreeSet<Cell>) -> Clip {
    let mut clip = Clip::default();
    let mut found: Vec<(Cell, ClipChunk)> = Vec::new();
    for &cell in cells {
        let Some(tile) = tiles.tile(cell.tile()) else {
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
                heights: heights::heights(chunk),
                paint,
                textures,
                colours: colours::capture(tile, index),
                holes: chunk.head().holes(),
                pools: liquid::pools(tile, index),
                area: chunk.head().area_id(),
                impassable: impass::impassable(tile, index),
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
        .region(crate::adt::Region::Normals)
        .map(|sub| sub.data.clone())
}

/// Recompute the normals of the chunks `moved` names and of each one's four
/// neighbours, and add an [`Edit::Normals`] to `edits` for each that changed.
/// A normal is a central difference, so a chunk beside one that moved is
/// shaded from the moved heights too, and a chunk on a tile's side reads the
/// heights across it. Runs after every height is written, since it reads
/// them.
///
/// The editor's height brush shades its own strokes inside each tile and asks
/// this for the chunks on a tile's sides when the stroke ends.
pub fn reshade_across(
    tiles: &mut impl TilesMut,
    moved: &BTreeSet<Cell>,
    edits: &mut BTreeMap<(u32, u32), Vec<Edit>>,
) {
    let mut shade: BTreeSet<Cell> = BTreeSet::new();
    for &cell in moved {
        shade.insert(cell);
        shade.extend(cell.neighbours().into_iter().filter_map(|(_, other)| other));
    }
    for (coord, cells) in by_tile(shade.into_iter()) {
        if tiles.tile(coord).is_none() {
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
                if chunk_on_grid(nx, ny).is_some() {
                    continue;
                }
                if let Some(found) = cell.step(dx, dy).and_then(|other| chunk_heights(tiles, other)) {
                    beyond.insert((nx, ny), found);
                }
            }
        }
        let Some(tile) = tiles.tile_mut(coord) else {
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

/// Write `clip` centred on `at` into `loaded` and answer the edits, with the
/// cells written; a cell whose tile is not open is skipped.
///
/// Each part is written only where `parts` asks for it and only where it
/// differs from what the chunk holds. The pasted block's edge meets the ground
/// around it at whatever height that ground has; [`stitch`] joins the two.
pub fn paste(loaded: &mut impl TilesMut, clip: &Clip, at: Cell, parts: Parts, level: Level) -> Written {
    let mut tiles: BTreeMap<(u32, u32), Vec<(Cell, &ClipChunk)>> = BTreeMap::new();
    for (cell, chunk) in clip.footprint(at) {
        if loaded.tile(cell.tile()).is_some() {
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
                let Some(tile) = loaded.tile(*coord) else {
                    continue;
                };
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
        let Some(tile) = loaded.tile_mut(coord) else {
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
                let before = ChunkLiquid::capture(tile, chunk);
                if let Some(open) = tile.chunk_mut(chunk) {
                    liquid::set_pools(open, &from.pools);
                }
                let after = ChunkLiquid::capture(tile, chunk);
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
    Written {
        edits,
        moved,
        cells: written,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adt::blank::{blank_tile, chunk_origin, CHUNKS};

    const BASE: &str = "tileset\\test\\base.blp";

    /// Blank tiles at `coords`.
    fn tiles_at(coords: &[(u32, u32)]) -> HashMap<(u32, u32), AdtFile> {
        coords
            .iter()
            .map(|&coord| (coord, blank_tile(coord.0, coord.1, BASE, 0.0, 12)))
            .collect()
    }

    /// A cell names the chunk the file has at that place: its tile, its index
    /// in the file, and the world position `tile_for_position` and `chunk_at`
    /// answer for. A step of one column is one chunk towards smaller y, and
    /// from the last column it crosses into the next tile.
    #[test]
    fn a_cell_is_the_chunk_at_that_place_on_the_grid() {
        let here = (31, 49);
        let tiles = tiles_at(&[here, (32, 49), (31, 50)]);
        let tile = &tiles[&here];
        for index in 0..CHUNKS {
            let head = tile.chunk(index).unwrap().head();
            let cell = Cell::of(here, head.index());
            assert_eq!(cell.tile(), here);
            assert_eq!(cell.chunk_in(tile), Some(index));
            let origin = chunk_origin(here.0, here.1, index);
            let centre = [origin[0] - CHUNK_SIZE * 0.5, origin[1] - CHUNK_SIZE * 0.5];
            assert_eq!(Cell::at(&tiles, centre[0], centre[1]), Some(cell), "chunk {index}");
            // One column on is one chunk towards smaller y; one row on is one
            // chunk towards smaller x.
            let column = Cell { x: cell.x + 1, y: cell.y };
            let row = Cell { x: cell.x, y: cell.y + 1 };
            assert_eq!(Cell::at(&tiles, centre[0], centre[1] - CHUNK_SIZE), Some(column));
            assert_eq!(Cell::at(&tiles, centre[0] - CHUNK_SIZE, centre[1]), Some(row));
        }
        assert_eq!(Cell::of_chunk(here, 17), Cell::of(here, (1, 1)));
        assert_eq!(Cell::of(here, (15, 0)).neighbours()[0].1.unwrap().tile(), (32, 49));
        assert_eq!(Cell::of(here, (0, 15)).neighbours()[2].1.unwrap().tile(), (31, 50));
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
            let at = chunk_origin(tile.0, tile.1, chunk_index(within.0 as usize, within.1 as usize));
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

    /// A copied block turned four quarter turns, or mirrored twice, is the
    /// block as copied. One quarter turn swaps its sizes, moves the
    /// north-west chunk to the north-east and that chunk's north-west vertex
    /// to its north-east, and leaves neighbouring chunks agreeing along the
    /// edge they share, so the turned block is still one piece of ground.
    #[test]
    fn a_copied_block_turns_and_mirrors_as_one_piece() {
        let here = (31, 49);
        let mut tiles = tiles_at(&[here]);
        let source: BTreeSet<Cell> = block(Cell::of(here, (4, 4)), Cell::of(here, (5, 6)))
            .into_iter()
            .collect();
        // One plane across the whole block, so neighbouring chunks agree on
        // the vertices they share before anything is turned.
        for cell in &source {
            let tile = tiles.get_mut(&cell.tile()).unwrap();
            let index = cell.chunk_in(tile).unwrap();
            let origin = tile.chunk(index).unwrap().head().position();
            let heights: Vec<f32> = (0..HEIGHTS_PER_CHUNK)
                .map(|vertex| {
                    let [x, y, _] = heights::vertex_position(origin, vertex, 0.0);
                    (x * 0.25 + y * 0.5).round()
                })
                .collect();
            heights::set_heights(tile.chunk_mut(index).unwrap(), &heights);
        }
        // One hole, in the north-west square of the north-west chunk.
        {
            let corner = Cell::of(here, (4, 4));
            let tile = tiles.get_mut(&here).unwrap();
            let chunk = corner.chunk_in(tile).unwrap();
            let before = tile.chunk(chunk).unwrap().head().holes();
            Edit::Holes {
                chunk,
                before,
                after: 1,
            }
            .apply(tile);
        }
        let clip = copy(&tiles, &source);
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
        let (north_west, north_east) = (outer_index(0, 0), outer_index(0, 8));
        assert_eq!(now.heights[north_east], was.heights[north_west]);
        // The hole was bit 0, the north-west square; it is the north-east now.
        assert_eq!((was.holes, now.holes), (1, 1 << 3));
        // Along the edge two turned chunks share, their heights agree.
        let (west, east) = (at_offset(&turned, (0, 0)), at_offset(&turned, (1, 0)));
        for row in 0..OUTER_SIDE {
            assert_eq!(
                west.heights[outer_index(row, 8)],
                east.heights[outer_index(row, 0)],
                "row {row}"
            );
        }
        let (north, south) = (at_offset(&turned, (0, 0)), at_offset(&turned, (0, 1)));
        for col in 0..OUTER_SIDE {
            assert_eq!(
                north.heights[outer_index(8, col)],
                south.heights[outer_index(0, col)],
                "column {col}"
            );
        }
        // A mirrored block's west column is the copied block's east column.
        let mirrored = clip.turned(Turn::Mirror);
        let (copied, flipped) = (at_offset(&clip, (1, 0)), at_offset(&mirrored, (0, 0)));
        assert_eq!(flipped.heights[outer_index(0, 0)], copied.heights[outer_index(0, 8)]);

        // The turned block pastes as any other does.
        let at = Cell::of(here, (10, 10));
        let written = paste(&mut tiles, &turned, at, Parts::default(), Level::Absolute);
        assert_eq!(written.cells.len(), 6);
    }
}
