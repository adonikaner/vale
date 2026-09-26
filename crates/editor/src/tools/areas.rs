//! Which place in the world each chunk belongs to — `MCNK`'s `areaId`.
//!
//! ## The one tool whose result is invisible
//!
//! Every other tool here changes the picture: the ground moves, the paint
//! changes, a tree appears, a hole opens. An area id changes **nothing on
//! screen at all**. It is a row id in a header, read by `GetZoneText`, by the
//! minimap's label, by the world map's highlight, by which chat channels exist
//! where you stand, and by whether a duel may be started — and by nothing that
//! is drawn. See `vale_edit::adt::area`.
//!
//! Two consequences, and they shape the whole tool.
//!
//! **There is no live path and no re-read.** Nothing has to be caught up,
//! because nothing was built from it. This is the cheapest edit in the crate.
//!
//! **The overlay is not a garnish, it is the tool.** A person editing zones
//! cannot see what they are editing or what they have done, so while this tool
//! is chosen **the ground is drawn as its area**: a solid colour per chunk,
//! derived from the area id, washed over the terrain. Without it this is a
//! number field that changes an invisible field to an invisible value.
//!
//! ## It is one 16x16 image per tile, and no geometry at all
//!
//! The first draft drew the *boundaries* — every edge where two chunks disagree
//! — as draped lines. It worked and it was the wrong picture twice over: an
//! outline says where a zone ends and not what is inside it, and every line had
//! to be draped over the ground by sampling it, which is a search per sample.
//!
//! The ground is already addressed per chunk. `Adt::atlas_uv` maps a vertex to
//! its own chunk's cell of a 16x16 grid over the alpha atlas, inset by half a
//! texel so no sample can leave its cell — so a **16x16 image sampled with that
//! same coordinate, nearest**, is exactly this chunk's own texel and nothing
//! else's. `render::terrain::TileTint` is that image and
//! `TerrainParams::tint` is how much of it to show; both are the client's, both
//! are off in the client, and together they cost a kilobyte per tile and not one
//! triangle.
//!
//! What is left drawn as lines is only the **brush preview**, which is a handful
//! of chunks and has to read *against* the wash rather than as part of it.
//!
//! ## …and the proof is a playtest
//!
//! `vale_assets::MapTerrain::area_at` is the client's **only** source for
//! where the character is standing — nothing in the protocol carries a zone —
//! and the local simulation already opens its terrain through this editor's
//! overlay. So the check on this tool needs no server: paint a chunk, press
//! **Playtest**, walk onto it, and the zone text is the new name.
//!
//! ## The eyedropper is half of using it
//!
//! Zone editing is nearly always *extending an area that already exists* rather
//! than inventing one — a subzone grows, a border moves. So `Space` takes the
//! area under the pointer as the one to paint with, which turns "make this like
//! that" into two gestures with no trip to the picker at all.

use crate::session::EditSession;
use crate::tools::Tool;
use vale_client::render::axes;
use vale_client::render::terrain::{TerrainGround, TerrainMaterial, TerrainTile, TileTint};
use vale_client::world::camera::WorldCamera;
use vale_edit::adt::area;
use vale_edit::ops::AreaBrush;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

/// How large the area brush may be, in yards.
///
/// The upper bound is the height brush's own argument one size down: a stroke
/// can only reach tiles this session has open, which is the 3x3, and half of
/// that is the honest cap. The lower bound is well under half a chunk, which is
/// the radius at which a stroke is exactly the square under the pointer.
pub const RADIUS: std::ops::RangeInclusive<f32> = 1.0..=400.0;

/// The key that takes the area under the pointer as the one to paint with.
///
/// `Space`, which is free — the camera flies on `WASD`, `Q` and `E` — and which
/// the texture tool already uses for the other "act on the chunk under the
/// pointer without moving off it" gesture.
const EYEDROP: KeyCode = KeyCode::Space;

/// The zone tool's settings, and what the pointer is over.
#[derive(Resource, Debug, Default)]
pub struct Areas {
    pub brush: AreaBrush,
    /// The chunk under the pointer, as `(tile, chunk)`.
    pub at: Option<((u32, u32), usize)>,
    /// …and the area it currently carries, which is what the panel names and
    /// what `Space` takes.
    pub under: Option<u32>,
    /// The chunks a press would paint, in the tile they are in — drawn by
    /// [`draw`] before anything happens, because nothing about the result is
    /// visible afterwards.
    pub covered: Vec<usize>,
    /// What the picker's search box holds, kept here so it survives the tool
    /// being switched away from and back.
    pub search: String,
    /// …and which folder of the area tree it is showing — see
    /// [`crate::ui::inspector`], which builds the tree out of `AreaTable`'s own
    /// parents.
    pub folder: String,
    /// Whether the press that began this stroke belonged to the world, and
    /// whether one is open — *a drag belongs to where it began*, the rule every
    /// tool in this directory keeps arriving at.
    armed: bool,
    painting: bool,
}

pub struct AreaToolPlugin;

impl Plugin for AreaToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Areas>().add_systems(
            Update,
            // **After the pick**, like every tool here: reading the pointer
            // before the ray is this frame's paints where the pointer was.
            (aim, stroke).chain().after(crate::pick::aim),
        );
        // …and after the stroke, so a chunk painted this frame is washed this
        // frame rather than next. Without it the one thing the tool shows lags
        // the one thing it does by a frame, which on a held stroke reads as the
        // brush trailing the pointer.
        app.add_systems(Update, (draw, wash).after(stroke));
    }
}

/// How much of the area colour is washed over the ground, 0..1.
///
/// Not 1. The point is to see the *zones*, and a flat replacement throws away
/// the shading and the shape of the land with it — a hillside and a valley in
/// one area become one colour and the map stops being a place. Three quarters
/// leaves the terrain's own light and a hint of its texture underneath, which is
/// enough to navigate by while the colour is unmistakable.
const WASH: f32 = 0.75;

/// Draw the ground as its area, while this tool is chosen.
///
/// **The tool is the picture** — see the module comment. What this does is fill
/// each open tile's `TileTint` from its chunks' area ids and raise
/// `TerrainParams::tint` on that tile's ground materials; when the tool is not
/// chosen it lowers them again, which is the whole of putting the world back.
///
/// The image is rewritten only when the tile's areas have actually changed,
/// which is checked by hashing its 256 ids — 256 four-byte reads per tile per
/// frame against a 1 KB upload and a texture re-prepare. That also catches, for
/// free, the three things that change a tile's areas without this tool doing
/// anything: an undo, a redo, and a tile being read again.
fn wash(
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    session: Option<Res<EditSession>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    tiles: Query<(&TerrainTile, &TileTint)>,
    ground: Query<(&ChildOf, &MeshMaterial3d<TerrainMaterial>), With<TerrainGround>>,
    mut shown: Local<HashMap<(u32, u32), u64>>,
) {
    let on = state.editing() && *tool == Tool::Areas;
    // **Every ground material, not only the open tiles'.** Turning the tool off
    // has to put back every tile that was ever washed, including ones that have
    // since streamed out of the session's own 3x3.
    let wanted = if on { WASH } else { 0.0 };
    for (_, handle) in &ground {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            if material.params.tint != wanted {
                material.params.tint = wanted;
            }
        }
    }
    if !on {
        shown.clear();
        return;
    }
    let Some(session) = session else { return };

    for (tile, tint) in &tiles {
        let Some(open) = session.tiles.get(&tile.coord) else {
            continue;
        };
        let grid = grid_of(open);
        // A cheap order-dependent hash of the 256 ids: what is being asked is
        // only "is this the same picture", and a collision costs one frame of a
        // stale wash rather than anything wrong.
        let mut mark = 0xcbf2_9ce4_8422_2325u64;
        for id in grid {
            mark = (mark ^ u64::from(id)).wrapping_mul(0x1000_0000_01b3);
        }
        if shown.get(&tile.coord) == Some(&mark) {
            continue;
        }
        let Some(mut image) = images.get_mut(&tint.0) else {
            // The pixels have not landed yet — the tile hands its images over
            // one a frame. Left unmarked, so it is filled on a later frame.
            continue;
        };
        let Some(data) = image.data.as_mut() else {
            continue;
        };
        for (index, id) in grid.iter().enumerate() {
            let [r, g, b, _] = colour_of(*id).to_srgba().to_u8_array();
            // **`0` is transparent**, which is what leaves a chunk belonging to
            // nowhere looking like ground rather than like a zone of its own.
            // It is a real value and the shipped tiles have it; painting it a
            // colour would say it was somewhere.
            let alpha = if *id == 0 { 0 } else { 255 };
            data[index * 4..index * 4 + 4].copy_from_slice(&[r, g, b, alpha]);
        }
        shown.insert(tile.coord, mark);
    }
}

/// Work out which chunk the pointer is over, what it carries, and what a press
/// would cover.
#[allow(clippy::too_many_arguments)]
fn aim(
    mut areas: ResMut<Areas>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    areas.at = None;
    areas.under = None;
    areas.covered.clear();
    if !state.editing() || *tool != Tool::Areas {
        return;
    }
    let Some(session) = session else { return };
    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);
    // Control and the wheel resizes, which is the gesture both brushes use.
    if in_world && keys.pressed(KeyCode::ControlLeft) && scroll.delta.y != 0.0 {
        areas.brush.radius = (areas.brush.radius * (1.0 + scroll.delta.y * 0.1))
            .clamp(*RADIUS.start(), *RADIUS.end());
    }
    if !in_world {
        return;
    }

    // **The ground as though it had no holes** — see [`crate::pick::solid_under`].
    // A cave mouth is a hole with a building behind it, and its chunks have an
    // area like any others; aiming at the drawn ground would make exactly those
    // unreachable.
    let Some(point) = crate::pick::solid_under(&session, &windows, &camera) else {
        return;
    };
    let coord = vale_assets::tile_for_position(point.x, point.y);
    let Some(tile) = session.tiles.get(&coord) else {
        return;
    };
    let Some(chunk) = vale_edit::adt::heights::chunk_at(tile, point.x, point.y) else {
        return;
    };
    areas.at = Some((coord, chunk));
    areas.under = area::area(tile, chunk);
    // **From the brush's own rule and not a second one**, which is what makes
    // the preview a preview rather than a guess. See `AreaBrush::covers`.
    areas.covered = areas.brush.covers(tile, [point.x, point.y]);

    if keys.just_pressed(EYEDROP) && !wants.wants_keyboard_input() {
        if let Some(id) = areas.under {
            areas.brush.area = id;
        }
    }
}

/// Paint while the button is held.
#[allow(clippy::too_many_arguments)]
fn stroke(
    mut areas: ResMut<Areas>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    // **The release is answered wherever the pointer is**, and always, or the
    // history is left open and the next change folds into it.
    if buttons.just_released(MouseButton::Left) {
        if areas.painting {
            session.history.end();
        }
        areas.armed = false;
        areas.painting = false;
        return;
    }
    if !state.editing() || *tool != Tool::Areas {
        return;
    }
    if buttons.just_pressed(MouseButton::Left) {
        areas.armed = crate::ui::over_the_world(&viewport, &wants, &windows);
    }
    if !buttons.pressed(MouseButton::Left) || !areas.armed {
        return;
    }

    // **Every tile the circle reaches**, on the height brush's own argument: a
    // stroke that stopped at a tile border would leave the zone ending in a
    // straight line exactly there with nothing about either file wrong.
    let Some(point) = crate::pick::solid_under(session, &windows, &camera) else {
        return;
    };
    let brush = vale_edit::ops::AreaBrush {
        radius: areas.brush.radius,
        area: areas.brush.area,
    };
    let mut painted = 0usize;
    for coord in tiles_under(brush.radius, point) {
        let key = session.key(coord);
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        let edits = brush.stroke(tile, [point.x, point.y]);
        if edits.is_empty() {
            continue;
        }
        painted += edits.len();
        if !areas.painting {
            areas.painting = true;
            session.history.begin("Set area");
        }
        session.history.record(&key, edits);
        // **Published and nothing else.** No chunk is marked dirty, no tile is
        // marked stale: nothing on screen was built from an area id. See the
        // module comment.
        session.publish(coord);
    }
    if painted > 0 {
        session.status = format!("area {} on {painted} chunk(s)", brush.area);
    }
}

/// Which tiles a circle of this radius at this point can reach.
///
/// The same walk the two brushes make, and for the same reason: a tile is a
/// 533-yard square and a brush is a circle.
fn tiles_under(radius: f32, at: Vec3) -> Vec<(u32, u32)> {
    let mut found: Vec<(u32, u32)> = Vec::new();
    for dy in [-radius, 0.0, radius] {
        for dx in [-radius, 0.0, radius] {
            let coord = vale_assets::tile_for_position(at.x + dx, at.y + dy);
            if !found.contains(&coord) {
                found.push(coord);
            }
        }
    }
    found
}

/// How many segments each chunk edge of the overlay is drawn in.
///
/// A chunk is 33 yards and a boundary runs across whatever is under it, so the
/// line has to follow the ground rather than cut through the hill it is on —
/// the lesson the hole outline paid for. Four is a sample every eight yards.
const DRAPE: usize = 4;

/// …and how far above the ground it is lifted, in yards. It draws over the
/// world anyway (see [`crate::tools::gizmo::EditorHandles`]); this is so it
/// reads as lying *on* the ground rather than inside it.
const LIFT: f32 = 0.1;

/// Outline what a press would paint.
///
/// **All that is left of the overlay.** The ground itself is drawn as its area
/// by [`wash`] — see the module comment — so what lines are still for is the one
/// thing a wash cannot say: *which chunks the next press takes*. They are drawn
/// in the area's own colour against a ground already washed in it, so the
/// outline is what reads rather than the fill.
///
/// It is a handful of chunks, which is why this can afford to drape at all: each
/// chunk's ground is built once and sampled directly, so there is no search per
/// sample.
fn draw(
    mut gizmos: Gizmos<crate::tools::gizmo::EditorHandles>,
    areas: Res<Areas>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !state.editing() || *tool != Tool::Areas {
        return;
    }
    let (Some(session), Some((coord, _))) = (session, areas.at) else {
        return;
    };
    let Some(tile) = session.tiles.get(&coord) else {
        return;
    };
    // **A light edge and not the area's own colour.** The ground under it is
    // already washed in that colour, so drawing the outline in it would be
    // drawing the preview in the one shade guaranteed to be invisible.
    let colour = Color::srgb(1.0, 1.0, 1.0);
    for &chunk in &areas.covered {
        let Some(square) = tile.chunk(chunk) else {
            continue;
        };
        let ground = vale_edit::adt::heights::ground(square);
        for edge in [Along::X, Along::Y, Along::NegX, Along::NegY] {
            let (from, to) = edge.edge(ground.position);
            run(&mut gizmos, &ground, from, to, colour);
        }
    }
}

/// A tile's 256 area ids, in the order the file lists its chunks.
///
/// `index_y * 16 + index_x`, which is what `vale_edit::ops`' own tests
/// already rest on. Built once per tile per frame: 256 reads of four bytes.
fn grid_of(tile: &vale_edit::adt::AdtFile) -> [u32; 256] {
    let mut grid = [0u32; 256];
    for (index, slot) in grid.iter_mut().enumerate() {
        *slot = tile
            .chunk(index)
            .map(|chunk| chunk.head().area_id())
            .unwrap_or(0);
    }
    grid
}

/// A colour for an area id.
///
/// **A hash and not a table.** There are 1,081 rows in `AreaTable` and the point
/// is only that two neighbouring zones look different; a palette anybody chose
/// would be a hundred colours nobody agreed on and would still collide. The
/// saturation and value are fixed so every zone reads at the same weight
/// against the ground.
fn colour_of(area: u32) -> Color {
    // A cheap integer hash, so neighbouring ids — which zones very often have —
    // land far apart on the wheel rather than next to each other.
    let mut hash = area.wrapping_mul(2_654_435_761);
    hash ^= hash >> 15;
    Color::hsl((hash % 360) as f32, 0.75, 0.60)
}

/// Which neighbour of a chunk, and therefore which of its four edges.
///
/// A chunk's origin is its **maximum** corner and the grid runs in decreasing x
/// and y from it, so [`Along::X`] and [`Along::Y`] alone cover every shared edge
/// in the map exactly once — the other two belong to the chunks on the far side
/// of them. The two negative ones exist only for drawing a single chunk's whole
/// outline, which the brush preview does.
///
/// **It is an enum and not an axis index**, because the first draft used an
/// index and got the pairing wrong: the probe looked at the neighbour along y
/// and the line was drawn along the edge shared with the one along x. Every
/// boundary was therefore drawn one edge away from where it is, which on a
/// staircase of 33-yard squares looks exactly like a boundary and is not one.
/// Naming them makes [`Self::neighbour`], [`Self::next`] and [`Self::edge`]
/// answer about the same thing by construction.
#[derive(Clone, Copy)]
enum Along {
    X,
    Y,
    NegX,
    NegY,
}

impl Along {
    /// The centre of the neighbouring chunk one step this way, in the world.
    ///
    /// **Only the check calls it**, and that is what it is for: the midpoint of
    /// a shared edge is halfway between the two chunk centres, which is true of
    /// a shared edge and of nothing else — so this is how [`Self::edge`] is
    /// held to naming the side it says it does. Stating the neighbour a second
    /// way is the whole value of the test; deriving it from `edge` would be
    /// checking `edge` against itself.
    #[cfg(test)]
    fn neighbour(self, origin: [f32; 3]) -> [f32; 2] {
        let size = vale_assets::world::adt::CHUNK_SIZE;
        let (dx, dy) = match self {
            Along::X => (-1.5, -0.5),
            Along::Y => (-0.5, -1.5),
            Along::NegX => (0.5, -0.5),
            Along::NegY => (-0.5, 0.5),
        };
        [origin[0] + size * dx, origin[1] + size * dy]
    }

    /// …and the edge this chunk shares with it.
    fn edge(self, origin: [f32; 3]) -> ((f32, f32), (f32, f32)) {
        let size = vale_assets::world::adt::CHUNK_SIZE;
        let (x0, y0) = (origin[0], origin[1]);
        let (x1, y1) = (origin[0] - size, origin[1] - size);
        match self {
            Along::X => ((x1, y0), (x1, y1)),
            Along::Y => ((x0, y1), (x1, y1)),
            Along::NegX => ((x0, y0), (x0, y1)),
            Along::NegY => ((x0, y0), (x1, y0)),
        }
    }
}

/// A line between two world points, draped over **this chunk's own** ground in
/// [`DRAPE`] steps.
///
/// It takes the chunk's `ChunkGround` rather than the session because the caller
/// always knows which chunk it is drawing, and asking the session instead means
/// a 256-chunk search per sample — see [`draw`]. The samples are pulled a
/// hair inside the square, because `ChunkGround::height_at` is exclusive at its
/// far edge and every edge drawn here is exactly on one.
fn run(
    gizmos: &mut Gizmos<crate::tools::gizmo::EditorHandles>,
    ground: &vale_assets::world::adt::ChunkGround,
    from: (f32, f32),
    to: (f32, f32),
    colour: Color,
) {
    let size = vale_assets::world::adt::CHUNK_SIZE;
    let inside = |value: f32, origin: f32| value.clamp(origin - size + INSET, origin - INSET);
    let at = |x: f32, y: f32| {
        let z = ground
            .height_at(inside(x, ground.position[0]), inside(y, ground.position[1]))
            // The hole mask is the chunk's own, so a boundary across a cave
            // mouth would drop to the reference height without this.
            .unwrap_or(ground.position[2]);
        Vec3::from(axes::to_bevy([x, y, z + LIFT]))
    };
    let mut last = at(from.0, from.1);
    for step in 1..=DRAPE {
        let k = step as f32 / DRAPE as f32;
        let next = at(from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k);
        gizmos.line(last, next, colour);
        last = next;
    }
}

/// How far inside its own square a sample on a chunk's edge is taken, in yards.
/// Far below the 4.17-yard cell the height is interpolated across, and enough
/// that the exclusive far edge answers.
const INSET: f32 = 0.05;

// ------------------------------------------------------------- impassability

/// **Mark the chunk under the pointer as one a server may not walk on.**
///
/// A second thing this tool does, because it is the same gesture on the same
/// unit: one word per chunk, clicked rather than painted, invisible in the
/// viewport until something draws it. It is a mode of this tool rather than a
/// tool of its own, because a rail tile for one checkbox is hard to find.
///
/// **It is the server's flag and not the client's.** vmangos reads `MCNK`'s
/// `0x02` when it builds `mmaps`, and nothing in this client reads it at all —
/// so marking a hill changes where a *character* may go once the server's maps
/// are rebuilt from the edited files, and changes nothing on screen ever. See
/// `vale_edit::adt::impass`, which carries the measurement that no shipped
/// 1.12 tile sets it.
pub fn set_impassable(
    session: &mut EditSession,
    coord: (u32, u32),
    chunk: usize,
    impassable: bool,
) -> bool {
    let key = session.key(coord);
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return false;
    };
    let before = tile
        .chunks
        .get(chunk)
        .map(|c| c.head().flags())
        .unwrap_or(0);
    if !vale_edit::adt::impass::set_impassable(tile, chunk, impassable) {
        return false;
    }
    let after = tile.chunks[chunk].head().flags();
    session.history.begin(match impassable {
        true => "Impassable".to_string(),
        false => "Passable".to_string(),
    });
    session.history.record(
        &key,
        [vale_edit::ops::Edit::Flags {
            chunk,
            before,
            after,
        }],
    );
    session.history.end();
    session.publish(coord);
    true
}

/// How many of a tile's chunks are marked, for the panel to report.
pub fn impassable_count(session: &EditSession, coord: (u32, u32)) -> Option<usize> {
    session
        .tiles
        .get(&coord)
        .map(vale_edit::adt::impass::count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::adt::CHUNK_SIZE;

    /// **The edge drawn is the edge shared with the neighbour looked at.**
    ///
    /// The one thing here that is silently wrong when it is wrong. The first
    /// draft indexed the two by an axis number and crossed them: the area was
    /// compared against the chunk one step along y and the line was drawn on the
    /// side shared with the one along x, so every boundary in the world was
    /// drawn one square away from where it is. On a staircase of 33-yard
    /// squares that looks exactly like a boundary.
    ///
    /// The check is the geometry rather than a second reading of the table: the
    /// **midpoint of the shared edge is halfway between the two chunk centres**,
    /// which is true of a shared edge and of nothing else.
    #[test]
    fn each_edge_is_the_one_shared_with_the_neighbour_it_names() {
        let origin = [100.0f32, 200.0, 7.0];
        let centre = [origin[0] - CHUNK_SIZE * 0.5, origin[1] - CHUNK_SIZE * 0.5];
        for along in [Along::X, Along::Y, Along::NegX, Along::NegY] {
            let theirs = along.neighbour(origin);
            let (from, to) = along.edge(origin);
            let middle = [(from.0 + to.0) * 0.5, (from.1 + to.1) * 0.5];
            for axis in 0..2 {
                let between = (centre[axis] + theirs[axis]) * 0.5;
                assert!(
                    (middle[axis] - between).abs() < 1e-3,
                    "{axis}: edge middle {middle:?} is not between {centre:?} and {theirs:?}"
                );
            }
        }
    }
}
