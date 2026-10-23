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
//! The placement tools' rules (`crate::tools::placements::group`), with the one difference a grid
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
//! The grid's rules, the census, the stitch, and copy, turn and paste are in
//! `vale_edit::ops::chunks`. This file holds the tool's state, its gestures,
//! the undo entries and the outline.
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
use vale_assets::world::adt::CHUNK_SIZE;
use vale_client::render::axes;
use vale_client::world::camera::WorldCamera;
use vale_edit::adt::AdtFile;
use vale_edit::ops::chunks::{self as grid, block, border, by_tile, released, repaint, texture_id, tile_cells, Written};
use vale_edit::ops::Edit;

pub use vale_edit::ops::chunks::{copy, Cell, Census, Clip, Level, Parts, Stitch, Turn, Yields, FURTHEST};

/// The session's open tiles, as the chunk grid reads them.
impl grid::Tiles for EditSession {
    fn tile(&self, coord: (u32, u32)) -> Option<&AdtFile> {
        self.tiles.get(&coord)
    }
}

impl grid::TilesMut for EditSession {
    fn tile_mut(&mut self, coord: (u32, u32)) -> Option<&mut AdtFile> {
        self.tiles.get_mut(&coord)
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
            self.counted = Some((stamp, grid::census(session, &self.selected)));
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

/// The area id of one cell's chunk, when its tile is open.
pub fn area_of(session: &EditSession, cell: Cell) -> Option<u32> {
    let tile = session.tiles.get(&cell.tile())?;
    Some(tile.chunk(cell.chunk_in(tile)?)?.head().area_id())
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
    let Written { mut edits, moved, .. } = grid::stitch(session, cells, how);
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
    grid::reshade_across(session, &moved, &mut edits);
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
    let Written {
        mut edits,
        moved,
        cells,
    } = grid::paste(session, clip, at, parts, level);
    // Locked vertices are put back before the shading is worked out, as in
    // `stitch_in`.
    for (coord, list) in edits.iter_mut() {
        session.hold_locked(*coord, list);
    }
    grid::reshade_across(session, &moved, &mut edits);
    commit(session, edits, true);
    cells
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
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
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
    chunks.at = Cell::at(&*session, point.x, point.y);
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
                adding: crate::tools::placements::group::shift(&keys),
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
            match crate::tools::placements::group::shift(&keys) {
                true => chunks.add(tile_cells(at.tile())),
                false => chunks.select(tile_cells(at.tile())),
            }
            chunks.primary = Some(at);
            session.bypass_change_detection().status =
                format!("{} chunks selected", chunks.selected.len());
        }
    }
    if control && keys.just_pressed(KeyCode::KeyC) {
        chunks.clip = copy(&**session, &chunks.selected);
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
/// it. See `crate::args::tools::Tools::chunks`.
fn scripted(
    mut chunks: ResMut<Chunks>,
    mut session: Option<ResMut<EditSession>>,
    args: Res<crate::args::Args>,
    mut done: Local<bool>,
) {
    let (Some(points), Some(session)) = (args.tools.chunks.as_ref(), session.as_mut()) else {
        return;
    };
    if *done {
        return;
    }
    let cells: Option<Vec<Cell>> = points
        .iter()
        .take(2)
        .map(|&(x, y)| Cell::at(&**session, x, y))
        .collect();
    let Some(cells) = cells.filter(|cells| !cells.is_empty()) else {
        return;
    };
    *done = true;
    let (from, to) = (cells[0], cells[cells.len() - 1]);
    chunks.select(block(from, to));
    chunks.primary = Some(to);
    info!("--chunks: {} selected", chunks.selected.len());
    if args.tools.stitch {
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

/// Draw the border of `cells` on the ground. See [`draw`].
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
    use vale_assets::world::adt::{outer_index, HEIGHTS_PER_CHUNK, OUTER_SIDE};
    use vale_edit::adt::{alpha, blank::{blank_tile, CHUNKS}, heights};

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
            assert_eq!(heights_of(&session, cell), from.heights());
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
        let (pasted, copied) = (heights_of(&session, at), clip.chunks[0].heights());
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
            let heights = vec![height; HEIGHTS_PER_CHUNK];
            heights::set_heights(tile.chunk_mut(index).unwrap(), &heights);
        }
    }

    /// One outer vertex's height, by row and column.
    fn height_at(session: &EditSession, cell: Cell, row: usize, col: usize) -> f32 {
        heights_of(session, cell)[outer_index(row, col)]
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
        assert!(normals[outer_index(4, 8)][1].abs() > 0.01, "{:?}", normals[outer_index(4, 8)]);

        undo(&mut session);
        assert!(heights_of(&session, corner).iter().all(|&h| (h - 20.0).abs() < 1e-3));
        let normals = vale_edit::adt::heights::normals(
            session.tiles[&here].chunk(outside.chunk_in(&session.tiles[&here]).unwrap()).unwrap(),
        );
        assert!(normals[outer_index(4, 8)][1].abs() < 0.01);
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
        let n = normals[outer_index(4, 0)];
        assert!(n[1].abs() > 0.01, "{n:?}");
        assert!(session.unsaved.contains(&here) && session.unsaved.contains(&there));

        undo(&mut session);
        assert!(heights_of(&session, inside).iter().all(|&h| (h - 20.0).abs() < 1e-3));
        let tile = &session.tiles[&there];
        let normals = vale_edit::adt::heights::normals(tile.chunk(across.chunk_in(tile).unwrap()).unwrap());
        assert!(normals[outer_index(4, 0)][1].abs() < 0.01);
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
