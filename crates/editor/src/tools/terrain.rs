//! The height brush: raise, lower, flatten, smooth.
//!
//! ## The edit loop
//!
//! [`super::open_tiles`] opens the tiles; it belongs to the `tools` module, not
//! to this tool. This tool runs five steps:
//!
//! 1. [`stroke`] runs the brush while the left button is down, applies the edits
//!    to the open tile, and folds them into one entry on the history.
//! 2. [`live_ground`] writes the changed chunks' vertices straight into the
//!    meshes already on the GPU, in the frame the mouse moved.
//! 3. [`live_foliage`] rebuilds the grass on those chunks once the button comes
//!    up. Grass is the one thing standing on the ground that a vertex patch
//!    does not move.
//! 4. [`publish`] writes the edited tile's bytes into the overlay when the
//!    button is released, so the archives return the edited bytes.
//! 5. [`remesh`] forgets a tile the live path could not reach, so the streamer
//!    reads it again.
//!
//! None of the five runs during a playtest. See [`crate::playtest`].
//!
//! ## What the tool does besides the five brush modes
//!
//! ```text
//! flatten        to the height under the pointer or a typed one; both ways,
//!                or only filling, or only cutting; level, or a plane tilted
//!                toward a compass bearing through where the stroke began
//! select         the left button chooses vertices in place of sculpting;
//!                the panel then moves, levels, tilts or smooths them, or
//!                keeps strokes inside them ([`Terrain::vertices`],
//!                [`Terrain::only_inside`])
//! lock           the selection, or a tile's four sides, is added to the
//!                tile's locked vertices, which no height operation moves.
//!                Stored in the project; see `EditSession::locked` and
//!                `EditSession::hold_locked`
//! tilt           a tilted flatten, and a selection with a tilt set, draw
//!                the plane's slope: a level line, the plane's line and an
//!                arrow uphill ([`draw_tilt`])
//! tile sides     a stroke shades each tile from its own heights, and the
//!                release shades the chunks on a tile's side again from both
//!                tiles ([`shade_seams`])
//! objects        what stands on moved ground is carried at the release
//!                ([`carry`])
//! ```
//!
//! ## Patching the ground while the button is held
//!
//! Rebuilding a tile means the client's streamer reads the file, meshes 256
//! chunks and decodes eleven tilesets. That takes a third of a second in a
//! debug build, which is too slow for every frame. So the ground is not rebuilt
//! while a stroke is held. [`live_ground`] rewrites the positions and normals
//! of the chunks the stroke touched, in place, in the buffers the draw groups
//! already hold: four chunks at 320 vertices each rather than a tile at 81,840.
//!
//! Three items make this possible, and each is documented where it is
//! defined: `vale_edit::adt::mesh` for where a chunk's vertices sit in a
//! tile's buffer, `vale_client::render::terrain::GroundSources` for where each
//! of a draw group's vertices came from, and
//! `vale_client::render::terrain::LiveEdits` for the two costs of keeping
//! either.
//!
//! [`remesh`] is the fallback, not the normal path. A tile whose draw groups
//! carry no sources is read again instead. That happens in the client's own
//! build, and for a tile that arrived before the switch was set.
//!
//! ## What a patched vertex does not move
//!
//! A live patch moves the ground's own positions and normals and nothing else.
//! Anything the tile built from those heights stays where it was. The foliage
//! is the visible case: the tufts are a merged mesh at the heights the chunk had
//! when the mesh was made, so an undone raise left a chunk's grass in the air
//! where the hill had been. [`live_foliage`] rebuilds it. Foliage is the only
//! population that needs a live rebuild, because it is the only one placed by
//! the terrain's own heights.
//!
//! A doodad or a building stands at the absolute height in its record, so
//! raising the ground under a tree buries it. When the stroke ends, [`carry`]
//! moves every placement over ground the stroke moved by as much as the
//! ground under it moved, in the stroke's own undo entry
//! ([`Terrain::objects_follow`], on by default). It runs at the release and not
//! per frame, because it reads the ground under every placement of the tile.
//!
//! Other items read the heights and are left to a re-read:
//!
//! * `recompute_normals` stops at the tile's edge, so a stroke across a border
//!   leaves a shading seam.
//!
//! The draw group's `Aabb` is probably no longer one of them. It is baked when
//! the mesh is built, so ground raised far above its original range could be
//! frustum-culled early. Bevy 0.19's `calculate_bounds` has a second query,
//! `Or<(AssetChanged<Mesh3d>, Changed<Mesh3d>)>`, that recomputes an existing
//! `Aabb` when the mesh asset is mutated. [`live_ground`] mutates the mesh
//! through `Assets::get_mut`, which raises `Modified`. This comes from reading
//! Bevy's source and has not been measured. It is recorded because the `Aabb`
//! was a candidate cause of a reported artefact, and this makes it less likely.
//!
//! Reading the tile again fixes all of these. [`crate::playtest`] does that on
//! its way into a login, so a playtest draws ground built from the bytes, not
//! patched in place.

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::Tool;
use crate::tools::{Wheel, CORE};
use vale_client::render::foliage::TileFoliagePlans;
use vale_client::render::terrain::{GroundSources, LoadedTiles, TerrainTile};
use super::chunks::Cell;
use std::collections::{BTreeMap, BTreeSet};
use vale_edit::ops::vertices::{Mask, Selected};
use vale_edit::ops::Edit;
use vale_edit::ops::{Brush, Falloff, Mode, Shape};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

/// How large the height brush may be, in yards.
///
/// The upper bound follows from the open block. A stroke can only reach tiles
/// this session has open, which is [`crate::OPEN_BLOCK`] yards across, so a
/// brush wider than that paints a circle with pieces missing rather than a
/// bigger circle. The cap is half of it: a brush centred anywhere in the middle
/// tile still lands entirely on open ground.
///
/// The bound is derived from [`crate::REACH`], not written as a number, so the
/// two cannot drift. A run that narrows the block with `--reach` keeps this cap.
/// That is harmless: a brush past the open ground paints the open part of its
/// circle and nothing else. Earlier values were 200, a guess that refused work
/// the tool can do, and then 800, half the 3x3 the editor used to stream.
pub const RADIUS: std::ops::RangeInclusive<f32> = 0.5..=(crate::OPEN_BLOCK / 2.0);

/// How strong the height brush may be, in yards a second, for raise and lower.
///
/// The upper bound is set high enough that it does not limit anything. A height
/// is an `f32` in the file and a stroke moves it by
/// `strength * weight * seconds`, so a large number only moves the ground a long
/// way in one frame, which is what a user who types a large number wants. The
/// previous ceiling of 60 was a guess.
pub const STRENGTH: std::ops::RangeInclusive<f32> = 0.0..=100_000.0;

/// How strong the height brush may be for flatten and smooth, the two modes
/// that converge on a height rather than climb.
///
/// The unit is a fraction of the remaining distance a second, and
/// `Brush::moved` clamps one step to 1.0 of it. At sixty frames a second any
/// value above about 60 already covers the whole distance in one frame. The
/// range is wide only so that no typed value is refused.
pub const RATE: std::ops::RangeInclusive<f32> = 0.0..=1_000.0;

/// The terrain tool's settings.
#[derive(Resource, Debug, Clone)]
pub struct Terrain {
    pub brush: Brush,
    /// What [`Mode::Flatten`] flattens toward when the stroke begins: the height
    /// under the pointer at that moment, so a flatten is "make it like here".
    pub flatten_to_cursor: bool,
    /// Whether the doodads and buildings over ground a stroke moves are moved
    /// by as much when the stroke ends, in the stroke's own undo entry. See
    /// `vale_edit::ops::follow`.
    pub objects_follow: bool,
    /// The height a flatten goes to while [`Self::flatten_to_cursor`] is off.
    pub flatten_height: f32,
    /// How steep the plane a flatten goes to is, in degrees from level. Zero
    /// is a level flatten.
    pub tilt_angle: f32,
    /// The compass bearing the tilted plane rises toward, in degrees: 0 is
    /// north and 90 east.
    pub tilt_toward: f32,
    /// Whether the left button selects vertices in place of sculpting. See
    /// [`Self::vertices`].
    pub selecting: bool,
    /// The selected vertices, per open tile. Chosen by painting over them
    /// while [`Self::selecting`] is on, then changed from the panel, used to
    /// keep strokes inside them by [`Self::only_inside`], or added to the
    /// tile's locks. See `vale_edit::ops::vertices`.
    pub vertices: bevy::platform::collections::HashMap<(u32, u32), Selected>,
    /// Whether a brush stroke moves only the selected vertices.
    pub only_inside: bool,
    /// How many locked vertex entries the open tiles hold, for the panel.
    /// Written by [`vertex_asks`] each frame the tool is chosen.
    pub locked: usize,
    /// What the panel asks to be done with the selection, taken by
    /// [`vertex_asks`]. The panel holds no session.
    pub ask: Option<VertexAsk>,
    /// How many entries the selection holds and their mean height, for the
    /// panel. Written by [`vertex_asks`] each frame the tool is chosen.
    pub selected: usize,
    pub selected_mean: f32,
    /// The selection's centre, where a tilt pivots.
    pub selected_centre: [f32; 2],
    /// The selection's own tilt: degrees from level and the bearing uphill,
    /// as the flatten's are. Back to level whenever the selection changes,
    /// so the sliders say how far this selection has been tilted.
    pub vertex_tilt: [f32; 2],
    /// Whether a tilt puts the selection on a flat slope through its centre,
    /// in place of leaning it with its bumps kept.
    pub tilt_flat: bool,
    /// The slope this selection has been given so far, and the point it
    /// pivots on. A lean adds only the change, so the sliders can be dragged
    /// back and forth. See [`VertexAsk::Tilt`].
    tilt_applied: [f32; 2],
    tilt_pivot: Option<[f32; 2]>,
    /// The count and centre the selection had when its tilt was last reset.
    tilt_for: (usize, [f32; 2]),
}

/// What the panel asks to be done with the selected vertices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VertexAsk {
    /// Move them all by one amount, so that their mean height is this.
    MoveTo(f32),
    /// Put every one at their mean height.
    Level,
    /// Tilt them by [`Terrain::vertex_tilt`], pivoting on their centre.
    /// Asked on every change of the sliders, so the selection follows them.
    ///
    /// By default the selection leans: each vertex moves by the change in a
    /// plane's height under it, so its bumps are kept, and only the change
    /// since the last ask is added. With [`Terrain::tilt_flat`] each vertex
    /// is put on the plane through the centre at the mean height, a flat
    /// ramp; the mean does not move, so asking again is stable.
    Tilt,
    /// Move each part of the way toward the vertices beside it.
    Smooth,
    /// Select nothing.
    Clear,
    /// Add the selection to its tiles' locked vertices.
    Lock,
    /// Take the selection out of its tiles' locked vertices.
    Unlock,
    /// Lock the vertices on all four sides of each tile that holds part of
    /// the selection.
    LockSides,
    /// Unlock every vertex of every open tile.
    UnlockAll,
}

/// How far one Smooth moves a selected vertex toward its neighbours' mean.
const SMOOTH_PASS: f32 = 0.5;

impl Default for Terrain {
    fn default() -> Terrain {
        Terrain {
            brush: Brush::default(),
            flatten_to_cursor: true,
            objects_follow: true,
            flatten_height: 0.0,
            tilt_angle: 0.0,
            tilt_toward: 0.0,
            selecting: false,
            vertices: Default::default(),
            only_inside: false,
            locked: 0,
            ask: None,
            selected: 0,
            selected_mean: 0.0,
            selected_centre: [0.0; 2],
            vertex_tilt: [0.0; 2],
            tilt_flat: false,
            tilt_applied: [0.0; 2],
            tilt_pivot: None,
            tilt_for: (0, [0.0; 2]),
        }
    }
}

/// Whether a stroke is being held.
///
/// It does not name a tile. A brush is a circle in the world and a tile is a
/// 533-yard square, so a stroke near a border edits both sides of it; which
/// tiles those are is asked again every frame, because the pointer moves.
#[derive(Resource, Default)]
struct Held {
    on: bool,
    /// The ground height under every placement of each tile the stroke has
    /// touched, read before the stroke first moved that tile. The release
    /// measures against it. Empty while [`Terrain::objects_follow`] is off.
    standing: bevy::platform::collections::HashMap<(u32, u32), vale_edit::ops::follow::Standing>,
    /// The chunks on a tile's side that the stroke moved. The stroke shades
    /// each tile from that tile's own heights, which is a one-sided
    /// difference on a side; the release shades these again from both tiles.
    /// See [`shade_seams`].
    sides: BTreeSet<Cell>,
}

pub struct TerrainToolPlugin;

impl Plugin for TerrainToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Terrain>()
            .init_resource::<Held>()
            .add_systems(
                Update,
                (
                    // After the pick, which aims the stroke. Run before it, the
                    // stroke would paint at the pointer's position from the
                    // previous frame, and while the camera moves the stroke
                    // would trail the cursor.
                    (
                        stroke,
                        vertex_asks,
                        super::shortcuts,
                        live_ground,
                        live_foliage,
                        publish,
                        remesh,
                    )
                        .chain()
                        .after(crate::pick::aim),
                    draw_brush,
                    draw_vertices,
                    vertices_on_the_command_line,
                ),
            )
            // In `PostUpdate`, before visibility is propagated. The streamer
            // spawns a replacement tile in `Update` and it becomes queryable at
            // the next sync point, so a system in `Update` first sees it one
            // frame after it was spawned. That frame draws two copies of the
            // same ground in the same place, which speckles. Here the tile is
            // hidden on the frame it appears, and the `before` makes the hiding
            // take effect in that frame rather than the next.
            .add_systems(
                PostUpdate,
                swap.before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
            );
    }
}

/// Run the brush while the left button is held.
#[allow(clippy::too_many_arguments)]
fn stroke(
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    mut terrain: ResMut<Terrain>,
    mut held: ResMut<Held>,
    cursor: Res<Cursor>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    state: Res<crate::playtest::Playtest>,
    time: Res<Time>,
    pressure: crate::tablet::Pressure,
) {
    // During a playtest the left button and the wheel belong to the game: the
    // 1.12.1 client steers with the left button and zooms with the wheel.
    if !state.editing() {
        return;
    }
    // A press that landed on a panel belongs to the panel.
    // [`crate::ui::over_the_world`] documents the two answers egui gives and
    // why neither works.
    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);
    // The wheel sets one of three numbers, chosen by the modifier held. See
    // [`Wheel`], the single list, which `camera::fly` also reads to ignore the
    // same notch.
    if in_world && scroll.delta.y != 0.0 {
        let step = 1.0 + scroll.delta.y * 0.1;
        match Wheel::held(&keys) {
            Some(Wheel::Radius) => {
                terrain.brush.radius =
                    (terrain.brush.radius * step).clamp(*RADIUS.start(), *RADIUS.end());
            }
            Some(Wheel::Strength) => {
                // A proportional step from a floor. The ranges span four
                // decades, so a fixed step is too small at the top or too large
                // at the bottom. The floor lets the value rise from zero, which
                // multiplication alone cannot do.
                let range = match terrain.brush.mode {
                    Mode::Raise | Mode::Lower => STRENGTH,
                    _ => RATE,
                };
                terrain.brush.strength =
                    (terrain.brush.strength.max(0.01) * step).clamp(*range.start(), *range.end());
            }
            // The core is a fraction, so it takes a fixed step: a twentieth of
            // the radius a notch, which covers the whole range in a turn of the
            // wheel.
            Some(Wheel::Core) => {
                terrain.brush.core =
                    (terrain.brush.core + scroll.delta.y * 0.05).clamp(*CORE.start(), *CORE.end());
            }
            None => {}
        }
    }

    let Some(session) = session.as_mut() else {
        return;
    };
    if buttons.just_released(MouseButton::Left) {
        // What stood on the ground the stroke moved is moved with it, into
        // the stroke's own entry, before the entry is closed.
        shade_seams(session, &std::mem::take(&mut held.sides));
        let carried = carry(session, std::mem::take(&mut held.standing));
        if carried > 0 {
            session.status = match carried {
                1 => "1 doodad/WMO placement moved with the ground".to_string(),
                n => format!("{n} doodad/WMO placements moved with the ground"),
            };
        }
        session.history.end();
        held.on = false;
        return;
    }
    if *tool != Tool::Terrain {
        return;
    }
    // The panel check applies to the press, not to every frame: a stroke
    // belongs to where it started, as in other paint programs. Checked per
    // frame, a stroke that crossed a panel stopped in the middle and started
    // again on the way back.
    if buttons.just_pressed(MouseButton::Left) && !in_world {
        return;
    }
    // The ground under the pointer, which is also what says the pointer is over
    // a tile this session has open at all.
    let Some(at) = cursor.ground else { return };

    // Selecting vertices: the left button puts the vertices under the brush's
    // footprint into the selection, and takes them out with Shift held. It
    // edits nothing, so no history entry is opened.
    if terrain.selecting {
        if buttons.just_pressed(MouseButton::Left) {
            held.on = true;
        }
        if !buttons.pressed(MouseButton::Left) || !held.on {
            return;
        }
        let on = !(keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight));
        let (radius, shape) = (terrain.brush.radius, terrain.brush.shape);
        for coord in tiles_in_range(radius, at) {
            let Some(tile) = session.tiles.get(&coord) else {
                continue;
            };
            let set = terrain.vertices.entry(coord).or_default();
            set.mark(tile, [at.x, at.y], radius, shape, on);
            if set.is_empty() {
                terrain.vertices.remove(&coord);
            }
        }
        return;
    }

    if buttons.just_pressed(MouseButton::Left) {
        // The flatten target is set once, when the button goes down. Taken
        // from the pointer every frame, it would be the height the brush had
        // just made, and the stroke would never converge. The plane's pivot is
        // the same point, so a tilted flatten is a ramp through where the
        // stroke began.
        if let Mode::Flatten { .. } = terrain.brush.mode {
            let to = match terrain.flatten_to_cursor {
                true => at.z,
                false => terrain.flatten_height,
            };
            terrain.brush.mode = Mode::Flatten { to };
            terrain.brush.pivot = [at.x, at.y];
            terrain.brush.tilt = match terrain.tilt_angle > 0.0 {
                true => Brush::tilted(terrain.tilt_angle, terrain.tilt_toward),
                false => [0.0; 2],
            };
        }
        let label = match terrain.brush.mode {
            Mode::Raise => "Raise terrain",
            Mode::Lower => "Lower terrain",
            Mode::Flatten { .. } => "Flatten terrain",
            Mode::Smooth => "Smooth terrain",
            Mode::Noise => "Roughen terrain",
        };
        session.history.begin(label);
        held.on = true;
    }
    if !buttons.pressed(MouseButton::Left) || !held.on {
        return;
    }

    // Every tile the circle reaches is edited, not only the one the stroke
    // began on. A brush that stopped at the tile border left a step in the
    // ground there: one side raised and the other not, with both files valid.
    // The four corners of the brush's bounding box name the tiles, which is at
    // most four and usually one.
    // A pen's pressure scales this frame's strength and radius. See
    // `crate::tablet::Pressure`.
    let mut brush = terrain.brush;
    let by = pressure.scale();
    brush.strength *= by.strength;
    brush.radius = (brush.radius * by.radius).max(*RADIUS.start());
    let seconds = time.delta_secs();
    // The height a smooth pulls toward is computed once across every tile the
    // brush reaches, before any of them move. Computed per tile, or per chunk
    // as it once was, each piece converges on its own height and the boundary
    // between two pieces becomes a step. See `Brush::weighed`.
    //
    // It is a separate pass for two reasons. The tiles are borrowed immutably
    // to read the mean and mutably to stroke them. A mean taken while the
    // ground is moving would mix heights from before and after the edit.
    let level = match brush.mode {
        Mode::Smooth => {
            let (mut sum, mut weight) = (0.0, 0.0);
            for coord in tiles_under(&brush, at) {
                if let Some(tile) = session.tiles.get(&coord) {
                    let (s, w) = brush.weighed(tile, [at.x, at.y]);
                    sum += s;
                    weight += w;
                }
            }
            (weight > 0.0).then(|| (sum / weight) as f32)
        }
        _ => None,
    };
    for coord in tiles_under(&brush, at) {
        let key = session.key(coord);
        let Some(tile) = session.tiles.get_mut(&coord) else {
            // Not open yet. `open_tiles` opens the 3x3 one tile a frame and
            // will reach this one shortly. Skipping it costs a few frames in
            // which the stroke does not reach that side. Opening it here would
            // be a two-megabyte parse in the middle of a drag.
            continue;
        };
        // Before the stroke first moves this tile. See [`Held::standing`].
        if terrain.objects_follow && !held.standing.contains_key(&coord) {
            held.standing
                .insert(coord, vale_edit::ops::follow::standing(tile));
        }
        // A stroke kept inside the selection does nothing on a tile with
        // none of it. Locked vertices are put back afterwards, by
        // `hold_locked`, as they are for every height operation.
        let mask = match (terrain.only_inside, terrain.vertices.get(&coord)) {
            (true, None) => continue,
            (true, Some(set)) => Some((set, Mask::Confine)),
            (false, _) => None,
        };
        let mut edits = brush.stroke_masked(tile, [at.x, at.y], seconds, level, mask);
        session.hold_locked(coord, &mut edits);
        if edits.is_empty() {
            continue;
        }
        let chunks: Vec<usize> = edits.iter().filter_map(Edit::chunk).collect();
        session.history.record(&key, edits);
        for chunk in chunks {
            session.touched(coord, chunk);
            if let Some(cell) = side_cell(coord, chunk) {
                held.sides.insert(cell);
            }
        }
    }
}

/// The cell of a chunk on one of its tile's four sides, or `None` for a chunk
/// inside the tile. `chunk` is the chunk's place in the file, which is row by
/// row.
fn side_cell(coord: (u32, u32), chunk: usize) -> Option<Cell> {
    let (x, y) = ((chunk % 16) as u32, (chunk / 16) as u32);
    (x == 0 || x == 15 || y == 0 || y == 15).then(|| Cell::of(coord, (x, y)))
}

/// Shade the chunks in `sides`, and the chunks across each tile side from
/// them, from the heights on both sides, and record what changed in the
/// history entry that is open.
///
/// A stroke's own shading reads one tile, so a vertex on a tile's side is
/// shaded from one side of it and the two tiles disagree there: a line of
/// light or shadow along the border. This is `chunks::reshade_across`, which
/// the stitch uses, run over the border chunks a stroke moved.
fn shade_seams(session: &mut EditSession, sides: &BTreeSet<Cell>) {
    if sides.is_empty() {
        return;
    }
    let mut edits: BTreeMap<(u32, u32), Vec<Edit>> = BTreeMap::new();
    super::chunks::reshade_across(session, sides, &mut edits);
    for (coord, edits) in edits {
        if edits.is_empty() {
            continue;
        }
        let key = session.key(coord);
        let chunks: Vec<usize> = edits.iter().filter_map(Edit::chunk).collect();
        session.history.record(&key, edits);
        for chunk in chunks {
            session.touched(coord, chunk);
        }
    }
}

/// Carry out what the panel asked of the selected vertices, and keep the two
/// numbers the panel shows about them.
///
/// A move is a gesture: the panel's height field is dragged through many
/// values, and they fold into one undo entry. Each tile's heights are written
/// by `vale_edit::ops::vertices`, the tile sides are shaded from both tiles,
/// and what stands on the moved ground is carried, as at the end of a stroke.
///
/// The tiles are published once the left button is up. A drag changes the
/// selection every frame, and laying a tile out is two megabytes.
#[allow(clippy::too_many_arguments)]
fn vertex_asks(
    mut session: Option<ResMut<EditSession>>,
    mut terrain: ResMut<Terrain>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    mut unpublished: Local<BTreeSet<(u32, u32)>>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if !state.editing() || *tool != Tool::Terrain {
        return;
    }
    // A selection in a tile that has been closed is let go: its vertices are
    // indices into a file that is no longer open.
    let closed: Vec<(u32, u32)> = terrain
        .vertices
        .keys()
        .copied()
        .filter(|coord| !session.tiles.contains_key(coord))
        .collect();
    if !closed.is_empty() {
        for coord in closed {
            terrain.vertices.remove(&coord);
        }
    }

    // How many are selected, and their centre at their mean height.
    let centre_of = |terrain: &Terrain, session: &EditSession| -> (usize, [f32; 3]) {
        let (mut sum, mut count) = ([0.0f64; 3], 0usize);
        for (coord, set) in &terrain.vertices {
            if let Some(tile) = session.tiles.get(coord) {
                let (s, n) = set.weighed(tile);
                for axis in 0..3 {
                    sum[axis] += s[axis];
                }
                count += n;
            }
        }
        match count {
            0 => (0, [0.0; 3]),
            n => (n, sum.map(|axis| (axis / n as f64) as f32)),
        }
    };
    let (count, centre) = centre_of(&terrain, session);

    let asked = terrain.ask.take();
    if let Some(ask) = asked {
        match ask {
            VertexAsk::Clear => terrain.vertices.clear(),
            VertexAsk::Lock | VertexAsk::Unlock | VertexAsk::LockSides | VertexAsk::UnlockAll => {
                session.status = change_locks(session, &terrain, ask);
            }
            _ if count == 0 => {}
            _ => {
                match ask {
                    VertexAsk::MoveTo(_) => session.history.begin_gesture(
                        "Move vertices",
                        "terrain vertices",
                        time.elapsed_secs_f64(),
                    ),
                    VertexAsk::Level => session.history.begin("Level vertices"),
                    VertexAsk::Tilt => session.history.begin_gesture(
                        "Tilt vertices",
                        "terrain tilt",
                        time.elapsed_secs_f64(),
                    ),
                    _ => session.history.begin("Smooth vertices"),
                }
                // Every tile's new heights are worked out before any is
                // written, so a smooth reads the ground on both sides of a
                // tile's edge as it was and the two copies of an edge vertex
                // move alike.
                let slope = Brush::tilted(terrain.vertex_tilt[0], terrain.vertex_tilt[1]);
                let pivot = *terrain.tilt_pivot.get_or_insert([centre[0], centre[1]]);
                let change = [
                    slope[0] - terrain.tilt_applied[0],
                    slope[1] - terrain.tilt_applied[1],
                ];
                let flat = terrain.tilt_flat;
                let ground = |x: f32, y: f32| {
                    let coord = vale_assets::tile_for_position(x, y);
                    session
                        .tiles
                        .get(&coord)
                        .and_then(|tile| vale_edit::adt::heights::height_at(tile, x, y))
                };
                let plans: Vec<((u32, u32), vale_edit::ops::vertices::Plan)> = terrain
                    .vertices
                    .iter()
                    .filter_map(|(coord, set)| {
                        let tile = session.tiles.get(coord)?;
                        let plan = match ask {
                            VertexAsk::MoveTo(to) => {
                                set.plan(tile, |at| at[2] + to - centre[2])
                            }
                            VertexAsk::Level => set.plan(tile, |_| centre[2]),
                            VertexAsk::Tilt if flat => set.plan(tile, |at| {
                                vale_edit::ops::vertices::plane(
                                    [pivot[0], pivot[1], centre[2]],
                                    slope,
                                    at,
                                )
                            }),
                            VertexAsk::Tilt => set.plan(tile, |at| {
                                at[2] + vale_edit::ops::vertices::plane(
                                    [pivot[0], pivot[1], 0.0],
                                    change,
                                    at,
                                )
                            }),
                            _ => set.smoothed(tile, SMOOTH_PASS, &ground),
                        };
                        Some((*coord, plan))
                    })
                    .collect();
                let mut standing = Vec::new();
                let mut sides: BTreeSet<Cell> = BTreeSet::new();
                for (coord, plan) in plans {
                    let key = session.key(coord);
                    let Some(tile) = session.tiles.get_mut(&coord) else {
                        continue;
                    };
                    if terrain.objects_follow {
                        standing.push((coord, vale_edit::ops::follow::standing(tile)));
                    }
                    let mut edits = plan.write(tile);
                    session.hold_locked(coord, &mut edits);
                    if edits.is_empty() {
                        continue;
                    }
                    let chunks: Vec<usize> = edits.iter().filter_map(Edit::chunk).collect();
                    session.history.record(&key, edits);
                    for chunk in chunks {
                        session.touched(coord, chunk);
                        sides.extend(side_cell(coord, chunk));
                    }
                    unpublished.insert(coord);
                }
                shade_seams(session, &sides);
                carry(session, standing);
                session.history.end();
            }
        }
    }

    if matches!(asked, Some(VertexAsk::Tilt)) {
        terrain.tilt_applied = Brush::tilted(terrain.vertex_tilt[0], terrain.vertex_tilt[1]);
    }
    let (count, centre) = centre_of(&terrain, session);
    // A different selection starts level: its tilt so far is none, and its
    // pivot is its own centre. A tilt moves no vertex sideways, so the count
    // and the centre across are the same before and after one.
    // A Level takes any lean out, so it starts the tilt over too.
    let shape = (count, [centre[0], centre[1]]);
    match asked {
        Some(VertexAsk::Tilt) => terrain.tilt_for = shape,
        Some(VertexAsk::Level) => terrain.tilt_for = (usize::MAX, [0.0; 2]),
        _ => {}
    }
    if terrain.tilt_for != shape {
        terrain.tilt_for = shape;
        terrain.vertex_tilt[0] = 0.0;
        terrain.tilt_applied = [0.0; 2];
        terrain.tilt_pivot = None;
    }
    // The locked entries across the open tiles. Reading a tile's locks the
    // first time is a file read; after that it is a count over the chunks
    // that have any.
    let open: Vec<(u32, u32)> = session.tiles.keys().copied().collect();
    let locked: usize = open.into_iter().map(|coord| session.locked(coord).count()).sum();
    if terrain.locked != locked {
        terrain.locked = locked;
    }
    if terrain.selected != count || terrain.selected_mean != centre[2] {
        terrain.selected = count;
        terrain.selected_mean = centre[2];
    }
    if terrain.selected_centre != [centre[0], centre[1]] {
        terrain.selected_centre = [centre[0], centre[1]];
    }
    if !unpublished.is_empty() && !buttons.pressed(MouseButton::Left) {
        for coord in std::mem::take(&mut *unpublished) {
            session.publish(coord);
        }
    }
}

/// Carry out one of the four lock requests, and say what it did for the
/// status line. Locks are written to the project at once and are not on the
/// undo history; see `EditSession::set_locked`.
fn change_locks(session: &mut EditSession, terrain: &Terrain, ask: VertexAsk) -> String {
    let mut changed = 0usize;
    if ask == VertexAsk::UnlockAll {
        let open: Vec<(u32, u32)> = session.tiles.keys().copied().collect();
        for coord in open {
            let had = session.locked(coord).count();
            if had > 0 {
                changed += had;
                session.set_locked(coord, Selected::default());
            }
        }
        return format!("unlocked {changed} vertex entries on the open tiles");
    }
    for (&coord, set) in &terrain.vertices {
        let Some(tile) = session.tiles.get(&coord) else {
            continue;
        };
        let sides = (ask == VertexAsk::LockSides).then(|| Selected::tile_sides(tile));
        let mut locks = session.locked(coord).clone();
        changed += match (ask, &sides) {
            (VertexAsk::Unlock, _) => locks.take(set),
            (_, Some(sides)) => locks.add(sides),
            _ => locks.add(set),
        };
        session.set_locked(coord, locks);
    }
    match ask {
        VertexAsk::Unlock => format!("unlocked {changed} selected vertex entries"),
        VertexAsk::LockSides => {
            format!("locked {changed} vertex entries on the sides of the selected tiles")
        }
        _ => format!("locked {changed} selected vertex entries"),
    }
}

/// `--vertices <radius>[,<rise>]`: select the vertices round the camera's
/// target once its tile is open, and move them when a rise is given. See
/// [`crate::Args::vertices`].
fn vertices_on_the_command_line(
    args: Res<crate::Args>,
    session: Option<Res<EditSession>>,
    focus: Res<vale_client::render::focus::WorldFocus>,
    mut terrain: ResMut<Terrain>,
    mut done: Local<u8>,
) {
    let Some((radius, rise)) = args.vertices else {
        return;
    };
    if *done == 0 {
        if let Some((angle, toward)) = args.tilt {
            terrain.tilt_angle = angle;
            terrain.tilt_toward = toward;
        }
    }
    let Some(session) = session else { return };
    let at = focus.position;
    match *done {
        0 => {
            let coord = vale_assets::tile_for_position(at.x, at.y);
            if !focus.present || !session.tiles.contains_key(&coord) {
                return;
            }
            *done = 1;
            terrain.selecting = true;
            terrain.brush.radius = radius;
            let shape = terrain.brush.shape;
            for coord in tiles_in_range(radius, at) {
                if let Some(tile) = session.tiles.get(&coord) {
                    terrain
                        .vertices
                        .entry(coord)
                        .or_default()
                        .mark(tile, [at.x, at.y], radius, shape, true);
                }
            }
        }
        // A frame later, once `vertex_asks` has the selection's mean.
        1 if terrain.selected > 0 => {
            *done = 2;
            info!(
                "--vertices: {} selected at a mean of {:.2} yd",
                terrain.selected, terrain.selected_mean
            );
            if let Some(rise) = rise {
                terrain.ask = Some(VertexAsk::MoveTo(terrain.selected_mean + rise));
            } else if let Some((angle, toward)) = args.tilt {
                // Set once the selection is made, which starts it level.
                terrain.vertex_tilt = [angle, toward];
                terrain.ask = Some(VertexAsk::Tilt);
            }
        }
        2 if (rise.is_some() || args.tilt.is_some()) && terrain.ask.is_none() => {
            *done = 3;
            info!(
                "--vertices: moved to a mean of {:.2} yd, tilt {} toward {}",
                terrain.selected_mean, terrain.vertex_tilt[0], terrain.vertex_tilt[1]
            );
        }
        _ => {}
    }
}

/// Mark every selected vertex near the pointer with a low dome on the ground,
/// and every locked one with the same dome in red. A mark behind a fold of the
/// ground is drawn faint, so the selection is seen through the folds.
///
/// A dome replaced the vertical post a selected vertex first had. A selection
/// of hundreds of posts hid the ground; domes show the patch of ground being
/// chosen. WoWEdit's vertex colour picker marks the vertices under its brush
/// the same way.
///
/// The domes are a fixed size rather than a few pixels: there can be
/// [`VERTEX_MARKS`] of them, and a fixed size leaves their poses unchanged
/// while the camera moves.
fn draw_vertices(
    mut marks: ResMut<crate::marks::Marks>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    terrain: Res<Terrain>,
    focus: Res<vale_client::render::focus::WorldFocus>,
) {
    if !state.editing() || *tool != Tool::Terrain {
        return;
    }
    if terrain.vertices.is_empty() && terrain.locked == 0 {
        return;
    }
    let Some(session) = session else { return };
    // Locked vertices first. A vertex both locked and selected is drawn red
    // alone: two domes in one place would fight over the depth and flicker,
    // and the lock is what decides whether a stroke moves it.
    let world = |point: Vec3| vale_client::render::axes::to_bevy(point.to_array());
    let look = crate::marks::Look::Ghosted;
    let mut drawn = 0;
    let mut red = std::collections::HashSet::new();
    if terrain.locked > 0 {
        for (coord, tile) in &session.tiles {
            let Some(locks) = session.locks_if_read(*coord) else {
                continue;
            };
            for position in locks.positions(tile) {
                let at = Vec3::from(position);
                if at.distance(focus.position) > VERTEX_MARK_RANGE {
                    continue;
                }
                if drawn >= VERTEX_MARKS {
                    break;
                }
                drawn += 1;
                red.insert(at.to_array().map(f32::to_bits));
                marks.cap(world(at), VERTEX_CAP, VERTEX_CAP_HEIGHT, LOCK_COLOUR, look);
            }
        }
    }
    // Yellow while the selection is only a selection, green while strokes
    // are kept inside it.
    let colour = match terrain.only_inside {
        false => Color::srgb(1.0, 0.9, 0.35),
        true => Color::srgb(0.45, 1.0, 0.5),
    };
    // While selecting with a tilt set, the plane a Tilt would put the
    // selection on, through its centre.
    if terrain.selecting && terrain.vertex_tilt[0] > 0.0 && terrain.selected > 0 {
        let [x, y] = terrain.tilt_pivot.unwrap_or(terrain.selected_centre);
        draw_tilt(
            &mut marks,
            Vec3::new(x, y, terrain.selected_mean),
            terrain.brush.radius,
            terrain.vertex_tilt[0],
            terrain.vertex_tilt[1],
            TILT_COLOUR,
        );
    }
    for (coord, set) in &terrain.vertices {
        let Some(tile) = session.tiles.get(coord) else {
            continue;
        };
        for position in set.positions(tile) {
            let at = Vec3::from(position);
            if at.distance(focus.position) > VERTEX_MARK_RANGE {
                continue;
            }
            if drawn >= VERTEX_MARKS {
                return;
            }
            if red.contains(&at.to_array().map(f32::to_bits)) {
                continue;
            }
            drawn += 1;
            marks.cap(world(at), VERTEX_CAP, VERTEX_CAP_HEIGHT, colour, look);
        }
    }
}

/// A selected vertex's dome, in yards: its radius and its height. Neighbouring
/// vertices are about 2.9 yards apart, so the domes of a selection do not
/// touch, and low enough that a slope shows through between them.
const VERTEX_CAP: f32 = 0.7;
const VERTEX_CAP_HEIGHT: f32 = 0.3;

/// The colour of a locked vertex's mark.
const LOCK_COLOUR: Color = Color::srgb(1.0, 0.45, 0.4);

/// The colour of [`draw_tilt`]'s lines.
const TILT_COLOUR: Color = Color::srgb(0.35, 0.95, 1.0);

/// The slope of a plane, drawn at a point on it: a dim level line and the
/// plane's own line, crossing at `at` and running `reach` yards each way
/// along the bearing the plane rises toward, the rise between their uphill
/// ends, and an arrowhead on the plane's uphill end. `at` is in world
/// coordinates, its height on the plane.
///
/// Half the plane is usually under the ground, so the hidden part is drawn
/// faint rather than not at all.
fn draw_tilt(
    marks: &mut crate::marks::Marks,
    at: Vec3,
    reach: f32,
    angle: f32,
    toward: f32,
    colour: Color,
) {
    let world = |point: Vec3| vale_client::render::axes::to_bevy(point.to_array());
    // 0 is north, world +x, and 90 east, world -y. See `Brush::tilted`.
    let bearing = toward.to_radians();
    let along = Vec3::new(bearing.cos(), -bearing.sin(), 0.0);
    let rise = angle.clamp(0.0, 89.0).to_radians().tan();
    let up = along + Vec3::Z * rise;
    let dim = colour.with_alpha(0.45);
    let (level_low, level_high) = (at - along * reach, at + along * reach);
    let (low, high) = (at - up * reach, at + up * reach);
    let look = crate::marks::Look::Ghosted;
    marks.line(world(level_low), world(level_high), dim, look);
    marks.line(world(level_high), world(high), dim, look);
    // The plane's line with a head on its uphill end.
    let thickness = marks.line_radius(world(at)) * 1.3;
    marks.arrow(world(low), world(high), thickness, colour, look);
}

/// How far from the camera's target a selected vertex is still marked, in
/// yards, and the most marks drawn in a frame. A mark is a mesh posed every
/// frame.
const VERTEX_MARK_RANGE: f32 = 400.0;
const VERTEX_MARKS: usize = 20_000;

/// Move every doodad and building of the tiles in `standing` by as much as
/// the ground under it has moved since `standing` was read, and record the
/// moves in the history entry that is open. Returns how many moved.
///
/// `pub(crate)` because the grade tool moves the ground in one press and
/// carries what stands on it the same way. See `vale_edit::ops::follow`.
pub(crate) fn carry(
    session: &mut EditSession,
    standing: impl IntoIterator<Item = ((u32, u32), vale_edit::ops::follow::Standing)>,
) -> usize {
    let mut carried = 0;
    for (coord, mut was) in standing {
        // A locked placement is left where it is: `follow` moves only the
        // placements `was` names.
        let locks = session.placement_locks();
        was.doodads.retain(|(id, _)| !locks.doodads.contains(id));
        was.buildings.retain(|(id, _)| !locks.wmos.contains(id));
        let key = session.key(coord);
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        let done = vale_edit::ops::follow::follow(tile, &was);
        if done.edits.is_empty() {
            continue;
        }
        carried += done.edits.len();
        session.history.record(&key, done.edits);
        // The drawn copies are put where the records now say by the placement
        // tools' own reconciles, which run whatever tool is chosen.
        for index in done.doodads {
            session.moved(coord, index);
        }
        for index in done.buildings {
            session.moved_building(coord, index);
        }
    }
    carried
}

/// Push the changed chunks' vertices into the meshes that are already drawn.
///
/// Runs every frame there is anything to push, which during a stroke is every
/// frame. A draw group is only written to when it actually holds one of the
/// changed chunks, because writing to a `Mesh` marks it changed and Bevy
/// re-uploads the whole of it: a group can span a tile, and a tile is 81,840
/// vertices.
///
/// A tile whose groups carry no [`GroundSources`] cannot be patched — the client
/// builds them that way and so does a tile that arrived before the editor set
/// the switch — and is handed to [`remesh`] instead.
fn live_ground(
    mut session: Option<ResMut<EditSession>>,
    tiles: Query<(&TerrainTile, &Children)>,
    grounds: Query<(&Mesh3d, &GroundSources)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if session.dirty.is_empty() {
        return;
    }
    let dirty: Vec<((u32, u32), Vec<usize>)> = session
        .dirty
        .iter()
        .map(|(coord, chunks)| (*coord, chunks.iter().copied().collect()))
        .collect();

    for (coord, chunks) in dirty {
        let Some(tile) = session.tiles.get(&coord) else {
            continue;
        };
        // The new vertices, as one run per changed chunk, not a map. Every
        // vertex of every draw group of the tile is looked up here, up to
        // 81,840 on every frame of a stroke, and a stroke touches at most four
        // chunks. At that count four integer comparisons are measurably faster
        // than a hash lookup.
        let mut moved: Vec<(u32, Vec<vale_assets::world::adt::MeshVertex>)> = Vec::new();
        for chunk in &chunks {
            let (start, _) = vale_edit::adt::mesh::chunk_span(tile, *chunk);
            let Some(vertices) = vale_edit::adt::mesh::chunk_vertices(tile, *chunk) else {
                continue;
            };
            moved.push((start as u32, vertices));
        }
        if moved.is_empty() {
            continue;
        }
        // Where a tile vertex went, or `None` for one no chunk in this stroke
        // moved.
        let at_index = |index: u32| {
            moved.iter().find_map(|(start, vertices)| {
                let offset = index.checked_sub(*start)? as usize;
                vertices.get(offset)
            })
        };

        let mut reached = false;
        for (_, children) in tiles.iter().filter(|(tile, _)| tile.coord == coord) {
            for child in children.iter() {
                let Ok((mesh, sources)) = grounds.get(child) else {
                    continue;
                };
                reached = true;
                // Which of this group's vertices moved, before touching the
                // mesh at all: a group holding none of them must not be marked
                // changed.
                let hits: Vec<(usize, &vale_assets::world::adt::MeshVertex)> = sources
                    .0
                    .iter()
                    .enumerate()
                    .filter_map(|(at, source)| at_index(*source).map(|vertex| (at, vertex)))
                    .collect();
                if hits.is_empty() {
                    continue;
                }
                let Some(mut mesh) = meshes.get_mut(&mesh.0) else {
                    // The mesh was built for the render world alone and is gone
                    // from here. Nothing to do but read the tile again.
                    reached = false;
                    continue;
                };
                write_vertices(&mut mesh, &hits);
            }
        }
        if reached {
            session.dirty.remove(&coord);
        } else if !tiles.iter().any(|(tile, _)| tile.coord == coord) {
            // The tile is not on screen at all — off the 3x3, or still loading.
            // Its bytes are already right, so whatever streams it next draws the
            // edit; there is nothing to patch and nothing to reload.
            session.dirty.remove(&coord);
        } else {
            session.dirty.remove(&coord);
            session.stale.insert(coord);
        }
    }
}

/// Put the grass back on the ground the brush has moved.
///
/// Runs once the stroke is over, not on every frame of it. A tuft's mesh is
/// merged from the heights it was built with, so it has no vertices to patch
/// the way [`live_ground`] patches the terrain: the chunk's whole lawn is built
/// again. That is a few thousand vertices per chunk per material, which is
/// affordable once a stroke but not sixty times a second. Rebuilding during the
/// stroke would also replace the visible mesh on every frame, which flickers.
///
/// Without this, the foliage floated where the old ground was. An undone raise
/// showed it most clearly: the ground went back and the grass stayed in the air
/// at the height the raise had reached. Everything that reads the ground outside
/// the tile's own mesh has the same problem. See [`remesh`] for the ones this
/// does not cover.
fn live_foliage(
    mut session: Option<ResMut<EditSession>>,
    mut tiles: Query<(&TerrainTile, &mut TileFoliagePlans)>,
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if buttons.pressed(MouseButton::Left) || session.regrow.is_empty() {
        return;
    }
    // Taken whole: a chunk whose tile is not on screen has nothing to replant
    // and must not be kept waiting for one, because the tile's own bytes are
    // already right and whatever streams it next grows the new lawn from them.
    let regrow = std::mem::take(&mut session.regrow);
    let mut dead: Vec<Entity> = Vec::new();
    let mut replanted = 0;
    for (coord, chunks) in regrow {
        let Some(tile) = session.tiles.get(&coord) else {
            continue;
        };
        let Some((_, mut plans)) = tiles.iter_mut().find(|(tile, _)| tile.coord == coord) else {
            continue;
        };
        for index in chunks {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            // The heights and the recomputed normals. The stroke has already
            // run `vale_edit::adt::heights::recompute_normals`. A lawn
            // regrounded onto the old normals is lit as the hill was before it
            // was raised.
            let ground = vale_edit::adt::heights::ground(chunk);
            let normals = vale_edit::adt::heights::normals(chunk);
            if plans.reground(index, ground, &normals, &mut dead) {
                replanted += 1;
            }
        }
    }
    for entity in dead {
        commands.entity(entity).despawn();
    }
    if replanted > 0 {
        debug!("{replanted} chunks of foliage regrounded");
    }
}

/// Write the moved vertices into one group's buffers.
///
/// Positions and normals cross into Bevy's axes here, which is the same crossing
/// `render::terrain`'s own builder makes and for the same reason: a change of
/// basis is a rotation, so a normal takes it exactly as a position does.
fn write_vertices(mesh: &mut Mesh, hits: &[(usize, &vale_assets::world::adt::MeshVertex)]) {
    use vale_client::render::axes;
    use bevy::mesh::VertexAttributeValues;

    if let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for (at, vertex) in hits {
            if let Some(slot) = positions.get_mut(*at) {
                *slot = axes::to_bevy(vertex.position).to_array();
            }
        }
    }
    if let Some(VertexAttributeValues::Float32x3(normals)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_NORMAL)
    {
        for (at, vertex) in hits {
            if let Some(slot) = normals.get_mut(*at) {
                *slot = axes::to_bevy(vertex.normal).to_array();
            }
        }
    }
    // The vertex colour, the third attribute a vertex has. It uses this path
    // rather than one of its own because `MCCV` has one fixed length or is
    // absent (see `vale_edit::adt::colours`). A colour stroke therefore changes
    // a vertex as a height stroke does, and the mesh keeps its shape, its
    // indices and its length. The attribute is absent in a client build, which
    // never uploads it. It is present in every editor build, because
    // `LiveEdits` requests it whether or not the tile has colours yet.
    if let Some(VertexAttributeValues::Float32x4(colours)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR)
    {
        for (at, vertex) in hits {
            if let Some(slot) = colours.get_mut(*at) {
                *slot = vertex.colour;
            }
        }
    }
}

/// The tiles a brush's circle reaches.
///
/// Returns every tile in the bounding box's range, not only the tiles under the
/// four corners. The corners are enough while a brush is smaller than a tile,
/// as it was when the ceiling was 200 yards against a tile's 533. A 600-yard
/// brush spans three tiles on an axis and its four corners name none of the
/// middle ones, so it would paint a ring around a cross of untouched ground.
///
/// It uses the box rather than the circle because a corner of the box outside
/// the circle adds at most one extra tile whose vertices all weigh zero.
fn tiles_under(brush: &Brush, at: Vec3) -> Vec<(u32, u32)> {
    tiles_in_range(brush.radius, at)
}

/// The tiles a circle of any radius reaches. The texture brush uses it too.
pub(crate) fn tiles_in_range(radius: f32, at: Vec3) -> Vec<(u32, u32)> {
    let a = vale_assets::tile_for_position(at.x - radius, at.y - radius);
    let b = vale_assets::tile_for_position(at.x + radius, at.y + radius);
    let mut found = Vec::new();
    for y in a.1.min(b.1)..=a.1.max(b.1) {
        for x in a.0.min(b.0)..=a.0.max(b.0) {
            found.push((x, y));
        }
    }
    found
}

/// Put the edited bytes where the archives will answer with them, once the
/// stroke that made them is over.
///
/// On release rather than every frame: `AdtFile::write` lays out a whole tile,
/// which is two megabytes of memcpy, and nothing reads those bytes until
/// something asks the archives for the tile. What the ground on screen is drawn
/// from is [`live_ground`]'s patch, not this.
fn publish(mut session: Option<ResMut<EditSession>>, buttons: Res<ButtonInput<MouseButton>>) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let unsaved: Vec<(u32, u32)> = session.unsaved.iter().copied().collect();
    for coord in unsaved {
        session.publish(coord);
    }
}

/// Marks a tile on screen that a fresh read will replace. The number is its
/// batch: the other tiles it changes places with at the same time.
///
/// The tile stays drawn, unchanged, until [`swap`] removes it on the frame
/// every replacement in its batch is whole. See [`remesh`] for why a tile is
/// replaced at all and [`swap`] for why tiles are replaced in batches.
#[derive(Component)]
pub struct Replacing(pub u64);

/// Hand a stale tile back to the streamer, keeping the old one on screen.
///
/// ## Why the old tile stays on screen until the new one is whole
///
/// Despawning the old tile at once leaves a third of a second with no tile
/// there, because despawning is instant and reading a tile again is a parse, a
/// mesh of 256 chunks and eleven texture decodes. At editing distance one tile
/// is most of the screen, so the gap looked like the whole screen flashing and
/// the terrain disappearing. This function used to do that.
///
/// Leaving the old tile and spawning the new one visible over it is worse. Two
/// copies of identical ground in the same place z-fight along every triangle
/// until one is removed.
///
/// So the old tile is marked and left alone, the new one is spawned hidden,
/// and the two change places in one frame. `LoadedTiles::arriving` says when
/// the new one is whole. A host cannot work that out on its own, because the
/// upload budget spreads a tile's atlas, textures and draw groups over several
/// frames.
///
/// A tile whose replacement never arrives, such as an ADT that does not parse,
/// keeps the old one on screen indefinitely. That is the intended failure.
fn remesh(
    mut session: Option<ResMut<EditSession>>,
    mut loaded: ResMut<LoadedTiles>,
    tiles: Query<(Entity, &TerrainTile, Option<&Replacing>)>,
    mut commands: Commands,
    // One batch per pass, not per tile. See [`swap`]. Tiles made stale together
    // are replaced together, which matters for the one edit that makes two
    // tiles stale at once: a placement moved across a tile border.
    mut batch: Local<u64>,
    // The change count each in-flight replacement was started at. See the
    // loop below, and `crate::session::EditSession::changes`.
    mut started: Local<bevy::platform::collections::HashMap<(u32, u32), u64>>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };

    // A replacement reads the file once, when it is started. That read cannot
    // see a change made between then and the frame it lands, and nothing
    // downstream corrects it: the live sets (the ground's vertices, a
    // placement's transform, a chunk's blend map) were drained against the
    // outgoing copy, which is despawned when the incoming one appears. The tile
    // on screen then shows the older bytes until something else makes it stale.
    //
    // Example: a building stayed where it had been dragged after an undo, with
    // the file already correct. A drag ends by requesting the tile again, and
    // that read takes a third of a second, or seconds on a city tile. `Ctrl+Z`
    // inside that window moves the outgoing copy back, and then a copy built
    // before the undo replaces it.
    //
    // The fix is to request another read, which converges. The one-at-a-time
    // guard below does the same: the file is correct, so reading it again gives
    // the correct tile.
    started.retain(|coord, _| {
        tiles
            .iter()
            .any(|(_, tile, going)| tile.coord == *coord && going.is_some())
    });
    let mut behind: Vec<(u32, u32)> = Vec::new();
    // Compared against every in-memory change and not only against publishes:
    // a vertex drag changes the tile on every frame and publishes it only when
    // the button comes up. See `crate::session::EditSession::changes`.
    for (coord, had) in started.iter() {
        if *had != session.changes(*coord) {
            behind.push(*coord);
        }
    }
    for coord in behind {
        session.stale.insert(coord);
    }
    // Stale tiles are replaced while the button is still down. This used to
    // wait for the release, because a rebuild left a hole in the world for a
    // third of a second. A replacement now keeps the old tile on screen until
    // the new one is whole, so a rebuild costs a task on the compute pool and
    // shows nothing on screen. Waiting for the release meant paint on a newly
    // reached tile did not show until the mouse button was let go.
    //
    // The one-at-a-time guard below keeps this from queueing a rebuild every
    // frame: a tile already being replaced stays stale and is picked up when
    // the one in flight lands, by which point the file holds everything
    // painted since.
    if session.stale.is_empty() {
        return;
    }
    let stale: Vec<(u32, u32)> = session.stale.drain().collect();
    *batch += 1;
    for coord in stale {
        // One replacement at a time per tile. A second `reload` while the
        // first is still arriving would leave two outgoing copies and no way to
        // tell which one the incoming copy replaces. The tile stays stale and
        // is picked up on a later frame, by which point the file holds
        // everything the stroke wrote.
        if tiles
            .iter()
            .any(|(_, tile, going)| tile.coord == coord && going.is_some())
        {
            session.stale.insert(coord);
            continue;
        }
        for (entity, tile, _) in &tiles {
            if tile.coord == coord {
                commands.entity(entity).insert(Replacing(*batch));
            }
        }
        // Publish first, so the read sees the tile as it is now. A read is of
        // the overlay, which holds what was last published, and a tile can be
        // ahead of that: a vertex drag publishes when the button comes up, but
        // carrying the placements on the moved ground can mark the tile stale
        // on any frame of the drag. Read from the older bytes, the replacement
        // drew part of the ground as it was before the drag.
        if session.is_unpublished(coord) {
            session.publish(coord);
        }
        // What the read about to start will see. Taken here rather than when it
        // lands, so the error is on the side of one read too many.
        started.insert(coord, session.changes(coord));
        // The old tile is not despawned here. `LoadedTiles`' own comment warns
        // that forgetting a tile without despawning it draws the tile twice.
        // [`swap`] does the despawn, one frame later than this function used
        // to.
        loaded.reload(coord);
    }
}

/// Put the replacements on screen and take the old tiles off, in one frame.
///
/// ## Tiles made stale together change over together
///
/// One tile at a time is correct for a texture edit, which affects one tile. It
/// is wrong for an edit that moves an object between two tiles. A placement
/// dragged past a tile border is taken out of one file and put into the other,
/// so both tiles are read again, and the two reads finish at different times.
/// If each tile swapped as soon as it was ready, then between the two swaps the
/// tile that gained the placement draws it while the tile that lost it still
/// draws the old copy, and the object is on screen twice.
///
/// In Stormwind that window lasts seconds: a city tile is 311 groups and
/// 844,627 vertices, so the read that removes the placement takes far longer
/// than the read that adds it.
///
/// So the batch is the unit. Every tile [`remesh`] made stale in one pass
/// carries the same number, and none of them changes over until all of them
/// are whole.
///
/// Runs every frame there is an outgoing tile, which is for about a third of a
/// second after an edit that needs a re-read and never otherwise.
fn swap(
    loaded: Res<LoadedTiles>,
    mut tiles: Query<(Entity, &TerrainTile, Option<&Replacing>, &mut Visibility)>,
    mut commands: Commands,
) {
    let outgoing: Vec<(Entity, (u32, u32), u64)> = tiles
        .iter()
        .filter_map(|(entity, tile, going, _)| Some((entity, tile.coord, going?.0)))
        .collect();
    if outgoing.is_empty() {
        return;
    }

    let mut batches: Vec<u64> = outgoing.iter().map(|(_, _, batch)| *batch).collect();
    batches.sort_unstable();
    batches.dedup();

    for batch in batches {
        let mine: Vec<(Entity, (u32, u32))> = outgoing
            .iter()
            .filter(|(_, _, had)| *had == batch)
            .map(|(entity, coord, _)| (*entity, *coord))
            .collect();

        // Every replacement in the batch, and whether all of them have landed.
        // A replacement is whole when every draw group has arrived, not when it
        // is spawned. A tile's root exists from its first instalment and its
        // ground arrives over the following frames, so swapping when the root
        // appears could show a tile with two of its thirty draw groups.
        let mut incoming: Vec<Entity> = Vec::new();
        let mut ready = true;
        for (old, coord) in &mine {
            let mut found = false;
            for (entity, tile, going, _) in tiles.iter() {
                if tile.coord == *coord && going.is_none() && entity != *old {
                    incoming.push(entity);
                    found = true;
                }
            }
            if !found || loaded.arriving() == Some(*coord) {
                ready = false;
            }
        }

        for entity in incoming {
            let Ok((.., mut visible)) = tiles.get_mut(entity) else {
                continue;
            };
            let wanted = match ready {
                true => Visibility::Inherited,
                false => Visibility::Hidden,
            };
            // Guarded on the value, because `DerefMut` marks a component changed
            // whether or not it changed and this runs every frame of the swap.
            if *visible != wanted {
                *visible = wanted;
            }
        }
        if ready {
            for (old, _) in mine {
                commands.entity(old).despawn();
            }
        }
    }
}

/// The brush ring, drawn on the ground it is about to change.
///
/// The ring follows the edited heights rather than the drawn mesh, for the same
/// reason [`crate::pick`] marches its ray against them: while a stroke is held,
/// the two differ by everything painted so far, and the ring is the only thing
/// on screen that is up to date.
fn draw_brush(
    mut marks: ResMut<crate::marks::Marks>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    terrain: Res<Terrain>,
    cursor: Res<Cursor>,
    held: Res<Held>,
    pressure: crate::tablet::Pressure,
) {
    if *tool != Tool::Terrain {
        return;
    }
    // `cursor.ground` is `None` for the whole of a playtest (see
    // [`crate::pick::aim`]), so no separate playtest check is needed here.
    let (Some(session), Some(at)) = (session, cursor.ground) else {
        return;
    };
    let colour = match terrain.brush.mode {
        // While the pointer selects, the ring is the footprint a press
        // selects and no brush mode's.
        _ if terrain.selecting => Color::WHITE,
        Mode::Raise => Color::srgb(0.4, 0.9, 0.4),
        Mode::Lower => Color::srgb(0.9, 0.5, 0.3),
        Mode::Flatten { .. } => Color::srgb(0.4, 0.7, 1.0),
        Mode::Smooth => Color::srgb(0.9, 0.9, 0.4),
        Mode::Noise => Color::srgb(0.8, 0.5, 0.9),
    };
    // Two rings: the radius, and the half-radius where a smooth falloff still
    // has most of its strength. Both follow the ground rather than lying flat.
    // A flat circle seen on a slope would look like an ellipse in the wrong
    // place.
    // At the radius a pen's pressure gives it, which is what the stroke
    // reaches this frame. The vertex selection is not scaled.
    let radius = match terrain.selecting {
        true => terrain.brush.radius,
        false => (terrain.brush.radius * pressure.scale().radius).max(*RADIUS.start()),
    };
    rings(
        &mut marks,
        &session,
        at,
        radius,
        terrain.brush.core,
        terrain.brush.shape,
        colour,
        held.on,
    );
    // A tilted flatten: the plane it goes to, at the pointer. While a stroke
    // is held that is the plane through where it began; before, the plane
    // the next press would take.
    if let (false, Mode::Flatten { to }) = (terrain.selecting, terrain.brush.mode) {
        if terrain.tilt_angle > 0.0 {
            let height = match (held.on, terrain.flatten_to_cursor) {
                (true, _) => vale_edit::ops::vertices::plane(
                    [terrain.brush.pivot[0], terrain.brush.pivot[1], to],
                    terrain.brush.tilt,
                    at.to_array(),
                ),
                (false, true) => at.z,
                (false, false) => terrain.flatten_height,
            };
            draw_tilt(
                &mut marks,
                Vec3::new(at.x, at.y, height),
                terrain.brush.radius,
                terrain.tilt_angle,
                terrain.tilt_toward,
                TILT_COLOUR,
            );
        }
    }
}

/// The two rings every brush draws: the radius, and the core inside it.
///
/// The inner ring marks the core, and is not drawn when the core is zero. It
/// used to sit at half the radius and mark nothing in particular. Users read
/// it as where the pressure is focused, which is what the core now is.
///
/// It is one function so that the three brushes that take a core cannot differ
/// about whether they draw it. Each brush used to draw the rings itself, and
/// the shading brush did not draw the core: its `Alt`+wheel set a core the
/// preview never showed, so the control that decides where the paint lands
/// had no feedback.
///
/// `active` is true while a stroke is changing the ground. The rings are then
/// drawn solid and a little closer to white, at the preview's width, so a
/// stroke that is being applied looks different from the preview of the next
/// one. They were drawn twice as wide and 45% toward white as well, and the
/// heavier line hid the ground being painted.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rings(
    marks: &mut crate::marks::Marks,
    session: &EditSession,
    at: Vec3,
    radius: f32,
    core: f32,
    shape: Shape,
    colour: Color,
    active: bool,
) {
    use bevy::color::Mix;
    let colour = match active {
        true => colour.mix(&Color::WHITE, ACTIVE_WHITE),
        false => colour,
    };
    let inner = (core > 0.0).then(|| radius * core);
    for (radius, shade) in [Some(radius), inner]
        .into_iter()
        .zip([1.0, 0.6])
        .filter_map(|(radius, shade)| Some((radius?, shade)))
    {
        ring(marks, session, at, radius, shape, colour.with_alpha(shade), active);
    }
}

/// How far toward white an active stroke's rings are drawn, 0 to 1.
const ACTIVE_WHITE: f32 = 0.2;

/// One ring of 64 segments, each vertex dropped onto the edited ground.
///
/// While `solid` is false, three segments of each four are drawn, so the ring
/// is sixteen dashes and reads as a brush rather than as a line on the map.
/// While it is true, every segment is drawn, at the same width: see
/// [`rings`]. A dash is a tube a few
/// pixels thick ([`crate::marks::Marks::line`]), so a hill between the camera
/// and the ring hides it; the hidden part is drawn faint
/// ([`crate::marks::Look::Ghosted`]).
///
/// The texture brush uses it too, rather than a copy of its own, so the two
/// brushes cannot disagree about where the pointer is.
///
/// A ring vertex with no ground under it breaks the ring rather than taking
/// another height. There are three cases: a hole in the terrain, a tile that
/// is not open, and the ground beyond the edge of the map. An earlier version
/// used the height under the pointer for all three. During a raise that height
/// is the top of the hill being built, so the ring drew a spike up to it
/// wherever it crossed a hole, which looked like torn ground. A gap in the ring
/// shows that there is no ground there.
pub(crate) fn ring(
    marks: &mut crate::marks::Marks,
    session: &EditSession,
    at: Vec3,
    radius: f32,
    shape: Shape,
    colour: Color,
    solid: bool,
) {
    const SEGMENTS: usize = 64;
    /// Yards above the ground, so the line is not buried in the mesh it is
    /// drawn over.
    const LIFT: f32 = 0.15;

    // The shape's own outline, not always a circle, and taken from the shape
    // rather than computed here. A preview built from a second copy of the
    // footprint logic could show the brush reaching where it does not, and the
    // two would drift apart the first time a norm was corrected. See
    // `vale_edit::ops::Shape::outline`.
    let rim = shape.outline(radius, SEGMENTS);
    let point = |&(dx, dy): &(f32, f32)| -> Option<Vec3> {
        let (x, y) = (at.x + dx, at.y + dy);
        let coord = vale_assets::tile_for_position(x, y);
        let z = session
            .tiles
            .get(&coord)
            .and_then(|tile| vale_edit::adt::heights::height_at(tile, x, y))?;
        Some(vale_client::render::axes::to_bevy([x, y, z + LIFT]))
    };
    let points: Vec<Option<Vec3>> = rim.iter().map(point).collect();
    const WIDTH: f32 = 1.5;
    for i in (0..points.len()).filter(|i| solid || i % 4 != 3) {
        if let (Some(from), Some(to)) = (points[i], points[(i + 1) % points.len()]) {
            // Resting on the ground rather than half in it. See
            // `crate::marks::Marks::ground_line`.
            marks.wide_ground_line(from, to, WIDTH, colour, crate::marks::Look::Ghosted);
        }
    }
}

/// How many yards across a bump may be, for [`Mode::Noise`].
///
/// The floor is the 4.17 yards between vertices: below that the lattice is finer
/// than the ground can express and the result is the white noise this replaced.
/// The ceiling is about a chunk and a half, past which one brush sits inside a
/// single feature and the stroke is a raise or a lower depending on where it
/// landed.
pub const SCALE: std::ops::RangeInclusive<f32> = 4.17..=200.0;

/// The five modes, as a list a panel can offer.
pub const MODES: [(&str, Mode); 5] = [
    ("Raise", Mode::Raise),
    ("Lower", Mode::Lower),
    ("Flatten", Mode::Flatten { to: 0.0 }),
    ("Smooth", Mode::Smooth),
    ("Noise", Mode::Noise),
];

/// The five falloffs, as a list a panel can offer.
///
/// They are in order of how much of the radius keeps its strength, which is
/// the choice a user makes: `Dome` holds nearly all of it and `Sharp` loses it
/// at once. The other three are the original falloffs.
pub const FALLOFFS: [(&str, Falloff); 5] = [
    ("Smooth", Falloff::Smooth),
    ("Linear", Falloff::Linear),
    ("Flat", Falloff::Flat),
    ("Sharp", Falloff::Sharp),
    ("Dome", Falloff::Dome),
];

/// The three footprints, as a list a panel can offer. `vale_edit::ops::Shape`
/// explains why a shape is a distance rather than a mask. That design lets these
/// three and the five falloffs above be two lists rather than fifteen
/// combinations.
pub const SHAPES: [(&str, Shape); 3] = [
    ("Circle", Shape::Circle),
    ("Square", Shape::Square),
    ("Diamond", Shape::Diamond),
];
